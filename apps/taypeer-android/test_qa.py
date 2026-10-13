import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
from unittest.mock import Mock, patch
import xml.etree.ElementTree as ET
from contextlib import redirect_stdout
from types import SimpleNamespace


spec = importlib.util.spec_from_file_location("qa", Path(__file__).with_name("qa.py"))
qa = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qa)

coverage_spec = importlib.util.spec_from_file_location("verify_coverage", Path(__file__).with_name("verify_coverage.py"))
coverage = importlib.util.module_from_spec(coverage_spec)
coverage_spec.loader.exec_module(coverage)


def status(code, name="publicFixture", detail=""):
    return ("INSTRUMENTATION_STATUS: class=dev.taypeer.PublicTest\n"
            f"INSTRUMENTATION_STATUS: test={name}\n{detail}"
            f"INSTRUMENTATION_STATUS_CODE: {code}\n")


class InstrumentationResults(unittest.TestCase):
    def test_success_requires_completed_tests_and_successful_terminal(self):
        output = status(1) + status(0) + "INSTRUMENTATION_CODE: -1\n"
        cases, success = qa.parse_instrumentation(output)
        self.assertTrue(success)
        self.assertEqual([case["status"] for case in cases], ["passed"])
        self.assertFalse(qa.parse_instrumentation(status(1))[1])
        self.assertFalse(qa.parse_instrumentation("INSTRUMENTATION_CODE: -1\n")[1])

    def test_adb_zero_exit_does_not_hide_test_failure(self):
        output = status(-2, detail="INSTRUMENTATION_STATUS: stack=java.lang.AssertionError\n  at PUBLIC test\n")
        cases, success = qa.parse_instrumentation(output + "INSTRUMENTATION_CODE: -1\n")
        self.assertFalse(success)
        self.assertEqual(cases[0]["status"], "failed")
        self.assertIn("at PUBLIC test", cases[0]["detail"])

    def test_crash_after_passes_is_failure_in_junit(self):
        output = status(0) + "INSTRUMENTATION_RESULT: shortMsg=Process crashed\nINSTRUMENTATION_CODE: 0\n"
        cases, success = qa.parse_instrumentation(output)
        self.assertFalse(success)
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "junit.xml"
            qa.write_junit(path, cases, success, output, 1.5)
            root = ET.parse(path).getroot()
            self.assertEqual(root.attrib["tests"], "2")
            self.assertEqual(root.attrib["failures"], "1")
            self.assertEqual(len(root.findall("testcase/failure")), 1)

    def test_ignored_and_assumption_failures_remain_distinct_from_failure(self):
        cases, success = qa.parse_instrumentation(status(-3, "ignored") + status(-4, "assumption")
                                                  + status(0, "passed") + "INSTRUMENTATION_CODE: -1\n")
        self.assertTrue(success)
        self.assertEqual([case["status"] for case in cases], ["skipped", "skipped", "passed"])


class LiveInstrumentation(unittest.TestCase):
    selection = "dev.taypeer.DocumentTest#twoRealDocumentsSaveReopenAndLockIndependently"
    name = "twoRealDocumentsSaveReopenAndLockIndependently"

    def frame(self, code, *, name=None, fields=""):
        return ("INSTRUMENTATION_STATUS: class=dev.taypeer.DocumentTest\n"
                f"INSTRUMENTATION_STATUS: test={name or self.name}\n{fields}"
                f"INSTRUMENTATION_STATUS_CODE: {code}\n")

    def progress(self, folder):
        return qa.InstrumentationProgress([self.selection], Path(folder), "emulator-PUBLIC")

    def test_progress_accepts_only_selected_declared_cases_and_bounded_fixed_phases(self):
        with tempfile.TemporaryDirectory() as folder, redirect_stdout(io.StringIO()) as printed:
            progress = self.progress(folder)
            progress.feed(self.frame(1, name="PUBLIC_unknown_method"))
            progress.feed(self.frame(1).replace("dev.taypeer.DocumentTest", "other.PUBLIC_foreign"))
            self.assertNotIn("class", json.loads(progress.path.read_text()))
            public = self.frame(1, fields="INSTRUMENTATION_STATUS: stack=PUBLIC_private_marker\n")
            progress.feed(public[:12]); progress.feed(public[12:])
            for extra in (
                "qa_phase=PUBLIC_private_marker\nINSTRUMENTATION_STATUS: qa_row=1\nINSTRUMENTATION_STATUS: qa_elapsed_ms=2\n",
                "qa_phase=clone\nINSTRUMENTATION_STATUS: qa_row=70\nINSTRUMENTATION_STATUS: qa_elapsed_ms=2\n",
                "qa_phase=clone\nINSTRUMENTATION_STATUS: qa_row=1\nINSTRUMENTATION_STATUS: qa_elapsed_ms=99999999\n",
            ):
                progress.feed(self.frame(2, fields="INSTRUMENTATION_STATUS: " + extra))
            progress.feed(self.frame(2, fields="INSTRUMENTATION_STATUS: qa_phase=clone\nINSTRUMENTATION_STATUS: qa_row=69\nINSTRUMENTATION_STATUS: qa_elapsed_ms=1234\n"))
            state = json.loads(progress.path.read_text())
            self.assertEqual((state["row"], state["phase"], state["fixture_elapsed_ms"]), (69, "clone", 1234))
            self.assertNotIn("PUBLIC_private_marker", progress.path.read_text() + printed.getvalue())
            self.assertFalse(progress.path.with_suffix(".json.tmp").exists())
            progress.finish(False)
            self.assertEqual(json.loads(progress.path.read_text())["run"], "failed")

    def test_nonzero_process_exit_cannot_become_success_after_passing_status(self):
        output = self.frame(0) + "INSTRUMENTATION_CODE: -1\n"
        with tempfile.TemporaryDirectory() as folder, redirect_stdout(io.StringIO()), \
                patch.object(qa, "adb", return_value=subprocess.CompletedProcess([], 7, output, "")):
            path = Path(folder)
            self.assertFalse(qa.instrument({}, "emulator-PUBLIC", [self.selection], {}, path, 30))
            self.assertEqual((path / "instrumentation.log").read_text(), output)
            self.assertEqual(json.loads((path / "progress.json").read_text())["run"], "failed")
            self.assertEqual(ET.parse(path / "junit.xml").getroot().attrib["failures"], "1")

    def test_private_spike_timeout_stops_only_its_two_owned_packages(self):
        package = "dev.taypeer.biometricspike"
        failure = subprocess.TimeoutExpired([], 1, output=self.frame(1), stderr="")
        with tempfile.TemporaryDirectory() as folder, redirect_stdout(io.StringIO()), \
                patch.object(qa, "adb", side_effect=[failure, Mock(returncode=0), Mock(returncode=0)]) as adb:
            self.assertFalse(qa.instrument({}, "emulator-PUBLIC", [self.selection], {}, Path(folder), 1,
                                           package=package, runner=package + ".test/androidx.test.runner.AndroidJUnitRunner"))
            self.assertEqual([call.args[5] for call in adb.call_args_list[1:]], [package + ".test", package])
            self.assertTrue(adb.call_args_list[0].args[-1].startswith(package + ".test/"))

    def test_foreign_runner_and_spike_product_coverage_are_rejected_before_adb(self):
        with tempfile.TemporaryDirectory() as folder, patch.object(qa, "adb") as adb:
            for package, runner, covered in (
                ("foreign.app", "foreign.app.test/PublicRunner", False),
                ("dev.taypeer.biometricspike", qa.RUNNER, False),
                ("dev.taypeer.biometricspike", "dev.taypeer.biometricspike.test/PublicRunner", True),
            ):
                with self.assertRaises(qa.QaError):
                    qa.instrument({}, "emulator-PUBLIC", [self.selection], {}, Path(folder), 1,
                                  package=package, runner=runner, coverage=covered)
            adb.assert_not_called()

    def test_heartbeat_is_rate_limited_and_progress_does_not_replace_completion(self):
        clock = [100.]
        with tempfile.TemporaryDirectory() as folder, patch.object(qa.time, "monotonic", side_effect=lambda: clock[0]), redirect_stdout(io.StringIO()) as printed:
            progress = self.progress(folder)
            progress.feed(self.frame(1))
            for second in range(1, 31):
                clock[0] = 100. + second
                progress.tick()
            self.assertEqual(len(printed.getvalue().splitlines()), 3)
            self.assertEqual(json.loads(progress.path.read_text())["host_elapsed_ms"], 30_000)
            self.assertFalse(qa.parse_instrumentation(self.frame(1))[1])
            # Large arbitrary stream lines remain outside the small sanitized parser.
            progress.feed("PUBLIC_" * 1000 + "\n")
            self.assertEqual(progress.latest["method"], self.name)

    def test_stream_drains_large_stderr_and_preserves_split_utf8_and_full_terminal_text(self):
        stdout = self.frame(1) + "PUBLIC UTF8 Текст\n" + self.frame(0) + "INSTRUMENTATION_CODE: -1\n"
        stderr = "PUBLIC stderr\n" * 40_000
        encoded = stdout.encode()
        split = encoded.index("Т".encode()) + 1
        script = ("import os\n"
                  f"os.write(1, {encoded[:split]!r})\n"
                  f"os.write(2, {stderr.encode()!r})\n"
                  f"os.write(1, {encoded[split:]!r})\n")
        with tempfile.TemporaryDirectory() as folder, redirect_stdout(io.StringIO()):
            source = Path(folder) / "PUBLIC-child.py"
            source.write_text(script)
            result = qa.run_instrumentation([sys.executable, str(source)], env=os.environ.copy(), timeout=10, progress=self.progress(folder))
            self.assertEqual(result.returncode, 0)
            self.assertEqual(result.stdout, stdout)
            self.assertEqual(result.stderr, stderr)
            self.assertTrue(qa.parse_instrumentation(result.stdout)[1])

    def test_timeout_kills_only_owned_group_and_retains_partial_evidence(self):
        stdout, stderr = self.frame(1), "PUBLIC timeout stderr\n"
        script = ("import signal,sys,time\n"
                  "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                  f"sys.stdout.write({stdout!r}); sys.stdout.flush()\n"
                  f"sys.stderr.write({stderr!r}); sys.stderr.flush()\n"
                  "time.sleep(60)\n")
        with tempfile.TemporaryDirectory() as folder, redirect_stdout(io.StringIO()), \
                patch.object(qa, "INSTRUMENTATION_TERM_GRACE", .1), \
                patch.object(qa, "INSTRUMENTATION_KILL_GRACE", .1), \
                patch.object(qa.os, "killpg", wraps=os.killpg) as killed:
            source = Path(folder) / "PUBLIC-timeout.py"
            source.write_text(script)
            progress = self.progress(folder)
            started = time.monotonic()
            with self.assertRaises(subprocess.TimeoutExpired) as failure:
                qa.run_instrumentation([sys.executable, str(source)], env=os.environ.copy(), timeout=1.5, progress=progress)
            self.assertLess(time.monotonic() - started, 10)
            self.assertEqual(failure.exception.output, stdout)
            self.assertEqual(failure.exception.stderr, stderr)
            self.assertEqual([call.args[1] for call in killed.call_args_list], [signal.SIGTERM, signal.SIGKILL])
            self.assertEqual(len({call.args[0] for call in killed.call_args_list}), 1)
            self.assertFalse(qa.parse_instrumentation(failure.exception.output)[1])
            progress.finish(False)
            self.assertEqual(json.loads(progress.path.read_text())["run"], "timed_out")


class CoverageInstrumentation(unittest.TestCase):
    @staticmethod
    def pool(name, *, identity=None):
        # Public constant-pool fixtures exercise the guard without executing JVM
        # classes or depending on an Android SDK. Two slots belong to a Long.
        import struct
        strings = [name.encode()]
        if identity is not None:
            strings += [b"$jacocoInit", b"$jacocoData", b"org/jacoco/agent/rt/internal_PUBLIC/Offline"]
        entries = b"".join(b"\x01" + struct.pack(">H", len(value)) + value for value in strings)
        if identity is not None:
            entries += b"\x05" + struct.pack(">Q", identity)
        return b"\xca\xfe\xba\xbe\x00\x00\x00\x3d" + struct.pack(">H", len(strings) + 1 + (2 if identity is not None else 0)) + entries

    def fixture(self, directory, *, instrumented):
        raw, output = Path(directory) / "raw", Path(directory) / "instrumented"
        for name in coverage.REQUIRED:
            relative = Path("dev/taypeer") / (name + ".class")
            original = self.pool("PUBLIC " + name)
            for root, data in ((raw, original), (output, self.pool("PUBLIC " + name, identity=coverage.class_id(original)) if instrumented else original)):
                (root / relative).parent.mkdir(parents=True, exist_ok=True)
                (root / relative).write_bytes(data)
        return raw, output

    def test_exact_copies_cannot_pass_as_instrumented_and_cached_repair_remains_valid(self):
        with tempfile.TemporaryDirectory() as directory:
            raw, output = self.fixture(directory, instrumented=False)
            self.assertEqual(coverage.verify([raw], output, repaired=False), 3)
            with self.assertRaisesRegex(coverage.CoverageError, "no matching JaCoCo probes"):
                coverage.verify([raw], output, repaired=True)
            raw, output = self.fixture(directory, instrumented=True)
            self.assertEqual(coverage.verify([raw], output, repaired=False), 3)
            self.assertEqual(coverage.verify([raw], output, repaired=True), 3)

    def test_stale_probes_and_instrumented_input_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            raw, output = self.fixture(directory, instrumented=True)
            source = raw / "dev/taypeer/RestoreRevisionState.class"
            original = source.read_bytes()
            source.write_bytes(self.pool("PUBLIC changed class"))
            with self.assertRaisesRegex(coverage.CoverageError, "Stale or unrelated"):
                coverage.verify([raw], output, repaired=False)
            source.write_bytes(self.pool("PUBLIC input with probes", identity=coverage.class_id(original)))
            with self.assertRaisesRegex(coverage.CoverageError, "original, uninstrumented"):
                coverage.verify([raw], output, repaired=False)

    def test_generated_resources_and_bridge_are_outside_product_repair(self):
        with tempfile.TemporaryDirectory() as directory:
            raw, output = self.fixture(directory, instrumented=True)
            for relative in ("dev/taypeer/R.class", "dev/taypeer/R$string.class", "dev/taypeer/bridge/ReceivedSourceRow.class"):
                excluded = raw / relative
                excluded.parent.mkdir(parents=True, exist_ok=True)
                excluded.write_bytes(b"PUBLIC excluded file has no probes")
            self.assertEqual(coverage.verify([raw], output, repaired=True), 3)
            self.assertTrue(coverage.product_r_class(Path("dev/taypeer/RelaySettingsState.class")))
            self.assertFalse(coverage.product_r_class(Path("other/RelaySettingsState.class")))


class ScreenSettings(unittest.TestCase):
    def test_restores_exact_existing_overrides_after_test_exception(self):
        commands = []

        def fake_adb(env, serial, *args, **kwargs):
            commands.append(args)
            output = {("shell", "wm", "size"): "Physical size: 1080x2280\nOverride size: 720x1280\n",
                      ("shell", "wm", "density"): "Physical density: 440\nOverride density: 320\n"}.get(args, "")
            return subprocess.CompletedProcess(args, 0, output, "")

        with patch.object(qa, "adb", side_effect=fake_adb):
            with self.assertRaisesRegex(RuntimeError, "PUBLIC failure"):
                with qa.screen_size({}, "emulator-5554", "360x640"):
                    raise RuntimeError("PUBLIC failure")
        self.assertEqual(commands[-2:], [("shell", "wm", "size", "720x1280"),
                                         ("shell", "wm", "density", "320")])

    def test_original_physical_settings_are_reset_instead_of_added_overrides(self):
        commands = []

        def fake_adb(env, serial, *args):
            commands.append(args)
            return subprocess.CompletedProcess(args, 0, "Physical density: 440\n", "")

        with patch.object(qa, "adb", side_effect=fake_adb):
            with qa.screen_size({}, "emulator-5554", "844x390"):
                pass
        self.assertEqual(commands[-2:], [("shell", "wm", "size", "reset"),
                                         ("shell", "wm", "density", "reset")])


class ScreenshotArtifacts(unittest.TestCase):
    def archive(self, name, *, kind=tarfile.REGTYPE):
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w") as archive:
            directory = tarfile.TarInfo("qa-screenshots")
            directory.type = tarfile.DIRTYPE
            archive.addfile(directory)
            member = tarfile.TarInfo(name)
            member.type = kind
            member.size = 3 if kind == tarfile.REGTYPE else 0
            archive.addfile(member, io.BytesIO(b"PNG") if member.size else None)
        return buffer.getvalue()

    def test_extracts_only_test_owned_png_bytes(self):
        with tempfile.TemporaryDirectory() as folder:
            destination = Path(folder)
            self.assertEqual(qa.extract_screenshots(self.archive("qa-screenshots/welcome.png"), destination),
                             ["welcome.png"])
            self.assertEqual((destination / "welcome.png").read_bytes(), b"PNG")

    def test_rejects_profile_files_traversal_and_links(self):
        with tempfile.TemporaryDirectory() as folder:
            for name, kind in [("../outside.png", tarfile.REGTYPE),
                               ("/absolute.png", tarfile.REGTYPE),
                               ("qa-screenshots/profile.taypeer", tarfile.REGTYPE),
                               ("qa-screenshots/sub/welcome.png", tarfile.REGTYPE),
                               ("qa-screenshots/link.png", tarfile.SYMTYPE)]:
                with self.subTest(name=name), self.assertRaises(qa.QaError):
                    qa.extract_screenshots(self.archive(name, kind=kind), Path(folder))


class BiometryStage(unittest.TestCase):
    selection = "dev.taypeer.BiometricPromptCoordinatorTest#cancelRetiresPrivatePipeBeforeRealOsDismissalAndBlocksReplacementPrompt"

    def run_stage(self, folder, *, skipped=False, setup_error=False, restore_error=False):
        helper = Mock()
        def prepare(tools, env, serial, path, *, adopt):
            path.write_text("PUBLIC receipt")
            if setup_error:
                raise qa.QaError("PUBLIC unsupported Settings state")
        helper.prepare.side_effect = prepare
        helper.restore.return_value = {"restored": True}
        if restore_error:
            helper.restore.side_effect = qa.QaError("PUBLIC restoration failure")
        def instrument(env, serial, classes, arguments, output, timeout, **kwargs):
            name, _, method = self.selection.partition("#")
            qa.write_junit(output / "junit.xml", [{"class": name, "name": method,
                "status": "skipped" if skipped else "passed", "detail": "PUBLIC"}], True, "PUBLIC", 1)
            return True
        args = SimpleNamespace(biometry_state=None, timeout=30, coverage=False)
        with patch.object(qa, "enrollment_helper", return_value=helper), \
                patch.object(qa.threading, "Thread") as thread, \
                patch.object(qa, "adb", return_value=subprocess.CompletedProcess([], 0, "", "")), \
                patch.object(qa, "instrument", side_effect=instrument):
            thread.return_value.is_alive.return_value = False
            passed = qa.biometry_stage({}, "emulator-5554", [self.selection], args, Path(folder))
        helper.restore.assert_called_once()
        return passed, ET.parse(Path(folder) / "biometry/junit.xml").getroot()

    def test_real_cases_and_verified_restore_are_both_required(self):
        with tempfile.TemporaryDirectory() as folder:
            passed, junit = self.run_stage(folder)
            self.assertTrue(passed)
            self.assertEqual("0", junit.get("failures"))

    def test_setup_failure_still_restores_write_ahead_receipt(self):
        with tempfile.TemporaryDirectory() as folder:
            passed, junit = self.run_stage(folder, setup_error=True)
            self.assertFalse(passed)
            self.assertEqual("1", junit.get("failures"))

    def test_restore_failure_preserves_test_pass_but_fails_stage(self):
        with tempfile.TemporaryDirectory() as folder:
            passed, junit = self.run_stage(folder, restore_error=True)
            self.assertFalse(passed)
            self.assertEqual("2", junit.get("tests"))
            self.assertEqual("1", junit.get("failures"))

    def test_assumption_skip_cannot_be_a_biometry_pass(self):
        with tempfile.TemporaryDirectory() as folder:
            passed, junit = self.run_stage(folder, skipped=True)
            self.assertFalse(passed)
            self.assertEqual("1", junit.get("skipped"))
            self.assertEqual("1", junit.get("failures"))


class BiometryEnrollmentReceipt(unittest.TestCase):
    def setUp(self):
        self.helper = qa.enrollment_helper()
        self.current = {"serial": "emulator-5554", "boot": "PUBLIC-boot", "user": "0",
                        "api": 31, "fingerprint": "PUBLIC-image"}

    def test_write_ahead_receipt_exists_before_failed_first_pin_mutation(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "PUBLIC-state.json"
            def lock(tools, env, serial, command, *arguments, expected):
                if command == "set-pin":
                    state = json.loads(path.read_text())
                    self.assertEqual("prepared", state["phase"])
                    self.assertFalse(state["restored"])
                    raise qa.QaError("PUBLIC first mutation failed")
            with patch.object(self.helper, "session", return_value=self.current), \
                    patch.object(self.helper, "disabled", return_value="true"), \
                    patch.object(self.helper, "lock", side_effect=lock):
                with self.assertRaises(qa.QaError):
                    self.helper.prepare(qa, {}, "emulator-5554", path)
            self.assertEqual("prepared", json.loads(path.read_text())["phase"])

    def test_original_v1_owned_receipt_adopts_without_reenrollment(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "PUBLIC-state.json"
            path.write_text(json.dumps({"serial": "emulator-5554", "boot": "PUBLIC-boot",
                "previous_disabled": "true", "restored": False}))
            with patch.object(self.helper, "session", return_value=self.current), \
                    patch.object(self.helper, "disabled", return_value="false") as disabled, \
                    patch.object(self.helper, "lock") as lock, patch.object(qa, "adb") as adb:
                state = self.helper.prepare(qa, {}, "emulator-5554", path, adopt=True)
            lock.assert_called_once()
            self.assertEqual("verify", lock.call_args.args[3])
            self.assertEqual(("--old", "111111"), lock.call_args.args[4:])
            disabled.assert_called_once_with(qa, {}, "emulator-5554", old="111111")
            adb.assert_not_called()
            self.assertEqual(2, state["version"])
            self.assertEqual("public-pin-verified", state["phase"])

    def test_wrong_boot_receipt_never_authorizes_clearing_a_lock(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "PUBLIC-state.json"
            path.write_text(json.dumps({"serial": "emulator-5554", "boot": "PUBLIC-other-boot",
                "previous_disabled": "true", "restored": False}))
            with patch.object(self.helper, "session", return_value=self.current), \
                    patch.object(self.helper, "lock") as lock:
                with self.assertRaises(qa.QaError):
                    self.helper.restore(qa, {}, "emulator-5554", path)
            lock.assert_not_called()

    def test_restore_marks_receipt_only_after_public_pin_clear_and_empty_credential_verification(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "PUBLIC-state.json"
            path.write_text(json.dumps({"serial": "emulator-5554", "boot": "PUBLIC-boot",
                "previous_disabled": "true", "restored": False}))
            with patch.object(self.helper, "session", return_value=self.current), \
                    patch.object(self.helper, "disabled", side_effect=["false", "true"]), \
                    patch.object(self.helper, "empty_credential_verified", return_value=False), \
                    patch.object(self.helper, "lock") as lock, patch.object(qa, "adb"):
                result = self.helper.restore(qa, {}, "emulator-5554", path)
            self.assertEqual(["verify", "clear", "verify", "set-disabled"], [call.args[3] for call in lock.call_args_list])
            self.assertEqual((), lock.call_args_list[2].args[4:])
            self.assertTrue(result["restored"])

    def test_api31_secured_query_carries_verified_public_pin_before_legacy_adoption(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "PUBLIC-state.json"
            path.write_text(json.dumps({"serial": "emulator-5554", "boot": "PUBLIC-boot",
                "previous_disabled": "true", "restored": False}))
            commands = []
            def adb(env, serial, *arguments, **kwargs):
                commands.append(arguments)
                if arguments[2] == "verify" and arguments[-2:] == ("--old", "111111"):
                    return subprocess.CompletedProcess([], 0, "Lock credential verified successfully\n", "")
                if arguments[2] == "get-disabled" and arguments[-2:] == ("--old", "111111"):
                    return subprocess.CompletedProcess([], 0, "false\n", "")
                return subprocess.CompletedProcess([], 0, "", "IllegalArgumentException: Credential can't be null or empty")
            with patch.object(self.helper, "session", return_value=self.current), patch.object(qa, "adb", side_effect=adb):
                state = self.helper.prepare(qa, {}, "emulator-5554", path, adopt=True)
            self.assertEqual(["verify", "get-disabled"], [command[2] for command in commands])
            self.assertEqual("public-pin-verified", state["phase"])

    def test_write_ahead_restore_before_pin_requires_positive_empty_credential(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "PUBLIC-state.json"
            path.write_text(json.dumps({"version": 2, **self.current,
                "previous_disabled": "true", "restored": False, "phase": "prepared"}))
            with patch.object(self.helper, "session", return_value=self.current), \
                    patch.object(self.helper, "empty_credential_verified", return_value=True), \
                    patch.object(self.helper, "disabled", return_value="true"), \
                    patch.object(self.helper, "lock") as lock, patch.object(qa, "adb"):
                result = self.helper.restore(qa, {}, "emulator-5554", path)
            self.assertEqual(["verify", "set-disabled"], [call.args[3] for call in lock.call_args_list])
            self.assertTrue(result["restored"])

    def test_restored_receipt_cannot_clear_later_lock_even_with_same_public_pin(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "PUBLIC-state.json"
            path.write_text(json.dumps({"serial": "emulator-5554", "boot": "PUBLIC-boot",
                "previous_disabled": "true", "restored": True}))
            with patch.object(self.helper, "session", return_value=self.current), \
                    patch.object(self.helper, "empty_credential_verified", return_value=False), \
                    patch.object(self.helper, "lock") as lock:
                with self.assertRaises(qa.QaError):
                    self.helper.restore(qa, {}, "emulator-5554", path)
            lock.assert_not_called()

    def test_fresh_setup_refuses_api31_unknown_secure_query_without_mutation(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "PUBLIC-state.json"
            with patch.object(self.helper, "session", return_value=self.current), \
                    patch.object(qa, "adb", return_value=subprocess.CompletedProcess([], 0, "", "PUBLIC credential error")), \
                    patch.object(self.helper, "lock") as lock:
                with self.assertRaises(qa.QaError):
                    self.helper.prepare(qa, {}, "emulator-5554", path)
            self.assertFalse(path.exists())
            lock.assert_not_called()

    def test_enrollment_waits_for_fresh_settings_and_never_inputs_on_launcher(self):
        projections = iter([
            '<hierarchy><node package="com.google.android.apps.nexuslauncher" password="true" '
            'text="NEXT" clickable="true" bounds="[0,0][48,48]"/></hierarchy>',
            '<hierarchy><node package="com.android.settings" text="Fingerprint added" '
            'resource-id="com.android.settings:id/suc_layout_title"/>'
            '<node package="com.android.settings" text="DONE" clickable="true" '
            'bounds="[0,0][48,48]"/></hierarchy>',
        ])
        commands = []
        def adb(env, serial, *arguments, **kwargs):
            commands.append(arguments)
            output = next(projections) if arguments[:2] == ("exec-out", "cat") else ""
            return subprocess.CompletedProcess([], 0, output, "")
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "PUBLIC-state.json"
            with patch.object(self.helper, "session", return_value=self.current), \
                    patch.object(self.helper, "disabled", return_value="true"), \
                    patch.object(self.helper, "lock"), patch.object(qa, "adb", side_effect=adb), \
                    patch.object(self.helper.time, "sleep"):
                state = self.helper.prepare(qa, {}, "emulator-5554", path)
            self.assertEqual("enrolled", state["phase"])
            self.assertIn(("shell", "am", "start", "-S", "-W", "-a", "android.settings.FINGERPRINT_ENROLL"), commands)
            self.assertFalse(any(command[:3] == ("shell", "input", "text") for command in commands))
            self.assertEqual(1, sum(command[:3] == ("shell", "input", "tap") for command in commands))

    def test_unknown_settings_state_still_fails_and_retains_restoration_receipt(self):
        projection = '<hierarchy><node package="com.android.settings" text="PUBLIC unknown state"/></hierarchy>'
        def adb(env, serial, *arguments, **kwargs):
            return subprocess.CompletedProcess([], 0, projection if arguments[:2] == ("exec-out", "cat") else "", "")
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "PUBLIC-state.json"
            with patch.object(self.helper, "session", return_value=self.current), \
                    patch.object(self.helper, "disabled", return_value="true"), \
                    patch.object(self.helper, "lock"), patch.object(qa, "adb", side_effect=adb):
                with self.assertRaisesRegex(qa.QaError, "unsupported state"):
                    self.helper.prepare(qa, {}, "emulator-5554", path)
            state = json.loads(path.read_text())
            self.assertEqual("pin-set", state["phase"])
            self.assertFalse(state["restored"])


class SuiteBoundaries(unittest.TestCase):
    def test_network_suite_uses_real_android_protocol_tests(self):
        with tempfile.TemporaryDirectory() as folder:
            project = Path(folder)
            sources = project / "app/src/androidTest/java/dev/taypeer"
            sources.mkdir(parents=True)
            for name in ("P2PTest", "ExchangeTest", "ReceivedSourcesTest", "AndroidVisualTest", "DocumentUiTest", "PasswordChangeUiTest", "HistoryUiTest", "SecretFocusUiTest"):
                (sources / f"{name}.kt").touch()
            with patch.object(qa, "PROJECT", project):
                self.assertEqual(set(qa.test_classes("network")),
                                 {"dev.taypeer.P2PTest", "dev.taypeer.ExchangeTest", "dev.taypeer.ReceivedSourcesTest"})
                self.assertNotIn("dev.taypeer.ReceivedSourcesTest", qa.test_classes("platform"))
                self.assertTrue({"dev.taypeer.PasswordChangeUiTest", "dev.taypeer.HistoryUiTest", "dev.taypeer.SecretFocusUiTest"}
                                <= set(qa.test_classes("ui")))
                self.assertTrue({"dev.taypeer.PasswordChangeUiTest", "dev.taypeer.HistoryUiTest", "dev.taypeer.SecretFocusUiTest"}
                                .isdisjoint(qa.test_classes("platform")))
        self.assertNotIn(qa.VISUAL_CLASS, qa.test_classes("full"))
        self.assertTrue({"dev.taypeer.InvitationUiTest", "dev.taypeer.PreferencesUiTest", "dev.taypeer.QrCameraTest"}
                        <= set(qa.test_classes("ui")))

    def test_biometry_stays_in_full_and_routes_out_of_unenrolled_platform(self):
        name = "dev.taypeer.BiometricPromptCoordinatorTest"
        self.assertIn(name, qa.test_classes("full"))
        self.assertNotIn(name, qa.test_classes("platform"))
        self.assertEqual({"dev.taypeer." + declared for declared in qa.BIOMETRIC_CLASSES}, set(qa.test_classes("biometry")))
        method = name + "#cancelRetiresPrivatePipeBeforeRealOsDismissalAndBlocksReplacementPrompt"
        self.assertEqual((["dev.taypeer.DocumentTest"], [method]),
                         qa.split_biometry(["dev.taypeer.DocumentTest", method]))
        with self.assertRaises(qa.QaError):
            qa.split_biometry([name + "#PUBLIC_undeclared"])

    def test_explicit_biometry_requires_authorization_before_install(self):
        args = SimpleNamespace(command="test", suite="full",
            test_class=["dev.taypeer.BiometricPromptCoordinatorTest"], authorize_public_biometry=False)
        with patch.object(qa, "devices", return_value={"emulator-5554": "device"}), \
                patch.object(qa, "metadata", return_value={"abi": "arm64-v8a", "api": 31}), \
                patch.object(qa, "install") as install:
            with self.assertRaises(qa.QaError):
                qa.target({}, "emulator-5554", args, Path("PUBLIC-unused"))
            install.assert_not_called()

    def test_physical_devices_are_rejected_before_install(self):
        with patch.object(qa, "install") as install:
            with self.assertRaisesRegex(qa.QaError, "physical-device"):
                qa.target({}, "PUBLIC-hardware-serial", None, Path("unused"))
            install.assert_not_called()


class VisualProfiles(unittest.TestCase):
    def test_matrix_passes_actual_content_dimensions_to_instrumentation(self):
        sizes = {name: qa.visual_arguments(locale, theme, font, size) for name, locale, theme, font, size in qa.VISUAL_CASES}
        self.assertEqual((sizes["ru-light-small"]["qaWidthDp"], sizes["ru-light-small"]["qaHeightDp"]), ("360", "584"))
        self.assertEqual((sizes["ru-dark-landscape"]["qaWidthDp"], sizes["ru-dark-landscape"]["qaHeightDp"]), ("844", "334"))
        self.assertEqual(sizes["ru-dark-font200"]["qaFontScale"], "2")

    def test_fixed_size_capture_cannot_claim_to_pass_a_different_profile(self):
        from PIL import Image
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)
            Image.new("RGBA", (390, 788), "white").save(path / "public.png")
            with self.assertRaisesRegex(qa.QaError, "Wrong screenshot size"):
                qa.validate_screenshots(path, ["public.png"], qa.visual_arguments("ru", "light", "1", "360x640"))

    def test_regression_compare_fails_visible_drift_and_reports_exact_fraction(self):
        from PIL import Image
        with tempfile.TemporaryDirectory() as folder:
            actual, baseline = Path(folder) / "actual", Path(folder) / "baseline"
            actual.mkdir(); baseline.mkdir()
            capture = {"locale": "en", "theme": "light", "font_scale": 1., "screen_dp": "2x58", "screenshots": ["public.png"]}
            (baseline / "capture.json").write_text(json.dumps(capture))
            image = Image.new("RGBA", (2, 2), "white")
            image.save(baseline / "public.png")
            image.putpixel((0, 0), (0, 0, 0, 255))
            image.save(actual / "public.png")
            self.assertFalse(qa.compare_baseline(actual, baseline, ["public.png"], capture, tolerance=0, max_difference=0))
            report = json.loads((actual / "comparison.json").read_text())
            self.assertEqual(report["images"][0]["difference_fraction"], .25)
            self.assertTrue((actual / "public-diff.png").is_file())
            self.assertTrue(qa.compare_baseline(actual, baseline, ["public.png"], capture, tolerance=0, max_difference=.25))

    def test_compare_honors_channel_tolerance_and_rejects_profile_mismatch(self):
        from PIL import Image
        with tempfile.TemporaryDirectory() as folder:
            actual, baseline = Path(folder) / "actual", Path(folder) / "baseline"
            actual.mkdir(); baseline.mkdir()
            capture = {"locale": "en", "theme": "light", "font_scale": 1., "screen_dp": "1x57", "screenshots": ["public.png"]}
            (baseline / "capture.json").write_text(json.dumps(capture))
            Image.new("RGBA", (1, 1), (255, 255, 255, 255)).save(baseline / "public.png")
            Image.new("RGBA", (1, 1), (254, 255, 255, 255)).save(actual / "public.png")
            self.assertTrue(qa.compare_baseline(actual, baseline, ["public.png"], capture, tolerance=1, max_difference=0))
            with self.assertRaisesRegex(qa.QaError, "profile mismatch"):
                qa.compare_baseline(actual, baseline, ["public.png"], capture | {"theme": "dark"}, tolerance=0, max_difference=0)


class CameraPermissionLifetime(unittest.TestCase):
    def test_revoke_precedes_scenario_and_original_grant_restored_after_error(self):
        commands = []

        def fake_adb(env, serial, *args, **kwargs):
            commands.append(args)
            output = "android.permission.CAMERA: granted=true" if args[1] == "dumpsys" else ""
            return subprocess.CompletedProcess(args, 0, output, "")

        with patch.object(qa, "adb", side_effect=fake_adb):
            with self.assertRaisesRegex(RuntimeError, "PUBLIC interrupted"):
                with qa.camera_denied({}, "emulator-5554") as arguments:
                    self.assertEqual(arguments, {})
                    self.assertEqual(commands[-2], ("shell", "pm", "revoke", qa.PACKAGE, qa.CAMERA_PERMISSION))
                    raise RuntimeError("PUBLIC interrupted")
        self.assertEqual(commands[-1], ("shell", "pm", "grant", qa.PACKAGE, qa.CAMERA_PERMISSION))

    def test_unsupported_android12_flag_command_uses_only_the_camera_test_api(self):
        commands = []

        def fake_adb(env, serial, *args, **kwargs):
            commands.append(args)
            if "clear-permission-flags" in args:
                self.assertFalse(kwargs["check"])
                return subprocess.CompletedProcess(args, 255, "", "Unknown command: clear-permission-flags\n")
            return subprocess.CompletedProcess(args, 0, "android.permission.CAMERA: granted=true", "")

        with patch.object(qa, "adb", side_effect=fake_adb):
            with qa.camera_denied({}, "emulator-5554") as arguments:
                self.assertEqual(arguments, {"qaResetCameraPermissionFlags": "true"})
        self.assertEqual(commands[-1], ("shell", "pm", "grant", qa.PACKAGE, qa.CAMERA_PERMISSION))
        self.assertFalse(any("reset-permissions" in command or "clear" in command for command in commands))

    def test_permission_flag_access_failure_is_not_treated_as_missing_command(self):
        commands = []

        def fake_adb(env, serial, *args, **kwargs):
            commands.append(args)
            if "clear-permission-flags" in args:
                return subprocess.CompletedProcess(args, 1, "", "PUBLIC permission denied")
            return subprocess.CompletedProcess(args, 0, "android.permission.CAMERA: granted=false", "")

        with patch.object(qa, "adb", side_effect=fake_adb):
            with self.assertRaisesRegex(qa.QaError, "permission denied"):
                with qa.camera_denied({}, "emulator-5554"):
                    self.fail("Denied flag reset must not start instrumentation")
        self.assertEqual(commands[-1], ("shell", "pm", "revoke", qa.PACKAGE, qa.CAMERA_PERMISSION))


class LocalRelayLifetime(unittest.TestCase):
    readiness = '{"url":"https://127.0.0.1:43210", "root_der":[1,2,3]}'

    def test_public_certificate_is_forwarded_and_own_mapping_removed_after_test_failure(self):
        process = Mock()
        with tempfile.TemporaryDirectory() as folder, \
                patch.object(qa.subprocess, "Popen", return_value=process), \
                patch.object(qa, "read_relay_ready", return_value=self.readiness), \
                patch.object(qa, "adb") as adb, patch.object(qa, "stop_relay") as stop:
            with self.assertRaisesRegex(RuntimeError, "PUBLIC test failure"):
                with qa.local_relay({}, "emulator-5554", Path(folder)) as arguments:
                    self.assertEqual(arguments, {"relayUrl": "https://127.0.0.1:43210", "relayCertificate": "AQID"})
                    raise RuntimeError("PUBLIC test failure")
            self.assertEqual([call.args[2:] for call in adb.call_args_list],
                             [("reverse", "--no-rebind", "tcp:43210", "tcp:43210"),
                              ("reverse", "--remove", "tcp:43210")])
            stop.assert_called_once_with(process)

    def test_existing_mapping_is_not_replaced_or_removed(self):
        process = Mock()
        with tempfile.TemporaryDirectory() as folder, \
                patch.object(qa.subprocess, "Popen", return_value=process), \
                patch.object(qa, "read_relay_ready", return_value=self.readiness), \
                patch.object(qa, "adb", side_effect=qa.QaError("PUBLIC mapping exists")) as adb, \
                patch.object(qa, "stop_relay") as stop:
            with self.assertRaisesRegex(qa.QaError, "mapping exists"):
                with qa.local_relay({}, "emulator-5554", Path(folder)):
                    self.fail("Mapping conflict must prevent instrumentation")
            self.assertEqual(adb.call_count, 1)
            self.assertIn("--no-rebind", adb.call_args.args)
            stop.assert_called_once_with(process)

    def test_failed_cleanup_still_stops_the_owned_fixture(self):
        process = Mock()
        with tempfile.TemporaryDirectory() as folder, \
                patch.object(qa.subprocess, "Popen", return_value=process), \
                patch.object(qa, "read_relay_ready", return_value=self.readiness), \
                patch.object(qa, "adb", side_effect=[None, qa.QaError("PUBLIC remove failed")]), \
                patch.object(qa, "stop_relay") as stop:
            with self.assertRaisesRegex(qa.QaError, "remove failed"):
                with qa.local_relay({}, "emulator-5554", Path(folder)):
                    pass
            stop.assert_called_once_with(process)

    def test_remote_or_non_tls_fixture_endpoints_are_rejected(self):
        for url in ["http://127.0.0.1:1234", "https://remote.example:1234", "https://localhost"]:
            with self.subTest(url=url), self.assertRaises(qa.QaError):
                qa.relay_parameters(json.dumps({"url": url, "root_der": [1]}))

    def test_network_arguments_select_relay_only_for_the_requiring_protocol_scenario(self):
        with patch.object(qa, "local_relay") as fixture:
            fixture.return_value.__enter__.return_value = {"PUBLIC": "relay"}
            with qa.network_arguments({}, "emulator-5554", ["dev.taypeer.P2PTest"], Path("unused")) as arguments:
                self.assertEqual(arguments, {"PUBLIC": "relay"})
            fixture.assert_called_once()
        with patch.object(qa, "local_relay") as fixture:
            with qa.network_arguments({}, "emulator-5554", ["dev.taypeer.ExchangeTest"], Path("unused")) as arguments:
                self.assertEqual(arguments, {})
            fixture.assert_not_called()

    def test_received_source_suite_and_targeted_methods_get_the_strict_relay_fixture(self):
        for selection in ("dev.taypeer.ReceivedSourcesTest", "dev.taypeer.ReceivedSourcesTest#PUBLIC_lateOfflineCiphertext"):
            with self.subTest(selection=selection), patch.object(qa, "local_relay") as fixture:
                fixture.return_value.__enter__.return_value = {"PUBLIC": "relay"}
                with qa.network_arguments({}, "emulator-5554", [selection], Path("unused")) as arguments:
                    self.assertEqual(arguments, {"PUBLIC": "relay"})
                fixture.assert_called_once()

    def test_ready_fixture_receives_newline_and_waits_for_orderly_shutdown(self):
        process = Mock()
        process.poll.return_value = None
        qa.stop_relay(process)
        process.stdin.write.assert_called_once_with(b"\n")
        process.stdin.flush.assert_called_once()
        process.communicate.assert_called_once_with(timeout=15)


class FrozenRelayEvidence(unittest.TestCase):
    def capture(self, folder):
        path = Path(folder)
        executable = path / "public-relay-fixture"
        executable.write_bytes(b"PUBLIC captured test binary")
        executable.chmod(0o700)
        captured = {"artifact": "public-relay-fixture", "sha256": qa.hashlib.sha256(executable.read_bytes()).hexdigest(),
                    "app_apk_sha256": "PUBLIC app", "test_apk_sha256": "PUBLIC tests"}
        (path / "manifest.json").write_text(json.dumps(captured))
        (path / "environment.json").write_text(json.dumps({"apk_sha256": {"app": "PUBLIC app", "tests": "PUBLIC tests"}}))
        return path, executable

    def test_matching_capture_launches_without_rebuilding_and_records_exact_hash(self):
        with tempfile.TemporaryDirectory() as folder:
            path, executable = self.capture(folder)
            self.assertEqual(qa.relay_command(executable, path), [str(executable.resolve())])
            self.assertEqual(json.loads((path / "relay-fixture.json").read_text())["sha256"], qa.hashlib.sha256(executable.read_bytes()).hexdigest())
        self.assertEqual(qa.relay_command(None, Path("unused"))[:2], ["cargo", "run"])

    def test_changed_binary_or_apks_refuse_before_any_fixture_process(self):
        for change in ("binary", "apks"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as folder:
                path, executable = self.capture(folder)
                if change == "binary":
                    executable.write_bytes(b"PUBLIC altered binary")
                else:
                    (path / "environment.json").write_text(json.dumps({"apk_sha256": {"app": "PUBLIC other", "tests": "PUBLIC tests"}}))
                with self.assertRaises(qa.QaError): qa.relay_command(executable, path)
                self.assertFalse((path / "relay-fixture.json").exists())

    def test_nonexecutable_or_oversized_manifest_is_rejected(self):
        for change in ("mode", "manifest"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as folder:
                path, executable = self.capture(folder)
                if change == "mode": executable.chmod(0o600)
                else: (path / "manifest.json").write_bytes(b" " * 4097)
                with self.assertRaises(qa.QaError): qa.relay_command(executable, path)


class FrozenCoverageSourceEvidence(unittest.TestCase):
    def capture(self, folder):
        project = Path(folder) / "project"
        apk = project / "app/build/outputs/apk/debug/app-debug.apk"
        apk.parent.mkdir(parents=True)
        apk.write_bytes(b"PUBLIC product APK")
        source = Path(folder) / "capture/java"
        source.mkdir(parents=True)
        (source / "Public.kt").write_bytes(b"PUBLIC captured source")
        manifest = {"version": 1, "app_apk_sha256": qa.hashlib.sha256(apk.read_bytes()).hexdigest(),
                    "files": {"Public.kt": qa.hashlib.sha256((source / "Public.kt").read_bytes()).hexdigest()}}
        (source.parent / "manifest.json").write_text(json.dumps(manifest))
        return project, apk, source

    def test_valid_capture_survives_workspace_source_changes(self):
        with tempfile.TemporaryDirectory() as folder:
            project, apk, source = self.capture(folder)
            current = project / "app/src/main/java/Public.kt"
            current.parent.mkdir(parents=True)
            current.write_bytes(b"PUBLIC later implementation")
            with patch.object(qa, "PROJECT", project):
                self.assertEqual(qa.frozen_coverage_sources(source), source.resolve())

    def test_changed_source_or_product_apk_cannot_be_reported_as_the_capture(self):
        for change in ("source", "apk"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as folder:
                project, apk, source = self.capture(folder)
                (apk if change == "apk" else source / "Public.kt").write_bytes(b"PUBLIC different generation")
                with patch.object(qa, "PROJECT", project), self.assertRaises(qa.QaError):
                    qa.frozen_coverage_sources(source)


class CoverageEvidence(unittest.TestCase):
    def test_coverage_removes_stale_data_then_pulls_only_the_private_execution_file(self):
        commands = []

        def fake_adb(env, serial, *args, **kwargs):
            commands.append(args)
            if args[:2] == ("shell", "am"):
                return subprocess.CompletedProcess(args, 0, status(0) + "INSTRUMENTATION_CODE: -1\n", "")
            if args[0] == "exec-out":
                return subprocess.CompletedProcess(args, 0, b"PUBLIC coverage bytes", b"")
            return subprocess.CompletedProcess(args, 0, "", "")

        with tempfile.TemporaryDirectory() as folder, patch.object(qa, "adb", side_effect=fake_adb), redirect_stdout(io.StringIO()):
            path = Path(folder)
            self.assertTrue(qa.instrument({}, "emulator-5554", ["dev.taypeer.PublicTest"], {}, path, 30, coverage=True))
            self.assertEqual((path / "instrumentation.ec").read_bytes(), b"PUBLIC coverage bytes")
        self.assertEqual(commands[0], ("shell", "run-as", qa.PACKAGE, "rm", "-f", "files/qa-coverage.ec"))
        self.assertIn("coverageFile", commands[1])
        self.assertIn(f"/data/data/{qa.PACKAGE}/files/qa-coverage.ec", commands[1])
        self.assertEqual(commands[-1], ("exec-out", "run-as", qa.PACKAGE, "cat", "files/qa-coverage.ec"))


if __name__ == "__main__":
    unittest.main()
