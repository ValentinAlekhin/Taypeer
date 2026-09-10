#!/usr/bin/env python3
"""Create a disposable public demo through the real CLI, never user credentials."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "target/release/taypeer-cli"
DIRECTORY = ROOT / "artifacts/cli-binary-demo"
DATABASE = DIRECTORY / "public.taypeer"
PASSWORD = b"PUBLIC-DEMO-ONLY-42"


def run(*arguments, opened=True):
    command = [str(BINARY), "--json", "--password-stdin"]
    if opened:
        command += ["--file", str(DATABASE)]
    result = subprocess.run(command + list(arguments), input=PASSWORD, capture_output=True, timeout=60)
    if result.returncode:
        raise RuntimeError("CLI demo command failed; no existing database was overwritten")
    return json.loads(result.stdout)


def main():
    if not BINARY.is_file():
        raise RuntimeError("Build first: cargo build --release --locked -p taypeer-cli")
    if DATABASE.exists():
        raise RuntimeError("Demo database already exists; refusing to replace it")
    DIRECTORY.mkdir(parents=True, exist_ok=True)
    run("db", "create", str(DATABASE), "--name", "PUBLIC Taypeer demo", opened=False)
    group = run("group", "create", "--name", "PUBLIC accounts")["id"]
    form = {
        "title": {"action": "set", "value": "PUBLIC sample account"},
        "username": {"action": "set", "value": "public-user@example.invalid"},
        "password": {"action": "set", "value": "PUBLIC-ENTRY-ONLY-42"},
        "notes": {"action": "set", "value": "Synthetic data created by the real CLI."},
    }
    with tempfile.TemporaryDirectory(prefix="taypeer-public-demo-") as temporary:
        input_path = Path(temporary) / "public-input.json"
        input_path.write_text(json.dumps(form), encoding="utf-8")
        entry = run("entry", "create", "--group", group, "--input", str(input_path))
    run("entry", "update", entry, "--title", "PUBLIC saved account")
    cloned_entry = run("entry", "clone", entry, "--group", group, "--title", "PUBLIC cloned account",
                       "--operation", "PUBLIC demo clone")
    destination = run("group", "create", "--name", "PUBLIC archive")["id"]
    run("entry", "move", cloned_entry, "--group", destination, "--operation", "PUBLIC demo move")
    cloned_group = run("group", "clone", destination, "--name", "PUBLIC trash example",
                       "--operation", "PUBLIC demo clone subtree")[0]["id"]
    with tempfile.TemporaryDirectory(prefix="taypeer-public-selection-") as temporary:
        selection = Path(temporary) / "selection.json"
        selection.write_text(json.dumps(run("group", "trash", cloned_group)), encoding="utf-8")
        run("trash", "confirm", "--input", str(selection), "--yes", "--operation", "PUBLIC demo trash subtree")
        selection.write_text(json.dumps(run("entry", "trash", entry)), encoding="utf-8")
        run("trash", "confirm", "--input", str(selection), "--yes", "--operation", "PUBLIC demo trash entry")
        selection.write_text(json.dumps(run("trash", "prepare", "restore", "entry", entry,
                                            "--destination", group)), encoding="utf-8")
        run("trash", "confirm", "--input", str(selection), "--yes", "--operation", "PUBLIC demo restore entry")
    assert len(run("entry", "list")) == 2
    assert len(run("history", "list", entry)) == 3
    assert len(run("trash", "list")) == 2
    original = DIRECTORY / "PUBLIC-original.txt"
    replacement = DIRECTORY / "PUBLIC-replacement.txt"
    icon = DIRECTORY / "PUBLIC-icon.svg"
    original.write_bytes(b"PUBLIC original attachment\n")
    replacement.write_bytes(b"PUBLIC replacement attachment\n")
    icon.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24">'
                    '<circle cx="12" cy="12" r="10" fill="#336699"/></svg>', encoding="utf-8")
    run("attachment", "add", "--entry", entry, str(original), "--operation", "PUBLIC demo attachment")
    attachment_view = run("attachment", "list", "--entry", entry)
    attachment = attachment_view["attachments"][0]["id"]
    old_blob = attachment_view["attachments"][0]["contents"][0]["id"]
    old_revision = run("history", "list", entry)[-1]["id"]
    run("attachment", "rename", "--entry", entry, attachment, "--name", "PUBLIC notes.txt")
    run("attachment", "replace", "--entry", entry, attachment, str(replacement),
        "--operation", "PUBLIC demo replacement")
    run("icon", "file", "--entry", entry, str(icon), "--operation", "PUBLIC demo icon")
    run("icon", "set", "--group", group, "folder-open")
    run("appearance", "set", "--entry", entry, "--foreground", "#336699", "--background", "#e6f0ff")
    run("attachment", "export", "--entry", entry, "--revision", old_revision, old_blob,
        "--output", str(DIRECTORY / "PUBLIC-exported-history.txt"))
    assert (DIRECTORY / "PUBLIC-exported-history.txt").read_bytes() == original.read_bytes()
    binary_clone = run("entry", "clone", entry, "--group", destination, "--title", "PUBLIC binary clone",
                       "--operation", "PUBLIC demo binary clone")
    cloned_attachment = run("attachment", "list", "--entry", binary_clone)["attachments"][0]
    assert cloned_attachment["id"] != attachment
    usage = run("storage", "usage")
    assert usage["attachment_bytes"] == original.stat().st_size + replacement.stat().st_size
    (DIRECTORY / "storage.json").write_text(json.dumps(usage, indent=2), encoding="utf-8")
    (DIRECTORY / "restore-trash.json").write_text(
        json.dumps(run("trash", "prepare", "restore", "group", cloned_group), indent=2), encoding="utf-8")
    (DIRECTORY / "tree.json").write_text(json.dumps(run("group", "tree"), indent=2), encoding="utf-8")
    checksum = hashlib.sha256(DATABASE.read_bytes()).hexdigest()
    (DIRECTORY / "README.md").write_text(
        "# Public CLI demo\n\n"
        "Synthetic data only. No user data or real credentials.\n\n"
        f"Master password: `{PASSWORD.decode()}`\n\n"
        "From repository root:\n\n"
        "```sh\nrtk target/release/taypeer-cli --lang ru --file artifacts/cli-binary-demo/public.taypeer session\n```\n\n"
        "Then run `group tree`, `entry list`, `trash list`, `db lock`, `db unlock`, `exit`.\n\n"
        "Restore the reviewed example subtree inside that session:\n\n"
        "```text\ntrash confirm --input artifacts/cli-binary-demo/restore-trash.json --yes --operation PUBLIC-review-restore\n```\n\n"
        f"Original entry history: `history list {entry}`.\n\n"
        "Format: development container 1, schema 3; not a stable compatibility fixture.\n"
        "Source: scripts/create-cli-demo.py. Reproduction into an absent destination: "
        "`rtk proxy python3 scripts/create-cli-demo.py`.\n"
        "Expected: two active groups, three active entries; attachment replacement history, "
        "a custom SVG, a bundled group icon, explicit colors and a binary clone with a new attachment ID; "
        "one cloned group and its entry in the trash. The clone was moved before subtree cloning.\n"
        "Binary commands inside the session:\n\n"
        f"```text\nattachment list --entry {entry}\nicon show --entry {entry}\nstorage usage\n```\n\n"
        f"Original attachment revision: `{old_revision}`; blob: `{old_blob}`.\n"
        "`PUBLIC-exported-history.txt` was exported through the CLI and compared with its original.\n"
        "Content origin: generated public strings and SVG in that script; no external content.\n"
        f"SHA-256: `{checksum}`\n",
        encoding="utf-8",
    )
    print("PUBLIC CLI demo created: artifacts/cli-binary-demo/public.taypeer (attachments, icons, colors, history, tree and trash)")


if __name__ == "__main__":
    main()
