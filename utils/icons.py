#!/usr/bin/env python3
"""Builds crates/nestlocal/src/ui/icons.deflate: the icons the Nest's home screen
cards can draw, by Home Assistant's names for them.

Home Assistant gives each entity an icon name (its `icon` attribute), such as
mdi:lightbulb-group, or m3o:table-lamp from the Material Symbols integration.
Only the name reaches the thermostat, so it carries the drawings: a subset of
Material Design Icons and of Google's Material Symbols (both Apache-2.0),
chosen by whole words of their names, for what a wall thermostat is likely to show.

    npm pack @mdi/js @material-symbols/svg-400   # into a scratch folder, unpacked
    utils/icons.py MDI_PACKAGE_DIR SYMBOLS_PACKAGE_DIR

It holds lines, raw-deflated (nestlocal inflates them when it starts its UI): "<name>\t<box>\t<SVG path>", box the path's square
viewBox: 24 (0 0 24 24) or 960 (0 -960 960 960).
"""
import os
import re
import sys
import zlib

WORDS = set("""
light lights lightbulb lamp lamps bulb ceiling chandelier led strip spotlight sconce
fan speaker television tv music play pause volume cast soundbar radio
thermostat radiator heat fire snowflake air hvac boiler
sofa bed home house power toggle switch outlet plug socket
door doorbell bell cctv camera garage gate lock
water coffee kettle blinds curtains window robot vacuum washing dishwasher
""".split())

# The cards' own: Nest settings and the lists' Back.
NAMES = {"cog", "cog-outline", "arrow-left", "thermometer", "play", "pause"}

# Home Assistant's Material Symbols sets, by its prefixes.
SETS = {"outlined": "m3o", "rounded": "m3r", "sharp": "m3s"}

OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "nestlocal", "src", "ui", "icons.deflate")


def wanted(name):
    """Named for one of WORDS as a whole word: table-lamp, not highlight."""
    return name in NAMES or any(word in WORDS for word in name.split("-"))


def kebab(camel):
    """mdiLightbulbGroup2 -> lightbulb-group-2"""
    words = re.findall(r"[A-Z][a-z]*|[0-9]+[a-z]*", camel)
    return "-".join(w.lower() for w in words)


def main(mdi_dir, symbols_dir):
    lines = []
    source = open(os.path.join(mdi_dir, "mdi.js")).read()
    for camel, path in re.findall(r'export var mdi([A-Za-z0-9]+) = "([^"]+)"', source):
        name = kebab(camel)
        if wanted(name):
            lines.append(f"mdi:{name}\t24\t{path}")
    for folder, prefix in SETS.items():
        directory = os.path.join(symbols_dir, folder)
        for file in sorted(os.listdir(directory)):
            stem, fill = file[:-4], ""
            if stem.endswith("-fill"):
                stem, fill = stem[:-5], "f"
            name = stem.replace("_", "-")
            if not wanted(name):
                continue
            svg = open(os.path.join(directory, file)).read()
            paths = re.findall(r'<path d="([^"]+)"', svg)
            if len(paths) == 1:
                lines.append(f"{prefix}{fill}:{name}\t960\t{paths[0]}")
    lines.sort()
    deflate = zlib.compressobj(9, zlib.DEFLATED, -15)
    with open(OUT, "wb") as out:
        out.write(deflate.compress(("\n".join(lines) + "\n").encode()) + deflate.flush())
    print(f"{len(lines)} icons, {os.path.getsize(OUT)} bytes: {os.path.normpath(OUT)}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(os.path.join(sys.argv[1]), sys.argv[2])
