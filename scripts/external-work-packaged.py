#!/usr/bin/env python3
"""Build controlled native package inputs and run both real packaged smokes."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile

ROOT = Path(__file__).resolve().parents[1]
APPS = {'loop-cli': 'engine', 'software-change-provider': 'software',
        'policy-document-provider': 'policy', 'research-provider': 'research'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in APPS.values():
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--output-root', required=True, type=Path)
    args = parser.parse_args()
    root = args.output_root.resolve()
    if not args.output_root.is_absolute() or root == ROOT or ROOT in root.parents:
        parser.error('output-root must be absolute and outside checkout')
    root.mkdir(parents=True, exist_ok=False)
    target = {('Darwin', 'arm64'): 'aarch64-apple-darwin',
              ('Linux', 'x86_64'): 'x86_64-unknown-linux-gnu'}[(platform.system(), platform.machine())]
    version = subprocess.check_output([args.engine.resolve(), '--version'], text=True).strip()
    installed, archives = {}, {}
    for app, argument in APPS.items():
        source = getattr(args, argument).resolve()
        destination = root / 'installed' / app / source.name
        destination.parent.mkdir(parents=True)
        shutil.copy2(source, destination)
        installed[app] = destination
        archive = root / f'{app}-{target}.tar.xz'
        with tarfile.open(archive, 'w:xz') as package:
            package.add(destination, arcname=f'{app}-{target}/{source.name}')
        archives[app] = archive
    commands = []
    for mode, inputs in [('installed', installed), ('archive', archives)]:
        hashes = {app: hashlib.sha256(path.read_bytes()).hexdigest() for app, path in inputs.items()}
        identity = root / f'{mode}-identity.json'
        identity.write_text(json.dumps(dict(version=version, platform=target, sha256=hashes), indent=2))
        argv = [sys.executable, str(ROOT / 'scripts/packaged-smoke.py'), '--mode', mode,
                '--expected-version', version, '--platform', target, '--output-root', str(root / mode),
                '--package-identity', str(identity)]
        # Keep installation inputs separate from the smoke output root.
        if mode == 'installed':
            argv[argv.index('--output-root') + 1] = str(root / 'installed-smoke')
        for app, path in inputs.items():
            argv += ['--archive' if mode == 'archive' else '--binary', f'{app}={path}']
            if mode == 'archive':
                (Path(str(path) + '.sha256')).write_text(f'{hashes[app]}  {path.name}\n')
                argv += ['--checksum', f'{app}={hashes[app]}']
        with (root / f'{mode}.stdout').open('w') as out, (root / f'{mode}.stderr').open('w') as err:
            result = subprocess.run(argv, stdout=out, stderr=err)
        commands.append(dict(argv=argv, exit_code=result.returncode))
        (root / 'commands.json').write_text(json.dumps(commands, indent=2))
        if result.returncode:
            return result.returncode
    return 0


if __name__ == '__main__':
    sys.exit(main())
