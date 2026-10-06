#!/usr/bin/env python3
"""Render isolated product fields and compare with saved OpenPencil nodes."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

try:
    from PIL import Image, ImageChops, ImageDraw
except ModuleNotFoundError:
    print("component comparison: install Pillow from wireframes/requirements.txt", file=sys.stderr)
    sys.exit(2)

ROOT = Path(__file__).resolve().parents[4]
CATALOG = ROOT / "apps/taypeer/crates/taypeer-ui/tests/components/cases.json"
OPENPENCIL_VERSION = "0.14.0"


def command(args, *, stdin=None):
    result = subprocess.run(args, cwd=ROOT, input=stdin, text=True,
                            capture_output=True, check=False)
    if result.returncode:
        raise RuntimeError(f"Command failed: {args[0]}\n{result.stderr or result.stdout}")
    return result.stdout.strip()


def resolve_designs(fig, cases):
    # Names survive FIG serialization; reject ambiguity instead of using stale IDs.
    code = "const cases = " + json.dumps(cases, ensure_ascii=False) + ";\n" + """
    return cases.map(c => {
      const pages = figma.root.children.filter(p => p.name === c.design.page);
      if (pages.length !== 1) throw new Error(c.id + ': expected one design page');
      const nodes = pages[0].findAll(n => n.name === c.design.node);
      if (nodes.length !== 1) throw new Error(c.id + ': expected one design node');
      const n = nodes[0];
      let background;
      for (let parent = n.parent; parent && !background; parent = parent.parent) {
        const paints = Array.isArray(parent.fills)
          ? parent.fills.filter(f => f.visible !== false) : [];
        if (!paints.length) continue;
        if (paints.length !== 1 || paints[0].type !== 'SOLID'
            || (paints[0].opacity ?? 1) !== 1 || (paints[0].color.a ?? 1) !== 1)
          throw new Error(c.id + ': reference needs a single opaque parent background');
        const color = paints[0].color;
        background = [color.r, color.g, color.b].map(v => Math.round(v * 255));
      }
      if (!background) throw new Error(c.id + ': missing reference background');
      const texts = n.findAll(t => t.type === 'TEXT').map(t => t.characters);
      if (texts.length !== 1 || texts[0] !== c.text)
        throw new Error(c.id + ': fixture text differs from the design');
      if (n.width !== c.width || n.height !== c.height)
        throw new Error(c.id + ': fixture dimensions differ from the design');
      // The current references are axis-aligned fields without effects or
      // overflowing children. Export includes their centered focus stroke.
      const descendants = [n, ...n.findAll(() => true)];
      if (descendants.some(t => (t.rotation ?? 0) !== 0 || (t.effects ?? []).length))
        throw new Error(c.id + ': effects or rotations need an explicit export adapter');
      const bounds = n.absoluteBoundingBox;
      if (descendants.slice(1).some(t => {
        const b = t.absoluteBoundingBox;
        return b && (b.x < bounds.x || b.y < bounds.y
          || b.x + b.width > bounds.x + bounds.width
          || b.y + b.height > bounds.y + bounds.height)
          || (t.strokes ?? []).some(s => s.visible !== false);
      })) throw new Error(c.id + ': overflowing child paint needs an explicit export adapter');
      const outset = Math.max(0, ...(n.strokes ?? []).filter(s => s.visible !== false).map(s =>
        s.align === 'OUTSIDE' ? s.weight : s.align === 'CENTER' ? s.weight / 2 : 0));
      return {case_id:c.id, id:n.id, page:pages[0].name, name:n.name,
              width:n.width, height:n.height, background, outset};
    });
    """
    return json.loads(command(["openpencil", "eval", str(fig), "--stdin", "--json"], stdin=code))


def compare_pixels(reference, actual, tolerance):
    """Compare native RGB pixels; each changed pixel has a channel above tolerance."""
    if reference.size != actual.size:
        raise ValueError(f"Image dimensions differ: {reference.size} vs {actual.size}; no resizing allowed")
    diff = ImageChops.difference(reference.convert("RGB"), actual.convert("RGB"))
    histogram = diff.histogram()
    channel_values = [sum(value * count for value, count in enumerate(histogram[start:start + 256]))
                      for start in (0, 256, 512)]
    red, green, blue = diff.split()
    channel_max = ImageChops.lighter(ImageChops.lighter(red, green), blue)
    maxima = channel_max.histogram()
    changed = sum(maxima[tolerance + 1:])
    pixels = diff.width * diff.height
    metrics = {
        "width": diff.width,
        "height": diff.height,
        "changed_pixels": changed,
        "changed_percent": changed * 100 / pixels,
        "mean_absolute_channel_difference": sum(channel_values) / (pixels * 3),
        "maximum_channel_difference": channel_max.getextrema()[1],
        "pixel_tolerance": tolerance,
    }
    return diff, metrics


def save_comparison(directory, actual_path, raw_reference_path, design, capture, tolerance):
    with Image.open(actual_path) as source:
        actual = source.convert("RGBA")
    with Image.open(raw_reference_path) as source:
        raw = source.convert("RGBA")
    if actual.getchannel("A").getextrema() != (255, 255):
        raise ValueError("GPUI capture must be opaque")
    scale, padding, outset = capture["scale"], capture["padding"], design["outset"]
    if outset > padding:
        raise ValueError("reference stroke exceeds the shared capture padding")
    offset = (padding - outset) * scale
    expected = ((design["width"] + 2 * outset) * scale,
                (design["height"] + 2 * outset) * scale)
    canvas_size = ((design["width"] + 2 * padding) * scale,
                   (design["height"] + 2 * padding) * scale)
    if offset != int(offset) or raw.size != expected or actual.size != canvas_size:
        raise ValueError("export/capture dimensions differ from the declared bounds; no resizing allowed")
    reference = Image.new("RGBA", actual.size, tuple(design["background"]) + (255,))
    reference.alpha_composite(raw, (int(offset), int(offset)))
    diff, metrics = compare_pixels(reference, actual, tolerance)
    reference.save(directory / "reference.png")
    overlay = Image.blend(reference, actual, 0.5)
    overlay.save(directory / "overlay.png")
    diff.save(directory / "diff.png")
    panels = [("OpenPencil reference", reference), ("GPUI actual", actual),
              ("50% overlay", overlay), ("Absolute RGB difference", diff)]
    panel_height = actual.height + 40
    montage = Image.new("RGB", (actual.width + 32, panel_height * 4 + 16), "#f3f3f3")
    draw = ImageDraw.Draw(montage)
    for index, (label, image) in enumerate(panels):
        top = 16 + index * panel_height
        draw.text((16, top), label, fill="#222222")
        montage.paste(image.convert("RGB"), (16, top + 24))
    montage.save(directory / "comparison.png")
    return metrics


def bounded_float(value):
    number = float(value)
    if not math.isfinite(number) or not 0 <= number <= 100:
        raise argparse.ArgumentTypeError("must be a finite percentage between 0 and 100")
    return number


def run():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", action="append", help="case ID; repeat to select multiple cases")
    parser.add_argument("--list", action="store_true", help="list the component catalog without rendering")
    parser.add_argument("--fig", type=Path, default=ROOT / "wireframes/taypeer.fig")
    parser.add_argument("--output", type=Path, default=ROOT / "artifacts/component-checks")
    parser.add_argument("--pixel-tolerance", type=int, default=0, choices=range(256), metavar="0..255")
    parser.add_argument("--max-changed-percent", type=bounded_float,
                        help="fail if changed pixels exceed this percentage; default is informational")
    args = parser.parse_args()
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    if catalog["schema_version"] != 1:
        raise ValueError("unsupported component catalog version")
    cases = catalog["cases"]
    if args.list:
        for case in cases:
            print(f"{case['id']}: {case['design']['node']}")
        return 0
    selected = set(args.case or [c["id"] for c in cases])
    if selected - {c["id"] for c in cases}:
        parser.error("unknown case ID; use --list")
    cases = [c for c in cases if c["id"] in selected]
    if sys.platform != "darwin":
        raise RuntimeError("component screenshots require the macOS Metal headless renderer")
    version = command(["openpencil", "--version"])
    if version != OPENPENCIL_VERSION:
        raise RuntimeError(f"expected OpenPencil CLI {OPENPENCIL_VERSION}, found {version}")
    fig = args.fig.resolve(strict=True)
    output = args.output.resolve()
    designs = resolve_designs(fig, cases)
    report = {
        "schema_version": 1,
        "fig": str(fig),
        "fig_sha256": hashlib.sha256(fig.read_bytes()).hexdigest(),
        "catalog_sha256": hashlib.sha256(CATALOG.read_bytes()).hexdigest(),
        "openpencil_version": version,
        "platform": sys.platform,
        "gpui_font": "bundled Inter 4.1",
        "reference_font": "Inter resolved by OpenPencil CLI",
        "max_changed_percent": args.max_changed_percent,
        "cases": [],
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    # Publish only after every render/export/measurement succeeds. A failed run
    # leaves the preceding report intact; the JSON lists only this run's cases.
    with tempfile.TemporaryDirectory(prefix="component-checks-", dir=output.parent) as staging:
        work = Path(staging)
        render_args = ["cargo", "test", "-p", "taypeer-ui", "--features", "component-rendering",
                       "--test", "components", "--locked", "--", "--output", str(work / "rendered")]
        for case in cases:
            render_args.extend(["--case", case["id"]])
        # Render once; the Rust catalog validation and capture assertions also run.
        print(command(render_args))
        captures = json.loads((work / "rendered/captures.json").read_text())
        for case, design in zip(cases, designs, strict=True):
            capture = next(c for c in captures if c["id"] == case["id"])
            directory = work / case["id"]
            directory.mkdir()
            shutil.copyfile(work / "rendered" / capture["file"], directory / "actual.png")
            command(["openpencil", "export", str(fig), "--node", design["id"],
                     "--scale", str(capture["scale"]), "--output", str(directory / "design.png")])
            metrics = save_comparison(directory, directory / "actual.png", directory / "design.png",
                                      design, capture, args.pixel_tolerance)
            if (metrics["width"], metrics["height"]) != (capture["width"], capture["height"]):
                raise ValueError("capture metadata differs from actual PNG dimensions")
            status = "informational" if args.max_changed_percent is None else (
                "passed" if metrics["changed_percent"] <= args.max_changed_percent else "failed")
            report["cases"].append({"id": case["id"], "design": design, "capture": capture,
                                    "metrics": metrics, "status": status})
        shutil.rmtree(work / "rendered")
        (work / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        shutil.copytree(work, output, dirs_exist_ok=True)
    for item in report["cases"]:
        print(f"{item['id']}: {item['metrics']['changed_percent']:.2f}% changed ({item['status']})")
        print(f"Comparison: {output / item['id'] / 'comparison.png'}")
    print(f"Report: {output / 'report.json'}")
    return int(any(c["status"] == "failed" for c in report["cases"]))


if __name__ == "__main__":
    try:
        sys.exit(run())
    except (OSError, ValueError, RuntimeError, StopIteration) as error:
        print(f"component comparison: {error}", file=sys.stderr)
        sys.exit(2)
