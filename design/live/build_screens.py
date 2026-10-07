#!/usr/bin/env python3
"""Regenerate the station icon masks used by the live training screen.

``apps/hub-server/static/training.html`` is the canonical page source.  The
generator deliberately updates only the marked CSS block containing the icon
masks and the review sheet; it never reconstructs or replaces the page.  This
keeps hand-maintained UI changes, including training-specific actions, intact.
"""

from __future__ import annotations

import argparse
import base64
import io
import re
import sys
from pathlib import Path

import numpy as np
from PIL import Image


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_ICONS = Path(__file__).resolve().parent / "icons"
DEFAULT_TRAINING = ROOT / "apps/hub-server/static/training.html"
DEFAULT_ICON_SHEET = Path(__file__).resolve().parent / "icon-sheet.html"

MASK_SIDE = 256
MASK_MARGIN = 0.06
EXPECTED_ICONS = {
    "burpee_broad_jump",
    "farmers_carry",
    "rowing",
    "sandbag_lunges",
    "skierg",
    "sled_pull",
    "sled_push",
    "wall_balls",
}
MARKER_START = "        /* BEGIN GENERATED STATION ICON MASKS. DO NOT EDIT. */"
MARKER_END = "        /* END GENERATED STATION ICON MASKS. DO NOT EDIT. */"


def _mask(path: Path) -> str:
    """Return the supplied glyph as a white-on-transparent PNG payload."""

    pixels = np.asarray(Image.open(path).convert("RGB")).astype(float)
    alpha = np.clip((np.maximum(pixels[:, :, 0], pixels[:, :, 1]) - 30.0) / 205.0, 0, 1)
    height, width = alpha.shape
    inner = int(MASK_SIDE * (1 - 2 * MASK_MARGIN))
    scale = min(inner / width, inner / height)
    resized_width = max(1, int(round(width * scale)))
    resized_height = max(1, int(round(height * scale)))
    layer = Image.fromarray((alpha * 255).astype("uint8"), "L").resize(
        (resized_width, resized_height), Image.LANCZOS
    )
    canvas = Image.new("L", (MASK_SIDE, MASK_SIDE), 0)
    canvas.paste(layer, ((MASK_SIDE - resized_width) // 2, (MASK_SIDE - resized_height) // 2))
    output = Image.merge("RGBA", (Image.new("L", (MASK_SIDE, MASK_SIDE), 255),) * 3 + (canvas,))
    buffer = io.BytesIO()
    output.save(buffer, "PNG", optimize=True)
    return base64.b64encode(buffer.getvalue()).decode("ascii")


def load_masks(icon_dir: Path = DEFAULT_ICONS) -> dict[str, str]:
    masks = {path.stem: _mask(path) for path in sorted(icon_dir.glob("*.png"))}
    if set(masks) != EXPECTED_ICONS:
        missing = sorted(EXPECTED_ICONS - set(masks))
        unexpected = sorted(set(masks) - EXPECTED_ICONS)
        raise ValueError(f"station icon set changed; missing={missing}, unexpected={unexpected}")
    return masks


def _mask_rule(name: str, payload: str) -> str:
    return (
        f"        .pg-{name}{{-webkit-mask-image:url(data:image/png;base64,{payload});"
        f"mask-image:url(data:image/png;base64,{payload});}}"
    )


def generated_mask_block(masks: dict[str, str]) -> str:
    rules = "\n".join(_mask_rule(name, masks[name]) for name in sorted(masks))
    return f"{MARKER_START}\n{rules}\n{MARKER_END}"


def replace_generated_block(page: str, masks: dict[str, str]) -> str:
    pattern = re.compile(
        rf"^{re.escape(MARKER_START)}$.*?^{re.escape(MARKER_END)}$",
        flags=re.MULTILINE | re.DOTALL,
    )
    matches = pattern.findall(page)
    if len(matches) != 1:
        raise ValueError(
            "training.html must contain exactly one generated station icon block "
            f"(found {len(matches)})"
        )
    return pattern.sub(generated_mask_block(masks), page, count=1)


def icon_sheet(masks: dict[str, str]) -> str:
    css = "\n".join(
        [
            ".pg{width:1em;height:1em;display:inline-block;background-color:currentColor;",
            "-webkit-mask-repeat:no-repeat;mask-repeat:no-repeat;-webkit-mask-size:contain;mask-size:contain;",
            "-webkit-mask-position:center;mask-position:center;}",
        ]
        + [_mask_rule(name, masks[name]).strip() for name in sorted(masks)]
    )
    cells = "".join(
        f'<div class="cell"><span class="ic pg pg-{name}"></span>'
        f'<div class="lbl">{index}. {name.replace("_", " ").upper()}</div></div>'
        for index, name in enumerate(sorted(masks), 1)
    )
    return (
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>HYROX station pictograms</title>"
        f"<style>body{{margin:0;background:#0A0A0A;color:#fff;font-family:system-ui,sans-serif;"
        f"width:800px;}}.grid{{display:grid;grid-template-columns:repeat(4,1fr);gap:5px;padding:8px;}}"
        f".cell{{background:#131314;border:1px solid #343535;padding:8px;text-align:center;}}"
        f".ic{{display:block;font-size:170px;color:#FEE400;}}{css}"
        f".lbl{{margin-top:5px;font-size:13px;letter-spacing:.1em;color:#9c9c9c;}}</style>"
        f"</head><body><div class=\"grid\">{cells}</div></body></html>"
    )


def _check(path: Path, expected: str, label: str) -> None:
    if not path.exists():
        raise ValueError(f"{label} is missing: {path}")
    actual = path.read_text(encoding="utf-8")
    if actual != expected:
        raise ValueError(f"{label} is stale; run build_screens.py to regenerate it: {path}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if the tracked training icon block is stale")
    parser.add_argument(
        "--check-icon-sheet",
        action="store_true",
        help="also check the optional generated review sheet (only with --check)",
    )
    parser.add_argument("--icons", type=Path, default=DEFAULT_ICONS)
    parser.add_argument("--training", type=Path, default=DEFAULT_TRAINING)
    parser.add_argument("--icon-sheet", type=Path, default=DEFAULT_ICON_SHEET)
    args = parser.parse_args(argv)

    masks = load_masks(args.icons)
    page = args.training.read_text(encoding="utf-8")
    expected_page = replace_generated_block(page, masks)
    expected_sheet = icon_sheet(masks)

    try:
        if args.check:
            _check(args.training, expected_page, "training.html icon block")
            if args.check_icon_sheet:
                _check(args.icon_sheet, expected_sheet, "icon-sheet.html")
        else:
            if page != expected_page:
                args.training.write_text(expected_page, encoding="utf-8")
            if not args.icon_sheet.exists() or args.icon_sheet.read_text(encoding="utf-8") != expected_sheet:
                args.icon_sheet.write_text(expected_sheet, encoding="utf-8")
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1

    action = "checked" if args.check else "updated"
    print(f"{action} station icon masks ({len(masks)} icons); page source preserved")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
