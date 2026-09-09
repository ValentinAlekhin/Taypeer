#!/usr/bin/env python3
"""Run synthetic Rust/JNI suites on one explicitly selected ARM64 emulator."""
import argparse
import json
import pathlib
import shlex
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
for option in ("build-json", "adb", "dex", "jni-library", "output"):
    parser.add_argument("--" + option, required=True)
parser.add_argument("--serial", default="emulator-5554")
args = parser.parse_args()
remote = "/data/local/tmp/taypeer-spike-suite"
adb = [args.adb, "-s", args.serial]
artifacts = {}
build_succeeded = False
for line in pathlib.Path(args.build_json).read_text().splitlines():
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        continue
    if event.get("reason") == "build-finished":
        build_succeeded = event["success"]
    if (event.get("reason") != "compiler-artifact" or not event.get("executable")
            or "taypeer-encrypted-sync-spike" not in event.get("package_id", "")):
        continue
    target = event["target"]
    name = target["name"]
    if event["profile"]["test"]:
        if "lib" in target["kind"] or "rlib" in target["kind"]:
            name = "lib-tests"
        elif target["kind"] == ["test"]:
            name += "-tests"
        else:
            continue
    artifacts[name] = event["executable"]
if not build_succeeded:
    raise RuntimeError("build JSON does not confirm a successful build")
suites = ["lib-tests", "storage-tests", "session-tests", "network-tests",
          "taypeer-encrypted-sync-spike", "memory-probe"]
for name in suites:
    if name not in artifacts:
        raise RuntimeError("missing artifact: " + name)
for path in [*artifacts.values(), args.dex, args.jni_library]:
    if not pathlib.Path(path).is_file():
        raise RuntimeError("missing build output: " + path)

subprocess.run(adb + ["shell", "mkdir -p " + shlex.quote(remote)], check=True)
for name, local in artifacts.items():
    subprocess.run(adb + ["push", local, remote + "/" + name], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
for local, name in [(args.dex, "classes.dex"),
                    (args.jni_library, "libtaypeer_encrypted_sync_spike.so")]:
    subprocess.run(adb + ["push", local, remote + "/" + name], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
# exec preserves the child PID used by crash/timeout cleanup in the Rust tests.
launcher = ("#!/system/bin/sh\nexport CLASSPATH=" + remote + "/classes.dex\n"
            'exec app_process /system/bin TaypeerNative "$@"\n')
subprocess.run(adb + ["shell", "cat > " + remote + "/sync-node"],
               input=launcher, text=True, check=True)
subprocess.run(adb + ["shell", "chmod 700 " + remote + "/*"], check=True)
results = {}
with open(args.output, "w") as log:
    for prop in ["ro.build.version.release", "ro.build.version.sdk",
                 "ro.product.cpu.abi", "ro.kernel.qemu"]:
        result = subprocess.run(adb + ["shell", "getprop", prop], text=True,
                                capture_output=True, check=True)
        log.write(prop + "=" + result.stdout.strip() + "\n")
    for name in suites:
        flags = " --test-threads=2" if name.endswith("-tests") else ""
        command = ("cd " + remote + " && TMPDIR=" + remote
                   + " TAYPEER_TEST_BIN_DIR=" + remote + " ./" + name + flags)
        result = subprocess.run(adb + ["shell", command], text=True,
                                capture_output=True, timeout=240)
        results[name] = result.returncode
        if "panicked at" in result.stdout + result.stderr:
            results[name] = 1
        log.write("\nCOMMAND " + name + "\n" + result.stdout + result.stderr
                  + "exit=" + str(result.returncode) + "\n")
        log.flush()
        print(name + " exit=" + str(result.returncode), flush=True)
print(json.dumps(results))
# A known failed cleanup probe is reported explicitly, never counted as a passed test.
passed = all(code == 0 for name, code in results.items() if name != "memory-probe")
raise SystemExit(0 if passed and results["memory-probe"] == 2 else 1)
