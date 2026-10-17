"""Writes encrypted-rc4.pdf: one page of text, user password "secret", owner password "owner".

Standard security handler revision 2 (40-bit RC4), built with the standard library only so the
fixture can be regenerated anywhere: python make_encrypted.py
"""

import hashlib
import pathlib
import struct

PAD = bytes.fromhex(
    "28BF4E5E4E758A4164004E56FFFA01082E2E00B6D0683E802F0CA9FE6453697A"
)
USER, OWNER = b"secret", b"owner"
PERMS = -4  # all permissions
FILE_ID = bytes(range(16))


def rc4(key, data):
    s = list(range(256))
    j = 0
    for i in range(256):
        j = (j + s[i] + key[i % len(key)]) % 256
        s[i], s[j] = s[j], s[i]
    out, i, j = bytearray(), 0, 0
    for byte in data:
        i = (i + 1) % 256
        j = (j + s[i]) % 256
        s[i], s[j] = s[j], s[i]
        out.append(byte ^ s[(s[i] + s[j]) % 256])
    return bytes(out)


def padded(pw):
    return (pw + PAD)[:32]


owner_key = hashlib.md5(padded(OWNER)).digest()[:5]
O = rc4(owner_key, padded(USER))
file_key = hashlib.md5(padded(USER) + O + struct.pack("<i", PERMS) + FILE_ID).digest()[:5]
U = rc4(file_key, PAD)


def obj_key(num):
    return hashlib.md5(file_key + num.to_bytes(3, "little") + (0).to_bytes(2, "little")).digest()[:10]


content = b"BT /F1 24 Tf 72 700 Td (Hello secret world) Tj ET"
objects = {
    1: b"<< /Type /Catalog /Pages 2 0 R >>",
    2: b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R "
    b"/Resources << /Font << /F1 5 0 R >> >> >>",
    4: b"<< /Length %d >>\nstream\n" % len(content) + rc4(obj_key(4), content) + b"\nendstream",
    5: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    6: b"<< /Filter /Standard /V 1 /R 2 /O <" + O.hex().encode() + b"> /U <"
    + U.hex().encode() + b"> /P %d >>" % PERMS,
}

out = bytearray(b"%PDF-1.4\n")
offsets = {}
for num, body in objects.items():
    offsets[num] = len(out)
    out += b"%d 0 obj\n" % num + body + b"\nendobj\n"
xref = len(out)
out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
for num in sorted(objects):
    out += b"%010d 00000 n \n" % offsets[num]
out += (
    b"trailer\n<< /Size %d /Root 1 0 R /Encrypt 6 0 R /ID [<" % (len(objects) + 1)
    + FILE_ID.hex().encode() + b"> <" + FILE_ID.hex().encode() + b">] >>\nstartxref\n%d\n%%%%EOF\n" % xref
)
pathlib.Path(__file__).with_name("encrypted-rc4.pdf").write_bytes(out)
