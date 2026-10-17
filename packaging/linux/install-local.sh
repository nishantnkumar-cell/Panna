#!/bin/sh
# Installs a release build for the current user without Flatpak, with desktop integration
# (menu entry, icon, "Open with" for PDFs) on KDE Plasma, GNOME and other XDG desktops.
set -eu
ID=io.github.nishantnkumar_cell.PdfViewer
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
PREFIX=${PREFIX:-$HOME/.local}

cargo build --release --locked -p pdf-app --manifest-path "$ROOT/Cargo.toml"
install -Dm755 "$ROOT/target/release/pdf-viewer" "$PREFIX/bin/pdf-viewer"
install -Dm644 "$ROOT/packaging/linux/$ID.desktop" "$PREFIX/share/applications/$ID.desktop"
install -Dm644 "$ROOT/packaging/linux/$ID.metainfo.xml" "$PREFIX/share/metainfo/$ID.metainfo.xml"
install -Dm644 "$ROOT/packaging/linux/$ID.svg" "$PREFIX/share/icons/hicolor/scalable/apps/$ID.svg"

command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$PREFIX/share/icons/hicolor" || true
command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
echo "Installed to $PREFIX. Make sure $PREFIX/bin is on your PATH."
