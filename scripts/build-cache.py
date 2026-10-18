#!/usr/bin/env python3
"""Report local build caches; --clean removes only the listed disposable caches.

Run cleanup only while builds and QA are idle. The process check is a preflight,
not a lock: it cannot prevent another agent from starting a build afterward.
"""

import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
LIMIT = 25 * 1024**3
OLD_BUILDS = (
    "spikes/gpui-android/legacy/target",
    "spikes/encrypted-sync/fuzz/target",
    "spikes/android-biometric-pipe/rust-wrap/target",
    "spikes/android-biometric-pipe/app/build",
    "spikes/android-compose/app/build",
)
REPORT_PATHS = (
    "target",
    "fuzz/target",
    *OLD_BUILDS,
    "apps/taypeer-android/.tools/android-sdk",
    "apps/taypeer-android/.tools/avd",
    "apps/taypeer-android/.tools/gradle-cache",
    "apps/taypeer-android/app/build",
    "artifacts",
)
BUILD_COMMAND = re.compile(
    r"(?:^|[/\s])(?:cargo|rustc|rustdoc|qa\.py|check\.sh|build(?:-macos|-native)?\.sh|check-linux-package\.sh|"
    r"gradlew?)(?:\s|$)|org\.gradle\.(?:wrapper\.GradleWrapperMain|launcher\.GradleMain)"
)


def checked_path(root, relative):
    path = root / relative
    path.relative_to(root)
    if ".." in Path(relative).parts or Path(relative).is_absolute():
        raise RuntimeError(f"invalid cache path: {relative}")
    current = root
    for part in Path(relative).parts:
        current /= part
        if current.is_symlink():
            raise RuntimeError(f"refusing symlink: {current.relative_to(root)}")
    return path


def directory_bytes(path):
    """Allocated bytes, as in du; do not follow links or count their targets."""
    if not path.exists():
        return 0
    total = 0
    seen = set()
    for parent, directories, files in os.walk(path, followlinks=False):
        directories[:] = [name for name in directories if not (Path(parent) / name).is_symlink()]
        for name in [".", *files]:
            entry = (Path(parent) / name).lstat()
            identity = (entry.st_dev, entry.st_ino)
            if identity in seen:
                continue
            seen.add(identity)
            total += getattr(entry, "st_blocks", (entry.st_size + 511) // 512) * 512
    return total


def cleanup_paths(root):
    candidates = list(OLD_BUILDS)
    target = checked_path(root, "target")
    profiles = ["debug", "release"]
    if target.is_dir():
        for entry in sorted(target.iterdir()):
            if re.fullmatch(r"[\w]+(?:-[\w]+){2,}", entry.name):
                checked_path(root, entry.relative_to(root))
                profiles.extend(f"{entry.name}/{profile}" for profile in ("debug", "release"))
    candidates.extend(f"target/{profile}/incremental" for profile in profiles)
    candidates.append("fuzz/target/debug/incremental")
    return [relative for relative in candidates if checked_path(root, relative).exists()]


def validate_cleanup(root, candidates, tracked):
    for relative in candidates:
        path = checked_path(root, relative)
        if not path.is_dir():
            raise RuntimeError(f"refusing non-directory: {relative}")
        if any(name == relative or name.startswith(relative + "/") for name in tracked):
            raise RuntimeError(f"refusing cache containing tracked files: {relative}")
        for parent, directories, files in os.walk(path, followlinks=False):
            for name in directories + files:
                child = Path(parent) / name
                if child.is_symlink():
                    raise RuntimeError(f"refusing symlink: {child.relative_to(root)}")


def active_builds(processes):
    active = []
    for line in processes.splitlines():
        fields = line.strip().split(None, 1)
        if len(fields) != 2:
            continue
        pid, command_line = fields
        if BUILD_COMMAND.search(command_line):
            active.append(pid)
    return active


def remove_caches(root, candidates):
    processes = subprocess.run(
        ["ps", "-axo", "pid=,comm=,args="], check=True, capture_output=True, text=True
    ).stdout
    active = active_builds(processes)
    if active:
        raise RuntimeError(f"build or QA processes are active (PIDs: {', '.join(active)}); retry when idle")
    tracked = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"], check=True, capture_output=True
    ).stdout
    tracked = [os.fsdecode(name) for name in tracked.split(b"\0") if name]
    validate_cleanup(root, candidates, tracked)
    for relative in candidates:
        validate_cleanup(root, [relative], tracked)
        shutil.rmtree(checked_path(root, relative))
        print(f"Removed {relative}")


def gib(size):
    return f"{size / 1024**3:.2f} GiB"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--clean", action="store_true", help="remove only disposable caches while all builds are idle")
    mode.add_argument("--check", action="store_true", help="exit 1 if known Cargo targets exceed 25 GiB")
    args = parser.parse_args()
    try:
        sizes = {}
        for relative in REPORT_PATHS:
            sizes[relative] = directory_bytes(checked_path(ROOT, relative))
            print(f"{gib(sizes[relative]):>12}  {relative}")
        cargo_total = sum(sizes[name] for name in ("target", "fuzz/target", *OLD_BUILDS[:3]))
        print(f"Known Cargo targets: {gib(cargo_total)}; advisory limit: {gib(LIMIT)}")
        if cargo_total > LIMIT:
            print("Above advisory limit; review cache growth before the next full build.")
        candidates = cleanup_paths(ROOT)
        clean_bytes = sum(directory_bytes(ROOT / relative) for relative in candidates)
        print(f"Disposable caches: {gib(clean_bytes)}")
        for relative in candidates:
            print(f"  {relative}")
        if args.clean:
            remove_caches(ROOT, candidates)
        else:
            print("Preview only. Use --clean while builds and QA are idle to remove these caches.")
        return int(args.check and cargo_total > LIMIT)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"build-cache: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
