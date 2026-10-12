"""Test reporting and failure boundaries without treating mocks as board evidence."""

import contextlib
import io
import json
import pathlib
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from unittest.mock import patch

from suite import Restart, Skip, Suite


class HarnessTests(unittest.TestCase):
    def suite(self, directory):
        suite = object.__new__(Suite)
        suite.selected = None
        suite.unavailable = False
        suite.restarts = 0
        suite.uptime_start = suite.uptime_end = None
        suite.token = 'secret-test-session'
        suite.last = ''
        suite.results = []
        suite.boot = 'original-boot'
        suite.base = 'http://localhost'
        suite.artifacts = pathlib.Path(directory)
        suite.root = '/userdata/test'
        suite.remote = '/userdata'
        suite.board = lambda *_: 'log token=' + suite.token
        suite.check_boot = lambda: None
        return suite

    def test_failure_redacts_response_and_log_and_exits_non_green(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict('os.environ', COMMIT='test'):
            suite = self.suite(directory)

            def failure():
                suite.last = '{"token":"secret-test-session", "state":"idle"}'
                raise AssertionError('expected running, actual idle')

            with contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertFalse(suite.test('T1.2', '23217e1', failure))
            report = (suite.artifacts / 'report.json').read_text()
            self.assertNotIn(suite.token, report + output.getvalue())
            self.assertIn('expected running, actual idle', report)
            self.assertEqual(json.loads(report)['summary']['FAIL'], 1)

    def test_skip_is_not_counted_as_pass(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict('os.environ', COMMIT='test'):
            suite = self.suite(directory)

            def absent_sink():
                raise Skip('no HDMI mode')

            with contextlib.redirect_stdout(io.StringIO()):
                suite.test('T3.3', '23217e1', absent_sink)
            summary = json.loads((suite.artifacts / 'report.json').read_text())['summary']
            self.assertEqual(summary, {'PASS': 0, 'FAIL': 0, 'SKIP': 1})

    def test_restart_stops_following_tests_and_records_failure(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict('os.environ', COMMIT='test'):
            suite = self.suite(directory)

            def reboot():
                raise Restart('boot changed')

            suite.check_boot = reboot
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(Restart):
                suite.test('T2.1', 'b33e7d7', lambda: None)
            self.assertEqual(suite.results[0]['status'], 'FAIL')
            self.assertIn('BOARD RESTART', suite.results[0]['detail'])

    def test_wrong_http_status_is_not_accepted_as_application_error(self):
        class WrongStatus(BaseHTTPRequestHandler):
            def do_POST(self):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{"code":500,"message":"bad","data":null}')

            def log_message(self, *_):
                pass

        with HTTPServer(('127.0.0.1', 0), WrongStatus) as server:
            worker = threading.Thread(target=server.handle_request)
            worker.start()
            try:
                suite = object.__new__(Suite)
                suite.token = ''
                suite.base = 'http://127.0.0.1:' + str(server.server_port)
                with self.assertRaisesRegex(AssertionError, 'expected HTTP 500, actual 200'):
                    suite.api('POST', '/api/device/add', {}, expected=500)
            finally:
                worker.join(timeout=5)


if __name__ == '__main__':
    unittest.main()
