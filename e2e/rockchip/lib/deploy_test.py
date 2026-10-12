"""Source ownership and interruption matter more than successful command mocks."""
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

from deploy import Runner


class OwnershipTests(unittest.TestCase):
    def runner(self, directory):
        r = object.__new__(Runner)
        r.cache = pathlib.Path(directory)
        r.run_id = 'test'
        r.mutation_file = None
        return r

    def test_mutation_restores_exact_original_content(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory, 'source.rs')
            path.write_text('user original dirty content\n')
            r = self.runner(directory)
            r.backup_source(path)
            r.change_source('mutant\n')
            r.restore_source()
            self.assertEqual(path.read_text(), 'user original dirty content\n')
            self.assertIsNone(r.mutation_file)

    def test_external_edit_is_preserved_instead_of_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory, 'source.rs')
            path.write_text('original\n')
            r = self.runner(directory)
            r.backup_source(path)
            r.change_source('mutant\n')
            path.write_text('concurrent user edit\n')
            with self.assertRaisesRegex(RuntimeError, 'changed outside'):
                r.restore_source()
            self.assertEqual(path.read_text(), 'concurrent user edit\n')
            self.assertEqual(r.mutation_backup.read_text(), 'original\n')

    @unittest.skipUnless(os.name == 'posix', 'Signal process-group verification is POSIX-specific')
    def test_timed_out_owned_command_is_terminated(self):
        with tempfile.TemporaryDirectory() as directory:
            r = self.runner(directory)
            marker = pathlib.Path(directory, 'terminated')
            script = 'import pathlib,signal,time,sys\nsignal.signal(signal.SIGTERM, lambda *_: (pathlib.Path(sys.argv[1]).write_text("terminated"), sys.exit(0)))\ntime.sleep(30)'
            with self.assertRaises(subprocess.TimeoutExpired):
                r.command([sys.executable, '-c', script, str(marker)], timeout=0.3)
            self.assertEqual(marker.read_text(), 'terminated')


if __name__ == '__main__':
    unittest.main()
