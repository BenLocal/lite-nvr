"""A package cannot be reused after changing its bytes, source or commit."""
import contextlib
import io
import pathlib
import tempfile
import unittest
from unittest.mock import patch

import provenance


class ProvenanceTests(unittest.TestCase):
    def test_only_matching_package_source_and_commit_is_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            package = pathlib.Path(directory) / 'package.tar.gz'
            package.write_bytes(b'fresh-package')
            def run(action, commit='commit-a'):
                with patch('sys.argv', ['provenance.py', action, str(package), directory, commit]), contextlib.redirect_stdout(io.StringIO()):
                    provenance.main()
            with patch('provenance.source_digest', return_value='source-a'):
                with self.assertRaisesRegex(AssertionError, 'lacks build provenance'):
                    run('verify')
                run('write')
                run('verify')
                with self.assertRaisesRegex(AssertionError, 'does not match'):
                    run('verify', commit='commit-b')
                package.write_bytes(b'old-or-mutant-package')
                with self.assertRaisesRegex(AssertionError, 'does not match'):
                    run('verify')
                package.write_bytes(b'fresh-package')
            with patch('provenance.source_digest', return_value='source-b'):
                with self.assertRaisesRegex(AssertionError, 'does not match'):
                    run('verify')


if __name__ == '__main__':
    unittest.main()
