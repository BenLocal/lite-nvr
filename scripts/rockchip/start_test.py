"""Launcher process matching with a same-executable RKLLM worker."""

import pathlib
import shutil
import subprocess
import tempfile
import unittest


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.directory.name)
        (self.root / "bin").mkdir()
        self.launcher = self.root / "start.sh"
        shutil.copyfile(pathlib.Path(__file__).with_name("start.sh"), self.launcher)
        source = self.root / "fixture.c"
        source.write_text("#include <unistd.h>\nint main(void) { for (;;) pause(); }\n")
        self.binary = self.root / "bin" / "nvr"
        subprocess.run(["cc", str(source), "-o", str(self.binary)], check=True)
        self.children = []

    def tearDown(self):
        for child in self.children:
            if child.poll() is None:
                child.kill()
            child.wait(timeout=5)
        self.directory.cleanup()

    def spawn(self, *args):
        child = subprocess.Popen([str(self.binary), *args])
        self.children.append(child)
        return child

    def run_launcher(self, command):
        return subprocess.run(
            ["sh", str(self.launcher), command],
            check=False,
            capture_output=True,
            text=True,
            timeout=5,
        )

    def test_worker_alone_is_not_a_running_host(self):
        self.spawn("--nvr-rkllm-worker")
        result = self.run_launcher("status")
        self.assertEqual(result.returncode, 3, result.stdout + result.stderr)
        self.assertIn("nvr not running", result.stdout)

    def test_stop_signals_the_host_and_leaves_worker_cleanup_to_it(self):
        host = self.spawn()
        worker = self.spawn("--nvr-rkllm-worker")
        result = self.run_launcher("stop")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        host.wait(timeout=5)
        # The fixture has no parent-death handler. Remaining alive proves
        # the launcher only signalled the host; the real host owns cleanup.
        self.assertIsNone(worker.poll())


if __name__ == "__main__":
    unittest.main()
