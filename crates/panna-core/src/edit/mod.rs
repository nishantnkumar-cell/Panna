//! Writing changes back into a PDF as an incremental update.

pub(crate) mod annot;
mod convert;
pub(crate) mod crypt;
mod field;
pub(crate) mod lex;
mod marks;
mod new_field;
mod outline;
pub(crate) use outline::with_outline;
mod view;
pub(crate) use view::with_initial_view;
mod pack;
mod pdfa;
mod picture;
mod rebuild;
mod redact;
pub(crate) mod scrub;
mod sign;
mod tag;
mod update;

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use hayro::hayro_interpret::TransformExt;
use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, ObjectIdentifier};

pub use annot::{Annotation, Color, Kind, LINE_HEIGHT, NOTE_SIZE, STAMPS, stamp_label, text_width, wrap};
pub use picture::StampImage;

pub use convert::{MAX_PICTURES, pictures_to_pdf};
pub use crypt::Protection;
pub use marks::{HeaderFooter, Watermark, fill as fill_header_footer, today};
pub use rebuild::{Cleanup, Margins, PageSource};
pub(crate) use sign::prepare as prepare_signature;
pub use sign::{Corner, SignOptions, Visible};
pub(crate) use tag::{describe, tag};

use crate::{CommentId, FieldValue, ObjRef};

/// Unsaved changes to a document's comments.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Changes {
    /// New annotations, oldest first.
    pub add: Vec<Annotation>,
    /// Saved comments to take out.
    pub remove: Vec<CommentId>,
    /// Form fields filled in, by the field's object, with their new values.
    pub fields: Vec<(ObjRef, FieldValue)>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty() && self.fields.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SaveError {
    /// The file's structure is too damaged to append to safely.
    Damaged,
    /// The file is encrypted in a way changes can't be saved into.
    Encrypted,
    NoSuchPage(usize),
    EngineCrashed,
    Io(String),
    /// The file's permissions don't allow printing it.
    PrintingNotAllowed,
    /// Cropping would leave too little of a page.
    CropTooLarge,
    /// Changing the password protection needs the owner password.
    ProtectionLocked,
    /// A marked area covers a form field, whose value would stay in the form.
    RedactsFormField,
    /// PDF/A files can't be password protected.
    ArchiveProtected,
    /// Signing password-protected files isn't supported.
    SigningProtected,
    /// Only the first signature can certify a document.
    CertifySigned,
    /// The certificate file couldn't be used, and why.
    Certificate(String),
    /// Tagging password-protected files isn't supported.
    TaggingProtected,
    /// The document has tags already.
    AlreadyTagged,
    /// No page has text to tag.
    NothingToTag,
    /// Text recognition found no words on the pages that needed it; `true` when every page
    /// asked about already has text.
    NothingRecognised(bool),
    /// The text recognition engine failed, and why.
    Recognition(String),
    /// Another file to take pages from couldn't be used, and why, as a sentence.
    OtherFile(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Damaged => write!(
                f,
                "This PDF's internal structure is damaged, so changes can't be added to it safely."
            ),
            Self::Encrypted => write!(
                f,
                "This PDF's password protection uses a method Panna can't save changes into."
            ),
            Self::NoSuchPage(i) => write!(f, "Page {} doesn't exist.", i + 1),
            Self::EngineCrashed => write!(f, "The PDF engine failed while preparing the changes."),
            Self::Io(e) => write!(f, "Couldn't write the file: {e}"),
            Self::CropTooLarge => write!(f, "Those margins would leave too little of the page."),
            Self::ProtectionLocked => write!(
                f,
                "Only the owner can change this PDF's password protection. Open it with the \
                 owner password first."
            ),
            Self::ArchiveProtected => write!(
                f,
                "PDF/A files can't have password protection. Remove it first, from Password \
                 protection in All tools."
            ),
            Self::SigningProtected => write!(
                f,
                "Password-protected PDFs can't be signed yet. Remove the protection first, from \
                 Password protection in All tools."
            ),
            Self::CertifySigned => write!(
                f,
                "This PDF is already signed, so it can't be certified: only the first signature \
                 can certify a document."
            ),
            Self::Certificate(why) | Self::OtherFile(why) => write!(f, "{why}"),
            Self::TaggingProtected => write!(
                f,
                "Panna can't add tags to a password-protected PDF. Remove the protection first, \
                 from Password protection in All tools."
            ),
            Self::AlreadyTagged => write!(f, "This PDF has tags already."),
            Self::NothingRecognised(true) => write!(
                f,
                "These pages have text already, so there's nothing to recognise."
            ),
            Self::NothingRecognised(false) => write!(f, "No text was found on these pages."),
            Self::Recognition(why) => write!(f, "Text recognition failed: {why}"),
            Self::NothingToTag => write!(
                f,
                "This PDF has no text to tag. If it's scanned, its text needs recognising first."
            ),
            Self::RedactsFormField => write!(
                f,
                "A marked area covers a form field. Flatten the form first (in the page tools), \
                 then redact."
            ),
            Self::PrintingNotAllowed => write!(
                f,
                "This PDF's author doesn't allow it to be printed. Opening it with the owner \
                 password allows printing."
            ),
        }
    }
}

impl std::error::Error for SaveError {}

/// Returns `original` with `changes` appended as an incremental update. With no changes the
/// bytes are returned unchanged.
///
/// `password` is the one the file was opened with; for encrypted files it is needed again to
/// encrypt what is added.
pub(crate) fn apply(
    pdf: &Pdf,
    original: &[u8],
    changes: &Changes,
    password: &str,
) -> Result<Vec<u8>, SaveError> {
    if changes.is_empty() {
        return Ok(original.to_vec());
    }
    let trailer = update::read_trailer(original)?;
    let crypt = match &trailer.encrypt {
        Some(raw) => Some(encryption(pdf, raw, trailer.id.as_deref(), password)?),
        None => None,
    };
    let mut out = update::Update::new(trailer);
    if let Some(crypt) = crypt {
        out.encrypt_with(crypt);
    }
    let date = pdf_date(SystemTime::now());
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());

    let mut by_page: BTreeMap<usize, Vec<&Annotation>> = BTreeMap::new();
    for a in &changes.add {
        by_page.entry(a.page).or_default().push(a);
    }
    let mut removed: BTreeMap<usize, HashSet<ObjRef>> = BTreeMap::new();
    for id in &changes.remove {
        by_page.entry(id.page).or_default();
        let refs = removed.entry(id.page).or_default();
        refs.insert(id.annot);
        refs.extend(id.popup);
    }
    let pages = pdf.pages();
    let mut serial = 0;
    for (index, list) in by_page {
        let page = pages.get(index).ok_or(SaveError::NoSuchPage(index))?;
        let raw = page.raw();
        let id = raw.obj_id().ok_or(SaveError::Damaged)?;
        let (num, generation) = (
            u32::try_from(id.obj_number).map_err(|_| SaveError::Damaged)?,
            u16::try_from(id.gen_number).map_err(|_| SaveError::Damaged)?,
        );
        let info = annot::PageInfo {
            to_user: page.initial_transform(true).to_kurbo().inverse(),
            page_ref: format!("{num} {generation} R"),
        };
        let mut refs = Vec::new();
        for a in list {
            let annot_id = out.alloc();
            let ap_id = a.kind.drawn().then(|| out.alloc());
            serial += 1;
            // An image stamp's picture, and its transparency, are objects of their own.
            let image_ref = match &a.kind {
                Kind::Stamp {
                    image: Some(image), ..
                } => {
                    let image_id = out.alloc();
                    let mask_id = image.has_mask().then(|| out.alloc());
                    let mask_ref = mask_id.map(|id| format!("{id} 0 R"));
                    let (body, mask) = image.objects(mask_ref.as_deref());
                    out.put(image_id, 0, body)?;
                    if let (Some(id), Some(mask)) = (mask_id, mask) {
                        out.put(id, 0, mask)?;
                    }
                    Some(format!("{image_id} 0 R"))
                }
                _ => None,
            };
            // Image stamps are named apart, so that reading the file back doesn't take them
            // for word stamps to redraw.
            let image_tag = if image_ref.is_some() { "image-" } else { "" };
            let built = a.build(
                &info,
                &ap_id.map_or_else(String::new, |id| format!("{id} 0 R")),
                image_ref.as_deref(),
                &format!("panna-{image_tag}{stamp:x}-{serial}"),
                &date,
            );
            out.put(annot_id, 0, built.dict)?;
            if let (Some(id), Some(appearance)) = (ap_id, built.appearance) {
                out.put(id, 0, appearance)?;
            }
            refs.push(format!("{annot_id} 0 R"));
        }
        // The page is rewritten with every entry copied byte for byte, except /Annots, which
        // keeps the existing annotations and adds the new ones.
        let data = raw.data();
        let entries = lex::dict_entries(data).ok_or(SaveError::Damaged)?;
        let mut dict = b"<<".to_vec();
        for (key, value) in &entries {
            if *key == b"Annots" {
                continue;
            }
            dict.push(b'/');
            dict.extend_from_slice(key);
            dict.push(b' ');
            dict.extend_from_slice(data.get(value.clone()).ok_or(SaveError::Damaged)?);
            dict.push(b'\n');
        }
        dict.extend_from_slice(b"/Annots [");
        if let Some(existing) = raw.get::<Array<'_>>(b"Annots") {
            let gone = removed.get(&index);
            let data = existing.data();
            for item in lex::array_items(data).ok_or(SaveError::Damaged)? {
                let value = data.get(item).ok_or(SaveError::Damaged)?;
                if gone.is_some_and(|gone| lex::reference(value).is_some_and(|r| gone.contains(&r))) {
                    continue;
                }
                dict.extend_from_slice(value);
                dict.push(b' ');
            }
        }
        dict.extend_from_slice(refs.join(" ").as_bytes());
        dict.extend_from_slice(b"]\n>>");
        out.put_copied(num, generation, dict);
    }
    fill_fields(pdf, &mut out, &changes.fields)?;
    Ok(out.finish(original))
}

/// `original` with an update that hides the layers in `hidden` and shows every other layer,
/// for viewing only: the catalog's default layer settings are replaced by just that choice.
pub(crate) fn with_layers(
    pdf: &Pdf,
    original: &[u8],
    layers: &[crate::ObjRef],
    hidden: &[crate::ObjRef],
) -> Result<Vec<u8>, SaveError> {
    let xref = pdf.xref();
    let root = xref.root_id();
    let catalog = xref
        .get::<hayro::hayro_syntax::object::Dict<'_>>(root)
        .ok_or(SaveError::Damaged)?;
    let (num, generation) = (
        u32::try_from(root.obj_number).map_err(|_| SaveError::Damaged)?,
        u16::try_from(root.gen_number).map_err(|_| SaveError::Damaged)?,
    );
    let data = catalog.data();
    let entries = lex::dict_entries(data).ok_or(SaveError::Damaged)?;
    let mut dict = b"<<".to_vec();
    for (key, value) in &entries {
        if *key == b"OCProperties" {
            continue;
        }
        dict.push(b'/');
        dict.extend_from_slice(key);
        dict.push(b' ');
        dict.extend_from_slice(data.get(value.clone()).ok_or(SaveError::Damaged)?);
        dict.push(b'\n');
    }
    let list = |refs: &[crate::ObjRef]| {
        refs.iter()
            .map(|r| format!("{} {} R", r.number, r.generation))
            .collect::<Vec<_>>()
            .join(" ")
    };
    dict.extend_from_slice(
        format!(
            "/OCProperties << /OCGs [{}] /D << /BaseState /ON /OFF [{}] >> >>\n>>",
            list(layers),
            list(hidden)
        )
        .as_bytes(),
    );
    let mut out = update::Update::new(update::read_trailer(original)?);
    out.put_copied(num, generation, dict);
    Ok(out.finish(original))
}

/// One document taking part in a rebuild: parsed, its bytes, and its password.
pub(crate) type Input<'a> = (&'a Pdf, &'a [u8], &'a str);

/// A whole new file with the pages in `plan`, taken from `docs` (the first is the one being
/// changed, whose catalog, document information and password the new file keeps).
pub(crate) fn rearrange(docs: &[Input<'_>], plan: &[PageSource]) -> Result<Vec<u8>, SaveError> {
    rebuild_from(docs, plan, &[], Output::Keep, Cleanup::default())
}

/// The first document with `count` of its pages from `first` replaced by the second's pages
/// from `from`. Bookmarks and links to a replaced page go to the page that replaced it.
pub(crate) fn replace(
    docs: &[Input<'_>],
    own: usize,
    first: usize,
    count: usize,
    from: usize,
) -> Result<Vec<u8>, SaveError> {
    let mut plan = Vec::with_capacity(own);
    let mut extras = Vec::with_capacity(own);
    for index in 0..own {
        let replaced = index >= first && index - first < count;
        plan.push(if replaced {
            PageSource::Page {
                doc: 1,
                index: from + (index - first),
                turns: 0,
            }
        } else {
            PageSource::Page {
                doc: 0,
                index,
                turns: 0,
            }
        });
        extras.push(rebuild::Extra {
            stands_for: replaced.then_some(index),
            ..rebuild::Extra::default()
        });
    }
    rebuild_from(docs, &plan, &extras, Output::Keep, Cleanup::default())
}

/// Every page of `doc`, unchanged and unturned.
fn every_page(count: usize) -> Vec<PageSource> {
    (0..count)
        .map(|index| PageSource::Page {
            doc: 0,
            index,
            turns: 0,
        })
        .collect()
}

/// Every page of `doc`, with the pages at `pages` cropped by `margins`.
pub(crate) fn crop(
    doc: Input<'_>,
    count: usize,
    pages: &[usize],
    margins: Margins,
) -> Result<Vec<u8>, SaveError> {
    let extras: Vec<rebuild::Extra> = (0..count)
        .map(|i| rebuild::Extra {
            crop: pages.contains(&i).then_some(margins),
            ..rebuild::Extra::default()
        })
        .collect();
    rebuild_from(
        &[doc],
        &every_page(count),
        &extras,
        Output::Keep,
        Cleanup::default(),
    )
}

/// Every page of `doc`, with `marks` drawn on the pages at `pages`, numbered in that order.
pub(crate) fn header_footer(
    doc: Input<'_>,
    count: usize,
    pages: &[usize],
    marks: &HeaderFooter,
) -> Result<Vec<u8>, SaveError> {
    let date = marks::today();
    let mut extras = vec![rebuild::Extra::default(); count];
    for (i, &page) in pages.iter().enumerate() {
        if let Some(extra) = extras.get_mut(page) {
            let number = u64::from(marks.first) + i as u64;
            extra
                .over
                .push(marks::Drawing::Lines(marks.for_page(number, pages.len(), &date)));
        }
    }
    rebuild_from(
        &[doc],
        &every_page(count),
        &extras,
        Output::Keep,
        Cleanup::default(),
    )
}

pub(crate) use marks::Words;

/// Every page of `doc`, with the words recognised on some pages drawn invisibly over them
/// (each word as text the size of its box, as page text boxes), so they can be found,
/// selected and copied.
pub(crate) fn with_hidden_text(
    doc: Input<'_>,
    count: usize,
    words: &[(usize, Words)],
) -> Result<Vec<u8>, SaveError> {
    let mut extras = vec![rebuild::Extra::default(); count];
    for (page, words) in words {
        let extra = extras.get_mut(*page).ok_or(SaveError::NoSuchPage(*page))?;
        extra.over.push(marks::Drawing::Hidden(words.clone()));
    }
    rebuild_from(
        &[doc],
        &every_page(count),
        &extras,
        Output::Keep,
        Cleanup::default(),
    )
}

/// Every page of `doc`, without the hidden information `clean` names.
pub(crate) fn clean_up(doc: Input<'_>, count: usize, clean: Cleanup) -> Result<Vec<u8>, SaveError> {
    rebuild_from(&[doc], &every_page(count), &[], Output::Keep, clean)
}

/// Whether the file `bytes` has an identifier (/ID) in its trailer.
pub(crate) fn has_file_id(bytes: &[u8]) -> bool {
    update::read_trailer(bytes).is_ok_and(|t| t.id.is_some())
}

/// Whether the file `bytes` has password protection.
pub(crate) fn protected(bytes: &[u8]) -> bool {
    update::read_trailer(bytes).is_ok_and(|t| t.encrypt.is_some())
}

/// For a file protected with RC4 or AES-128 and opened with the owner password: the same
/// protection with AES-256, and the encryption method's name for any protected file.
pub(crate) fn protection_method(doc: Input<'_>) -> Option<(&'static str, Option<Protection>)> {
    let (pdf, bytes, password) = doc;
    let trailer = update::read_trailer(bytes).ok()?;
    let crypt = encryption(pdf, trailer.encrypt.as_deref()?, trailer.id.as_deref(), password).ok()?;
    Some((crypt.method_name(), crypt.upgrade(password)))
}

/// Every page of `doc` with new password protection, or none.
pub(crate) fn protect(
    doc: Input<'_>,
    count: usize,
    protection: Option<&Protection>,
) -> Result<Vec<u8>, SaveError> {
    rebuild_from(
        &[doc],
        &every_page(count),
        &[],
        Output::Change(protection),
        Cleanup::default(),
    )
}

/// Every page of `doc`, with its comments, its form fields or both drawn into the pages. With
/// the fields flattened, the document no longer has a form.
pub(crate) fn flatten(
    doc: Input<'_>,
    count: usize,
    comments: bool,
    fields: bool,
) -> Result<Vec<u8>, SaveError> {
    let extra = rebuild::Extra {
        flatten: rebuild::Flatten { comments, fields },
        ..rebuild::Extra::default()
    };
    rebuild_from(
        &[doc],
        &every_page(count),
        &vec![extra; count],
        Output::Keep,
        Cleanup::default(),
    )
}

/// Every page of `doc`, with `mark` drawn on the pages at `pages`, numbered in that order.
pub(crate) fn watermark(
    doc: Input<'_>,
    count: usize,
    pages: &[usize],
    mark: &Watermark,
) -> Result<Vec<u8>, SaveError> {
    let date = marks::today();
    let mut extras = vec![rebuild::Extra::default(); count];
    for (i, &page) in pages.iter().enumerate() {
        if let Some(extra) = extras.get_mut(page) {
            let (under, over) = mark.for_page(1 + i as u64, pages.len(), &date);
            extra.under.extend(under);
            extra.over.extend(over);
        }
    }
    rebuild_from(
        &[doc],
        &every_page(count),
        &extras,
        Output::Keep,
        Cleanup::default(),
    )
}

/// A change to a picture a page draws.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PictureChange {
    /// To this box, in the same space as [`crate::TextBox`] page text.
    Move(crate::TextBox),
    /// With this picture, keeping its shape, in the old one's place.
    Replace(StampImage),
    Delete,
}

/// `doc` with the pictures page `page` draws changed: each by its place in the page's list
/// of pictures.
pub(crate) fn change_pictures(
    doc: Input<'_>,
    count: usize,
    page: usize,
    changes: &[(usize, PictureChange)],
) -> Result<Vec<u8>, SaveError> {
    let mut extras = vec![rebuild::Extra::default(); count];
    let extra = extras.get_mut(page).ok_or(SaveError::NoSuchPage(page))?;
    extra.pictures = changes.to_vec();
    rebuild_from(
        &[doc],
        &every_page(count),
        &extras,
        Output::Keep,
        Cleanup::default(),
    )
}

/// The pictures page `page` draws, each as the box it covers.
pub(crate) fn page_pictures(pdf: &Pdf, page: usize) -> Vec<crate::TextBox> {
    rebuild::page_pictures(pdf, page)
}

/// The rectangles and straight lines page `page` draws, as shown.
pub(crate) fn page_shapes(pdf: &Pdf, page: usize) -> (Vec<crate::TextBox>, Vec<crate::TextBox>) {
    rebuild::page_shapes(pdf, page)
}

/// A form field to make.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NewField {
    pub page: usize,
    /// Where it goes, in the same space as [`crate::TextBox`] page text.
    pub rect: crate::TextBox,
    pub name: String,
    /// Shown when the pointer rests on it.
    pub tooltip: String,
    pub required: bool,
    pub kind: NewFieldKind,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum NewFieldKind {
    Text {
        multiline: bool,
    },
    CheckBox,
    /// A drop-down list of these choices.
    Dropdown(Vec<String>),
    /// A list box showing these choices, one of which can be picked.
    List(Vec<String>),
    /// A group of radio buttons, one per choice, on the field's page; only one can be on.
    /// The field's own `rect` isn't used.
    Radio(Vec<RadioButton>),
}

/// One button of a new radio group.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RadioButton {
    /// What the group's value is when this button is on.
    pub choice: String,
    /// Where it goes, in the same space as [`crate::TextBox`] page text.
    pub rect: crate::TextBox,
}

/// `doc` with the fields in `add` added to the form, and the fields `drop` (with their
/// widgets `widgets`) taken out of it.
pub(crate) fn form_fields(
    doc: Input<'_>,
    count: usize,
    add: &[NewField],
    drop: &[ObjRef],
    widgets: &[(usize, ObjRef)],
) -> Result<Vec<u8>, SaveError> {
    let mut extras = vec![rebuild::Extra::default(); count];
    for field in add {
        let extra = extras
            .get_mut(field.page)
            .ok_or(SaveError::NoSuchPage(field.page))?;
        extra.fields.push(field.clone());
    }
    for (page, widget) in widgets {
        if let Some(extra) = extras.get_mut(*page) {
            extra.drop_annots.push(*widget);
        }
    }
    if let Some(first) = extras.first_mut() {
        first.drop_fields.extend_from_slice(drop);
    }
    rebuild_from(
        &[doc],
        &every_page(count),
        &extras,
        Output::Keep,
        Cleanup::default(),
    )
}

/// Something added to a page itself, rather than as a comment. Places are in points from the
/// top-left corner of the page as shown (the same space as [`crate::TextBox`]).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Addition {
    /// Lines of text in Helvetica, the first line's top-left corner at `at`.
    Text {
        at: (f32, f32),
        text: String,
        size: f32,
        color: Color,
    },
    /// A picture filling `rect`.
    Picture { rect: crate::TextBox, image: StampImage },
    /// An area that opens a web or email address, or goes to a page, when clicked.
    Link {
        rect: crate::TextBox,
        target: crate::LinkTarget,
    },
    /// The text in `area` taken out (only the text: pictures and drawings stay) and `text`
    /// written in its place in Helvetica, at `size`, in `color` or the colour the old text
    /// was drawn in. With a `leading` (the distance between baselines), `area` is a
    /// paragraph and the text is wrapped to its width.
    ReplaceText {
        area: crate::TextBox,
        text: String,
        size: f32,
        color: Option<Color>,
        leading: Option<f32>,
    },
}

/// Every page of `doc`, with `additions` added to page `page`.
pub(crate) fn add(
    doc: Input<'_>,
    count: usize,
    page: usize,
    additions: &[Addition],
) -> Result<Vec<u8>, SaveError> {
    let mut extras = vec![rebuild::Extra::default(); count];
    let extra = extras.get_mut(page).ok_or(SaveError::NoSuchPage(page))?;
    let corners = |r: &crate::TextBox| [r.x0, r.y0, r.x1, r.y1].map(f64::from);
    for addition in additions {
        match addition {
            Addition::Text {
                at,
                text,
                size,
                color,
            } => {
                let lines: Vec<String> = text.lines().map(str::to_owned).collect();
                if lines.iter().all(|l| l.trim().is_empty()) {
                    continue;
                }
                extra.over.push(marks::Drawing::Text {
                    x: f64::from(at.0),
                    top: f64::from(at.1),
                    lines,
                    size: f64::from(size.clamp(1.0, 500.0)),
                    leading: f64::from(size.clamp(1.0, 500.0) * annot::LINE_HEIGHT),
                    color: *color,
                });
            }
            Addition::Picture { rect, image } => extra.over.push(marks::Drawing::Picture {
                rect: corners(rect),
                image: image.clone(),
            }),
            Addition::Link { rect, target } => extra.links.push((corners(rect), target.clone())),
            Addition::ReplaceText {
                area,
                text,
                size,
                color,
                leading,
            } => {
                extra.redact.push(corners(area));
                extra.text_only = true;
                let size = f64::from(size.clamp(1.0, 500.0));
                let paragraph = leading.map(|l| f64::from(l).clamp(size * 0.8, size * 4.0));
                let lines: Vec<String> = match paragraph {
                    // A little room, as the measured width is the longest line's.
                    Some(_) => annot::wrap(text.trim_end(), f64::from(area.width()) * 1.02 / size),
                    None => text.lines().map(str::to_owned).collect(),
                };
                let same = match lines.as_slice() {
                    [line] if !line.trim().is_empty() => rebuild::same_font(doc.0, page, corners(area), line),
                    _ => None,
                };
                if let Some(line) = same {
                    extra.over.push(marks::Drawing::SameFont {
                        line,
                        color: color.unwrap_or(Color::BLACK),
                    });
                } else if lines.iter().any(|l| !l.trim().is_empty()) {
                    extra.over.push(marks::Drawing::Text {
                        x: f64::from(area.x0),
                        // Page text boxes reach 0.8 of the size above the baseline and 0.2
                        // below; added text's baseline is 0.77 below its top.
                        top: match paragraph {
                            Some(_) => f64::from(area.y0) + size * 0.03,
                            None => f64::from(area.y1) - size * 0.97,
                        },
                        lines,
                        size,
                        leading: paragraph.unwrap_or(size * f64::from(annot::LINE_HEIGHT)),
                        color: color.unwrap_or(Color::BLACK),
                    });
                }
            }
        }
    }
    rebuild_from(
        &[doc],
        &every_page(count),
        &extras,
        Output::Keep,
        Cleanup::default(),
    )
}

/// `doc` with everything inside `areas` taken out for good (text, pictures, drawings and
/// comments), and black boxes left in their place. Each page's areas are left, top, right,
/// bottom from the top-left corner of the page as shown.
pub(crate) fn redact(
    doc: Input<'_>,
    count: usize,
    areas: &[(usize, Vec<crate::TextBox>)],
    scrub: &[String],
) -> Result<Vec<u8>, SaveError> {
    let mut extras = vec![rebuild::Extra::default(); count];
    if let Some(first) = extras.first_mut() {
        first.scrub = scrub::phrases(scrub.iter().map(String::as_str));
    }
    for (page, boxes) in areas {
        let extra = extras.get_mut(*page).ok_or(SaveError::NoSuchPage(*page))?;
        let rects: Vec<[f64; 4]> = boxes
            .iter()
            .map(|r| [r.x0.min(r.x1), r.y0.min(r.y1), r.x0.max(r.x1), r.y0.max(r.y1)].map(f64::from))
            .filter(|[x0, y0, x1, y1]| x1 > x0 && y1 > y0)
            .collect();
        if rects.is_empty() {
            continue;
        }
        extra.redact.extend_from_slice(&rects);
        extra.over.push(marks::Drawing::Boxes {
            rects,
            color: crate::Color([0.0, 0.0, 0.0]),
        });
    }
    rebuild_from(
        &[doc],
        &every_page(count),
        &extras,
        Output::Keep,
        Cleanup::default(),
    )
}

/// Every page of `doc` in a file without password protection, to hand to the printing
/// system, if the file's permissions allow printing.
pub(crate) fn print_copy(doc: Input<'_>, pages: usize) -> Result<Vec<u8>, SaveError> {
    rebuild_from(&[doc], &every_page(pages), &[], Output::Print, Cleanup::default())
}

/// What password protection a rebuilt file gets.
#[derive(Clone, Copy)]
enum Output<'a> {
    /// The first document's, as it was.
    Keep,
    /// None, for the printing system, if the permissions allow printing.
    Print,
    /// This, or none, if the owner (or the permissions) allow changing it.
    Change(Option<&'a Protection>),
}

fn rebuild_from(
    docs: &[Input<'_>],
    plan: &[PageSource],
    extras: &[rebuild::Extra],
    output: Output<'_>,
    clean: Cleanup,
) -> Result<Vec<u8>, SaveError> {
    rebuild_with(docs, plan, extras, output, clean, None)
}

/// Every page of `doc` in as small a file as can be written without changing how anything
/// looks (see [`pack`]), unless pictures may be made smaller too: then those sharper than
/// `pictures` dots per inch, as drawn, are scaled down to it.
pub(crate) fn compacted(doc: Input<'_>, count: usize, pictures: Option<u32>) -> Result<Vec<u8>, SaveError> {
    rebuild_with(
        &[doc],
        &every_page(count),
        &[],
        Output::Keep,
        Cleanup::default(),
        Some(pack::Compact { pictures }),
    )
}

fn rebuild_with(
    docs: &[Input<'_>],
    plan: &[PageSource],
    extras: &[rebuild::Extra],
    output: Output<'_>,
    clean: Cleanup,
    compact: Option<pack::Compact>,
) -> Result<Vec<u8>, SaveError> {
    let mut sources = Vec::with_capacity(docs.len());
    let mut first = None;
    for (i, &(pdf, bytes, password)) in docs.iter().enumerate() {
        let trailer = update::read_trailer(bytes)?;
        let crypt = match &trailer.encrypt {
            Some(raw) => Some(encryption(pdf, raw, trailer.id.as_deref(), password)?),
            None => None,
        };
        if i == 0 {
            let encryption = match output {
                Output::Print if crypt.as_ref().is_some_and(|c| !c.may_print()) => {
                    return Err(SaveError::PrintingNotAllowed);
                }
                Output::Change(_) if crypt.as_ref().is_some_and(|c| !c.may_change_protection()) => {
                    return Err(SaveError::ProtectionLocked);
                }
                Output::Change(Some(protection)) => {
                    let (crypt, dict, id) = protection.seal()?;
                    Some(rebuild::Encryption { crypt, dict, id })
                }
                Output::Keep => match (&trailer.encrypt, &crypt, &trailer.id) {
                    (Some(raw), Some(crypt), Some(id)) => Some(rebuild::Encryption {
                        crypt: crypt.clone(),
                        dict: encrypt_dict(pdf, raw)?,
                        id: id.clone(),
                    }),
                    _ => None,
                },
                Output::Print | Output::Change(None) => None,
            };
            let info = trailer.info.as_deref().and_then(lex::reference);
            first = Some((info, encryption));
        }
        sources.push(rebuild::Source { pdf, bytes, crypt });
    }
    let (info, encryption) = first.ok_or(SaveError::Damaged)?;
    rebuild::rebuild(&sources, plan, extras, info, encryption, clean, compact)
}

/// The /Encrypt dictionary's bytes, whether the trailer holds it or refers to it.
fn encrypt_dict(pdf: &Pdf, raw: &[u8]) -> Result<Vec<u8>, SaveError> {
    let raw = raw.trim_ascii();
    if raw.starts_with(b"<<") {
        return Ok(raw.to_vec());
    }
    let r = lex::reference(raw).ok_or(SaveError::Damaged)?;
    pdf.xref()
        .get::<Dict<'_>>(ObjectIdentifier::new(
            i32::try_from(r.number).map_err(|_| SaveError::Damaged)?,
            i32::from(r.generation),
        ))
        .map(|d| d.data().to_vec())
        .ok_or(SaveError::Damaged)
}

/// Writes filled-in field values: each field's /V, a new appearance for text and choice
/// widgets, and the on or off state of checkbox and radio widgets.
fn fill_fields(
    pdf: &Pdf,
    out: &mut update::Update,
    values: &[(ObjRef, FieldValue)],
) -> Result<(), SaveError> {
    if values.is_empty() {
        return Ok(());
    }
    let fields = crate::forms::read(pdf);
    let xref = pdf.xref();
    let get = |r: ObjRef| {
        xref.get::<Dict<'_>>(ObjectIdentifier::new(
            i32::try_from(r.number).unwrap_or(-1),
            i32::from(r.generation),
        ))
    };
    // Every edit to one object, so a field that is also its own widget is written once.
    let mut edits: BTreeMap<(u32, u16), field::Entries> = BTreeMap::new();
    for (id, value) in values {
        let Some(f) = fields.iter().find(|f| f.id == *id) else {
            continue;
        };
        edits
            .entry((id.number, id.generation))
            .or_default()
            .push(("V", field::value(f, value)));
        for w in &f.widgets {
            let entry = match value {
                FieldValue::State(state) => {
                    let on = state.is_some() && w.on == *state;
                    let shown = if on {
                        w.on.as_deref().unwrap_or("Off")
                    } else {
                        "Off"
                    };
                    ("AS", field::name(shown))
                }
                FieldValue::Text(text) => {
                    let Some(widget) = get(w.annot) else { continue };
                    let Some(stream) = field::appearance(f, &widget, text) else {
                        continue;
                    };
                    let ap = out.alloc();
                    out.put(ap, 0, stream)?;
                    ("AP", format!("<< /N {ap} 0 R >>"))
                }
            };
            edits
                .entry((w.annot.number, w.annot.generation))
                .or_default()
                .push(entry);
        }
    }
    for ((number, generation), set) in edits {
        let dict = get(ObjRef { number, generation }).ok_or(SaveError::Damaged)?;
        let set = set
            .into_iter()
            .map(|(key, value)| Ok((key, out.seal_value(number, generation, value)?)))
            .collect::<Result<field::Entries, SaveError>>()?;
        out.put_copied(number, generation, rewrite(dict.data(), &set)?);
    }
    Ok(())
}

/// For an RC4 or AES-128 file and its owner password, the user password, which is what opens
/// it for reading (the owner password is still what saving needs). Works on the raw bytes, as
/// the file can't be parsed without it.
pub(crate) fn user_password_for(bytes: &[u8], owner: &str) -> Option<zeroize::Zeroizing<Vec<u8>>> {
    let trailer = update::read_trailer(bytes).ok()?;
    let raw = trailer.encrypt.as_deref()?.trim_ascii();
    let dict = if raw.starts_with(b"<<") {
        raw
    } else {
        // The encryption dictionary is never in an object stream, so it is "N G obj" in the
        // file itself; the last one is the current one.
        let r = lex::reference(raw)?;
        let head = format!("{} {} obj", r.number, r.generation);
        let at = bytes
            .windows(head.len())
            .enumerate()
            .rev()
            .find(|(i, w)| {
                *w == head.as_bytes()
                    && (*i == 0 || bytes.get(i - 1).is_some_and(|b| b.is_ascii_whitespace()))
            })
            .map(|(i, _)| i + head.len())?;
        bytes.get(at..)?.trim_ascii_start()
    };
    let first_id = trailer
        .id
        .as_deref()
        .and_then(|id| {
            let inner = id.trim_ascii().strip_prefix(b"[")?.strip_suffix(b"]")?;
            let items = lex::array_items(inner)?;
            crypt::parse_string(inner.get(items.first()?.clone())?)
        })
        .unwrap_or_default();
    let crypt = crypt::Crypt::new(dict, &first_id, owner).ok()?;
    crypt.user_password().map(|p| p.to_vec().into())
}

/// The file key for an encrypted file, from its /Encrypt dictionary (inline or referenced),
/// the first part of its /ID, and the password it was opened with.
fn encryption(pdf: &Pdf, raw: &[u8], id: Option<&[u8]>, password: &str) -> Result<crypt::Crypt, SaveError> {
    let first_id = id
        .and_then(|id| {
            let inner = id.trim_ascii().strip_prefix(b"[")?.strip_suffix(b"]")?;
            let items = lex::array_items(inner)?;
            crypt::parse_string(inner.get(items.first()?.clone())?)
        })
        .unwrap_or_default();
    let raw = raw.trim_ascii();
    if raw.starts_with(b"<<") {
        return crypt::Crypt::new(raw, &first_id, password);
    }
    let r = lex::reference(raw).ok_or(SaveError::Damaged)?;
    let dict = pdf
        .xref()
        .get::<Dict<'_>>(ObjectIdentifier::new(
            i32::try_from(r.number).map_err(|_| SaveError::Damaged)?,
            i32::from(r.generation),
        ))
        .ok_or(SaveError::Damaged)?;
    crypt::Crypt::new(dict.data(), &first_id, password)
}

/// A dictionary with `set` replacing (or adding) those keys, everything else copied byte for
/// byte.
fn rewrite(data: &[u8], set: &field::Entries) -> Result<Vec<u8>, SaveError> {
    let entries = lex::dict_entries(data).ok_or(SaveError::Damaged)?;
    let mut dict = b"<<".to_vec();
    for (key, value) in &entries {
        if set.iter().any(|(k, _)| k.as_bytes() == *key) {
            continue;
        }
        dict.push(b'/');
        dict.extend_from_slice(key);
        dict.push(b' ');
        dict.extend_from_slice(data.get(value.clone()).ok_or(SaveError::Damaged)?);
        dict.push(b'\n');
    }
    for (key, value) in set {
        dict.extend_from_slice(format!("/{key} {value}\n").as_bytes());
    }
    dict.extend_from_slice(b">>");
    Ok(dict)
}

/// A PDF date string in UTC, like `D:20261017093000Z`.
fn pdf_date(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "D:{year:04}{month:02}{day:02}{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// A small PDF from numbered objects (1-based, the first the catalog) with a classic
/// cross-reference table.
pub(crate) fn assemble(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn dates_are_utc_civil_dates() {
        let t = UNIX_EPOCH + Duration::from_secs(1_792_229_400);
        assert_eq!(pdf_date(t), "D:20261017093000Z");
        assert_eq!(pdf_date(UNIX_EPOCH), "D:19700101000000Z");
        assert_eq!(
            pdf_date(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "D:20000229000000Z"
        );
    }
}
