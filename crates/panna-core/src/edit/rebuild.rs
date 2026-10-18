//! Writing a whole new file instead of appending to the old one: for page changes (removed
//! pages must not stay recoverable in an earlier revision), extracting pages and combining
//! files.
//!
//! Only objects reachable from the new catalog are written, renumbered from 1, so whatever the
//! new file no longer uses is gone. Strings are decrypted on the way in and, when the result
//! keeps the first file's password, encrypted again on the way out.

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Dict, Object, ObjectIdentifier};

use super::SaveError;
use super::annot::n;
use super::crypt::Crypt;
use super::lex;
use crate::ObjRef;

/// Where one page of the new file comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageSource {
    /// Page `index` of document `doc` (0 is the document being changed), turned clockwise by
    /// `turns` quarter turns.
    Page { doc: usize, index: usize, turns: u8 },
    /// An empty page, in points.
    Blank { width: f32, height: f32 },
}

/// Page attributes a page can take from the page tree above it (ISO 32000-2, 7.7.3.4).
const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// Objects copied from one document at most; guards against reference bombs.
const MAX_OBJECTS: usize = 2_000_000;

pub(crate) struct Source<'a> {
    pub pdf: &'a Pdf,
    /// The bytes `pdf` was parsed from.
    pub bytes: &'a [u8],
    /// For an encrypted file, the key to read its strings.
    pub crypt: Option<Crypt>,
}

/// Encryption for the new file, kept from the first document.
pub(crate) struct Encryption {
    pub crypt: Crypt,
    /// The /Encrypt dictionary, written as it was.
    pub dict: Vec<u8>,
    /// The trailer's /ID array, which the key depends on.
    pub id: Vec<u8>,
}

struct Writer {
    /// Object bodies by number - 1, unencrypted.
    objects: Vec<Option<Vec<u8>>>,
    /// Per source: where each copied object went, or `None` when it is left out.
    maps: Vec<HashMap<ObjRef, Option<u32>>>,
    queue: VecDeque<(usize, ObjRef, u32)>,
}

impl Writer {
    fn alloc(&mut self) -> u32 {
        self.objects.push(None);
        self.objects.len() as u32
    }

    fn set(&mut self, num: u32, body: Vec<u8>) {
        if let Some(slot) = self.objects.get_mut(num as usize - 1) {
            *slot = Some(body);
        }
    }

    /// The new number for `r` in source `doc`, queueing it to be copied the first time.
    fn map(&mut self, doc: usize, r: ObjRef) -> Option<u32> {
        if let Some(known) = self.maps.get(doc).and_then(|m| m.get(&r)) {
            return *known;
        }
        if self.maps.get(doc).is_some_and(|m| m.len() >= MAX_OBJECTS) {
            return None;
        }
        let num = self.alloc();
        if let Some(m) = self.maps.get_mut(doc) {
            m.insert(r, Some(num));
        }
        self.queue.push_back((doc, r, num));
        Some(num)
    }
}

fn id(r: ObjRef) -> ObjectIdentifier {
    ObjectIdentifier::new(i32::try_from(r.number).unwrap_or(-1), i32::from(r.generation))
}

fn obj_ref(id: ObjectIdentifier) -> Option<ObjRef> {
    Some(ObjRef {
        number: u32::try_from(id.obj_number).ok()?,
        generation: u16::try_from(id.gen_number).ok()?,
    })
}

/// Whether `part` is a slice of `whole`. Values read straight from the file still have their
/// strings encrypted; values from a (decrypted) object stream don't.
fn within(part: &[u8], whole: &[u8]) -> bool {
    let (p, w) = (part.as_ptr() as usize, whole.as_ptr() as usize);
    p >= w && p + part.len() <= w + whole.len()
}

/// PDF syntax from `owner` (an object of source `doc`) with references renumbered and strings
/// decrypted.
fn copy_syntax(
    w: &mut Writer,
    sources: &[Source<'_>],
    doc: usize,
    owner: ObjRef,
    data: &[u8],
) -> Result<Vec<u8>, SaveError> {
    let source = sources.get(doc).ok_or(SaveError::Damaged)?;
    let crypt = source.crypt.as_ref().filter(|_| within(data, source.bytes));
    lex::transform(data, &mut |r| w.map(doc, r), &mut |s: &[u8]| match crypt {
        Some(c) => c.open_string(owner.number, owner.generation, s),
        None => Ok(s.to_vec()),
    })
    .ok_or(SaveError::Damaged)?
}

fn name(bytes: &[u8]) -> String {
    let mut out = String::from("/");
    for &b in bytes {
        if (0x21..=0x7e).contains(&b) && !b"#()<>[]{}/%".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "#{b:02X}");
        }
    }
    out
}

/// One copied object's body.
fn copy_object(w: &mut Writer, sources: &[Source<'_>], doc: usize, r: ObjRef) -> Result<Vec<u8>, SaveError> {
    let source = sources.get(doc).ok_or(SaveError::Damaged)?;
    let Some(object) = source.pdf.xref().get::<Object<'_>>(id(r)) else {
        return Ok(b"null".to_vec());
    };
    Ok(match object {
        Object::Stream(stream) => {
            let dict = stream.dict().data();
            let entries = lex::dict_entries(dict).ok_or(SaveError::Damaged)?;
            let mut body = b"<<".to_vec();
            for (key, value) in &entries {
                if *key == b"Length" {
                    continue;
                }
                body.push(b'/');
                body.extend_from_slice(key);
                body.push(b' ');
                let value = dict.get(value.clone()).ok_or(SaveError::Damaged)?;
                body.extend(copy_syntax(w, sources, doc, r, value)?);
                body.push(b'\n');
            }
            let data = stream.raw_data();
            body.extend_from_slice(format!("/Length {}\n>>\nstream\n", data.len()).as_bytes());
            body.extend_from_slice(&data);
            body.extend_from_slice(b"\nendstream");
            body
        }
        Object::Dict(dict) => copy_syntax(w, sources, doc, r, dict.data())?,
        Object::Array(array) => {
            let data = array.data();
            let inner = copy_syntax(w, sources, doc, r, data)?;
            if data.trim_ascii_start().starts_with(b"[") {
                inner
            } else {
                [&b"["[..], &inner, b"]"].concat()
            }
        }
        Object::String(s) => {
            let mut out = String::from("<");
            for b in s.as_bytes() {
                let _ = write!(out, "{b:02X}");
            }
            out.push('>');
            out.into_bytes()
        }
        Object::Name(n) => name(&n).into_bytes(),
        Object::Number(v) => {
            let v = v.as_f64();
            if v.fract() == 0.0 && v.abs() < 1e15 {
                format!("{}", v as i64).into_bytes()
            } else {
                n(v).into_bytes()
            }
        }
        Object::Boolean(b) => if b { "true" } else { "false" }.as_bytes().to_vec(),
        Object::Null(_) => b"null".to_vec(),
    })
}

/// The page dictionaries of a document, and every page tree node above them.
fn page_tree(pdf: &Pdf) -> (Vec<(ObjRef, Dict<'_>)>, Vec<ObjRef>) {
    let mut pages = Vec::new();
    let mut nodes = Vec::new();
    for page in pdf.pages().iter() {
        let raw = page.raw();
        let Some(r) = raw.obj_id().and_then(obj_ref) else {
            continue;
        };
        let mut parent = raw
            .get_ref(b"Parent")
            .and_then(|p| ObjRef::new(p.obj_number, p.gen_number));
        let mut depth = 0;
        while let Some(p) = parent.filter(|p| !nodes.contains(p)) {
            nodes.push(p);
            depth += 1;
            if depth > 64 {
                break;
            }
            parent = pdf
                .xref()
                .get::<Dict<'_>>(id(p))
                .and_then(|d| d.get_ref(b"Parent"))
                .and_then(|p| ObjRef::new(p.obj_number, p.gen_number));
        }
        pages.push((r, raw.clone()));
    }
    (pages, nodes)
}

/// An entry of a page, or of the nearest node above it that has it: the owning object and the
/// value's bytes.
fn inherited<'a>(pdf: &'a Pdf, page: ObjRef, dict: &Dict<'a>, key: &[u8]) -> Option<(ObjRef, &'a [u8])> {
    let mut owner = page;
    let mut current = dict.clone();
    for _ in 0..64 {
        let data = current.data();
        let entries = lex::dict_entries(data)?;
        if let Some((_, value)) = entries.iter().find(|(k, _)| *k == key) {
            return Some((owner, data.get(value.clone())?));
        }
        let parent = entries
            .iter()
            .find(|(k, _)| *k == b"Parent")
            .and_then(|(_, v)| lex::reference(data.get(v.clone())?))?;
        owner = parent;
        current = pdf.xref().get::<Dict<'_>>(id(parent))?;
    }
    None
}

/// Writes a new file with the pages in `plan`, the first source's catalog, document
/// information and everything they use.
pub(crate) fn rebuild(
    sources: &[Source<'_>],
    plan: &[PageSource],
    info: Option<ObjRef>,
    encryption: Option<Encryption>,
) -> Result<Vec<u8>, SaveError> {
    let first = sources.first().ok_or(SaveError::Damaged)?;
    let mut w = Writer {
        objects: Vec::new(),
        maps: vec![HashMap::new(); sources.len()],
        queue: VecDeque::new(),
    };
    let catalog_num = w.alloc();
    let pages_num = w.alloc();

    // Page tree nodes are replaced by one new node; pages left out become null wherever
    // they're referred to (outline entries, links).
    let trees: Vec<_> = sources.iter().map(|s| page_tree(s.pdf)).collect();
    for (doc, (pages, nodes)) in trees.iter().enumerate() {
        let map = w.maps.get_mut(doc).ok_or(SaveError::Damaged)?;
        for node in nodes {
            map.insert(*node, (doc == 0).then_some(pages_num));
        }
        for (r, _) in pages {
            map.insert(*r, None);
        }
    }
    let mut kids = Vec::new();
    let mut page_nums = Vec::new();
    for source in plan {
        let num = w.alloc();
        kids.push(format!("{num} 0 R"));
        page_nums.push(num);
        if let PageSource::Page { doc, index, .. } = source {
            let r = trees
                .get(*doc)
                .and_then(|(pages, _)| pages.get(*index))
                .map(|(r, _)| *r)
                .ok_or(SaveError::NoSuchPage(*index))?;
            // Links and outline entries to the page go to its first copy.
            if let Some(map) = w.maps.get_mut(*doc)
                && map.get(&r).is_some_and(Option::is_none)
            {
                map.insert(r, Some(num));
            }
        }
    }
    for (source, num) in plan.iter().zip(&page_nums) {
        let body = match *source {
            PageSource::Blank { width, height } => format!(
                "<< /Type /Page /Parent {pages_num} 0 R /MediaBox [0 0 {} {}] /Resources << >> >>",
                n(f64::from(width)),
                n(f64::from(height))
            )
            .into_bytes(),
            PageSource::Page { doc, index, turns } => {
                let (pages, _) = trees.get(doc).ok_or(SaveError::Damaged)?;
                let (r, dict) = pages.get(index).ok_or(SaveError::NoSuchPage(index))?;
                let pdf = sources.get(doc).ok_or(SaveError::Damaged)?.pdf;
                let data = dict.data();
                let mut body = b"<<".to_vec();
                for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
                    if key == b"Parent" || INHERITED.contains(&key) {
                        continue;
                    }
                    body.push(b'/');
                    body.extend_from_slice(key);
                    body.push(b' ');
                    let value = data.get(value).ok_or(SaveError::Damaged)?;
                    body.extend(copy_syntax(&mut w, sources, doc, *r, value)?);
                    body.push(b'\n');
                }
                for key in INHERITED {
                    let Some((owner, value)) = inherited(pdf, *r, dict, key) else {
                        continue;
                    };
                    if key == b"Rotate" {
                        continue;
                    }
                    body.push(b'/');
                    body.extend_from_slice(key);
                    body.push(b' ');
                    body.extend(copy_syntax(&mut w, sources, doc, owner, value)?);
                    body.push(b'\n');
                }
                let rotate = inherited(pdf, *r, dict, b"Rotate")
                    .and_then(|(_, v)| std::str::from_utf8(v).ok()?.trim().parse::<f64>().ok())
                    .map_or(0, |v| v as i64);
                let rotate = (rotate + 90 * i64::from(turns)).rem_euclid(360);
                body.extend_from_slice(format!("/Parent {pages_num} 0 R\n/Rotate {rotate}\n>>").as_bytes());
                body
            }
        };
        w.set(*num, body);
    }
    w.set(
        pages_num,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            kids.len()
        )
        .into_bytes(),
    );

    // The catalog, pointing at the new page tree. Page labels count pages by position, so
    // they only stay when the pages do.
    let root = obj_ref(first.pdf.xref().root_id()).ok_or(SaveError::Damaged)?;
    let catalog = first
        .pdf
        .xref()
        .get::<Dict<'_>>(id(root))
        .ok_or(SaveError::Damaged)?;
    let same_pages = plan.len() == trees.first().map_or(0, |(p, _)| p.len())
        && plan
            .iter()
            .enumerate()
            .all(|(i, p)| matches!(p, PageSource::Page { doc: 0, index, turns: _ } if *index == i));
    let data = catalog.data();
    let mut body = b"<<".to_vec();
    for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
        if key == b"Pages" || (key == b"PageLabels" && !same_pages) {
            continue;
        }
        body.push(b'/');
        body.extend_from_slice(key);
        body.push(b' ');
        let value = data.get(value).ok_or(SaveError::Damaged)?;
        body.extend(copy_syntax(&mut w, sources, 0, root, value)?);
        body.push(b'\n');
    }
    body.extend_from_slice(format!("/Pages {pages_num} 0 R\n>>").as_bytes());
    w.set(catalog_num, body);
    let info_num = info.and_then(|r| w.map(0, r));

    while let Some((doc, r, num)) = w.queue.pop_front() {
        let body = copy_object(&mut w, sources, doc, r)?;
        w.set(num, body);
    }

    // Encrypted output: everything but the encryption dictionary itself is sealed.
    let encrypt_num = encryption.as_ref().map(|_| w.alloc());
    let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::with_capacity(w.objects.len());
    for (i, body) in w.objects.iter().enumerate() {
        let num = i as u32 + 1;
        offsets.push(out.len());
        let body = body.as_deref().unwrap_or(b"null");
        let body = match &encryption {
            Some(e) if Some(num) == encrypt_num => e.dict.clone(),
            Some(e) => e.crypt.seal(num, 0, body)?,
            None => body.to_vec(),
        };
        out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    let mut xref = format!("xref\n0 {}\n0000000000 65535 f\r\n", offsets.len() + 1);
    for offset in offsets {
        let _ = write!(xref, "{offset:010} 00000 n\r\n");
    }
    let _ = write!(
        xref,
        "trailer\n<< /Size {} /Root {catalog_num} 0 R ",
        w.objects.len() + 1
    );
    if let Some(info) = info_num {
        let _ = write!(xref, "/Info {info} 0 R ");
    }
    if let (Some(e), Some(num)) = (&encryption, encrypt_num) {
        let _ = write!(xref, "/Encrypt {num} 0 R /ID {} ", String::from_utf8_lossy(&e.id));
    }
    let _ = write!(xref, ">>\nstartxref\n{xref_at}\n%%EOF\n");
    out.extend_from_slice(xref.as_bytes());
    Ok(out)
}
