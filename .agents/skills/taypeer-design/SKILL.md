---
name: taypeer-design
description: Edit, review, build and verify this repository's Taypeer OpenPencil mockups and keep their design rules aligned with spec.md. Use for Taypeer screens, entry tabs, forms, icons and design-artifact maintenance; not for unrelated application implementation.
---

# Taypeer design

Locate the project root from this skill directory (`../../..`). Read `wireframes/README.md` for design rules, commands and artifact ownership; consult the relevant part of `spec.md` for behavior. Reuse the available `open-pencil` skill for editor/API operations and its versioned compatibility reference when relevant.

## Work from the actual design

1. Inspect the relevant saved previews and source before changing a screen. Read the matching live document when the user is editing in OpenPencil; match its file path using document discovery.
2. Edit `wireframes/source/compact.js` and the source assets. These generate the editable FIG. Update the specification only when behavior or an accepted design rule changes.
3. Keep shared geometry, field variants and icon components centralized. Preserve the user's compact GPUI direction and existing decisions recorded in the README. Routine edits do not restart a grill interview or require a new approval ritual.
4. Execute the workflow below, inspect actual PNGs, and fix defects at the shared component or layout owner.

## Commands

Run from the project root, respecting its RTK instructions:

```sh
rtk proxy python3 wireframes/scripts/design.py build
rtk proxy python3 wireframes/scripts/design.py verify
rtk proxy python3 wireframes/scripts/design.py export
```

`build` creates `wireframes/build/taypeer.fig`; it does not replace the delivered FIG. `verify` checks the serialized candidate. `export` generates a complete preview set under `wireframes/build/previews/`, replacing old previews only after successful rendering. Inspect affected screens at full size and relevant contact sheets. Use `--published` with verify/export to inspect the delivered file instead.

Once the candidate is reviewable and the requested changes are authorized, discover the live document ID and run:

```sh
rtk proxy openpencil documents --json
rtk proxy python3 wireframes/scripts/design.py publish --document-id DOCUMENT_ID
```

`publish` here means replacing the local deliverable, not deploying or uploading. It checks source/candidate hashes, saved-file changes, the last published scene, and unsaved live changes, then keeps a backup under `build/backups/`. There is no force flag. If a guard rejects replacement, preserve and port manual edits into the source; do not fake the baseline. The initial baseline is established only by a build that reproduces the saved scene. If the app is unavailable, candidate preparation remains possible but replacement waits for a trustworthy live check.

Reload `wireframes/taypeer.fig` in OpenPencil after publication. Re-query IDs, switch to the correct page, select the changed artboard and zoom to fit. Never save the previous live state over the newly published file. Run export with `--published` if an index for the delivered file is needed.

## Maintenance

- Keep source, licenses, the compact Lucide base library and delivered FIG. Generated candidates, reports, previews and backups belong under ignored `build/`.
- `assets/lucide-extra.json` is the single source for additional normalized Lucide paths. Do not re-inline another copy into the generator.
- Test workflow changes with `rtk proxy python3 -m unittest discover -s wireframes/scripts -p 'test_*.py'`. These tests cover preservation of manual edits and failed exports; they do not replace visual review.
- The snapshot guard compares visual scene data, not all possible Figma metadata. Preserve backups and review manual changes explicitly.
- Do not add product artboards merely to enumerate every state. Use component examples and concise behavior rules where those suffice. Static FIG review does not prove GPUI runtime behavior.
