"""Writes the encrypted test fixtures: one page of text and one text field, user password
"secret", owner password "owner".

- encrypted-rc4.pdf: standard security handler revision 2 (40-bit RC4)
- encrypted-aes128.pdf: revision 4 with the AESV2 crypt filter
- encrypted-aes256.pdf: revision 6 with the AESV3 crypt filter

RC4 and the key derivations use the standard library; AES uses the `cryptography` package.
Run with: python make_encrypted.py
"""

import hashlib
import pathlib
import struct

from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

PAD = bytes.fromhex(
    "28BF4E5E4E758A4164004E56FFFA01082E2E00B6D0683E802F0CA9FE6453697A"
)
USER, OWNER = b"secret", b"owner"
PERMS = -4  # all permissions
FILE_ID = bytes(range(16))
# Fixed so the fixtures are reproducible; real files use random IVs and salts.
IV = bytes(range(100, 116))


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


def aes_cbc(key, iv, data, pad=True):
    if pad:
        n = 16 - len(data) % 16
        data += bytes([n]) * n
    enc = Cipher(algorithms.AES(key), modes.CBC(iv)).encryptor()
    return enc.update(data) + enc.finalize()


def padded(pw):
    return (pw + PAD)[:32]


def legacy(revision, length):
    """O, U and the file key for revisions 2 to 4."""
    n = length // 8
    digest = hashlib.md5(padded(OWNER)).digest()
    if revision >= 3:
        for _ in range(50):
            digest = hashlib.md5(digest[:n]).digest()
    owner_key = digest[:n]
    o = rc4(owner_key, padded(USER))
    if revision >= 3:
        for i in range(1, 20):
            o = rc4(bytes(b ^ i for b in owner_key), o)
    digest = hashlib.md5(padded(USER) + o + struct.pack("<i", PERMS) + FILE_ID).digest()
    if revision >= 3:
        for _ in range(50):
            digest = hashlib.md5(digest[:n]).digest()
    key = digest[:n]
    if revision == 2:
        u = rc4(key, PAD)
    else:
        u = rc4(key, hashlib.md5(PAD + FILE_ID).digest())
        for i in range(1, 20):
            u = rc4(bytes(b ^ i for b in key), u)
        u += bytes(16)
    return o, u, key


def hash_r6(password, salt, udata=b""):
    """Algorithm 2.B."""
    k = hashlib.sha256(password + salt + udata).digest()
    rounds = 0
    while True:
        k1 = (password + k + udata) * 64
        e = aes_cbc(k[:16], k[16:32], k1, pad=False)
        k = [hashlib.sha256, hashlib.sha384, hashlib.sha512][sum(e[:16]) % 3](e).digest()
        rounds += 1
        if rounds >= 64 and e[-1] <= rounds - 32:
            return k[:32]


def write(name, version, revision, encrypt_entries, seal):
    content = b"BT /F1 24 Tf 72 700 Td (Hello secret world) Tj ET"
    objects = {
        1: b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [7 0 R] /DA "
        + seal(1, b"/Helv 12 Tf 0 g")
        + b" /DR << /Font << /Helv 5 0 R >> >> >> >>",
        2: b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R "
        b"/Resources << /Font << /F1 5 0 R >> >> /Annots [7 0 R] >>",
        4: None,
        5: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        6: b"<< /Filter /Standard /V %d /R %d /P %d " % (version, revision, PERMS)
        + encrypt_entries
        + b" >>",
        7: b"<< /Type /Annot /Subtype /Widget /FT /Tx /T "
        + seal(7, b"name")
        + b" /Rect [72 600 300 630] /P 3 0 R /F 4 /DA "
        + seal(7, b"/Helv 12 Tf 0 g")
        + b" >>",
    }
    data = seal(4, content, raw=True)
    objects[4] = b"<< /Length %d >>\nstream\n" % len(data) + data + b"\nendstream"

    out = bytearray(b"%PDF-1.7\n")
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
        + FILE_ID.hex().encode()
        + b"> <"
        + FILE_ID.hex().encode()
        + b">] >>\nstartxref\n%d\n%%%%EOF\n" % xref
    )
    pathlib.Path(__file__).with_name(name).write_bytes(out)


def hexstr(b):
    return b"<" + b.hex().encode() + b">"


def sealer(encrypt):
    def seal(num, data, raw=False):
        sealed = encrypt(num, data)
        return sealed if raw else hexstr(sealed)

    return seal


def obj_key(key, num, salt=b""):
    digest = hashlib.md5(key + num.to_bytes(3, "little") + bytes(2) + salt).digest()
    return digest[: min(len(key) + 5, 16)]


# Revision 2, 40-bit RC4.
o, u, key = legacy(2, 40)
write(
    "encrypted-rc4.pdf",
    1,
    2,
    b"/O " + hexstr(o) + b" /U " + hexstr(u),
    sealer(lambda num, data: rc4(obj_key(key, num), data)),
)

# Revision 4, AES-128.
o, u, key = legacy(4, 128)
write(
    "encrypted-aes128.pdf",
    4,
    4,
    b"/Length 128 /O " + hexstr(o) + b" /U " + hexstr(u)
    + b" /CF << /StdCF << /CFM /AESV2 /Length 16 /AuthEvent /DocOpen >> >> /StmF /StdCF /StrF /StdCF",
    sealer(lambda num, data: IV + aes_cbc(obj_key(key, num, b"sAlT"), IV, data)),
)

# Revision 6, AES-256.
key = bytes(range(200, 232))
uvs, uks, ovs, oks = b"uvsalt!!", b"ukssalt!", b"ovsalt!!", b"okssalt!"
u = hash_r6(USER, uvs) + uvs + uks
ue = aes_cbc(hash_r6(USER, uks), bytes(16), key, pad=False)
o = hash_r6(OWNER, ovs, u) + ovs + oks
oe = aes_cbc(hash_r6(OWNER, oks, u), bytes(16), key, pad=False)
perms_plain = struct.pack("<i", PERMS) + b"\xff\xff\xff\xffTadb" + b"0000"
enc = Cipher(algorithms.AES(key), modes.ECB()).encryptor()
perms = enc.update(perms_plain) + enc.finalize()
write(
    "encrypted-aes256.pdf",
    5,
    6,
    b"/Length 256 /O " + hexstr(o) + b" /U " + hexstr(u) + b" /OE " + hexstr(oe)
    + b" /UE " + hexstr(ue) + b" /Perms " + hexstr(perms)
    + b" /CF << /StdCF << /CFM /AESV3 /Length 32 /AuthEvent /DocOpen >> >> /StmF /StdCF /StrF /StdCF",
    sealer(lambda num, data: IV + aes_cbc(key, IV, data)),
)
