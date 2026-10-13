"""Guard existing JaCoCo task output against skipped or stale product R classes."""
import argparse
from pathlib import Path
import struct


REQUIRED = ("RestoreRevisionState", "ReceivedSourcesState", "RelaySettingsState")


class CoverageError(Exception):
    pass


def class_id(data: bytes) -> int:
    """JaCoCo CRC64 class identity, including its Java 9 compatibility mapping."""
    if len(data) >= 8 and data[:4] == b"\xca\xfe\xba\xbe" and data[7] == 53:
        data = data[:7] + b"\x34" + data[8:]
    checksum = 0
    for byte in data:
        checksum ^= byte
        for _ in range(8):
            checksum = (checksum >> 1) ^ (0xD800000000000000 if checksum & 1 else 0)
    return checksum


def probe_identity(data: bytes) -> set[int]:
    """Read constant-pool probe markers and IDs without loading JVM classes."""
    if len(data) < 10 or data[:4] != b"\xca\xfe\xba\xbe":
        raise CoverageError("Invalid class file in coverage output")
    count = struct.unpack_from(">H", data, 8)[0]
    offset, index = 10, 1
    strings, identities = set(), set()
    try:
        while index < count:
            tag = data[offset]
            offset += 1
            if tag == 1:
                length = struct.unpack_from(">H", data, offset)[0]
                offset += 2
                strings.add(data[offset:offset + length])
                offset += length
            elif tag == 5:
                identities.add(struct.unpack_from(">Q", data, offset)[0])
                offset += 8
                index += 1
            elif tag == 6:
                offset += 8
                index += 1
            elif tag in (3, 4, 9, 10, 11, 12, 17, 18):
                offset += 4
            elif tag in (7, 8, 16, 19, 20):
                offset += 2
            elif tag == 15:
                offset += 3
            else:
                raise CoverageError("Unknown class constant in coverage output")
            if offset > len(data):
                raise CoverageError("Truncated class constants in coverage output")
            index += 1
    except (IndexError, struct.error) as error:
        raise CoverageError("Truncated class constants in coverage output") from error
    has_runtime = any(value.startswith(b"org/jacoco/agent/rt/internal_") and
                      value.endswith(b"/Offline") for value in strings)
    if {b"$jacocoInit", b"$jacocoData"} <= strings and has_runtime:
        return identities
    return set()


def product_r_class(relative: Path) -> bool:
    return (relative.parts[:2] == ("dev", "taypeer") and
            relative.parts[:3] != ("dev", "taypeer", "bridge") and
            relative.suffix == ".class" and relative.name.startswith("R") and
            relative.name != "R.class" and not relative.name.startswith("R$"))


def verify(raw_roots: list[Path], output: Path, *, repaired: bool) -> int:
    output = output.resolve()
    seen, checked = set(), 0
    for raw_root in raw_roots:
        raw_root = raw_root.resolve()
        if raw_root == output or raw_root in output.parents or output in raw_root.parents:
            raise CoverageError("Original and instrumented class directories must be separate")
        for raw in sorted(raw_root.rglob("R*.class")):
            relative = raw.relative_to(raw_root)
            if not product_r_class(relative):
                continue
            original = raw.read_bytes()
            if probe_identity(original):
                raise CoverageError(f"Expected original, uninstrumented class: {relative}")
            copied = output / relative
            if not copied.is_file():
                raise CoverageError(f"JaCoCo task output is missing: {relative}")
            actual = copied.read_bytes()
            has_exact_probes = class_id(original) in probe_identity(actual)
            # Incremental reruns can leave previously repaired, unchanged classes.
            # Accept only the original copy or probes belonging to that exact input.
            if original != actual and not has_exact_probes:
                raise CoverageError(f"Stale or unrelated JaCoCo task output: {relative}")
            if relative.parent == Path("dev/taypeer") and relative.stem in REQUIRED:
                seen.add(relative.stem)
                if repaired and not has_exact_probes:
                    raise CoverageError(f"Product class has no matching JaCoCo probes: {relative}")
            checked += 1
    missing = set(REQUIRED) - seen
    if missing:
        raise CoverageError("Required product coverage classes missing: " + ", ".join(sorted(missing)))
    return checked


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--raw", action="append", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--phase", choices=("before", "after"), required=True)
    args = parser.parse_args()
    try:
        count = verify(args.raw, args.output, repaired=args.phase == "after")
    except (CoverageError, OSError) as error:
        parser.exit(1, f"Coverage instrumentation guard: {error}\n")
    print(f"Coverage instrumentation guard: {count} product R classes checked ({args.phase})")


if __name__ == "__main__":
    main()
