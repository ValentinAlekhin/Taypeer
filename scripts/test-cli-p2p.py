#!/usr/bin/env python3
"""Three real CLI processes and native profiles; synthetic database content only."""
import argparse
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import time

spec = importlib.util.spec_from_file_location("cli_pty", Path(__file__).with_name("test-cli-pty.py"))
pty_cli = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pty_cli)


def response(terminal, command):
    result = pty_cli.json_response(terminal.command(command))
    assert not isinstance(result, dict) or "error" not in result, f"Command failed: {command.split()[0]}: {result.get('error')}"
    return result


def unlock(terminal):
    terminal.send("db unlock")
    terminal.wait(b"Master password: ")
    terminal.send("PUBLIC_PTY_MASTER_CANARY")
    result = terminal.wait(b"\x1b[6n")
    terminal.wait(b"\x1b[?25h")
    assert b'"error"' not in result, "Unlock failed"


def admit(manager, recipient, path):
    code = response(manager, "invite create")
    # Bearer codes are held only in memory and entered through hidden input.
    manager.transcript = b""
    recipient.send(f'invite join "{path}"')
    recipient.wait(b"Invitation JSON code: ")
    recipient.send(json.dumps(code, separators=(",", ":")))
    result = recipient.wait(b"\x1b[6n")
    recipient.wait(b"\x1b[?25h")
    pending = pty_cli.json_response(result)
    assert "Pending" in pending, "Join did not reach approval"
    request = pending["Pending"]
    assert b'"error"' not in manager.command(f"invite approve {request}"), "Approval failed"
    joined = response(recipient, f"invite resume {request}")
    assert "Received" in joined, "Approved archive was not received"
    unlock(recipient)


def create_entry(terminal, group, title):
    command = f'entry create --group {group} --title "{title}"'
    for _ in range(5):
        result = terminal.command(command)
        if b'"error"' not in result:
            return
        error = pty_cli.json_response(result)["error"]
        # Receipt delivery can invalidate the exact file generation while the editor
        # is open. Reapply received data and confirm the same retained draft; never
        # repeat entry creation after an uncertain write or suppress another error.
        assert error.get("detail") == {"Service": {"Storage": "Changed"}}, error
        response(terminal, "sync apply")
        command = "draft save"
        time.sleep(0.1)
    raise AssertionError("The same draft could not be confirmed after receipt contention")


def await_entry(terminal, title):
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        entries = response(terminal, "entry list")
        if any(entry["title"] == title for entry in entries):
            return
        time.sleep(0.25)
    raise AssertionError("Admitted change did not converge")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", nargs="?", default="target/debug/taypeer-cli")
    parser.add_argument("--relay")
    parser.add_argument("--relay-ca")
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    terminals = []
    with tempfile.TemporaryDirectory(prefix="taypeer-public-p2p-") as directory:
        root = Path(directory)
        try:
            for label in ("a", "b", "c"):
                terminals.append(pty_cli.Terminal(binary, root / label / "profile"))
            a, b, c = terminals
            a.create(root / "a.taypeer")
            if args.relay:
                for terminal in terminals:
                    address = response(terminal, f'sync start --relay "{args.relay}" --relay-only --relay-ca "{args.relay_ca}"')
                    assert args.relay in json.dumps(address), "Relay route was not advertised"
                    assert "ip" not in address.get("addrs", {}), "Direct route advertised in relay-only mode"
            group = response(a, 'group create --name "PUBLIC shared group"')["id"]
            admit(a, b, root / "b.taypeer")
            print("P2P: second process admitted and unlocked", flush=True)
            admit(a, c, root / "c.taypeer")
            print("P2P: third process admitted and unlocked", flush=True)
            create_entry(a, group, "PUBLIC network entry")
            await_entry(b, "PUBLIC network entry")
            await_entry(c, "PUBLIC network entry")
            print("P2P: signed change converged across three processes", flush=True)
            assert b'"error"' not in b.command("db lock")
            create_entry(a, group, "PUBLIC while locked")
            await_entry(c, "PUBLIC while locked")
            time.sleep(3)
            unlock(b)
            await_entry(b, "PUBLIC while locked")
            for terminal in terminals:
                assert b"PUBLIC_PTY_MASTER_CANARY" not in terminal.transcript
            print("P2P: locked receipt, unlock application and hidden passwords passed", flush=True)
            for terminal in terminals:
                assert b'"error"' not in terminal.command("sync stop")
            # Actual child prepares ciphertext; the parent publishes a separate file.
            password = root / "PUBLIC recovery password.json"
            password.write_text(json.dumps("PUBLIC_RECOVERED_MASTER_CANARY"), encoding="utf-8")
            destination = root / "recovered.taypeer"
            source = (root / "a.taypeer").read_bytes()
            command = f'device recover "{destination}" --operation {"ab" * 32} --input "{password}"'
            recovered = response(a, command)
            assert response(a, command) == recovered, "Recovery retry changed identity"
            assert (root / "a.taypeer").read_bytes() == source, "Recovery changed the original"
            assert b'"error"' not in a.command("db close")
            a.send(f'db open "{destination}"')
            a.wait(b"Master password: ")
            a.send("PUBLIC_RECOVERED_MASTER_CANARY")
            opened = a.wait(b"\x1b[6n")
            a.wait(b"\x1b[?25h")
            assert b'"error"' not in opened, "Recovered archive could not be opened"
            assert len(response(a, "entry list")) == 2, "Recovery lost accepted entries"
            assert response(a, "sync sources") == [], "Accepted sources became pending"
            assert not response(a, "sync collect")["held"], "Recovery blocked safe collection"
            assert b'"error"' not in a.command('group create --name "PUBLIC recovered manager"')
            assert b"PUBLIC_RECOVERED_MASTER_CANARY" not in a.transcript
            print("P2P: separate recovery, exact retry, reopen, collection and editing passed", flush=True)

        finally:
            for terminal in terminals:
                terminal.close()


if __name__ == "__main__":
    main()
