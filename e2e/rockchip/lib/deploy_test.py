"""Source ownership and interruption matter more than successful command mocks."""

import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
import unittest.mock

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


class RemoteBaseTests(unittest.TestCase):
    def runner(self, **env):
        base = dict(
            RK_HOST='board',
            RK_USER='root',
            RK_ARTIFACT_DIR=tempfile.gettempdir(),
            RK_REMOTE_BASE='',
        )
        with unittest.mock.patch.dict(os.environ, dict(base, **env)):
            if 'RK_REMOTE_BASE' not in env:
                del os.environ['RK_REMOTE_BASE']
            return Runner(pathlib.Path(__file__).resolve().parents[1])

    def test_defaults_to_userdata(self):
        self.assertEqual(self.runner().remote_base, '/userdata')

    def test_accepts_a_data_disk(self):
        self.assertEqual(self.runner(RK_REMOTE_BASE='/mnt/data/e2e').remote_base, '/mnt/data/e2e')

    def test_ssh_reuses_one_authenticated_connection(self):
        runner = self.runner()
        options = ' '.join(runner.ssh)
        self.assertIn('ControlMaster=auto', options)
        path = next(o for o in runner.ssh if o.startswith('ControlPath='))[len('ControlPath=') :]
        self.assertIn(runner.run_id, path)
        self.assertLess(len(path), 104, 'Unix socket paths are limited on macOS')
        self.assertIn('ControlMaster=auto', ' '.join(runner.scp))

    def test_rejects_relative_traversal_and_shell_characters(self):
        for value in ['data', '/mnt/../etc', '/mnt/data e2e', '/mnt/$(id)']:
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.runner(RK_REMOTE_BASE=value)


if __name__ == '__main__':
    unittest.main()
