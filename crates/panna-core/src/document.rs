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
    TooLarge { bytes: u64, limit: u64 },
    TooManyPages { pages: usize, limit: usize },
    PasswordRequired,
    UnsupportedEncryption,
    Invalid,
    EngineCrashed,
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
    limits: Limits,
    settings: InterpreterSettings,
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
        let password = password.unwrap_or("");
        let parsed = catch_unwind(AssertUnwindSafe(|| {
            Pdf::new_with_password(Arc::new(bytes), password)
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
        Ok(Self {
            pdf,
            limits,
            settings: InterpreterSettings::default(),
        })
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
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
            let title = self
                .pdf
                .metadata()
                .title
                .as_deref()
                .map(|t| String::from_utf8_lossy(t).trim().to_owned())
                .filter(|t| !t.is_empty());
            DocumentInfo { pages, title }
        }))
        .map_err(|_| OpenError::EngineCrashed)
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
