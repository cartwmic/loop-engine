"""Repository-owned C public contract tests, using only explicitly supplied binaries."""
import argparse
import importlib.util
import json
from pathlib import Path
import sys

from capture_support import Capture, require


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def compatibility(args):
    """Providers without admission keep accepting engine-owned opaque context."""
    log = Capture(args.output_root / 'commands', args)
    for alias, binary, profile in (
        ('policy-document', args.policy, 'policy-document-provider/data/readme.json'),
        ('research', args.research, 'research-provider/data/configs/standard.json'),
    ):
        root = args.output_root / alias
        root.mkdir()
        config = root / 'providers.toml'
        config.write_text(f'[providers.{alias}]\ncommand = {json.dumps(str(binary))}\nargs = []\n')
        data = json.loads((args.source_root / 'crates' / profile).read_text())
        artifacts = root / 'artifacts'
        artifacts.mkdir()
        data['artifact_root'] = str(artifacts)
        if alias == 'policy-document':
            target = root / 'README.md'
            target.write_text('# Fixture\n')
            data['target']['path'] = str(target)
        selected = root / 'profile.json'
        selected.write_text(json.dumps(data))
        base = [args.engine, '--database', root / 'run.sqlite', '--config', config, '--json']
        started = log.execute([*base, 'start', '--id', alias, alias, '@' + str(selected), 'compatibility fixture'])
        require(started['status'] == 'completed', started)
        shown = log.execute([*base, 'show', '--view', 'full', alias])
        require(not shown['result']['workflow_graph'].get('admit', False), shown)
        before = shown['result']['context']
        payload = root / 'note.json'
        payload.write_text(json.dumps({'fixture': alias}))
        added = log.execute([*base, 'append', '--record-id', 'opaque', alias, 'fixture-note', '@' + str(payload)])
        require(added['status'] == 'completed', added)
        after = log.execute([*base, 'show', '--view', 'full', alias])['result']['context']
        require(after[:-1] == before and after[-1]['data'] == {'fixture': alias}, after)
        require(log.execute([*base, 'terminate', alias])['status'] == 'completed', alias)
    log.finalize()


def guidance(args):
    """Successor-facing contracts remain discoverable in shipped guidance."""
    contracts = {
        'docs/agent-usage.md': ('fail closed', 'assessment_id', 'record:ACTUAL_CONTEXT_ID#JSON-Pointer', 'intent-correction', 'multiline',
                                'declared-baseline-to-covered-commit', 'no tool ban'),
        'crates/software-change-provider/skills/using-software-change-provider/SKILL.md': ('admission', 'assessment_id', 'review_record_sources', 'intent-correction',
                                                                                        'declared-baseline-to-covered-commit', 'no tool ban'),
        'crates/software-change-provider/README.md': ('review-candidates', 'prepare-validation'),
    }
    for name, terms in contracts.items():
        text = (args.source_root / name).read_text()
        for term in terms:
            require(term in text, f'Missing documented public contract {term!r} in {name}')


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    for flag in ('source-root', 'output-root', 'engine', 'provider', 'checker', 'policy', 'research'):
        parser.add_argument('--' + flag, required=True, type=Path)
    args = parser.parse_args()
    require(not sys.flags.optimize, 'Fixture assertions must be enabled')
    for key, value in vars(args).items():
        require(value.is_absolute(), key)
        setattr(args, key, value.resolve())
    require(not args.output_root.exists(), 'Fresh output directory required')
    require(args.source_root not in args.output_root.parents, 'Test catalog must be outside checkout')
    args.output_root.mkdir()
    sys.path.insert(0, str(args.source_root / 'scripts'))
    journey = load(args.source_root / 'scripts/software-change-journey.py', 'c_public_journey')
    external = load(args.source_root / 'scripts/external-work-journey.py', 'c_public_external')
    import review_cases
    import admission_cases
    from types import SimpleNamespace
    args.mode, args.data_root = 'source', str(args.source_root)
    cases = [
        'direct-admission', 'ac4-missing', 'ac4-inconsistent', 'ac4-mixed-authors',
        'ac5-review-steering', 'admission-metadata', 'correction-citations',
        'scope-change', 'overlay-checkpoint', 'no-admission-compatibility', 'shipped-guidance',
    ]
    for case in cases:
        selected = SimpleNamespace(**vars(args))
        selected.output_root = args.output_root / case
        selected.output_root.mkdir()
        print('START ' + case, flush=True)
        if case == 'direct-admission':
            result = external.admission_recovery(SimpleNamespace(engine=args.engine, software=args.provider), selected.output_root)
            require(result['status'] == 'passed' and result['observed_workflow_admit']
                    and len(result['cases']) == 6 and len(result['success_controls']) == 2, result)
        elif case.startswith(('ac4-', 'ac5-')):
            review_cases.run_case(selected, journey, case)
        elif case == 'no-admission-compatibility':
            compatibility(selected)
        elif case == 'shipped-guidance':
            guidance(selected)
        else:
            admission_cases.run_case(selected, journey, case)
        print('PASS ' + case, flush=True)
    print(json.dumps({'completed': cases, 'semantic_review': False}))


if __name__ == '__main__':
    main()
