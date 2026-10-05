#!/usr/bin/env python3
"""Complete-inventory trust recovery through a real terminal; PUBLIC data only."""
import argparse
import importlib.util
import json
from pathlib import Path
import tempfile

spec = importlib.util.spec_from_file_location("cli_pty", Path(__file__).with_name("test-cli-pty.py"))
pty_cli = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pty_cli)

PASSWORD = "PUBLIC_RECOVERED_MASTER_CANARY"


def response(terminal, command):
    result = pty_cli.json_response(terminal.command(command))
    assert not isinstance(result, dict) or result.get("error") is None, "Synthetic recovery command failed"
    return result


def close_database(terminal):
    # JSON null is an intentional successful close, outside the object/list decoder.
    assert b'"error"' not in terminal.command("db close"), "Database close failed"


def open_recovered(terminal, path):
    terminal.send(f'db open "{path}"')
    terminal.wait(b"Master password: ")
    terminal.send(PASSWORD)
    opened = pty_cli.json_response(terminal.wait(b"\x1b[6n"))
    terminal.wait(b"\x1b[?25h")
    assert opened.get("error") is None, "Recovered archive could not be opened"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", nargs="?", default="target/debug/taypeer-cli")
    parser.add_argument("--public-fixture-profile", action="store_true")
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    with tempfile.TemporaryDirectory(prefix="taypeer-public-recovery-") as directory:
        root = Path(directory)
        profile = root / "profile"
        terminal = pty_cli.Terminal(binary, profile, args.public_fixture_profile)
        try:
            original = terminal.create(root / "initial.taypeer")
            response(terminal, 'group create --name "PUBLIC original group"')
            original_bytes = original.read_bytes()
            destination = root / "recovered.taypeer"
            password_file = root / "PUBLIC password.json"
            password_file.write_text(json.dumps(PASSWORD), encoding="utf-8")
            operation = "cd" * 32
            recover = f'device recover "{destination}" --operation {operation} --input "{password_file}"'
            recovered = response(terminal, recover)
            assert response(terminal, recover) == recovered, "Recovery retry changed identity"
            other = root / "PUBLIC duplicate.taypeer"
            rejected = pty_cli.json_response(terminal.command(
                f'device recover "{other}" --operation {operation} --input "{password_file}"'
            ))
            assert rejected.get("error") is not None and not other.exists(), "Recovery operation changed its bound destination"
            close_database(terminal)
            open_recovered(terminal, destination)
            assert len(response(terminal, "group list")) == 1, "Recovery lost accepted content"
            assert response(terminal, "sync sources") == [], "Accepted sources became pending"
            # This fixture has no incoming offers, unlike a live multi-peer inventory.
            assert not response(terminal, "sync collect")["held"], "Complete recovered inventory blocked collection"
            response(terminal, 'group create --name "PUBLIC recovered manager"')
            close_database(terminal)
            saved = destination.read_bytes()
            open_recovered(terminal, destination)
            assert len(response(terminal, "group list")) == 2
            assert destination.read_bytes() == saved, "Reopen rewrote the recovered archive"
            assert original.read_bytes() == original_bytes, "Recovery changed the original"
            assert PASSWORD.encode() not in terminal.transcript
        finally:
            terminal.close()
        terminal = pty_cli.Terminal(binary, profile, args.public_fixture_profile)
        try:
            open_recovered(terminal, destination)
            assert len(response(terminal, "group list")) == 2
            assert destination.read_bytes() == saved, "Restart rewrote the recovered archive"
            assert original.read_bytes() == original_bytes, "Restart changed the original"
            assert PASSWORD.encode() not in terminal.transcript
            print("Recovery CLI: exact retry, immutable destination, reopen, collection, edit, restart and preserved source passed", flush=True)
        finally:
            terminal.close()


if __name__ == "__main__":
    main()
