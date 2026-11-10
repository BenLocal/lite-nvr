"""Check download integrity and cache reuse without network access."""

import hashlib
import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "package_extras", Path(__file__).resolve().parents[1] / "download-package-extras.py"
)
extras = importlib.util.module_from_spec(spec)
spec.loader.exec_module(extras)


class DownloadTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.source = self.root / "source"
        self.source.write_bytes(b"verified runtime")
        self.checksum = hashlib.sha256(self.source.read_bytes()).hexdigest()
        self.destination = self.root / "cache/runtime.so"

    def test_download_and_reuse_verified_cache(self):
        extras.fetch(self.source.as_uri(), self.destination, self.checksum)
        self.source.unlink()
        extras.fetch(self.source.as_uri(), self.destination, self.checksum)
        self.assertEqual(self.destination.read_bytes(), b"verified runtime")

    def test_replaces_corrupted_cache(self):
        self.destination.parent.mkdir()
        self.destination.write_bytes(b"corrupted")
        extras.fetch(self.source.as_uri(), self.destination, self.checksum)
        self.assertEqual(extras.sha256(self.destination), self.checksum)

    def test_rejects_download_with_wrong_checksum(self):
        with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
            extras.fetch(self.source.as_uri(), self.destination, "0" * 64)
        self.assertFalse(self.destination.exists())
        self.assertFalse(self.destination.with_suffix(".so.tmp").exists())


if __name__ == "__main__":
    unittest.main()
