"""Make the bundled fonts not overlap, so egui's fallback order doesn't matter.

- Inter defines ~745 private-use (U+E000-U+F8FF) alternates that shadow icon glyphs.
- Phosphor maps a-z and a few ASCII codepoints (for ligatures) that shadow text.

Run after replacing either font:  python tools/fix_fonts.py
Needs fontTools (pip install fonttools).
"""

import pathlib
from fontTools.ttLib import TTFont

ROOT = pathlib.Path(__file__).resolve().parent.parent / "crates" / "ui" / "assets"


def is_pua(c: int) -> bool:
    return 0xE000 <= c <= 0xF8FF


def strip(path: pathlib.Path, keep) -> int:
    font = TTFont(path)
    removed = 0
    for table in font["cmap"].tables:
        if not hasattr(table, "cmap") or table.cmap is None:
            continue
        drop = [c for c in table.cmap if not keep(c)]
        for c in drop:
            del table.cmap[c]
        removed += len(drop)
    font.save(path)
    return removed


for f in sorted((ROOT / "fonts").glob("Inter-*.ttf")):
    print(f.name, "removed", strip(f, lambda c: not is_pua(c)), "private-use entries")
print("Phosphor.ttf removed", strip(ROOT / "Phosphor.ttf", is_pua), "non-icon entries")
