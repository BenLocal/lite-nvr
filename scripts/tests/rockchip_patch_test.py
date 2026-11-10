"""Verify that Rockchip overrides are temporary, including on build failures."""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class PatchTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        (self.root / "scripts").mkdir()
        (self.root / ".cargo").mkdir()
        (self.root / "tools").mkdir()
        shutil.copy2(ROOT / "scripts/with-rockchip-patch.sh", self.root / "scripts")
        (self.root / "scripts/prepare-rockchip-patch.sh").write_text(
            'printf "%s/.cache/patched\\n" "$PWD"\n'
        )
        self.config = '[env]\nFFMPEG_DIR = "ffmpeg"\n'
        (self.root / ".cargo/config.toml").write_text(self.config)
        (self.root / "Cargo.lock").write_text("original lock\n")
        cargo = self.root / "tools/cargo"
        cargo.write_text("""#!/usr/bin/env python3
import os, pathlib, sys
config = pathlib.Path('.cargo/config.toml').read_text()
assert '[patch.crates-io]' in config
assert '.cache/patched/ffmpeg-next' in config
assert '.cache/patched/ffmpeg-sys-next' in config
if sys.argv[1] == 'metadata':
    pathlib.Path('Cargo.lock').write_text('patched lock')
else:
    assert pathlib.Path('Cargo.lock').read_text() == 'patched lock'
    sys.exit(int(os.environ.get('BUILD_RESULT', '0')))
""")
        cargo.chmod(0o755)

    def run_build(self, result):
        outcome = subprocess.run(
            ["bash", "scripts/with-rockchip-patch.sh", "cargo", "build", "--locked"],
            check=False,
            cwd=self.root,
            env=dict(
                os.environ,
                PATH=str(self.root / "tools") + os.pathsep + os.environ["PATH"],
                BUILD_RESULT=str(result),
            ),
            capture_output=True,
            text=True,
        )
        self.assertEqual(outcome.returncode, result, outcome.stderr)
        self.assertEqual((self.root / ".cargo/config.toml").read_text(), self.config)
        self.assertEqual((self.root / "Cargo.lock").read_text(), "original lock\n")
        self.assertFalse((self.root / ".cache/rockchip-build.lock").exists())
        self.assertFalse(list((self.root / ".cache").glob("rockchip-*.*")))

    def test_restores_workspace_after_success(self):
        self.run_build(0)

    def test_restores_workspace_after_failure(self):
        self.run_build(7)


if __name__ == "__main__":
    unittest.main()
