#!/usr/bin/env python3
"""Content fingerprints and incremental frontend preparation for Linux packages."""

import filecmp
import hashlib
import os
import shutil
import subprocess
import sys
from pathlib import Path


def fingerprint(paths):
    digest = hashlib.sha256()

    def visit(path):
        digest.update(str(path).encode() + b"\0")
        if path.is_symlink():
            digest.update(b"link\0" + os.readlink(path).encode() + b"\0")
        elif path.is_file():
            digest.update(b"file\0" + str(path.stat().st_mode).encode() + b"\0")
            with path.open("rb") as source:
                for chunk in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(chunk)
            digest.update(b"\0")
        elif path.is_dir():
            digest.update(b"dir\0")
            for child in sorted(path.iterdir()):
                visit(child)
        else:
            digest.update(b"missing\0")

    for path in paths:
        visit(Path(path))
    return digest.hexdigest()


def prepare_frontend():
    app = Path("nvr-dashboard/app")
    cache = Path("target/cross-deps")
    cache.mkdir(parents=True, exist_ok=True)
    inputs = [app / "src", app / "public"] + sorted(
        path
        for path in app.iterdir()
        if path.is_file()
        and not path.name.endswith(".tsbuildinfo")
        and path.name not in {".eslintcache", ".DS_Store"}
    )
    source_hash = fingerprint(inputs)
    deps_hash = fingerprint(
        [app / "package.json", app / "package-lock.json", app / ".npmrc"]
    )
    deps_stamp = cache / "frontend-deps.sha256"
    build_stamp = cache / "frontend.sha256"
    dist = app / "dist"
    if (
        not (app / "node_modules").is_dir()
        or not deps_stamp.exists()
        or deps_stamp.read_text() != deps_hash
    ):
        subprocess.run(["npm", "ci"], cwd=app, check=True)
        deps_stamp.write_text(deps_hash)
    signature = source_hash + "\n" + fingerprint([dist])
    if (
        os.environ.get("FORCE_REBUILD")
        or not (dist / "index.html").is_file()
        or not build_stamp.exists()
        or build_stamp.read_text() != signature
    ):
        subprocess.run(["npm", "run", "build"], cwd=app, check=True)
        build_stamp.write_text(source_hash + "\n" + fingerprint([dist]))
    else:
        print("Frontend unchanged; skipping npm build.")


def sync_file(source, destination):
    source, destination = Path(source), Path(destination)
    if source.is_symlink():
        if destination.is_symlink() and os.readlink(source) == os.readlink(destination):
            return
        if destination.exists() or destination.is_symlink():
            remove(destination)
        destination.symlink_to(os.readlink(source))
    elif (
        destination.is_symlink()
        or not destination.is_file()
        or not filecmp.cmp(source, destination, shallow=False)
        or source.stat().st_mode != destination.stat().st_mode
    ):
        if destination.exists() or destination.is_symlink():
            remove(destination)
        shutil.copy2(source, destination)


def remove(path):
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    else:
        path.unlink()


def sync_directory(sources, destination):
    destination = Path(destination)
    if destination.is_symlink() or (destination.exists() and not destination.is_dir()):
        remove(destination)
    destination.mkdir(parents=True, exist_ok=True)
    entries = {path.name: path for path in sources}
    for child in destination.iterdir():
        if child.name not in entries:
            remove(child)
    for name, source in entries.items():
        target = destination / name
        if source.is_dir() and not source.is_symlink():
            sync_directory(list(source.iterdir()), target)
        else:
            sync_file(source, target)


if __name__ == "__main__":
    if sys.argv[1] == "frontend":
        prepare_frontend()
    elif sys.argv[1] == "sync":
        sync_directory(list(Path(sys.argv[2]).iterdir()), sys.argv[3])
    elif sys.argv[1] == "sync-files":
        sync_directory([Path(path) for path in sys.argv[3:]], sys.argv[2])
    elif sys.argv[1] == "copy":
        sync_file(sys.argv[2], sys.argv[3])
    else:
        print(fingerprint(sys.argv[1:]))
