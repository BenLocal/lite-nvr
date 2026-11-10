#!/usr/bin/env python3
"""Download pinned Rockchip runtimes for offline packages."""

import hashlib
import json
import subprocess
import sys
from pathlib import Path

RKNN = "2c723177a94fac65cbc036d722907ddbc91980cf"
RKLLM = "f7390530443bf84f0394255a449d7cbe81e69d1c"


def sha256(path):
    with path.open("rb") as source:
        digest = hashlib.sha256()
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
        return digest.hexdigest()


def fetch(url, destination, expected=None):
    if destination.is_file() and (expected is None or sha256(destination) == expected):
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp")
    try:
        subprocess.run(
            ["curl", "-fsSL", "--retry", "3", "-o", str(temporary), url], check=True
        )
        if expected and sha256(temporary) != expected:
            raise ValueError(f"SHA-256 mismatch: {url}")
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)


def prepare(arch):
    cache = Path("target/cross-deps/offline")
    output = cache / arch
    (output / "lib").mkdir(parents=True, exist_ok=True)
    licenses = output / "licenses"
    licenses.mkdir(exist_ok=True)
    records = []

    def dependency(name, url, destination, expected):
        fetch(url, destination, expected)
        records.append({"name": name, "url": url, "sha256": sha256(destination)})

    if arch == "rockchip":
        for name, repo, commit, relative, checksum in [
            (
                "librknnrt.so",
                "rknn-toolkit2",
                RKNN,
                "rknpu2/runtime/Linux/librknn_api/aarch64",
                "4b6cc575bedf8952b13db4b1190fa3b3d2ac213f00be774ff6ccd19d651970c0",
            ),
            (
                "librkllmrt.so",
                "rknn-llm",
                RKLLM,
                "rkllm-runtime/Linux/librkllm_api/aarch64",
                "f25e9b099db08aaacd0a3ac62b4697d3951d6ae61ae41ea09f6702cfa89eb32c",
            ),
        ]:
            dependency(
                name,
                f"https://raw.githubusercontent.com/airockchip/{repo}/{commit}/{relative}/{name}",
                output / "lib" / name,
                checksum,
            )
            fetch(
                f"https://raw.githubusercontent.com/airockchip/{repo}/{commit}/LICENSE",
                licenses / f"{repo}.txt",
            )
    (output / "DEPENDENCIES.json").write_text(json.dumps(records, indent=2) + "\n")


if __name__ == "__main__":
    prepare(sys.argv[1])
