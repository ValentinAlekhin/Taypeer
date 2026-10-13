#!/usr/bin/env python3
"""Build once, run explicit ARM64 emulator suites, and retain reviewable evidence."""

from __future__ import annotations

import argparse
import base64
import codecs
import hashlib
import io
import importlib.util
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import threading
import time
from types import SimpleNamespace
import xml.etree.ElementTree as ET
from urllib.parse import urlsplit
from contextlib import contextmanager
from datetime import datetime, timezone


PROJECT = Path(__file__).resolve().parent
REPOSITORY = PROJECT.parent.parent
PACKAGE = "dev.taypeer"
RUNNER = f"{PACKAGE}.test/{PACKAGE}.TaypeerTestRunner"
VISUAL_CLASS = f"{PACKAGE}.AndroidVisualTest"
CAMERA_DENIED_CLASS = f"{PACKAGE}.QrPermissionScenario"
CAMERA_PERMISSION = "android.permission.CAMERA"
UI_CLASSES = {"DocumentUiTest", "GeneratorTest", "InvitationUiTest", "PreferencesUiTest", "QrCameraTest", "AndroidNavigationTest", "AndroidLifecycleUiTest", "RelaySettingsUiTest", "IncomingUiTest", "PasswordChangeUiTest", "HistoryUiTest", "SecretFocusUiTest", "SecretAccessUiTest", "ImageIconUiTest", "GroupParentUiTest", "RecoveryUiTest", "ManagementUiTest", "ManagementNavigationUiTest", "CiphertextCopyUiTest", "ContextActionsUiTest", "SearchUiTest", "MetadataHistoryUiTest", "PolicyUiTest", "GroupFaviconsUiTest", "GroupFaviconsNavigationUiTest"}
NETWORK_CLASSES = {"P2PTest", "ExchangeTest", "ReceivedSourcesTest", "ManagementNetworkTest"}
BIOMETRIC_TESTS = {"BiometricPromptCoordinatorTest": {
    "cancelRetiresPrivatePipeBeforeRealOsDismissalAndBlocksReplacementPrompt",
    "recreationRebindsSameRequestAndRealStrongCryptoObjectTransfersOnlyOnce",
}, "ProductBiometryUiTest": {
    "realEnrollmentAndKeyedOpenKeepSavedContentAndDisableRequiresExactGeneration",
    "realOsNegativeButtonNeverEnablesSetupAndMasterFallbackKeepsTheFile",
    "backgroundAfterRealOsSuccessBeforeProviderCannotPublishOrReviveAndMasterReopens",
}, "ProductBiometryStateTest": {
    "lateOldPreparationCannotDisownNewAttemptOrPublishAfterItsIndependentClear",
}}
BIOMETRIC_CLASSES = set(BIOMETRIC_TESTS)
VISUAL_CASES = (
    ("ru-dark", "ru", "dark", "1", "390x844"),
    ("en-light", "en", "light", "1", "390x844"),
    ("ru-light-small", "ru", "light", "1", "360x640"),
    ("en-dark-large", "en", "dark", "1", "430x932"),
    ("ru-dark-landscape", "ru", "dark", "1", "844x390"),
    ("ru-dark-font200", "ru", "dark", "2", "390x844"),
)


class QaError(RuntimeError):
    pass


def require_pillow():
    try:
        import PIL
    except ImportError as error:
        raise QaError("Visual QA needs Pillow: python3 -m pip install -r "
                      "apps/taypeer-android/requirements-qa.txt (from repository root)") from error


def run(command: list[str], *, env: dict[str, str], timeout: int = 60,
        log: Path | None = None, check: bool = True, binary: bool = False):
    process = subprocess.Popen(command, cwd=PROJECT, env=env, start_new_session=True,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=not binary)
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            stdout, stderr = process.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate()
        if log:
            output = stdout + stderr
            log.write_bytes(output if binary else output.encode())
        raise subprocess.TimeoutExpired(command, timeout, output=stdout, stderr=stderr)
    result = subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
    if log:
        output = result.stdout + result.stderr
        log.write_bytes(output if binary else output.encode())
    if check and result.returncode:
        detail = result.stderr if binary else (result.stderr or result.stdout)[-2000:]
        raise QaError(f"{Path(command[0]).name} exited {result.returncode}: {detail}")
    return result


class InstrumentationProgress:
    """Informational, allowlisted evidence; only the existing terminal parser decides success."""
    PHASES = {"create", "title", "image", "save", "cleanup", "clone"}

    def __init__(self, classes: list[str], output: Path, serial: str):
        self.selected = {}
        for selection in classes:
            name, _, method = selection.partition("#")
            if re.fullmatch(r"dev\.taypeer\.[A-Za-z][A-Za-z0-9_]{0,100}", name):
                source = PROJECT / "app/src/androidTest/java/dev/taypeer" / (name.rsplit(".", 1)[1] + ".kt")
                declared = set(re.findall(r"@Test(?:\([^\n]*\))?\s+fun\s+([A-Za-z_][A-Za-z0-9_]*)", source.read_text())) if source.is_file() else set()
                self.selected.setdefault(name, set()).update(declared & {method} if method else declared)
        self.path = output / "progress.json"
        self.serial = serial
        self.started = time.monotonic()
        self.last_print = self.started - 15
        self.fields: dict[str, str] = {}
        self.line = ""
        self.discard_line = False
        self.latest: dict[str, str | int] = {"run": "running"}
        self.timed_out = False
        self.tick(force_write=True)

    def feed(self, text: str):
        for part in text.splitlines(keepends=True):
            if not self.discard_line:
                self.line += part
                if len(self.line) > 4096:
                    self.line = ""
                    self.discard_line = True
            if part.endswith("\n"):
                if not self.discard_line:
                    self.accept_line(self.line.rstrip("\r\n"))
                self.line = ""
                self.discard_line = False

    def accept_line(self, line: str):
        match = re.fullmatch(r"INSTRUMENTATION_STATUS: (class|test|qa_phase|qa_row|qa_elapsed_ms)=(.{0,256})", line)
        if match:
            key, value = match.groups()
            self.fields[key] = value
            return
        code = re.fullmatch(r"INSTRUMENTATION_STATUS_CODE:\s*(-?\d{1,2})\s*", line)
        if not code:
            return
        fields, self.fields = self.fields, {}
        name, method = fields.get("class", ""), fields.get("test", "")
        selected = self.selected.get(name)
        if selected is None or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,179}", method):
            return
        if method not in selected:
            return
        status = {1: "started", 0: "passed", -1: "failed", -2: "failed", -3: "skipped", -4: "skipped"}.get(int(code[1]))
        event: dict[str, str | int] = {"class": name, "method": method, "run": "running"}
        if int(code[1]) == 2:
            row, elapsed = fields.get("qa_row", ""), fields.get("qa_elapsed_ms", "")
            if (fields.get("qa_phase") not in self.PHASES
                    or not re.fullmatch(r"\d{1,2}", row) or int(row) > 69
                    or not re.fullmatch(r"\d{1,8}", elapsed) or int(elapsed) > 86_400_000):
                return
            event.update(status="running", phase=fields["qa_phase"], row=int(row), fixture_elapsed_ms=int(elapsed))
        elif status:
            event["status"] = status
        else:
            return
        self.latest = event
        self.tick()

    def tick(self, *, force_write: bool = False):
        now = time.monotonic()
        due = now - self.last_print >= 15
        if not self.latest and not force_write:
            return
        state = self.latest | {"host_elapsed_ms": int((now - self.started) * 1000)}
        temporary = self.path.with_suffix(".json.tmp")
        temporary.write_text(json.dumps(state, sort_keys=True) + "\n", encoding="utf-8")
        temporary.replace(self.path)
        if due and "class" in self.latest and "method" in self.latest:
            case = self.latest["class"] + "#" + self.latest["method"]
            phase = f" row={self.latest['row']} phase={self.latest['phase']}" if "row" in self.latest else ""
            print(f"{self.serial}: {case} {self.latest['status']}{phase} elapsed_ms={state['host_elapsed_ms']}", flush=True)
            self.last_print = now

    def finish(self, success: bool):
        self.latest["run"] = "timed_out" if self.timed_out else "passed" if success else "failed"
        self.tick(force_write=True)


INSTRUMENTATION_TERM_GRACE = 5
INSTRUMENTATION_KILL_GRACE = 1


def run_instrumentation(command: list[str], *, env: dict[str, str], timeout: int,
                        progress: InstrumentationProgress):
    """Drain both owned pipes live; retain complete output and bound timeout teardown."""
    process = subprocess.Popen(command, cwd=PROJECT, env=env, start_new_session=True,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    chunks = {"stdout": [], "stderr": []}
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    deadline = time.monotonic() + timeout
    terminate_at = None
    kill_at = None

    def signal_owned(kind):
        try:
            os.killpg(process.pid, kind)
        except ProcessLookupError:
            pass

    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ, "stdout")
            selector.register(process.stderr, selectors.EVENT_READ, "stderr")
            while selector.get_map() or process.poll() is None:
                now = time.monotonic()
                if terminate_at is None and now >= deadline:
                    progress.timed_out = True
                    signal_owned(signal.SIGTERM)
                    terminate_at = now
                if terminate_at is not None and kill_at is None and now - terminate_at >= INSTRUMENTATION_TERM_GRACE:
                    signal_owned(signal.SIGKILL)
                    kill_at = now
                if kill_at is not None and now - kill_at >= INSTRUMENTATION_KILL_GRACE:
                    break
                next_deadline = (kill_at + INSTRUMENTATION_KILL_GRACE if kill_at is not None else
                                 terminate_at + INSTRUMENTATION_TERM_GRACE if terminate_at is not None else deadline)
                for key, _ in selector.select(timeout=min(1, max(0, next_deadline - now))):
                    part = os.read(key.fileobj.fileno(), 65536)
                    if not part:
                        selector.unregister(key.fileobj)
                        continue
                    chunks[key.data].append(part)
                    if key.data == "stdout":
                        progress.feed(decoder.decode(part))
                progress.tick()
        process.wait(timeout=INSTRUMENTATION_KILL_GRACE)
    finally:
        if process.poll() is None:
            signal_owned(signal.SIGKILL)
            process.wait(timeout=INSTRUMENTATION_KILL_GRACE)
        process.stdout.close()
        process.stderr.close()
    stdout = b"".join(chunks["stdout"]).decode("utf-8", errors="replace")
    stderr = b"".join(chunks["stderr"]).decode("utf-8", errors="replace")
    if progress.timed_out:
        raise subprocess.TimeoutExpired(command, timeout, output=stdout, stderr=stderr)
    return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


def environment() -> dict[str, str]:
    env = os.environ.copy()
    env.setdefault("ANDROID_HOME", str(PROJECT / ".tools/android-sdk"))
    env.setdefault("ANDROID_AVD_HOME", str(PROJECT / ".tools/avd"))
    env.setdefault("ANDROID_USER_HOME", str(PROJECT / ".tools/android-user"))
    env.setdefault("GRADLE_USER_HOME", str(PROJECT / ".tools/gradle-cache"))
    return env


def adb_path(env: dict[str, str]) -> str:
    path = Path(env["ANDROID_HOME"]) / "platform-tools/adb"
    if not path.is_file():
        raise QaError(f"Missing adb: {path}. Install SDK platform-tools first.")
    return str(path)


def adb(env: dict[str, str], serial: str | None, *args: str, **kwargs):
    command = [adb_path(env)]
    if serial:
        command.extend(["-s", serial])
    progress = kwargs.pop("instrumentation_progress", None)
    if progress is not None:
        return run_instrumentation(command + list(args), env=env, progress=progress, **kwargs)
    return run(command + list(args), env=env, **kwargs)


def devices(env: dict[str, str]) -> dict[str, str]:
    text = adb(env, None, "devices").stdout
    return dict(line.split()[:2] for line in text.splitlines()[1:] if "\t" in line)


def metadata(env: dict[str, str], serial: str) -> dict[str, str | int]:
    commands = {
        "api": ("getprop", "ro.build.version.sdk"),
        "abi": ("getprop", "ro.product.cpu.abi"),
        "page_size": ("getconf", "PAGE_SIZE"),
        "size": ("wm", "size"),
        "density": ("wm", "density"),
        "fingerprint": ("getprop", "ro.build.fingerprint"),
    }
    info: dict[str, str | int] = {"serial": serial}
    for key, command in commands.items():
        output = adb(env, serial, "shell", *command).stdout.strip()
        info[key] = int(output) if key in {"api", "page_size"} else output
    return info


def test_classes(suite: str) -> list[str]:
    names = sorted(path.stem for path in (PROJECT / "app/src/androidTest/java/dev/taypeer").glob("*Test.kt")
                   if path.stem != "AndroidVisualTest")
    if suite == "ui":
        names = [name for name in names if name in UI_CLASSES]
    elif suite == "network":
        names = [name for name in names if name in NETWORK_CLASSES]
    elif suite == "platform":
        names = [name for name in names if name not in UI_CLASSES | NETWORK_CLASSES | BIOMETRIC_CLASSES]
    elif suite == "biometry":
        names = [name for name in names if name in BIOMETRIC_CLASSES]
    return [f"{PACKAGE}.{name}" for name in names]


def split_biometry(classes: list[str]) -> tuple[list[str], list[str]]:
    ordinary, biometric = [], []
    for selected in classes:
        name, _, method = selected.partition("#")
        short = name.removeprefix(PACKAGE + ".")
        if name == PACKAGE + "." + short and short in BIOMETRIC_CLASSES:
            if method and method not in BIOMETRIC_TESTS[short]:
                raise QaError("Select an exact declared biometric test method")
            biometric.append(selected)
        else:
            ordinary.append(selected)
    return ordinary, biometric


def biometric_cases(classes: list[str]) -> set[tuple[str, str]]:
    _, selected = split_biometry(classes)
    return {(name, method) for selection in selected
            for name, _, requested in [selection.partition("#")]
            for method in ({requested} if requested else BIOMETRIC_TESTS[name.rsplit(".", 1)[1]])}


def parse_instrumentation(output: str) -> tuple[list[dict[str, str]], bool]:
    """adb itself can exit zero while instrumentation tests failed or crashed."""
    cases: list[dict[str, str]] = []
    fields: dict[str, str] = {}
    active_key: str | None = None
    for line in output.splitlines():
        match = re.match(r"INSTRUMENTATION_STATUS: ([^=]+)=(.*)", line)
        if match:
            active_key, value = match.groups()
            fields[active_key] = value
        elif line.startswith("INSTRUMENTATION_STATUS_CODE:"):
            code = int(line.split(":", 1)[1].strip())
            if code <= 0 and fields.get("test"):
                cases.append({
                    "class": fields.get("class", "unknown"),
                    "name": fields["test"],
                    "status": "passed" if code == 0 else "skipped" if code in {-3, -4} else "failed",
                    "detail": fields.get("stack", fields.get("stream", "")),
                })
            fields = {}
            active_key = None
        elif active_key and not line.startswith("INSTRUMENTATION_"):
            fields[active_key] += "\n" + line
    terminal = re.search(r"^INSTRUMENTATION_CODE:\s*-1\s*$", output, re.MULTILINE)
    success = bool(cases and terminal and not any(case["status"] == "failed" for case in cases)
                   and not re.search(r"INSTRUMENTATION_(?:FAILED|ABORTED)|Process crashed|FAILURES!!!", output))
    return cases, success


def write_junit(path: Path, cases: list[dict[str, str]], success: bool, output: str, duration: float):
    suite = ET.Element("testsuite", name="Taypeer Android", tests=str(len(cases) or 1),
                       failures=str(sum(case["status"] == "failed" for case in cases) + (not success and not cases)),
                       skipped=str(sum(case["status"] == "skipped" for case in cases)),
                       time=f"{duration:.3f}")
    for case in cases:
        node = ET.SubElement(suite, "testcase", classname=case["class"], name=case["name"])
        if case["status"] == "failed":
            ET.SubElement(node, "failure", message="Instrumentation test failed").text = case["detail"]
        elif case["status"] == "skipped":
            ET.SubElement(node, "skipped").text = case["detail"]
    if not success and not any(case["status"] == "failed" for case in cases):
        node = ET.SubElement(suite, "testcase", name="instrumentation-completion")
        ET.SubElement(node, "failure", message="No successful instrumentation completion").text = output[-8000:]
        suite.set("tests", str(len(cases) + 1))
        suite.set("failures", "1")
    ET.SubElement(suite, "system-out").text = output
    ET.indent(suite)
    ET.ElementTree(suite).write(path, encoding="utf-8", xml_declaration=True)


def override_value(output: str) -> str | None:
    match = re.search(r"^Override (?:size|density):\s*(\S+)", output, re.MULTILINE)
    return match.group(1) if match else None


@contextmanager
def screen_size(env: dict[str, str], serial: str, dp: str):
    width, height = map(int, dp.split("x"))
    old = {key: override_value(adb(env, serial, "shell", "wm", key).stdout) for key in ("size", "density")}
    try:
        # mdpi makes the exported Compose pixels and mockup dp directly comparable.
        adb(env, serial, "shell", "wm", "density", "160")
        adb(env, serial, "shell", "wm", "size", f"{width}x{height}")
        yield
    finally:
        for key in ("size", "density"):
            adb(env, serial, "shell", "wm", key, old[key] or "reset")


def extract_screenshots(archive: bytes, destination: Path) -> list[str]:
    """Extract only PNG files from the test-owned directory, never app state."""
    names = []
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        for member in tar.getmembers():
            path = Path(member.name)
            if path.is_absolute() or ".." in path.parts:
                raise QaError(f"Unsafe screenshot archive member: {member.name}")
            if member.isdir() and member.name.rstrip("/") == "qa-screenshots":
                continue
            if not member.isfile():
                raise QaError(f"Unexpected screenshot archive member: {member.name}")
            if path.suffix != ".png" or path.parts[0] != "qa-screenshots":
                raise QaError(f"Unexpected screenshot artifact: {member.name}")
            source = tar.extractfile(member)
            if source is None:
                raise QaError(f"Missing screenshot bytes: {member.name}")
            if len(path.parts) != 2 or path.name in names:
                raise QaError(f"Ambiguous screenshot artifact: {member.name}")
            target = destination / path.name
            target.write_bytes(source.read())
            names.append(path.name)
    if not names:
        raise QaError("Visual test produced no screenshots")
    return sorted(names)


def visual_arguments(locale: str, theme: str, font: str, size: str) -> dict[str, str]:
    width, height = map(int, size.split("x"))
    # FIG reserves status 32 dp + navigation 24 dp; captures contain product content only.
    return {"qaLocale": locale, "qaTheme": theme, "qaFontScale": font,
            "qaWidthDp": str(width), "qaHeightDp": str(height - 56)}


def validate_screenshots(folder: Path, names: list[str], arguments: dict[str, str]):
    from PIL import Image
    expected = (int(arguments["qaWidthDp"]), int(arguments["qaHeightDp"]))
    for name in names:
        with Image.open(folder / name) as image:
            if image.size != expected:
                raise QaError(f"Wrong screenshot size for {name}: {image.size}; expected {expected}. "
                              "Visual test must honor qaWidthDp/qaHeightDp.")


def compare_baseline(folder: Path, baseline: Path, names: list[str], capture: dict,
                     *, tolerance: int, max_difference: float) -> bool:
    from PIL import Image, ImageChops
    reference_capture = baseline / "capture.json"
    if not reference_capture.is_file():
        raise QaError(f"Baseline profile missing: {reference_capture}")
    reference = json.loads(reference_capture.read_text())
    for key in ("locale", "theme", "font_scale", "screen_dp", "screenshots"):
        if reference.get(key) != capture.get(key):
            raise QaError(f"Baseline capture profile mismatch ({key}): {baseline}")
    report = {"baseline": str(baseline.resolve()), "pixel_tolerance": tolerance,
              "max_difference_fraction": max_difference, "images": [], "success": True}
    for name in names:
        reference_path = baseline / name
        item = {"name": name, "success": False}
        if not reference_path.is_file():
            item["error"] = "Missing reviewed baseline PNG"
        else:
            with Image.open(folder / name) as actual_image, Image.open(reference_path) as expected_image:
                actual, expected = actual_image.convert("RGBA"), expected_image.convert("RGBA")
                if actual.size != expected.size:
                    item["error"] = f"Dimensions changed: {expected.size} -> {actual.size}"
                else:
                    diff = ImageChops.difference(actual, expected)
                    channel_max = diff.getchannel("R")
                    for channel in ("G", "B", "A"):
                        channel_max = ImageChops.lighter(channel_max, diff.getchannel(channel))
                    histogram = channel_max.histogram()
                    changed = sum(histogram[tolerance + 1:])
                    ratio = changed / (actual.width * actual.height)
                    item.update(changed_pixels=changed, difference_fraction=ratio, success=ratio <= max_difference)
                    if not item["success"]:
                        # RGB diff preserves opaque output even when alpha did not change.
                        diff.convert("RGB").save(folder / f"{Path(name).stem}-diff.png")
        report["images"].append(item)
        report["success"] = report["success"] and item["success"]
    (folder / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    return report["success"]


@contextmanager
def camera_denied(env: dict[str, str], serial: str):
    text = adb(env, serial, "shell", "dumpsys", "package", PACKAGE).stdout
    was_granted = bool(re.search(r"android\.permission\.CAMERA:\s*granted=true", text))
    try:
        # Revocation kills the app UID. It must precede, never interrupt, instrumentation.
        adb(env, serial, "shell", "pm", "revoke", PACKAGE, CAMERA_PERMISSION)
        reset = adb(env, serial, "shell", "pm", "clear-permission-flags", PACKAGE,
                    CAMERA_PERMISSION, "user-set", "user-fixed", check=False)
        arguments = {}
        if reset.returncode:
            detail = (reset.stdout + reset.stderr).strip()
            if detail != "Unknown command: clear-permission-flags":
                raise QaError(f"Camera permission flags reset failed: {detail[-2000:]}")
            # Android 12 lacks this shell command. Instrumentation may use the
            # platform TestApi for this package/permission; no app data is reset.
            arguments["qaResetCameraPermissionFlags"] = "true"
        yield arguments
    finally:
        adb(env, serial, "shell", "pm", "grant" if was_granted else "revoke", PACKAGE, CAMERA_PERMISSION)


def relay_parameters(line: str) -> tuple[int, dict[str, str]]:
    try:
        config = json.loads(line)
        url = urlsplit(config["url"])
        port = url.port
        der = bytes(config["root_der"])
    except (ValueError, TypeError, KeyError) as error:
        raise QaError("Invalid public TLS relay readiness JSON") from error
    if url.scheme != "https" or url.hostname not in {"127.0.0.1", "localhost"} or not port:
        raise QaError("Public relay fixture must provide a loopback HTTPS endpoint")
    if not der:
        raise QaError("Public relay fixture did not provide its TLS root certificate")
    return port, {"relayUrl": config["url"], "relayCertificate": base64.b64encode(der).decode("ascii")}


def read_relay_ready(process: subprocess.Popen, timeout: int = 90) -> str:
    deadline = time.monotonic() + timeout
    data = bytearray()
    with selectors.DefaultSelector() as selector:
        selector.register(process.stdout, selectors.EVENT_READ)
        while time.monotonic() < deadline:
            if not selector.select(timeout=min(1, max(0, deadline - time.monotonic()))):
                if process.poll() is not None:
                    raise QaError("Public TLS relay fixture exited before readiness; see public-relay.log")
                continue
            part = os.read(process.stdout.fileno(), 4096)
            if not part:
                raise QaError("Public TLS relay fixture closed stdout before readiness")
            data.extend(part)
            if len(data) > 65536:
                raise QaError("Public TLS relay fixture readiness output exceeded 64 KiB")
            if b"\n" in data:
                return bytes(data).split(b"\n", 1)[0].decode("utf-8")
    raise QaError(f"Public TLS relay fixture did not become ready within {timeout}s")


def stop_relay(process: subprocess.Popen):
    try:
        if process.poll() is None and process.stdin:
            process.stdin.write(b"\n")
            process.stdin.flush()
        process.communicate(timeout=15)
    except (subprocess.TimeoutExpired, BrokenPipeError):
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.communicate(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.communicate()


def relay_command(fixture: Path | None, output: Path) -> list[str]:
    if fixture is None:
        return ["cargo", "run", "--locked", "-p", "taypeer-android", "--example",
                "public-relay-fixture", "--features", "android-test-support"]
    fixture = fixture.resolve(strict=True)
    manifest = fixture.parent / "manifest.json"
    if not fixture.is_file() or not os.access(fixture, os.X_OK) or manifest.stat().st_size > 4096:
        raise QaError("Frozen relay needs an executable and a bounded capture manifest")
    captured = json.loads(manifest.read_text())
    digest = hashlib.sha256(fixture.read_bytes()).hexdigest()
    expected = captured.get("sha256")
    if captured.get("artifact") != "public-relay-fixture" or not isinstance(expected, str) or digest != expected:
        raise QaError("Frozen relay does not match its captured SHA-256")
    environment = json.loads((output / "environment.json").read_text())
    for label, key in (("app", "app_apk_sha256"), ("tests", "test_apk_sha256")):
        if environment["apk_sha256"][label] != captured.get(key):
            raise QaError("Frozen relay capture belongs to different APKs")
    (output / "relay-fixture.json").write_text(json.dumps({"path": str(fixture), "sha256": digest}, indent=2) + "\n")
    return [str(fixture)]


@contextmanager
def local_relay(env: dict[str, str], serial: str, output: Path, fixture: Path | None = None):
    """Serve an ephemeral strict-TLS relay; reverse only our own mapping."""
    with (output / "public-relay.log").open("wb") as log:
        process = subprocess.Popen(relay_command(fixture, output),
                                   cwd=REPOSITORY, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=log, start_new_session=True)
        port = None
        mapped = False
        try:
            port, arguments = relay_parameters(read_relay_ready(process))
            # Refuse to overwrite a reverse mapping owned by another invocation.
            adb(env, serial, "reverse", "--no-rebind", f"tcp:{port}", f"tcp:{port}")
            mapped = True
            yield arguments
        finally:
            try:
                if mapped:
                    adb(env, serial, "reverse", "--remove", f"tcp:{port}")
            finally:
                stop_relay(process)


@contextmanager
def network_arguments(env: dict[str, str], serial: str, classes: list[str], output: Path, fixture: Path | None = None):
    requires_relay = any(name == f"{PACKAGE}.P2PTest" or name.startswith(f"{PACKAGE}.P2PTest#offlineConcurrent")
                         or name.partition("#")[0] == f"{PACKAGE}.ReceivedSourcesTest"
                         for name in classes)
    if requires_relay:
        with local_relay(env, serial, output, fixture) as arguments:
            yield arguments
    else:
        yield {}


def build(env: dict[str, str], output: Path, timeout: int, *, coverage: bool = False):
    print("Building Rust/bindings, JVM tests, lint, APKs and checking ARM64 16-KiB packaging", flush=True)
    run(["sh", "check.sh"] + (["-PqaCoverage=true"] if coverage else []),
        env=env, timeout=timeout, log=output / "host.log")
    reports = PROJECT / "app/build/reports"
    if reports.exists():
        shutil.copytree(reports, output / "host-reports")


def install(env: dict[str, str], serial: str) -> dict[str, str]:
    artifacts = {"app": PROJECT / "app/build/outputs/apk/debug/app-debug.apk",
                 "tests": PROJECT / "app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"}
    hashes = {}
    for name, path in artifacts.items():
        if not path.is_file():
            raise QaError(f"Missing {path}; run without --skip-build first")
        hashes[name] = hashlib.sha256(path.read_bytes()).hexdigest()
        adb(env, serial, "install", "-r", "-t", str(path), timeout=180)
    return hashes


def instrument(env: dict[str, str], serial: str, classes: list[str], arguments: dict[str, str],
               output: Path, timeout: int, *, coverage: bool = False,
               package: str = PACKAGE, runner: str = RUNNER) -> bool:
    if (not re.fullmatch(r"dev\.taypeer(?:\.[a-z][a-z0-9]*)*", package)
            or not runner.startswith(package + ".test/")
            or not re.fullmatch(r"[A-Za-z0-9_./]+", runner)):
        raise QaError("Instrumentation needs an explicitly owned Taypeer package and runner")
    if coverage and package != PACKAGE:
        raise QaError("Product coverage cannot be collected from a private spike package")
    if coverage:
        adb(env, serial, "shell", "run-as", PACKAGE, "rm", "-f", "files/qa-coverage.ec")
        arguments = arguments | {"coverage": "true", "coverageFile": f"/data/data/{PACKAGE}/files/qa-coverage.ec"}
    command = ["shell", "am", "instrument", "-w", "-r", "-e", "class", ",".join(classes)]
    for key, value in arguments.items():
        command.extend(["-e", key, value])
    command.append(runner)
    start = time.monotonic()
    progress = InstrumentationProgress(classes, output, serial)
    try:
        result = adb(env, serial, *command, timeout=timeout, instrumentation_progress=progress)
        text = result.stdout + result.stderr
        completed = result.returncode == 0
    except subprocess.TimeoutExpired as error:
        text = (error.output or "") + (error.stderr or "") + f"\nInstrumentation timed out after {timeout}s\n"
        completed = False
        # The adb client exiting does not stop instrumentation inside a shared AVD.
        for owned_package in (package + ".test", package):
            adb(env, serial, "shell", "am", "force-stop", owned_package, check=False)
    (output / "instrumentation.log").write_text(text)
    cases, success = parse_instrumentation(text)
    success = success and completed
    progress.finish(success)
    write_junit(output / "junit.xml", cases, success, text, time.monotonic() - start)
    print(f"{serial}: {len(cases)} tests, {'PASS' if success else 'FAIL'} ({output})", flush=True)
    if coverage and completed:
        data = adb(env, serial, "exec-out", "run-as", PACKAGE, "cat", "files/qa-coverage.ec", binary=True).stdout
        if not data:
            raise QaError("Instrumentation produced no coverage bytes; rebuild with --coverage")
        (output / "instrumentation.ec").write_bytes(data)
    return success


def frozen_coverage_sources(directory: Path) -> Path:
    directory = directory.resolve(strict=True)
    manifest = directory.parent / "manifest.json"
    if manifest.stat().st_size > 65536:
        raise QaError("Coverage source capture exceeds its manifest bound")
    captured = json.loads(manifest.read_text())
    apk = PROJECT / "app/build/outputs/apk/debug/app-debug.apk"
    if captured.get("version") != 1 or captured.get("app_apk_sha256") != hashlib.sha256(apk.read_bytes()).hexdigest():
        raise QaError("Coverage source capture belongs to a different product APK")
    files = captured.get("files")
    if not isinstance(files, dict) or not files or len(files) > 512:
        raise QaError("Coverage source capture has no bounded file inventory")
    for relative, digest in files.items():
        path = Path(relative)
        if path.is_absolute() or ".." in path.parts:
            raise QaError("Coverage source capture has an invalid file path")
        if hashlib.sha256((directory / path).read_bytes()).hexdigest() != digest:
            raise QaError("Coverage source capture was changed after the APK build")
    return directory


def coverage_report(env: dict[str, str], output: Path, timeout: int, sources: Path | None = None):
    command = ["./gradlew", "--no-daemon", ":app:qaCoverageReport", "-PqaCoverage=true", f"-PqaCoverageData={output}"]
    if sources is not None:
        command.append(f"-PqaCoverageSources={frozen_coverage_sources(sources)}")
    run(command,
        env=env, timeout=timeout, log=output / "coverage-report.log")
    shutil.copytree(PROJECT / "app/build/reports/qa-coverage", output / "coverage")


@contextmanager
def emulator(env: dict[str, str], avd: str, output: Path, boot_timeout: int):
    tool = Path(env["ANDROID_HOME"]) / "emulator/emulator"
    existing = devices(env)
    port = next((port for port in range(5560, 5680, 2) if f"emulator-{port}" not in existing
                 and available_port(port) and available_port(port + 1)), None)
    if port is None:
        raise QaError("No free emulator console/adb port pair")
    serial = f"emulator-{port}"
    with (output / f"emulator-{avd}.log").open("w") as log:
        process = subprocess.Popen([str(tool), "-avd", avd, "-port", str(port), "-no-window", "-no-audio",
                                    "-no-snapshot", "-no-boot-anim", "-gpu", "swiftshader_indirect", "-cores", "2",
                                    "-memory", "2048"], cwd=PROJECT, env=env, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + boot_timeout
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise QaError(f"AVD {avd} exited during boot; see {log.name}")
                if devices(env).get(serial) == "device":
                    booted = adb(env, serial, "shell", "getprop", "sys.boot_completed", check=False).stdout.strip()
                    if booted == "1":
                        yield serial
                        return
                time.sleep(1)
            raise QaError(f"AVD {avd} did not boot within {boot_timeout}s")
        finally:
            # Stop only the emulator process created by this invocation.
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=15)


def available_port(port: int) -> bool:
    with socket.socket() as sock:
        try:
            sock.bind(("127.0.0.1", port))
            return True
        except OSError:
            return False


def enrollment_helper():
    path = REPOSITORY / "spikes/android-biometric-pipe/enroll.py"
    spec = importlib.util.spec_from_file_location("taypeer_public_enrollment", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def biometric_result_complete(path: Path, classes: list[str]) -> bool:
    root = ET.parse(path).getroot()
    cases = root.findall("testcase")
    expected = biometric_cases(classes)
    actual = [(case.get("classname"), case.get("name")) for case in cases]
    return (len(actual) == len(expected) and set(actual) == expected
            and not any(case.find("failure") is not None or case.find("skipped") is not None for case in cases))


def biometric_failure(path: Path, message: str):
    if not path.exists():
        write_junit(path, [], False, message, 0)
        return
    tree = ET.parse(path)
    root = tree.getroot()
    case = ET.SubElement(root, "testcase", classname=f"{PACKAGE}.BiometryStage", name="biometry-stage-completion")
    ET.SubElement(case, "failure", message="Biometry setup, driver, test completion or restoration failed").text = message
    root.set("tests", str(int(root.get("tests", "0")) + 1))
    root.set("failures", str(int(root.get("failures", "0")) + 1))
    ET.indent(tree)
    tree.write(path, encoding="utf-8", xml_declaration=True)


def biometry_stage(env: dict[str, str], serial: str, classes: list[str], args, folder: Path) -> bool:
    stage = folder / "biometry"
    stage.mkdir()
    helper = enrollment_helper()
    tools = SimpleNamespace(QaError=QaError, devices=devices, metadata=metadata, adb=adb)
    provided = getattr(args, "biometry_state", None)
    state_path = provided.resolve() if provided else stage / "PUBLIC-enrollment-state.json"
    stopped = threading.Event()
    failures = []
    touches = []
    driver = None
    passed = False
    restored = False

    def authorize():
        while not stopped.wait(0.5):
            try:
                marker = adb(env, serial, "shell", "run-as", PACKAGE, "head", "-c", "32",
                             "files/PUBLIC-biometric-prompt-ready", check=False, timeout=5)
                if marker.returncode:
                    if "No such file or directory" not in marker.stderr + marker.stdout:
                        raise QaError("PUBLIC biometric marker could not be read")
                elif marker.stdout == "touch-1\n":
                    if not stopped.is_set():
                        adb(env, serial, "emu", "finger", "touch", "1", timeout=5)
                        touches.append(time.monotonic())
                else:
                    raise QaError("Unexpected PUBLIC biometric marker")
            except (QaError, subprocess.TimeoutExpired, OSError) as error:
                failures.append("PUBLIC biometric driver failed: " + type(error).__name__)
                stopped.set()

    try:
        helper.prepare(tools, env, serial, state_path, adopt=provided is not None)
        adb(env, serial, "shell", "run-as", PACKAGE, "rm", "-f", "files/PUBLIC-biometric-prompt-ready")
        driver = threading.Thread(target=authorize, name="PUBLIC-biometric-finger", daemon=True)
        driver.start()
        passed = instrument(env, serial, classes, {}, stage, args.timeout, coverage=args.coverage)
        if not biometric_result_complete(stage / "junit.xml", classes):
            passed = False
            failures.append("Exact biometric tests must all complete without skips")
        if any("recreationRebindsSameRequestAndRealStrongCryptoObjectTransfersOnlyOnce" in method
               for _, method in biometric_cases(classes)) and not touches:
            passed = False
            failures.append("Positive OS authentication had no PUBLIC driver event")
    except (QaError, subprocess.TimeoutExpired, OSError, ValueError, ET.ParseError) as error:
        failures.append("PUBLIC biometry stage failed: " + str(error))
    finally:
        stopped.set()
        if driver:
            # qa.run may spend ten seconds terminating a timed-out adb client.
            driver.join(timeout=20)
            if driver.is_alive():
                failures.append("PUBLIC biometric driver did not terminate")
        try:
            if not passed or failures:
                # Cleanup cannot turn an OS dismissal failure into a successful test.
                for package in (PACKAGE + ".test", PACKAGE):
                    adb(env, serial, "shell", "am", "force-stop", package, check=False)
            adb(env, serial, "shell", "run-as", PACKAGE, "rm", "-f", "files/PUBLIC-biometric-prompt-ready")
        except (QaError, subprocess.TimeoutExpired, OSError):
            failures.append("PUBLIC biometric application cleanup failed")
        try:
            if state_path.exists():
                restored = helper.restore(tools, env, serial, state_path).get("restored") is True
            else:
                failures.append("No PUBLIC restoration receipt was produced")
        except (QaError, subprocess.TimeoutExpired, OSError, ValueError):
            failures.append("PUBLIC lock restoration failed; retain receipt for explicit restore")
    passed = passed and restored and not failures
    if not passed:
        biometric_failure(stage / "junit.xml", "\n".join(failures) or "Biometric instrumentation failed")
    (stage / "stage.json").write_text(json.dumps({"success": passed, "restored": restored,
        "classes": classes, "receipt": str(state_path), "public_finger_events": len(touches),
        "failures": failures}, indent=2) + "\n")
    return passed


def target(env: dict[str, str], serial: str, args, output: Path, *, folder_name: str | None = None) -> bool:
    if not serial.startswith("emulator-"):
        raise QaError("QA uses emulator profiles; a physical-device serial is not accepted")
    if devices(env).get(serial) != "device":
        raise QaError(f"{serial} is absent, offline or unauthorized")
    info = metadata(env, serial)
    if info["abi"] != "arm64-v8a" or int(info["api"]) < 31:
        raise QaError(f"Expected ARM64 Android 12+: {info}")
    classes, biometric = [], []
    if args.command == "test":
        classes, biometric = split_biometry(args.test_class or
            ([] if args.suite == "camera-denied" else test_classes(args.suite)))
        if biometric and not getattr(args, "authorize_public_biometry", False):
            raise QaError("Biometry/full requires --authorize-public-biometry on an explicitly owned PUBLIC emulator")
    folder = output / (folder_name or serial)
    folder.mkdir()
    info["apk_sha256"] = install(env, serial)
    (folder / "environment.json").write_text(json.dumps(info, indent=2) + "\n")
    if args.command == "test":
        denied_classes = [name for name in classes if name == CAMERA_DENIED_CLASS or name.startswith(CAMERA_DENIED_CLASS + "#")]
        classes = [name for name in classes if name not in denied_classes]
        denied = (not args.test_class and args.suite in {"full", "ui", "camera-denied"}) or bool(denied_classes)
        passed = True
        if denied:
            with camera_denied(env, serial) as permission_arguments:
                denied_folder = folder / "camera-denied"
                denied_folder.mkdir()
                passed = instrument(env, serial, denied_classes or [CAMERA_DENIED_CLASS], permission_arguments, denied_folder, args.timeout, coverage=args.coverage)
                if classes:
                    with network_arguments(env, serial, classes, folder, args.relay_fixture) as arguments:
                        passed = instrument(env, serial, classes, arguments, folder, args.timeout, coverage=args.coverage) and passed
        elif classes:
            with network_arguments(env, serial, classes, folder, args.relay_fixture) as arguments:
                passed = instrument(env, serial, classes, arguments, folder, args.timeout, coverage=args.coverage)
        if not classes and not biometric and not denied:
            raise QaError(f"No test classes in {args.suite} suite")
        if biometric:
            passed = biometry_stage(env, serial, biometric, args, folder) and passed
        return passed
    if args.baseline:
        reference_environment = json.loads((args.baseline / "environment.json").read_text())
        for key in ("api", "abi", "page_size", "fingerprint"):
            if reference_environment.get(key) != info.get(key):
                raise QaError(f"Reviewed baseline device profile mismatch ({key}); use a baseline from this system image")
    success = True
    for name, locale, theme, font, size in VISUAL_CASES:
        if args.case and name not in args.case:
            continue
        case_folder = folder / name
        case_folder.mkdir()
        with screen_size(env, serial, size):
            arguments = visual_arguments(locale, theme, font, size)
            # Delete only stale PNG evidence, not the emulator or application profile.
            adb(env, serial, "shell", "run-as", PACKAGE, "rm", "-rf", "files/qa-screenshots")
            passed = instrument(env, serial, [VISUAL_CLASS],
                                arguments,
                                case_folder, args.timeout)
            success = passed and success
            if passed:
                archive = adb(env, serial, "exec-out", "run-as", PACKAGE, "tar", "-C", "files", "-cf", "-",
                              "qa-screenshots", binary=True).stdout
                screenshots = extract_screenshots(archive, case_folder)
                validate_screenshots(case_folder, screenshots, arguments)
                capture = {"locale": locale, "theme": theme, "font_scale": float(font), "screen_dp": size,
                           "content_px": [int(arguments["qaWidthDp"]), int(arguments["qaHeightDp"])], "screenshots": screenshots}
                (case_folder / "capture.json").write_text(json.dumps(capture, indent=2) + "\n")
                if args.baseline:
                    success = compare_baseline(case_folder, args.baseline / name, screenshots, capture,
                                               tolerance=args.pixel_tolerance, max_difference=args.max_difference) and success
    return success


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("doctor", help="Read-only SDK/JDK/emulator inventory")
    for name in ("host", "test", "visual"):
        command = commands.add_parser(name)
        command.add_argument("--output", type=Path, help="New evidence directory (must not exist)")
        command.add_argument("--timeout", type=int, default=1800, help="Seconds per build/test invocation")
        if name != "visual":
            command.add_argument("--coverage", action="store_true", help="Build instrumented debug APK and report product Kotlin/Java JaCoCo coverage")
            command.add_argument("--coverage-sources", type=Path, help="Frozen product source capture tied to the installed coverage APK")
        if name != "host":
            command.add_argument("--serial", action="append", default=[], help="Existing emulator, repeat for matrix")
            command.add_argument("--avd", action="append", default=[], help="AVD to boot and stop, repeat for matrix")
            command.add_argument("--boot-timeout", type=int, default=180)
            command.add_argument("--skip-build", action="store_true", help="Use already-built APKs; hashes retained")
        if name == "test":
            command.add_argument("--suite", choices=("full", "ui", "platform", "network", "camera-denied", "biometry"), default="full")
            command.add_argument("--test-class", action="append", help="Exact class or class#method; repeat allowed")
            command.add_argument("--authorize-public-biometry", action="store_true", help="Permit owned PUBLIC PIN/finger setup and verified restoration")
            command.add_argument("--biometry-state", type=Path, help="Adopt an existing same-boot owned PUBLIC restoration receipt (one target only)")
            command.add_argument("--relay-fixture", type=Path, help="Frozen private TLS fixture with matching manifest and APK hashes")
        if name == "visual":
            command.add_argument("--case", action="append", choices=[case[0] for case in VISUAL_CASES])
            command.add_argument("--baseline", type=Path, help="Reviewed device-profile directory with environment.json and per-case PNGs")
            command.add_argument("--pixel-tolerance", type=int, default=0, choices=range(256), metavar="0..255")
            command.add_argument("--max-difference", type=float, default=0.0, help="Allowed changed pixel fraction 0..1 (default exact)")
    args = parser.parse_args()
    env = environment()
    if args.command == "visual":
        require_pillow()
    if args.command == "visual" and not 0 <= args.max_difference <= 1:
        parser.error("--max-difference must be a fraction between 0 and 1")
    if args.command == "doctor":
        info = {"sdk": env["ANDROID_HOME"], "avd_home": env["ANDROID_AVD_HOME"],
                "java": run(["java", "-version"], env=env).stderr.strip(),
                "devices": [metadata(env, serial) if state == "device" else {"serial": serial, "state": state}
                            for serial, state in devices(env).items()]}
        print(json.dumps(info, indent=2))
        return 0
    if args.command != "host" and not args.serial and not args.avd:
        parser.error("Select at least one --serial or --avd; attached devices are never selected implicitly")
    if args.command == "test" and args.biometry_state:
        if not args.authorize_public_biometry or len(args.serial) + len(args.avd) != 1:
            parser.error("--biometry-state requires explicit PUBLIC authorization and exactly one target")
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    output = (args.output or REPOSITORY / "artifacts/android" / stamp).resolve()
    output.mkdir(parents=True, exist_ok=False)
    print(f"Evidence: {output}", flush=True)
    status = {"command": args.command, "started_utc": stamp, "success": False}
    try:
        if args.command == "host" or not args.skip_build:
            build(env, output, args.timeout, coverage=getattr(args, "coverage", False))
        success = True
        if args.command != "host":
            for serial in args.serial:
                success = target(env, serial, args, output) and success
            for avd in args.avd:
                with emulator(env, avd, output, args.boot_timeout) as serial:
                    success = target(env, serial, args, output, folder_name=avd) and success
        if getattr(args, "coverage", False):
            coverage_report(env, output, args.timeout, args.coverage_sources)
        status["success"] = success
        return 0 if success else 1
    except (QaError, subprocess.TimeoutExpired, OSError, ValueError) as error:
        status["error"] = str(error)
        print(f"QA failed: {error}", file=sys.stderr)
        return 1
    finally:
        results = []
        for path in sorted(output.rglob("junit.xml")):
            suite = ET.parse(path).getroot()
            results.append({"path": str(path.relative_to(output)),
                            **{key: int(suite.get(key, "0")) for key in ("tests", "failures", "skipped")}})
        status["results"] = results
        status["totals"] = {key: sum(result[key] for result in results) for key in ("tests", "failures", "skipped")}
        status["finished_utc"] = datetime.now(timezone.utc).isoformat()
        (output / "summary.json").write_text(json.dumps(status, indent=2) + "\n")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (QaError, subprocess.TimeoutExpired, OSError) as error:
        print(f"QA failed: {error}", file=sys.stderr)
        sys.exit(1)
