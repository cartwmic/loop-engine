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
  RELEASE_PLAN_FILE     dist plan path prepare-release.sh writes
                        (default: /tmp/loop-engine-dist-plan.json)
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
  if [[ -n ${version:-} ]]; then
    printf 'once it is fixed, resume with: scripts/release-after-merge.sh --version %s%s\n' \
      "$version" "${notes:+ --notes $notes}" >&2
  else
    printf 'once it is fixed, rerun scripts/release-after-merge.sh\n' >&2
  fi
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

# release_url TAG: sets release to the release URL, or empty when the tag has no release.
# Any other gh error stops: an unreadable release is never taken for "not released".
release_url() {
  local out err
  release=
  err=$(mktemp)
  if out=$(gh release view "$1" --json url 2>"$err"); then
    rm -f "$err"
    release=$(printf '%s' "$out" | json_get url)
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
  release_url "v$main_version"
  if [[ -z $release ]]; then
    version=$main_version
    say "main is at $main_version with no release yet: releasing it"
  else
    released_sha=$(git rev-parse --verify --quiet "refs/tags/v$main_version^{commit}" || true)
    [[ $released_sha != "$(git rev-parse origin/main)" ]] \
      || stop "nothing merged since v$main_version: main is still its release commit"
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
  say "resume or check later with: scripts/release-after-merge.sh --version $version"
  say "next (playbook rule 7c57de5a): ~/.local/user_scripts/import_release_skills.py bump $tag"
}

main_sha() {
  gh api 'repos/{owner}/{repo}/commits/main' | json_get sha
}

check_main() {
  local tip
  tip=$(main_sha) || stop "gh api commits/main failed"
  [[ $tip == "$merge_sha" ]] \
    || stop "main moved: main is $tip, not the release SHA $merge_sha; get exact-SHA proof for the new tip before releasing"
}

# Sets runs: release.yml dispatch runs on the SHA, oldest first, one "id status conclusion url" per line.
# These helpers set globals rather than print, so stop() exits the script, not a $(...) subshell.
release_runs() {
  local listed
  listed=$(gh run list --workflow release.yml --event workflow_dispatch --limit 50 \
    --json databaseId,status,conclusion,headSha,createdAt,url) || stop "gh run list release.yml failed"
  runs=$(printf '%s' "$listed" | python3 -c '
import json, sys
runs = sorted((r for r in json.load(sys.stdin) if r["headSha"] == sys.argv[1]), key=lambda r: r["createdAt"])
for r in runs:
    print(r["databaseId"], r["status"], r["conclusion"] or "-", r["url"])
' "$merge_sha") || stop "could not read the release.yml run list"
}

# Waits until the run completes; the next snapshot reads its result.
wait_run() {
  local id=$1 view
  while :; do
    view=$(gh run view "$id" --json status,conclusion,headSha,url) || stop "gh run view $id failed"
    [[ $(printf '%s' "$view" | json_get headSha) == "$merge_sha" ]] \
      || stop "release run $(printf '%s' "$view" | json_get url) is on another SHA"
    if [[ $(printf '%s' "$view" | json_get status) == completed ]]; then
      return 0
    fi
    sleep "$poll_seconds"
  done
}

# Adds the run it started to ours.
dispatch() {
  local tag_input=$1 before new polls=0
  check_main
  release_runs
  before=$(printf '%s\n' "$runs" | cut -d' ' -f1 | tr '\n' ' ')
  say "dispatching release.yml tag=$tag_input on main ($merge_sha)"
  gh workflow run release.yml --ref main -f "tag=$tag_input" || stop "gh workflow run release.yml failed"
  while :; do
    release_runs
    new=$(printf '%s\n' "$runs" | awk -v seen="$before" 'BEGIN { n = split(seen, s, " "); for (i = 1; i <= n; i++) old[s[i]] = 1 } NF && !($1 in old) { print $1, $4 }' | tail -n 1)
    [[ -z $new ]] || break
    polls=$((polls + 1))
    [[ $polls -lt $appear_polls ]] || stop "the dispatched release.yml run did not appear for $merge_sha"
    sleep "$poll_seconds"
  done
  ours+=" ${new%% *} "
  say "dispatched: ${new#* }"
}

# Runs this invocation dispatched or waited on. A failed run outside this list failed before this invocation started.
ours=" "

# decide: the release decision, one case per row of this table (DESIGN.md "Release decision").
# It reads one snapshot (runs, then release) and sets action, reason, phase, run_id and run_url.
#
#   #   release  publish run          dry run          action
#   1   no       none                 none             dispatch dry-run
#   2   no       none                 running          wait
#   3   no       none                 failed, ours     stop
#   4   no       none                 failed, earlier  dispatch dry-run again
#   5   no       none                 succeeded        dispatch publish
#   6   no       running              -                wait
#   7   no       failed, ours         -                stop
#   8   no       failed, earlier      -                dispatch publish again
#   9   no       succeeded            -                stop (no release)
#   10  yes      none                 any              stop (no publish run)
#   11  yes      running              -                wait
#   12  yes      failed               -                stop
#   13  yes      succeeded            -                done
#
# Runs carry no readable input, so phase comes from order: the first successful run on the SHA is the
# dry run, runs before it are failed dry runs, and every run after it is a publish.
decide() {
  local state rel mine dry_url seen
  state=$(printf '%s\n' "$runs" | awk '
    NF { n++; id[n] = $1; st[n] = $2; c[n] = $3; u[n] = $4; if (!dry && $2 == "completed" && $3 == "success") dry = n }
    END {
      if (n == 0 || dry == n) { phase = dry ? "publish" : "dry-run"; s = "none"; i = "-"; url = "-" }
      else {
        phase = dry ? "publish" : "dry-run"; i = id[n]; url = u[n]
        if (st[n] != "completed") s = "running"; else if (c[n] != "success") s = "failed"; else s = "succeeded"
      }
      print phase, s, i, url, (dry ? u[dry] : "-")
    }')
  read -r phase run_state run_id run_url dry_url <<<"$state"
  [[ $dry_url == - ]] || dry_run_url=$dry_url
  rel=no
  [[ -z $release ]] || rel=yes
  if [[ $rel == yes && $phase == dry-run ]]; then phase=publish run_state=none; fi
  mine=earlier
  [[ $ours != *" $run_id "* ]] || mine=ours
  case $rel/$phase/$run_state/$mine in
    no/dry-run/none/*)         action=dispatch reason="dry run next" ;;                                    # 1
    no/dry-run/running/*)      action="wait" ;;                                                             # 2
    no/dry-run/failed/ours)    action=stop reason="dry-run run failed: $run_url" ;;                       # 3
    no/dry-run/failed/earlier) action=dispatch reason="the last dry-run run failed before this invocation ($run_url): dispatching it again" ;; # 4
    no/publish/none/*)         action=dispatch reason="dry run green: $dry_run_url" ;;                     # 5
    no/publish/running/*)      action="wait" ;;                                                             # 6
    no/publish/failed/ours)    action=stop reason="publish run failed: $run_url" ;;                       # 7
    no/publish/failed/earlier) action=dispatch reason="the last publish run failed before this invocation ($run_url): dispatching it again" ;; # 8
    no/publish/succeeded/*)    action=stop reason="publish run $run_url succeeded but $tag has no release" ;; # 9
    yes/publish/none/*)        action=stop reason="$tag has a release but no release.yml publish run on $merge_sha in the last 50 dispatch runs; check $release by hand" ;; # 10
    yes/publish/running/*)     action="wait" ;;                                                             # 11
    yes/publish/failed/*)      action=stop reason="$tag has a release but its publish run failed: $run_url; the release may be incomplete, check it by hand" ;; # 12
    yes/publish/succeeded/*)   action="done" publish_url=$run_url ;;                                        # 13
    *)                         action=stop reason="no rule for $rel/$phase/$run_state/$mine" ;;
  esac
  step=$phase
  seen=$run_state
  [[ $run_state != failed ]] || seen="failed ($mine)"
  say "decision: release=$rel $phase=$seen -> $action"
}

# release_loop: snapshot, decide, act, until a stop or done.
release_loop() {
  while :; do
    step=release
    release_runs
    release_url "$tag"
    decide
    case $action in
      dispatch)
        say "$reason"
        if [[ $phase == publish ]]; then dispatch "$tag"; else dispatch dry-run; fi
        ;;
      wait)
        ours+="$run_id "
        say "waiting for $run_url"
        wait_run "$run_id"
        ;;
      stop) stop "$reason" ;;
      done)
        summary "$release"
        exit 0
        ;;
    esac
  done
}

step=release
release_url "$tag"
if [[ -n $release ]]; then
  # An existing release is done only when its publish run succeeded: decide on the tag's commit.
  merge_sha=$(git rev-parse --verify --quiet "refs/tags/$tag^{commit}" || true)
  [[ -n $merge_sha ]] || stop "$tag has a release but origin has no $tag tag; check $release by hand"
  say "$tag has a release on $merge_sha: checking its publish run"
  release_loop
fi

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
  plan_file=${RELEASE_PLAN_FILE:-/tmp/loop-engine-dist-plan.json}
  main_checkout=$(git worktree list --porcelain | sed -n '1s/^worktree //p')
  worktree=${RELEASE_WORKTREE:-$(dirname "$main_checkout")/loop-engine-release-$tag}
  prepared=
  # Only a worktree of this repo on $branch is ours to reuse; anything else at the path, or commits on
  # $branch other than the prep commit, stops so the caller decides (RELEASE_WORKTREE picks another path).
  if [[ -e $worktree ]]; then
    [[ $(git -C "$worktree" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) \
      == "$(git rev-parse --path-format=absolute --git-common-dir)" \
      && $(git -C "$worktree" branch --show-current) == "$branch" ]] \
      || stop "$worktree exists and is not this repo's worktree on $branch; move it or set RELEASE_WORKTREE"
    [[ -z $(git -C "$worktree" status --porcelain=v1 --untracked-files=all) ]] \
      || stop "dirty tree: $worktree has uncommitted changes from an earlier attempt; inspect them, keep what you need, clean the tree, then resume"
    if [[ $(git -C "$worktree" log -1 --format=%s) == "release: prepare $tag" ]] \
      && git -C "$worktree" merge-base --is-ancestor HEAD~1 origin/main; then
      prepared=yes
      say "reusing the prepared commit in $worktree"
    elif git -C "$worktree" merge-base --is-ancestor HEAD origin/main; then
      say "moving the clean worktree $worktree up to origin/main"
      git -C "$worktree" merge --quiet --ff-only origin/main || stop "could not fast-forward $worktree"
    else
      stop "$worktree has commits on $branch that are not the release prep; inspect them, then move or remove that branch"
    fi
  else
    if git rev-parse --verify --quiet "refs/heads/$branch" >/dev/null \
      && ! git merge-base --is-ancestor "refs/heads/$branch" origin/main; then
      stop "local branch $branch has commits not on main; inspect them, then delete or rename the branch"
    fi
    say "worktree $worktree on $branch from origin/main"
    git worktree add --quiet -B "$branch" "$worktree" origin/main || stop "git worktree add failed"
  fi
  if [[ -z $prepared ]]; then
    # prepare-release.sh writes the dist plan here; AGENTS.md keeps another run's plan from being overwritten.
    [[ ! -e $plan_file ]] \
      || stop "$plan_file exists; check it is a finished release's plan, remove it, then rerun"
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
    unexpected=$(git -C "$worktree" diff --cached --name-only | grep -vxE 'Cargo\.toml|Cargo\.lock|CHANGELOG\.md' || true)
    [[ -z $unexpected ]] || stop "the prepared tree changes more than Cargo.toml, Cargo.lock and CHANGELOG.md: $(echo "$unexpected" | tr '\n' ' ')"
    say "staged: $(git -C "$worktree" diff --cached --name-status | tr '\t\n' ' ;')"
    # Content check: Cargo files change only version lines; CHANGELOG.md only gains lines.
    git -C "$worktree" diff --cached >> "$log_file"
    odd=$(git -C "$worktree" diff --cached -U0 | python3 -c '
import re, sys
path = None
for line in sys.stdin:
    if line.startswith("+++ b/"):
        path = line[6:].strip()
    elif line.startswith(("+++", "---", "diff ", "index ", "@@")):
        continue
    elif line[:1] in "+-":
        if path in ("Cargo.toml", "Cargo.lock"):
            ok = re.fullmatch(r"[+-]version = \x22[^\x22]+\x22", line.rstrip())
        else:
            ok = line.startswith("+")
        if not ok:
            print(f"{path}: {line.rstrip()}")
')
    [[ -z $odd ]] || stop "the prepared diff has lines other than version bumps and new CHANGELOG text: $odd"
    git -C "$worktree" commit --quiet -m "release: prepare $tag" || stop "git commit failed"
    rm -f "$plan_file"
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
    cd "$main_checkout"  # the caller may be inside the worktree being removed
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

release_loop
