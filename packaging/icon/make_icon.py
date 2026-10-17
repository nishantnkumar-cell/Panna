import sys, math
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

font_path, out = sys.argv[1], sys.argv[2]
f = TTFont(font_path)
gs = f.getGlyphSet(); cmap = f.getBestCmap()
CAP_PX = 11.5
s = CAP_PX / f['OS/2'].sCapHeight
def glyph(ch, x, baseline):
    pen = SVGPathPen(gs)
    gs[cmap[ord(ch)]].draw(TransformPen(pen, (s, 0, 0, -s, x, baseline)))
    return pen.getCommands(), gs[cmap[ord(ch)]].width * s

# Superellipse ("squircle") tile, matching the Chitra icon's shape.
def squircle(cx, cy, r, n=5, steps=96):
    pts = []
    for i in range(steps):
        t = 2 * math.pi * i / steps
        c, si = math.cos(t), math.sin(t)
        x = cx + r * math.copysign(abs(c) ** (2 / n), c)
        y = cy + r * math.copysign(abs(si) ** (2 / n), si)
        pts.append(f"{x:.2f} {y:.2f}")
    return "M" + " L".join(pts) + "Z"

centers = [50, 66, 82]
paths, widths = [], []
for ch, c in zip("PDF", centers):
    d, w = glyph(ch, 42, c + CAP_PX / 2)
    paths.append(d); widths.append(w)
line_x = 42 + max(widths) + 4
lens = [84 - line_x, 84 - line_x, (84 - line_x) * 0.5]
lines = "\n".join(f'  <rect x="{line_x:.1f}" y="{c-2.5}" width="{l:.1f}" height="5" rx="2.5" fill="url(#line)"/>' for c, l in zip(centers, lens))
letters = "\n".join(f'    <path d="{d}"/>' for d in paths)
hexagon = " ".join(f"{11*math.cos(math.radians(30+60*k)):.2f},{11*math.sin(math.radians(30+60*k)):.2f}" for k in range(6))
svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
  <!-- Panna. Palette matches the Chitra icon: indigo-to-plum squircle, coral-to-peach accents,
       dark hexagon "gem". Letters are Inter ExtraBold outlines (SIL OFL 1.1). -->
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#2b1a56"/><stop offset="1" stop-color="#5b1c58"/>
    </linearGradient>
    <linearGradient id="ink" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#d15c6a"/><stop offset="1" stop-color="#f69360"/>
    </linearGradient>
    <linearGradient id="line" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="#f6b9a6"/><stop offset="1" stop-color="#f2c9d4"/>
    </linearGradient>
  </defs>
  <path d="{squircle(64, 64, 58)}" fill="url(#tile)"/>
  <path d="M38 24h38l20 20v56a4 4 0 0 1-4 4H38a4 4 0 0 1-4-4V28a4 4 0 0 1 4-4z" fill="#fff8f3"/>
  <path d="M76 24v16a4 4 0 0 0 4 4h16z" fill="#f69360"/>
  <g fill="url(#ink)">
{letters}
  </g>
{lines}
  <g transform="translate(84 90)">
    <polygon points="{hexagon}" fill="#1f0c2e" stroke="#f69360" stroke-width="2.5" stroke-linejoin="round"/>
    <circle cx="-3" cy="-3" r="2.4" fill="#fde7c8"/>
    <circle cx="3.2" cy="2.6" r="1.1" fill="#d15c6a"/>
  </g>
</svg>
'''
open(out, "w").write(svg)
