"""Driver-owned public-process and external reviewer recording helpers."""
import json
import subprocess


def process(argv, packet=None, *, env=None, cwd=None):
    result = subprocess.run([str(arg) for arg in argv], input=None if packet is None else json.dumps(packet),
                            text=True, capture_output=True, env=env, cwd=cwd)
    if result.returncode:
        raise RuntimeError(f"{argv}: {result.returncode}: {result.stdout}\n{result.stderr}")
    return json.loads(result.stdout)


def engine_call(engine, database, config, args):
    return process([engine, '--database', database, '--config', config, '--json', *args])


def record_reviews(engine, database, config, provider, run_id, gate, policies,
                   subject, revision):
    """Intake scripted judgments, inspect them, then explicitly append candidates."""
    shown = engine_call(engine, database, config, ['show', '--view', 'full', run_id])
    for index, policy in enumerate(policies):
        author = {'name': f'external-{gate}-{index}', 'kind': 'script'}
        assignment = {'gate': gate, 'review_stage': 'ordinary', 'subject': subject,
                      'subject_revision': revision, 'policy_ids': [policy]}
        item = {'policy_id': policy, 'result': 'pass', 'findings': ''}
        packet = {'show': shown, 'assignment': assignment, 'author': author,
                  'request': {'text': f'Review {subject} for {policy}.'},
                  'result': {'text': json.dumps({'items': [item]})}}
        intake = process([provider, 'review-candidates'], packet)
        candidates = intake.get('candidates', [])
        if not candidates:
            raise RuntimeError(f'No ready external review candidates: {intake}')
        for candidate in candidates:
            evidence = candidate['data']
            if 'gate' not in evidence:
                raise RuntimeError(f'Unexpected candidate shape: {candidate}')
            response = engine_call(engine, database, config,
                                   ['append', '--record-id', f'{gate}-{index}', run_id,
                                    'review-evidence', json.dumps(evidence)])
            if response.get('status') != 'completed':
                raise RuntimeError(f'Could not record external evidence: {response}')
    return shown
