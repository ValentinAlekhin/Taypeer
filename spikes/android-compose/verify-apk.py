"""Check the native packaging of the synthetic ARM64 probe, without executing it."""

import struct
import sys
import zipfile


def verify(path):
    with zipfile.ZipFile(path) as apk:
        libraries = [name for name in apk.namelist() if name.startswith("lib/") and name.endswith(".so")]
        assert "lib/arm64-v8a/libtaypeer_ui_probe.so" in libraries, "Rust library missing"
        assert "lib/arm64-v8a/libjnidispatch.so" in libraries, "JNA library missing"
        for name in libraries:
            assert name.startswith("lib/arm64-v8a/"), f"Unexpected ABI: {name}"
            binary = apk.read(name)
            assert binary[:6] == b"\x7fELF\x02\x01", f"Not ELF64 little-endian: {name}"
            assert struct.unpack_from("<H", binary, 18)[0] == 183, f"Not AArch64: {name}"
            offset = struct.unpack_from("<Q", binary, 32)[0]
            size, count = struct.unpack_from("<HH", binary, 54)
            loads = 0
            for index in range(count):
                header = struct.unpack_from("<IIQQQQQQ", binary, offset + index * size)
                if header[0] == 1:
                    loads += 1
                    assert header[7] >= 16384, f"Insufficient LOAD alignment: {name}"
                    assert header[2] % 16384 == header[3] % 16384, f"Misaligned LOAD: {name}"
            assert loads, f"Missing LOAD segments: {name}"
            print(f"{name}: AArch64, {loads} LOAD segments, 16 KiB alignment")


if __name__ == "__main__":
    verify(sys.argv[1])
