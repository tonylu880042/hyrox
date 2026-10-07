#!/usr/bin/env python3
"""Regression tests for the icon-only live-screen generator."""

from __future__ import annotations

import re
import shutil
import sys
import tempfile
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import build_screens  # noqa: E402


class BuildScreensTest(unittest.TestCase):
    def test_regeneration_preserves_the_canonical_page(self) -> None:
        source = build_screens.DEFAULT_TRAINING.read_text(encoding="utf-8")
        masks = build_screens.load_masks()
        regenerated = build_screens.replace_generated_block(source, masks)

        marker_pattern = re.compile(
            rf"^{re.escape(build_screens.MARKER_START)}$.*?^{re.escape(build_screens.MARKER_END)}$",
            flags=re.MULTILINE | re.DOTALL,
        )
        self.assertEqual(marker_pattern.sub("ICON_BLOCK", source), marker_pattern.sub("ICON_BLOCK", regenerated))
        self.assertIn('id="training-overview"', regenerated)
        self.assertIn("next_plan", regenerated)

    def test_cli_regeneration_in_a_temporary_directory_keeps_training_features(self) -> None:
        with tempfile.TemporaryDirectory(prefix="hyrox-screen-generator-") as directory:
            temporary = Path(directory)
            training = temporary / "training.html"
            sheet = temporary / "icon-sheet.html"
            shutil.copy2(build_screens.DEFAULT_TRAINING, training)

            self.assertEqual(
                build_screens.main(
                    [
                        "--icons",
                        str(build_screens.DEFAULT_ICONS),
                        "--training",
                        str(training),
                        "--icon-sheet",
                        str(sheet),
                    ]
                ),
                0,
            )
            regenerated = training.read_text(encoding="utf-8")
            self.assertIn('id="training-overview"', regenerated)
            self.assertIn("next_plan", regenerated)
            self.assertIn("function trainingSummary", regenerated)

            self.assertEqual(
                build_screens.main(
                    [
                        "--check",
                        "--icons",
                        str(build_screens.DEFAULT_ICONS),
                        "--training",
                        str(training),
                        "--icon-sheet",
                        str(sheet),
                        "--check-icon-sheet",
                    ]
                ),
                0,
            )


if __name__ == "__main__":
    unittest.main()
