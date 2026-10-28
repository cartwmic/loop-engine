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

# (name, file, original text, mutated text). Each original must occur exactly once.
MUTATIONS = [
    ('intent gate', EVALUATE, '("explore", "intent-ready") => intent_ready(records, &input),', '("explore", "intent-ready") => current_intent(records).map(|_| ()),'),
    ('intent check required for b', EVALUATE, '    if !input.review {\n        return Ok(());', '    if true {\n        return Ok(());'),
    ('intent check not required for c', EVALUATE, '    if !input.review {\n        return Ok(());', '    if false {\n        return Ok(());'),
    ('intent check names an intent record', EVALUATE, '.find(|r| r.kind == "intent" && r.id.as_str() == check.intent)', '.find(|r| r.kind == "intent")'),
    ('intent check has its own questions', EVALUATE, '        &INTENT_QUESTIONS,\n', '        &QUESTIONS,\n'),
    ('intent check disposition', EVALUATE, 'if disposed(records, check_id, &check.findings).map_err(incomplete)? &&', 'if false &&'),
    ('intent check fix needs a newer intent', EVALUATE, '? && check.intent == id {', '? && false {'),
    ('b asks for the intent check', WORKFLOW, 'if review {\n        explore.push_str(', 'if false {\n        explore.push_str('),
    ('c asks for no intent check', WORKFLOW, 'if review {\n        explore.push_str(', 'if true {\n        explore.push_str('),
    ('owner decision', EVALUATE, 'match evaluation.status {\n        Status::Approved => {}', 'match evaluation.status {\n        _ => {}\n        Status::Approved => {}'),
    ('hard routing', EVALUATE, 'if (event == "approved-hard") != intent.hard {', 'if false {'),
    ('plan gate', EVALUATE, 'fn plan(records: &[ContextRecord]) -> Gate {\n', 'fn plan(records: &[ContextRecord]) -> Gate {\n    if true {\n        return Ok(());\n    }\n'),
    ('verification stale', EVALUATE, 'if v.tree != now {', 'if false {'),
    ('verification missing criterion', EVALUATE, 'None => problems.push(json!({"criterion": criterion.id, "problem": "missing: no check result"})),', 'None => {}'),
    ('verification failed check', EVALUATE, 'Some(r) if r.timed_out || r.exit_code != Some(0) =>', 'Some(r) if false =>'),
    ('verification approved check', EVALUATE, 'Some(r) if r.check_sha256 != criterion.check.sha256() =>', 'Some(r) if false =>'),
    ('verification of current intent', EVALUATE, 'match recorded.filter(|(_, v)| v.intent == id) {', 'match recorded {'),
    ('independence of reviewers', EVALUATE, '        || reviewer\n            .name\n            .trim()\n            .eq_ignore_ascii_case(intent.author.name.trim())\n', ''),
    ('fresh context of reviewers', EVALUATE, 'if !fresh_context {', 'if false {'),
    ('all questions answered', EVALUATE, 'questions.iter().find(|(q, _)| !asked.contains(q))', 'questions.iter().find(|(q, _)| false && !asked.contains(q))'),
    ('failed question needs finding', EVALUATE, 'if !findings.iter().any(|f| f.question == answer.question) {', 'if false {'),
    ('finding fields', EVALUATE, 'if fields.iter().any(|f| f.trim().is_empty())\n            ||', 'if false\n            ||'),
    ('finding disposition', EVALUATE, '            _ => {\n                return Err(format!(\n                    "finding {} needs a disposition (fixed or rejected) with a reason",\n                    finding.id\n                ))\n            }', '            _ => {}'),
    ('review tree change', EVALUATE, 'if review.tree != now && !fixed {', 'if false {'),
    ('review verification first', EVALUATE, 'fn review(records: &[ContextRecord], input: &InitialInput, tree: TreeIdentity) -> Gate {\n    verification(records, input, tree)?;\n', 'fn review(records: &[ContextRecord], input: &InitialInput, tree: TreeIdentity) -> Gate {\n'),
    ('downstream approval', EVALUATE, '    require_approved(records, &id)?;\n    if intent.hard {', '    if intent.hard {'),
    ('downstream plan', EVALUATE, '    if intent.hard {\n        plan(records)?;\n    }', ''),
    ('first well-formed record by sequence, not order', EVALUATE, '    candidates.sort_by_key(|r| r.sequence);\n', ''),
    ('first well-formed record is authoritative', EVALUATE, 'candidates.sort_by_key(|r| r.sequence);', 'candidates.sort_by_key(|r| std::cmp::Reverse(r.sequence));'),
    ('verify tree stability', RUN, 'if trees.windows(2).all(|pair| pair[0] == pair[1]) {', 'if true {'),
    ('verify kills the process group', RUN, '    kill_group(pid);\n    let mut tail', '    let mut tail'),
    ('verify entry tree', RUN, 'let mut trees = vec![tree_identity(&input)?];', 'let mut trees = Vec::new();'),
    ('repo is the worktree root', RUN, 'if worktree_root(&given)? != given {', 'if false {'),
    ('setup uses the worktree root', PACKET, '    let repo = worktree_root(&repo).map_err(|e| format!("--repo: {e}"))?;\n', ''),
    ('packet keeps every tracked path', PACKET, 'std::fs::copy(real_index.trim(), &index)', 'Err::<u64, _>(std::io::Error::from(std::io::ErrorKind::NotFound))'),
    ('packet eligibility', PACKET, 'verification_ready(&records, &input, &tree_identity)', 'Ok::<(), serde_json::Value>(())'),
    ('c has no review state', WORKFLOW, 'if review {\n        states.push', 'if true {\n        states.push'),
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
