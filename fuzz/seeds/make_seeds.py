#!/usr/bin/env python3
"""Writes the fuzzers' seed inputs: small PDFs that each reach a different part of the
parser and the editing code, a few hostile ones, and comment and form data files.

    python3 fuzz/seeds/make_seeds.py

The output is committed (pdf/ and data/), so running this is only needed after changing it.
Real-world PDFs are added in CI from the pdf.js test suite; see fuzz/README.md.
"""
import pathlib
import zlib

HERE = pathlib.Path(__file__).resolve().parent


def build(objects, trailer_extra="", header=b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n"):
    """A file with a classic cross-reference table. Objects are str or bytes bodies."""
    out = bytearray(header)
    offsets = []
    for i, body in enumerate(objects, 1):
        offsets.append(len(out))
        body = body.encode("latin-1") if isinstance(body, str) else body
        out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    for offset in offsets:
        out += b"%010d 00000 n \n" % offset
    out += b"trailer\n<< /Size %d /Root 1 0 R %s >>\nstartxref\n%d\n%%%%EOF\n" % (
        len(objects) + 1,
        trailer_extra.encode(),
        xref,
    )
    return bytes(out)


def stream(dict_, data, compress=False):
    if isinstance(data, str):
        data = data.encode("latin-1")
    if compress:
        data = zlib.compress(data)
        dict_ += " /Filter /FlateDecode"
    return b"<< %s /Length %d >>\nstream\n" % (dict_.encode(), len(data)) + data + b"\nendstream"


FONT = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
CATALOG = "<< /Type /Catalog /Pages 2 0 R >>"
PAGES = "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"


def page(extra=""):
    return (
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R "
        "/Resources << /Font << /F1 5 0 R >> >> %s >>" % extra
    )


def text_page():
    content = "BT /F1 18 Tf 20 150 Td (Hello, seed) Tj 0 -24 Td [(Kern) -250 (ed)] TJ ET"
    return build([CATALOG, PAGES, page(), stream("", content, True), FONT])


def shapes_and_pictures():
    rgb = bytes(range(48))  # 4 x 4 pixels
    content = (
        "q 1 0 0 rg 20 20 80 60 re f Q q 0 0 1 RG 4 w 120 20 m 200 120 l 260 40 280 180 290 190 c S Q "
        "q 60 0 0 60 200 120 cm /Im1 Do Q "
        "q 20 0 0 20 20 150 cm BI /W 2 /H 2 /CS /G /BPC 8 ID \x00\x7f\x7f\xff EI Q"
    )
    return build(
        [
            CATALOG,
            PAGES,
            page().replace(">> >>", ">> /XObject << /Im1 6 0 R >> >>"),
            stream("", content),
            FONT,
            stream("/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8", rgb, True),
        ]
    )


def form_and_comments():
    return build(
        [
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R 7 0 R] /DA (/Helv 0 Tf 0 g) "
            "/DR << /Font << /Helv 5 0 R >> >> >> >>",
            PAGES,
            page("/Annots [6 0 R 7 0 R 8 0 R 9 0 R 10 0 R 11 0 R]"),
            stream("", "BT /F1 12 Tf 20 170 Td (Name and choice) Tj ET"),
            FONT,
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Pat) /Rect [20 130 180 150] /P 3 0 R /DA (/Helv 12 Tf 0 g) >>",
            "<< /Type /Annot /Subtype /Widget /FT /Btn /T (agree) /V /Yes /AS /Yes /Rect [200 130 214 144] /P 3 0 R "
            "/AP << /N << /Yes 12 0 R /Off 12 0 R >> >> >>",
            "<< /Type /Annot /Subtype /Text /Rect [240 170 260 190] /Contents (A note) /T (Reviewer) /Popup 9 0 R >>",
            "<< /Type /Annot /Subtype /Popup /Rect [260 100 300 190] /Parent 8 0 R >>",
            "<< /Type /Annot /Subtype /Highlight /Rect [20 165 120 185] /QuadPoints [20 185 120 185 20 165 120 165] "
            "/C [1 1 0] /Contents (Marked) >>",
            "<< /Type /Annot /Subtype /Text /Rect [240 140 260 160] /Contents (Agreed) /IRT 8 0 R /State /Accepted /StateModel /Review >>",
            stream("/Type /XObject /Subtype /Form /BBox [0 0 14 14]", "0 0 14 14 re S"),
        ]
    )


def outline_links_and_attachment():
    return build(
        [
            "<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R /Names << /EmbeddedFiles << /Names [(a.txt) 9 0 R] >> >> "
            "/PageLabels << /Nums [0 << /S /r >>] >> /OpenAction [3 0 R /Fit] >>",
            PAGES,
            page("/Annots [10 0 R 11 0 R]"),
            stream("", "BT /F1 12 Tf 20 100 Td (Links) Tj ET"),
            FONT,
            "<< /Type /Outlines /First 7 0 R /Last 8 0 R /Count 2 >>",
            "<< /Title <FEFF00480069> /Parent 6 0 R /Next 8 0 R /Dest [3 0 R /XYZ 0 200 0] >>",
            "<< /Title (Second) /Parent 6 0 R /Prev 7 0 R /A << /S /GoTo /D [3 0 R /Fit] >> >>",
            "<< /Type /Filespec /F (a.txt) /UF (a.txt) /EF << /F 12 0 R >> >>",
            "<< /Type /Annot /Subtype /Link /Rect [20 95 80 115] /A << /S /URI /URI (https://example.com/) >> >>",
            "<< /Type /Annot /Subtype /Link /Rect [100 95 160 115] /Dest [3 0 R /Fit] >>",
            stream("/Type /EmbeddedFile", "attached text"),
        ]
    )


def object_streams():
    """Objects packed in an object stream, found through a cross-reference stream."""
    head = b"%PDF-1.7\n"
    content = stream("", "BT /F1 12 Tf 20 100 Td (Packed) Tj ET")
    packed = [CATALOG, PAGES, page(), FONT]
    nums = [1, 2, 3, 5]
    bodies, offsets, pos = [], [], 0
    for body in packed:
        offsets.append(pos)
        b = body.encode() + b"\n"
        bodies.append(b)
        pos += len(b)
    index = " ".join("%d %d" % (n, o) for n, o in zip(nums, offsets)) + "\n"
    data = index.encode() + b"".join(bodies)
    objstm = stream("/Type /ObjStm /N %d /First %d" % (len(packed), len(index)), data, True)
    out = bytearray(head)
    off4 = len(out)
    out += b"4 0 obj\n" + content + b"\nendobj\n"
    off6 = len(out)
    out += b"6 0 obj\n" + objstm + b"\nendobj\n"
    off7 = len(out)
    rows = [(0, 0, 0xFFFF)]
    for n in range(1, 8):
        if n in nums:
            rows.append((2, 6, nums.index(n)))
        elif n == 4:
            rows.append((1, off4, 0))
        elif n == 6:
            rows.append((1, off6, 0))
        else:
            rows.append((1, off7, 0))
    table = b"".join(bytes([t]) + f.to_bytes(4, "big") + g.to_bytes(2, "big") for t, f, g in rows)
    xref = stream("/Type /XRef /Size 8 /W [1 4 2] /Root 1 0 R", table)
    out += b"7 0 obj\n" + xref + b"\nendobj\nstartxref\n%d\n%%%%EOF\n" % off7
    return bytes(out)


def incremental_update():
    first = text_page()
    # A second revision replacing the page's content.
    content = stream("", "BT /F1 18 Tf 20 150 Td (Changed) Tj ET")
    prev = int(first.rsplit(b"startxref\n", 1)[1].split(b"\n")[0])
    out = bytearray(first)
    off = len(out)
    out += b"4 0 obj\n" + content + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n4 1\n%010d 00000 n \ntrailer\n<< /Size 6 /Root 1 0 R /Prev %d >>\nstartxref\n%d\n%%%%EOF\n" % (off, prev, xref)
    return bytes(out)


def hostile():
    """Inputs that have hung or exhausted naive readers."""
    cycle = build(
        [
            "<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>",
            PAGES,
            page(),
            stream("", "q Q"),
            FONT,
            "<< /First 7 0 R /Count 2 >>",
            "<< /Title (Loop) /Parent 6 0 R /Next 8 0 R /First 8 0 R >>",
            "<< /Title (Back) /Parent 6 0 R /Next 7 0 R /First 7 0 R >>",
        ]
    )
    tree_loop = build(
        [CATALOG, "<< /Type /Pages /Kids [2 0 R 3 0 R] /Count 2 >>", page(), stream("", ""), FONT]
    )
    self_form = build(
        [
            CATALOG,
            PAGES,
            page().replace(">> >>", ">> /XObject << /X 6 0 R >> >>"),
            stream("", "/X Do"),
            FONT,
            stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /X 6 0 R >> >>", "/X Do"),
        ]
    )
    deep = "q " * 5000 + "Q " * 5000
    nesting = build([CATALOG, PAGES, page(), stream("", deep, True), FONT])
    truncated = text_page()[:-60]
    prev_loop = text_page().replace(b"/Root 1 0 R ", b"/Root 1 0 R /Prev 9 ")
    return {
        "outline-cycle.pdf": cycle,
        "page-tree-loop.pdf": tree_loop,
        "form-draws-itself.pdf": self_form,
        "deep-nesting.pdf": nesting,
        "truncated.pdf": truncated,
        "bad-prev.pdf": prev_loop,
    }


XFDF = b"""<?xml version="1.0" encoding="UTF-8"?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <fields><field name="name"><value>Pat</value></field><field name="agree"><value>Yes</value></field></fields>
  <annots>
    <text page="0" rect="240,170,260,190" color="#FFFF00" title="Reviewer" name="n1"><contents>A note</contents></text>
    <highlight page="0" rect="20,165,120,185" coords="20,185,120,185,20,165,120,165" color="#FFFF00"/>
    <ink page="0" rect="10,10,100,100" width="2"><inklist><gesture>10,10;50,80;100,20</gesture></inklist></ink>
    <freetext page="0" rect="20,20,200,60"><contents>Typed</contents></freetext>
  </annots>
</xfdf>
"""

FDF = b"""%FDF-1.2
1 0 obj
<< /FDF << /Fields [<< /T (name) /V (Pat) >> << /T (agree) /V /Yes >>]
/Annots [<< /Type /Annot /Subtype /Square /Page 0 /Rect [10 10 90 60] /C [1 0 0] /Contents (Box) >>] >> >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
"""


def main():
    pdfs = {
        "text.pdf": text_page(),
        "shapes-and-pictures.pdf": shapes_and_pictures(),
        "form-and-comments.pdf": form_and_comments(),
        "outline-links-attachment.pdf": outline_links_and_attachment(),
        "object-streams.pdf": object_streams(),
        "incremental-update.pdf": incremental_update(),
        **hostile(),
    }
    (HERE / "pdf").mkdir(exist_ok=True)
    for name, data in pdfs.items():
        (HERE / "pdf" / name).write_bytes(data)
    (HERE / "data").mkdir(exist_ok=True)
    (HERE / "data" / "comments.xfdf").write_bytes(XFDF)
    (HERE / "data" / "fields.fdf").write_bytes(FDF)
    (HERE / "data" / "fields.csv").write_bytes(b"name,agree\nPat,Yes\n")


if __name__ == "__main__":
    main()
