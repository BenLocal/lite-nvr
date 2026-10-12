#!/usr/bin/env python3
"""Build and run isolated Rockchip e2e tests; all host orchestration is Python."""

import argparse
import os
import pathlib
import signal
import subprocess
import sys

if not __debug__:
    raise SystemExit('E2E assertions require Python without -O/PYTHONOPTIMIZE')

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / 'lib'))
sys.path.insert(0, str(HERE / 'host'))
from deploy import Runner
from mutations import CASES, run_mutation
from suite import EnvironmentFailure
from suite import main as run_suite


def lifecycle_failure(runner, status):
    if not runner.artifacts.exists():
        return
    if runner.install and not (runner.artifacts / 'report.json').exists():
        try:
            import shlex

            runner.board(
                'tail -n 20 ' + shlex.quote(runner.install + '/nvr.log'),
                log=runner.artifacts / 'failure-board.log',
            )
        except (OSError, RuntimeError, subprocess.TimeoutExpired):
            pass  # The lifecycle report explicitly records unavailable logs.
    env = dict(os.environ, COMMIT=runner.commit)
    subprocess.run(
        [
            sys.executable,
            str(HERE / 'lib/lifecycle_report.py'),
            str(status),
            runner.phase,
            str(runner.artifacts),
        ],
        env=env,
        check=False,
    )


def main():
    groups = (
        ['all', 'T0', 'T1', 'T2', 'T3']
        + [f'T1.{i}' for i in range(1, 10)]
        + [f'T2.{i}' for i in range(1, 12)]
        + [f'T3.{i}' for i in range(1, 6)]
    )
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('group', nargs='?', default='all', choices=groups)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument('--baseline', action='store_true', help='Deployment baseline only')
    modes.add_argument(
        '--mutate', choices=CASES, help='Verify a source regression with paired deployments'
    )
    args = parser.parse_args()
    runner = Runner(HERE)
    status = 1
    try:
        runner.setup()
        if args.mutate:
            run_mutation(runner, args.mutate, run_suite)
            status = 0
        else:
            package = os.environ.get('RK_PACKAGE')
            if package:
                runner.package = pathlib.Path(package).resolve()
                runner.provenance('verify')
            else:
                runner.build()
            runner.deploy()
            runner.connect()
            runner.phase = 'suite'
            status = run_suite('--baseline' if args.baseline else args.group)
            runner.check_suite_unchanged()
    except KeyboardInterrupt:
        status = 130
        runner.phase = 'suite.interrupted'
        print('# Interrupted; cleaning up this run', flush=True)
    except (
        OSError,
        ValueError,
        RuntimeError,
        AssertionError,
        EnvironmentFailure,
        subprocess.TimeoutExpired,
    ) as error:
        print('# ' + str(error), file=sys.stderr, flush=True)
    finally:
        try:
            runner.stop()
        except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
            status = 1
            runner.phase = 'cleanup.service'
            print('# Cleanup failed: ' + str(error), file=sys.stderr, flush=True)
        try:
            runner.restore_source()
        except (OSError, RuntimeError) as error:
            status = 1
            runner.phase = 'cleanup.source'
            print('# Source restore failed: ' + str(error), file=sys.stderr, flush=True)
        if status:
            lifecycle_failure(runner, status)
        runner.unlock()
    return status


if __name__ == '__main__':
    # SIGTERM follows the same owned-resource cleanup path as Ctrl-C.
    if hasattr(signal, 'SIGTERM'):

        def terminate(*_):
            raise KeyboardInterrupt

        signal.signal(signal.SIGTERM, terminate)
    try:
        sys.exit(main())
    except ValueError as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
