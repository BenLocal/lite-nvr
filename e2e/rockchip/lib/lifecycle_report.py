#!/usr/bin/env python3
"""Record build/deployment/outer-cleanup failures even before the suite starts."""
import json
import os
import pathlib
import re
import sys

status, phase, directory = sys.argv[1:]
directory = pathlib.Path(directory)
path = directory / 'report.json'
if path.exists():
    report = json.loads(path.read_text())
    report['lifecycle_error'] = dict(phase=phase, exit_code=int(status))
    if not any(r['status'] == 'FAIL' for r in report['results']):
        report['results'].append(dict(id=phase, commit=report['commit'], status='FAIL', detail=f'Lifecycle exit={status}; inspect cleanup/build logs'))
        report['summary']['FAIL'] += 1
        print(f'Bail out! {phase} failed, exit={status}')
else:
    def safe(text):
        return re.sub(r'("(?:token|password)"\s*:\s*")[^"]*', r'\1<redacted>', text)
    log = directory / 'failure-board.log'
    tail = safe(log.read_text() if log.exists() else 'Board log unavailable before deployment')
    response = ''
    for name in ['start.log', 'tunnel.log', 'build.log']:
        candidate = directory / name
        if candidate.exists():
            response = safe(candidate.read_text(errors='replace')[-500:])
            break
    result = dict(id=phase, commit=os.environ.get('COMMIT', 'unknown'), status='FAIL',
                  detail=f'Expected successful {phase}, actual exit={status}; see artifact logs',
                  response=response, log_tail=tail)
    report = dict(commit=result['commit'], results=[result], summary={'PASS': 0, 'FAIL': 1, 'SKIP': 0})
    print(f'TAP version 13\nnot ok 1 - {phase}\n1..1\n# PASS 0 FAIL 1 SKIP 0')
path.write_text(json.dumps(report, indent=2, ensure_ascii=False))
