#!/usr/bin/env python3
"""Set up / restore a PUBLIC fingerprint on an explicitly selected, unsecured owned emulator."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import tempfile
import time
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
PIN = "111111"  # Intentionally PUBLIC and limited to the separate test emulator.
XML = "/sdcard/PUBLIC-taypeer-enroll.xml"


def session(qa, env, serial):
    if not serial.startswith("emulator-") or qa.devices(env).get(serial) != "device":
        raise qa.QaError("PUBLIC enrollment requires an explicitly owned existing emulator")
    metadata = qa.metadata(env, serial)
    if metadata["abi"] != "arm64-v8a" or metadata["api"] < 31:
        raise qa.QaError("PUBLIC enrollment requires the supported ARM64 emulator image")
    if qa.adb(env, serial, "shell", "getprop", "ro.kernel.qemu").stdout.strip() != "1":
        raise qa.QaError("PUBLIC enrollment requires a real emulator transport")
    user = qa.adb(env, serial, "shell", "am", "get-current-user").stdout.strip()
    if user != "0":
        raise qa.QaError("PUBLIC enrollment supports only the owned emulator's primary user")
    boot = qa.adb(env, serial, "shell", "cat", "/proc/sys/kernel/random/boot_id").stdout.strip()
    if not re.fullmatch(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}", boot):
        raise qa.QaError("PUBLIC emulator boot identity could not be verified")
    return {"serial": serial, "boot": boot,
            "user": user, "api": metadata["api"], "fingerprint": metadata["fingerprint"]}


def write_state(path, state, *, create=False):
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = json.dumps(state, indent=2) + "\n"
    if create:
        with path.open("x") as output:
            output.write(encoded)
            output.flush()
            os.fsync(output.fileno())
    else:
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as output:
                temporary = Path(output.name)
                output.write(encoded)
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary, path)
        finally:
            if temporary and temporary.exists():
                temporary.unlink()
    descriptor = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def receipt(qa, path, current):
    if path.stat().st_size > 4096:
        raise qa.QaError("PUBLIC restoration receipt exceeds its bound")
    state = json.loads(path.read_text())
    if not isinstance(state, dict) or state.get("version", 1) not in (1, 2):
        raise qa.QaError("Unsupported PUBLIC restoration receipt")
    if (state.get("serial") != current["serial"] or state.get("boot") != current["boot"]
            or state.get("previous_disabled") != "true" or not isinstance(state.get("restored"), bool)):
        raise qa.QaError("This restoration receipt belongs to a different emulator session")
    if state.get("version", 1) == 2:
        if any(state.get(key) != current[key] for key in ("user", "api", "fingerprint")):
            raise qa.QaError("PUBLIC restoration receipt has a different user or system image")
    return state


def lock(qa, env, serial, command, *arguments, expected):
    result = qa.adb(env, serial, "shell", "locksettings", command, "--user", "0", *arguments, check=False)
    if result.returncode != 0 or result.stdout.strip() != expected:
        # Never report arbitrary credential / Settings command output.
        raise qa.QaError(f"PUBLIC locksettings {command} did not confirm its exact result")


def disabled(qa, env, serial, *, old=None):
    arguments = ("--old", old) if old is not None else ()
    response = qa.adb(env, serial, "shell", "locksettings", "get-disabled", "--user", "0", *arguments, check=False)
    result = response.stdout.strip()
    if response.returncode != 0 or result not in ("true", "false"):
        raise qa.QaError("PUBLIC lockscreen state could not be verified")
    return result


def empty_credential_verified(qa, env, serial):
    response = qa.adb(env, serial, "shell", "locksettings", "verify", "--user", "0", check=False)
    # An error is never interpreted as an absent lock. Only this exact positive
    # result permits the no-PIN write-ahead / partially completed restore path.
    return response.returncode == 0 and response.stdout.strip() == "Lock credential verified successfully"


def restore(qa, env, serial, path):
    current = session(qa, env, serial)
    state = receipt(qa, path, current)
    if not empty_credential_verified(qa, env, serial):
        if state["restored"]:
            raise qa.QaError("A restored receipt cannot authorize clearing a later lock")
        lock(qa, env, serial, "verify", "--old", PIN, expected="Lock credential verified successfully")
        # API31 requires the old credential even for this read-only query.
        if disabled(qa, env, serial, old=PIN) != "false":
            raise qa.QaError("PUBLIC secured lock state could not be verified")
        lock(qa, env, serial, "clear", "--old", PIN, expected="Lock credential cleared")
    lock(qa, env, serial, "verify", expected="Lock credential verified successfully")
    if not state["restored"]:
        lock(qa, env, serial, "set-disabled", "true", expected="Lock screen disabled set to true")
    if disabled(qa, env, serial) != "true":
        raise qa.QaError("PUBLIC restoration did not recover the original unlocked lockscreen")
    qa.adb(env, serial, "shell", "rm", "-f", XML)
    state.update(version=2, **current, restored=True, phase="restored")
    write_state(path, state)
    return state


def prepare(qa, env, serial, path, *, adopt=False):
    current = session(qa, env, serial)
    if adopt:
        state = receipt(qa, path, current)
        if state["restored"]:
            raise qa.QaError("Adopt only an active owned PUBLIC enrollment receipt")
        lock(qa, env, serial, "verify", "--old", PIN, expected="Lock credential verified successfully")
        if disabled(qa, env, serial, old=PIN) != "false":
            raise qa.QaError("Adopt only an active owned PUBLIC enrollment receipt")
        # Legacy receipts deliberately have no enrollment claim. Actual prompt
        # tests still require BiometricManager / Keystore to confirm enrollment.
        state.update(version=2, **current, phase="public-pin-verified")
        write_state(path, state)
        return state
    if path.exists():
        raise qa.QaError("Do not overwrite an existing emulator restoration receipt")
    if disabled(qa, env, serial) != "true":
        raise qa.QaError("PUBLIC setup refuses an emulator with an existing enabled lock")
    lock(qa, env, serial, "verify", expected="Lock credential verified successfully")
    state = {"version": 2, **current, "previous_disabled": "true", "restored": False, "phase": "prepared"}
    # Durable write-ahead receipt precedes the first mutation of the emulator.
    write_state(path, state, create=True)
    lock(qa, env, serial, "set-pin", PIN, expected="Pin set to '111111'")
    lock(qa, env, serial, "verify", "--old", PIN, expected="Lock credential verified successfully")
    state["phase"] = "pin-set"
    write_state(path, state)
    # A previous enrollment Activity can remain in the Settings task after its
    # credential is restored. Start a fresh owned Settings process and wait for
    # it; reusing that completed Activity can immediately return to the launcher.
    qa.adb(env, serial, "shell", "am", "start", "-S", "-W", "-a", "android.settings.FINGERPRINT_ENROLL")
    deadline = time.monotonic() + 60
    for _ in range(30):
        if time.monotonic() >= deadline:
            break
        qa.adb(env, serial, "shell", "uiautomator", "dump", XML, timeout=10)
        dump = qa.adb(env, serial, "exec-out", "cat", XML).stdout
        if len(dump) > 1_048_576:
            raise qa.QaError("PUBLIC enrollment Settings projection exceeded its bound")
        tree = ET.fromstring(dump)
        nodes = list(tree.iter("node"))
        if not any(node.get("package") == "com.android.settings" for node in nodes):
            # Window transitions can outlive am's launch acknowledgement. Never
            # send PIN/taps to an unrelated foreground surface.
            time.sleep(0.1)
            continue
        title = next((node.get("text", "") for node in nodes
                      if node.get("resource-id", "").endswith("suc_layout_title")), "")
        if any(node.get("password") == "true" for node in nodes):
            qa.adb(env, serial, "shell", "input", "text", PIN)
            qa.adb(env, serial, "shell", "input", "keyevent", "66")
            continue
        action = next((node for label in ("DONE", "I AGREE", "MORE", "NEXT")
                       for node in nodes if node.get("text") == label and node.get("clickable") == "true"), None)
        if action is not None:
            bounds = action.get("bounds", "").replace("][", ",").strip("[]").split(",")
            left, top, right, bottom = map(int, bounds)
            qa.adb(env, serial, "shell", "input", "tap", str((left + right) // 2), str((top + bottom) // 2))
            if action.get("text") == "DONE" and title == "Fingerprint added":
                qa.adb(env, serial, "shell", "input", "keyevent", "3")
                state["phase"] = "enrolled"
                write_state(path, state)
                return state
            continue
        if title in {"Touch the sensor", "Lift, then touch again", "Touch the fingerprint sensor"}:
            qa.adb(env, serial, "emu", "finger", "touch", "1")
            continue
        # Report only an unrecognised state, never arbitrary Settings contents or PIN nodes.
        raise qa.QaError("PUBLIC fingerprint Settings flow has an unsupported state; restore using the receipt")
    raise qa.QaError("PUBLIC fingerprint Settings flow did not finish; restore using the receipt")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--state", required=True, type=Path)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--restore", action="store_true")
    mode.add_argument("--adopt", action="store_true", help="Verify an existing same-boot owned PUBLIC receipt")
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("qa", ROOT / "apps/taypeer-android/qa.py")
    qa = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(qa)
    env = qa.environment()
    if args.restore:
        restore(qa, env, args.serial, args.state)
        print("Owned emulator PUBLIC lock state restored and verified")
    else:
        prepare(qa, env, args.serial, args.state, adopt=args.adopt)
        print("Owned emulator PUBLIC enrollment prepared; actual OS tests verify authorization")


if __name__ == "__main__":
    main()
