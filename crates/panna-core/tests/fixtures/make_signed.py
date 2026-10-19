"""Writes the signed test fixtures: one page with a signature field, signed with a
self-signed certificate for "Test Signer" made on the spot.

- signed-rsa.pdf: RSA 2048 with SHA-256 (adbe.pkcs7.detached)
- signed-ecdsa.pdf: ECDSA P-256 with SHA-256 (ETSI.CAdES.detached)
- signed-then-changed.pdf: signed-rsa.pdf with an incremental update added after signing
- signer-rsa.p12, signer-ec.p12: certificate files for "Pat Example" to sign with, password
  "secret"

The signatures are made by the openssl command-line tool.
Run with: python make_signed.py
"""

import pathlib
import subprocess
import tempfile

HERE = pathlib.Path(__file__).parent
SIG_LEN = 8192  # bytes of room for the signature, written as hex


def pdf(subfilter):
    """The file with a placeholder signature, and where the placeholder's hex starts."""
    contents = b"<" + b"0" * (SIG_LEN * 2) + b">"
    byte_range = b"/ByteRange [0000000000 0000000000 0000000000 0000000000]"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /SigFlags 3 >> >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R] >>",
        b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature1) /Rect [0 0 0 0] /F 132 "
        b"/P 3 0 R /V 5 0 R >>",
        b"<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /" + subfilter + b" "
        + byte_range + b" /Contents " + contents
        + b" /M (D:20261019120000Z) /Name (Test Signer) /Reason (I approve) "
        b"/Location (Leeds) >>",
    ]
    out = bytearray(b"%PDF-1.7\n")
    offsets = []
    for i, body in enumerate(objects):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % (i + 1) + body + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    for o in offsets:
        out += b"%010d 00000 n \n" % o
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objects) + 1, xref)
    start = out.index(b"/Contents <") + len(b"/Contents ")
    end = start + len(contents)
    ranges = b"/ByteRange [%010d %010d %010d %010d]" % (0, start, end, len(out) - end)
    at = out.index(byte_range)
    out[at : at + len(byte_range)] = ranges
    return out, start, end


def sign(subfilter, key_args, name):
    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        key, cert, data, sig = (tmp / n for n in ("key.pem", "cert.pem", "data", "sig.der"))
        subprocess.run(["openssl", "genpkey", *key_args, "-out", str(key)], check=True)
        subprocess.run(
            ["openssl", "req", "-new", "-x509", "-key", str(key), "-out", str(cert),
             "-days", "3650", "-subj", "/CN=Test Signer/O=Panna tests", "-sha256"],
            check=True,
        )
        out, start, end = pdf(subfilter)
        data.write_bytes(bytes(out[:start]) + bytes(out[end:]))
        subprocess.run(
            ["openssl", "cms", "-sign", "-binary", "-in", str(data), "-signer", str(cert),
             "-inkey", str(key), "-outform", "DER", "-out", str(sig), "-md", "sha256",
             "-nosmimecap"],
            check=True,
        )
        hexsig = sig.read_bytes().hex().upper().encode()
        assert len(hexsig) <= SIG_LEN * 2
        out[start + 1 : start + 1 + len(hexsig)] = hexsig
        (HERE / name).write_bytes(out)
        return out


rsa = sign(b"adbe.pkcs7.detached", ["-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048"],
           "signed-rsa.pdf")
sign(b"ETSI.CAdES.detached", ["-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256"],
     "signed-ecdsa.pdf")

# A later revision: the page gets a new object, after the signature was made.
prev = rsa.rindex(b"startxref")
prev_xref = int(rsa[prev + len(b"startxref"):].split()[0])
update = bytearray(rsa)
offset = len(update)
update += b"6 0 obj\n<< /Producer (Someone else) >>\nendobj\n"
xref = len(update)
update += b"xref\n6 1\n%010d 00000 n \n" % offset
update += b"trailer\n<< /Size 7 /Root 1 0 R /Info 6 0 R /Prev %d >>\nstartxref\n%d\n%%%%EOF\n" % (
    prev_xref, xref)
(HERE / "signed-then-changed.pdf").write_bytes(update)

# Certificate files to sign with, password "secret".
for name, key_args in [
    ("signer-rsa.p12", ["-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048"]),
    ("signer-ec.p12", ["-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256"]),
]:
    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        key, cert = tmp / "key.pem", tmp / "cert.pem"
        subprocess.run(["openssl", "genpkey", *key_args, "-out", str(key)], check=True)
        subprocess.run(
            ["openssl", "req", "-new", "-x509", "-key", str(key), "-out", str(cert), "-days",
             "3650", "-subj", "/CN=Pat Example/O=Panna tests", "-sha256"],
            check=True,
        )
        subprocess.run(
            ["openssl", "pkcs12", "-export", "-inkey", str(key), "-in", str(cert), "-out",
             str(HERE / name), "-passout", "pass:secret"],
            check=True,
        )
