#!/bin/sh
# Assembles the homepage: site/ with the pictures, icon and fonts it shows
# from assets/, into DIR (default dist/site). The Site workflow publishes
# that to GitHub Pages; open DIR/index.html to look at it locally.
#
#     utils/build-site.sh [DIR]
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
out=${1:-dist/site}
rm -rf "$out"
mkdir -p "$out"
cp site/index.html site/showcase.mp4 site/poster.jpg "$out/"
cp assets/icon/fledge.svg assets/screens/cards.png assets/screens/fledge.png assets/screens/app.png "$out/"
cp assets/fonts/DMSans-Regular.ttf assets/fonts/DMSans-Medium.ttf assets/fonts/Outfit-Light.ttf "$out/"
echo "The homepage is in $out"
