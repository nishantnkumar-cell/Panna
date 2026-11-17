//! Files attached to a document: in its EmbeddedFiles name tree, and on its pages as file
//! attachment annotations. Videos, sounds and 3D models carried by screen, movie, rich media
//! and 3D annotations are listed with them. They are only listed and saved, never opened or
//! played: media players and 3D viewers are a large attack surface.

use std::collections::HashSet;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier, Stream};

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
    /// What kind of media it is, when it's played on a page rather than attached.
    #[serde(default)]
    pub media: Option<Media>,
}

/// Media a page plays: listed so it can be saved, never played inside Panna.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Media {
    Video,
    Sound,
    Model3D,
    /// Rich media content of another kind, such as a Flash file.
    Other,
}

impl Media {
    pub fn label(self) -> &'static str {
        match self {
            Media::Video => "Video",
            Media::Sound => "Sound",
            Media::Model3D => "3D model",
            Media::Other => "Rich media",
        }
    }

    /// From a MIME type such as `video/mp4`.
    fn from_mime(mime: &str) -> Option<Self> {
        let mime = mime.to_ascii_lowercase();
        if mime.starts_with("video/") {
            Some(Media::Video)
        } else if mime.starts_with("audio/") {
            Some(Media::Sound)
        } else if mime.starts_with("model/") {
            Some(Media::Model3D)
        } else {
            None
        }
    }

    /// From a file name's extension.
    fn from_name(name: &str) -> Self {
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "mp4" | "m4v" | "mov" | "avi" | "wmv" | "flv" | "f4v" | "mpg" | "mpeg" | "webm" | "mkv"
            | "3gp" => Media::Video,
            "mp3" | "wav" | "aif" | "aiff" | "aac" | "m4a" | "wma" | "ogg" | "flac" | "au" | "snd" => {
                Media::Sound
            }
            "u3d" | "prc" | "stl" | "obj" | "glb" | "gltf" => Media::Model3D,
            _ => Media::Other,
        }
    }
}

impl Attachment {
    /// Whether the file is a PDF, by its name.
    pub fn is_pdf(&self) -> bool {
        self.name
            .trim_end_matches(['.', ' '])
            .rsplit_once('.')
            .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("pdf"))
    }

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

/// Where an attached file's bytes are: a file specification, or (for a 3D model) a stream
/// with the name to save it under.
enum Source<'a> {
    Spec(Dict<'a>),
    Raw(Stream<'a>, String),
}

impl Source<'_> {
    fn id(&self) -> Option<ObjectIdentifier> {
        match self {
            Source::Spec(spec) => spec.obj_id(),
            Source::Raw(stream, _) => Some(stream.obj_id()),
        }
    }
}

/// One listed file: where it is, the page it's on, and the kind of media it is.
type Entry<'a> = (Source<'a>, Option<usize>, Option<Media>);

/// Every attached file, with the page it's on: the document's first, then each page's in
/// order, a page's attachments and media together.
fn specs(pdf: &Pdf) -> Vec<Entry<'_>> {
    let mut out: Vec<Entry<'_>> = Vec::new();
    let xref = pdf.xref();
    if let Some(tree) = xref
        .get::<Dict<'_>>(xref.root_id())
        .and_then(|catalog| catalog.get::<Dict<'_>>(b"Names"))
        .and_then(|names| names.get::<Dict<'_>>(b"EmbeddedFiles"))
    {
        let mut budget = MAX_NODES;
        let mut found = Vec::new();
        walk(tree, 0, &mut budget, &mut HashSet::new(), &mut found);
        out.extend(found.into_iter().map(|spec| (Source::Spec(spec), None, None)));
    }
    for (index, page) in pdf.pages().iter().enumerate() {
        let Some(annots) = page.raw().get::<Array<'_>>(b"Annots") else {
            continue;
        };
        for annot in annots.iter::<Dict<'_>>().take(4_000) {
            let subtype = annot.get::<Name<'_>>(b"Subtype");
            let found = match subtype.as_ref().map(Name::as_str) {
                Some("FileAttachment") => annot
                    .get::<Dict<'_>>(b"FS")
                    .map(|spec| vec![(Source::Spec(spec), None)])
                    .unwrap_or_default(),
                Some("Screen") => screen(&annot),
                Some("Movie") => annot
                    .get::<Dict<'_>>(b"Movie")
                    .and_then(|movie| movie.get::<Dict<'_>>(b"F"))
                    .map(|spec| vec![(Source::Spec(spec), Some(Media::Video))])
                    .unwrap_or_default(),
                Some("RichMedia") => rich_media(&annot),
                Some("3D") => model(&annot, index).into_iter().collect(),
                _ => Vec::new(),
            };
            for (source, media) in found {
                // A file can be listed in more than one place; it's listed once.
                let listed = source
                    .id()
                    .is_some_and(|id| out.iter().any(|(s, ..)| s.id() == Some(id)));
                if !listed && out.len() < MAX_FILES {
                    out.push((source, Some(index), media));
                }
            }
        }
    }
    out
}

/// The clip a screen annotation's rendition plays: `/A` (or the page-open action) names a
/// rendition, either a media rendition or a selector whose first choice is one.
fn screen<'a>(annot: &Dict<'a>) -> Vec<(Source<'a>, Option<Media>)> {
    let actions = [
        annot.get::<Dict<'a>>(b"A"),
        annot
            .get::<Dict<'a>>(b"AA")
            .and_then(|aa| aa.get::<Dict<'a>>(b"PO")),
        annot
            .get::<Dict<'a>>(b"AA")
            .and_then(|aa| aa.get::<Dict<'a>>(b"PV")),
    ];
    let mut out = Vec::new();
    for action in actions.into_iter().flatten() {
        let Some(rendition) = action.get::<Dict<'a>>(b"R") else {
            continue;
        };
        // A selector lists renditions in order of preference; the first media one is kept.
        let rendition = match rendition.get::<Array<'a>>(b"R") {
            Some(choices) => choices
                .iter::<Dict<'a>>()
                .take(64)
                .find(|r| r.get::<Dict<'a>>(b"C").is_some()),
            None => Some(rendition),
        };
        let Some(clip) = rendition.and_then(|r| r.get::<Dict<'a>>(b"C")) else {
            continue;
        };
        let kind = clip
            .get::<hayro::hayro_syntax::object::String<'_>>(b"CT")
            .and_then(|ct| Media::from_mime(&String::from_utf8_lossy(ct.as_bytes())));
        if let Some(spec) = clip.get::<Dict<'a>>(b"D") {
            out.push((Source::Spec(spec), Some(kind.unwrap_or(Media::Video))));
        }
    }
    out
}

/// The files a rich media annotation carries: its content's Assets name tree.
fn rich_media<'a>(annot: &Dict<'a>) -> Vec<(Source<'a>, Option<Media>)> {
    let Some(assets) = annot
        .get::<Dict<'a>>(b"RichMediaContent")
        .and_then(|content| content.get::<Dict<'a>>(b"Assets"))
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut budget = MAX_NODES;
    walk(assets, 0, &mut budget, &mut HashSet::new(), &mut found);
    found
        .into_iter()
        .map(|spec| {
            let media = Media::from_name(&spec_name(&spec));
            (Source::Spec(spec), Some(media))
        })
        .collect()
}

/// A 3D annotation's model: its `/3DD` stream, or a reference dictionary's.
fn model<'a>(annot: &Dict<'a>, page: usize) -> Option<(Source<'a>, Option<Media>)> {
    let stream = annot.get::<Stream<'a>>(b"3DD").or_else(|| {
        annot
            .get::<Dict<'a>>(b"3DD")
            .and_then(|reference| reference.get::<Stream<'a>>(b"3DD"))
    })?;
    let format = stream.dict().get::<Name<'_>>(b"Subtype");
    let ext = match format.as_ref().map(Name::as_str) {
        Some("PRC") => "prc",
        _ => "u3d",
    };
    let name = format!("3D model, page {}.{ext}", page + 1);
    Some((Source::Raw(stream, name), Some(Media::Model3D)))
}

fn walk<'a>(
    node: Dict<'a>,
    depth: u16,
    budget: &mut usize,
    visited: &mut HashSet<ObjectIdentifier>,
    out: &mut Vec<Dict<'a>>,
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
                out.push(spec);
            }
        }
    }
    if let Some(kids) = node.get::<Array<'a>>(b"Kids") {
        for kid in kids.iter::<Dict<'a>>() {
            walk(kid, depth + 1, budget, visited, out);
        }
    }
}

/// Whether the document is a PDF Portfolio: its catalog has a `/Collection`.
pub(crate) fn is_portfolio(pdf: &Pdf) -> bool {
    let xref = pdf.xref();
    xref.get::<Dict<'_>>(xref.root_id())
        .is_some_and(|catalog| catalog.contains_key(b"Collection"))
}

/// The embedded file stream of a file specification.
fn file<'a>(spec: &Dict<'a>) -> Option<Stream<'a>> {
    let ef = spec.get::<Dict<'a>>(b"EF")?;
    ef.get::<Stream<'a>>(b"UF").or_else(|| ef.get::<Stream<'a>>(b"F"))
}

/// The stream holding a source's bytes; `None` for a file kept outside the PDF.
fn stream<'a>(source: &Source<'a>) -> Option<Stream<'a>> {
    match source {
        Source::Spec(spec) => file(spec),
        Source::Raw(stream, _) => Some(stream.clone()),
    }
}

/// A file specification's name as written: the Unicode name, else the plain one.
fn spec_name(spec: &Dict<'_>) -> String {
    let text = |key: &[u8]| {
        spec.get::<hayro::hayro_syntax::object::String<'_>>(key)
            .map(|s| decode_text_string(s.as_bytes()))
    };
    text(b"UF").or_else(|| text(b"F")).unwrap_or_default()
}

/// The files attached to `pdf`, the document's first. Media kept in files outside the PDF
/// aren't listed: there is nothing in the document to save.
pub(crate) fn read(pdf: &Pdf) -> Vec<Attachment> {
    specs(pdf)
        .into_iter()
        .filter_map(|(source, page, media)| {
            let stream = stream(&source)?;
            let (name, description) = match &source {
                Source::Spec(spec) => (
                    spec_name(spec),
                    spec.get::<hayro::hayro_syntax::object::String<'_>>(b"Desc")
                        .map(|s| decode_text_string(s.as_bytes()))
                        .unwrap_or_default(),
                ),
                Source::Raw(_, name) => (name.clone(), String::new()),
            };
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
                description,
                size,
                page,
                media,
            })
        })
        .collect()
}

/// The contents of the attached file at `index` in [`read`]'s list, if it unpacks to no more
/// than `limit` bytes (measured first, so a decompression bomb is never unpacked).
pub(crate) fn data(pdf: &Pdf, index: usize, limit: u64) -> Option<Vec<u8>> {
    let stream = specs(pdf)
        .into_iter()
        .filter_map(|(source, ..)| stream(&source))
        .nth(index)?;
    crate::guard::decoded_size(&stream, limit).ok()?;
    stream.decoded().ok().map(|d| d.into_owned())
}
