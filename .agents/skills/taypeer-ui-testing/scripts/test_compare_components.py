"""Pixel and layout diagnostics, independent of Metal and OpenPencil."""
import importlib.util
import argparse
from pathlib import Path
import tempfile
import unittest

from PIL import Image

spec = importlib.util.spec_from_file_location("compare_components", Path(__file__).with_name("compare-components.py"))
comparison = importlib.util.module_from_spec(spec)
spec.loader.exec_module(comparison)


class PixelComparisonTests(unittest.TestCase):
    def test_identical_pixels_have_zero_difference(self):
        image = Image.new("RGB", (2, 2), (25, 38, 70))
        diff, metrics = comparison.compare_pixels(image, image, 0)
        self.assertEqual(diff.getbbox(), None)
        self.assertEqual(metrics["changed_percent"], 0)
        self.assertEqual(metrics["mean_absolute_channel_difference"], 0)
        self.assertEqual(metrics["maximum_channel_difference"], 0)

    def test_tolerance_is_per_pixel_maximum_channel(self):
        reference = Image.new("RGB", (2, 1))
        actual = Image.new("RGB", (2, 1))
        actual.putpixel((0, 0), (8, 0, 0))
        actual.putpixel((1, 0), (0, 9, 0))
        _, metrics = comparison.compare_pixels(reference, actual, 8)
        self.assertEqual(metrics["changed_pixels"], 1)
        self.assertEqual(metrics["changed_percent"], 50)
        self.assertAlmostEqual(metrics["mean_absolute_channel_difference"], 17 / 6)
        self.assertEqual(metrics["maximum_channel_difference"], 9)

    def test_dimensions_are_never_resized_to_match(self):
        with self.assertRaisesRegex(ValueError, "no resizing allowed"):
            comparison.compare_pixels(Image.new("RGB", (2, 2)), Image.new("RGB", (3, 2)), 0)

    def test_centered_stroke_is_preserved_on_the_shared_canvas(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            # A 2×2 logical field with a one-pixel external stroke, rendered at 2×.
            raw = Image.new("RGBA", (8, 8), "white")
            raw.paste(Image.new("RGBA", (4, 4)), (2, 2))
            actual = Image.new("RGBA", (12, 12), "black")
            actual.alpha_composite(raw, (2, 2))
            actual.save(directory / "actual.png")
            raw.save(directory / "design.png")
            metrics = comparison.save_comparison(
                directory, directory / "actual.png", directory / "design.png",
                {"width": 2, "height": 2, "outset": 1, "background": [0, 0, 0]},
                {"scale": 2, "padding": 2}, 0,
            )
            self.assertEqual(metrics["changed_percent"], 0)
            with Image.open(directory / "reference.png") as reference:
                self.assertEqual(reference.getpixel((2, 2)), (255, 255, 255, 255))
                self.assertEqual(reference.size, actual.size)

    def test_nonfinite_threshold_is_rejected(self):
        for value in ("nan", "inf", "-1", "101"):
            with self.assertRaises(argparse.ArgumentTypeError):
                comparison.bounded_float(value)


class LayoutComparisonTests(unittest.TestCase):
    @staticmethod
    def anchors():
        return {
            "component": {"x": 0, "y": 0, "width": 614, "height": 44},
            "label": {"x": 24, "y": 12, "width": 144, "height": 20},
            "field": {"x": 184, "y": 6, "width": 406, "height": 32},
            "text": {"x": 184, "y": 12, "width": 398, "height": 20},
        }

    def test_spacing_and_internal_padding_are_measured_without_pixels(self):
        reference = self.anchors()
        import copy
        actual = copy.deepcopy(reference)
        actual["label"]["x"] += 3
        actual["field"]["x"] += 8
        actual["field"]["width"] -= 12
        actual["text"]["x"] += 18
        actual["component"]["height"] += 4
        result = comparison.compare_layout(reference, actual)
        measured = {m["name"]: m for m in result["measurements"]}
        self.assertEqual(measured["component.content.left"]["delta"], 3)
        self.assertEqual(measured["label.field.gap"]["delta"], 5)
        self.assertEqual(measured["component.content.right"]["delta"], 4)
        self.assertEqual(measured["field.text.left"]["delta"], 10)
        self.assertEqual(measured["component.height"]["delta"], 4)
        self.assertEqual(result["units"], "logical_px")

    def test_additional_and_missing_actions_are_diagnostics(self):
        reference, actual = self.anchors(), self.anchors()
        reference["action.favicon"] = dict(x=558, y=6, width=32, height=32)
        actual["action.clear"] = dict(x=558, y=6, width=32, height=32)
        result = comparison.compare_layout(reference, actual)
        self.assertEqual(result["missing_actual"], ["action.favicon"])
        self.assertEqual(result["extra_actual"], ["action.clear"])

    def test_invalid_or_missing_required_geometry_is_not_silently_accepted(self):
        for role in ("component", "field", "text", "label"):
            actual = self.anchors()
            del actual[role]
            with self.assertRaisesRegex(ValueError, "missing required"):
                comparison.compare_layout(self.anchors(), actual)
        actual = self.anchors()
        actual["field"]["width"] = float("nan")
        with self.assertRaisesRegex(ValueError, "invalid layout anchor"):
            comparison.compare_layout(self.anchors(), actual)

    def test_region_crop_keeps_scope_siblings_and_uses_native_scale(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "design.png"
            scope = Image.new("RGB", (20, 16), "black")
            # Paint belonging to a sibling of the row marker.
            scope.putpixel((5, 9), (255, 0, 0))
            scope.save(path)
            design = {"export_width": 10, "export_height": 8,
                      "region": {"x": 2, "y": 4, "width": 6, "height": 2}}
            comparison.crop_reference(path, design, 2)
            with Image.open(path.with_name("region.png")) as row:
                self.assertEqual(row.size, (12, 4))
                self.assertEqual(row.getpixel((1, 1)), (255, 0, 0))
            design["region"]["x"] = 0.1
            with self.assertRaisesRegex(ValueError, "native export pixels"):
                comparison.crop_reference(path, design, 2)

    def test_different_heights_are_padded_without_resizing_or_hiding_paint(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            actual = Image.new("RGBA", (12, 16), "black")
            actual.putpixel((6, 14), (255, 0, 0, 255))
            actual.save(directory / "actual.png")
            Image.new("RGBA", (4, 4), "black").save(directory / "design.png")
            metrics = comparison.save_comparison(
                directory, directory / "actual.png", directory / "design.png",
                {"width": 2, "height": 2, "outset": 0, "background": [0, 0, 0]},
                {"scale": 2, "padding": 2, "width": 12, "height": 16}, 0,
            )
            self.assertEqual((metrics["width"], metrics["height"]), (12, 16))
            with Image.open(directory / "diff.png") as diff:
                self.assertEqual(diff.getpixel((6, 14)), (255, 0, 0))


if __name__ == "__main__":
    unittest.main()
