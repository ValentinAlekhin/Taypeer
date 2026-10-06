#!/usr/bin/env python3
"""Compare isolated product components with FIG paint and logical layout geometry."""
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
    from PIL import Image, ImageChops, ImageDraw, ImageFont
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
    const unique = (nodes, context) => {
      if (nodes.length !== 1) throw new Error(context + ': expected one design node');
      return nodes[0];
    };
    const rect = (n, origin) => {
      const b = n.absoluteBoundingBox;
      if (!b || ![b.x,b.y,b.width,b.height].every(Number.isFinite) || b.width<=0 || b.height<=0)
        throw new Error('invalid design anchor bounds');
      return {x:b.x-origin.x,y:b.y-origin.y,width:b.width,height:b.height};
    };
    return cases.map(c => {
      const pages = figma.root.children.filter(p => p.name === c.design.page);
      if (pages.length !== 1) throw new Error(c.id + ': expected one design page');
      let scope = pages[0];
      for (const name of c.design.ancestors ?? [])
        scope = unique(scope.children.filter(n=>n.name===name), c.id + '/' + name);
      const nodes = scope.findAll(n => n.name === c.design.node);
      if (nodes.length !== 1) throw new Error(c.id + ': expected one design node');
      const n = nodes[0];
      const origin = n.absoluteBoundingBox;
      if (c.kind === 'editor-row') {
        const anchors = {component:rect(n,origin)};
        const resolved = {};
        const fieldTarget = unique(scope.findAll(t=>t.name===c.design.anchors.field), c.id+'/field');
        for (const [role,name] of Object.entries(c.design.anchors)) {
          const targetScope = role === 'text' ? fieldTarget : scope;
          const target = unique(targetScope.findAll(t=>t.name===name), c.id + '/' + role);
          resolved[role] = target;
          anchors[role] = rect(target,origin);
        }
        if (resolved.text.type !== 'TEXT' || resolved.text.characters !== c.text)
          throw new Error(c.id + ': fixture text differs from design');
        const related = [scope,n,...Object.values(resolved)];
        if (related.some(t=>(t.rotation??0)!==0 || (t.effects??[]).length)
            || (scope.strokes??[]).some(s=>s.visible!==false))
          throw new Error(c.id + ': reference needs an axis-aligned effect-free scope');
        const paint = scope.fills.filter(f=>f.visible!==false);
        if (paint.length!==1 || paint[0].type!=='SOLID'
            || (paint[0].opacity??1)!==1 || (paint[0].color.a??1)!==1)
          throw new Error(c.id + ': reference scope must be opaque');
        const background = ['r','g','b'].map(k=>Math.round(paint[0].color[k]*255));
        return {case_id:c.id,id:scope.id,page:pages[0].name,name:n.name,
          width:n.width,height:n.height,outset:0,background,anchors,
          export_width:scope.width,export_height:scope.height,
          region:rect(n,scope.absoluteBoundingBox)};
      }
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
              width:n.width, height:n.height, background, outset,
              anchors:{component:rect(n,origin),field:rect(n,origin),
                text:rect(unique(n.findAll(t=>t.type==='TEXT'),c.id+'/text'),origin)}};
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
    if offset != int(offset) or raw.size != expected:
        raise ValueError("export dimensions differ from declared bounds; no resizing allowed")
    declared = (capture.get("width", actual.width), capture.get("height", actual.height))
    if actual.size != declared:
        raise ValueError("capture metadata differs from actual PNG dimensions")
    reference_size = (math.ceil((design["width"] + 2 * padding) * scale),
                      math.ceil((design["height"] + 2 * padding) * scale))
    canvas_size = (max(actual.width, reference_size[0]), max(actual.height, reference_size[1]))
    reference = Image.new("RGBA", canvas_size, tuple(design["background"]) + (255,))
    reference.alpha_composite(raw, (int(offset), int(offset)))
    if actual.size != canvas_size:
        canvas = Image.new("RGBA", canvas_size, actual.getpixel((0, 0)))
        canvas.alpha_composite(actual)
        actual = canvas
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


def crop_reference(path, design, scale):
    """Crop a declared row region from its painted scope, including sibling nodes."""
    region = design["region"]
    coordinates = [region["x"] * scale, region["y"] * scale,
                   (region["x"] + region["width"]) * scale,
                   (region["y"] + region["height"]) * scale]
    if any(v != int(v) for v in coordinates):
        raise ValueError("reference region must align with native export pixels")
    with Image.open(path) as image:
        expected = (design["export_width"] * scale, design["export_height"] * scale)
        if image.size != expected:
            raise ValueError("scope export dimensions differ; no resizing allowed")
        left, top, right, bottom = map(int, coordinates)
        if not (0 <= left < right <= image.width and 0 <= top < bottom <= image.height):
            raise ValueError("reference region exceeds exported scope")
        image.crop((left, top, right, bottom)).save(path.with_name("region.png"))


def validate_anchors(anchors):
    for role in ("component", "field", "text"):
        if role not in anchors:
            raise ValueError(f"missing required layout anchor: {role}")
    for role, rect in anchors.items():
        if set(rect) != {"x", "y", "width", "height"} or any(
                not isinstance(v, (int, float)) or not math.isfinite(v) for v in rect.values()):
            raise ValueError(f"invalid layout anchor: {role}")
        if rect["width"] <= 0 or rect["height"] <= 0:
            raise ValueError(f"empty layout anchor: {role}")


def layout_values(anchors):
    """Measure boxes and spaces, not glyph ink or inferred CSS declarations."""
    validate_anchors(anchors)
    component, field, text = (anchors[k] for k in ("component", "field", "text"))
    right = lambda r: r["x"] + r["width"]
    bottom = lambda r: r["y"] + r["height"]
    values = {
        "field.text.left": text["x"] - field["x"],
        "field.text.top": text["y"] - field["y"],
        "field.text.right": right(field) - right(text),
        "field.text.bottom": bottom(field) - bottom(text),
        "component.field.top": field["y"] - component["y"],
        "component.field.bottom": bottom(component) - bottom(field),
    }
    if "label" in anchors:
        label = anchors["label"]
        actions = [r for k, r in anchors.items() if k.startswith("action.")]
        values.update({
            "component.content.left": label["x"] - component["x"],
            "component.content.right": right(component) - max(map(right, [field, *actions])),
            "label.field.gap": field["x"] - right(label),
            "label.field.center-offset": label["y"] + label["height"] / 2
                - field["y"] - field["height"] / 2,
        })
        if actions:
            ordered = sorted(actions, key=lambda r: r["x"])
            values["field.actions.gap"] = ordered[0]["x"] - right(field)
            for index, (a, b) in enumerate(zip(ordered, ordered[1:])):
                values[f"actions.gap.{index + 1}"] = b["x"] - right(a)
    for role, rect in sorted(anchors.items()):
        for coordinate, value in rect.items():
            values[f"{role}.{coordinate}"] = value
    return values


def compare_layout(reference, actual):
    a, b = layout_values(reference), layout_values(actual)
    measurements = [{"name": name, "reference": value, "actual": b[name],
                     "delta": b[name] - value} for name, value in a.items() if name in b]
    missing = sorted(set(reference) - set(actual))
    # Missing required semantic anchors cannot silently become visual acceptance.
    if any(not role.startswith("action.") for role in missing):
        raise ValueError("missing required actual layout anchors: " + ", ".join(missing))
    return {"units": "logical_px", "delta_direction": "actual_minus_reference",
            "text_bounds_kind": "layout_area_not_glyph_ink", "measurements": measurements,
            "missing_actual": missing, "extra_actual": sorted(set(actual) - set(reference))}


def save_layout(directory, design, capture, layout):
    font = ImageFont.load_default(size=18)
    small = ImageFont.load_default(size=14)
    with Image.open(directory / "reference.png") as source:
        reference = source.convert("RGB")
    with Image.open(directory / "actual.png") as source:
        actual = source.convert("RGB")
    width = max(820, reference.width, actual.width)
    panel_height = max(reference.height, actual.height) + 45
    rows = [m for m in layout["measurements"] if m["delta"] != 0
            or ".text." in m["name"] or m["name"].startswith("component.content.")
            or m["name"] == "label.field.gap"]
    notices = [f"Extra GPUI element: {r}" for r in layout["extra_actual"]]
    notices += [f"Missing GPUI element: {r}" for r in layout["missing_actual"]]
    roles = sorted(set(design["anchors"]) | set(capture["anchors"]))
    height = panel_height * 2 + 100 + (len(rows) + len(notices) + len(roles)) * 25
    canvas = Image.new("RGB", (width + 32, height), "#f3f3f3")
    draw = ImageDraw.Draw(canvas)
    palette = ["#ee6840", "#21b4dc", "#b68bff", "#53c968", "#f4cc45", "#f096d4"]
    for index, (name, image, anchors) in enumerate([
            ("OpenPencil layout", reference, design["anchors"]),
            ("GPUI layout", actual, capture["anchors"])]):
        top = index * panel_height + 12
        draw.text((16, top), name, font=font, fill="#222222")
        canvas.paste(image, (16, top + 28))
        for number, role in enumerate(roles):
            if role not in anchors:
                continue
            rect = anchors[role]
            scale, padding = capture["scale"], capture["padding"]
            x = 16 + (rect["x"] + padding) * scale
            y = top + 28 + (rect["y"] + padding) * scale
            color = palette[number % len(palette)]
            draw.rectangle((x, y, x + rect["width"] * scale - 1,
                            y + rect["height"] * scale - 1), outline=color, width=2)
            draw.text((x + 2, y + 1), str(number + 1), font=small, fill=color,
                      stroke_width=1, stroke_fill="#000000")
    top = panel_height * 2 + 16
    for number, role in enumerate(roles):
        draw.text((16, top), f"{number + 1}: {role}", font=font,
                  fill=palette[number % len(palette)])
        top += 25
    draw.text((16, top), "Metric (logical px)", font=font, fill="#222222")
    for x, label in [(480, "FIG"), (595, "GPUI"), (715, "Delta")]:
        draw.text((x, top), label, font=font, fill="#222222")
    top += 30
    for m in rows:
        draw.text((16, top), m["name"], font=font, fill="#222222")
        for x, value in [(480, m["reference"]), (595, m["actual"]), (715, m["delta"])]:
            draw.text((x, top), f"{value:+.2f}" if x == 715 else f"{value:.2f}",
                      font=font, fill="#b53b20" if x == 715 and value != 0 else "#222222")
        top += 25
    for notice in notices:
        draw.text((16, top), notice, font=font, fill="#b53b20")
        top += 25
    draw.text((16, top + 10), "Text boxes are layout areas, not glyph ink. Pixel-perfect matching is not required.",
              font=small, fill="#555555")
    canvas.save(directory / "layout.png")


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
    if catalog["schema_version"] != 2:
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
        "schema_version": 2,
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
            reference_path = directory / "design.png"
            if "region" in design:
                crop_reference(reference_path, design, capture["scale"])
                reference_path = directory / "region.png"
            layout = compare_layout(design["anchors"], capture["anchors"])
            metrics = save_comparison(directory, directory / "actual.png", reference_path,
                                      design, capture, args.pixel_tolerance)
            save_layout(directory, design, capture, layout)
            status = "informational" if args.max_changed_percent is None else (
                "passed" if metrics["changed_percent"] <= args.max_changed_percent else "failed")
            report["cases"].append({"id": case["id"], "design": design, "capture": capture,
                                    "metrics": metrics, "layout": layout, "status": status})
        shutil.rmtree(work / "rendered")
        (work / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        shutil.copytree(work, output, dirs_exist_ok=True)
    for item in report["cases"]:
        print(f"{item['id']}: {item['metrics']['changed_percent']:.2f}% changed ({item['status']})")
        differences = [m for m in item["layout"]["measurements"] if m["delta"] != 0]
        for m in differences[:12]:
            print(f"  {m['name']}: FIG {m['reference']:.2f}, GPUI {m['actual']:.2f}, delta {m['delta']:+.2f} logical px")
        for role in item["layout"]["extra_actual"]:
            print(f"  Extra GPUI element: {role}")
        for role in item["layout"]["missing_actual"]:
            print(f"  Missing GPUI element: {role}")
        print(f"Layout: {output / item['id'] / 'layout.png'}")
        print(f"Comparison: {output / item['id'] / 'comparison.png'}")
    print(f"Report: {output / 'report.json'}")
    return int(any(c["status"] == "failed" for c in report["cases"]))


if __name__ == "__main__":
    try:
        sys.exit(run())
    except (OSError, ValueError, RuntimeError, StopIteration) as error:
        print(f"component comparison: {error}", file=sys.stderr)
        sys.exit(2)
