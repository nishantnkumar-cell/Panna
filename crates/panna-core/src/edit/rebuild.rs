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
use hayro::hayro_syntax::object::{Dict, Object, ObjectIdentifier, Stream};

use super::SaveError;
use super::annot::n;
use super::crypt::Crypt;
use super::lex;
use crate::ObjRef;

/// Where one page of the new file comes from.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
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

    /// A new number for `r` in source `doc` that the caller fills in itself, or `None` when
    /// `r` already has one.
    fn claim(&mut self, doc: usize, r: ObjRef) -> Option<u32> {
        if self.maps.get(doc)?.contains_key(&r) {
            return None;
        }
        let num = self.alloc();
        self.maps.get_mut(doc)?.insert(r, Some(num));
        Some(num)
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

/// Margins to trim from a page as it is shown, in points: top, right, bottom, left.
pub type Margins = [f32; 4];

/// Changes made to one page of the new file on the way.
#[derive(Debug, Clone, Default)]
pub(crate) struct Extra {
    /// Margins to crop.
    pub crop: Option<Margins>,
    /// What to draw behind the page's content, and over it.
    pub under: Vec<super::marks::Drawing>,
    pub over: Vec<super::marks::Drawing>,
    /// Annotations to draw into the page's content and remove.
    pub flatten: Flatten,
    /// Links to add: left, top, right, bottom from the top-left corner of the page as shown.
    pub links: Vec<([f64; 4], crate::LinkTarget)>,
    /// Areas to take content out of for good, in the same space as `links`.
    pub redact: Vec<[f64; 4]>,
    /// Take only text out of the `redact` areas, leaving pictures, drawings and comments.
    pub text_only: bool,
    /// Form fields to add, placed in the same space as `links`.
    pub fields: Vec<super::NewField>,
    /// Widgets to take off the page, and fields to take out of the form.
    pub drop_annots: Vec<ObjRef>,
    pub drop_fields: Vec<ObjRef>,
    /// Pictures the page draws to move, replace or take out, by their place in
    /// [`page_pictures`].
    pub pictures: Vec<(usize, super::PictureChange)>,
    /// Redacted phrases (see [`super::scrub`]) to take out of the whole document: comments
    /// that mention one go, and they are blacked out in bookmark titles. Read from every
    /// page's extra.
    pub scrub: Vec<String>,
}

/// Hidden information to leave out of the new file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cleanup {
    /// The document information (title, author, dates, software) and XMP metadata.
    pub metadata: bool,
    /// JavaScript and actions that run by themselves (on opening, on page views, on fields).
    pub scripts: bool,
    /// Attached files, in the document and on the pages.
    pub attachments: bool,
    /// Comments and markup; links and form fields stay.
    pub comments: bool,
    /// Makes the copy PDF/A-2b where it can: new metadata naming it PDF/A (the document
    /// information goes into it), an sRGB output intent, and comments and fields set to
    /// print. Scripts should go too.
    pub archive: bool,
}

impl Cleanup {
    fn annotations(self) -> bool {
        self.scripts || self.attachments || self.comments
    }
}

/// Whether `annot`'s action (/A) runs JavaScript.
fn runs_script(annot: &Dict<'_>) -> bool {
    use hayro::hayro_syntax::object::Name;
    annot
        .get::<Dict<'_>>(b"A")
        .and_then(|a| a.get::<Name<'_>>(b"S"))
        .is_some_and(|s| s.as_str() == "JavaScript")
}

/// Whether `dict` is a form field (or a field's parent), whose actions run as it's filled in.
fn is_field(dict: &Dict<'_>) -> bool {
    dict.contains_key(b"FT") || (dict.contains_key(b"Kids") && dict.contains_key(b"T"))
}

/// A copied dictionary without its additional actions (scripts that run as a field is
/// typed in, worked out, formatted or checked), nor its action when that's a `script`.
fn without_actions(body: &[u8], script: bool) -> Option<Vec<u8>> {
    if !body.trim_ascii_start().starts_with(b"<<") {
        return None;
    }
    let mut out = b"<<".to_vec();
    for (key, value) in lex::dict_entries(body)? {
        if key == b"AA" || (key == b"A" && script) {
            continue;
        }
        out.push(b'/');
        out.extend_from_slice(key);
        out.push(b' ');
        out.extend_from_slice(body.get(value)?);
        out.push(b'\n');
    }
    out.extend_from_slice(b">>");
    Some(out)
}

/// Which annotations become part of the page's content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Flatten {
    /// Comments, markup, stamps and drawings.
    pub comments: bool,
    /// Form fields.
    pub fields: bool,
}

impl Flatten {
    fn any(self) -> bool {
        self.comments || self.fields
    }

    /// Whether an annotation of `subtype` is flattened. Links stay links, and pop-ups go with
    /// the comments they belong to.
    fn takes(self, subtype: &str) -> bool {
        match subtype {
            "Link" => false,
            "Widget" => self.fields,
            _ => self.comments,
        }
    }
}

/// An annotation drawn into the page: its appearance stream and the matrix placing it.
struct Flattened {
    appearance: ObjRef,
    matrix: [f64; 6],
}

/// Hidden (2) or not shown on screen (32): left out when flattening.
const HIDDEN_FLAGS: i64 = 2 | 32;

/// The annotation `annot` as it would be drawn: its appearance (for its current state) placed
/// on its rectangle (ISO 32000-2, 12.5.5). `None` when it has nothing to draw.
fn appearance(pdf: &Pdf, annot: &Dict<'_>) -> Option<Flattened> {
    use hayro::hayro_syntax::object::{Array, Name, Stream};
    let numbers = |dict: &Dict<'_>, key: &[u8]| -> Option<Vec<f64>> {
        Some(dict.get::<Array<'_>>(key)?.iter::<f64>().take(6).collect())
    };
    let ap = annot.get::<Dict<'_>>(b"AP")?;
    let appearance = match ap.get_ref(b"N") {
        Some(r) => r,
        None => {
            let states = ap.get::<Dict<'_>>(b"N")?;
            let state = annot.get::<Name<'_>>(b"AS")?;
            states.get_ref(state.as_str().as_bytes())?
        }
    };
    let appearance = ObjRef::new(appearance.obj_number, appearance.gen_number)?;
    let form = pdf.xref().get::<Stream<'_>>(id(appearance))?;
    let form = form.dict();
    let [bx0, by0, bx1, by1]: [f64; 4] = numbers(form, b"BBox")?.try_into().ok()?;
    let [a, b, c, d, e, f]: [f64; 6] = numbers(form, b"Matrix")
        .and_then(|m| m.try_into().ok())
        .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    // The box the form's matrix turns its bounding box into.
    let corners =
        [(bx0, by0), (bx1, by0), (bx0, by1), (bx1, by1)].map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
    let xs = corners.map(|(x, _)| x);
    let ys = corners.map(|(_, y)| y);
    let low = |v: [f64; 4]| v.into_iter().fold(f64::INFINITY, f64::min);
    let high = |v: [f64; 4]| v.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let (tx0, tx1, ty0, ty1) = (low(xs), high(xs), low(ys), high(ys));
    let [rx0, ry0, rx1, ry1]: [f64; 4] = numbers(annot, b"Rect")?.get(..4)?.try_into().ok()?;
    let (rx0, rx1, ry0, ry1) = (rx0.min(rx1), rx0.max(rx1), ry0.min(ry1), ry0.max(ry1));
    if tx1 - tx0 <= 0.0 || ty1 - ty0 <= 0.0 || !(tx0 + tx1 + ty0 + ty1).is_finite() {
        return None;
    }
    let (sx, sy) = ((rx1 - rx0) / (tx1 - tx0), (ry1 - ry0) / (ty1 - ty0));
    Some(Flattened {
        appearance,
        matrix: [sx, 0.0, 0.0, sy, rx0 - sx * tx0, ry0 - sy * ty0],
    })
}

/// The smallest a cropped page may be, in points a side.
const MIN_CROPPED: f64 = 18.0;

/// The page's visible box (its crop box within its media box) with `margins` trimmed from
/// the sides as shown on screen, given its `/Rotate`.
fn cropped(media: [f64; 4], crop: Option<[f64; 4]>, rotate: i64, margins: Margins) -> Option<[f64; 4]> {
    let norm = |b: [f64; 4]| [b[0].min(b[2]), b[1].min(b[3]), b[0].max(b[2]), b[1].max(b[3])];
    let media = norm(media);
    let [mut x0, mut y0, mut x1, mut y1] = match crop.map(norm) {
        Some(c) => [
            c[0].max(media[0]),
            c[1].max(media[1]),
            c[2].min(media[2]),
            c[3].min(media[3]),
        ],
        None => media,
    };
    let [top, right, bottom, left] = margins.map(|m| f64::from(m.max(0.0)));
    // Which side of the unturned page each shown side is.
    let (from_x0, from_y0, from_x1, from_y1) = match rotate.rem_euclid(360) {
        90 => (top, left, bottom, right),
        180 => (right, top, left, bottom),
        270 => (bottom, right, top, left),
        _ => (left, bottom, right, top),
    };
    x0 += from_x0;
    y0 += from_y0;
    x1 -= from_x1;
    y1 -= from_y1;
    (x1 - x0 >= MIN_CROPPED && y1 - y0 >= MIN_CROPPED).then_some([x0, y0, x1, y1])
}

/// For the visible box of a page turned by `rotate`, the matrix from a space with its origin at
/// the bottom-left corner as shown and its axes as shown, to the page's own space; and the
/// page's width and height as shown.
fn shown_space([x0, y0, x1, y1]: [f64; 4], rotate: i64) -> ([f64; 6], f64, f64) {
    let (w, h) = (x1 - x0, y1 - y0);
    match rotate.rem_euclid(360) {
        90 => ([0.0, 1.0, -1.0, 0.0, x1, y0], h, w),
        180 => ([-1.0, 0.0, 0.0, -1.0, x1, y1], w, h),
        270 => ([0.0, -1.0, 1.0, 0.0, x0, y1], h, w),
        _ => ([1.0, 0.0, 0.0, 1.0, x0, y0], w, h),
    }
}

/// A page's resources (`owner`'s value, if it has any) as a new dictionary that also names
/// each of `forms` (a name and an object number).
fn with_xobjects<'a>(
    w: &mut Writer,
    sources: &[Source<'a>],
    doc: usize,
    resources: Option<(ObjRef, &'a [u8])>,
    forms: &[(String, u32)],
) -> Result<Vec<u8>, SaveError> {
    let pdf: &'a Pdf = sources.get(doc).ok_or(SaveError::Damaged)?.pdf;
    // A dictionary written in place, or the object it refers to, with the object it's in.
    let resolve = |owner: ObjRef, value: &'a [u8]| -> Option<(ObjRef, &'a [u8])> {
        let (owner, data) = match lex::reference(value) {
            Some(r) => (r, pdf.xref().get::<Dict<'a>>(id(r))?.data()),
            None => (owner, value),
        };
        data.trim_ascii_start()
            .starts_with(b"<<")
            .then_some((owner, data))
    };
    let mut out = b"<<".to_vec();
    let mut xobjects = Vec::new();
    for (name, form) in forms {
        if *form == 0 {
            // Taken out by redaction.
            xobjects.extend_from_slice(format!(" /{name} null").as_bytes());
        } else {
            xobjects.extend_from_slice(format!(" /{name} {form} 0 R").as_bytes());
        }
    }
    let ours = |key: &[u8]| forms.iter().any(|(name, _)| name.as_bytes() == key);
    let copy =
        |w: &mut Writer, out: &mut Vec<u8>, owner: ObjRef, data: &[u8], skip: &dyn Fn(&[u8]) -> bool| {
            for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
                if skip(key) {
                    continue;
                }
                out.extend_from_slice(b" /");
                out.extend_from_slice(key);
                out.push(b' ');
                let value = data.get(value).ok_or(SaveError::Damaged)?;
                out.extend(copy_syntax(w, sources, doc, owner, value)?);
            }
            Ok::<_, SaveError>(())
        };
    if let Some((owner, data)) = resources.and_then(|(o, v)| resolve(o, v)) {
        copy(w, &mut out, owner, data, &|k| k == b"XObject")?;
        let entries = lex::dict_entries(data).ok_or(SaveError::Damaged)?;
        if let Some((sub_owner, sub)) = entries
            .iter()
            .find(|(k, _)| *k == b"XObject")
            .and_then(|(_, v)| resolve(owner, data.get(v.clone())?))
        {
            copy(w, &mut xobjects, sub_owner, sub, &ours)?;
        }
    }
    out.extend_from_slice(b" /XObject <<");
    out.extend(xobjects);
    out.extend_from_slice(b">>\n>>");
    Ok(out)
}

/// The catalog's name dictionary (`value`, written in place or referred to) without the
/// scripts or attached files `clean` leaves out; `None` when nothing else is in it.
fn names_without(
    w: &mut Writer,
    sources: &[Source<'_>],
    root: ObjRef,
    value: &[u8],
    clean: Cleanup,
) -> Result<Option<Vec<u8>>, SaveError> {
    let pdf = sources.first().ok_or(SaveError::Damaged)?.pdf;
    let (owner, data) = match lex::reference(value) {
        Some(r) => match pdf.xref().get::<Dict<'_>>(id(r)) {
            Some(d) => (r, d.data()),
            None => return Ok(None),
        },
        None => (root, value),
    };
    let mut out = b"<<".to_vec();
    let mut any = false;
    for (key, v) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
        if (clean.scripts && key == b"JavaScript") || (clean.attachments && key == b"EmbeddedFiles") {
            continue;
        }
        any = true;
        out.extend_from_slice(b" /");
        out.extend_from_slice(key);
        out.push(b' ');
        let v = data.get(v).ok_or(SaveError::Damaged)?;
        out.extend(copy_syntax(w, sources, 0, owner, v)?);
    }
    out.extend_from_slice(b" >>");
    Ok(any.then_some(out))
}

/// The four numbers of a rectangle written as an array.
fn rect(value: &[u8]) -> Option<[f64; 4]> {
    let inner = value.trim_ascii().strip_prefix(b"[")?.strip_suffix(b"]")?;
    let numbers: Vec<f64> = std::str::from_utf8(inner)
        .ok()?
        .split_ascii_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    numbers.try_into().ok()
}

/// Writes a new file with the pages in `plan`, the first source's catalog, document
/// information and everything they use. Pages are changed as `extras` says (by place in
/// `plan`; it may be shorter).
pub(crate) fn rebuild(
    sources: &[Source<'_>],
    plan: &[PageSource],
    extras: &[Extra],
    info: Option<ObjRef>,
    encryption: Option<Encryption>,
    clean: Cleanup,
    compact: Option<super::pack::Compact>,
) -> Result<Vec<u8>, SaveError> {
    let first = sources.first().ok_or(SaveError::Damaged)?;
    let info = info.filter(|_| !clean.metadata && !clean.archive);
    let mut scrub: Vec<String> = Vec::new();
    for phrase in extras.iter().flat_map(|e| &e.scrub) {
        if !scrub.contains(phrase) {
            scrub.push(phrase.clone());
        }
    }
    // Pictures already written, by StampImage::key.
    let mut pictures_written: HashMap<usize, u32> = HashMap::new();
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
    // Form fields added to pages, for the form's field list.
    let mut new_fields: Vec<u32> = Vec::new();
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
    for (place, (source, num)) in plan.iter().zip(&page_nums).enumerate() {
        let extra = extras.get(place).cloned().unwrap_or_default();
        let crop = extra.crop;
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
                let mut contents = None;
                let rotate = inherited(pdf, *r, dict, b"Rotate")
                    .and_then(|(_, v)| std::str::from_utf8(v).ok()?.trim().parse::<f64>().ok())
                    .map_or(0, |v| v as i64);
                let rotate = (rotate + 90 * i64::from(turns)).rem_euclid(360);
                let boxed = |key: &[u8]| inherited(pdf, *r, dict, key).and_then(|(_, v)| rect(v));
                let media = boxed(b"MediaBox").unwrap_or([0.0, 0.0, 612.0, 792.0]);
                let shown = match crop {
                    Some(margins) => {
                        let [x0, y0, x1, y1] = cropped(media, boxed(b"CropBox"), rotate, margins)
                            .ok_or(SaveError::CropTooLarge)?;
                        body.extend_from_slice(
                            format!("/CropBox [{} {} {} {}]\n", n(x0), n(y0), n(x1), n(y1)).as_bytes(),
                        );
                        [x0, y0, x1, y1]
                    }
                    None => cropped(media, boxed(b"CropBox"), rotate, [0.0; 4]).unwrap_or(media),
                };
                // Marked areas in the page's own space.
                let areas: Vec<[f64; 4]> = {
                    let (matrix, _, height) = shown_space(shown, rotate);
                    let [a, b, c, d, e, f] = matrix;
                    let user = |x: f64, y: f64| (a * x + c * (height - y) + e, b * x + d * (height - y) + f);
                    extra
                        .redact
                        .iter()
                        .map(|&[x0, top, x1, bottom]| {
                            let (ux0, uy0) = user(x0, top);
                            let (ux1, uy1) = user(x1, bottom);
                            [ux0.min(ux1), uy0.min(uy1), ux0.max(ux1), uy0.max(uy1)]
                        })
                        .collect()
                };
                let redacting = !areas.is_empty();
                // Annotations drawn into the page, and the ones left as they are.
                let mut flattened = Vec::new();
                let mut kept = String::new();
                let filtering = extra.flatten.any()
                    || clean.annotations()
                    || !extra.links.is_empty()
                    || !extra.fields.is_empty()
                    || !extra.drop_annots.is_empty()
                    || redacting
                    || !scrub.is_empty();
                let mut gone = if redacting && !extra.text_only {
                    redacted_annotations(dict, &areas)?
                } else {
                    std::collections::HashSet::new()
                };
                if !scrub.is_empty() {
                    gone.extend(mentioning_annotations(dict, &scrub));
                }
                if filtering
                    && let Some(annots) = dict.get::<hayro::hayro_syntax::object::Array<'_>>(b"Annots")
                {
                    for annot in annots.iter::<Dict<'_>>().take(20_000) {
                        let Some(r) = annot.obj_id().and_then(obj_ref) else {
                            continue;
                        };
                        let subtype = annot
                            .get::<hayro::hayro_syntax::object::Name<'_>>(b"Subtype")
                            .map(|n| n.as_str().to_owned())
                            .unwrap_or_default();
                        if subtype == "Popup" && (extra.flatten.comments || clean.comments) {
                            continue;
                        }
                        if gone.contains(&r) || extra.drop_annots.contains(&r) {
                            continue;
                        }
                        let script = runs_script(&annot);
                        if (clean.comments && subtype != "Link" && subtype != "Widget")
                            || (clean.attachments && subtype == "FileAttachment")
                            || (clean.scripts && subtype == "Link" && script)
                        {
                            continue;
                        }
                        if extra.flatten.takes(&subtype) {
                            let hidden = annot.get::<i64>(b"F").unwrap_or(0) & HIDDEN_FLAGS != 0;
                            if hidden {
                                continue;
                            }
                            // Without an appearance there's nothing to draw; it stays.
                            if let Some(f) = appearance(pdf, &annot) {
                                flattened.push(f);
                                continue;
                            }
                        }
                        let has_actions = script || annot.get::<Dict<'_>>(b"AA").is_some();
                        let flags = annot.get::<i64>(b"F").unwrap_or(0);
                        // PDF/A wants every comment and field printed, none hidden.
                        let printed = (flags | 4) & !(1 | 2 | 32);
                        let reflag = clean.archive && subtype != "Popup" && flags != printed;
                        if (clean.scripts && has_actions) || reflag {
                            // A copy without its scripts, under the number everything that
                            // refers to it will use.
                            if let Some(num) = w.claim(doc, r) {
                                let data = annot.data();
                                let mut body = b"<<".to_vec();
                                for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
                                    if (clean.scripts && (key == b"AA" || (key == b"A" && script)))
                                        || (reflag && key == b"F")
                                    {
                                        continue;
                                    }
                                    body.push(b'/');
                                    body.extend_from_slice(key);
                                    body.push(b' ');
                                    let value = data.get(value).ok_or(SaveError::Damaged)?;
                                    body.extend(copy_syntax(&mut w, sources, doc, r, value)?);
                                    body.push(b'\n');
                                }
                                if reflag {
                                    body.extend_from_slice(format!("/F {printed}\n").as_bytes());
                                }
                                body.extend_from_slice(b">>");
                                w.set(num, body);
                                kept.push_str(&format!("{num} 0 R "));
                                continue;
                            }
                        }
                        if let Some(num) = w.map(doc, r) {
                            kept.push_str(&format!("{num} 0 R "));
                        }
                    }
                }
                let editing_pictures = !extra.pictures.is_empty();
                let drawn = !extra.under.is_empty()
                    || !extra.over.is_empty()
                    || !flattened.is_empty()
                    || redacting
                    || editing_pictures;
                for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
                    if key == b"Parent" || INHERITED.contains(&key) {
                        continue;
                    }
                    if (key == b"Annots" && filtering) || (key == b"AA" && clean.scripts) {
                        continue;
                    }
                    // A redacted page's thumbnail picture would show what was taken out.
                    if (redacting || editing_pictures) && (key == b"Contents" || key == b"Thumb") {
                        continue;
                    }
                    if key == b"Contents" && drawn {
                        let value = data.get(value).ok_or(SaveError::Damaged)?;
                        contents = Some(copy_syntax(&mut w, sources, doc, *r, value)?);
                        continue;
                    }
                    body.push(b'/');
                    body.extend_from_slice(key);
                    body.push(b' ');
                    let value = data.get(value).ok_or(SaveError::Damaged)?;
                    body.extend(copy_syntax(&mut w, sources, doc, *r, value)?);
                    body.push(b'\n');
                }
                let mut resources = None;
                for key in INHERITED {
                    let Some((owner, value)) = inherited(pdf, *r, dict, key) else {
                        continue;
                    };
                    if key == b"Resources" && drawn {
                        resources = Some((owner, value));
                        continue;
                    }
                    if key == b"Rotate" || (key == b"CropBox" && crop.is_some()) {
                        continue;
                    }
                    body.push(b'/');
                    body.extend_from_slice(key);
                    body.push(b' ');
                    body.extend(copy_syntax(&mut w, sources, doc, owner, value)?);
                    body.push(b'\n');
                }
                if !extra.links.is_empty() {
                    let (matrix, _, height) = shown_space(shown, rotate);
                    let [a, b, c, d, e, f] = matrix;
                    let user = |x: f64, y: f64| (a * x + c * (height - y) + e, b * x + d * (height - y) + f);
                    for ([x0, top, x1, bottom], target) in &extra.links {
                        let action = match target {
                            crate::LinkTarget::Uri(uri) if crate::safe_uri(uri) => {
                                format!(
                                    "/A << /S /URI /URI ({}) >>",
                                    super::annot::literal(uri.as_bytes())
                                )
                            }
                            crate::LinkTarget::Page(i) => match page_nums.get(*i) {
                                Some(num) => format!("/Dest [{num} 0 R /Fit]"),
                                None => continue,
                            },
                            crate::LinkTarget::Uri(_) => continue,
                        };
                        let (ux0, uy0) = user(*x0, *top);
                        let (ux1, uy1) = user(*x1, *bottom);
                        let link = w.alloc();
                        w.set(
                            link,
                            format!(
                                "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 0] /F 4 /P {num} 0 R {action} >>",
                                n(ux0.min(ux1)),
                                n(uy0.min(uy1)),
                                n(ux0.max(ux1)),
                                n(uy0.max(uy1)),
                            )
                            .into_bytes(),
                        );
                        kept.push_str(&format!("{link} 0 R "));
                    }
                }
                if !extra.fields.is_empty() {
                    let (matrix, _, height) = shown_space(shown, rotate);
                    let [a, b, c, d, e, f] = matrix;
                    let user = |x: f64, y: f64| (a * x + c * (height - y) + e, b * x + d * (height - y) + f);
                    let placed = |r: crate::TextBox| {
                        let (ux0, uy0) = user(f64::from(r.x0), f64::from(r.y0));
                        let (ux1, uy1) = user(f64::from(r.x1), f64::from(r.y1));
                        [ux0.min(ux1), uy0.min(uy1), ux0.max(ux1), uy0.max(uy1)]
                    };
                    for field in &extra.fields {
                        if let super::NewFieldKind::Radio(buttons) = &field.kind {
                            let group = w.alloc();
                            let mut kids = Vec::new();
                            for button in buttons {
                                let mut store = |body: Vec<u8>| {
                                    let at = w.alloc();
                                    w.set(at, body);
                                    at
                                };
                                let body = super::new_field::radio_widget(
                                    button,
                                    placed(button.rect),
                                    group,
                                    *num,
                                    rotate,
                                    &mut store,
                                );
                                let widget = w.alloc();
                                w.set(widget, body);
                                kids.push(widget);
                                kept.push_str(&format!("{widget} 0 R "));
                            }
                            w.set(group, super::new_field::radio_group(field, &kids));
                            new_fields.push(group);
                            continue;
                        }
                        let rect = placed(field.rect);
                        let mut store = |body: Vec<u8>| {
                            let at = w.alloc();
                            w.set(at, body);
                            at
                        };
                        let body = super::new_field::widget(field, rect, *num, rotate, &mut store);
                        let widget = w.alloc();
                        w.set(widget, body);
                        new_fields.push(widget);
                        kept.push_str(&format!("{widget} 0 R "));
                    }
                }
                if filtering && !kept.is_empty() {
                    body.extend_from_slice(format!("/Annots [{}]\n", kept.trim_end()).as_bytes());
                }
                if drawn {
                    let (matrix, width, height) = shown_space(shown, rotate);
                    let stream = |dict: &str, data: &str| {
                        format!("<< {dict}/Length {} >>\nstream\n{data}\nendstream", data.len()).into_bytes()
                    };
                    let mut forms = Vec::new();
                    let mut draw = |w: &mut Writer,
                                    forms: &mut Vec<(String, u32)>,
                                    drawings: &[super::marks::Drawing]| {
                        let mut ops = String::new();
                        for drawing in drawings {
                            let resources = match drawing.picture() {
                                Some(image) => {
                                    // A picture on many pages, such as a watermark, is written once.
                                    let num = match pictures_written.get(&image.key()) {
                                        Some(&num) => num,
                                        None => {
                                            let mask = image.has_mask().then(|| w.alloc());
                                            let (body, mask_body) =
                                                image.objects(mask.map(|m| format!("{m} 0 R")).as_deref());
                                            let num = w.alloc();
                                            w.set(num, body);
                                            if let (Some(m), Some(b)) = (mask, mask_body) {
                                                w.set(m, b);
                                            }
                                            pictures_written.insert(image.key(), num);
                                            num
                                        }
                                    };
                                    match drawing.opacity() {
                                        Some(a) => format!(
                                            "<< /XObject << /PaIm {num} 0 R >> /ExtGState << /PaGS << /Type \
                                             /ExtGState /CA {a} /ca {a} >> >> >>",
                                            a = n(a)
                                        ),
                                        None => format!("<< /XObject << /PaIm {num} 0 R >> >>"),
                                    }
                                }
                                None => match drawing.font().and_then(|r| w.map(doc, r)) {
                                    Some(num) => format!("<< /Font << /PaFo {num} 0 R >> >>"),
                                    None => drawing.resources(),
                                },
                            };
                            let form = w.alloc();
                            w.set(
                                form,
                                stream(
                                    &format!(
                                        "/Type /XObject /Subtype /Form /BBox [0 0 {} {}] /Matrix [{}] /Resources {} ",
                                        n(width),
                                        n(height),
                                        matrix.map(n).join(" "),
                                        resources
                                    ),
                                    &drawing.content(width, height),
                                ),
                            );
                            let name = format!("PannaMarks{form}");
                            ops.push_str(&format!("/{name} Do "));
                            forms.push((name, form));
                        }
                        ops
                    };
                    if redacting {
                        let res = resources.and_then(|(owner, raw)| resolved(pdf, owner, raw));
                        let content = page_content(dict);
                        let mut counter = 0;
                        let (content, overrides) = redact_into(
                            &mut w,
                            sources,
                            doc,
                            res,
                            &content,
                            &areas,
                            super::redact::IDENTITY,
                            0,
                            &mut counter,
                            extra.text_only,
                        )?;
                        let num = w.alloc();
                        w.set(num, flate_stream("", &content));
                        contents = Some(format!("{num} 0 R").into_bytes());
                        forms.extend(overrides);
                    }
                    if editing_pictures && !redacting {
                        let res = resources.and_then(|(owner, raw)| resolved(pdf, owner, raw));
                        let content = page_content(dict);
                        let mut lookup = super::redact::Resources { dict: res };
                        let placed = super::redact::placed_images(&content, &mut lookup);
                        let (inv, _, height) = shown_space(shown, rotate);
                        let user = |r: &crate::TextBox| {
                            let [a, b, c, d, e, f] = inv;
                            let p = |x: f32, y: f32| {
                                let (x, y) = (f64::from(x), height - f64::from(y));
                                (a * x + c * y + e, b * x + d * y + f)
                            };
                            let (ux0, uy0) = p(r.x0, r.y0);
                            let (ux1, uy1) = p(r.x1, r.y1);
                            [ux0.min(ux1), uy0.min(uy1), ux0.max(ux1), uy0.max(uy1)]
                        };
                        let mut out = Vec::new();
                        let mut at = 0;
                        for (i, place) in placed.iter().enumerate() {
                            let Some((_, change)) = extra.pictures.iter().rev().find(|(p, _)| *p == i) else {
                                continue;
                            };
                            let Some(raw) = content.get(place.range.clone()) else {
                                continue;
                            };
                            out.extend_from_slice(content.get(at..place.range.start).unwrap_or_default());
                            at = place.range.end;
                            let from = place.bounds();
                            let wrap = |to: [f64; 4], draw: &[u8]| -> Vec<u8> {
                                match super::redact::moved(place.ctm, from, to) {
                                    Some(m) => {
                                        let mut op = format!("q {} cm ", m.map(n).join(" ")).into_bytes();
                                        op.extend_from_slice(draw);
                                        op.extend_from_slice(b" Q");
                                        op
                                    }
                                    None => draw.to_vec(),
                                }
                            };
                            match change {
                                super::PictureChange::Delete => {}
                                super::PictureChange::Move(rect) => out.extend(wrap(user(rect), raw)),
                                super::PictureChange::Replace(image) => {
                                    let mask = image.has_mask().then(|| w.alloc());
                                    let (body, mask_body) =
                                        image.objects(mask.map(|m| format!("{m} 0 R")).as_deref());
                                    let num = w.alloc();
                                    w.set(num, body);
                                    if let (Some(m), Some(b)) = (mask, mask_body) {
                                        w.set(m, b);
                                    }
                                    let name = format!("PannaPicture{num}");
                                    forms.push((name.clone(), num));
                                    // The new picture keeps its own shape, centred where the
                                    // old one was and upright as shown, whatever turned the
                                    // old one. Worked out in shown space, bottom up.
                                    let Some(shown_of) = super::redact::invert(inv) else {
                                        continue;
                                    };
                                    let a = super::redact::apply(shown_of, from[0], from[1]);
                                    let b = super::redact::apply(shown_of, from[2], from[3]);
                                    let (sx0, sy0) = (a.0.min(b.0), a.1.min(b.1));
                                    let (bw, bh) = ((a.0 - b.0).abs(), (a.1 - b.1).abs());
                                    let aspect = f64::from(image.width) / f64::from(image.height.max(1));
                                    let (pw, ph) = if bw / bh.max(1e-9) > aspect {
                                        (bh * aspect, bh)
                                    } else {
                                        (bw, bw / aspect)
                                    };
                                    let place_in_shown =
                                        [pw, 0.0, 0.0, ph, sx0 + (bw - pw) / 2.0, sy0 + (bh - ph) / 2.0];
                                    let wanted = super::redact::concat(place_in_shown, inv);
                                    // Drawn where the old one was, so its own placing is undone.
                                    let Some(undo) = super::redact::invert(place.ctm) else {
                                        continue;
                                    };
                                    let m = super::redact::concat(wanted, undo);
                                    out.extend_from_slice(
                                        format!("q {} cm /{name} Do Q", m.map(n).join(" ")).as_bytes(),
                                    );
                                    // The old picture isn't drawn any more; its operation goes.
                                    let _ = raw;
                                }
                            }
                        }
                        out.extend_from_slice(content.get(at..).unwrap_or_default());
                        let num = w.alloc();
                        w.set(num, flate_stream("", &out));
                        contents = Some(format!("{num} 0 R").into_bytes());
                    }
                    // The page's own drawing is kept apart in q ... Q, so whatever state it
                    // leaves behind doesn't move what's drawn over it.
                    let under = draw(&mut w, &mut forms, &extra.under);
                    let mut over = String::new();
                    for f in &flattened {
                        let Some(num) = w.map(doc, f.appearance) else {
                            continue;
                        };
                        let name = format!("PannaAnnot{num}");
                        over.push_str(&format!("q {} cm /{name} Do Q ", f.matrix.map(n).join(" ")));
                        forms.push((name, num));
                    }
                    over.push_str(&draw(&mut w, &mut forms, &extra.over));
                    let (before, after) = (w.alloc(), w.alloc());
                    w.set(before, stream("", &format!("{under}q")));
                    w.set(after, stream("", &format!("Q {over}")));
                    let inner = contents.unwrap_or_default();
                    let inner = inner.trim_ascii();
                    let inner = inner
                        .strip_prefix(b"[")
                        .and_then(|i| i.strip_suffix(b"]"))
                        .unwrap_or(inner);
                    body.extend_from_slice(format!("/Contents [{before} 0 R ").as_bytes());
                    body.extend_from_slice(inner);
                    body.extend_from_slice(format!(" {after} 0 R]\n/Resources ").as_bytes());
                    body.extend(with_xobjects(&mut w, sources, doc, resources, &forms)?);
                    body.push(b'\n');
                }
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
    let dropped_fields: Vec<ObjRef> = extras
        .iter()
        .flat_map(|e| e.drop_fields.iter().copied())
        .collect();
    let mut wrote_form = false;
    let mut body = b"<<".to_vec();
    for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
        // Flattened form fields are page content now, so the form goes.
        let flattened_form = key == b"AcroForm" && extras.iter().any(|e| e.flatten.fields);
        if key == b"Pages" || (key == b"PageLabels" && !same_pages) || flattened_form {
            continue;
        }
        if key == b"AcroForm" && (!new_fields.is_empty() || !dropped_fields.is_empty()) {
            let raw = data.get(value.clone()).ok_or(SaveError::Damaged)?;
            let form = match lex::reference(raw) {
                Some(r) => first.pdf.xref().get::<Dict<'_>>(id(r)),
                None => catalog.get::<Dict<'_>>(b"AcroForm"),
            };
            if let Some(form) = form {
                let body_form = form_with_fields(&mut w, sources, root, &form, &new_fields, &dropped_fields)?;
                body.extend_from_slice(b"/AcroForm ");
                body.extend(body_form);
                body.push(b'\n');
                wrote_form = true;
                continue;
            }
        }
        let value = data.get(value.clone()).ok_or(SaveError::Damaged)?;
        if ((clean.metadata || clean.archive) && key == b"Metadata")
            || (clean.archive && key == b"OutputIntents")
            || (clean.scripts && key == b"AA")
            // An open action that is a destination only turns to a page; any other runs an action.
            || (clean.scripts && key == b"OpenAction" && !value.trim_ascii_start().starts_with(b"["))
        {
            continue;
        }
        if key == b"Names" && (clean.scripts || clean.attachments) {
            let names = names_without(&mut w, sources, root, value, clean)?;
            if let Some(names) = names {
                body.extend_from_slice(b"/Names ");
                body.extend(names);
                body.push(b'\n');
            }
            continue;
        }
        body.push(b'/');
        body.extend_from_slice(key);
        body.push(b' ');
        body.extend(copy_syntax(&mut w, sources, 0, root, value)?);
        body.push(b'\n');
    }
    if !wrote_form && !new_fields.is_empty() {
        let fields: Vec<String> = new_fields.iter().map(|f| format!("{f} 0 R")).collect();
        body.extend_from_slice(
            format!(
                "/AcroForm << /Fields [{}] /NeedAppearances true /DA (/Helv 0 Tf 0 g) /DR {} >>\n",
                fields.join(" "),
                super::new_field::RESOURCES
            )
            .as_bytes(),
        );
    }
    if clean.archive {
        let meta = first.pdf.metadata();
        let text = |v: &Option<Vec<u8>>| v.as_deref().map(crate::document::decode_text_string);
        let xmp = super::pdfa::xmp(text(&meta.title).as_deref(), text(&meta.author).as_deref());
        let meta_num = w.alloc();
        let mut stream = format!(
            "<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n",
            xmp.len()
        )
        .into_bytes();
        stream.extend(xmp);
        stream.extend_from_slice(b"\nendstream");
        w.set(meta_num, stream);
        let profile = super::pdfa::srgb_profile();
        let profile_num = w.alloc();
        w.set(profile_num, flate_stream("/N 3", &profile));
        body.extend_from_slice(
            format!(
                "/Metadata {meta_num} 0 R\n/OutputIntents [<< /Type /OutputIntent /S /GTS_PDFA1 \
                 /OutputConditionIdentifier (sRGB IEC61966-2.1) /Info (sRGB IEC61966-2.1) \
                 /DestOutputProfile {profile_num} 0 R >>]\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("/Pages {pages_num} 0 R\n>>").as_bytes());
    w.set(catalog_num, body);
    let info_num = info.and_then(|r| w.map(0, r));

    while let Some((doc, r, num)) = w.queue.pop_front() {
        let mut body = copy_object(&mut w, sources, doc, r)?;
        if clean.scripts
            && let Some(field) = sources
                .get(doc)
                .and_then(|s| s.pdf.xref().get::<Dict<'_>>(id(r)))
                .filter(is_field)
        {
            body = without_actions(&body, runs_script(&field)).unwrap_or(body);
        }
        w.set(num, body);
    }
    if !scrub.is_empty() {
        retitle_bookmarks(&mut w, first.pdf, &scrub);
    }

    if let Some(compact) = compact {
        if let Some(dpi) = compact.pictures {
            let map = w.maps.first().ok_or(SaveError::Damaged)?;
            for (r, drawn) in drawn_pictures(first.pdf) {
                let (Some(Some(num)), Some(stream)) =
                    (map.get(&r), first.pdf.xref().get::<Stream<'_>>(id(r)))
                else {
                    continue;
                };
                let Some(slot) = w.objects.get_mut(*num as usize - 1) else {
                    continue;
                };
                if let Some(smaller) = slot
                    .as_deref()
                    .and_then(|body| super::pack::shrink_picture(&stream, body, drawn, dpi))
                {
                    *slot = Some(smaller);
                }
            }
        }
        super::pack::merge_duplicates(&mut w.objects, &[catalog_num, pages_num]);
        super::pack::compress_streams(&mut w.objects);
    }
    write_file(
        w.objects,
        catalog_num,
        info_num,
        encryption.as_ref(),
        compact.is_some(),
    )
}

/// A resources dictionary written in `owner` or referred to from it.
fn resolved<'a>(pdf: &'a Pdf, owner: ObjRef, raw: &'a [u8]) -> Option<Dict<'a>> {
    match lex::reference(raw) {
        Some(r) => pdf.xref().get::<Dict<'a>>(id(r)),
        None => pdf
            .xref()
            .get::<Dict<'a>>(id(owner))?
            .get::<Dict<'a>>(b"Resources"),
    }
}

/// The form dictionary with `added` fields at the end of its field list, `dropped` ones left
/// out, and viewers asked to redraw what fields show.
fn form_with_fields(
    w: &mut Writer,
    sources: &[Source<'_>],
    root: ObjRef,
    form: &Dict<'_>,
    added: &[u32],
    dropped: &[ObjRef],
) -> Result<Vec<u8>, SaveError> {
    use hayro::hayro_syntax::object::Array;
    let data = form.data();
    let mut body = b"<<".to_vec();
    let mut has_dr = false;
    for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
        if key == b"Fields" || key == b"NeedAppearances" {
            continue;
        }
        has_dr |= key == b"DR";
        body.push(b'/');
        body.extend_from_slice(key);
        body.push(b' ');
        let value = data.get(value).ok_or(SaveError::Damaged)?;
        body.extend(copy_syntax(w, sources, 0, root, value)?);
        body.push(b'\n');
    }
    let mut fields = Vec::new();
    if let Some(list) = form.get::<Array<'_>>(b"Fields") {
        for field in list.iter::<Dict<'_>>().take(100_000) {
            let Some(r) = field.obj_id().and_then(obj_ref) else {
                continue;
            };
            if dropped.contains(&r) {
                continue;
            }
            if let Some(num) = w.map(0, r) {
                fields.push(format!("{num} 0 R"));
            }
        }
    }
    fields.extend(added.iter().map(|f| format!("{f} 0 R")));
    if !has_dr {
        body.extend_from_slice(format!("/DR {}\n", super::new_field::RESOURCES).as_bytes());
    }
    body.extend_from_slice(format!("/Fields [{}]\n/NeedAppearances true\n>>", fields.join(" ")).as_bytes());
    Ok(body)
}

/// The largest size, in points, each picture is drawn at on `pdf`'s pages, by object: drawn
/// by the page itself or inside the forms it draws. Pictures drawn only by comments, patterns
/// or text aren't included.
fn drawn_pictures(pdf: &Pdf) -> HashMap<ObjRef, (f64, f64)> {
    let mut out = HashMap::new();
    let mut budget = 10_000usize;
    let (pages, _) = page_tree(pdf);
    for (r, dict) in &pages {
        let res = inherited(pdf, *r, dict, b"Resources").and_then(|(owner, raw)| resolved(pdf, owner, raw));
        drawn_in(
            &page_content(dict),
            res,
            super::redact::IDENTITY,
            0,
            &mut budget,
            &mut out,
        );
    }
    out
}

/// Adds the pictures `content` draws with `ctm` to `out`, looking into forms up to a few
/// levels deep, and at most `budget` forms in all.
fn drawn_in(
    content: &[u8],
    res: Option<Dict<'_>>,
    ctm: super::redact::Matrix,
    depth: usize,
    budget: &mut usize,
    out: &mut HashMap<ObjRef, (f64, f64)>,
) {
    use super::redact::{Lookup as _, XObject, concat};
    let mut lookup = super::redact::Resources { dict: res.clone() };
    for placed in super::redact::placed_images(content, &mut lookup) {
        let Some((_, Some(r))) = lookup.stream(&placed.name) else {
            continue;
        };
        let m = concat(placed.ctm, ctm);
        let size = out.entry(r).or_insert((0.0, 0.0));
        size.0 = size.0.max(m[0].hypot(m[1]));
        size.1 = size.1.max(m[2].hypot(m[3]));
    }
    if depth >= 4 {
        return;
    }
    for placed in super::redact::placed_forms(content, &mut lookup) {
        let (Some((stream, _)), Some(XObject::Form { matrix, .. })) =
            (lookup.stream(&placed.name), lookup.xobject(&placed.name))
        else {
            continue;
        };
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let Ok(inner) = stream.decoded() else { continue };
        // Old files leave a form's resources out, meaning the page's.
        let inner_res = stream
            .dict()
            .get::<Dict<'_>>(b"Resources")
            .or_else(|| res.clone());
        let at = concat(matrix, concat(placed.ctm, ctm));
        drawn_in(&inner, inner_res, at, depth + 1, budget, out);
    }
}

/// New text written in the font a line was in, where it was.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SameFont {
    /// The font's object.
    pub font: ObjRef,
    /// The new text, in the font's character codes.
    pub bytes: Vec<u8>,
    /// The size and sideways stretch (100 is none) on the page as shown.
    pub size: f64,
    pub stretch: f64,
    /// Where the first glyph's origin was, from the bottom-left corner of the page as shown.
    pub origin: (f64, f64),
}

/// How to write `text` in place of the line in `area` (a page text box) on page `index`, in
/// the font most of that line is in: when the line is upright and every character of `text`
/// is one the font can show. Fonts embedded as subsets can only show the characters the page
/// already uses them for.
pub(crate) fn same_font(pdf: &Pdf, index: usize, area: [f64; 4], text: &str) -> Option<SameFont> {
    use super::redact::{IDENTITY, apply, concat, invert};
    let (content, res, to_shown, height) = shown_page(pdf, index)?;
    let to_user = invert(to_shown)?;
    let [x0, top, x1, bottom] = area;
    let (ux0, uy0) = apply(to_user, x0, height - top);
    let (ux1, uy1) = apply(to_user, x1, height - bottom);
    let areas = [[ux0.min(ux1), uy0.min(uy1), ux0.max(ux1), uy0.max(uy1)]];
    let mut lookup = super::redact::Resources { dict: res.clone() };
    let mut names = || String::from("PannaUnused");
    let found = super::redact::redact(&content, &mut lookup, &areas, IDENTITY, true, &mut names)?;
    // The font with the most glyphs in the line, and where its first one was.
    let mut counts: HashMap<&[u8], usize> = HashMap::new();
    for taken in &found.taken {
        *counts.entry(taken.font.as_slice()).or_default() += taken.glyphs;
    }
    let (name, _) = counts.into_iter().max_by_key(|(_, n)| *n)?;
    let first = found.taken.iter().find(|t| t.font == name)?;
    let m = concat(first.at, to_shown);
    let size = m[3];
    let upright = m[0] > 0.0 && size > 0.0 && m[1].abs() < 0.01 * size && m[2].abs() < 0.01 * size;
    if !upright {
        return None;
    }
    let fonts = res?.get::<Dict<'_>>(b"Font")?;
    let font = fonts.get::<Dict<'_>>(name)?;
    let r = fonts.get_ref(name).and_then(obj_ref_of)?;
    let used = found.used.get(name).cloned().unwrap_or_default();
    Some(SameFont {
        font: r,
        bytes: encode(&font, text, &used)?,
        size,
        stretch: 100.0 * m[0] / size,
        origin: (m[4], m[5]),
    })
}

/// `text` in the character codes of `font`, when it can show every character: by its
/// ToUnicode map, or by WinAnsi or standard encoding for simple fonts without one. Subsets
/// may only use `used` codes, the ones the page shows with them.
fn encode(font: &Dict<'_>, text: &str, used: &std::collections::HashSet<u32>) -> Option<Vec<u8>> {
    use hayro::hayro_interpret::hayro_cmap::{BfString, CMap};
    use hayro::hayro_syntax::object::{Array, Name};
    let info = super::redact::font_from(font)?;
    let two = match info.codes {
        super::redact::Codes::One => false,
        super::redact::Codes::Two => true,
        super::redact::Codes::Unknown => return None,
    };
    let base = font
        .get::<Name<'_>>(b"BaseFont")
        .map(|n| n.as_str().to_owned())
        .unwrap_or_default();
    let subset = base.split_once('+').is_some_and(|(tag, _)| tag.len() == 6);
    let descriptor = font.get::<Dict<'_>>(b"FontDescriptor").or_else(|| {
        font.get::<Array<'_>>(b"DescendantFonts")?
            .iter::<Dict<'_>>()
            .next()?
            .get::<Dict<'_>>(b"FontDescriptor")
    });
    let embedded = descriptor.as_ref().is_some_and(|d| {
        [&b"FontFile"[..], b"FontFile2", b"FontFile3"]
            .iter()
            .any(|k| d.contains_key(k))
    });
    let allowed = |code: u32| (!subset && !two) || used.contains(&code);
    let mut map: HashMap<char, u32> = HashMap::new();
    if let Some(unicode) = font.get::<Stream<'_>>(b"ToUnicode") {
        let data = unicode.decoded().ok()?;
        let cmap = CMap::parse(&data, hayro::hayro_interpret::hayro_cmap::load_embedded)?;
        let codes: Vec<u32> = if two || subset {
            used.iter().copied().collect()
        } else {
            (0..256).collect()
        };
        for code in codes {
            if let Some(BfString::Char(c)) = cmap.lookup_bf_string(code) {
                map.entry(c).or_insert(code);
            }
        }
    } else if !two {
        let encoding = font.get::<Name<'_>>(b"Encoding").map(|n| n.as_str().to_owned());
        let win_ansi = match encoding.as_deref() {
            Some("WinAnsiEncoding") => true,
            // Standard encoding matches WinAnsi for letters, digits and plain punctuation.
            None | Some("StandardEncoding") => false,
            _ => return None,
        };
        for c in text.chars() {
            let code = if win_ansi {
                match super::annot::win_ansi(&c.to_string()).as_slice() {
                    [b] if *b != b'?' || c == '?' => u32::from(*b),
                    _ => return None,
                }
            } else if c.is_ascii_alphanumeric() || " .,:;-()!?/&%".contains(c) {
                u32::from(c as u8)
            } else {
                return None;
            };
            map.insert(c, code);
        }
    } else {
        return None;
    }
    let mut out = Vec::new();
    for c in text.chars() {
        let code = *map.get(&c)?;
        // Fonts that aren't embedded are drawn by the viewer's own, which have every glyph.
        if !allowed(code) || (embedded && info.width(code).is_none()) {
            return None;
        }
        if two {
            out.extend_from_slice(&(code as u16).to_be_bytes());
        } else {
            out.push(u8::try_from(code).ok()?);
        }
    }
    Some(out)
}

/// The pictures page `index` draws itself, in the order they're drawn, each as the box it
/// covers in the same space as page text.
pub(crate) fn page_pictures(pdf: &Pdf, index: usize) -> Vec<crate::TextBox> {
    let Some((content, res, to_shown, height)) = shown_page(pdf, index) else {
        return Vec::new();
    };
    let mut lookup = super::redact::Resources { dict: res };
    super::redact::placed_images(&content, &mut lookup)
        .iter()
        .map(|p| shown_box(to_shown, height, p.bounds()))
        .collect()
}

/// The rectangles and straight lines page `index` draws, as shown (see
/// [`super::redact::drawn_shapes`]); a line's box runs from end to end.
pub(crate) fn page_shapes(pdf: &Pdf, index: usize) -> (Vec<crate::TextBox>, Vec<crate::TextBox>) {
    let Some((content, _, to_shown, height)) = shown_page(pdf, index) else {
        return (Vec::new(), Vec::new());
    };
    let (rects, lines) = super::redact::drawn_shapes(&content);
    (
        rects
            .into_iter()
            .map(|r| shown_box(to_shown, height, r))
            .collect(),
        lines
            .into_iter()
            .map(|l| shown_box(to_shown, height, l))
            .collect(),
    )
}

/// Page `index`'s content and resources, with the matrix from its user space to shown space
/// (bottom up) and its shown height.
fn shown_page<'a>(
    pdf: &'a Pdf,
    index: usize,
) -> Option<(Vec<u8>, Option<Dict<'a>>, super::redact::Matrix, f64)> {
    let (pages, _) = page_tree(pdf);
    let (r, dict) = pages.get(index)?;
    let rotate = inherited(pdf, *r, dict, b"Rotate")
        .and_then(|(_, v)| std::str::from_utf8(v).ok()?.trim().parse::<f64>().ok())
        .map_or(0, |v| (v as i64).rem_euclid(360));
    let boxed = |key: &[u8]| inherited(pdf, *r, dict, key).and_then(|(_, v)| rect(v));
    let media = boxed(b"MediaBox").unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let shown = cropped(media, boxed(b"CropBox"), rotate, [0.0; 4]).unwrap_or(media);
    let res = inherited(pdf, *r, dict, b"Resources").and_then(|(owner, raw)| resolved(pdf, owner, raw));
    let (matrix, _, height) = shown_space(shown, rotate);
    let to_shown = super::redact::invert(matrix)?;
    Some((page_content(dict), res, to_shown, height))
}

fn shown_box(to_shown: super::redact::Matrix, height: f64, [x0, y0, x1, y1]: [f64; 4]) -> crate::TextBox {
    let a = super::redact::apply(to_shown, x0, y0);
    let b = super::redact::apply(to_shown, x1, y1);
    crate::TextBox {
        x0: a.0.min(b.0) as f32,
        y0: (height - a.1.max(b.1)) as f32,
        x1: a.0.max(b.0) as f32,
        y1: (height - a.1.min(b.1)) as f32,
    }
}

/// A page's content, all its streams decoded and joined.
pub(crate) fn page_content(page: &Dict<'_>) -> Vec<u8> {
    use hayro::hayro_syntax::object::{Array, Stream};
    let mut out = Vec::new();
    if let Some(stream) = page.get::<Stream<'_>>(b"Contents") {
        out.extend_from_slice(&stream.decoded().unwrap_or_default());
    } else if let Some(list) = page.get::<Array<'_>>(b"Contents") {
        for stream in list.iter::<Stream<'_>>().take(10_000) {
            out.extend_from_slice(&stream.decoded().unwrap_or_default());
            out.push(b'\n');
        }
    }
    out
}

fn flate_stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let packed = super::pack::deflate(data);
    let mut body = format!(
        "<< {dict}/Filter /FlateDecode /Length {} >>\nstream\n",
        packed.len()
    )
    .into_bytes();
    body.extend_from_slice(&packed);
    body.extend_from_slice(b"\nendstream");
    body
}

/// The annotations on a page that touch one of `areas` (in the page's own space), with their
/// pop-ups and replies. Fails on a form field, whose value lives on in the form.
fn redacted_annotations(
    page: &Dict<'_>,
    areas: &[[f64; 4]],
) -> Result<std::collections::HashSet<ObjRef>, SaveError> {
    use hayro::hayro_syntax::object::Array;
    annotations_where(page, |annot, subtype| {
        let Some(rect) = annot.get::<Array<'_>>(b"Rect") else {
            return Ok(false);
        };
        let v: Vec<f64> = rect.iter::<f64>().take(4).collect();
        let [x0, y0, x1, y1] = v[..] else { return Ok(false) };
        let b = [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)];
        let hit = areas
            .iter()
            .any(|a| b[0] < a[2] && a[0] < b[2] && b[1] < a[3] && a[1] < b[3]);
        if hit && subtype == Some("Widget") {
            return Err(SaveError::RedactsFormField);
        }
        Ok(hit)
    })
}

/// The comments on a page whose text mentions one of `phrases`, with their pop-ups and
/// replies. Links and form fields aren't comments, and stay.
fn mentioning_annotations(page: &Dict<'_>, phrases: &[String]) -> std::collections::HashSet<ObjRef> {
    use hayro::hayro_syntax::object::String as PdfString;
    annotations_where(page, |annot, subtype| {
        if matches!(subtype, Some("Widget" | "Link")) {
            return Ok(false);
        }
        Ok([b"Contents".as_slice(), b"Subj", b"RC"].iter().any(|key| {
            annot.get::<PdfString<'_>>(key).is_some_and(|s| {
                super::scrub::mentions(&crate::document::decode_text_string(s.as_bytes()), phrases)
            })
        }))
    })
    .unwrap_or_default()
}

/// The annotations on a page that `hit` picks (given each one and its subtype), with their
/// pop-ups and replies.
fn annotations_where(
    page: &Dict<'_>,
    hit: impl Fn(&Dict<'_>, Option<&str>) -> Result<bool, SaveError>,
) -> Result<std::collections::HashSet<ObjRef>, SaveError> {
    use hayro::hayro_syntax::object::{Array, Name};
    let mut gone = std::collections::HashSet::new();
    let Some(annots) = page.get::<Array<'_>>(b"Annots") else {
        return Ok(gone);
    };
    let list: Vec<(ObjRef, Dict<'_>)> = annots
        .iter::<Dict<'_>>()
        .take(20_000)
        .filter_map(|a| Some((a.obj_id().and_then(obj_ref)?, a)))
        .collect();
    for (r, annot) in &list {
        let subtype = annot.get::<Name<'_>>(b"Subtype").map(|n| n.as_str().to_owned());
        if subtype.as_deref() == Some("Popup") {
            continue;
        }
        if hit(annot, subtype.as_deref())? {
            gone.insert(*r);
        }
    }
    // Pop-ups and replies of what went, and replies to those.
    for _ in 0..16 {
        let before = gone.len();
        for (r, annot) in &list {
            let linked = [b"Parent".as_slice(), b"IRT"]
                .iter()
                .filter_map(|k| annot.get_ref(k).and_then(obj_ref_of))
                .any(|p| gone.contains(&p));
            if linked {
                gone.insert(*r);
            }
        }
        if gone.len() == before {
            break;
        }
    }
    Ok(gone)
}

fn obj_ref_of(r: hayro::hayro_syntax::object::ObjRef) -> Option<ObjRef> {
    ObjRef::new(r.obj_number, r.gen_number)
}

/// Forms renamed in a page's or form's resources: the name, and the copy's object number (0
/// to remove the name).
type FormOverrides = Vec<(String, u32)>;

/// Redacts `content` (drawn with `ctm`, using `res`), writing redacted copies of the forms it
/// touches. Returns the new content and the resource changes it needs: forms renamed to their
/// copies, and pictures and original forms dropped.
#[allow(clippy::too_many_arguments)]
fn redact_into<'a>(
    w: &mut Writer,
    sources: &[Source<'a>],
    doc: usize,
    res: Option<Dict<'a>>,
    content: &[u8],
    areas: &[[f64; 4]],
    ctm: super::redact::Matrix,
    depth: usize,
    counter: &mut u32,
    text_only: bool,
) -> Result<(Vec<u8>, FormOverrides), SaveError> {
    let mut lookup = super::redact::Resources { dict: res.clone() };
    let mut names = || {
        *counter += 1;
        format!("PannaRedact{counter}")
    };
    // Content Panna can't follow is dropped rather than kept unredacted; when only editing
    // text, it's kept as it was.
    let redacted = super::redact::redact(content, &mut lookup, areas, ctm, text_only, &mut names)
        .unwrap_or_else(|| super::redact::Redacted {
            content: if text_only { content.to_vec() } else { Vec::new() },
            ..super::redact::Redacted::default()
        });
    let mut overrides: Vec<(String, u32)> = Vec::new();
    for image in &redacted.images {
        overrides.push((String::from_utf8_lossy(image).into_owned(), 0));
    }
    for name in redacted
        .forms
        .iter()
        .map(|f| &f.name)
        .collect::<std::collections::HashSet<_>>()
    {
        overrides.push((String::from_utf8_lossy(name).into_owned(), 0));
    }
    for form in &redacted.forms {
        let Some((stream, Some(form_ref))) = lookup.stream(&form.name) else {
            continue;
        };
        let num = w.alloc();
        overrides.push((form.new_name.clone(), num));
        let dict = stream.dict();
        let own = dict.get::<Dict<'a>>(b"Resources");
        let (inner, inner_overrides) = if depth + 1 >= super::redact::MAX_DEPTH {
            (Vec::new(), Vec::new())
        } else {
            let data = stream.decoded().unwrap_or_default();
            redact_into(
                w,
                sources,
                doc,
                own.clone().or_else(|| res.clone()),
                &data,
                areas,
                form.ctm,
                depth + 1,
                counter,
                text_only,
            )?
        };
        let data = dict.data();
        let mut body = b"<<".to_vec();
        let mut resources_raw = None;
        for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
            let value = data.get(value).ok_or(SaveError::Damaged)?;
            if key == b"Resources" {
                resources_raw = Some(value);
                continue;
            }
            if matches!(key, b"Length" | b"Filter" | b"DecodeParms") {
                continue;
            }
            body.push(b'/');
            body.extend_from_slice(key);
            body.push(b' ');
            body.extend(copy_syntax(w, sources, doc, form_ref, value)?);
            body.push(b'\n');
        }
        // A form without resources of its own uses the page's; its copy names them itself.
        body.extend_from_slice(b"/Resources ");
        match resources_raw {
            Some(raw) => body.extend(with_xobjects(
                w,
                sources,
                doc,
                Some((form_ref, raw)),
                &inner_overrides,
            )?),
            None => body.extend(with_xobjects(w, sources, doc, None, &inner_overrides)?),
        }
        let packed = super::pack::deflate(&inner);
        body.extend_from_slice(
            format!("\n/Filter /FlateDecode /Length {} >>\nstream\n", packed.len()).as_bytes(),
        );
        body.extend_from_slice(&packed);
        body.extend_from_slice(b"\nendstream");
        w.set(num, body);
    }
    Ok((redacted.content, overrides))
}

/// The file: a header, every object (sealed for an encrypted file), then the cross-reference
/// table and trailer. With `compact`, objects that aren't streams go into compressed object
/// streams and the cross-reference table is a compressed stream too.
fn write_file(
    mut objects: Vec<Option<Vec<u8>>>,
    catalog_num: u32,
    info_num: Option<u32>,
    encryption: Option<&Encryption>,
    compact: bool,
) -> Result<Vec<u8>, SaveError> {
    let mut packed = HashMap::new();
    let mut stream_nums = Vec::new();
    if compact {
        let (streams, places) = super::pack::object_streams(&objects, &[]);
        for body in streams {
            objects.push(Some(body));
            stream_nums.push(objects.len() as u32);
        }
        packed = places;
    }
    // Encrypted output: everything but the encryption dictionary itself is sealed.
    let encrypt_num = encryption.map(|_| {
        objects.push(None);
        objects.len() as u32
    });
    let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        let num = i as u32 + 1;
        offsets.push(out.len());
        if packed.contains_key(&num) {
            continue;
        }
        let body = body.as_deref().unwrap_or(b"null");
        let body = match encryption {
            Some(e) if Some(num) == encrypt_num => e.dict.clone(),
            Some(e) => e.crypt.seal(num, 0, body)?,
            None => body.to_vec(),
        };
        out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    let mut trailer = format!("/Root {catalog_num} 0 R ");
    if let Some(info) = info_num {
        let _ = write!(trailer, "/Info {info} 0 R ");
    }
    if let (Some(e), Some(num)) = (encryption, encrypt_num) {
        let _ = write!(
            trailer,
            "/Encrypt {num} 0 R /ID {} ",
            String::from_utf8_lossy(&e.id)
        );
    } else {
        // An identifier made from the content, as PDF/A and many tools want one.
        use md5::{Digest, Md5};
        let digest = Md5::digest(&out);
        let hex: String = digest.iter().map(|b| format!("{b:02X}")).collect();
        let _ = write!(trailer, "/ID [<{hex}> <{hex}>] ");
    }
    if compact {
        // The cross-reference stream is the last object, and lists itself.
        let size = objects.len() + 2;
        let mut table = super::pack::xref_entry(0, 0, 65535).to_vec();
        for (i, offset) in offsets.iter().enumerate() {
            let num = i as u32 + 1;
            let entry = match packed.get(&num) {
                Some(&(s, index)) => super::pack::xref_entry(
                    2,
                    stream_nums.get(s).copied().ok_or(SaveError::Damaged)?,
                    u16::try_from(index).map_err(|_| SaveError::Damaged)?,
                ),
                None => {
                    super::pack::xref_entry(1, u32::try_from(*offset).map_err(|_| SaveError::Damaged)?, 0)
                }
            };
            table.extend_from_slice(&entry);
        }
        table.extend_from_slice(&super::pack::xref_entry(
            1,
            u32::try_from(xref_at).map_err(|_| SaveError::Damaged)?,
            0,
        ));
        let data = super::pack::deflate(&table);
        out.extend_from_slice(
            format!(
                "{} 0 obj\n<< /Type /XRef /Size {size} /W [1 4 2] {trailer}/Filter /FlateDecode /Length {} >>\nstream\n",
                size - 1,
                data.len()
            )
            .as_bytes(),
        );
        out.extend_from_slice(&data);
        out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
        return Ok(out);
    }
    let mut xref = format!("xref\n0 {}\n0000000000 65535 f\r\n", offsets.len() + 1);
    for offset in offsets {
        let _ = write!(xref, "{offset:010} 00000 n\r\n");
    }
    let _ = write!(
        xref,
        "trailer\n<< /Size {} {trailer}>>\nstartxref\n{xref_at}\n%%EOF\n",
        objects.len() + 1
    );
    out.extend_from_slice(xref.as_bytes());
    Ok(out)
}

/// Blacks out `phrases` in the titles of the copied bookmarks of the first source.
fn retitle_bookmarks(w: &mut Writer, pdf: &Pdf, phrases: &[String]) {
    for (oid, title) in crate::outline::titled(pdf) {
        let Some(title) = super::scrub::black_out(&title, phrases) else {
            continue;
        };
        let Some(Some(num)) = obj_ref(oid).and_then(|r| w.maps.first()?.get(&r).copied()) else {
            continue;
        };
        let Some(slot) = w.objects.get_mut(num as usize - 1) else {
            continue;
        };
        if let Some(body) = slot
            .as_deref()
            .and_then(|b| with_entry(b, b"Title", &super::annot::text_string(&title)))
        {
            *slot = Some(body);
        }
    }
}

/// The dictionary `body` with `key` set to `value`.
fn with_entry(body: &[u8], key: &[u8], value: &str) -> Option<Vec<u8>> {
    let mut out = b"<<".to_vec();
    for (k, v) in lex::dict_entries(body)? {
        if k == key {
            continue;
        }
        out.push(b'/');
        out.extend_from_slice(k);
        out.push(b' ');
        out.extend_from_slice(body.get(v)?);
        out.push(b'\n');
    }
    out.push(b'/');
    out.extend_from_slice(key);
    out.push(b' ');
    out.extend_from_slice(value.as_bytes());
    out.extend_from_slice(b"\n>>");
    Some(out)
}

#[cfg(test)]
mod crop_tests {
    use super::cropped;

    #[test]
    fn margins_follow_the_page_as_shown() {
        let media = [0.0, 0.0, 600.0, 800.0];
        let top = [100.0, 0.0, 0.0, 0.0];
        // Unturned, the top of the screen is the top of the page.
        assert_eq!(cropped(media, None, 0, top), Some([0.0, 0.0, 600.0, 700.0]));
        // Turned a quarter clockwise, the page's left edge is at the top of the screen.
        assert_eq!(cropped(media, None, 90, top), Some([100.0, 0.0, 600.0, 800.0]));
        assert_eq!(cropped(media, None, 180, top), Some([0.0, 100.0, 600.0, 800.0]));
        assert_eq!(cropped(media, None, 270, top), Some([0.0, 0.0, 500.0, 800.0]));
        // An existing crop box is the starting point, kept within the media box.
        assert_eq!(
            cropped(media, Some([-50.0, 50.0, 550.0, 750.0]), 0, [0.0; 4]),
            Some([0.0, 50.0, 550.0, 750.0])
        );
        assert_eq!(cropped(media, None, 0, [790.0, 0.0, 0.0, 0.0]), None);
    }
}
