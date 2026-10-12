"""Failures before HTTP tests and after successful tests must still be red."""
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


class LifecycleTests(unittest.TestCase):
    def test_deploy_failure_creates_non_green_report(self):
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run([sys.executable, str(pathlib.Path(__file__).with_name('lifecycle_report.py')), '1', 'T0.deploy', directory], capture_output=True, text=True, check=True)
            report = json.loads(pathlib.Path(directory, 'report.json').read_text())
            self.assertEqual(report['summary']['FAIL'], 1)
            self.assertIn('not ok 1', result.stdout)

    def test_stop_failure_cannot_leave_green_summary(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory, 'report.json')
            path.write_text(json.dumps({'commit': 'test', 'results': [{'id': 'T0.1', 'status': 'PASS'}], 'summary': {'PASS': 1, 'FAIL': 0, 'SKIP': 0}}))
            result = subprocess.run([sys.executable, str(pathlib.Path(__file__).with_name('lifecycle_report.py')), '1', 'cleanup.service', directory], capture_output=True, text=True, check=True)
            report = json.loads(path.read_text())
            self.assertEqual(report['summary']['FAIL'], 1)
            self.assertEqual(report['results'][-1]['status'], 'FAIL')
            self.assertIn('Bail out!', result.stdout)


if __name__ == '__main__':
    unittest.main()
