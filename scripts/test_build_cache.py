import importlib.util
import contextlib
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("build_cache", Path(__file__).with_name("build-cache.py"))
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)


class BuildCacheTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def directory(self, relative):
        path = self.root / relative
        path.mkdir(parents=True)
        (path / "object").write_bytes(b"disposable")
        return path

    def test_only_allowlisted_cache_directories_are_selected(self):
        expected = {
            "target/debug/incremental",
            "target/x86_64-pc-windows-msvc/debug/incremental",
            "fuzz/target/debug/incremental",
            *cache.OLD_BUILDS,
        }
        for relative in expected | {
            "target/debug/deps",
            "target/debug/build/dependency/incremental",
            "artifacts/incremental",
            "apps/taypeer-android/.tools/avd",
        }:
            self.directory(relative)
        self.assertEqual(set(cache.cleanup_paths(self.root)), expected)

    def test_symlink_ancestor_and_descendant_are_refused(self):
        outside = self.directory("preserved")
        (self.root / "target").symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(RuntimeError, "symlink"):
            cache.cleanup_paths(self.root)
        (self.root / "target").unlink()
        path = self.directory("target/debug/incremental")
        (path / "link").symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(RuntimeError, "symlink"):
            cache.validate_cleanup(self.root, ["target/debug/incremental"], [])
        self.assertTrue((outside / "object").exists())

    def test_tracked_files_and_path_traversal_are_refused(self):
        self.directory("target/debug/incremental")
        with self.assertRaisesRegex(RuntimeError, "tracked"):
            cache.validate_cleanup(self.root, ["target/debug/incremental"], ["target/debug/incremental/object"])
        for relative in ("../outside", str(self.root.parent)):
            with self.assertRaises((RuntimeError, ValueError)):
                cache.checked_path(self.root, relative)

    def test_process_guard_does_not_block_idle_java_daemons(self):
        processes = """\
10 /usr/bin/cargo cargo test --workspace
11 /bin/sh /bin/sh scripts/check.sh
12 /usr/bin/python3 python3 apps/taypeer-android/qa.py test --suite network
13 /usr/bin/java java org.gradle.launcher.daemon.bootstrap.GradleDaemon 8.13
14 /usr/bin/sh sh apps/taypeer-android/gradlew assembleDebug
15 /usr/bin/python3 python3 scripts/build-cache.py --clean
16 /usr/bin/rustc rustc --crate-name example
17 /bin/sh sh apps/taypeer-android/build-native.sh debug
18 /bin/sh sh spikes/gpui-android/legacy/build.sh
19 /usr/bin/java gradle assembleDebug
20 /Volumes/Build Tools/bin/rustc rustc --crate-name example
21 /usr/bin/rustdoc
"""
        self.assertEqual(cache.active_builds(processes), ["10", "11", "12", "14", "16", "17", "18", "19", "20", "21"])

    def test_size_does_not_follow_links_or_double_count_hard_links(self):
        path = self.directory("cache")
        outside = self.root / "preserved"
        outside.write_bytes(b"outside" * 10000)
        os.link(path / "object", path / "hard-link")
        (path / "symlink").symlink_to(outside)
        expected = sum(entry.lstat().st_blocks * 512 for entry in (path, path / "object", path / "symlink"))
        self.assertEqual(cache.directory_bytes(path), expected)

    def test_cleanup_checks_every_candidate_before_removing_anything(self):
        first = self.directory("target/debug/incremental")
        self.directory(cache.OLD_BUILDS[0])
        results = [
            subprocess.CompletedProcess([], 0, stdout=""),
            subprocess.CompletedProcess([], 0, stdout=(cache.OLD_BUILDS[0] + "/object\0").encode()),
        ]
        with patch.object(cache.subprocess, "run", side_effect=results):
            with self.assertRaisesRegex(RuntimeError, "tracked"):
                cache.remove_caches(self.root, ["target/debug/incremental", cache.OLD_BUILDS[0]])
        self.assertTrue(first.is_dir())

    def test_cleanup_removes_cache_and_preserves_other_build_outputs(self):
        candidate = self.directory("target/debug/incremental")
        preserved = self.directory("target/debug/deps")
        results = [
            subprocess.CompletedProcess([], 0, stdout=""),
            subprocess.CompletedProcess([], 0, stdout=b""),
        ]
        with patch.object(cache.subprocess, "run", side_effect=results), contextlib.redirect_stdout(io.StringIO()):
            cache.remove_caches(self.root, ["target/debug/incremental"])
        self.assertFalse(candidate.exists())
        self.assertTrue((preserved / "object").is_file())

    def test_active_build_refuses_cleanup_without_reading_git(self):
        candidate = self.directory("target/debug/incremental")
        with patch.object(cache.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, stdout="1 cargo cargo check")) as run:
            with self.assertRaisesRegex(RuntimeError, "active"):
                cache.remove_caches(self.root, ["target/debug/incremental"])
        self.assertEqual(run.call_count, 1)
        self.assertTrue(candidate.is_dir())

    def test_default_report_is_read_only_and_budget_check_is_optional(self):
        candidate = self.directory("target/debug/incremental")
        with patch.object(cache, "ROOT", self.root), patch.object(cache, "remove_caches") as remove:
            with patch.object(cache.sys, "argv", ["build-cache.py"]), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(cache.main(), 0)
            with patch.object(cache.sys, "argv", ["build-cache.py", "--check"]), patch.object(
                cache, "directory_bytes", return_value=cache.LIMIT
            ), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(cache.main(), 1)
            remove.assert_not_called()
        self.assertTrue(candidate.is_dir())


if __name__ == "__main__":
    unittest.main()
