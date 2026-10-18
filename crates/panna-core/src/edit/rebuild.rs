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
}

/// Hidden information to leave out of the new file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cleanup {
    /// The document information (title, author, dates, software) and XMP metadata.
    pub metadata: bool,
    /// JavaScript and actions that run by themselves (on opening, on page views, on fields).
    pub scripts: bool,
    /// Attached files, in the document and on the pages.
    pub attachments: bool,
    /// Comments and markup; links and form fields stay.
    pub comments: bool,
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
        xobjects.extend_from_slice(format!(" /{name} {form} 0 R").as_bytes());
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
) -> Result<Vec<u8>, SaveError> {
    let first = sources.first().ok_or(SaveError::Damaged)?;
    let info = info.filter(|_| !clean.metadata);
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
                // Annotations drawn into the page, and the ones left as they are.
                let mut flattened = Vec::new();
                let mut kept = String::new();
                let filtering = extra.flatten.any() || clean.annotations();
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
                        if clean.scripts && has_actions {
                            // A copy without its scripts, under the number everything that
                            // refers to it will use.
                            if let Some(num) = w.claim(doc, r) {
                                let data = annot.data();
                                let mut body = b"<<".to_vec();
                                for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
                                    if key == b"AA" || (key == b"A" && script) {
                                        continue;
                                    }
                                    body.push(b'/');
                                    body.extend_from_slice(key);
                                    body.push(b' ');
                                    let value = data.get(value).ok_or(SaveError::Damaged)?;
                                    body.extend(copy_syntax(&mut w, sources, doc, r, value)?);
                                    body.push(b'\n');
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
                let drawn = !extra.under.is_empty() || !extra.over.is_empty() || !flattened.is_empty();
                for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
                    if key == b"Parent" || INHERITED.contains(&key) {
                        continue;
                    }
                    if (key == b"Annots" && filtering) || (key == b"AA" && clean.scripts) {
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
                if filtering && !kept.is_empty() {
                    body.extend_from_slice(format!("/Annots [{}]\n", kept.trim_end()).as_bytes());
                }
                if drawn {
                    let (matrix, width, height) = shown_space(shown, rotate);
                    let stream = |dict: &str, data: &str| {
                        format!("<< {dict}/Length {} >>\nstream\n{data}\nendstream", data.len()).into_bytes()
                    };
                    let mut forms = Vec::new();
                    let draw = |w: &mut Writer,
                                forms: &mut Vec<(String, u32)>,
                                drawings: &[super::marks::Drawing]| {
                        let mut ops = String::new();
                        for drawing in drawings {
                            let form = w.alloc();
                            w.set(
                                form,
                                stream(
                                    &format!(
                                        "/Type /XObject /Subtype /Form /BBox [0 0 {} {}] /Matrix [{}] /Resources {} ",
                                        n(width),
                                        n(height),
                                        matrix.map(n).join(" "),
                                        drawing.resources()
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
    let mut body = b"<<".to_vec();
    for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
        // Flattened form fields are page content now, so the form goes.
        let flattened_form = key == b"AcroForm" && extras.iter().any(|e| e.flatten.fields);
        if key == b"Pages" || (key == b"PageLabels" && !same_pages) || flattened_form {
            continue;
        }
        let value = data.get(value.clone()).ok_or(SaveError::Damaged)?;
        if (clean.metadata && key == b"Metadata")
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
