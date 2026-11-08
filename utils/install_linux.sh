#!/bin/sh
# Install Fledge into the user's desktop: the binary, the icon, and a
# .desktop entry. This is what gives the window an icon under Wayland (and a
# launcher entry everywhere) -- the app_id dev.fledge.app maps to this entry.
#
#     sh utils/install_linux.sh
#
# Build the binary first: cargo build --release -p fledge-gui
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_dir"

bin=target/release/fledge
[ -x "$bin" ] || { echo "build first: cargo build --release -p fledge-gui"; exit 1; }

share="${XDG_DATA_HOME:-$HOME/.local/share}"
bindir="$HOME/.local/bin"
icons="$share/icons/hicolor/256x256/apps"
apps="$share/applications"
mkdir -p "$bindir" "$icons" "$apps"

install -m755 "$bin" "$bindir/fledge"
install -m644 assets/icon/fledge.png "$icons/dev.fledge.app.png"
sed "s|^Exec=.*|Exec=$bindir/fledge|" assets/fledge.desktop > "$apps/dev.fledge.app.desktop"

command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -f "$share/icons/hicolor" 2>/dev/null || true
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" 2>/dev/null || true

echo "installed:"
echo "  $bindir/fledge"
echo "  $icons/dev.fledge.app.png"
echo "  $apps/dev.fledge.app.desktop"
echo "ensure $bindir is on your PATH; the icon shows after the compositor rescans (re-login if needed)."
