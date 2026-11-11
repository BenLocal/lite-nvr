"""Launcher process matching with a same-executable RKLLM worker."""

import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest


@unittest.skipUnless(
    os.name == "posix" and pathlib.Path("/proc").is_dir(), "requires Linux /proc"
)
class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.directory.name)
        (self.root / "bin").mkdir()
        self.launcher = self.root / "start.sh"
        shutil.copyfile(
            pathlib.Path(__file__).resolve().parents[2] / "packaging/common/start.sh", self.launcher
        )
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


class EnvFileTests(unittest.TestCase):
    def test_env_file_supplies_defaults_without_overriding(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "bin").mkdir()
            shutil.copyfile(
                pathlib.Path(__file__).resolve().parents[2] / "packaging/common/start.sh",
                root / "start.sh",
            )
            nvr = root / "bin" / "nvr"
            nvr.write_text(
                '#!/bin/sh\nprintf "%s|%s|%s\\n" "$NVR_A" "$NVR_B" "$RUST_LOG"\n'
            )
            nvr.chmod(0o755)
            (root / "nvr.env").write_text(
                "# comment\n\nNVR_A=a b=$HOME\nNVR_B=from-file\nRUST_LOG=debug\n"
                "bad-key=x\nNOVALUE"
            )
            env = dict(os.environ, NVR_B="from-env")
            env.pop("NVR_A", None)
            env.pop("RUST_LOG", None)
            result = subprocess.run(
                ["sh", str(root / "start.sh"), "run"],
                check=False,
                capture_output=True,
                text=True,
                env=env,
                timeout=5,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "a b=$HOME|from-env|debug\n")


if __name__ == "__main__":
    unittest.main()
