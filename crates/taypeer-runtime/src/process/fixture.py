"""PUBLIC IPC fixture: no credentials, user files or networking."""
import json
import pathlib
import struct
import sys
import time

mode, ready = sys.argv[1:]

def read():
    header = sys.stdin.buffer.read(4)
    if len(header) != 4:
        sys.exit(0)
    length = struct.unpack("<I", header)[0]
    return json.loads(sys.stdin.buffer.read(length))

def message(value):
    data = json.dumps(value).encode()
    sys.stdout.buffer.write(struct.pack("<I", len(data)) + data)
    sys.stdout.buffer.flush()

def response(result):
    message({"Response": {"result": result}})

read()
if mode == "opening":
    pathlib.Path(ready).write_text("PUBLIC opening")
    time.sleep(60)
response({"Ok": "PUBLIC database"})
if mode == "exit_idle":
    time.sleep(0.3)
    sys.exit(0)
while True:
    command = read()
    if command == "Lock":
        if mode == "draft_error":
            response({"Err": {"Service": {"Storage": "Io"}}})
        else:
            response({"Ok": None})
        sys.exit(0)
    pathlib.Path(ready).write_text("PUBLIC command started")
    if mode == "hang":
        time.sleep(60)
    if mode == "partial":
        sys.stdout.buffer.write(b"\x20\x00\x00\x00{")
        sys.stdout.buffer.flush()
        time.sleep(60)
    if mode == "late":
        time.sleep(0.25)
    if mode == "io":
        message({"Io": {"Snapshot": {"known": None}}})
        read()
    response({"Ok": "PUBLIC revealed value"})
