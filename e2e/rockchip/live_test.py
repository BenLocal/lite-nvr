"""The live entry must never stop a service it did not deploy."""

import sys
import unittest
from unittest.mock import Mock, patch

import live


class ExistingServiceTests(unittest.TestCase):
    def run_failed(self, stage):
        runner = Mock(install=None, run_id='test')
        getattr(runner, stage).side_effect = RuntimeError(stage + ' failed')

        def stop():
            self.assertIsNone(runner.install, 'Existing installation must not reach Runner.stop')

        runner.stop.side_effect = stop
        with (
            patch.object(sys, 'argv', ['live.py', '--install-dir', '/existing/nvr']),
            patch.object(live, 'Runner', return_value=runner),
            patch.object(live, 'identity', return_value='pid=123 sha256=original'),
        ):
            with self.assertRaisesRegex(RuntimeError, stage + ' failed'):
                live.main()
        runner.stop.assert_called_once()
        runner.unlock.assert_called_once()

    def test_setup_failure_does_not_stop_existing_service(self):
        self.run_failed('setup')

    def test_connection_failure_does_not_stop_existing_service(self):
        self.run_failed('connect')


if __name__ == '__main__':
    unittest.main()
