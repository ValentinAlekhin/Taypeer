#!/usr/bin/env python3
"""Separate PUBLIC APK: reuse product QA environment, parsing and owned-process cleanup."""
from pathlib import Path
import argparse
import base64
import hashlib
import importlib.util
import json
import re
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parents[2]
SPIKE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("taypeer_android_qa", ROOT / "apps/taypeer-android/qa.py")
qa = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qa)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--product-native-capture", required=True, type=Path,
                        help="Immutable Native library/bindings capture from a successful product build")
    parser.add_argument("--test-class", action="append", default=[],
                        help="Select an exact PUBLIC spike class or class#method for an independent regression")
    parser.add_argument("--authorize-public-finger", action="store_true",
                        help="Run actual OS prompt tests with an already enrolled PUBLIC emulator finger 1")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = qa.environment()
    native_capture = args.product_native_capture.resolve(strict=True)
    manifest = native_capture / "manifest.json"
    if manifest.stat().st_size > 65536:
        raise qa.QaError("PUBLIC Native capture exceeds its manifest bound")
    native = json.loads(manifest.read_text())
    inventory = native.get("files")
    if native.get("version") != 1 or not isinstance(inventory, dict) or not 1 <= len(inventory) <= 64:
        raise qa.QaError("PUBLIC Native capture has no bounded file inventory")
    for relative, digest in inventory.items():
        path = Path(relative)
        if path.is_absolute() or ".." in path.parts or not (native_capture / path).resolve().is_relative_to(native_capture):
            raise qa.QaError("PUBLIC Native capture contains an invalid path")
        if hashlib.sha256((native_capture / path).read_bytes()).hexdigest() != digest:
            raise qa.QaError("PUBLIC Native capture changed after the product build")
    binding = native_capture / "uniffi/dev/taypeer/bridge/taypeer_android.kt"
    if "verifyPublicBiometricPipe" not in binding.read_text() or not (native_capture / "jniLibs/arm64-v8a/libtaypeer_android.so").is_file():
        raise qa.QaError("PUBLIC Native capture lacks the actual private FD importer")
    env["TAYPEER_PUBLIC_NATIVE_CAPTURE"] = str(native_capture)
    if not args.serial.startswith("emulator-") or qa.devices(env).get(args.serial) != "device":
        raise qa.QaError("Select an existing owned emulator explicitly")
    info = qa.metadata(env, args.serial)
    if info["abi"] != "arm64-v8a" or info["api"] < 31:
        raise qa.QaError("The private pipe spike requires ARM64 Android 12+")
    product_sources = [qa.PROJECT / "app/src/main/cpp/biometric_key_pipe.cpp",
                       qa.PROJECT / "app/src/main/java/dev/taypeer/platform/BiometricKeyPipe.kt",
                       qa.PROJECT / "app/src/main/java/dev/taypeer/platform/BiometricOsKey.kt"]
    build_sources = product_sources + sorted((SPIKE / "app/src").rglob("*.kt"))
    build_sources += [SPIKE / "app/build.gradle.kts", SPIKE / "app/src/main/AndroidManifest.xml"]
    def input_hashes():
        return {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in build_sources}
    inputs = input_hashes()
    build_stamp = SPIKE / "app/build/PUBLIC-build-inputs.json"
    if args.skip_build:
        captured = json.loads(build_stamp.read_text())
        if captured.get("version") != 1 or captured.get("inputs") != inputs or captured.get("native") != native:
            raise qa.QaError("PUBLIC spike APK belongs to different sources or Native input; rebuild")
    if not args.skip_build:
        qa.run([str(qa.PROJECT / "gradlew"), "-p", str(SPIKE), "--no-daemon",
                ":app:assembleDebug", ":app:assembleDebugAndroidTest", "--write-locks"],
               env=env, timeout=600, log=output / "build.log")
        if input_hashes() != inputs:
            raise qa.QaError("PUBLIC spike sources changed while the APK was building")
    rust_binary = SPIKE / "rust-wrap/target/debug/taypeer-public-biometric-wrap"
    if args.authorize_public_finger:
        qa.run(["cargo", "build", "--locked", "--manifest-path", str(SPIKE / "rust-wrap/Cargo.toml")],
               env=env, timeout=300, log=output / "rust-wrap-build.log")
    info["apk_sha256"] = {}
    info["product_native_capture"] = native
    for label, relative in (("app", "debug/app-debug.apk"), ("tests", "androidTest/debug/app-debug-androidTest.apk")):
        path = SPIKE / "app/build/outputs/apk" / relative
        info["apk_sha256"][label] = hashlib.sha256(path.read_bytes()).hexdigest()
    if args.skip_build and captured.get("apk_sha256") != info["apk_sha256"]:
        raise qa.QaError("PUBLIC spike APK changed after its build capture")
    if not args.skip_build:
        build_stamp.write_text(json.dumps({"version": 1, "inputs": inputs, "native": native,
                                           "apk_sha256": info["apk_sha256"]}, indent=2) + "\n")
    for _, relative in (("app", "debug/app-debug.apk"), ("tests", "androidTest/debug/app-debug-androidTest.apk")):
        path = SPIKE / "app/build/outputs/apk" / relative
        qa.adb(env, args.serial, "install", "-r", "-t", str(path), timeout=120)
    qa.adb(env, args.serial, "shell", "am", "force-stop", "dev.taypeer.biometricspike")
    info["scope"] = ("PUBLIC actual Android Keystore/OS CryptoObject/product-JNI/isolated Rust FD importer; no document acceptance"
                     if args.authorize_public_finger else
                     "PUBLIC JCA/product-JNI/isolated Rust FD ownership; no biometric authorization or document acceptance")
    info["source_sha256"] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                             for path in (qa.PROJECT / "app/src/main/cpp/biometric_key_pipe.cpp",
                                          qa.PROJECT / "app/src/main/java/dev/taypeer/platform/BiometricKeyPipe.kt",
                                          qa.PROJECT / "app/src/main/java/dev/taypeer/platform/BiometricOsKey.kt")}
    evidence_sources = list((SPIKE / "app/src").rglob("*.kt")) + list((SPIKE / "rust-wrap/src").rglob("*.rs"))
    evidence_sources += [SPIKE / "rust-wrap/Cargo.toml", SPIKE / "rust-wrap/Cargo.lock", SPIKE / "app/build.gradle.kts",
                         SPIKE / "app/gradle.lockfile", SPIKE / "app/src/main/AndroidManifest.xml", Path(__file__).resolve()]
    info["source_sha256"].update({str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                                  for path in sorted(evidence_sources)})
    (output / "environment.json").write_text(json.dumps(info, indent=2) + "\n")
    started = time.monotonic()
    stopped = threading.Event()
    driver_failures = []
    touches = []
    wraps = []

    def authorize():
        while not stopped.wait(0.5):
            try:
                marker = qa.adb(env, args.serial, "shell", "run-as", "dev.taypeer.biometricspike",
                                "cat", "files/PUBLIC-biometric-prompt-ready", check=False, timeout=5)
                if marker.returncode == 0 and marker.stdout in ("touch-1\n", "touch-2\n"):
                    finger = "1" if marker.stdout == "touch-1\n" else "2"
                    qa.adb(env, args.serial, "emu", "finger", "touch", finger, timeout=5)
                    touches.append(time.monotonic() - started)
                elif marker.returncode == 0 and marker.stdout == "wrap-rust\n":
                    encoded = qa.adb(env, args.serial, "shell", "run-as", "dev.taypeer.biometricspike",
                                     "cat", "files/PUBLIC-rust-spki.b64", timeout=5).stdout
                    if len(encoded) > 5500:
                        raise qa.QaError("PUBLIC SPKI fixture exceeded its bound")
                    spki = base64.b64decode(encoded, validate=True)
                    digest = hashlib.sha256(spki).hexdigest()
                    if digest not in wraps:
                        source = output / f"PUBLIC-rust-spki-{len(wraps)}.der"
                        wrapped = output / f"PUBLIC-rust-ciphertext-{len(wraps)}.bin"
                        source.write_bytes(spki)
                        qa.run([str(rust_binary), str(source), str(wrapped)], env=env, timeout=10)
                        if wrapped.stat().st_size != 256:
                            raise qa.QaError("PUBLIC Rust fixture did not return exact RSA ciphertext")
                        temporary = "/data/local/tmp/PUBLIC-biometricspike-ciphertext.bin"
                        qa.adb(env, args.serial, "push", str(wrapped), temporary, timeout=10)
                        qa.adb(env, args.serial, "shell", "run-as", "dev.taypeer.biometricspike",
                               "cp", temporary, "files/PUBLIC-rust-wrapped.pending", timeout=5)
                        qa.adb(env, args.serial, "shell", "run-as", "dev.taypeer.biometricspike",
                               "mv", "files/PUBLIC-rust-wrapped.pending", "files/PUBLIC-rust-wrapped.bin", timeout=5)
                        qa.adb(env, args.serial, "shell", "rm", "-f", temporary, timeout=5)
                        wraps.append(digest)
            except (qa.QaError, subprocess.TimeoutExpired, ValueError) as failure:
                driver_failures.append(str(failure))
                stopped.set()

    classes = ["dev.taypeer.biometricspike.PipeOwnershipTest", "dev.taypeer.biometricspike.ProviderCancellationTest", "dev.taypeer.biometricspike.IsolatedPipeTest"]
    driver = None
    if args.authorize_public_finger:
        # Only the separate PUBLIC test APK can request a console finger event.
        qa.adb(env, args.serial, "shell", "run-as", "dev.taypeer.biometricspike",
               "rm", "-f", "files/PUBLIC-biometric-prompt-ready")
        classes.append("dev.taypeer.biometricspike.OsAuthorizationTest")
    if args.test_class:
        allowed = set(classes)
        for selected in args.test_class:
            name, _, method = selected.partition("#")
            if name not in allowed or (method and not re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*", method)):
                raise qa.QaError("Select an exact owned PUBLIC spike class or method")
        classes = args.test_class
    if args.authorize_public_finger:
        driver = threading.Thread(target=authorize, name="PUBLIC-emulator-finger", daemon=True)
        driver.start()
    try:
        success = qa.instrument(env, args.serial, classes, {}, output, 240,
                                package="dev.taypeer.biometricspike",
                                runner="dev.taypeer.biometricspike.test/androidx.test.runner.AndroidJUnitRunner")
    finally:
        stopped.set()
        if driver:
            driver.join(timeout=6)
            if driver.is_alive():
                driver_failures.append("PUBLIC finger driver did not terminate")
    success = success and not driver_failures
    (output / "summary.json").write_text(json.dumps({"success": success, "scope": info["scope"],
                                                   "elapsed_seconds": time.monotonic() - started,
                                                   "public_finger_events": len(touches),
                                                   "public_rust_wraps": len(wraps),
                                                   "driver_failures": driver_failures}, indent=2) + "\n")
    return 0 if success else 1


if __name__ == "__main__":
    raise SystemExit(main())
