---
name: taypeer-ui-testing
description: Develop and verify Taypeer macOS UI behavior with readable Rust scenarios against the real AppView and runtime using GPUI Kit test support. Use for UI behavior changes and regressions; native macOS integration still requires device checks.
---

# Taypeer UI testing

Use the repository architecture and Rust skills. Read [testing documentation](../../../docs/testing.md)
for commands, supported configurations and remaining acceptance gaps, then inspect
[the session](../../../apps/taypeer/src/desktop/testing.rs) and
[scenarios](../../../apps/taypeer/tests/ui.rs).

Before experimenting, state the expected user-visible behavior and durable outcome.
Build a short Rust scenario, run it, investigate failures and retain the finished
scenario in the repository so Cargo can replay it without AI or desktop focus.

Use the real AppView and its handlers. Do not copy a screen into a test, expose stores
as a public API, or mutate business state to simulate user actions. Use the pinned
`gpui-kit` source selected by Cargo.lock: `crates/kit/src/test.rs` describes events
and `crates/base/src/test_support.rs` describes observation. Do not guess APIs from
another revision. Add IDs to real controls; resolve repeated IDs with `within`.

Assert both the relevant control state (visibility, enabled state, focus, value,
selection or geometry) and the observable operation result. Missing accessibility
facts do not prove a control is enabled. Saving requires reopening the file;
exchange requires observing the other device. Receiving ciphertext while locked
does not establish that it has been applied.

Use fresh temporary profiles, explicit worker paths and public synthetic fixtures.
Never change HOME, touch user preferences, invoke Keychain as a fallback, enable
public relay, or use the system clipboard. Drive file selection through the narrow scripted picker used by the session;
keep the actual UI handler and subsequent file operations. Pump GPUI while waiting for real background work with a wall-clock deadline;
advancing virtual time alone does not finish network or process I/O.

Keep failure steps and diagnostics free of input values, invitation codes and protected
content. Do not debug-print ElementSnapshot: it includes labels and values. An element
tree is not a screenshot. Report unavailable rendering explicitly. Do not weaken an
assertion, replace an expected result with the observed failure, or ignore a scenario
to make a run green. Fix the cause or report the precise unresolved limitation.

Use Peekaboo for native windows, actual desktop input, IME, Keychain and macOS
integration. These checks complement Rust scenarios and are not part of their replay.
