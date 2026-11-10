"""Exercise build.sh argument forwarding with the host Bash, including macOS Bash 3."""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class BuildTest(unittest.TestCase):
    def test_package_and_workspace_scopes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            (root / "ffmpeg/lib").mkdir(parents=True)
            (root / "tools").mkdir()
            shutil.copy2(ROOT / "scripts/build.sh", root / "scripts/build.sh")
            cargo = root / "tools/cargo"
            cargo.write_text(
                "#!/usr/bin/env python3\nimport json,sys\nprint(json.dumps(sys.argv[1:]))\n"
            )
            cargo.chmod(0o755)
            env = dict(
                os.environ,
                PATH=str(root / "tools") + os.pathsep + os.environ["PATH"],
                FFMPEG_DIR=str(root / "ffmpeg"),
                ZLM_DIR="",
                SHERPA_ONNX_LIB_DIR="",
            )
            for args, expected in [
                (["check", "-p", "nvr-asr"], ["check", "-p", "nvr-asr"]),
                (["check"], ["check", "--workspace"]),
            ]:
                with self.subTest(args=args):
                    result = subprocess.run(
                        ["bash", str(root / "scripts/build.sh"), *args],
                        env=env,
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(json.loads(result.stdout), expected)


if __name__ == "__main__":
    unittest.main()
