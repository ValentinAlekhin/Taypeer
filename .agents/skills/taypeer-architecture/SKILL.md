---
name: taypeer-architecture
description: Assess architecture impact before Taypeer changes, maintain its LikeC4 model before implementation, and keep model, contracts and code aligned. Applies to Taypeer features, fixes, refactors, design and documentation; not unrelated repositories.
---

# Taypeer architecture

Find the repository root three levels above this skill directory. Read
[the architecture guide](../../../docs/architecture.md) for view IDs, notation,
commands and document ownership; consult the relevant specification and contracts.

## Before implementation

Identify the affected model elements and interactions before each task. A change
to responsibilities, dependencies, data or trust boundaries, public interfaces,
deployment or cross-component behavior has architecture impact. First edit the
model and run the pinned CLI validation, then implement the change. Requirement
changes belong in the specification and exact contracts remain in Markdown.

A fix inside an existing responsibility, a local refactor, a wording correction
or a visual adjustment can have no architecture impact. Explain that briefly in
the work update and continue without a cosmetic model edit. No separate approval,
PR report or architecture commit is required for already authorized work.

## Model and tools

- Use the project `likec4` MCP to read elements, relationships and views. If it is
  unavailable, read `architecture/*.c4` and use the local CLI; this does not block work.
- Use the installed `likec4-dsl` skill for syntax only. Inspect `--help` and validate
  with the pinned package: its parser is authoritative when upstream examples differ.
  LikeC4 1.59.3 rejects view-level `metadata`, the identifier `icons`, and self-relations;
  put source references on elements and describe recursive domain structure in prose.
- Use one definition per element and meaningful relationship, full FQNs across
  files, projections for views, and `instanceOf` for repeated deployments. Do not
  reproduce a runtime component for each platform or create empty Rust crates.
- Keep descriptions short. Fields, algorithms, invariants and rationale belong
  in their Markdown owner. Source/contract/evidence paths are relative to repo root.
- `scaffold` is existing bootstrap code, `planned` is future product code;
  `conceptual` and `external` classify domain entities and context. Evidence from
  a spike never means product acceptance. Change statuses only with actual evidence.

Run commands from the repository root, respecting its RTK instructions:

```sh
pnpm arch:format
pnpm arch:check
pnpm arch:dev
```

Before implementation, full `pnpm exec likec4 validate architecture` must pass
after a model change. File-scoped diagnostics help debugging but do not replace
whole-model validation. After implementation reconcile the model and contracts,
run `sh scripts/check.sh`, and visually inspect changed views. Build the viewer
with `pnpm arch:build` when changing tooling or preparing a shareable artifact.

The checks validate documentation artifacts, not Rust behavior or the chronological
order of edits. Report product tests separately; architecture tooling does not
close security, Android, memory or KDBX acceptance gates.
