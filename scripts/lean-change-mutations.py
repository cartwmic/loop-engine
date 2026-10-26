#!/usr/bin/env python3
"""Mutation check: disable each lean-change gate in turn and require the unit tests or the journey to fail."""
import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EVALUATE = ROOT / 'crates/lean-change-provider/src/evaluate.rs'
WORKFLOW = ROOT / 'crates/lean-change-provider/src/workflow.rs'
RUN = ROOT / 'crates/lean-change-provider/src/run.rs'
PACKET = ROOT / 'crates/lean-change-provider/src/packet.rs'

# (name, file, original text, mutated text). Each original must occur exactly once.
MUTATIONS = [
    ('intent gate', EVALUATE, '("explore", "intent-ready") => current_intent(records).map(|_| ()),', '("explore", "intent-ready") => Ok(()),'),
    ('owner decision', EVALUATE, 'match evaluation.status {\n        Status::Approved => {}', 'match evaluation.status {\n        _ => {}\n        Status::Approved => {}'),
    ('hard routing', EVALUATE, 'if (event == "approved-hard") != intent.hard {', 'if false {'),
    ('plan gate', EVALUATE, 'fn plan(records: &[ContextRecord]) -> Gate {\n', 'fn plan(records: &[ContextRecord]) -> Gate {\n    if true {\n        return Ok(());\n    }\n'),
    ('verification stale', EVALUATE, 'if v.tree != now {', 'if false {'),
    ('verification missing criterion', EVALUATE, 'None => problems.push(json!({"criterion": criterion.id, "problem": "missing: no check result"})),', 'None => {}'),
    ('verification failed check', EVALUATE, 'Some(r) if r.timed_out || r.exit_code != Some(0) =>', 'Some(r) if false =>'),
    ('verification approved check', EVALUATE, 'Some(r) if r.check_sha256 != criterion.check.sha256() =>', 'Some(r) if false =>'),
    ('verification of current intent', EVALUATE, 'match recorded.filter(|(_, v)| v.intent == id) {', 'match recorded {'),
    ('review independence', EVALUATE, '        || review\n            .reviewer\n            .name\n            .trim()\n            .eq_ignore_ascii_case(intent.author.name.trim())\n', ''),
    ('review fresh context', EVALUATE, 'if !review.fresh_context {', 'if false {'),
    ('review all questions', EVALUATE, 'QUESTIONS.iter().find(|(q, _)| !asked.contains(q))', 'QUESTIONS.iter().find(|(q, _)| false && !asked.contains(q))'),
    ('review failed question needs finding', EVALUATE, 'if !review\n            .findings\n            .iter()\n            .any(|f| f.question == answer.question)\n        {', 'if false {'),
    ('review finding fields', EVALUATE, 'if fields.iter().any(|f| f.trim().is_empty())\n            ||', 'if false\n            ||'),
    ('review disposition', EVALUATE, '            _ => {\n                return Err(incomplete(format!(\n                    "finding {} needs a disposition (fixed or rejected) with a reason",\n                    finding.id\n                )))\n            }', '            _ => {}'),
    ('review tree change', EVALUATE, 'if review.tree != now && !fixed {', 'if false {'),
    ('review verification first', EVALUATE, 'fn review(records: &[ContextRecord], input: &InitialInput, tree: TreeIdentity) -> Gate {\n    verification(records, input, tree)?;\n', 'fn review(records: &[ContextRecord], input: &InitialInput, tree: TreeIdentity) -> Gate {\n'),
    ('downstream approval', EVALUATE, '    require_approved(records, &id)?;\n    if intent.hard {', '    if intent.hard {'),
    ('downstream plan', EVALUATE, '    if intent.hard {\n        plan(records)?;\n    }', ''),
    ('first review is authoritative', EVALUATE, 'reviews.sort_by_key(|r| r.sequence);', 'reviews.sort_by_key(|r| std::cmp::Reverse(r.sequence));'),
    ('verify tree stability', RUN, 'if trees.windows(2).all(|pair| pair[0] == pair[1]) {', 'if true {'),
    ('verify kills the process group', RUN, '    kill_group(pid);\n    let mut tail', '    let mut tail'),
    ('verify entry tree', RUN, 'let mut trees = vec![tree_identity(&input)?];', 'let mut trees = Vec::new();'),
    ('packet eligibility', PACKET, 'verification_ready(&records, &input, &tree_identity)', 'Ok::<(), serde_json::Value>(())'),
    ('c has no review state', WORKFLOW, 'if review {\n        states.push', 'if true {\n        states.push'),
]


def run(args, **kw):
    return subprocess.run(args, cwd=ROOT, text=True, capture_output=True, **kw)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--engine')
    parser.add_argument('--list', action='store_true')
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
    target = json.loads(run(['cargo', 'metadata', '--format-version', '1', '--no-deps'], check=True).stdout)['target_directory']
    provider = Path(target) / 'debug/lean-change'
    survivors = []
    for name, path, old, new in MUTATIONS:
        original = path.read_text()
        assert original.count(old) == 1, f'stale mutation: {name}'
        try:
            path.write_text(original.replace(old, new))
            build = run(['cargo', 'build', '-p', 'lean-change-provider', '--locked'])
            if build.returncode:
                print(f'{name}: DOES NOT COMPILE\n{build.stderr[-800:]}')
                survivors.append(name)
                continue
            unit = run(['cargo', 'test', '-p', 'lean-change-provider', '--locked', '--quiet'])
            if unit.returncode:
                print(f'{name}: killed by unit tests')
                continue
            journey = run([sys.executable, 'scripts/lean-change-journey.py', '--engine', str(engine), '--provider', str(provider)])
            if journey.returncode:
                print(f'{name}: killed by the journey')
                continue
            print(f'{name}: SURVIVED')
            survivors.append(name)
        finally:
            path.write_text(original)
    assert not survivors, f'mutations not caught: {survivors}'
    print('every lean-change gate mutation was caught')


if __name__ == '__main__':
    main()
