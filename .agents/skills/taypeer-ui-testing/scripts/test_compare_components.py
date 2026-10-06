"""Meaningful pixel comparison checks, independent of Metal and OpenPencil."""
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


if __name__ == "__main__":
    unittest.main()
