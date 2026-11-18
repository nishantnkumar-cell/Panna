#!/bin/sh
# Builds a .deb for Debian, Ubuntu and their derivatives from an already built binary.
#   packaging/linux/make-deb.sh target/x86_64-unknown-linux-gnu/release/panna 0.1.0 [out-dir]
# Needs dpkg-dev (dpkg-deb, dpkg-shlibdeps). The package architecture is the build machine's.
set -eu
ID=io.github.nishantnkumar_cell.Panna
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
VERSION=${2#v}
OUT=$(mkdir -p "${3:-.}" && cd "${3:-.}" && pwd)
ARCH=$(dpkg --print-architecture)

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
R=$WORK/root
install -Dm755 "$BIN" "$R/usr/bin/panna"
install -Dm644 "$HERE/$ID.desktop" "$R/usr/share/applications/$ID.desktop"
install -Dm644 "$HERE/$ID.metainfo.xml" "$R/usr/share/metainfo/$ID.metainfo.xml"
install -Dm644 "$HERE/$ID.svg" "$R/usr/share/icons/hicolor/scalable/apps/$ID.svg"
install -Dm644 "$HERE/$ID.thumbnailer" "$R/usr/share/thumbnailers/$ID.thumbnailer"

# Libraries the binary links to directly. Windowing, keyboard and GPU libraries are opened at
# run time, so they're listed by hand below.
mkdir -p "$WORK/debian"
printf 'Source: panna\n\nPackage: panna\nArchitecture: any\n' > "$WORK/debian/control"
SHLIBS=$(cd "$WORK" && dpkg-shlibdeps -O "$R/usr/bin/panna" | sed -n 's/^shlibs:Depends=//p')

mkdir -p "$R/DEBIAN"
cat > "$R/DEBIAN/control" <<EOF
Package: panna
Version: $VERSION
Architecture: $ARCH
Maintainer: Nishant <281151553+nishantnkumar-cell@users.noreply.github.com>
Installed-Size: $(du -sk "$R/usr" | cut -f1)
Depends: $SHLIBS, libxkbcommon0
Recommends: libwayland-client0, libwayland-cursor0, libxkbcommon-x11-0, libx11-xcb1, libxcursor1, libxi6, libxrandr2, libvulkan1 | libegl1
Section: graphics
Priority: optional
Homepage: https://github.com/nishantnkumar-cell/Panna
Description: Offline, memory-safe PDF viewer and editor
 View, annotate, fill in and edit PDF files without any network access.
 Written in Rust.
EOF

dpkg-deb --root-owner-group -Zxz --build "$R" "$OUT/panna_${VERSION}_${ARCH}.deb"
echo "$OUT/panna_${VERSION}_${ARCH}.deb"
