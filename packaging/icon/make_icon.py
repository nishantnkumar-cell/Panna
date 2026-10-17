import sys
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

font_path, out = sys.argv[1], sys.argv[2]
f = TTFont(font_path)
gs = f.getGlyphSet(); cmap = f.getBestCmap()
cap = f['OS/2'].sCapHeight
CAP_PX = 11.5
s = CAP_PX / cap
def glyph(ch, x, baseline):
    pen = SVGPathPen(gs)
    tp = TransformPen(pen, (s, 0, 0, -s, x, baseline))
    gs[cmap[ord(ch)]].draw(tp)
    return pen.getCommands(), gs[cmap[ord(ch)]].width * s

paths = []
widths = []
centers = [50, 66, 82]
for ch, c in zip("PDF", centers):
    d, w = glyph(ch, 42, c + CAP_PX / 2)
    paths.append(d); widths.append(w)
line_x = 42 + max(widths) + 4
lens = [84 - line_x, 84 - line_x, (84 - line_x) * 0.5]
lines = "\n".join(f'  <rect x="{line_x:.1f}" y="{c-2.5}" width="{l:.1f}" height="5" rx="2.5" fill="#a7f3d0"/>' for c, l in zip(centers, lens))
letters = "\n".join(f'    <path d="{d}"/>' for d in paths)
svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
  <!-- Panna: emerald app tile, page whose lines start with P, D, F. Letters are Inter glyph outlines (SIL OFL 1.1), so no font is needed at runtime. -->
  <defs>
    <linearGradient id="t" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#10b981"/><stop offset="1" stop-color="#047857"/>
    </linearGradient>
  </defs>
  <rect x="8" y="8" width="112" height="112" rx="26" fill="url(#t)"/>
  <path d="M38 24h38l20 20v56a4 4 0 0 1-4 4H38a4 4 0 0 1-4-4V28a4 4 0 0 1 4-4z" fill="#fff"/>
  <path d="M76 24v16a4 4 0 0 0 4 4h16z" fill="#d1fae5"/>
  <g fill="#047857">
{letters}
  </g>
{lines}
  <g transform="translate(84 90)">
    <path d="M0-12l10 7v10L0 12-10 5V-5z" fill="#059669" stroke="#fff" stroke-width="3" stroke-linejoin="round"/>
    <path d="M0-12l10 7L0 0-10-5z" fill="#34d399"/>
    <path d="M0 0l10 5L0 12z" fill="#065f46"/>
  </g>
</svg>
'''
open(out, "w").write(svg)
