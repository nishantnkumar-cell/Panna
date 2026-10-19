use std::fmt;
use std::fs::File;
use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::peniko::ImageAlphaType;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

use crate::Limits;

/// Size of a page in PDF points (1/72 inch), after applying the page's rotation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

/// What the UI needs to lay out a document before any page is rendered.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentInfo {
    pub pages: Vec<PageSize>,
    pub title: Option<String>,
    /// Label/value pairs for a document properties dialog, in display order.
    pub properties: Vec<(&'static str, String)>,
    /// Bookmarks, flattened in reading order.
    pub outline: Vec<crate::OutlineItem>,
    /// Comments already in the file, in page order.
    pub comments: Vec<crate::Comment>,
    /// The form's fields, if it has a form.
    pub fields: Vec<crate::Field>,
    /// Links on the pages, in page order.
    pub links: Vec<crate::Link>,
    /// Whether the file has password protection.
    pub protected: bool,
    /// Files attached to the document and its pages.
    pub attachments: Vec<crate::Attachment>,
    /// Layers that can be shown or hidden.
    pub layers: Vec<crate::Layer>,
    /// Digital signatures, each checked.
    pub signatures: Vec<crate::Signature>,
}

/// An RGBA8 bitmap (straight alpha, row-major, no padding).
#[derive(Clone, PartialEq, Eq)]
pub struct RenderedPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl RenderedPage {
    /// Returns the bitmap turned clockwise by `quarter_turns` × 90°.
    pub fn rotated(self, quarter_turns: u8) -> Self {
        let turns = quarter_turns % 4;
        if turns == 0 {
            return self;
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let (out_w, out_h) = if turns == 2 { (w, h) } else { (h, w) };
        let mut rgba = vec![0u8; self.rgba.len()];
        for (src, px) in self.rgba.as_chunks::<4>().0.iter().enumerate() {
            let (x, y) = (src % w, src / w);
            let (nx, ny) = match turns {
                1 => (h - 1 - y, x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            let dst = (ny * out_w + nx) * 4;
            if let Some(slot) = rgba.get_mut(dst..dst + 4) {
                slot.copy_from_slice(px);
            }
        }
        Self {
            width: out_w as u32,
            height: out_h as u32,
            rgba,
        }
    }

    /// The part of the bitmap inside `area`, given as `[left, top, right, bottom]` fractions of
    /// its width and height. At least one pixel is kept.
    pub fn cropped(self, area: [f32; 4]) -> Self {
        let (w, h) = (self.width as usize, self.height as usize);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let at = |f: f32, size: usize| ((f.clamp(0.0, 1.0) * size as f32).round() as usize).min(size);
        let [l, t, r, b] = area;
        let (x0, x1) = (at(l.min(r), w), at(l.max(r), w));
        let (y0, y1) = (at(t.min(b), h), at(t.max(b), h));
        let x0 = x0.min(w.saturating_sub(1));
        let y0 = y0.min(h.saturating_sub(1));
        let (cw, ch) = (x1.max(x0 + 1).min(w) - x0, y1.max(y0 + 1).min(h) - y0);
        if w == 0 || h == 0 {
            return self;
        }
        let mut rgba = Vec::with_capacity(cw * ch * 4);
        for row in y0..y0 + ch {
            let start = (row * w + x0) * 4;
            rgba.extend_from_slice(self.rgba.get(start..start + cw * 4).unwrap_or_default());
        }
        Self {
            width: cw as u32,
            height: ch as u32,
            rgba,
        }
    }
}

impl fmt::Debug for RenderedPage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderedPage")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    Io(String),
    TooLarge {
        bytes: u64,
        limit: u64,
    },
    TooManyPages {
        pages: usize,
        limit: usize,
    },
    PasswordRequired,
    UnsupportedEncryption,
    Invalid,
    EngineCrashed,
    /// The file holds data built to exhaust memory when decoded.
    Hostile(crate::guard::Refusal),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use crate::guard::Refusal;
        let mb = |b: &u64| b / 1_000_000;
        match self {
            Self::Hostile(Refusal::Stream { limit, .. }) => write!(
                f,
                "This file wasn't opened: part of it expands to more than {} MB when unpacked. \
                 Files built like this are used to crash PDF readers.",
                mb(limit)
            ),
            Self::Hostile(Refusal::Page { page, limit, .. }) => write!(
                f,
                "This file wasn't opened: page {} expands to more than {} MB when unpacked. \
                 Files built like this are used to crash PDF readers.",
                page + 1,
                mb(limit)
            ),
            Self::Hostile(Refusal::Image { width, height, .. }) => write!(
                f,
                "This file wasn't opened: it has a picture that claims to be {width} × {height} \
                 pixels, too large to show safely."
            ),
            Self::Io(e) => write!(f, "Couldn't read the file: {e}"),
            Self::TooLarge { bytes, limit } => write!(
                f,
                "The file is {} MB, which is over the {} MB limit.",
                bytes / 1_000_000,
                limit / 1_000_000
            ),
            Self::TooManyPages { pages, limit } => {
                write!(
                    f,
                    "The document has {pages} pages, which is over the {limit} page limit."
                )
            }
            Self::PasswordRequired => write!(f, "This PDF is password-protected."),
            Self::UnsupportedEncryption => write!(f, "This PDF uses an unsupported encryption method."),
            Self::Invalid => write!(f, "This file isn't a valid PDF, or it is too damaged to open."),
            Self::EngineCrashed => write!(f, "The PDF engine failed while reading this file."),
        }
    }
}

impl std::error::Error for OpenError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    NoSuchPage(usize),
    BadPageSize,
    EngineCrashed,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchPage(i) => write!(f, "Page {} doesn't exist.", i + 1),
            Self::BadPageSize => write!(f, "The page has an invalid size."),
            Self::EngineCrashed => write!(f, "The PDF engine failed while drawing this page."),
        }
    }
}

impl std::error::Error for RenderError {}

/// A parsed PDF document. Not `Sync`-shared: each document lives on one worker thread.
pub struct Document {
    pdf: Pdf,
    /// The file as read, kept for incremental saves.
    bytes: Arc<Vec<u8>>,
    limits: Limits,
    settings: InterpreterSettings,
    file_len: u64,
    /// The password the file was opened with, needed again to save into an encrypted file.
    password: String,
}

impl Document {
    /// Reads and parses a file, refusing anything over `limits.max_file_bytes`.
    pub fn open(path: &Path, password: Option<&str>, limits: Limits) -> Result<Self, OpenError> {
        let file = File::open(path).map_err(|e| OpenError::Io(e.to_string()))?;
        let len = file.metadata().map_err(|e| OpenError::Io(e.to_string()))?.len();
        if len > limits.max_file_bytes {
            return Err(OpenError::TooLarge {
                bytes: len,
                limit: limits.max_file_bytes,
            });
        }
        // `take` guards against the file growing between the size check and the read.
        let mut bytes = Vec::new();
        file.take(limits.max_file_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| OpenError::Io(e.to_string()))?;
        if bytes.len() as u64 > limits.max_file_bytes {
            return Err(OpenError::TooLarge {
                bytes: bytes.len() as u64,
                limit: limits.max_file_bytes,
            });
        }
        Self::from_bytes(bytes, password, limits)
    }

    /// Parses PDF bytes that are already in memory.
    pub fn from_bytes(bytes: Vec<u8>, password: Option<&str>, limits: Limits) -> Result<Self, OpenError> {
        if bytes.len() as u64 > limits.max_file_bytes {
            return Err(OpenError::TooLarge {
                bytes: bytes.len() as u64,
                limit: limits.max_file_bytes,
            });
        }
        let file_len = bytes.len() as u64;
        let password = password.unwrap_or("");
        let bytes = Arc::new(bytes);
        let parsed = catch_unwind(AssertUnwindSafe(|| {
            Pdf::new_with_password(Arc::clone(&bytes), password)
        }))
        .map_err(|_| OpenError::EngineCrashed)?;
        let pdf = parsed.map_err(|e| match e {
            LoadPdfError::Decryption(d) => decryption_error(d),
            LoadPdfError::Invalid => OpenError::Invalid,
        })?;
        let pages =
            catch_unwind(AssertUnwindSafe(|| pdf.pages().len())).map_err(|_| OpenError::EngineCrashed)?;
        if pages > limits.max_pages {
            return Err(OpenError::TooManyPages {
                pages,
                limit: limits.max_pages,
            });
        }
        catch_unwind(AssertUnwindSafe(|| crate::guard::check(&pdf, &limits)))
            .map_err(|_| OpenError::EngineCrashed)?
            .map_err(OpenError::Hostile)?;
        Ok(Self {
            pdf,
            bytes,
            limits,
            settings: InterpreterSettings::default(),
            file_len,
            password: password.to_owned(),
        })
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// The size of page `index` as shown, in points, or `None` past the last page.
    pub fn page_size(&self, index: usize) -> Option<PageSize> {
        let pages = self.pdf.pages();
        let (width, height) = pages.get(index)?.render_dimensions();
        Some(PageSize { width, height })
    }

    /// Page sizes and metadata for layout.
    pub fn info(&self) -> Result<DocumentInfo, OpenError> {
        catch_unwind(AssertUnwindSafe(|| {
            let pages = self
                .pdf
                .pages()
                .iter()
                .map(|page| {
                    let (width, height) = page.render_dimensions();
                    PageSize { width, height }
                })
                .collect();
            let meta = self.pdf.metadata();
            let text = |v: &Option<Vec<u8>>| {
                // Control characters (such as a C string's closing NUL) are dropped.
                v.as_deref()
                    .map(|t| {
                        let t: String = decode_text_string(t)
                            .chars()
                            .filter(|c| !c.is_control())
                            .collect();
                        t.trim().to_owned()
                    })
                    .filter(|t| !t.is_empty())
            };
            let title = text(&meta.title);
            let mut properties = Vec::new();
            for (label, value) in [
                ("Title", title.clone()),
                ("Author", text(&meta.author)),
                ("Subject", text(&meta.subject)),
                ("Keywords", text(&meta.keywords)),
                ("Creator", text(&meta.creator)),
                ("Producer", text(&meta.producer)),
                ("Created", meta.creation_date.as_ref().map(format_date)),
                ("Modified", meta.modification_date.as_ref().map(format_date)),
            ] {
                if let Some(value) = value {
                    properties.push((label, value));
                }
            }
            properties.push(("PDF version", pdf_version(self.pdf.version()).to_owned()));
            properties.push(("Pages", self.pdf.pages().len().to_string()));
            if let Some(first) = self.pdf.pages().first() {
                let (w, h) = first.render_dimensions();
                properties.push((
                    "Page size",
                    format!("{:.1} × {:.1} in ({w:.0} × {h:.0} pt)", w / 72.0, h / 72.0),
                ));
            }
            properties.push(("File size", format_bytes(self.file_len)));
            // A malformed outline shouldn't stop the document opening.
            let outline =
                catch_unwind(AssertUnwindSafe(|| crate::outline::read(&self.pdf))).unwrap_or_default();
            let comments = self.comments();
            let fields = catch_unwind(AssertUnwindSafe(|| crate::forms::read(&self.pdf))).unwrap_or_default();
            let links = catch_unwind(AssertUnwindSafe(|| crate::links::read(&self.pdf))).unwrap_or_default();
            let protected = crate::edit::protected(&self.bytes);
            properties.push((
                "Password protection",
                if protected { "Yes" } else { "None" }.to_owned(),
            ));
            let attachments =
                catch_unwind(AssertUnwindSafe(|| crate::attachments::read(&self.pdf))).unwrap_or_default();
            let layers =
                catch_unwind(AssertUnwindSafe(|| crate::layers::read(&self.pdf))).unwrap_or_default();
            let signatures = self.signatures();
            DocumentInfo {
                signatures,
                fields,
                links,
                protected,
                attachments,
                layers,
                pages,
                title,
                properties,
                outline,
                comments,
            }
        }))
        .map_err(|_| OpenError::EngineCrashed)
    }

    /// The text of page `index`, for search and copy.
    pub fn page_text(&self, index: usize) -> Result<crate::PageText, RenderError> {
        let pages = self.pdf.pages();
        let page = pages.get(index).ok_or(RenderError::NoSuchPage(index))?;
        catch_unwind(AssertUnwindSafe(|| crate::text::extract(page, &self.settings)))
            .map_err(|_| RenderError::EngineCrashed)
    }

    /// The file's bytes with `annotations` added as an incremental update, ready to save or
    /// to reopen. The original revision is kept intact at the start of the output.
    pub fn with_annotations(&self, annotations: &[crate::Annotation]) -> Result<Vec<u8>, crate::SaveError> {
        self.with_changes(&crate::Changes {
            add: annotations.to_vec(),
            ..crate::Changes::default()
        })
    }

    /// The file's bytes with `changes` (comments added and removed) as an incremental update.
    pub fn with_changes(&self, changes: &crate::Changes) -> Result<Vec<u8>, crate::SaveError> {
        catch_unwind(AssertUnwindSafe(|| {
            crate::edit::apply(&self.pdf, &self.bytes, changes, &self.password)
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// A whole new file with the pages in `plan`: this document's pages (doc 0) and those of
    /// `others` (doc 1 onwards), in any order, turned or repeated, plus blank pages. Nothing
    /// the new file doesn't use is kept, so pages left out can't be recovered from it. It keeps
    /// this document's password protection.
    pub fn rearranged(
        &self,
        plan: &[crate::PageSource],
        others: &[&Document],
    ) -> Result<Vec<u8>, crate::SaveError> {
        catch_unwind(AssertUnwindSafe(|| {
            let mut docs: Vec<crate::edit::Input<'_>> = vec![(&self.pdf, &self.bytes, &self.password)];
            docs.extend(
                others
                    .iter()
                    .map(|d| (&d.pdf, d.bytes.as_slice(), d.password.as_str())),
            );
            crate::edit::rearrange(&docs, plan)
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// The document with the pages at `pages` cropped by `margins` (top, right, bottom, left
    /// as shown, in points), as a new file.
    pub fn cropped(&self, pages: &[usize], margins: crate::Margins) -> Result<Vec<u8>, crate::SaveError> {
        catch_unwind(AssertUnwindSafe(|| {
            crate::edit::crop(
                (&self.pdf, &self.bytes, &self.password),
                self.page_count(),
                pages,
                margins,
            )
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// The document with header and footer text drawn on the pages at `pages`, numbered in
    /// that order, as a new file.
    pub fn with_header_footer(
        &self,
        pages: &[usize],
        marks: &crate::HeaderFooter,
    ) -> Result<Vec<u8>, crate::SaveError> {
        catch_unwind(AssertUnwindSafe(|| {
            crate::edit::header_footer(
                (&self.pdf, &self.bytes, &self.password),
                self.page_count(),
                pages,
                marks,
            )
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// The document with the text on `pages` recognised (OCR) and laid over them invisibly, so
    /// it can be found, selected and copied. Pages that have text already are left alone.
    /// Also gives how many pages got text, and how many words in all.
    pub fn recognised(&self, pages: &[usize]) -> Result<(Vec<u8>, usize, usize), crate::SaveError> {
        /// Pages are read at this resolution, in dots per inch...
        const DPI: f32 = 200.0;
        /// ...but no wider or taller than this, in pixels.
        const MAX_SIDE: f32 = 5000.0;
        let cache = self.new_cache();
        let mut found = Vec::new();
        let mut needed = false;
        for &page in pages {
            if self
                .page_text(page)
                .is_ok_and(|t| !t.slice(0..t.len()).trim().is_empty())
            {
                continue;
            }
            needed = true;
            let size = self.page_size(page).ok_or(crate::SaveError::NoSuchPage(page))?;
            let scale = (DPI / 72.0).min(MAX_SIDE / size.width.max(size.height).max(1.0));
            let picture = self
                .render_page(&cache, page, scale)
                .map_err(|e| crate::SaveError::Recognition(e.to_string()))?;
            let rgb: Vec<u8> = picture
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|[r, g, b, _]| [*r, *g, *b])
                .collect();
            let words = crate::ocr::recognise(&rgb, picture.width, picture.height)
                .map_err(crate::SaveError::Recognition)?;
            // From pixels to points, as shown.
            let x = f64::from(size.width) / f64::from(picture.width.max(1));
            let y = f64::from(size.height) / f64::from(picture.height.max(1));
            let words: crate::edit::Words = words
                .into_iter()
                .map(|w| {
                    let [x0, top, x1, bottom] = w.rect;
                    (w.text, [x0 * x, top * y, x1 * x, bottom * y])
                })
                .collect();
            if !words.is_empty() {
                found.push((page, words));
            }
        }
        if found.is_empty() {
            return Err(crate::SaveError::NothingRecognised(!needed));
        }
        let count = found.iter().map(|(_, w)| w.len()).sum();
        let bytes = catch_unwind(AssertUnwindSafe(|| {
            crate::edit::with_hidden_text(
                (&self.pdf, &self.bytes, &self.password),
                self.page_count(),
                &found,
            )
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)??;
        Ok((bytes, found.len(), count))
    }

    /// The document in as small a file as can be written without changing how it looks, or
    /// with pictures sharper than `pictures` dots per inch (as drawn) scaled down to it.
    pub fn compacted(&self, pictures: Option<u32>) -> Result<Vec<u8>, crate::SaveError> {
        catch_unwind(AssertUnwindSafe(|| {
            crate::edit::compacted(
                (&self.pdf, &self.bytes, &self.password),
                self.page_count(),
                pictures,
            )
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// The document with text, pictures or links added to page `page`, as a new file.
    pub fn with_additions(
        &self,
        page: usize,
        additions: &[crate::Addition],
    ) -> Result<Vec<u8>, crate::SaveError> {
        // Replaced text without a colour of its own takes the colour of the text it replaces.
        let additions: Vec<crate::Addition> = additions
            .iter()
            .map(|a| match a {
                crate::Addition::ReplaceText {
                    area,
                    text,
                    size,
                    color: None,
                } => crate::Addition::ReplaceText {
                    area: *area,
                    text: text.clone(),
                    size: *size,
                    color: self.text_color(page, *area),
                },
                other => other.clone(),
            })
            .collect();
        catch_unwind(AssertUnwindSafe(|| {
            crate::edit::add(
                (&self.pdf, &self.bytes, &self.password),
                self.page_count(),
                page,
                &additions,
            )
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// The colour text in `area` of page `page` is drawn in, as near as the picture of the
    /// page shows: the pixel that stands out most from the area's background.
    fn text_color(&self, page: usize, area: crate::TextBox) -> Option<crate::Color> {
        const SCALE: f32 = 2.0;
        let cache = self.new_cache();
        let shown = self.render_page(&cache, page, SCALE).ok()?;
        let scale = shown.width as f32 / self.page_size(page)?.width.max(1.0);
        let (w, h) = (shown.width as usize, shown.height as usize);
        let px = |v: f32, max: usize| ((v * scale).max(0.0) as usize).min(max);
        let (x0, x1) = (px(area.x0, w), px(area.x1, w));
        let (y0, y1) = (px(area.y0, h), px(area.y1, h));
        let mut counts: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
        let mut pixels = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let at = (y * w + x) * 4;
                if let Some([r, g, b]) = shown
                    .rgba
                    .get(at..at + 3)
                    .and_then(|p| <[u8; 3]>::try_from(p).ok())
                {
                    *counts.entry([r, g, b]).or_default() += 1;
                    pixels.push([r, g, b]);
                }
            }
        }
        let background = counts.into_iter().max_by_key(|(_, n)| *n)?.0;
        let distance = |p: &[u8; 3]| {
            p.iter()
                .zip(background)
                .map(|(a, b)| (i32::from(*a) - i32::from(b)).pow(2))
                .sum::<i32>()
        };
        let ink = pixels.iter().max_by_key(|p| distance(p))?;
        Some(crate::Color(ink.map(|c| f32::from(c) / 255.0)))
    }

    /// The document with `add` fields added to its form and the fields `remove` taken out,
    /// as a new file. Added fields named like one already there get a number after the name.
    pub fn with_form_fields(
        &self,
        add: &[crate::NewField],
        remove: &[crate::ObjRef],
    ) -> Result<Vec<u8>, crate::SaveError> {
        let fields = self.info().map(|i| i.fields).unwrap_or_default();
        let mut taken: std::collections::HashSet<String> = fields
            .iter()
            .filter(|f| !remove.contains(&f.id))
            .map(|f| f.name.clone())
            .collect();
        let add: Vec<crate::NewField> = add
            .iter()
            .map(|f| {
                let base = f.name.trim().replace('.', "_");
                let base = if base.is_empty() { "Field".to_owned() } else { base };
                let mut name = base.clone();
                let mut i = 2;
                while taken.contains(&name) {
                    name = format!("{base} {i}");
                    i += 1;
                }
                taken.insert(name.clone());
                crate::NewField { name, ..f.clone() }
            })
            .collect();
        let widgets: Vec<(usize, crate::ObjRef)> = fields
            .iter()
            .filter(|f| remove.contains(&f.id))
            .flat_map(|f| f.widgets.iter().map(|w| (w.page, w.annot)))
            .collect();
        catch_unwind(AssertUnwindSafe(|| {
            crate::edit::form_fields(
                (&self.pdf, &self.bytes, &self.password),
                self.page_count(),
                &add,
                remove,
                &widgets,
            )
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// The pictures each page draws (not those inside forms), each as the box it covers in
    /// the same space as page text.
    pub fn pictures(&self, page: usize) -> Vec<crate::TextBox> {
        catch_unwind(AssertUnwindSafe(|| crate::edit::page_pictures(&self.pdf, page))).unwrap_or_default()
    }

    /// What keeps the document from being PDF/A-2b, the archival kind of PDF: an empty list
    /// when the main rules are met. Not every rule of the standard is checked.
    pub fn preflight(&self) -> Vec<crate::Finding> {
        let encrypted = crate::edit::protected(&self.bytes);
        let has_id = crate::edit::has_file_id(&self.bytes);
        catch_unwind(AssertUnwindSafe(|| {
            crate::preflight::check(&self.pdf, encrypted, has_id, crate::Standard::PdfA)
        }))
        .unwrap_or_default()
    }

    /// The document's digital signatures, each checked: whether the signed bytes are
    /// unchanged and the signature matches the signer's certificate. Whether to trust that
    /// certificate isn't decided here.
    pub fn signatures(&self) -> Vec<crate::Signature> {
        catch_unwind(AssertUnwindSafe(|| crate::sig::read(&self.pdf, &self.bytes))).unwrap_or_default()
    }

    /// The document signed with the certificate and key in a PKCS #12 file (.p12 or .pfx),
    /// as an incremental update, so earlier signatures stay valid. The signature is invisible
    /// and listed on the first page.
    pub fn signed(
        &self,
        pkcs12: &[u8],
        password: &str,
        options: &crate::SignOptions,
    ) -> Result<Vec<u8>, crate::SaveError> {
        self.signed_by(
            &crate::DigitalId::File {
                pkcs12: std::sync::Arc::new(pkcs12.to_vec()),
                password: password.to_owned(),
            },
            options,
        )
    }

    /// The document signed with a digital ID, as [`Document::signed`] does.
    pub fn signed_by(
        &self,
        id: &crate::DigitalId,
        options: &crate::SignOptions,
    ) -> Result<Vec<u8>, crate::SaveError> {
        let signer = crate::sig::Signer::of(id).map_err(crate::SaveError::Certificate)?;
        let signed_before = !self.signatures().is_empty();
        let (mut bytes, room) = catch_unwind(AssertUnwindSafe(|| {
            crate::edit::prepare_signature(&self.pdf, &self.bytes, &signer.name(), options, signed_before)
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)??;
        let (before, after) = bytes.split_at(room.start);
        let after = after.get(room.len()..).unwrap_or_default();
        let der = signer
            .sign(&[before, after])
            .map_err(crate::SaveError::Certificate)?;
        let hex: String = der.iter().map(|b| format!("{b:02X}")).collect();
        let slot = bytes
            .get_mut(room.start + 1..room.start + 1 + hex.len())
            .filter(|_| hex.len() + 2 <= room.len())
            .ok_or_else(|| {
                crate::SaveError::Certificate("The certificate chain is too large to fit.".into())
            })?;
        slot.copy_from_slice(hex.as_bytes());
        Ok(bytes)
    }

    /// The document with tags added for screen readers: each run of text a paragraph, or a
    /// heading where its type is larger, in the order it's drawn. Fails if it has tags.
    pub fn tagged(&self) -> Result<Vec<u8>, crate::SaveError> {
        catch_unwind(AssertUnwindSafe(|| crate::edit::tag(&self.pdf, &self.bytes)))
            .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// What makes the document harder to use with screen readers and other assistive
    /// technology: an empty list when the main checks pass.
    pub fn accessibility(&self) -> Vec<crate::AccessFinding> {
        let pages = self.pdf.pages().len();
        // Pages that draw pictures and have no text are probably scanned.
        let image_only: Vec<usize> = (0..pages.min(2_000))
            .filter(|&p| {
                self.page_text(p)
                    .is_ok_and(|t| t.chars().iter().all(|c| c.is_whitespace()))
                    && !self.pictures(p).is_empty()
            })
            .collect();
        catch_unwind(AssertUnwindSafe(|| {
            let meta = self.pdf.metadata();
            let mut title = meta
                .title
                .as_deref()
                .is_some_and(|t| !decode_text_string(t).trim().is_empty());
            if !title {
                let xref = self.pdf.xref();
                let catalog = xref
                    .get::<hayro::hayro_syntax::object::Dict<'_>>(xref.root_id())
                    .unwrap_or_default();
                title = catalog
                    .get::<hayro::hayro_syntax::object::Stream<'_>>(b"Metadata")
                    .and_then(|m| m.decoded().ok().map(|d| d.into_owned()))
                    .is_some_and(|x| String::from_utf8_lossy(&x).contains("<dc:title>"));
            }
            let bookmarks = crate::outline::read(&self.pdf).len();
            let undescribed: Vec<(String, usize)> = crate::forms::read(&self.pdf)
                .into_iter()
                .filter(|f| f.tooltip.trim().is_empty())
                .filter_map(|f| Some((f.name, f.widgets.first()?.page)))
                .collect();
            crate::access::check(
                &self.pdf,
                &crate::access::Known {
                    title,
                    bookmarks,
                    image_only: &image_only,
                    undescribed: &undescribed,
                },
            )
        }))
        .unwrap_or_default()
    }

    /// What keeps the document from being PDF/X-4, the kind of PDF for printing: an empty
    /// list when the main rules are met. Not every rule of the standard is checked.
    pub fn preflight_x(&self) -> Vec<crate::Finding> {
        let encrypted = crate::edit::protected(&self.bytes);
        let has_id = crate::edit::has_file_id(&self.bytes);
        catch_unwind(AssertUnwindSafe(|| {
            crate::preflight::check(&self.pdf, encrypted, has_id, crate::Standard::PdfX)
        }))
        .unwrap_or_default()
    }

    /// Where form fields could go on each page: blanks, empty boxes and lines to write on,
    /// and box characters, named after the words beside them. Places that already have a
    /// field are left out.
    pub fn detect_fields(&self) -> Result<Vec<crate::NewField>, RenderError> {
        let fields = self.info().map_err(|_| RenderError::EngineCrashed)?.fields;
        let mut out = Vec::new();
        for index in 0..self.page_count() {
            let text = self.page_text(index)?;
            let existing: Vec<crate::TextBox> = fields
                .iter()
                .flat_map(|f| f.widgets.iter())
                .filter(|w| w.page == index)
                .map(|w| w.at)
                .collect();
            let found = catch_unwind(AssertUnwindSafe(|| {
                let (rects, lines) = crate::edit::page_shapes(&self.pdf, index);
                crate::detect::detect(&crate::detect::Page {
                    index,
                    text: &text,
                    rects: &rects,
                    lines: &lines,
                    existing: &existing,
                })
            }))
            .map_err(|_| RenderError::EngineCrashed)?;
            out.extend(found);
        }
        Ok(out)
    }

    /// The document with pictures on page `page` moved, replaced or taken out, as a new
    /// file. Each change names a picture by its place in [`Document::pictures`].
    pub fn with_pictures_changed(
        &self,
        page: usize,
        changes: &[(usize, crate::PictureChange)],
    ) -> Result<Vec<u8>, crate::SaveError> {
        catch_unwind(AssertUnwindSafe(|| {
            crate::edit::change_pictures(
                (&self.pdf, &self.bytes, &self.password),
                self.page_count(),
                page,
                changes,
            )
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// The differences between this document's text and `other`'s, word by word.
    pub fn compare_with(&self, other: &Document) -> Result<Vec<crate::Change>, RenderError> {
        let texts = |doc: &Document| -> Result<Vec<crate::PageText>, RenderError> {
            (0..doc.page_count()).map(|i| doc.page_text(i)).collect()
        };
        let (ours, theirs) = (texts(self)?, texts(other)?);
        let ours: Vec<&[char]> = ours.iter().map(crate::PageText::chars).collect();
        let theirs: Vec<&[char]> = theirs.iter().map(crate::PageText::chars).collect();
        catch_unwind(AssertUnwindSafe(|| crate::compare::compare(&ours, &theirs)))
            .map_err(|_| RenderError::EngineCrashed)
    }

    /// The document with everything inside the marked areas taken out for good and covered
    /// with black boxes, as a new file. Each page's areas are in the same space as its text
    /// boxes.
    pub fn redacted(&self, areas: &[(usize, Vec<crate::TextBox>)]) -> Result<Vec<u8>, crate::SaveError> {
        catch_unwind(AssertUnwindSafe(|| {
            crate::edit::redact((&self.pdf, &self.bytes, &self.password), self.page_count(), areas)
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// The document with `mark` drawn on the pages at `pages`, as a new file.
    pub fn with_watermark(
        &self,
        pages: &[usize],
        mark: &crate::Watermark,
    ) -> Result<Vec<u8>, crate::SaveError> {
        catch_unwind(AssertUnwindSafe(|| {
            crate::edit::watermark(
                (&self.pdf, &self.bytes, &self.password),
                self.page_count(),
                pages,
                mark,
            )
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// The document with its comments (`comments`), form fields (`fields`) or both drawn into
    /// the pages so they can no longer be changed, as a new file.
    pub fn flattened(&self, comments: bool, fields: bool) -> Result<Vec<u8>, crate::SaveError> {
        // Text and choice fields may rely on the viewer to draw them; writing their values
        // again gives each one an appearance to flatten.
        let drawn;
        let source = if fields {
            let info = self.info().map_err(|_| crate::SaveError::Damaged)?;
            let values: Vec<_> = info
                .fields
                .iter()
                .filter(|f| {
                    matches!(
                        f.kind,
                        crate::FieldKind::Text { .. } | crate::FieldKind::Choice { .. }
                    )
                })
                .map(|f| (f.id, f.value.clone()))
                .collect();
            if values.is_empty() {
                self
            } else {
                let changes = crate::Changes {
                    fields: values,
                    ..crate::Changes::default()
                };
                drawn = self
                    .reopen(self.with_changes(&changes)?)
                    .map_err(|_| crate::SaveError::Damaged)?;
                &drawn
            }
        } else {
            self
        };
        catch_unwind(AssertUnwindSafe(|| {
            crate::edit::flatten(
                (&source.pdf, &source.bytes, &source.password),
                source.page_count(),
                comments,
                fields,
            )
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// The document with new password protection, or none, as a new file, and the password
    /// that opens it with full rights. Fails unless this document was opened with the owner
    /// password or allows changes.
    pub fn protected(
        &self,
        protection: Option<&crate::Protection>,
    ) -> Result<(Vec<u8>, String), crate::SaveError> {
        let bytes = catch_unwind(AssertUnwindSafe(|| {
            crate::edit::protect(
                (&self.pdf, &self.bytes, &self.password),
                self.page_count(),
                protection,
            )
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)??;
        let password = match protection {
            Some(p) if !p.owner_password.is_empty() => p.owner_password.clone(),
            Some(p) => p.open_password.clone(),
            None => String::new(),
        };
        Ok((bytes, password))
    }

    /// The document without the hidden information `clean` names, as a new file.
    pub fn cleaned(&self, clean: crate::Cleanup) -> Result<Vec<u8>, crate::SaveError> {
        catch_unwind(AssertUnwindSafe(|| {
            crate::edit::clean_up((&self.pdf, &self.bytes, &self.password), self.page_count(), clean)
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// A copy made PDF/A-2b where it can be: scripts and forbidden actions taken out,
    /// metadata naming it PDF/A, an sRGB output intent, and comments and fields set to
    /// print. What can't be fixed this way, such as fonts that aren't embedded, stays, so
    /// check the copy with [`Document::preflight`].
    pub fn archived(&self) -> Result<Vec<u8>, crate::SaveError> {
        if crate::edit::protected(&self.bytes) {
            return Err(crate::SaveError::ArchiveProtected);
        }
        self.cleaned(crate::Cleanup {
            scripts: true,
            archive: true,
            ..crate::Cleanup::default()
        })
    }

    /// The contents of attached file `index` (in [`DocumentInfo::attachments`]).
    pub fn attachment(&self, index: usize) -> Option<Vec<u8>> {
        // An attachment may be as large as a file Panna would open, but no larger.
        let limit = self.limits.max_file_bytes;
        catch_unwind(AssertUnwindSafe(|| {
            crate::attachments::data(&self.pdf, index, limit)
        }))
        .ok()
        .flatten()
    }

    /// The whole document, without password protection, for the printing system. Fails when
    /// the file's permissions don't allow printing.
    pub fn print_copy(&self) -> Result<Vec<u8>, crate::SaveError> {
        catch_unwind(AssertUnwindSafe(|| {
            crate::edit::print_copy((&self.pdf, &self.bytes, &self.password), self.page_count())
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)?
    }

    /// The comments in the document, as listed in [`DocumentInfo::comments`].
    pub fn comments(&self) -> Vec<crate::Comment> {
        catch_unwind(AssertUnwindSafe(|| crate::comments::read(&self.pdf))).unwrap_or_default()
    }

    /// Parses `bytes`, a later revision of this file, with the same password and limits.
    /// This document as it looks with the layers in `hidden` hidden and the rest shown, for
    /// viewing; never saved.
    pub fn with_layers_hidden(&self, hidden: &[crate::ObjRef]) -> Result<Self, crate::SaveError> {
        let layers: Vec<crate::ObjRef> = crate::layers::read(&self.pdf).iter().map(|l| l.id).collect();
        let bytes = catch_unwind(AssertUnwindSafe(|| {
            crate::edit::with_layers(&self.pdf, &self.bytes, &layers, hidden)
        }))
        .map_err(|_| crate::SaveError::EngineCrashed)??;
        self.reopen(bytes).map_err(|_| crate::SaveError::Damaged)
    }

    /// The document's comments as an FDF or XFDF file, for the PDF named `pdf_name`.
    pub fn comments_file(&self, format: crate::DataFormat, pdf_name: &str) -> Vec<u8> {
        let comments =
            catch_unwind(AssertUnwindSafe(|| crate::commentdata::read(&self.pdf))).unwrap_or_default();
        crate::commentdata::export(&comments, format, pdf_name)
    }

    /// The comments in an FDF or XFDF file, as comments to add to this document, and how many
    /// in it couldn't be added: replies, comments on pages it doesn't have, and kinds Panna
    /// can't draw.
    pub fn comments_from_file(&self, bytes: &[u8]) -> Result<(Vec<crate::Annotation>, usize), String> {
        let markups = crate::commentdata::import(bytes)?;
        let total = markups.len();
        let pages = self.pdf.pages();
        let mut per_page: Vec<Vec<String>> = vec![Vec::new(); pages.len()];
        for (i, m) in markups.iter().enumerate() {
            if m.in_reply_to.is_some() {
                continue;
            }
            if let Some(list) = per_page.get_mut(m.page) {
                list.push(m.pdf_dict_for_import(i));
            }
        }
        // A stand-in document with the same pages as this one, holding just the comments, read
        // the way comments saved in a PDF are.
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                (0..pages.len())
                    .map(|i| format!("{} 0 R", i + 3))
                    .collect::<Vec<_>>()
                    .join(" "),
                pages.len()
            ),
        ];
        for (page, annots) in pages.iter().zip(&per_page) {
            let b = |r: hayro::hayro_syntax::object::Rect| format!("[{} {} {} {}]", r.x0, r.y0, r.x1, r.y1);
            let rotate = match page.rotation() {
                hayro::hayro_syntax::page::Rotation::None => 0,
                hayro::hayro_syntax::page::Rotation::Horizontal => 90,
                hayro::hayro_syntax::page::Rotation::Flipped => 180,
                hayro::hayro_syntax::page::Rotation::FlippedHorizontal => 270,
            };
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox {} /CropBox {} /Rotate {rotate} /Annots [{}] >>",
                b(page.media_box()),
                b(page.crop_box()),
                annots.join("\n")
            ));
        }
        let stand_in = crate::edit::assemble(&objects);
        let doc = Self::from_bytes(stand_in, None, self.limits).map_err(|e| e.to_string())?;
        let comments = doc.info().map_err(|e| e.to_string())?.comments;
        let added: Vec<crate::Annotation> = comments.into_iter().filter_map(|c| c.editable).collect();
        let skipped = total - added.len().min(total);
        Ok((added, skipped))
    }

    pub fn reopen(&self, bytes: Vec<u8>) -> Result<Self, OpenError> {
        Self::from_bytes(bytes, Some(&self.password), self.limits)
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The bytes this document was opened from.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// A render cache tied to this document. Reuse it across pages for speed.
    pub fn new_cache(&self) -> RenderCache<'_> {
        RenderCache::new()
    }

    /// Renders one page at `scale` pixels per point onto a white background. The scale is
    /// reduced if needed to stay inside [`Limits`]; check the returned bitmap size.
    pub fn render_page<'a>(
        &'a self,
        cache: &RenderCache<'a>,
        index: usize,
        scale: f32,
    ) -> Result<RenderedPage, RenderError> {
        let pages = self.pdf.pages();
        let page = pages.get(index).ok_or(RenderError::NoSuchPage(index))?;
        let (width, height) = page.render_dimensions();
        let scale = self
            .limits
            .clamp_scale(width, height, scale)
            .ok_or(RenderError::BadPageSize)?;
        let pixmap_settings = PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
        };
        let pixmap = catch_unwind(AssertUnwindSafe(|| {
            hayro::render(
                page,
                cache,
                &self.settings,
                &RenderSettings::default(),
                &pixmap_settings,
            )
        }))
        .map_err(|_| RenderError::EngineCrashed)?;
        let width = u32::from(pixmap.width());
        let height = u32::from(pixmap.height());
        let rgba = pixmap.take_rgba8(ImageAlphaType::Alpha);
        Ok(RenderedPage { width, height, rgba })
    }
}

fn decryption_error(e: hayro::hayro_syntax::DecryptionError) -> OpenError {
    use hayro::hayro_syntax::DecryptionError as D;
    match e {
        D::PasswordProtected => OpenError::PasswordRequired,
        D::UnsupportedAlgorithm => OpenError::UnsupportedEncryption,
        _ => OpenError::Invalid,
    }
}

/// Decodes a PDF text string:UTF-16BE or UTF-8 with a byte-order mark, otherwise
/// PDFDocEncoding.
pub(crate) fn decode_text_string(bytes: &[u8]) -> String {
    let text: String = if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair));
        char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    } else if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else {
        // ponytail: Latin-1 approximation of PDFDocEncoding; only 0x18–0x1F and 0x80–0x9F differ.
        bytes.iter().map(|&b| char::from(b)).collect()
    };
    without_overrides(&text)
}

/// `text` without Unicode's direction overrides, embeddings and isolates, which can make text
/// from a file show as something else (a signer's name, a file name, a bookmark). The marks
/// right-to-left languages use (U+200E, U+200F) stay.
pub(crate) fn without_overrides(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        .collect()
}

fn format_date(d: &hayro::hayro_syntax::object::DateTime) -> String {
    let sign = if d.utc_offset_hour < 0 { '-' } else { '+' };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC{sign}{:02}:{:02}",
        d.year,
        d.month,
        d.day,
        d.hour,
        d.minute,
        d.second,
        d.utc_offset_hour.unsigned_abs(),
        d.utc_offset_minute
    )
}

fn pdf_version(v: hayro::hayro_syntax::PdfVersion) -> &'static str {
    use hayro::hayro_syntax::PdfVersion as V;
    match v {
        V::Pdf10 => "1.0",
        V::Pdf11 => "1.1",
        V::Pdf12 => "1.2",
        V::Pdf13 => "1.3",
        V::Pdf14 => "1.4",
        V::Pdf15 => "1.5",
        V::Pdf16 => "1.6",
        V::Pdf17 => "1.7",
        V::Pdf20 => "2.0",
    }
}

fn format_bytes(n: u64) -> String {
    match n {
        0..1_000 => format!("{n} bytes"),
        1_000..1_000_000 => format!("{:.1} KB", n as f64 / 1e3),
        _ => format!("{:.1} MB", n as f64 / 1e6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_to_an_area() {
        // 4×2 pixels, each pixel's red value its index.
        let page = RenderedPage {
            width: 4,
            height: 2,
            rgba: (0..8u8).flat_map(|i| [i, 0, 0, 255]).collect(),
        };
        let part = page.clone().cropped([0.75, 1.0, 0.25, 0.5]);
        assert_eq!((part.width, part.height), (2, 1));
        assert_eq!(part.rgba.iter().step_by(4).copied().collect::<Vec<_>>(), [5, 6]);
        // Nothing selected still keeps a pixel; out of range is clamped.
        let dot = page.clone().cropped([2.0, 2.0, 2.0, 2.0]);
        assert_eq!(
            (dot.width, dot.height, dot.rgba.first().copied()),
            (1, 1, Some(7))
        );
        let all = page.clone().cropped([-1.0, -1.0, 2.0, 2.0]);
        assert_eq!(all, page);
    }

    #[test]
    fn decodes_text_strings() {
        assert_eq!(decode_text_string(b"Plain"), "Plain");
        assert_eq!(decode_text_string(&[0xFE, 0xFF, 0x00, 0x48, 0x00, 0xE9]), "Hé");
        assert_eq!(decode_text_string(&[0xEF, 0xBB, 0xBF, 0xC3, 0xA9]), "é");
        assert_eq!(decode_text_string(&[0xE9]), "é");
        // A right-to-left override is dropped; a right-to-left mark is kept.
        let utf16 = |s: &str| {
            [0xFE, 0xFF]
                .into_iter()
                .chain(s.encode_utf16().flat_map(u16::to_be_bytes))
                .collect::<Vec<u8>>()
        };
        assert_eq!(decode_text_string(&utf16("Bank\u{202E}cba")), "Bankcba");
        assert_eq!(decode_text_string(&utf16("\u{200F}abc")), "\u{200F}abc");
        // Lone surrogate and odd trailing byte don't panic.
        assert_eq!(decode_text_string(&[0xFE, 0xFF, 0xD8, 0x00, 0x41]), "\u{FFFD}");
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(format_bytes(512), "512 bytes");
        assert_eq!(format_bytes(2_500), "2.5 KB");
        assert_eq!(format_bytes(3_400_000), "3.4 MB");
    }
}
