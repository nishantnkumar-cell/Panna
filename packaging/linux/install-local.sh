#!/bin/sh
# Installs a release build for the current user without Flatpak, with desktop integration
# (menu entry, icon, "Open with" for PDFs) on KDE Plasma, GNOME and other XDG desktops.
set -eu
ID=io.github.nishantnkumar_cell.Panna
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
PREFIX=${PREFIX:-$HOME/.local}

cargo build --release --locked -p panna --manifest-path "$ROOT/Cargo.toml"
install -Dm755 "$ROOT/target/release/panna" "$PREFIX/bin/panna"
install -Dm644 "$ROOT/packaging/linux/$ID.desktop" "$PREFIX/share/applications/$ID.desktop"
install -Dm644 "$ROOT/packaging/linux/$ID.metainfo.xml" "$PREFIX/share/metainfo/$ID.metainfo.xml"
install -Dm644 "$ROOT/packaging/linux/$ID.svg" "$PREFIX/share/icons/hicolor/scalable/apps/$ID.svg"
# File manager thumbnails of PDFs, drawn by Panna's locked-down renderer. GNOME runs
# thumbnailers in a sandbox that only sees system folders, so it uses this one when
# PREFIX is /usr or /usr/local.
install -Dm644 "$ROOT/packaging/linux/$ID.thumbnailer" "$PREFIX/share/thumbnailers/$ID.thumbnailer"

command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$PREFIX/share/icons/hicolor" || true
command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
echo "Installed to $PREFIX. Make sure $PREFIX/bin is on your PATH."
