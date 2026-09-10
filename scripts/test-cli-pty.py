#!/usr/bin/env python3
"""Exercise the actual CLI terminal using public synthetic data only (POSIX)."""
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import subprocess
import sys
import tempfile
import time


def json_response(data):
    clean = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", data).decode()
    start = min(index for index in [clean.find("{"), clean.find("[")] if index >= 0)
    return json.JSONDecoder().raw_decode(clean[start:])[0]


class Terminal:
    def __init__(self, binary):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.execv(binary, [binary, "--json", "session"])
        self.pending = b""
        self.transcript = b""
        self.wait(b"\x1b[?25h")

    def send(self, text):
        os.write(self.fd, text.encode() + b"\r")

    def wait(self, marker, timeout=30):
        deadline = time.monotonic() + timeout
        while marker not in self.pending:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.fd], [], [], remaining)[0]:
                raise AssertionError("CLI terminal response timed out")
            data = os.read(self.fd, 65536)
            if not data:
                raise AssertionError("CLI terminal closed unexpectedly")
            self.pending += data
            self.transcript += data
            # Act as a terminal for Reedline's cursor-position queries.
            for _ in range(data.count(b"\x1b[6n")):
                os.write(self.fd, b"\x1b[1;1R")
        end = self.pending.index(marker) + len(marker)
        result, self.pending = self.pending[:end], self.pending[end:]
        return result

    def command(self, text):
        self.send(text)
        # Input redraws also query the cursor; first wait until Enter is submitted.
        self.wait(b"\r\n")
        result = self.wait(b"\x1b[6n")
        self.wait(b"\x1b[?25h")
        return result

    def create(self, path):
        self.send(f'db create "{path}" --name "PUBLIC PTY"')
        self.wait(b"Master password: ")
        self.send("PUBLIC_PTY_MASTER_CANARY")
        self.wait(b"Repeat master password: ")
        self.send("PUBLIC_PTY_MASTER_CANARY")
        result = self.wait(b"\x1b[6n")
        self.wait(b"\x1b[?25h")
        assert b'"locked":false' in result

    def close(self):
        os.close(self.fd)
        try:
            os.kill(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            os.waitpid(self.pid, 0)
        except ChildProcessError:
            pass


def main():
    binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/taypeer-cli").resolve())
    terminal = Terminal(binary)
    try:
        with tempfile.TemporaryDirectory(prefix="taypeer-public-pty-") as directory:
            path = Path(directory) / "public.taypeer"
            terminal.create(path)
            result = terminal.command('group create --name "PUBLIC group"')
            match = re.search(rb'\{"id":"([^"]+)"', result)
            assert match, "group creation returned no identity"
            group = match[1].decode()
            terminal.send(f'entry create --group {group} --title "PUBLIC entry" --password-prompt')
            terminal.wait(b"Entry password: ")
            terminal.send("PUBLIC_PTY_ENTRY_CANARY")
            result = terminal.wait(b"\x1b[6n")
            terminal.wait(b"\x1b[?25h")
            assert b'"error"' not in result
            shown = terminal.command("entry list")
            assert b"PUBLIC entry" in shown
            assert b"PUBLIC_PTY_ENTRY_CANARY" not in terminal.transcript
            assert b"PUBLIC_PTY_MASTER_CANARY" not in terminal.transcript
            terminal.create(Path(directory) / "public-second.taypeer")
            assert b"PUBLIC entry" not in terminal.command("entry list")
            assert b"PUBLIC entry" in terminal.command('search "PUBLIC entry"')
            terminal.command("db close")
            assert b"PUBLIC entry" in terminal.command("entry list")
            prepared = json_response(terminal.command(f"group trash {group}"))
            selection = Path(directory) / "public-selection.json"
            selection.write_text(json.dumps(prepared), encoding="utf-8")
            entry = json_response(terminal.command("entry list"))[0]["id"]
            terminal.command(f"draft edit {entry}")
            confirmation = f'trash confirm --input "{selection}" --yes --operation PUBLIC-PTY-trash'
            assert b"EditorAlreadyOpen" in terminal.command(confirmation)
            terminal.command("draft discard")
            assert b'"error"' not in terminal.command(confirmation)
            assert json_response(terminal.command("entry list")) == []
            assert len(json_response(terminal.command("trash list"))) == 2
            restore = json_response(terminal.command(f"trash prepare restore group {group}"))
            selection.write_text(json.dumps(restore), encoding="utf-8")
            assert b'"error"' not in terminal.command(
                f'trash confirm --input "{selection}" --yes --operation PUBLIC-PTY-restore')
            assert len(json_response(terminal.command(f"history list {entry}"))) == 2
            assert b'"error"' not in terminal.command(f"group move {group} --first --operation PUBLIC-PTY-move")
            assert b"PUBLIC_PTY_ENTRY_CANARY" not in terminal.transcript
            assert b"PUBLIC_PTY_MASTER_CANARY" not in terminal.transcript
            attachment_input = Path(directory) / "PUBLIC attachment.txt"
            attachment_input.write_bytes(b"PUBLIC PTY attachment contents")
            terminal.command(f"draft edit {entry}")
            add_attachment = f'attachment add --draft "{attachment_input}" --operation PUBLIC-PTY-attachment'
            assert b'"error"' not in terminal.command(add_attachment)
            assert len(json_response(terminal.command("attachment list --draft"))["attachments"]) == 1
            terminal.command("draft discard")
            assert json_response(terminal.command(f"attachment list --entry {entry}"))["attachments"] == []
            terminal.command(f"draft edit {entry}")
            assert b'"error"' not in terminal.command(add_attachment)
            assert b'"error"' not in terminal.command("icon set --draft key-round --operation PUBLIC-PTY-icon")
            terminal.command("db lock")
            denied = terminal.command("entry list")
            assert b'"Closed"' in denied
            terminal.send("db unlock")
            terminal.wait(b"Master password: ")
            terminal.send("PUBLIC_PTY_MASTER_CANARY")
            terminal.wait(b"\x1b[6n")
            terminal.wait(b"\x1b[?25h")
            assert b"PUBLIC entry" in terminal.command("entry list")
            assert b'"error"' in terminal.command("attachment list --draft")
            assert b'"error"' not in terminal.command("draft restore")
            restored = json_response(terminal.command("attachment list --draft"))
            assert len(restored["attachments"]) == 1
            blob = restored["attachments"][0]["contents"][0]["id"]
            output = Path(directory) / "PUBLIC exported.txt"
            assert b'"error"' not in terminal.command(f'attachment export --draft {blob} --output "{output}"')
            assert output.read_bytes() == attachment_input.read_bytes()
            assert b'"error"' not in terminal.command("draft save")
            terminal.send(f'entry create --group {group} --title "PUBLIC cancelled" --password-prompt')
            terminal.wait(b"Entry password: ")
            os.write(terminal.fd, b"\x03")
            assert b"input_error" in terminal.wait(b"\x1b[6n")
            terminal.wait(b"\x1b[?25h")
            assert b"PUBLIC entry" in terminal.command("entry list")
            assert b"PUBLIC cancelled" not in terminal.command("entry list")
            terminal.command("db close")
            assert b"no_database" in terminal.command("entry list")
            terminal.send("exit")
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                ended, status = os.waitpid(terminal.pid, os.WNOHANG)
                if ended:
                    assert os.waitstatus_to_exitcode(status) == 0
                    break
                if select.select([terminal.fd], [], [], 0.05)[0]:
                    try:
                        data = os.read(terminal.fd, 65536)
                    except OSError:
                        data = b""
                    for _ in range(data.count(b"\x1b[6n")):
                        os.write(terminal.fd, b"\x1b[1;1R")
            else:
                raise AssertionError("CLI did not terminate")
            result = subprocess.run(
                [binary, "--json", "--file", str(path), "--password-stdin", "entry", "list"],
                input=b"PUBLIC_PTY_MASTER_CANARY", capture_output=True, timeout=30,
            )
            assert result.returncode == 0, "worker retained the writer lock after exit"
            assert len(json.loads(result.stdout)) == 1
        print("CLI PTY: hidden input, cancellation, draft guard, trash/restore, move, binary draft cancel/restore/export/save, lock, close and reopen passed")
    finally:
        terminal.close()


if __name__ == "__main__":
    main()
