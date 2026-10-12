#!/usr/bin/env python3
"""Bind explicitly reused packages to the current source and package bytes."""
import hashlib
import json
import pathlib
import subprocess
import sys


def source_digest(repo):
    digest = hashlib.sha256()
    names = subprocess.check_output(['git', '-C', str(repo), 'ls-files', '--cached', '--others', '--exclude-standard', '-z']).split(b'\0')
    suffixes = {'.rs', '.toml', '.lock', '.vue', '.ts', '.js', '.json', '.css', '.scss', '.sh', '.c', '.cpp', '.h'}
    for name in sorted(n for n in names if n):
        path = pathlib.Path(name.decode())
        if path.parts[0] in ('e2e', 'docs') or (path.suffix not in suffixes and path.name != 'Makefile'):
            continue
        digest.update(name + b'\0')
        digest.update((repo / path).read_bytes())
    return digest.hexdigest()


def main():
    if not __debug__:
        raise SystemExit('Provenance checks require Python without -O/PYTHONOPTIMIZE')
    if sys.argv[1] == 'source':
        print(source_digest(pathlib.Path(sys.argv[2])))
        return
    action, package, repo, commit = sys.argv[1:]
    package, repo = pathlib.Path(package), pathlib.Path(repo)
    metadata = package.with_suffix(package.suffix + '.build.json')
    data = dict(commit=commit, source_digest=source_digest(repo), sha256=hashlib.sha256(package.read_bytes()).hexdigest(), package=package.name, feature='rockchip')
    if action == 'write':
        metadata.write_text(json.dumps(data, indent=2))
    else:
        assert action == 'verify'
        assert metadata.is_file(), 'Package lacks build provenance; rebuild instead of using an old dist package'
        assert json.loads(metadata.read_text()) == data, 'Package SHA256, commit or source does not match; rebuild'
    print(metadata)


if __name__ == '__main__':
    main()
