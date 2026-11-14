"""The FTP stub must satisfy a real FTP client's login exactly as T4.10 relies on."""

import ftplib
import pathlib
import subprocess
import sys
import tempfile
import time
import unittest

from coverage import FTP_STUB


class FtpStubTests(unittest.TestCase):
    def test_only_the_configured_password_logs_in(self):
        with tempfile.TemporaryDirectory() as directory:
            base = pathlib.Path(directory) / 'ftp'
            stub = subprocess.Popen([sys.executable, '-c', FTP_STUB, str(base), 'right'])
            try:
                port_file = base.with_suffix('.port')
                for _ in range(50):
                    if port_file.exists() and port_file.read_text():
                        break
                    time.sleep(0.1)
                port = int(port_file.read_text())
                ftp = ftplib.FTP()
                ftp.connect('127.0.0.1', port, timeout=5)
                ftp.login('e2e', 'right')
                ftp.quit()
                ftp = ftplib.FTP()
                ftp.connect('127.0.0.1', port, timeout=5)
                with self.assertRaises(ftplib.error_perm):
                    ftp.login('e2e', 'wrong')
                ftp.close()
            finally:
                stub.kill()
                stub.wait(timeout=5)


if __name__ == '__main__':
    unittest.main()
