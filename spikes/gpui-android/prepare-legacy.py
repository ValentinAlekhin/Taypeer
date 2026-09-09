#!/usr/bin/env python3
"""Fetch pinned candidate sources into ignored vendor/ and rebind Kit to one GPUI.

Run through RTK in the owner session. Only vendor/ is modified.
"""
from pathlib import Path
import subprocess
import re

ROOT = Path(__file__).resolve().parent
SOURCES = [
    ("mobile-zed", "https://github.com/tanlethanh/zed", "5955915c572ea9336d9648a2d286b399298fe5ff"),
    ("legacy-kit", "https://github.com/longbridge/gpui-kit", "e12e39ca55051d02f0e2ae8a910a674905c722d6"),
]
for name, repository, revision in SOURCES:
    destination = ROOT / "vendor" / name
    if not destination.exists():
        destination.mkdir(parents=True)
        subprocess.run(["rtk", "git", "init", str(destination)], check=True)
        subprocess.run(["rtk", "git", "-C", str(destination), "fetch", "--depth", "1", repository, revision], check=True)
        subprocess.run(["rtk", "git", "-C", str(destination), "checkout", "--detach", revision], check=True)
    if (destination / ".git").exists():
        actual = subprocess.check_output(["rtk", "proxy", "git", "-C", str(destination), "rev-parse", "HEAD"], text=True).strip()
        if actual != revision:
            raise SystemExit(f"Unexpected revision in {destination}: {actual}")
    elif not (destination / ".pass2p-source-rev").exists() or (destination / ".pass2p-source-rev").read_text().strip() != revision:
        raise SystemExit(f"Unverified source directory: {destination}")

manifest = ROOT / "vendor/legacy-kit/Cargo.toml"
source = manifest.read_text()
for name in ["gpui", "gpui_platform", "gpui_web", "gpui_macros", "reqwest_client"]:
    source = re.sub(r'(?m)^' + name + r' = \{ git = "https://github.com/zed-industries/zed"',
                    name + ' = { path = "../mobile-zed/crates/' + name + '"', source)
manifest.write_text(source)
print("Pinned sources ready. No GPUI or component API patches applied.")
