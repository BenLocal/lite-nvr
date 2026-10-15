#!/usr/bin/env python3
"""Run API/media regressions against an existing instance without redeploying it."""

import argparse
import json
import os
import pathlib
import shlex
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / 'lib'))
sys.path.insert(0, str(HERE / 'host'))
from deploy import Runner
from suite import Suite


def identity(runner):
    root = shlex.quote(runner.install + '/bin/nvr')
    return runner.board(
        f'for p in /proc/[0-9]*; do '
        f'if [ "$(readlink "$p/exe" 2>/dev/null)" = {root} ]; then '
        'echo "pid=${p#/proc/}"; sha256sum "$p/exe"; fi; done'
    ).strip()


def configuration(devices):
    return {d['id']: {k: v for k, v in d.items() if k not in ('state', 'flv_url')} for d in devices}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--install-dir', required=True, help='Existing board installation')
    parser.add_argument('--group', choices=['api', 'media', 'all'], default='api')
    args = parser.parse_args()
    runner = Runner(HERE)
    suite = None
    before = None
    status = 1
    try:
        runner.setup()
        runner.install = args.install_dir
        original = identity(runner)
        if not original:
            raise RuntimeError('No running NVR found at the supplied installation')
        runner.remote = '/userdata/lite-nvr-live-e2e-' + runner.run_id
        remote = shlex.quote(runner.remote)
        runner.board(f'mkdir {remote} && mkdir {remote}/board {remote}/results')
        runner.copy(HERE / 'board/source.sh', runner.remote + '/board/')
        runner.connect()
        # These nodes/outputs may already belong to the running instance.
        os.environ['RK_HDMI_LOOPBACK'] = '0'
        os.environ.pop('RK_VIDEO_DEVICE', None)
        os.environ.pop('RK_NO_SIGNAL_DEVICE', None)
        (runner.artifacts / 'runtime.json').write_text(
            json.dumps(
                dict(identity=original, suite_commit=runner.commit, mode='existing-instance'),
                indent=2,
            )
        )
        suite = Suite()
        suite.login()
        before = configuration(suite.list())
        suite.test('live.running', 'runtime', lambda: suite.api('GET', '/api/system/os'))
        if args.group in ('api', 'all'):
            suite.api_cases()
        if args.group in ('media', 'all'):
            suite.media_cases()
        runner.check_suite_unchanged()
        status = 0
    finally:
        try:
            if suite is not None:
                suite.test('cleanup', 'runtime', suite.cleanup, check_board=False)
                if before is not None:

                    def unchanged():
                        assert configuration(suite.list()) == before, 'Device configuration changed'
                        assert identity(runner) == original, 'NVR process or binary changed'

                    # cleanup logged out; use a fresh session to verify persistent state.
                    suite.token = ''
                    suite.login()
                    try:
                        suite.test('live.unchanged', 'runtime', unchanged)
                    finally:
                        suite.api('POST', '/api/user/logout')
                        suite.token = ''
                suite.write_report()
                print(
                    json.dumps(json.loads((suite.artifacts / 'report.json').read_text())['summary'])
                )
                if any(r['status'] == 'FAIL' for r in suite.results):
                    status = 1
        finally:
            # Runner.stop normally stops its deployed service. This entry owns only a tunnel.
            runner.install = None
            try:
                runner.stop()
            finally:
                runner.unlock()
    return status


if __name__ == '__main__':
    raise SystemExit(main())
