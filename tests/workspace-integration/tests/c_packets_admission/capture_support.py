"""Public-CLI fixture assertions and retained subprocess output for CI tests."""
import hashlib
import json
import os
from pathlib import Path
import subprocess

LABEL = 'Scripted fixture judgments; no model or human review claim'

def require(condition, detail):
    if not condition:
        raise AssertionError(str(detail))

def sha(data):
    return hashlib.sha256(data).hexdigest()

def dump(path, data):
    with Path(path).open('x', encoding='utf-8') as stream:
        json.dump(data, stream, indent=2, ensure_ascii=False)
        stream.write('\n')

class Capture:
    def __init__(self, root, args):
        self.root = root
        root.mkdir()
        self.index = root / 'commands.jsonl'
        self.rows = []
        self.args = args
        self.environment = {k:v for k,v in os.environ.items()
                            if not k.startswith(('LOOP_', 'XDG_', 'GIT_', 'BOOKENDS_'))}
        self.environment.update(PYTHONDONTWRITEBYTECODE='1', GIT_CONFIG_NOSYSTEM='1',
            GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull,
            GIT_TEMPLATE_DIR=str(root / 'git-template'))
        (root / 'git-template').mkdir()

    def execute(self, argv, packet=None, *, cwd=None, raw=False):
        argv = list(map(str, argv))
        cwd = Path(cwd or self.args.source_root).resolve()
        data = None if packet is None else json.dumps(packet).encode()
        result = subprocess.run(argv, input=data, cwd=cwd, env=self.environment,
                                capture_output=True, timeout=90)
        number = f'{len(self.rows)+1:06d}'
        def retain(name, value):
            if value is None:
                return None
            path = self.root / f'{number}.{name}'
            path.write_bytes(value)
            return {'path':path.name, 'sha256':sha(value), 'bytes':len(value)}
        row = {'command_id':number, 'argv':argv, 'cwd':str(cwd),
               'exit_code':result.returncode, 'stdin':retain('stdin', data),
               'stdout':retain('stdout', result.stdout), 'stderr':retain('stderr', result.stderr)}
        with self.index.open('a') as stream:
            stream.write(json.dumps(row)+'\n')
        self.rows.append(row)
        result = subprocess.CompletedProcess(argv, result.returncode,
                    result.stdout.decode(), result.stderr.decode())
        if raw:
            return result
        value = json.loads(result.stdout)
        require(result.returncode == 0 or value.get('status') == 'rejected', row)
        return value

    def finalize(self):
        if not (self.root / 'commands.json').exists():
            dump(self.root / 'commands.json', {'command_count':len(self.rows)})

def snapshot(ctx):
    return {'show': ctx['show'](), 'history': ctx['call']('history', ctx['run_id'])}

def invariant(before, after):
    a, b = before['show']['result'], after['show']['result']
    for key in ('context', 'current_state', 'state_visit', 'initial_input', 'lifecycle', 'has_overrides'):
        require(a[key] == b[key], f'Unexpected mutation of {key}')
    require(not b['has_overrides'], 'Override is never fixture clearance')

def checked_allow(before, response, after):
    require(response['status'] == 'completed', response)
    old, new = before['show']['result'], after['show']['result']
    require(old['current_state'] != new['current_state'] and new['state_visit'] > old['state_visit']
            and not new['has_overrides'], after)
    prior, following = before['history']['result'], after['history']['result']
    require(following[:len(prior)] == prior and len(following) > len(prior), after['history'])
    transitions = [r['action'] for r in following[len(prior):] if r['action']['kind'] == 'transition']
    require(len(transitions) == 1, transitions)
    action = transitions[0]
    require(action['outcome']['outcome'] == 'committed' and
            action['transition']['kind'] == 'checked' and
            action['transition']['source'] == old['current_state'] and
            action['transition']['target'] == new['current_state'], action)
    old_records = {r['id']: r for r in old['context']}
    new_records = {r['id']: r for r in new['context']}
    require(all(new_records.get(k) == v for k, v in old_records.items()), 'Transition lost originals')

def groups(policies):
    selected = {}
    for p in policies:
        selected.setdefault((p['review_stage'], p['group']), []).append(p)
    return sorted(selected.items(), key=lambda pair: (pair[0][0] != 'individual', pair[0]))
