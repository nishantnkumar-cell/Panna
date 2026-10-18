"""Rasterises the SVG icon into the Windows .ico, the macOS .icns and the window icon.

Needs rsvg-convert (librsvg) and Pillow. Run from the repository root.
"""
import subprocess
import tempfile
from pathlib import Path

from PIL import Image

SVG = "packaging/linux/io.github.nishantnkumar_cell.Panna.svg"
SIZES = [16, 24, 32, 48, 64, 128, 256, 512, 1024]

with tempfile.TemporaryDirectory() as tmp:
    imgs = {}
    for n in SIZES:
        png = Path(tmp) / f"{n}.png"
        subprocess.run(["rsvg-convert", "-w", str(n), "-h", str(n), SVG, "-o", str(png)], check=True)
        imgs[n] = Image.open(png).convert("RGBA")
        imgs[n].load()

ico = [16, 24, 32, 48, 64, 128, 256]
imgs[256].save("packaging/windows/panna.ico", sizes=[(n, n) for n in ico], append_images=[imgs[n] for n in ico[:-1]])
imgs[1024].save("packaging/macos/Panna.icns", append_images=[imgs[n] for n in [16, 32, 64, 128, 256, 512]])
Path("crates/panna/assets/icon-64.rgba").write_bytes(imgs[64].tobytes())
