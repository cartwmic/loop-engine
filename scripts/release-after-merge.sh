#!/usr/bin/env bash
# Cut a loop-engine release after a merge to main, as one command.
#
# Runs the release sequence in AGENTS.md ("Authorized release sequence") end to
# end and stops at the first failure, naming the step:
#   prepare  scripts/prepare-release.sh in a fresh worktree from origin/main,
#            plus the caller's CHANGELOG notes
#   pr       commit on release/vX.Y.Z, push, open the release-prep PR
#   checks   wait for the PR checks
#   merge    squash-merge the PR and read the merge SHA
#   ci       wait for push-event ci.yml green on that exact SHA
#   dry-run  dispatch release.yml with tag=dry-run on that SHA and wait
#   publish  dispatch release.yml with tag=vX.Y.Z and wait
# Every step reads GitHub first, so a rerun resumes where the last run stopped
# and never repeats a merge or a dispatch that succeeded or is still running.
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage: scripts/release-after-merge.sh [--version X.Y.Z] [--notes FILE]

  --version X.Y.Z  release version (default: main's version if it is not
                   released yet, else the next minor)
  --notes FILE     CHANGELOG text for this version (required when a
                   release-prep PR has to be made)

environment:
  RELEASE_PREPARE_CMD   prepare command run in the worktree with VERSION
                        (default: scripts/prepare-release.sh)
  RELEASE_WORKTREE      prep worktree (default: next to the main checkout,
                        loop-engine-release-vX.Y.Z)
  RELEASE_POLL_SECONDS  seconds between polls (default 30)
  RELEASE_APPEAR_POLLS  polls to wait for checks or a run to appear (default 20)
  RELEASE_LOG           log file (default:
                        $XDG_STATE_HOME/loop-engine/release-after-merge.log)
EOF
}

version=
notes=
while [[ $# -gt 0 ]]; do
  case $1 in
    --version) version=${2:?--version needs a value}; version=${version#v}; shift 2 ;;
    --notes) notes=${2:?--notes needs a file}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) usage; exit 2 ;;
  esac
done
if [[ -n $version && ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "--version must be a stable semantic version such as 0.35.0" >&2
  exit 2
fi
if [[ -n $notes ]]; then
  [[ -f $notes ]] || { echo "--notes file not found: $notes" >&2; exit 2; }
  notes=$(cd "$(dirname "$notes")" && pwd)/$(basename "$notes")
fi

poll_seconds=${RELEASE_POLL_SECONDS:-30}
appear_polls=${RELEASE_APPEAR_POLLS:-20}
log_file=${RELEASE_LOG:-${XDG_STATE_HOME:-$HOME/.local/state}/loop-engine/release-after-merge.log}
mkdir -p "$(dirname "$log_file")"
action_id="release-$(date -u +%Y%m%dT%H%M%SZ)-$$"
started=$(date +%s)
step=start

log_line() {
  printf '%s %s step=%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$action_id" "$step" "$*" >> "$log_file"
}
say() {
  printf '[%s] %s\n' "$step" "$*"
  log_line "$*"
}
finish() {
  status=$?
  log_line "end exit=$status duration=$(( $(date +%s) - started ))s"
}
trap finish EXIT
stop() {
  printf 'STOP [%s] %s\n' "$step" "$*" >&2
  log_line "STOP $*"
  printf 'rerun scripts/release-after-merge.sh to resume at this step once it is fixed\n' >&2
  exit 1
}

# json_get PATH: print one field of the JSON on stdin (dot path, list indexes
# allowed). Missing fields print nothing.
json_get() {
  python3 -c '
import json, sys
value = json.load(sys.stdin)
for part in sys.argv[1].split(".") if sys.argv[1] else []:
    if isinstance(value, list):
        value = value[int(part)] if part.isdigit() and int(part) < len(value) else None
    elif isinstance(value, dict):
        value = value.get(part)
    if value is None:
        break
if value is None:
    pass
elif isinstance(value, bool):
    print("true" if value else "false")
elif isinstance(value, (dict, list)):
    print(json.dumps(value))
else:
    print(value)
' "$1"
}

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"
log_line "begin args: version=${version:-default} notes=${notes:-none}"

step=fetch
git fetch --quiet --tags origin main || stop "git fetch origin failed"
main_version=$(git show origin/main:Cargo.toml | python3 -c '
import re, sys
match = re.search(r"(?ms)^\[workspace\.package\]\n(?:(?!^\[).)*?^version = \"([^\"]+)\"$", sys.stdin.read())
print(match.group(1) if match else "")
')
[[ -n $main_version ]] || stop "origin/main Cargo.toml has no [workspace.package] version"

# release_url TAG: the release URL, or nothing when the tag has no release.
release_url() {
  local out err
  err=$(mktemp)
  if out=$(gh release view "$1" --json url 2>"$err"); then
    rm -f "$err"
    printf '%s' "$out" | json_get url
    return 0
  fi
  if grep -q 'release not found' "$err"; then
    rm -f "$err"
    return 0
  fi
  cat "$err" >&2
  rm -f "$err"
  stop "gh release view $1 failed"
}

step=version
if [[ -z $version ]]; then
  main_release=$(release_url "v$main_version")
  if [[ -z $main_release ]]; then
    version=$main_version
    say "main is at $main_version with no release yet: releasing it"
  else
    version=$(python3 -c 'import sys; a, b, _ = sys.argv[1].split("."); print(f"{a}.{int(b) + 1}.0")' "$main_version")
    say "main is at released $main_version: next minor is $version"
  fi
fi
tag=v$version
branch=release/$tag
log_line "tag=$tag"

summary() {
  local url=$1 sha
  sha=$(git ls-remote origin "refs/tags/$tag^{}" | cut -f1)
  [[ -n $sha ]] || sha=$(git ls-remote origin "refs/tags/$tag" | cut -f1)
  step="done"
  say "released $tag"
  say "commit: ${sha:-unknown}"
  say "release: $url"
  [[ -z ${dry_run_url:-} ]] || say "dry run: $dry_run_url"
  [[ -z ${publish_url:-} ]] || say "publish run: $publish_url"
  say "next (playbook rule 7c57de5a): ~/.local/user_scripts/import_release_skills.py bump $tag"
}

step=release
existing=$(release_url "$tag")
if [[ -n $existing ]]; then
  say "$tag is already released"
  summary "$existing"
  exit 0
fi

main_sha() {
  gh api 'repos/{owner}/{repo}/commits/main' | json_get sha
}

# Find the prep PR for this version: the open one, else the merged one.
prs=$(gh pr list --head "$branch" --state all --json number,state,url,mergeCommit) || { step="pr"; stop "gh pr list failed"; }
pick_pr() {
  printf '%s' "$prs" | python3 -c '
import json, sys
prs = json.load(sys.stdin)
for state in ("OPEN", "MERGED"):
    for pr in prs:
        if pr["state"] == state:
            print(pr["number"], pr["state"], pr["url"], (pr.get("mergeCommit") or {}).get("oid", ""))
            sys.exit(0)
'
}
read -r pr_number pr_state pr_url merge_sha <<<"$(pick_pr)" || true

if [[ $version != "$main_version" && -z ${pr_number:-} ]]; then
  step=prepare
  [[ -n $notes ]] || stop "--notes FILE is required to write the CHANGELOG entry for $version"
  main_checkout=$(git worktree list --porcelain | sed -n '1s/^worktree //p')
  worktree=${RELEASE_WORKTREE:-$(dirname "$main_checkout")/loop-engine-release-$tag}
  prepared=
  if [[ -e $worktree ]]; then
    [[ -z $(git -C "$worktree" status --porcelain=v1 --untracked-files=all) ]] \
      || stop "dirty tree: $worktree has uncommitted changes from an earlier attempt; inspect it, then remove it with git worktree remove --force"
    if [[ $(git -C "$worktree" branch --show-current) == "$branch" ]] \
      && [[ $(git -C "$worktree" log -1 --format=%s) == "release: prepare $tag" ]]; then
      prepared=yes
      say "reusing the prepared commit in $worktree"
    else
      say "resetting the clean worktree $worktree to origin/main"
      git -C "$worktree" checkout --quiet -B "$branch" origin/main || stop "could not reset $worktree"
    fi
  else
    say "worktree $worktree on $branch from origin/main"
    git worktree add --quiet -B "$branch" "$worktree" origin/main || stop "git worktree add failed"
  fi
  if [[ -z $prepared ]]; then
    prepare_cmd=${RELEASE_PREPARE_CMD:-scripts/prepare-release.sh}
    say "running $prepare_cmd $version"
    (cd "$worktree" && $prepare_cmd "$version") || stop "$prepare_cmd $version failed in $worktree"
    say "adding the CHANGELOG entry from $notes"
    python3 - "$worktree/CHANGELOG.md" "$version" "$(date -u +%Y-%m-%d)" "$notes" <<'PY' || stop "could not add the CHANGELOG entry"
import sys
from pathlib import Path

path, version, date, notes = Path(sys.argv[1]), sys.argv[2], sys.argv[3], Path(sys.argv[4])
text = path.read_text()
heading = "# Changelog\n\n"
if not text.startswith(heading):
    raise SystemExit("CHANGELOG.md does not start with '# Changelog'")
if f"## [{version}]" in text:
    raise SystemExit(f"CHANGELOG.md already has a {version} section")
body = notes.read_text().strip()
if not body:
    raise SystemExit("--notes file is empty")
path.write_text(heading + f"## [{version}] - {date}\n\n{body}\n\n" + text[len(heading):])
PY
    step="pr"
    git -C "$worktree" add -A
    git -C "$worktree" commit --quiet -m "release: prepare $tag" || stop "git commit failed"
  fi
  step="pr"
  say "pushing $branch"
  git -C "$worktree" push --quiet -u origin "$branch" || stop "git push origin $branch failed"
  pr_url=$(cd "$worktree" && gh pr create --base main --head "$branch" --title "release: prepare $tag" \
    --body "Version bump, lock refresh and changelog for $tag. Prepared with scripts/release-after-merge.sh (scripts/prepare-release.sh $version).") \
    || stop "gh pr create failed"
  pr_number=${pr_url##*/}
  pr_state=OPEN
  say "opened $pr_url"
fi

if [[ ${pr_state:-} == OPEN ]]; then
  step=checks
  say "waiting for checks on $pr_url"
  empty_polls=0
  while :; do
    err=$(mktemp)
    out=$(gh pr checks "$pr_number" --json name,bucket,link 2>"$err") || true
    if [[ -z $out ]] && ! grep -q 'no checks reported' "$err"; then
      cat "$err" >&2; rm -f "$err"
      stop "gh pr checks $pr_number failed"
    fi
    rm -f "$err"
    verdict=$(printf '%s' "${out:-[]}" | python3 -c '
import json, sys
checks = json.load(sys.stdin)
red = [c["name"] for c in checks if c["bucket"] in ("fail", "cancel")]
if red:
    print("red", ",".join(red))
elif not checks:
    print("none")
elif all(c["bucket"] in ("pass", "skipping") for c in checks):
    print("green", ",".join(c["name"] for c in checks))
else:
    print("pending")
')
    case $verdict in
      red*) stop "checks red on $pr_url: ${verdict#red }" ;;
      green*) say "checks green: ${verdict#green }"; break ;;
      none)
        empty_polls=$((empty_polls + 1))
        [[ $empty_polls -lt $appear_polls ]] || stop "no checks reported on $pr_url" ;;
    esac
    sleep "$poll_seconds"
  done

  step=merge
  say "squash-merging $pr_url"
  gh pr merge "$pr_number" --squash || stop "gh pr merge $pr_number failed"
  view=$(gh pr view "$pr_number" --json state,mergeCommit,url) || stop "gh pr view $pr_number failed"
  [[ $(printf '%s' "$view" | json_get state) == MERGED ]] || stop "$pr_url is not merged"
  merge_sha=$(printf '%s' "$view" | json_get mergeCommit.oid)
  say "merged as $merge_sha"
  if [[ -n ${worktree:-} && -e $worktree ]]; then
    git worktree remove "$worktree" || say "could not remove $worktree; remove it by hand"
  fi
elif [[ ${pr_state:-} == MERGED ]]; then
  step=merge
  say "prep PR $pr_url already merged as $merge_sha"
else
  step=merge
  stop "main is at $version but no release-prep PR from $branch was found"
fi
[[ -n $merge_sha ]] || stop "no merge SHA for $pr_url"

check_main() {
  local tip
  tip=$(main_sha) || stop "gh api commits/main failed"
  [[ $tip == "$merge_sha" ]] \
    || stop "main moved: main is $tip, not the release SHA $merge_sha; get exact-SHA proof for the new tip before releasing"
}

step=ci
check_main
say "waiting for push-event ci.yml on $merge_sha"
empty_polls=0
while :; do
  run=$(gh run list --workflow ci.yml --branch main --event push --commit "$merge_sha" --limit 1 \
    --json databaseId,status,conclusion,headSha,url) || stop "gh run list failed"
  id=$(printf '%s' "$run" | json_get 0.databaseId)
  if [[ -z $id ]]; then
    empty_polls=$((empty_polls + 1))
    [[ $empty_polls -lt $appear_polls ]] || stop "push CI missing for $merge_sha"
  else
    url=$(printf '%s' "$run" | json_get 0.url)
    [[ $(printf '%s' "$run" | json_get 0.headSha) == "$merge_sha" ]] || stop "push CI $url is on another SHA"
    if [[ $(printf '%s' "$run" | json_get 0.status) == completed ]]; then
      [[ $(printf '%s' "$run" | json_get 0.conclusion) == success ]] || stop "push CI failed: $url"
      say "push CI green: $url"
      break
    fi
  fi
  sleep "$poll_seconds"
done

# release.yml dispatch runs on the SHA, oldest first: "id status conclusion url".
release_runs() {
  gh run list --workflow release.yml --event workflow_dispatch --limit 50 \
    --json databaseId,status,conclusion,headSha,createdAt,url \
    | python3 -c '
import json, sys
runs = sorted((r for r in json.load(sys.stdin) if r["headSha"] == sys.argv[1]), key=lambda r: r["createdAt"])
for r in runs:
    print(r["databaseId"], r["status"], r["conclusion"] or "-", r["url"])
' "$merge_sha"
}

wait_run() {
  local id=$1 view
  while :; do
    view=$(gh run view "$id" --json status,conclusion,headSha,url) || stop "gh run view $id failed"
    [[ $(printf '%s' "$view" | json_get headSha) == "$merge_sha" ]] \
      || stop "release run $(printf '%s' "$view" | json_get url) is on another SHA"
    if [[ $(printf '%s' "$view" | json_get status) == completed ]]; then
      printf '%s' "$view" | json_get conclusion
      return 0
    fi
    sleep "$poll_seconds"
  done
}

dispatch() {
  local tag_input=$1 before after new polls=0
  check_main
  before=$(release_runs | cut -d' ' -f1 | tr '\n' ' ')
  say "dispatching release.yml tag=$tag_input on main ($merge_sha)"
  gh workflow run release.yml --ref main -f "tag=$tag_input" || stop "gh workflow run release.yml failed"
  while :; do
    after=$(release_runs)
    new=$(printf '%s\n' "$after" | awk -v seen="$before" 'BEGIN { n = split(seen, s, " "); for (i = 1; i <= n; i++) old[s[i]] = 1 } NF && !($1 in old) { print $1, $4 }' | tail -n 1)
    [[ -z $new ]] || break
    polls=$((polls + 1))
    [[ $polls -lt $appear_polls ]] || stop "the dispatched release.yml run did not appear for $merge_sha"
    sleep "$poll_seconds"
  done
  say "dispatched: ${new#* }"
}

while :; do
  existing=$(release_url "$tag")
  if [[ -n $existing ]]; then
    summary "$existing"
    exit 0
  fi
  runs=$(release_runs) || stop "gh run list failed"
  # A success without a release is the dry run; runs after it are publishes.
  state=$(printf '%s\n' "$runs" | awk '
    NF { n++; id[n] = $1; st[n] = $2; c[n] = $3; u[n] = $4; if ($2 == "completed" && $3 == "success") ok = n }
    END {
      if (n == 0) { print "dry-run none"; exit }
      if (st[n] != "completed") { print (ok ? "publish" : "dry-run"), "running", id[n], u[n]; exit }
      if (!ok) { print "dry-run failed", id[n], u[n]; exit }
      if (ok == n) { print "publish ready", id[ok], u[ok]; exit }
      print "publish failed", id[n], u[n]
    }')
  read -r phase what run_id run_url <<<"$state"
  step=$phase
  case $what in
    running)
      say "waiting for $run_url"
      conclusion=$(wait_run "$run_id")
      [[ $conclusion == success ]] || stop "$phase run failed: $run_url"
      if [[ $phase == publish ]]; then
        publish_url=$run_url
        existing=$(release_url "$tag")
        [[ -n $existing ]] || stop "publish run $run_url succeeded but $tag has no release"
      else
        dry_run_url=$run_url
        say "dry run green: $run_url"
      fi
      ;;
    ready)
      dry_run_url=$run_url
      step="publish"
      dispatch "$tag"
      ;;
    none|failed)
      # A rerun after a failed dry run or publish dispatches that step again.
      [[ $what == none ]] || say "the last $phase run failed ($run_url): dispatching it again"
      if [[ $phase == publish ]]; then dispatch "$tag"; else dispatch dry-run; fi
      ;;
  esac
done
