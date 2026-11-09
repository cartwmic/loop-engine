#!/usr/bin/env python3
"""Mutation check: disable each lean-change gate in turn and require the unit tests or the journey to fail."""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EVALUATE = ROOT / 'crates/lean-change-provider/src/evaluate.rs'
WORKFLOW = ROOT / 'crates/lean-change-provider/src/workflow.rs'
RUN = ROOT / 'crates/lean-change-provider/src/run.rs'
PACKET = ROOT / 'crates/lean-change-provider/src/packet.rs'
MODEL = ROOT / 'crates/lean-change-provider/src/model.rs'

# (name, file, original text, mutated text). Each original must occur exactly once.
MUTATIONS = [
    ('intent gate', EVALUATE, '("explore", "intent-ready") => intent_ready(records, &input),', '("explore", "intent-ready") => current_intent(records, &input).map(|_| ()),'),
    ('intent check required for b', EVALUATE, '    if !input.review {\n        return Ok(());', '    if true {\n        return Ok(());'),
    ('intent check not required for c', EVALUATE, '    if !input.review {\n        return Ok(());', '    if false {\n        return Ok(());'),
    ('intent check names an intent record', EVALUATE, '.find(|r| r.kind == "intent" && r.id.as_str() == check.intent)', '.find(|r| r.kind == "intent")'),
    ('intent check has its own questions', EVALUATE, '        &INTENT_QUESTIONS,\n', '        &QUESTIONS,\n'),
    ('intent check disposition', EVALUATE, 'if disposed(records, check_id, schema, &check.findings).map_err(incomplete)?\n        &&', 'if false\n        &&'),
    ('intent check fix needs a newer intent', EVALUATE, '        && check.intent == id\n    {', '        && false\n    {'),
    ('b asks for the intent check', WORKFLOW, 'if review {\n        explore.push_str(', 'if false {\n        explore.push_str('),
    ('c asks for no intent check', WORKFLOW, 'if review {\n        explore.push_str(', 'if true {\n        explore.push_str('),
    ('owner decision', EVALUATE, 'match evaluation.status {\n        Status::Approved => {}', 'match evaluation.status {\n        _ => {}\n        Status::Approved => {}'),
    ('hard routing', EVALUATE, 'if (event == "approved-hard") != intent.hard {', 'if false {'),
    ('plan gate', EVALUATE, 'fn plan(records: &[ContextRecord], input: &InitialInput) -> Gate {\n', 'fn plan(records: &[ContextRecord], input: &InitialInput) -> Gate {\n    let _ = input;\n    if true {\n        return Ok(());\n    }\n'),
    ('verification stale', EVALUATE, 'if v.tree != now {', 'if false {'),
    ('verification missing criterion', EVALUATE, 'None => problems.push(json!({"criterion": criterion.id, "problem": "missing: no check result"})),', 'None => {}'),
    ('verification failed check', EVALUATE, 'Some(r) if r.timed_out || r.exit_code != Some(0) =>', 'Some(r) if false =>'),
    ('verification approved check', EVALUATE, 'Some(r) if r.check_sha256 != criterion.check.sha256() =>', 'Some(r) if false =>'),
    ('verification of current intent', EVALUATE, 'match recorded.filter(|(_, v)| v.intent == id) {', 'match recorded {'),
    ('independence of reviewers', EVALUATE, '        || reviewer\n            .name\n            .trim()\n            .eq_ignore_ascii_case(intent.author.name.trim())\n', ''),
    ('fresh context of reviewers', EVALUATE, 'if !fresh_context {', 'if false {'),
    ('all questions answered', EVALUATE, 'questions.iter().find(|(q, _)| !asked.contains(q))', 'questions.iter().find(|(q, _)| false && !asked.contains(q))'),
    ('failed question needs finding', EVALUATE, 'if !findings.iter().any(|f| f.question == answer.question) {', 'if false {'),
    ('finding fields', EVALUATE, '            if v1_only\n                || finding.id.trim().is_empty()\n                || finding.text.trim().is_empty()\n                || blank(&finding.evidence)\n                || blank(&finding.consequence)\n                ||', '            if v1_only\n                ||'),
    ('a record without findings_schema refuses the v1 fields', EVALUATE, '            if v1_only\n                || finding.id.trim()', '            if false\n                || finding.id.trim()'),
    ('finding disposition', EVALUATE, '            _ => {\n                return Err(format!(\n                    "finding {} needs a disposition (fixed or rejected) with a reason",\n                    finding.id\n                ))\n            }', '            _ => {}'),
    ('review tree change', EVALUATE, 'if review.tree != now && needs_fix && !fixed {', 'if false {'),
    ('a review with no blocking finding needs no fixed finding', EVALUATE, 'if review.tree != now && needs_fix && !fixed {', 'if review.tree != now && !fixed {'),
    ('a v0 review with no finding still needs a fixed finding after an edit', EVALUATE, '    let needs_fix = schema.is_none()\n        ||', '    let needs_fix = false\n        ||'),
    ('review verification first', EVALUATE, 'fn review(records: &[ContextRecord], input: &InitialInput, tree: TreeIdentity) -> Gate {\n    verification(records, input, tree)?;\n', 'fn review(records: &[ContextRecord], input: &InitialInput, tree: TreeIdentity) -> Gate {\n'),
    ('downstream approval', EVALUATE, '    require_approved(records, &id, &intent)?;\n    if intent.hard {', '    if intent.hard {'),
    ('downstream plan', EVALUATE, '    if intent.hard {\n        plan(records, input)?;\n    }', ''),
    ('first well-formed record by sequence, not order', EVALUATE, '    candidates.sort_by_key(|r| r.sequence);\n', ''),
    ('first well-formed record is authoritative', EVALUATE, 'candidates.sort_by_key(|r| r.sequence);', 'candidates.sort_by_key(|r| std::cmp::Reverse(r.sequence));'),
    ('verify tree stability', RUN, 'if trees.windows(2).all(|pair| pair[0] == pair[1]) {', 'if true {'),
    ('verify kills the process group', RUN, '    kill_group(pid);\n    let mut tail', '    let mut tail'),
    ('verify entry tree', RUN, 'let mut trees = vec![tree_identity(&input)?];', 'let mut trees = Vec::new();'),
    ('repo is the worktree root', RUN, 'if worktree_root(&given)? != given {', 'if false {'),
    ('setup uses the worktree root', PACKET, '    let repo = worktree_root(&repo).map_err(|e| format!("--repo: {e}"))?;\n', ''),
    ('packet keeps every tracked path', PACKET, 'std::fs::copy(real_index.trim(), &index)', 'Err::<u64, _>(std::io::Error::from(std::io::ErrorKind::NotFound))'),
    ('packet clears assume-unchanged', PACKET, '.and_then(|_| clear("--no-assume-unchanged"))', ''),
    ('packet clears skip-worktree', PACKET, '.and_then(|_| clear("--no-skip-worktree"))', ''),
    ('intent packet carries the principles', PACKET, '        principles = principles.text,\n        intent =', '        principles = "",\n        intent ='),
    ('packet eligibility', PACKET, 'verification_ready(&records, &input, &tree_identity)', 'Ok::<(), serde_json::Value>(())'),
    ('c has no review state', WORKFLOW, 'if review {\n        states.push', 'if true {\n        states.push'),
    ('old-vs-new intent is checked at intent-ready', EVALUATE, '.check_old(input.old_vs_new.is_some())', '.check_old(false)'),
    ('old is required in an old-vs-new profile', MODEL, '(None, true) => {', '(None, true) if false => {'),
    ('a 0.31.0 profile refuses old', MODEL, '(Some(_), false) => {', '(Some(_), false) if false => {'),
    ('none needs a nonblank reason', MODEL, 'is_none_or(|r| r.trim().is_empty()) =>', 'is_none_or(|r| r.trim().is_empty()) && false =>'),
    ('fails needs two old runs', MODEL, 'Expect::Fails => 2,', 'Expect::Fails => 1,'),
    ('a command that is not found is not a real failure', MODEL, '!matches!(code, 0 | 126 | 127)', '!matches!(code, 0 | 126)'),
    ('fails refuses a flaky pair', EVALUATE, 'if first.exit_code == second.exit_code {', 'if true {'),
    ('absent refuses no pass', EVALUATE, '.position(|run| run.exit_code == Some(0) && !run.timed_out)', '.position(|run| false)'),
    ('same compares the output', EVALUATE, 'if run.output_tail == current.output_tail {', 'if true {'),
    ('verified judges the expectation the record names', EVALUATE, 'let expect = expectation.expect;', 'let expect = current.old.as_ref().map_or(expectation.expect, |o| o.expect);'),
    ('old tree gets the proof files', RUN, 'lay_over_proof(&tree.root, repo, proof_paths)?;', 'let _ = (repo, proof_paths);'),
    ('old tree is removed after the runs', RUN, 'let _ = std::fs::remove_dir_all(&self.root);', 'let _ = &self.root;'),
    ('old tree makes a proof directory the base lacks', RUN, 'create_parent(&target).and_then(|()| std::fs::copy(&source, &target).map(|_| ()))', 'std::fs::copy(&source, &target).map(|_| ())'),
    ('a none criterion needs its old record', EVALUATE, 'let Some(old) = current.old.as_ref() else {', 'let Some(old) = current.old.as_ref() else { if expect == Expect::NotApplicable { return None; }'),
    ('verified refuses an old record that names another expectation', EVALUATE, 'let named = (old.expect != expect)', 'let named = (old.expect != expect && false)'),
    ('old tree removes the base files the repository no longer has', RUN, 'for name in base.difference(&current) {', 'for name in base.difference(&current).take(0) {'),
    ('old tree clears a directory the current file replaces', RUN, 'clear_target(&target)?;', 'remove_file_if_present(&target)?;'),
    ('old tree clears the nested directories a current file replaces', RUN, 'collect_directories(&path, dirs)?;\n', ''),
    ('old tree deletes a file left in a directory a current file replaces', RUN, 'if !is_dir {\n', 'if !is_dir {\n            std::fs::remove_file(&path).map_err(|e| e.to_string())?;\n            continue;\n        }\n        if false {\n'),
    ('proof path validation sees the path without its magic', MODEL, 'let (rest, top) = pathspec_after_magic(path);', 'let (rest, top) = (path, false);'),
    ('proof path validation reads past a short magic terminator', MODEL, "magic.strip_prefix(':').unwrap_or(magic)", 'magic'),
    ('setup writes old_vs_new', PACKET, 'input["old_vs_new"] = json!({"proof_paths": proof_paths});', ''),
    ('review packet lists old runs only where there are some', PACKET, '.filter(|old| !old.runs.is_empty())?;', '?;'),
    ('describe tells the driver about old', WORKFLOW, 'if old_vs_new {', 'if false {'),
    ('a v1 review answers the seventh question', MODEL, '    if findings_schema.is_some() {\n        &QUESTIONS', '    if false {\n        &QUESTIONS'),
    ('a review without findings_schema keeps the original six questions', MODEL, '    if findings_schema.is_some() {\n        &QUESTIONS', '    if true {\n        &QUESTIONS'),
    ('a v1 finding needs a severity', EVALUATE, 'if findings.iter().any(|f| f.severity.is_none()) {', 'if false {'),
    ('a record without findings_schema refuses head and brief_sha256', EVALUATE, '    if has_envelope_fields && findings_schema.is_none() {', '    if false {'),
    ('only schema 1 is accepted', EVALUATE, 'if findings_schema.is_some_and(|schema| schema != 1) {', 'if false {'),
    ('a blocking finding missing a field is demoted', MODEL, '        if missing.is_empty() {\n            Effective::Blocking', '        if true {\n            Effective::Blocking'),
    ('a note needs a disposition', EVALUATE, '        .filter(|f| f.effective(findings_schema) == Effective::Blocking)\n    {', '        .filter(|_| true)\n    {'),
    ('delegation coverage check', EVALUATE, '    if missing.is_empty() && extra.is_empty() {\n        return Ok(());', '    if true {\n        return Ok(());'),
    ('delegation outcome comparison', EVALUATE, '    if !differs.is_empty() {\n        return Outcome::Changed(differs);', '    if false {\n        return Outcome::Changed(differs);'),
    ('delegation rewrites are checked', EVALUATE, '    if problems == RewriteProblems::default() {\n        Outcome::Keeps', '    if true {\n        Outcome::Keeps'),
    ('a refused delegation never counts as a baseline', EVALUATE, '        if valid {\n', '        if valid || true {\n'),
]


def run(args, cwd, **kw):
    return subprocess.run(args, cwd=cwd, text=True, capture_output=True, **kw)


def copy_source(work):
    """Copy the tracked and non-ignored files of this checkout; mutants never touch the checkout or its target."""
    names = run(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], ROOT, check=True).stdout
    source = work / 'source'
    for name in filter(None, names.split('\0')):
        origin = ROOT / name
        if not origin.is_file():
            continue
        destination = source / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(origin, destination, follow_symlinks=False)
    return source


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--engine')
    parser.add_argument('--list', action='store_true')
    parser.add_argument('--work-root', help='driver-owned directory outside the checkout (default: a fresh temporary directory, removed after)')
    args = parser.parse_args()
    # Bare invocation is the recognized CI collection form; the full proof rebuilds the provider once per mutation.
    if args.list or len(sys.argv) == 1:
        for name, path, old, new in MUTATIONS:
            assert path.read_text().count(old) == 1, f'stale mutation: {name}'
        print(f'{len(MUTATIONS)} lean-change mutations are applicable')
        return
    if not args.engine:
        parser.error('--engine is required')
    engine = Path(args.engine).resolve()
    if args.work_root:
        work = Path(args.work_root).resolve()
        assert not work.is_relative_to(ROOT), '--work-root must be outside the checkout'
        work.mkdir(parents=True, exist_ok=True)
        work = Path(tempfile.mkdtemp(prefix='lean-change-mutations-', dir=work))
    else:
        work = Path(tempfile.mkdtemp(prefix='lean-change-mutations-'))
    try:
        survivors = mutate_all(engine, work)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    assert not survivors, f'mutations not caught: {survivors}'
    print('every lean-change gate mutation was caught')


def mutate_all(engine, work):
    # Mutants are built from a disposable copy into a disposable target directory, so the
    # checkout's sources and its target/debug/lean-change are never altered, even on interruption.
    source = copy_source(work)
    env = dict(os.environ, CARGO_TARGET_DIR=str(work / 'target'))
    provider = work / 'target/debug/lean-change'
    survivors = []
    for name, path, old, new in MUTATIONS:
        copy = source / path.relative_to(ROOT)
        original = copy.read_text()
        assert original.count(old) == 1, f'stale mutation: {name}'
        try:
            copy.write_text(original.replace(old, new))
            build = run(['cargo', 'build', '-p', 'lean-change-provider', '--locked'], source, env=env)
            if build.returncode:
                print(f'{name}: DOES NOT COMPILE\n{build.stderr[-800:]}')
                survivors.append(name)
                continue
            unit = run(['cargo', 'test', '-p', 'lean-change-provider', '--locked', '--quiet'], source, env=env)
            if unit.returncode:
                print(f'{name}: killed by unit tests')
                continue
            journey = run([sys.executable, 'scripts/lean-change-journey.py', '--engine', str(engine), '--provider', str(provider)], source, env=env)
            if journey.returncode:
                print(f'{name}: killed by the journey')
                continue
            print(f'{name}: SURVIVED')
            survivors.append(name)
        finally:
            copy.write_text(original)
    return survivors


if __name__ == '__main__':
    main()
