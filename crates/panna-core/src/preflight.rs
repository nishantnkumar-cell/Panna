//! Checking a document against the main rules of PDF/A-2b, the archival kind of PDF: all
//! fonts embedded, colours tied to a stated output, metadata that says it is PDF/A, and
//! nothing that runs, plays, or depends on files outside it. Also against PDF/X-4, the kind
//! for printing, which asks much the same plus a printing condition and each page's trimmed
//! size.

use std::collections::HashSet;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Name, Object, Stream};

/// Fonts, pictures and drawings looked at at most, so a hostile file stays quick to check.
const MAX_VISITS: usize = 20_000;
/// How deep drawings inside drawings are followed.
const MAX_DEPTH: u8 = 12;
/// Findings of one kind listed at most; the rest are counted.
const MAX_PER_RULE: usize = 20;

/// What a rule is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rule {
    Encrypted,
    NoFileId,
    NoMetadata,
    NotMarkedPdfA,
    NoOutputIntent,
    FontNotEmbedded,
    Scripts,
    AttachedFiles,
    MediaAnnotation,
    NoAppearance,
    NotPrinted,
    ForbiddenFilter,
    ExternalContent,
    NotMarkedPdfX,
    NoPrintingCondition,
    NoTrimBox,
}

/// Which standard to check against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standard {
    /// PDF/A-2b, for archiving.
    PdfA,
    /// PDF/X-4, for printing.
    PdfX,
}

impl Rule {
    /// What the rule asks, for people.
    pub fn title(self) -> &'static str {
        match self {
            Rule::Encrypted => "The file must not be password protected",
            Rule::NoFileId => "The file must have an identifier",
            Rule::NoMetadata => "The file must have XMP metadata",
            Rule::NotMarkedPdfA => "The metadata must say the file is PDF/A",
            Rule::NoOutputIntent => "Colours must be tied to an output profile (output intent)",
            Rule::FontNotEmbedded => "Every font must be embedded",
            Rule::Scripts => "Nothing may run JavaScript",
            Rule::AttachedFiles => "Attached files must themselves be PDF/A",
            Rule::MediaAnnotation => "Sound, movie and 3D content isn't allowed",
            Rule::NoAppearance => "Comments and fields must have their own appearance",
            Rule::NotPrinted => "Comments and fields must be set to print",
            Rule::ForbiddenFilter => "LZW compression isn't allowed",
            Rule::ExternalContent => "Content must not come from outside the file",
            Rule::NotMarkedPdfX => "The file must say it is PDF/X-4",
            Rule::NoPrintingCondition => {
                "The file must name the printing condition it's made for (output intent)"
            }
            Rule::NoTrimBox => "Every page must give its trimmed size (trim box)",
        }
    }
}

/// One thing that doesn't meet PDF/A.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: Rule,
    /// Where or what, such as a font's name.
    pub detail: String,
    pub page: Option<usize>,
}

struct Check<'a> {
    pdf: &'a Pdf,
    out: Vec<Finding>,
    seen: HashSet<(i32, i32)>,
    visits: usize,
    /// Checking for PDF/A, which has rules PDF/X doesn't.
    archive: bool,
}

impl Check<'_> {
    fn add(&mut self, rule: Rule, detail: impl Into<String>, page: Option<usize>) {
        let detail = detail.into();
        let same = self.out.iter().filter(|f| f.rule == rule).count();
        if same < MAX_PER_RULE && !self.out.iter().any(|f| f.rule == rule && f.detail == detail) {
            self.out.push(Finding { rule, detail, page });
        }
    }

    /// Whether `dict` is new to the check, and the check still has room.
    fn first_visit(&mut self, dict: &Dict<'_>) -> bool {
        self.visits += 1;
        if self.visits > MAX_VISITS {
            return false;
        }
        match dict.obj_id() {
            Some(id) => self.seen.insert((id.obj_number, id.gen_number)),
            None => true,
        }
    }

    fn stream(&mut self, stream: &Stream<'_>, page: Option<usize>) {
        let dict = stream.dict();
        if dict.contains_key(b"F") || dict.contains_key(b"FFilter") {
            self.add(Rule::ExternalContent, "A stream's data is in another file", page);
        }
        let lzw = |name: &str| name == "LZWDecode" || name == "LZW";
        let filters: Vec<String> = match dict.get::<Object<'_>>(b"Filter") {
            Some(Object::Name(n)) => vec![n.as_str().to_owned()],
            Some(Object::Array(a)) => a.iter::<Name<'_>>().map(|n| n.as_str().to_owned()).collect(),
            _ => Vec::new(),
        };
        if self.archive && filters.iter().any(|f| lzw(f)) {
            self.add(Rule::ForbiddenFilter, "A stream uses LZW", page);
        }
    }

    fn resources(&mut self, res: &Dict<'_>, page: usize, depth: u8) {
        if depth > MAX_DEPTH {
            return;
        }
        if let Some(fonts) = res.get::<Dict<'_>>(b"Font") {
            for key in fonts.keys().collect::<Vec<_>>() {
                if let Some(font) = fonts.get::<Dict<'_>>(key.as_ref()) {
                    self.font(&font, page, depth);
                }
            }
        }
        if let Some(xobjects) = res.get::<Dict<'_>>(b"XObject") {
            for key in xobjects.keys().collect::<Vec<_>>() {
                let Some(stream) = xobjects.get::<Stream<'_>>(key.as_ref()) else {
                    continue;
                };
                let dict = stream.dict().clone();
                if !self.first_visit(&dict) {
                    continue;
                }
                self.stream(&stream, Some(page));
                if dict.contains_key(b"OPI")
                    || dict
                        .get::<Name<'_>>(b"Subtype")
                        .is_some_and(|s| s.as_str() == "PS")
                {
                    self.add(
                        Rule::ExternalContent,
                        "A drawing refers to outside content",
                        Some(page),
                    );
                }
                if let Some(inner) = dict.get::<Dict<'_>>(b"Resources") {
                    self.resources(&inner, page, depth + 1);
                }
            }
        }
    }

    fn font(&mut self, font: &Dict<'_>, page: usize, depth: u8) {
        if !self.first_visit(font) {
            return;
        }
        let subtype = font
            .get::<Name<'_>>(b"Subtype")
            .map(|n| n.as_str().to_owned())
            .unwrap_or_default();
        let name = font
            .get::<Name<'_>>(b"BaseFont")
            .map(|n| n.as_str().to_owned())
            .unwrap_or_else(|| "A font".into());
        match subtype.as_str() {
            // Its glyphs are drawings in the file.
            "Type3" => {
                if let Some(res) = font.get::<Dict<'_>>(b"Resources") {
                    self.resources(&res, page, depth + 1);
                }
            }
            "Type0" => {
                let descendant = font
                    .get::<Array<'_>>(b"DescendantFonts")
                    .and_then(|a| a.iter::<Dict<'_>>().next());
                if let Some(d) = descendant {
                    self.font(&d, page, depth);
                }
            }
            _ => {
                let embedded = font.get::<Dict<'_>>(b"FontDescriptor").is_some_and(|d| {
                    d.contains_key(b"FontFile")
                        || d.contains_key(b"FontFile2")
                        || d.contains_key(b"FontFile3")
                });
                if !embedded {
                    // Subset prefixes such as ABCDEF+ say nothing to people.
                    let shown = name.split_once('+').map_or(name.as_str(), |(_, n)| n).to_owned();
                    self.add(Rule::FontNotEmbedded, shown, Some(page));
                }
            }
        }
    }

    fn actions(&mut self, dict: &Dict<'_>, page: Option<usize>, what: &str) {
        let runs_script = |action: &Dict<'_>| {
            action.get::<Name<'_>>(b"S").is_some_and(|s| {
                matches!(
                    s.as_str(),
                    "JavaScript"
                        | "Launch"
                        | "Sound"
                        | "Movie"
                        | "ImportData"
                        | "ResetForm"
                        | "Hide"
                        | "SetOCGState"
                        | "Rendition"
                        | "Trans"
                        | "GoTo3DView"
                )
            })
        };
        if dict.get::<Dict<'_>>(b"A").is_some_and(|a| runs_script(&a)) {
            self.add(
                Rule::Scripts,
                format!("{what} has an action PDF/A doesn't allow"),
                page,
            );
        }
        if dict.contains_key(b"AA") {
            self.add(Rule::Scripts, format!("{what} has automatic actions"), page);
        }
    }
}

pub(crate) fn check(pdf: &Pdf, encrypted: bool, has_id: bool, standard: Standard) -> Vec<Finding> {
    let archive = standard == Standard::PdfA;
    let mut c = Check {
        pdf,
        out: Vec::new(),
        seen: HashSet::new(),
        visits: 0,
        archive,
    };
    if encrypted {
        c.add(Rule::Encrypted, "Remove the password protection first", None);
    }
    if !has_id {
        c.add(Rule::NoFileId, "The file's trailer has no /ID", None);
    }
    let xref = c.pdf.xref();
    let catalog = xref.get::<Dict<'_>>(xref.root_id()).unwrap_or_default();

    match catalog.get::<Stream<'_>>(b"Metadata") {
        None => c.add(Rule::NoMetadata, "The document has no metadata stream", None),
        Some(meta) => {
            let xmp = meta.decoded().map(|d| d.into_owned()).unwrap_or_default();
            let text = String::from_utf8_lossy(&xmp);
            if archive && !text.contains("pdfaid:part") {
                c.add(Rule::NotMarkedPdfA, "No pdfaid:part in the metadata", None);
            }
            if !archive && !text.contains("PDF/X-4") {
                c.add(
                    Rule::NotMarkedPdfX,
                    "No pdfxid:GTS_PDFXVersion of PDF/X-4 in the metadata",
                    None,
                );
            }
        }
    }
    let intent = |kind: &str| {
        catalog.get::<Array<'_>>(b"OutputIntents").is_some_and(|a| {
            a.iter::<Dict<'_>>().any(|i| {
                i.get::<Name<'_>>(b"S").is_some_and(|s| s.as_str() == kind)
                    && (i.get::<Stream<'_>>(b"DestOutputProfile").is_some()
                        // A registered printing condition, such as FOGRA39, can stand in for
                        // the profile in PDF/X.
                        || (kind == "GTS_PDFX" && i.contains_key(b"OutputConditionIdentifier")))
            })
        })
    };
    if archive && !intent("GTS_PDFA1") {
        c.add(
            Rule::NoOutputIntent,
            "No PDF/A output intent with a colour profile",
            None,
        );
    }
    if !archive && !intent("GTS_PDFX") {
        c.add(
            Rule::NoPrintingCondition,
            "No PDF/X output intent with a printing condition or colour profile",
            None,
        );
    }
    if let Some(names) = catalog.get::<Dict<'_>>(b"Names") {
        if names.contains_key(b"JavaScript") {
            c.add(Rule::Scripts, "The document has JavaScript", None);
        }
        if archive && names.contains_key(b"EmbeddedFiles") {
            c.add(Rule::AttachedFiles, "The document has attached files", None);
        }
    }
    if let Some(open) = catalog.get::<Dict<'_>>(b"OpenAction")
        && open.get::<Name<'_>>(b"S").is_some_and(|s| s.as_str() != "GoTo")
    {
        c.add(Rule::Scripts, "Something runs when the document opens", None);
    }
    if catalog.contains_key(b"AA") {
        c.add(Rule::Scripts, "The document has automatic actions", None);
    }

    for (index, page) in pdf.pages().iter().enumerate() {
        let raw = page.raw();
        if !archive && !raw.contains_key(b"TrimBox") && !raw.contains_key(b"ArtBox") {
            c.add(Rule::NoTrimBox, "The page has no trim box", Some(index));
        }
        if raw.contains_key(b"AA") {
            c.add(Rule::Scripts, "A page has automatic actions", Some(index));
        }
        if let Some(contents) = raw.get::<Stream<'_>>(b"Contents") {
            c.stream(&contents, Some(index));
        } else if let Some(parts) = raw.get::<Array<'_>>(b"Contents") {
            for s in parts.iter::<Stream<'_>>() {
                c.stream(&s, Some(index));
            }
        }
        // hayro has already found the page's resources, inherited or not.
        let res = page.resources();
        let fonts = res.fonts.clone();
        for key in fonts.keys().collect::<Vec<_>>() {
            if let Some(font) = fonts.get::<Dict<'_>>(key.as_ref()) {
                c.font(&font, index, 0);
            }
        }
        let xobjects = res.x_objects.clone();
        for key in xobjects.keys().collect::<Vec<_>>() {
            let Some(stream) = xobjects.get::<Stream<'_>>(key.as_ref()) else {
                continue;
            };
            let dict = stream.dict().clone();
            if !c.first_visit(&dict) {
                continue;
            }
            c.stream(&stream, Some(index));
            if let Some(inner) = dict.get::<Dict<'_>>(b"Resources") {
                c.resources(&inner, index, 1);
            }
        }
        let Some(annots) = raw.get::<Array<'_>>(b"Annots") else {
            continue;
        };
        for annot in annots.iter::<Dict<'_>>().take(4_000) {
            let subtype = annot
                .get::<Name<'_>>(b"Subtype")
                .map(|n| n.as_str().to_owned())
                .unwrap_or_default();
            let what = if subtype == "Widget" {
                "A form field"
            } else {
                "A comment"
            };
            match subtype.as_str() {
                "Sound" | "Movie" | "Screen" | "3D" | "RichMedia" => {
                    c.add(
                        Rule::MediaAnnotation,
                        format!("{subtype} on the page"),
                        Some(index),
                    );
                    continue;
                }
                "FileAttachment" if archive => {
                    c.add(Rule::AttachedFiles, "A file attached to a comment", Some(index))
                }
                _ => {}
            }
            c.actions(&annot, Some(index), what);
            if !archive || subtype == "Popup" || subtype == "Link" {
                continue;
            }
            let rect_empty = annot
                .get::<Array<'_>>(b"Rect")
                .map(|r| r.iter::<f64>().collect::<Vec<_>>())
                .is_some_and(
                    |r| matches!(r[..], [x0, y0, x1, y1] if (x1 - x0).abs() < 1e-6 || (y1 - y0).abs() < 1e-6),
                );
            if !rect_empty
                && annot
                    .get::<Dict<'_>>(b"AP")
                    .and_then(|ap| ap.get::<Object<'_>>(b"N"))
                    .is_none()
            {
                c.add(Rule::NoAppearance, format!("{what} ({subtype})"), Some(index));
            }
            let flags = annot.get::<i64>(b"F").unwrap_or(0);
            // Print on; hidden, invisible and no-view off.
            if flags & 4 == 0 || flags & (1 | 2 | 32) != 0 {
                c.add(Rule::NotPrinted, format!("{what} ({subtype})"), Some(index));
            }
        }
    }
    c.out.sort_by_key(|f| (f.rule, f.page));
    c.out
}
