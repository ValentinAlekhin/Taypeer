---
name: taypeer-android-testing
description: Build and verify Taypeer Android UI, camera and network behavior on explicit ARM64 emulators, capture screen variants and compare reviewed visual baselines. Use for Android regressions and parity with Taypeer mockups; physical hardware acceptance remains separate.
---

# Taypeer Android testing

Use `apps/taypeer-android/qa.py`; do not reconstruct SDK paths, APK installation,
instrumentation parsing or local TLS setup in ad-hoc commands. Read
[the runbook](../../../docs/android-testing.md) for suites, matrix and evidence.
Product requirements and composition remain in `spec.md` and `wireframes/taypeer.fig`.

From repository root, respecting local RTK instructions:

```sh
rtk proxy python3 apps/taypeer-android/qa.py doctor
rtk proxy python3 apps/taypeer-android/qa.py test --serial emulator-5554 --suite full --authorize-public-biometry
rtk proxy python3 apps/taypeer-android/qa.py visual --skip-build --serial emulator-5554
```

Select serials explicitly. `--avd` boots/stops only its owned emulator; do not
stop a pre-existing emulator or reset shared adb. Keep one invocation per serial.
`--skip-build` requires current successful build; compare APK hashes when combining evidence.

Select `ui`, `platform`, `network` or exact `--test-class` for a regression;
use full API31/API36/API36-16K matrix after integrated behavior changes.
The full suite includes real OS biometric prompts on a dedicated PUBLIC AVD;
`--authorize-public-biometry` explicitly authorizes its temporary PIN/finger setup
and restoration. Use that flag only for this authorized test environment.
`full`/`ui` camera denial is a separate stage: revocation kills the UID and
must precede instrumentation. Keep the stage intact. P2P offline/restart requires
the runner-owned local strict-TLS relay; do not enable public relay for tests.
Its CA hook exists in debug only, and reverse mapping teardown must preserve
other invocations. Network startup and cleanup errors are test failures.

Inspect the visual matrix against FIG. `--baseline` compares a reviewed same-image
profile; never replace it automatically to pass. Pixel tolerance describes
renderer variation, not FIG acceptance. Builder captures do not replace real-worker tests.

Use public fixtures and dedicated test profiles only. Do not pull whole app data,
log invitations/keys or create plaintext DB fixtures. The runner extracts only
test-owned PNGs. Report suites, API/page sizes, APK hashes and failures from
summary/JUnit evidence. Zero tests and incomplete instrumentation are failures.

Optical camera quality, hardware Keystore/biometry, OEM lifecycle, native
accessibility/IME and memory acceptance remain separate conditions. Do not use
emulator successes or coverage percentages as proof of those conditions.
