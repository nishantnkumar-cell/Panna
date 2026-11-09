//! Files attached to a document: in its EmbeddedFiles name tree, and on its pages as file
//! attachment annotations. They are only listed and saved, never opened.

use std::collections::HashSet;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Object, ObjectIdentifier, Stream};

use crate::document::decode_text_string;

const MAX_DEPTH: u16 = 32;
const MAX_NODES: usize = 10_000;
const MAX_FILES: usize = 10_000;

/// An attached file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attachment {
    /// Its file name.
    pub name: String,
    pub description: String,
    /// Its size in bytes, when the file says.
    pub size: Option<u64>,
    /// The page it's attached to (from 0), or `None` for the document as a whole.
    pub page: Option<usize>,
}

impl Attachment {
    /// Whether the file is a program or script that would run if opened, so the user can be
    /// warned before saving it. Trailing dots and spaces, which Windows drops, are ignored.
    pub fn is_program(&self) -> bool {
        const RISKY: &[&str] = &[
            "exe", "com", "scr", "pif", "cpl", "msi", "msp", "msc", "bat", "cmd", "ps1", "psm1", "vbs",
            "vbe", "js", "jse", "wsf", "wsh", "hta", "lnk", "url", "reg", "dll", "jar", "app", "command",
            "sh", "desktop", "appimage", "docm", "xlsm", "pptm", "iso", "img", "vhd", "vhdx",
        ];
        let name = self.name.trim_end_matches(['.', ' ']).to_ascii_lowercase();
        name.rsplit_once('.').is_some_and(|(_, ext)| RISKY.contains(&ext))
    }
}

/// Unicode's explicit text-direction marks, embeddings, overrides and isolates.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Every attached file's specification, with the page it's on: the document's first, then
/// each page's in order.
fn specs(pdf: &Pdf) -> Vec<(Dict<'_>, Option<usize>)> {
    let mut out = Vec::new();
    let xref = pdf.xref();
    if let Some(tree) = xref
        .get::<Dict<'_>>(xref.root_id())
        .and_then(|catalog| catalog.get::<Dict<'_>>(b"Names"))
        .and_then(|names| names.get::<Dict<'_>>(b"EmbeddedFiles"))
    {
        let mut budget = MAX_NODES;
        walk(tree, 0, &mut budget, &mut HashSet::new(), &mut out);
    }
    for (index, page) in pdf.pages().iter().enumerate() {
        let Some(annots) = page.raw().get::<Array<'_>>(b"Annots") else {
            continue;
        };
        for annot in annots.iter::<Dict<'_>>().take(4_000) {
            // A file can be both in the tree and on a page; it's listed once.
            let listed = |spec: &Dict<'_>| {
                spec.obj_id()
                    .is_some_and(|id| out.iter().any(|(s, _): &(Dict<'_>, _)| s.obj_id() == Some(id)))
            };
            let is_file = annot
                .get::<hayro::hayro_syntax::object::Name<'_>>(b"Subtype")
                .is_some_and(|s| s.as_str() == "FileAttachment");
            if let Some(spec) = annot.get::<Dict<'_>>(b"FS").filter(|_| is_file)
                && !listed(&spec)
                && out.len() < MAX_FILES
            {
                out.push((spec, Some(index)));
            }
        }
    }
    out
}

fn walk<'a>(
    node: Dict<'a>,
    depth: u16,
    budget: &mut usize,
    visited: &mut HashSet<ObjectIdentifier>,
    out: &mut Vec<(Dict<'a>, Option<usize>)>,
) {
    if depth > MAX_DEPTH || *budget == 0 || out.len() >= MAX_FILES {
        return;
    }
    *budget -= 1;
    if let Some(id) = node.obj_id()
        && !visited.insert(id)
    {
        return;
    }
    if let Some(names) = node.get::<Array<'a>>(b"Names") {
        let mut entries = names.iter::<Object<'a>>();
        while let (Some(_), Some(value)) = (entries.next(), entries.next()) {
            if let Object::Dict(spec) = value
                && out.len() < MAX_FILES
            {
                out.push((spec, None));
            }
        }
    }
    if let Some(kids) = node.get::<Array<'a>>(b"Kids") {
        for kid in kids.iter::<Dict<'a>>() {
            walk(kid, depth + 1, budget, visited, out);
        }
    }
}

/// The embedded file stream of a file specification.
fn file<'a>(spec: &Dict<'a>) -> Option<Stream<'a>> {
    let ef = spec.get::<Dict<'a>>(b"EF")?;
    ef.get::<Stream<'a>>(b"UF").or_else(|| ef.get::<Stream<'a>>(b"F"))
}

/// The files attached to `pdf`, the document's first.
pub(crate) fn read(pdf: &Pdf) -> Vec<Attachment> {
    specs(pdf)
        .into_iter()
        .filter_map(|(spec, page)| {
            let stream = file(&spec)?;
            let text = |key: &[u8]| {
                spec.get::<hayro::hayro_syntax::object::String<'_>>(key)
                    .map(|s| decode_text_string(s.as_bytes()))
            };
            let name = text(b"UF").or_else(|| text(b"F")).unwrap_or_default();
            // Only the last part of the name: it becomes a file name when saved. Control and
            // text-direction characters go too: an override (U+202E) can make "invoice<U+202E>fdp.exe" show
            // as "invoiceexe.pdf".
            let name: String = name
                .rsplit(['/', '\\', ':'])
                .next()
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_control() && !is_bidi_control(*c))
                .collect();
            let name = name.trim().to_owned();
            let size = stream
                .dict()
                .get::<Dict<'_>>(b"Params")
                .and_then(|p| p.get::<i64>(b"Size"))
                .or_else(|| stream.dict().get::<i64>(b"DL"))
                .and_then(|s| u64::try_from(s).ok());
            Some(Attachment {
                name: if name.is_empty() {
                    "attachment".into()
                } else {
                    name
                },
                description: text(b"Desc").unwrap_or_default(),
                size,
                page,
            })
        })
        .collect()
}

/// The contents of the attached file at `index` in [`read`]'s list, if it unpacks to no more
/// than `limit` bytes (measured first, so a decompression bomb is never unpacked).
pub(crate) fn data(pdf: &Pdf, index: usize, limit: u64) -> Option<Vec<u8>> {
    let spec = specs(pdf)
        .into_iter()
        .filter(|(spec, _)| file(spec).is_some())
        .nth(index)?
        .0;
    let stream = file(&spec)?;
    crate::guard::decoded_size(&stream, limit).ok()?;
    stream.decoded().ok().map(|d| d.into_owned())
}
