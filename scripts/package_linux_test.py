"""Run the real packaging entry point with tiny SDKs and fake external builders."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


class PackageTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in [
            "scripts/package-linux.sh",
            "scripts/package-cache.py",
            "scripts/rockchip/start.sh",
        ]:
            self.write(name, (ROOT / name).read_text())
            (self.root / name).chmod((ROOT / name).stat().st_mode)
        self.write("nvr/Cargo.toml", 'version = "0.1.0"\n')
        self.write("Cargo.lock", 'name = "sherpa-onnx-sys"\nversion = "1.13.3"\n')
        self.write("LICENSE", "license")
        self.write("nvr-dashboard/app/package.json", "{}")
        self.write("nvr-dashboard/app/package-lock.json", "{}")
        self.write("nvr-dashboard/app/src/main.ts", "first")
        self.write("scripts/download-rockchip-ffmpeg.sh", "exit 0\n")
        self.write("scripts/download_sherpa_onnx_libs.sh", "exit 0\n")
        self.write("scripts/with-rockchip-patch.sh", '"$@"\n')
        for arch in ["amd64", "arm64", "rockchip"]:
            for file in [
                "ffmpeg/include/libavcodec/avcodec.h",
                "ffmpeg/bin/ffmpeg",
                "zlm/lib/libmk_api.so",
                "onnxruntime/lib/libonnxruntime.so",
                "onnxruntime/LICENSE",
            ]:
                self.write(f"target/cross-deps/{arch}/{file}", arch + file)
            (
                self.root
                / f"target/cross-deps/{arch}/onnxruntime/lib/libonnxruntime.so.1"
            ).symlink_to("libonnxruntime.so")
        self.write(
            "fake/bin/tool",
            """#!/usr/bin/env python3
import os, pathlib, sys
name=pathlib.Path(sys.argv[0]).name
args=sys.argv[1:]
with open('calls', 'a') as log: log.write(name+' '+' '.join(args)+'\\n')
if name=='cross':
    target=args[args.index('--target')+1]
    output=pathlib.Path(args[args.index('--target-dir')+1])/target/'release/nvr'
    output.parent.mkdir(parents=True,exist_ok=True)
    content=pathlib.Path('nvr-dashboard/app/src/main.ts').read_text()+str(output)
    if not output.exists() or output.read_text()!=content: output.write_text(content)
elif name=='docker' and args[:2]==['image','inspect']: print('sha256:image1')
elif name=='docker' and args[0]=='run':
    volume=args[args.index('-v')+1].split(':')[0]
    for lib in ['libstdc++.so.6','libgcc_s.so.1','libgomp.so.1']:
        pathlib.Path(volume,lib).write_text('runtime')
""",
        )
        # npm runs in a different directory, so give it a separate stub.
        self.write(
            "fake/bin/npm",
            """#!/usr/bin/env python3
from pathlib import Path
import sys
with open('../../calls','a') as log: log.write('npm '+' '.join(sys.argv[1:])+'\\n')
if sys.argv[1]=='ci': Path('node_modules').mkdir(exist_ok=True)
else:
    Path('dist').mkdir(exist_ok=True)
    Path('dist/index.html').write_text(Path('src/main.ts').read_text())
""",
        )
        for name in ["cross", "docker"]:
            (self.root / f"fake/bin/{name}").symlink_to("tool")
        for name in ["tool", "npm"]:
            (self.root / f"fake/bin/{name}").chmod(0o755)
        self.env = os.environ.copy()
        for key in list(self.env):
            if key.startswith("CROSS_") or key in [
                "APT_MIRROR",
                "FORCE_REBUILD",
                "PACKAGE_ARCHIVE",
            ]:
                self.env.pop(key)
        self.env["PATH"] = str(self.root / "fake/bin") + os.pathsep + self.env["PATH"]
        self.env["CARGO_HOME"] = str(self.root / "fake")

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

    def run_package(self, archive=False):
        env = dict(self.env, PACKAGE_ARCHIVE="1" if archive else "0")
        result = subprocess.run(
            ["bash", "scripts/package-linux.sh", "amd64", "arm64", "rockchip"],
            check=False,
            cwd=self.root,
            env=env,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def test_hot_update_preserves_files_and_skips_frontend_and_runtime(self):
        self.run_package()
        times = {
            path: path.lstat().st_mtime_ns
            for path in (self.root / "dist").rglob("*")
            if not path.is_dir()
        }
        self.run_package()
        self.assertEqual(times, {path: path.lstat().st_mtime_ns for path in times})
        calls = (self.root / "calls").read_text()
        self.assertEqual(calls.count("docker run "), 3)
        self.assertEqual(calls.count("npm ci"), 1)
        self.assertEqual(calls.count("npm run build"), 1)
        self.assertFalse(list((self.root / "dist").glob("*.tar.gz")))
        for arch in ["amd64", "arm64", "rockchip"]:
            self.assertIn("--target-dir target/package/" + arch, calls)

    def test_source_and_sdk_changes_update_only_changed_files(self):
        self.run_package()
        package = self.root / "dist/lite-nvr-0.1.0-linux-arm64-rockchip"
        sdk_time = (package / "ffmpeg/bin/ffmpeg").stat().st_mtime_ns
        self.write("nvr-dashboard/app/src/main.ts", "updated")
        self.run_package()
        self.assertTrue((package / "bin/nvr").read_text().startswith("updated"))
        self.assertEqual(sdk_time, (package / "ffmpeg/bin/ffmpeg").stat().st_mtime_ns)
        source = self.root / "target/cross-deps/rockchip/ffmpeg/bin/ffmpeg"
        source.unlink()
        self.run_package()
        self.assertFalse((package / "ffmpeg/bin/ffmpeg").exists())
        self.assertTrue((package / "lib/libonnxruntime.so.1").is_symlink())
        self.assertEqual((self.root / "calls").read_text().count("npm ci"), 1)

    def test_lock_change_and_missing_frontend_output(self):
        self.run_package()
        self.write("nvr-dashboard/app/package-lock.json", '{"changed": true}')
        self.run_package()
        (self.root / "nvr-dashboard/app/dist/index.html").unlink()
        self.run_package()
        calls = (self.root / "calls").read_text()
        self.assertEqual(calls.count("npm ci"), 2)
        self.assertEqual(calls.count("npm run build"), 3)

    def test_missing_runtime_and_modified_destination_are_repaired(self):
        self.run_package()
        runtime = self.root / "target/cross-deps/runtime-rockchip/libgomp.so.1"
        runtime.unlink()
        package = self.root / "dist/lite-nvr-0.1.0-linux-arm64-rockchip"
        (package / "bin/nvr").write_text("corrupted")
        (package / "lib/obsolete.so").write_text("obsolete")
        self.run_package()
        self.assertTrue(runtime.is_file())
        self.assertFalse((package / "lib/obsolete.so").exists())
        self.assertNotEqual((package / "bin/nvr").read_text(), "corrupted")

    def test_optional_archive_is_cached_and_invalidated(self):
        self.run_package(archive=True)
        archive = self.root / "dist/lite-nvr-0.1.0-linux-arm64-rockchip.tar.gz"
        before = archive.stat().st_mtime_ns
        self.run_package(archive=True)
        self.assertEqual(before, archive.stat().st_mtime_ns)
        self.write("target/cross-deps/rockchip/onnxruntime/LICENSE", "new license")
        self.run_package(archive=True)
        self.assertNotEqual(before, archive.stat().st_mtime_ns)
        self.assertTrue((self.root / "dist/SHA256SUMS").is_file())


if __name__ == "__main__":
    unittest.main()
