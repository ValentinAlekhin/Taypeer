---
name: taypeer-ui-testing
description: Verify Taypeer desktop component appearance against OpenPencil with isolated GPUI captures, and UI behavior with real AppView/runtime Rust scenarios. Use for visual changes and UI regressions; native integration requires device checks.
---

# Taypeer UI testing

Use the repository architecture and Rust skills. Read [testing documentation](../../../docs/testing.md)
for commands, supported configurations and remaining acceptance gaps, then inspect
[the session](../../../apps/taypeer/src/desktop/testing.rs) and
[scenarios](../../../apps/taypeer/tests/ui.rs).

## Component appearance

For visual iteration, run the [bundled comparison script](scripts/compare-components.py)
from the repository root, respecting its RTK instructions:

```sh
python3 .agents/skills/taypeer-ui-testing/scripts/compare-components.py --list
python3 .agents/skills/taypeer-ui-testing/scripts/compare-components.py --case entry-row-title-rest
```

Pixel-perfect matching is not required. Follow the mockup's composition, spacing,
sizing, alignment, typography hierarchy, color roles and control states. Small
antialiasing and rasterization differences are acceptable; systematic spacing or
alignment drift is not a renderer artifact.

Open `artifacts/component-checks/<case>/layout.png` first and read the logical-pixel
geometry deltas in `report.json`, then inspect `comparison.png` for typography,
colors and state paint. The technical capture margin is not layout padding;
text-area bounds are not tight glyph bounds. Additional or missing actions are
reported separately. Metrics alone do not explain the cause. After a relevant fix,
rerun the same case and inspect its new image. Output is agent diagnostics, not
a product screen or evidence of matching design acceptance.

The [catalog](../../../apps/taypeer/crates/taypeer-ui/tests/components/cases.json)
covers fields at rest and focus, plus complete editor title/URL rows. Use a full row
for parent spacing and action placement; bare field cases measure only the field's
internal layout. Add a retained preview using the real product builder and a
uniquely named FIG target when another component is needed;
do not reproduce its paint in the test. An absent or ambiguous reference is an error.
Preserve capture scale, padding and reference geometry; do not resize screenshots,
mask mismatches or change the design to make a comparison pass.

The comparison is informational by default; zero pixel difference is not the
acceptance criterion. Use an explicit pixel tolerance and changed-pixel threshold
when a task defines one. macOS Metal, OpenPencil CLI 0.14.0
and Pillow are required; commands, supported references and render-only variants
are in [testing](../../../docs/testing.md#изолированный-рендер-компонентов).

## Behavioral scenarios

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
