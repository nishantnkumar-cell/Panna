//! Checking digital signatures, offline: that the signed bytes haven't changed, that the
//! signature was made with the key of the certificate it carries, and what that certificate
//! says about the signer. Whether to trust the certificate can't be settled without a list
//! of trusted certificates or a revocation check, so it's reported, not decided.

use std::collections::HashMap;

use cms::cert::CertificateChoices;
use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerIdentifier, SignerInfo};
use der::asn1::{ObjectIdentifier, OctetString, UintRef};
use der::{Decode, Encode, Reader, SliceReader, Tagged};
use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Name, Object};
use num_bigint::BigUint;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};
use x509_cert::Certificate;
use x509_cert::name::Name as X509Name;

/// Signature fields looked at at most.
const MAX_SIGNATURES: usize = 200;
/// The largest signature read, in bytes.
const MAX_SIGNATURE_LEN: usize = 1 << 20;
/// How deep the form's field tree is followed.
const MAX_DEPTH: u8 = 32;

/// How a signature checks out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigStatus {
    /// The signed bytes are unchanged and the signature matches the certificate.
    Valid,
    /// It doesn't check out, and why.
    Invalid(String),
    /// It can't be checked here, and why.
    Unsupported(String),
}

/// A signature in the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub field: String,
    pub page: Option<usize>,
    /// Who signed: the certificate's name, or the name the signature gives.
    pub signer: String,
    /// Who issued the signer's certificate, or empty when the signer issued it themselves.
    pub issuer: String,
    /// When it was signed, as the signature says (not proved by a time stamp).
    pub time: Option<String>,
    pub reason: String,
    pub location: String,
    pub status: SigStatus,
    /// Whether the document has been changed since it was signed (a later revision added).
    pub changed_after: bool,
    /// Whether the certificate wasn't valid at the time of signing.
    pub outside_validity: bool,
    /// Whether this signature certifies the document (and what it allows afterwards: 1 no
    /// changes, 2 form filling and signing, 3 also comments).
    pub certifies: Option<u8>,
    /// The signer's certificate, then each certificate in the signature that issued the one
    /// before it (checked, not just by name).
    pub chain: Vec<Cert>,
}

/// A certificate: whose it is, who issued it, and its SHA-256 fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cert {
    pub name: String,
    /// Empty when the certificate issued itself.
    pub issuer: String,
    /// Upper-case hex.
    pub fingerprint: String,
    der: Vec<u8>,
}

impl Cert {
    fn of(cert: &Certificate) -> Option<Self> {
        let der = cert.to_der().ok()?;
        let tbs = &cert.tbs_certificate;
        Some(Self {
            name: common_name(&tbs.subject),
            issuer: if tbs.issuer == tbs.subject {
                String::new()
            } else {
                common_name(&tbs.issuer)
            },
            fingerprint: Sha256::digest(&der).iter().map(|b| format!("{b:02X}")).collect(),
            der,
        })
    }

    /// A certificate file's certificate: DER, or PEM ("-----BEGIN CERTIFICATE-----").
    pub fn from_file(bytes: &[u8]) -> Result<Self, String> {
        let der = match pem_body(bytes) {
            Some(der) => der,
            None => bytes.to_vec(),
        };
        let cert = Certificate::from_der(&der).map_err(|_| "That isn't a certificate file".to_owned())?;
        Self::of(&cert).ok_or_else(|| "The certificate can't be read".to_owned())
    }

    /// The certificate as DER, for saving.
    pub fn der(&self) -> &[u8] {
        &self.der
    }

    /// Whether `issuer`'s key signed this certificate.
    fn issued_by(&self, issuer: &Cert) -> bool {
        match (
            Certificate::from_der(&self.der),
            Certificate::from_der(&issuer.der),
        ) {
            (Ok(child), Ok(parent)) => cert_signed_by(&child, &parent),
            _ => false,
        }
    }
}

/// The first PEM certificate's bytes.
fn pem_body(bytes: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let start = text.find("-----BEGIN CERTIFICATE-----")? + "-----BEGIN CERTIFICATE-----".len();
    let end = start + text.get(start..)?.find("-----END CERTIFICATE-----")?;
    let b64: Vec<u8> = text
        .get(start..end)?
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    base64_decode(&b64)
}

fn base64_decode(data: &[u8]) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let data = data
        .strip_suffix(b"==")
        .or_else(|| data.strip_suffix(b"="))
        .unwrap_or(data);
    let mut out = Vec::with_capacity(data.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in data {
        acc = (acc << 6) | u32::from(value(c)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Who among `trusted` the signature's certificates lead to: one of them is trusted, or
/// the last one was issued by a trusted certificate.
pub fn trusted_by<'a>(sig: &Signature, trusted: &'a [Cert]) -> Option<&'a Cert> {
    let last = sig.chain.last()?;
    trusted.iter().find(|t| {
        sig.chain.iter().any(|c| c.fingerprint == t.fingerprint)
            || (last.issuer == t.name && last.issued_by(t))
    })
}

struct Raw {
    field: String,
    page: Option<usize>,
    name: String,
    time: Option<String>,
    reason: String,
    location: String,
    sub_filter: String,
    byte_range: Vec<i64>,
    certifies: Option<u8>,
}

/// The text of a PDF string entry.
fn text(dict: &Dict<'_>, key: &[u8]) -> String {
    match dict.get::<Object<'_>>(key) {
        Some(Object::String(s)) => crate::document::decode_text_string(s.as_bytes())
            .trim()
            .to_owned(),
        _ => String::new(),
    }
}

/// A PDF date such as D:20261019120000Z as 2026-10-19 12:00.
fn pdf_date(s: &str) -> Option<String> {
    let d = s.strip_prefix("D:").unwrap_or(s);
    let digits: String = d.chars().take_while(char::is_ascii_digit).collect();
    let part = |at: usize, len: usize| digits.get(at..at + len);
    let year = part(0, 4)?;
    let date = format!(
        "{year}-{}-{}",
        part(4, 2).unwrap_or("01"),
        part(6, 2).unwrap_or("01")
    );
    Some(match (part(8, 2), part(10, 2)) {
        (Some(h), Some(m)) => {
            let zone = d.get(digits.len()..).unwrap_or("").replace('\'', "");
            let zone = match zone.as_str() {
                "" => String::new(),
                "Z" => " UTC".into(),
                z => format!(" {z}"),
            };
            format!("{date} {h}:{m}{zone}")
        }
        _ => date,
    })
}

/// What the field walk gathers, the page of each widget, and the certifying signature.
type Walk<'a> = (Vec<Raw>, &'a HashMap<(i32, i32), usize>, Option<(i32, i32)>);

fn read_fields(pdf: &Pdf) -> Vec<Raw> {
    let xref = pdf.xref();
    let catalog = xref.get::<Dict<'_>>(xref.root_id()).unwrap_or_default();
    // Which page each widget is on.
    let mut pages = HashMap::new();
    for (index, page) in pdf.pages().iter().enumerate() {
        if let Some(annots) = page.raw().get::<Array<'_>>(b"Annots") {
            for a in annots.iter::<Dict<'_>>().take(4_000) {
                if let Some(id) = a.obj_id() {
                    pages.insert((id.obj_number, id.gen_number), index);
                }
            }
        }
    }
    let certifying = catalog
        .get::<Dict<'_>>(b"Perms")
        .and_then(|p| p.get_ref(b"DocMDP"))
        .map(|r| (r.obj_number, r.gen_number));
    let mut out = Vec::new();
    let Some(fields) = catalog
        .get::<Dict<'_>>(b"AcroForm")
        .and_then(|f| f.get::<Array<'_>>(b"Fields"))
    else {
        return out;
    };
    fn walk(field: &Dict<'_>, parent: &str, sig: bool, depth: u8, ctx: &mut Walk<'_>) {
        if depth > MAX_DEPTH || ctx.0.len() >= MAX_SIGNATURES {
            return;
        }
        let own = text(field, b"T");
        let name = match (parent.is_empty(), own.is_empty()) {
            (_, true) => parent.to_owned(),
            (true, false) => own,
            (false, false) => format!("{parent}.{own}"),
        };
        let sig = field.get::<Name<'_>>(b"FT").map_or(sig, |t| t.as_str() == "Sig");
        if let Some(kids) = field.get::<Array<'_>>(b"Kids") {
            for kid in kids.iter::<Dict<'_>>() {
                walk(&kid, &name, sig, depth + 1, ctx);
            }
        }
        if !sig {
            return;
        }
        let Some(value) = field.get::<Dict<'_>>(b"V") else {
            return;
        };
        let page = field
            .obj_id()
            .and_then(|id| ctx.1.get(&(id.obj_number, id.gen_number)).copied())
            .or_else(|| {
                let kid = field.get::<Array<'_>>(b"Kids")?.iter::<Dict<'_>>().next()?;
                let id = kid.obj_id()?;
                ctx.1.get(&(id.obj_number, id.gen_number)).copied()
            });
        let certifies = field.get_ref(b"V").and_then(|r| {
            (Some((r.obj_number, r.gen_number)) == ctx.2).then(|| {
                value
                    .get::<Array<'_>>(b"Reference")
                    .and_then(|refs| refs.iter::<Dict<'_>>().next())
                    .and_then(|r| r.get::<Dict<'_>>(b"TransformParams"))
                    .and_then(|t| t.get::<i64>(b"P"))
                    .map_or(2, |p| p.clamp(1, 3) as u8)
            })
        });
        ctx.0.push(Raw {
            field: name,
            page,
            name: text(&value, b"Name"),
            time: pdf_date(&text(&value, b"M")),
            reason: text(&value, b"Reason"),
            location: text(&value, b"Location"),
            sub_filter: value
                .get::<Name<'_>>(b"SubFilter")
                .map(|n| n.as_str().to_owned())
                .unwrap_or_default(),
            byte_range: value
                .get::<Array<'_>>(b"ByteRange")
                .map(|a| a.iter::<i64>().take(8).collect())
                .unwrap_or_default(),
            certifies,
        });
    }
    let mut ctx = (Vec::new(), &pages, certifying);
    for f in fields.iter::<Dict<'_>>() {
        walk(&f, "", false, 0, &mut ctx);
    }
    out.append(&mut ctx.0);
    out
}

#[derive(Clone, Copy)]
enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn from_oid(oid: &ObjectIdentifier) -> Option<Self> {
        Some(match oid.to_string().as_str() {
            "1.3.14.3.2.26" => Hash::Sha1,
            "2.16.840.1.101.3.4.2.1" => Hash::Sha256,
            "2.16.840.1.101.3.4.2.2" => Hash::Sha384,
            "2.16.840.1.101.3.4.2.3" => Hash::Sha512,
            _ => return None,
        })
    }

    fn digest(self, parts: &[&[u8]]) -> Vec<u8> {
        fn run<D: Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut d = D::new();
            for p in parts {
                d.update(p);
            }
            d.finalize().to_vec()
        }
        match self {
            Hash::Sha1 => run::<Sha1>(parts),
            Hash::Sha256 => run::<Sha256>(parts),
            Hash::Sha384 => run::<Sha384>(parts),
            Hash::Sha512 => run::<Sha512>(parts),
        }
    }

    /// The DER DigestInfo that comes before the hash in an RSA PKCS #1 v1.5 signature.
    fn digest_info(self) -> &'static [u8] {
        match self {
            Hash::Sha1 => &[
                0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04, 0x14,
            ],
            Hash::Sha256 => &[
                0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
                0x05, 0x00, 0x04, 0x20,
            ],
            Hash::Sha384 => &[
                0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02,
                0x05, 0x00, 0x04, 0x30,
            ],
            Hash::Sha512 => &[
                0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03,
                0x05, 0x00, 0x04, 0x40,
            ],
        }
    }
}

/// A name's common name, or the whole name when it has none.
fn common_name(name: &X509Name) -> String {
    for rdn in name.0.iter() {
        for atv in rdn.0.iter() {
            if atv.oid.to_string() == "2.5.4.3" {
                return String::from_utf8_lossy(atv.value.value()).into_owned();
            }
        }
    }
    name.to_string()
}

fn unix_date(t: x509_cert::time::Time) -> String {
    let d = t.to_date_time();
    format!(
        "{}-{:02}-{:02} {:02}:{:02} UTC",
        d.year(),
        d.month(),
        d.day(),
        d.hour(),
        d.minutes()
    )
}

/// RSA PKCS #1 v1.5: whether `sig` is `hash` signed with the key `(n, e)`.
fn rsa_verifies(n: &[u8], e: &[u8], sig: &[u8], hash: Hash, digest: &[u8]) -> bool {
    let n = BigUint::from_bytes_be(n);
    let e = BigUint::from_bytes_be(e);
    let k = n.to_bytes_be().len();
    let s = BigUint::from_bytes_be(sig);
    if sig.len() != k || s >= n || !(64..=2048).contains(&k) {
        return false;
    }
    let m = s.modpow(&e, &n).to_bytes_be();
    let mut em = vec![0u8; k - m.len()];
    em.extend(m);
    let info = hash.digest_info();
    let t_len = info.len() + digest.len();
    if k < t_len + 11 {
        return false;
    }
    let mut expected = vec![0x00, 0x01];
    expected.extend(std::iter::repeat_n(0xff, k - t_len - 3));
    expected.push(0x00);
    expected.extend(info);
    expected.extend(digest);
    em == expected
}

/// RSASSA-PSS's settings: the hash of the message, the hash MGF1 masks with, and the salt's
/// length in bytes.
struct Pss {
    hash: Hash,
    mask_hash: Hash,
    salt: usize,
}

/// The RSASSA-PSS-params of a signature algorithm, with their defaults (SHA-1, MGF1 with
/// SHA-1, 20 bytes of salt).
fn pss_params(params: Option<&der::Any>) -> Result<Pss, String> {
    let bad = || "The RSA-PSS settings can't be read".to_owned();
    let mut pss = Pss {
        hash: Hash::Sha1,
        mask_hash: Hash::Sha1,
        salt: 20,
    };
    let Some(params) = params else { return Ok(pss) };
    let mut r = SliceReader::new(params.value()).map_err(|_| bad())?;
    while !r.is_finished() {
        let item = der::Any::decode(&mut r).map_err(|_| bad())?;
        let der::Tag::ContextSpecific { number, .. } = item.tag() else {
            return Err(bad());
        };
        let alg = || spki::AlgorithmIdentifierOwned::from_der(item.value()).map_err(|_| bad());
        match number.value() {
            0 => pss.hash = Hash::from_oid(&alg()?.oid).ok_or_else(bad)?,
            1 => {
                let mgf = alg()?;
                if mgf.oid.to_string() != "1.2.840.113549.1.1.8" {
                    return Err("The RSA-PSS signature masks in a way Panna doesn't know".into());
                }
                let inner = mgf
                    .parameters
                    .as_ref()
                    .and_then(|p| p.decode_as::<spki::AlgorithmIdentifierOwned>().ok())
                    .ok_or_else(bad)?;
                pss.mask_hash = Hash::from_oid(&inner.oid).ok_or_else(bad)?;
            }
            2 => {
                let salt = u32::from_der(item.value()).map_err(|_| bad())?;
                pss.salt = usize::try_from(salt).map_err(|_| bad())?;
            }
            3 if u32::from_der(item.value()) == Ok(1) => {}
            _ => return Err(bad()),
        }
    }
    Ok(pss)
}

/// MGF1 with `hash`: `len` bytes of mask from `seed`.
fn mgf1(hash: Hash, seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 64);
    let mut counter = 0u32;
    while out.len() < len {
        out.extend(hash.digest(&[seed, &counter.to_be_bytes()]));
        counter += 1;
    }
    out.truncate(len);
    out
}

/// RSASSA-PSS (RFC 8017, EMSA-PSS-VERIFY): whether `sig` is `digest`, the message's hash,
/// signed with the key `(n, e)`.
fn rsa_pss_verifies(n: &[u8], e: &[u8], sig: &[u8], pss: &Pss, digest: &[u8]) -> bool {
    let n = BigUint::from_bytes_be(n);
    let e = BigUint::from_bytes_be(e);
    let k = n.to_bytes_be().len();
    let s = BigUint::from_bytes_be(sig);
    if sig.len() != k || s >= n || !(64..=2048).contains(&k) {
        return false;
    }
    let em_bits = (n.bits() - 1) as usize;
    let em_len = em_bits.div_ceil(8);
    let m = s.modpow(&e, &n).to_bytes_be();
    let h_len = digest.len();
    if m.len() > em_len || em_len < h_len + pss.salt + 2 {
        return false;
    }
    let mut em = vec![0u8; em_len - m.len()];
    em.extend(m);
    if em.last() != Some(&0xbc) {
        return false;
    }
    let (masked, rest) = em.split_at(em_len - h_len - 1);
    let h = rest.get(..h_len).unwrap_or_default();
    // The bits above em_bits must be clear.
    let spare = 8 * em_len - em_bits;
    let top = 0xffu8.checked_shr(spare as u32).unwrap_or(0);
    if masked.first().is_some_and(|b| b & !top != 0) {
        return false;
    }
    let mut db: Vec<u8> = masked
        .iter()
        .zip(mgf1(pss.mask_hash, h, masked.len()))
        .map(|(a, b)| a ^ b)
        .collect();
    if let Some(first) = db.first_mut() {
        *first &= top;
    }
    let pad = em_len - h_len - pss.salt - 2;
    if db.get(..pad).is_none_or(|z| z.iter().any(|&b| b != 0)) || db.get(pad) != Some(&1) {
        return false;
    }
    let salt = db.get(pad + 1..).unwrap_or_default();
    pss.hash.digest(&[&[0u8; 8], digest, salt]) == h
}

/// Whether `sig` over `message` was made with the certificate's key.
fn verifies(
    cert: &Certificate,
    algorithm: &spki::AlgorithmIdentifierOwned,
    sig: &[u8],
    digest_hash: Hash,
    message: &[u8],
) -> Result<bool, String> {
    let spki = &cert.tbs_certificate.subject_public_key_info;
    let key = spki.subject_public_key.raw_bytes();
    let alg = algorithm.oid.to_string();
    // The hash the signature algorithm names, when it names one.
    let named = match alg.as_str() {
        "1.2.840.113549.1.1.5" | "1.2.840.10045.4.1" => Some(Hash::Sha1),
        "1.2.840.113549.1.1.11" | "1.2.840.10045.4.3.2" => Some(Hash::Sha256),
        "1.2.840.113549.1.1.12" | "1.2.840.10045.4.3.3" => Some(Hash::Sha384),
        "1.2.840.113549.1.1.13" | "1.2.840.10045.4.3.4" => Some(Hash::Sha512),
        _ => None,
    };
    let hash = named.unwrap_or(digest_hash);
    let digest = hash.digest(&[message]);
    match spki.algorithm.oid.to_string().as_str() {
        // An RSA key, or one only for RSA-PSS.
        "1.2.840.113549.1.1.1" | "1.2.840.113549.1.1.10" => {
            let mut reader = SliceReader::new(key).map_err(|e| e.to_string())?;
            let (n, e) = reader
                .sequence(|r| Ok((UintRef::decode(r)?, UintRef::decode(r)?)))
                .map_err(|_| "The certificate's RSA key can't be read".to_owned())?;
            if alg == "1.2.840.113549.1.1.10" {
                let pss = pss_params(algorithm.parameters.as_ref())?;
                let digest = pss.hash.digest(&[message]);
                return Ok(rsa_pss_verifies(n.as_bytes(), e.as_bytes(), sig, &pss, &digest));
            }
            Ok(rsa_verifies(n.as_bytes(), e.as_bytes(), sig, hash, &digest))
        }
        "1.2.840.10045.2.1" => {
            use p256::ecdsa::signature::hazmat::PrehashVerifier;
            let curve = spki
                .algorithm
                .parameters
                .as_ref()
                .and_then(|p| p.decode_as::<ObjectIdentifier>().ok())
                .map(|o| o.to_string())
                .unwrap_or_default();
            match curve.as_str() {
                "1.2.840.10045.3.1.7" => {
                    let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(key)
                        .map_err(|_| "The certificate's key can't be read".to_owned())?;
                    let Ok(sig) = p256::ecdsa::Signature::from_der(sig) else {
                        return Ok(false);
                    };
                    Ok(key.verify_prehash(&digest, &sig).is_ok())
                }
                "1.3.132.0.34" => {
                    let key = p384::ecdsa::VerifyingKey::from_sec1_bytes(key)
                        .map_err(|_| "The certificate's key can't be read".to_owned())?;
                    let Ok(sig) = p384::ecdsa::Signature::from_der(sig) else {
                        return Ok(false);
                    };
                    Ok(key.verify_prehash(&digest, &sig).is_ok())
                }
                _ => Err("This kind of elliptic curve isn't supported yet".into()),
            }
        }
        _ => Err("This kind of key isn't supported yet".into()),
    }
}

/// Whether `parent`'s key signed `child`.
fn cert_signed_by(child: &Certificate, parent: &Certificate) -> bool {
    if child.tbs_certificate.issuer != parent.tbs_certificate.subject {
        return false;
    }
    let (Ok(tbs), Some(sig)) = (child.tbs_certificate.to_der(), child.signature.as_bytes()) else {
        return false;
    };
    verifies(parent, &child.signature_algorithm, sig, Hash::Sha256, &tbs).unwrap_or(false)
}

/// What a signature's CMS data says: who signed, when the signer says it was, how it checks
/// out, and whether the certificate was valid then.
struct Checked {
    signer: Option<String>,
    issuer: String,
    time: Option<String>,
    status: SigStatus,
    outside_validity: bool,
    chain: Vec<Cert>,
}

fn check_cms(der: &[u8], signed: &[&[u8]]) -> Checked {
    let mut out = Checked {
        signer: None,
        issuer: String::new(),
        time: None,
        status: SigStatus::Unsupported(String::new()),
        outside_validity: false,
        chain: Vec::new(),
    };
    let unsupported = |why: &str| SigStatus::Unsupported(why.to_owned());
    // The data is padded with zeros after the signature itself.
    let parsed = SliceReader::new(der)
        .and_then(|mut r| ContentInfo::decode(&mut r))
        .and_then(|ci| ci.content.decode_as::<SignedData>());
    let Ok(data) = parsed else {
        out.status = unsupported("The signature's data can't be read (it isn't DER-encoded CMS)");
        return out;
    };
    let Some(info) = data.signer_infos.0.iter().next() else {
        out.status = SigStatus::Invalid("The signature has no signer".into());
        return out;
    };
    let certs: Vec<&Certificate> = data
        .certificates
        .as_ref()
        .map(|c| {
            c.0.iter()
                .filter_map(|c| match c {
                    CertificateChoices::Certificate(c) => Some(c),
                    CertificateChoices::Other(_) => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let cert = certs.iter().copied().find(|c| match &info.sid {
        SignerIdentifier::IssuerAndSerialNumber(id) => {
            id.issuer.to_der().ok() == c.tbs_certificate.issuer.to_der().ok()
                && id.serial_number.to_der().ok() == c.tbs_certificate.serial_number.to_der().ok()
        }
        SignerIdentifier::SubjectKeyIdentifier(ski) => {
            c.tbs_certificate.extensions.as_ref().is_some_and(|exts| {
                exts.iter().any(|e| {
                    e.extn_id.to_string() == "2.5.29.14"
                        && OctetString::from_der(e.extn_value.as_bytes())
                            .is_ok_and(|v| v.as_bytes() == ski.0.as_bytes())
                })
            })
        }
    });
    let Some(cert) = cert else {
        out.status = unsupported("The signer's certificate isn't in the signature");
        return out;
    };
    // Up from the signer's certificate, through those in the signature that issued each.
    let mut at = cert;
    out.chain.extend(Cert::of(at));
    while out.chain.len() < 10 && at.tbs_certificate.issuer != at.tbs_certificate.subject {
        let Some(next) = certs.iter().copied().find(|c| cert_signed_by(at, c)) else {
            break;
        };
        out.chain.extend(Cert::of(next));
        at = next;
    }
    let tbs = &cert.tbs_certificate;
    out.signer = Some(common_name(&tbs.subject));
    if tbs.issuer != tbs.subject {
        out.issuer = common_name(&tbs.issuer);
    }
    let Some(hash) = Hash::from_oid(&info.digest_alg.oid) else {
        out.status = unsupported("The signature uses a hash Panna doesn't know");
        return out;
    };
    let content_digest = hash.digest(signed);
    let message = match &info.signed_attrs {
        Some(attrs) => {
            let mut digest_ok = None;
            for attr in attrs.iter() {
                match attr.oid.to_string().as_str() {
                    "1.2.840.113549.1.9.4" => {
                        digest_ok = attr
                            .values
                            .iter()
                            .next()
                            .and_then(|v| v.decode_as::<OctetString>().ok())
                            .map(|v| v.as_bytes() == content_digest.as_slice());
                    }
                    "1.2.840.113549.1.9.5" => {
                        let when = attr
                            .values
                            .iter()
                            .next()
                            .and_then(|v| x509_cert::time::Time::from_der(&v.to_der().ok()?).ok());
                        if let Some(when) = when {
                            let v = &tbs.validity;
                            out.outside_validity = when.to_unix_duration() < v.not_before.to_unix_duration()
                                || when.to_unix_duration() > v.not_after.to_unix_duration();
                            out.time = Some(unix_date(when));
                        }
                    }
                    _ => {}
                }
            }
            match digest_ok {
                Some(true) => {}
                Some(false) => {
                    out.status =
                        SigStatus::Invalid("The document has been altered since it was signed".into());
                    return out;
                }
                None => {
                    out.status = SigStatus::Invalid("The signature doesn't say what was signed".into());
                    return out;
                }
            }
            match attrs.to_der() {
                Ok(d) => d,
                Err(_) => {
                    out.status = unsupported("The signature's details can't be read");
                    return out;
                }
            }
        }
        None => signed.concat(),
    };
    out.status = match verifies(
        cert,
        &info.signature_algorithm,
        info.signature.as_bytes(),
        hash,
        &message,
    ) {
        Ok(true) => SigStatus::Valid,
        Ok(false) if info.signed_attrs.is_none() => {
            SigStatus::Invalid("The document has been altered since it was signed".into())
        }
        Ok(false) => SigStatus::Invalid("The signature doesn't match the signer's certificate".into()),
        Err(why) => SigStatus::Unsupported(why),
    };
    out
}

pub(crate) fn read(pdf: &Pdf, bytes: &[u8]) -> Vec<Signature> {
    let len = bytes.len();
    // A file may end with whitespace after its last %%EOF.
    let end_of_data = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(0, |p| p + 1);
    read_fields(pdf)
        .into_iter()
        .map(|raw| {
            let mut sig = Signature {
                field: raw.field,
                page: raw.page,
                signer: raw.name.clone(),
                issuer: String::new(),
                time: raw.time,
                reason: raw.reason,
                location: raw.location,
                status: SigStatus::Unsupported(String::new()),
                changed_after: false,
                outside_validity: false,
                certifies: raw.certifies,
                chain: Vec::new(),
            };
            let r: Vec<usize> = raw
                .byte_range
                .iter()
                .filter_map(|&v| usize::try_from(v).ok())
                .collect();
            let [a, b, c, d] = r[..] else {
                sig.status = SigStatus::Invalid("The signature doesn't say which bytes it covers".into());
                return sig;
            };
            let gap = bytes.get(a.saturating_add(b)..c);
            let (Some(first), Some(second), Some(gap)) = (
                bytes.get(a..a.saturating_add(b)),
                bytes.get(c..c.saturating_add(d)),
                gap,
            ) else {
                sig.status = SigStatus::Invalid("The signature covers bytes the file doesn't have".into());
                return sig;
            };
            if a != 0
                || gap.first() != Some(&b'<')
                || gap.last() != Some(&b'>')
                || gap.len() > MAX_SIGNATURE_LEN * 2 + 2
            {
                sig.status = SigStatus::Invalid("The signature covers the wrong part of the file".into());
                return sig;
            }
            let hex: Vec<u8> = gap
                .get(1..gap.len() - 1)
                .unwrap_or_default()
                .iter()
                .copied()
                .filter(|b| !b.is_ascii_whitespace())
                .collect();
            let der: Option<Vec<u8>> = hex
                .chunks(2)
                .map(|p| {
                    std::str::from_utf8(p)
                        .ok()
                        .and_then(|s| u8::from_str_radix(s, 16).ok())
                })
                .collect();
            let Some(der) = der else {
                sig.status = SigStatus::Invalid("The signature's data is damaged".into());
                return sig;
            };
            sig.changed_after = c + d < end_of_data.min(len);
            if raw.sub_filter == "adbe.x509.rsa_sha1" || raw.sub_filter == "adbe.pkcs7.sha1" {
                sig.status =
                    SigStatus::Unsupported(format!("{} signatures aren't supported yet", raw.sub_filter));
                return sig;
            }
            let checked = check_cms(&der, &[first, second]);
            if let Some(name) = checked.signer {
                sig.signer = name;
            }
            sig.issuer = checked.issuer;
            sig.time = checked.time.or(sig.time);
            sig.status = checked.status;
            sig.outside_validity = checked.outside_validity;
            sig.chain = checked.chain;
            sig
        })
        .collect()
}

/// Signs a SHA-256 digest with a key held outside Panna, such as in the system's certificate
/// store: an RSA key gives a PKCS #1 v1.5 signature, an elliptic-curve key its two numbers
/// one after the other, each padded to the curve's size.
pub type SignDigest = std::sync::Arc<dyn Fn(&[u8]) -> Result<Vec<u8>, String> + Send + Sync>;

/// A key held outside Panna, with its certificates.
#[derive(Clone)]
pub struct HeldKey {
    /// The certificates, the signer's own first, in DER.
    pub chain: Vec<Vec<u8>>,
    pub sign: SignDigest,
}

impl std::fmt::Debug for HeldKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldKey")
            .field("certificates", &self.chain.len())
            .finish()
    }
}

/// A digital ID to sign with.
#[derive(Clone, Debug)]
pub enum DigitalId {
    /// A certificate file (.p12 or .pfx) and its password.
    File {
        pkcs12: std::sync::Arc<Vec<u8>>,
        password: String,
    },
    Held(HeldKey),
}

/// A private key to sign with.
pub(crate) enum Key {
    Rsa(Box<rsa::RsaPrivateKey>),
    P256(p256::ecdsa::SigningKey),
    P384(p384::ecdsa::SigningKey),
    /// Held elsewhere; `curve` is P-256 or P-384's size in bytes for an elliptic-curve key.
    Held {
        sign: SignDigest,
        curve: Option<usize>,
    },
}

const UNSUPPORTED_KEY: &str = "The certificate's kind of key isn't supported yet (RSA, P-256 and P-384 are).";

/// The signer's key and certificates (their own first) from a PKCS #12 file.
pub(crate) struct Signer {
    key: Key,
    chain: Vec<Certificate>,
}

impl Signer {
    pub(crate) fn from_pkcs12(bytes: &[u8], password: &str) -> Result<Self, String> {
        let store = p12_keystore::KeyStore::from_pkcs12(bytes, password).map_err(|_| {
            "The password is wrong, or the file isn't a certificate file (.p12 or .pfx).".to_owned()
        })?;
        let (_, chain) = store
            .private_key_chain()
            .ok_or_else(|| "The certificate file has no private key to sign with.".to_owned())?;
        let key_der = chain.key();
        let key = {
            use p256::pkcs8::DecodePrivateKey;
            if let Ok(k) = rsa::RsaPrivateKey::from_pkcs8_der(key_der) {
                Key::Rsa(Box::new(k))
            } else if let Ok(k) = p256::ecdsa::SigningKey::from_pkcs8_der(key_der) {
                Key::P256(k)
            } else if let Ok(k) = p384::ecdsa::SigningKey::from_pkcs8_der(key_der) {
                Key::P384(k)
            } else {
                return Err(UNSUPPORTED_KEY.into());
            }
        };
        let chain: Vec<Certificate> = chain
            .chain()
            .iter()
            .filter_map(|c| Certificate::from_der(c.as_der()).ok())
            .collect();
        if chain.is_empty() {
            return Err("The certificate file has no certificate for its key.".into());
        }
        Ok(Self { key, chain })
    }

    pub(crate) fn of(id: &DigitalId) -> Result<Self, String> {
        match id {
            DigitalId::File { pkcs12, password } => Self::from_pkcs12(pkcs12, password),
            DigitalId::Held(held) => Self::held(held),
        }
    }

    fn held(held: &HeldKey) -> Result<Self, String> {
        let chain: Vec<Certificate> = held
            .chain
            .iter()
            .filter_map(|c| Certificate::from_der(c).ok())
            .collect();
        let cert = chain
            .first()
            .ok_or_else(|| "The certificate couldn't be read.".to_owned())?;
        let spki = &cert.tbs_certificate.subject_public_key_info;
        let curve = match spki.algorithm.oid.to_string().as_str() {
            "1.2.840.113549.1.1.1" => None,
            "1.2.840.10045.2.1" => {
                let named = spki
                    .algorithm
                    .parameters
                    .as_ref()
                    .and_then(|p| p.decode_as::<ObjectIdentifier>().ok())
                    .map(|o| o.to_string());
                match named.as_deref() {
                    Some("1.2.840.10045.3.1.7") => Some(32),
                    Some("1.3.132.0.34") => Some(48),
                    _ => return Err(UNSUPPORTED_KEY.into()),
                }
            }
            _ => return Err(UNSUPPORTED_KEY.into()),
        };
        Ok(Self {
            key: Key::Held {
                sign: std::sync::Arc::clone(&held.sign),
                curve,
            },
            chain,
        })
    }

    /// The name on the signer's certificate.
    pub(crate) fn name(&self) -> String {
        self.chain
            .first()
            .map(|c| common_name(&c.tbs_certificate.subject))
            .unwrap_or_default()
    }

    /// A detached CMS signature (as `adbe.pkcs7.detached` wants) over `parts`, with SHA-256.
    pub(crate) fn sign(&self, parts: &[&[u8]]) -> Result<Vec<u8>, String> {
        use cms::content_info::CmsVersion;
        use cms::signed_data::{CertificateSet, EncapsulatedContentInfo, SignerInfos};
        use der::asn1::{Any, SetOfVec};
        use spki::AlgorithmIdentifierOwned;
        use x509_cert::attr::Attribute;

        let fail = |e: der::Error| format!("The signature couldn't be made: {e}");
        let oid = |s: &str| ObjectIdentifier::new(s).map_err(|e| format!("{e}"));
        let cert = self.chain.first().ok_or("No certificate")?;
        let digest = Hash::Sha256.digest(parts);
        let attr = |id: &str, value: Any| -> Result<Attribute, String> {
            let mut values = SetOfVec::new();
            values.insert(value).map_err(fail)?;
            Ok(Attribute {
                oid: oid(id)?,
                values,
            })
        };
        let data_oid = oid("1.2.840.113549.1.7.1")?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?;
        let time = x509_cert::time::Time::UtcTime(der::asn1::UtcTime::from_unix_duration(now).map_err(fail)?);
        let mut attrs = SetOfVec::new();
        attrs
            .insert(attr(
                "1.2.840.113549.1.9.3",
                Any::encode_from(&data_oid).map_err(fail)?,
            )?)
            .map_err(fail)?;
        attrs
            .insert(attr(
                "1.2.840.113549.1.9.5",
                Any::encode_from(&time).map_err(fail)?,
            )?)
            .map_err(fail)?;
        attrs
            .insert(attr(
                "1.2.840.113549.1.9.4",
                Any::encode_from(&OctetString::new(digest).map_err(fail)?).map_err(fail)?,
            )?)
            .map_err(fail)?;
        let signed = attrs.to_der().map_err(fail)?;
        let message_digest = Hash::Sha256.digest(&[&signed]);
        let (signature, algorithm) = match &self.key {
            Key::Rsa(key) => {
                let mut rng = rand_core::OsRng;
                let sig = key
                    .sign_with_rng(&mut rng, rsa::Pkcs1v15Sign::new::<Sha256>(), &message_digest)
                    .map_err(|e| format!("The signature couldn't be made: {e}"))?;
                (sig, "1.2.840.113549.1.1.11")
            }
            Key::P256(key) => {
                use p256::ecdsa::signature::hazmat::PrehashSigner;
                let sig: p256::ecdsa::Signature = key
                    .sign_prehash(&message_digest)
                    .map_err(|e| format!("The signature couldn't be made: {e}"))?;
                (sig.to_der().as_bytes().to_vec(), "1.2.840.10045.4.3.2")
            }
            Key::P384(key) => {
                use p384::ecdsa::signature::hazmat::PrehashSigner;
                let sig: p384::ecdsa::Signature = key
                    .sign_prehash(&message_digest)
                    .map_err(|e| format!("The signature couldn't be made: {e}"))?;
                (sig.to_der().as_bytes().to_vec(), "1.2.840.10045.4.3.2")
            }
            Key::Held { sign, curve } => {
                let made = sign(&message_digest)?;
                let wrong = || "The signature that came back couldn't be read.".to_owned();
                match curve {
                    None => (made, "1.2.840.113549.1.1.11"),
                    Some(32) => (
                        p256::ecdsa::Signature::from_slice(&made)
                            .map_err(|_| wrong())?
                            .to_der()
                            .as_bytes()
                            .to_vec(),
                        "1.2.840.10045.4.3.2",
                    ),
                    Some(_) => (
                        p384::ecdsa::Signature::from_slice(&made)
                            .map_err(|_| wrong())?
                            .to_der()
                            .as_bytes()
                            .to_vec(),
                        "1.2.840.10045.4.3.2",
                    ),
                }
            }
        };
        let sha256 = AlgorithmIdentifierOwned {
            oid: oid("2.16.840.1.101.3.4.2.1")?,
            parameters: None,
        };
        let info = SignerInfo {
            version: CmsVersion::V1,
            sid: SignerIdentifier::IssuerAndSerialNumber(cms::cert::IssuerAndSerialNumber {
                issuer: cert.tbs_certificate.issuer.clone(),
                serial_number: cert.tbs_certificate.serial_number.clone(),
            }),
            digest_alg: sha256.clone(),
            signed_attrs: Some(attrs),
            signature_algorithm: AlgorithmIdentifierOwned {
                oid: oid(algorithm)?,
                parameters: matches!(self.key, Key::Rsa(_) | Key::Held { curve: None, .. })
                    .then_some(Any::null()),
            },
            signature: OctetString::new(signature).map_err(fail)?,
            unsigned_attrs: None,
        };
        let mut digests = SetOfVec::new();
        digests.insert(sha256).map_err(fail)?;
        let mut certs = SetOfVec::new();
        for c in &self.chain {
            certs
                .insert(CertificateChoices::Certificate(c.clone()))
                .map_err(fail)?;
        }
        let mut infos = SetOfVec::new();
        infos.insert(info).map_err(fail)?;
        let data = SignedData {
            version: CmsVersion::V1,
            digest_algorithms: digests,
            encap_content_info: EncapsulatedContentInfo {
                econtent_type: data_oid,
                econtent: None,
            },
            certificates: Some(CertificateSet(certs)),
            crls: None,
            signer_infos: SignerInfos(infos),
        };
        ContentInfo {
            content_type: oid("1.2.840.113549.1.7.2")?,
            content: Any::encode_from(&data).map_err(fail)?,
        }
        .to_der()
        .map_err(fail)
    }
}
