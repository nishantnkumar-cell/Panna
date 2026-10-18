//! Encrypting what an incremental update adds to a password-protected file, with the standard
//! security handler (ISO 32000-2, 7.6.4): RC4 and AES-128 (revisions 2 to 4) and AES-256
//! (revisions 5 and 6).
//!
//! The file key is worked out again from the password the file was opened with. New strings
//! and streams are encrypted with it; values copied from the original file are already
//! encrypted for their object and are left alone.

use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::{Aes128, Aes256};
use md5::{Digest, Md5};
use sha2::{Sha256, Sha384, Sha512};

use super::SaveError;
use super::lex::{Lexer, dict_entries};

const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E,
    0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// How one kind of data (strings or streams) is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    None,
    Rc4,
    Aes128,
    Aes256,
}

/// The file key and how to use it.
#[derive(Debug, Clone)]
pub(crate) struct Crypt {
    key: Vec<u8>,
    strings: Method,
    streams: Method,
    /// The permission flags (/P).
    p: i32,
    /// Opened with the owner password, which lifts the permission flags.
    owner: bool,
}

impl Crypt {
    /// Reads the encryption dictionary's raw bytes and works out the file key from `password`
    /// (user or owner). `id` is the first element of the trailer's /ID.
    pub(crate) fn new(dict: &[u8], id: &[u8], password: &str) -> Result<Self, SaveError> {
        let unsupported = || SaveError::Encrypted;
        let entries = dict_entries(dict).ok_or(SaveError::Damaged)?;
        let raw = |key: &[u8]| {
            entries
                .iter()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| dict.get(v.clone()))
        };
        let int = |key: &[u8]| {
            raw(key)
                .and_then(|v| std::str::from_utf8(v).ok())
                .and_then(|v| v.trim().parse::<i64>().ok())
        };
        let name = |v: &[u8]| Lexer::new(v, 0).name().map(<[u8]>::to_vec);
        let string = |key: &[u8]| raw(key).and_then(parse_string);
        if raw(b"Filter").and_then(name).as_deref() != Some(b"Standard") {
            return Err(unsupported());
        }
        let v = int(b"V").unwrap_or(0);
        let r = int(b"R").ok_or(SaveError::Damaged)?;
        let o = string(b"O").ok_or(SaveError::Damaged)?;
        let u = string(b"U").ok_or(SaveError::Damaged)?;
        let p = i32::try_from(int(b"P").ok_or(SaveError::Damaged)?).unwrap_or_else(|_| {
            // Some writers store /P as an unsigned 32-bit number.
            int(b"P")
                .and_then(|p| u32::try_from(p).ok())
                .map_or(0, |p| p as i32)
        });
        let encrypt_metadata = raw(b"EncryptMetadata").is_none_or(|v| v.trim_ascii() != b"false");

        // Version 4 and 5 name a crypt filter for strings and for streams.
        let filter = |key: &[u8]| -> Result<Method, SaveError> {
            let Some(chosen) = raw(key).and_then(name) else {
                return Ok(Method::None);
            };
            if chosen == b"Identity" {
                return Ok(Method::None);
            }
            let filters = raw(b"CF").ok_or(SaveError::Damaged)?;
            let list = dict_entries(filters).ok_or(SaveError::Damaged)?;
            let cf = list
                .iter()
                .find(|(k, _)| *k == chosen.as_slice())
                .and_then(|(_, v)| filters.get(v.clone()))
                .ok_or(SaveError::Damaged)?;
            let cf_entries = dict_entries(cf).ok_or(SaveError::Damaged)?;
            let cfm = cf_entries
                .iter()
                .find(|(k, _)| *k == b"CFM")
                .and_then(|(_, v)| cf.get(v.clone()))
                .and_then(name);
            match cfm.as_deref() {
                None | Some(b"None") => Ok(Method::None),
                Some(b"V2") => Ok(Method::Rc4),
                Some(b"AESV2") => Ok(Method::Aes128),
                Some(b"AESV3") => Ok(Method::Aes256),
                Some(_) => Err(unsupported()),
            }
        };

        match v {
            1 | 2 | 4 if (2..=4).contains(&r) => {
                let bytes = if r == 2 {
                    5
                } else {
                    let bits = int(b"Length").unwrap_or(if v == 4 { 128 } else { 40 });
                    usize::try_from(bits / 8)
                        .ok()
                        .filter(|n| (5..=16).contains(n))
                        .ok_or(SaveError::Damaged)?
                };
                let legacy = Legacy {
                    r,
                    bytes,
                    o: &o,
                    u: &u,
                    p,
                    id,
                    encrypt_metadata: encrypt_metadata || r < 4,
                };
                let (key, owner) = legacy.key(password.as_bytes()).ok_or(SaveError::Encrypted)?;
                let (strings, streams) = if v == 4 {
                    (filter(b"StrF")?, filter(b"StmF")?)
                } else {
                    (Method::Rc4, Method::Rc4)
                };
                Ok(Self {
                    key,
                    strings,
                    streams,
                    p,
                    owner,
                })
            }
            5 if r == 5 || r == 6 => {
                let oe = string(b"OE").ok_or(SaveError::Damaged)?;
                let ue = string(b"UE").ok_or(SaveError::Damaged)?;
                let (key, owner) =
                    modern_key(r, password.as_bytes(), &o, &u, &oe, &ue).ok_or(SaveError::Encrypted)?;
                Ok(Self {
                    key,
                    strings: filter(b"StrF")?,
                    streams: filter(b"StmF")?,
                    p,
                    owner,
                })
            }
            _ => Err(unsupported()),
        }
    }

    /// Whether the file's permissions allow printing (bit 3 of /P), or the owner password
    /// was given.
    pub(crate) fn may_print(&self) -> bool {
        self.owner || self.p & 0b100 != 0
    }

    /// Whether the file's protection may be changed or removed: opened with the owner
    /// password, or a file that allows changes (bit 4 of /P) anyway.
    pub(crate) fn may_change_protection(&self) -> bool {
        self.owner || self.p & 0b1000 != 0
    }

    /// Encrypts every string and the stream data in a new object's body.
    pub(crate) fn seal(&self, num: u32, generation: u16, body: &[u8]) -> Result<Vec<u8>, SaveError> {
        let (dict, data) = match find(body, b"\nstream\n") {
            Some(at) => {
                let start = at + b"\nstream\n".len();
                let end = rfind(body, b"\nendstream")
                    .filter(|e| *e >= start)
                    .ok_or(SaveError::Damaged)?;
                (
                    body.get(..at).unwrap_or_default(),
                    Some(body.get(start..end).unwrap_or_default()),
                )
            }
            None => (body, None),
        };
        let mut out = self.strings_in(num, generation, dict)?;
        if let Some(data) = data {
            let sealed = self.encrypt(self.streams, num, generation, data)?;
            out = set_length(&out, sealed.len()).ok_or(SaveError::Damaged)?;
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(&sealed);
            out.extend_from_slice(b"\nendstream");
        }
        Ok(out)
    }

    /// Re-encodes each string in `value` (PDF syntax without streams) encrypted for object
    /// `num`, as hex.
    pub(crate) fn strings_in(&self, num: u32, generation: u16, value: &[u8]) -> Result<Vec<u8>, SaveError> {
        let mut out = Vec::with_capacity(value.len() * 2);
        let mut i = 0;
        while let Some(&b) = value.get(i) {
            match b {
                b'<' if value.get(i + 1) == Some(&b'<') => {
                    out.extend_from_slice(b"<<");
                    i += 2;
                }
                b'(' | b'<' => {
                    let mut lex = Lexer::new(value, i);
                    let span = lex.value().ok_or(SaveError::Damaged)?;
                    let raw = value.get(span.clone()).ok_or(SaveError::Damaged)?;
                    let plain = parse_string(raw).ok_or(SaveError::Damaged)?;
                    let sealed = self.encrypt(self.strings, num, generation, &plain)?;
                    out.push(b'<');
                    for byte in sealed {
                        out.extend_from_slice(format!("{byte:02X}").as_bytes());
                    }
                    out.push(b'>');
                    i = span.end.max(i + 1);
                }
                _ => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        Ok(out)
    }

    /// The plain bytes of a string as stored in object `num`.
    pub(crate) fn open_string(&self, num: u32, generation: u16, data: &[u8]) -> Result<Vec<u8>, SaveError> {
        match self.strings {
            Method::None => Ok(data.to_vec()),
            Method::Rc4 => Ok(rc4(&self.object_key(num, generation, false), data)),
            Method::Aes128 => {
                let key = self.object_key(num, generation, true);
                let cipher = Aes128::new_from_slice(&key).map_err(|_| SaveError::Damaged)?;
                Ok(cbc_decrypt(|b| cipher.decrypt_block(b), data))
            }
            Method::Aes256 => {
                let cipher = Aes256::new_from_slice(&self.key).map_err(|_| SaveError::Damaged)?;
                Ok(cbc_decrypt(|b| cipher.decrypt_block(b), data))
            }
        }
    }

    fn encrypt(&self, method: Method, num: u32, generation: u16, data: &[u8]) -> Result<Vec<u8>, SaveError> {
        match method {
            Method::None => Ok(data.to_vec()),
            Method::Rc4 => Ok(rc4(&self.object_key(num, generation, false), data)),
            Method::Aes128 => {
                let key = self.object_key(num, generation, true);
                let cipher = Aes128::new_from_slice(&key).map_err(|_| SaveError::Damaged)?;
                cbc_encrypt(|b| cipher.encrypt_block(b), data)
            }
            Method::Aes256 => {
                let cipher = Aes256::new_from_slice(&self.key).map_err(|_| SaveError::Damaged)?;
                cbc_encrypt(|b| cipher.encrypt_block(b), data)
            }
        }
    }

    /// Algorithm 1: the key for one object, from the file key and the object's number.
    fn object_key(&self, num: u32, generation: u16, aes: bool) -> Vec<u8> {
        let mut h = Md5::new();
        h.update(&self.key);
        h.update(num.to_le_bytes().get(..3).unwrap_or_default());
        h.update(generation.to_le_bytes());
        if aes {
            h.update(b"sAlT");
        }
        let digest = h.finalize();
        digest
            .get(..(self.key.len() + 5).min(16))
            .unwrap_or_default()
            .to_vec()
    }
}

/// Password protection to give a file: AES-256 (revision 6, ISO 32000-2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Protection {
    /// Needed to open the file; empty opens it without one.
    pub open_password: String,
    /// Lifts the restrictions below. Empty makes one up that nobody knows, so they can't be
    /// lifted.
    pub owner_password: String,
    pub allow_printing: bool,
    /// Changing pages, comments and form fields.
    pub allow_changes: bool,
    /// Copying text and pictures (screen readers may always read it).
    pub allow_copying: bool,
}

impl Protection {
    /// The permission flags (/P): bits 3 (print) and 12 (print well), 4 (change), 6
    /// (comment), 9 (fill forms), 11 (assemble), 5 (copy); 10 (accessibility) always, and
    /// the reserved bits set.
    pub(crate) fn flags(&self) -> i32 {
        let bit = |n: u32| 1u32 << (n - 1);
        let mut p: u32 = 0xFFFF_F0C0 | bit(10);
        if self.allow_printing {
            p |= bit(3) | bit(12);
        }
        if self.allow_changes {
            p |= bit(4) | bit(6) | bit(9) | bit(11);
        }
        if self.allow_copying {
            p |= bit(5);
        }
        p as i32
    }

    /// The file key and encryption dictionary for a new file, and its /ID array.
    pub(crate) fn seal(&self) -> Result<(Crypt, Vec<u8>, Vec<u8>), SaveError> {
        let random = |n: usize| -> Result<Vec<u8>, SaveError> {
            let mut out = vec![0u8; n];
            getrandom::fill(&mut out).map_err(|e| SaveError::Io(e.to_string()))?;
            Ok(out)
        };
        let limit = |p: &str| p.as_bytes().get(..p.len().min(127)).unwrap_or_default().to_vec();
        let user = limit(&self.open_password);
        let owner = if self.owner_password.is_empty() {
            random(32)?
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
                .into_bytes()
        } else {
            limit(&self.owner_password)
        };
        let key = random(32)?;
        let wrap = |with: &[u8]| -> Result<Vec<u8>, SaveError> {
            // AES-256 in CBC mode with a zero IV and no padding.
            let cipher = Aes256::new_from_slice(with).map_err(|_| SaveError::Damaged)?;
            let mut prev = [0u8; 16];
            let mut out = Vec::with_capacity(32);
            for chunk in key.as_chunks::<16>().0 {
                let mut block = GenericArray::clone_from_slice(chunk);
                block.iter_mut().zip(prev).for_each(|(a, b)| *a ^= b);
                cipher.encrypt_block(&mut block);
                prev.copy_from_slice(&block);
                out.extend_from_slice(&block);
            }
            Ok(out)
        };
        // Algorithms 8 and 9.
        let (uvs, uks) = (random(8)?, random(8)?);
        let mut u = hash(6, &user, &uvs, &[]);
        u.extend_from_slice(&uvs);
        u.extend_from_slice(&uks);
        let ue = wrap(&hash(6, &user, &uks, &[]))?;
        let (ovs, oks) = (random(8)?, random(8)?);
        let mut o = hash(6, &owner, &ovs, &u);
        o.extend_from_slice(&ovs);
        o.extend_from_slice(&oks);
        let oe = wrap(&hash(6, &owner, &oks, &u))?;
        // Algorithm 10: the permissions, sealed so they can be checked.
        let p = self.flags();
        let mut perms = p.to_le_bytes().to_vec();
        perms.extend_from_slice(&[0xFF; 4]);
        perms.extend_from_slice(b"Tadb");
        perms.extend_from_slice(&random(4)?);
        let cipher = Aes256::new_from_slice(&key).map_err(|_| SaveError::Damaged)?;
        let mut block = GenericArray::clone_from_slice(&perms);
        cipher.encrypt_block(&mut block);
        let hex = |b: &[u8]| b.iter().map(|b| format!("{b:02X}")).collect::<String>();
        let dict = format!(
            "<< /Filter /Standard /V 5 /R 6 /Length 256 \
             /CF << /StdCF << /Type /CryptFilter /AuthEvent /DocOpen /CFM /AESV3 /Length 32 >> >> \
             /StmF /StdCF /StrF /StdCF /P {p} /O <{}> /U <{}> /OE <{}> /UE <{}> /Perms <{}> >>",
            hex(&o),
            hex(&u),
            hex(&oe),
            hex(&ue),
            hex(&block)
        );
        let id = hex(&random(16)?);
        let crypt = Crypt {
            key,
            strings: Method::Aes256,
            streams: Method::Aes256,
            p,
            owner: true,
        };
        Ok((crypt, dict.into_bytes(), format!("[<{id}> <{id}>]").into_bytes()))
    }
}

/// Revisions 2 to 4.
struct Legacy<'a> {
    r: i64,
    bytes: usize,
    o: &'a [u8],
    u: &'a [u8],
    p: i32,
    id: &'a [u8],
    encrypt_metadata: bool,
}

impl Legacy<'_> {
    /// The file key for a user password, or failing that an owner password, and whether it
    /// was the owner's.
    fn key(&self, password: &[u8]) -> Option<(Vec<u8>, bool)> {
        let user = padded(password);
        if let Some(key) = self.user_key(&user) {
            return Some((key, false));
        }
        // Algorithm 7: the owner password decrypts /O to the user password.
        let mut digest = Md5::digest(user).to_vec();
        if self.r >= 3 {
            for _ in 0..50 {
                digest = Md5::digest(digest.get(..self.bytes)?).to_vec();
            }
        }
        let owner_key = digest.get(..self.bytes)?;
        let mut recovered = self.o.get(..32)?.to_vec();
        if self.r == 2 {
            recovered = rc4(owner_key, &recovered);
        } else {
            for i in (0..20u8).rev() {
                let k: Vec<u8> = owner_key.iter().map(|b| b ^ i).collect();
                recovered = rc4(&k, &recovered);
            }
        }
        self.user_key(&padded(&recovered)).map(|key| (key, true))
    }

    /// Algorithm 2, checked against /U with algorithm 4 or 5.
    fn user_key(&self, padded: &[u8; 32]) -> Option<Vec<u8>> {
        let mut h = Md5::new();
        h.update(padded);
        h.update(self.o.get(..32)?);
        h.update(self.p.to_le_bytes());
        h.update(self.id);
        if self.r >= 4 && !self.encrypt_metadata {
            h.update([0xFF; 4]);
        }
        let mut digest = h.finalize().to_vec();
        if self.r >= 3 {
            for _ in 0..50 {
                digest = Md5::digest(digest.get(..self.bytes)?).to_vec();
            }
        }
        let key = digest.get(..self.bytes)?.to_vec();
        let matches = if self.r == 2 {
            rc4(&key, &PAD) == self.u.get(..32)?
        } else {
            let mut h = Md5::new();
            h.update(PAD);
            h.update(self.id);
            let mut x = rc4(&key, &h.finalize());
            for i in 1..20u8 {
                let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
                x = rc4(&k, &x);
            }
            x.get(..16) == self.u.get(..16)
        };
        matches.then_some(key)
    }
}

fn padded(password: &[u8]) -> [u8; 32] {
    let mut out = PAD;
    let n = password.len().min(32);
    for (slot, b) in out.iter_mut().zip(password.iter().take(n)) {
        *slot = *b;
    }
    if n < 32 {
        for (slot, b) in out.iter_mut().skip(n).zip(PAD) {
            *slot = b;
        }
    }
    out
}

/// Revisions 5 and 6 (algorithm 2.A): the file key, unwrapped with a user or owner password,
/// and whether it was the owner's.
fn modern_key(r: i64, password: &[u8], o: &[u8], u: &[u8], oe: &[u8], ue: &[u8]) -> Option<(Vec<u8>, bool)> {
    let password = password.get(..password.len().min(127))?;
    let u48 = u.get(..48)?;
    let user_wraps = if hash(r, password, u.get(32..40)?, &[]) == u.get(..32)? {
        true
    } else if hash(r, password, o.get(32..40)?, u48) == o.get(..32)? {
        false
    } else {
        return None;
    };
    let (intermediate, wrapped) = if user_wraps {
        (hash(r, password, u.get(40..48)?, &[]), ue)
    } else {
        (hash(r, password, o.get(40..48)?, u48), oe)
    };
    let cipher = Aes256::new_from_slice(&intermediate).ok()?;
    // AES-256 in CBC mode with a zero IV and no padding.
    let mut prev = [0u8; 16];
    let mut key = Vec::with_capacity(32);
    for chunk in wrapped.get(..32)?.as_chunks::<16>().0 {
        let mut block = GenericArray::clone_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        key.extend(block.iter().zip(prev).map(|(a, b)| a ^ b));
        prev.copy_from_slice(chunk);
    }
    Some((key, !user_wraps))
}

/// Algorithm 2.B (revision 6), or plain SHA-256 for revision 5.
fn hash(r: i64, password: &[u8], salt: &[u8], udata: &[u8]) -> Vec<u8> {
    let mut k = Sha256::new()
        .chain_update(password)
        .chain_update(salt)
        .chain_update(udata)
        .finalize()
        .to_vec();
    if r == 5 {
        return k;
    }
    let mut round: u32 = 0;
    loop {
        let mut k1 = Vec::with_capacity(64 * (password.len() + k.len() + udata.len()));
        for _ in 0..64 {
            k1.extend_from_slice(password);
            k1.extend_from_slice(&k);
            k1.extend_from_slice(udata);
        }
        let (Some(key), Some(iv)) = (k.get(..16), k.get(16..32)) else {
            return k;
        };
        let Ok(cipher) = Aes128::new_from_slice(key) else {
            return k;
        };
        let mut prev = [0u8; 16];
        prev.copy_from_slice(iv);
        let mut e = Vec::with_capacity(k1.len());
        for chunk in k1.as_chunks::<16>().0 {
            let mut block = GenericArray::clone_from_slice(chunk);
            block.iter_mut().zip(prev).for_each(|(a, b)| *a ^= b);
            cipher.encrypt_block(&mut block);
            prev.copy_from_slice(&block);
            e.extend_from_slice(&block);
        }
        let sum: u32 = e.iter().take(16).map(|b| u32::from(*b)).sum();
        k = match sum % 3 {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        round += 1;
        let last = u32::from(e.last().copied().unwrap_or(0));
        if round >= 64 && last + 32 <= round {
            break;
        }
    }
    k.truncate(32);
    k
}

// Every index is a `u8` (or below 256) into a 256-entry table, so none can be out of range.
#[allow(clippy::indexing_slicing)]
fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    if key.is_empty() {
        return data.to_vec();
    }
    let mut j: u8 = 0;
    for i in 0..256 {
        let k = key.get(i % key.len()).copied().unwrap_or(0);
        j = j.wrapping_add(s[i]).wrapping_add(k);
        s.swap(i, usize::from(j));
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[usize::from(i)]);
            s.swap(usize::from(i), usize::from(j));
            let k = s[usize::from(s[usize::from(i)].wrapping_add(s[usize::from(j)]))];
            b ^ k
        })
        .collect()
}

/// AES-CBC with a random IV written first and PKCS#7 padding, as PDF expects.
fn cbc_encrypt(
    encrypt_block: impl Fn(&mut GenericArray<u8, aes::cipher::consts::U16>),
    data: &[u8],
) -> Result<Vec<u8>, SaveError> {
    let mut iv = [0u8; 16];
    getrandom::fill(&mut iv).map_err(|e| SaveError::Io(e.to_string()))?;
    let pad = 16 - data.len() % 16;
    let mut plain = data.to_vec();
    plain.extend(std::iter::repeat_n(pad as u8, pad));
    let mut out = iv.to_vec();
    let mut prev = iv;
    for chunk in plain.as_chunks::<16>().0 {
        let mut block = GenericArray::clone_from_slice(chunk);
        block.iter_mut().zip(prev).for_each(|(a, b)| *a ^= b);
        encrypt_block(&mut block);
        prev.copy_from_slice(&block);
        out.extend_from_slice(&block);
    }
    Ok(out)
}

/// Reverses [`cbc_encrypt`]. Damaged input (too short, or bad padding) gives what can be
/// recovered, as readers do.
fn cbc_decrypt(
    decrypt_block: impl Fn(&mut GenericArray<u8, aes::cipher::consts::U16>),
    data: &[u8],
) -> Vec<u8> {
    let (blocks, _) = data.as_chunks::<16>();
    let Some((iv, blocks)) = blocks.split_first() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(data.len());
    let mut prev = *iv;
    for chunk in blocks {
        let mut block = GenericArray::clone_from_slice(chunk);
        decrypt_block(&mut block);
        block.iter_mut().zip(prev).for_each(|(a, b)| *a ^= b);
        prev = *chunk;
        out.extend_from_slice(&block);
    }
    if let Some(&pad) = out.last()
        && (1..=16).contains(&pad)
        && out.len() >= usize::from(pad)
    {
        out.truncate(out.len() - usize::from(pad));
    }
    out
}

/// The bytes of a literal `( … )` or hex `< … >` string.
pub(crate) fn parse_string(raw: &[u8]) -> Option<Vec<u8>> {
    let raw = raw.trim_ascii();
    match raw.first()? {
        b'<' => {
            let inner = raw.strip_prefix(b"<")?.strip_suffix(b">")?;
            let digits: Vec<u8> = inner
                .iter()
                .filter(|b| !b.is_ascii_whitespace())
                .map(|b| char::from(*b).to_digit(16).map(|d| d as u8))
                .collect::<Option<_>>()?;
            Some(
                digits
                    .chunks(2)
                    .map(|p| (p.first().copied().unwrap_or(0) << 4) | p.get(1).copied().unwrap_or(0))
                    .collect(),
            )
        }
        b'(' => {
            let inner = raw.strip_prefix(b"(")?.strip_suffix(b")")?;
            let mut out = Vec::with_capacity(inner.len());
            let mut it = inner.iter().copied().peekable();
            while let Some(b) = it.next() {
                if b == b'\r' {
                    // An end of line in a literal string is always a line feed.
                    it.next_if_eq(&b'\n');
                    out.push(b'\n');
                    continue;
                }
                if b != b'\\' {
                    out.push(b);
                    continue;
                }
                match it.next() {
                    Some(b'n') => out.push(b'\n'),
                    Some(b'r') => out.push(b'\r'),
                    Some(b't') => out.push(b'\t'),
                    Some(b'b') => out.push(8),
                    Some(b'f') => out.push(12),
                    Some(b'\r') => {
                        it.next_if_eq(&b'\n');
                    }
                    Some(b'\n') | None => {}
                    Some(d @ b'0'..=b'7') => {
                        let mut v = u32::from(d - b'0');
                        for _ in 0..2 {
                            match it.next_if(|c| (b'0'..=b'7').contains(c)) {
                                Some(c) => v = v * 8 + u32::from(c - b'0'),
                                None => break,
                            }
                        }
                        out.push((v & 0xFF) as u8);
                    }
                    Some(other) => out.push(other),
                }
            }
            Some(out)
        }
        _ => None,
    }
}

/// Replaces the number after `/Length` in a stream dictionary.
fn set_length(dict: &[u8], len: usize) -> Option<Vec<u8>> {
    let at = find(dict, b"/Length ")? + b"/Length ".len();
    let digits = dict.get(at..)?.iter().take_while(|b| b.is_ascii_digit()).count();
    let mut out = dict.get(..at)?.to_vec();
    out.extend_from_slice(len.to_string().as_bytes());
    out.extend_from_slice(dict.get(at + digits..)?);
    Some(out)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_matches_the_classic_test_vector() {
        assert_eq!(
            rc4(b"Key", b"Plaintext"),
            [0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]
        );
    }

    /// The encryption dictionary (object 6) of a test fixture.
    fn encrypt_dict(file: &[u8]) -> &[u8] {
        let start = find(file, b"6 0 obj\n").unwrap() + 8;
        let end = find(&file[start..], b"\nendobj").unwrap() + start;
        &file[start..end]
    }

    #[test]
    fn user_and_owner_passwords_give_the_same_key() {
        let id: Vec<u8> = (0..16).collect();
        for file in [
            &include_bytes!("../../tests/fixtures/encrypted-rc4.pdf")[..],
            include_bytes!("../../tests/fixtures/encrypted-aes128.pdf"),
            include_bytes!("../../tests/fixtures/encrypted-aes256.pdf"),
        ] {
            let dict = encrypt_dict(file);
            let user = Crypt::new(dict, &id, "secret").unwrap();
            let owner = Crypt::new(dict, &id, "owner").unwrap();
            assert_eq!(user.key, owner.key);
            assert!(matches!(
                Crypt::new(dict, &id, "wrong"),
                Err(SaveError::Encrypted)
            ));
        }
        // The AES-256 fixture's key is bytes 200 to 231.
        let aes256 = encrypt_dict(include_bytes!("../../tests/fixtures/encrypted-aes256.pdf"));
        let key: Vec<u8> = (200..232).collect();
        assert_eq!(Crypt::new(aes256, &id, "secret").unwrap().key, key);
    }

    #[test]
    fn strings_open_after_sealing() {
        let id: Vec<u8> = (0..16).collect();
        for file in [
            &include_bytes!("../../tests/fixtures/encrypted-rc4.pdf")[..],
            include_bytes!("../../tests/fixtures/encrypted-aes128.pdf"),
            include_bytes!("../../tests/fixtures/encrypted-aes256.pdf"),
        ] {
            let crypt = Crypt::new(encrypt_dict(file), &id, "secret").unwrap();
            for plain in [&b""[..], b"x", b"sixteen bytes!!!", b"Jane Doe (signed)"] {
                let sealed = crypt.encrypt(crypt.strings, 7, 0, plain).unwrap();
                assert_eq!(crypt.open_string(7, 0, &sealed).unwrap(), plain);
            }
        }
    }

    #[test]
    fn strings_are_parsed() {
        assert_eq!(parse_string(b"(a\\(b\\)\\101\\\nc)").unwrap(), b"a(b)Ac");
        assert_eq!(parse_string(b"<4142 4>").unwrap(), b"AB@");
    }

    #[test]
    fn stream_lengths_are_replaced() {
        assert_eq!(
            set_length(b"<< /Length 12 /X 1 >>", 345).unwrap(),
            b"<< /Length 345 /X 1 >>"
        );
    }
}
