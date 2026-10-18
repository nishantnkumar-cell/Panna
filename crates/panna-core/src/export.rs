//! Saving pages as plain text or as pictures.

use std::path::PathBuf;

use crate::{Document, RenderedPage, SaveError};

/// What pages become.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// One text file, pages one after another.
    Text,
    /// A PNG picture for each page.
    Png,
    /// A JPEG picture for each page.
    Jpeg,
}

/// Pages to save and where.
#[derive(Debug, Clone, PartialEq)]
pub struct Export {
    pub format: ExportFormat,
    /// Pictures' resolution, in dots per inch.
    pub dpi: f32,
    /// For text, every page goes to the first path; for pictures, each page goes to the path
    /// beside it.
    pub pages: Vec<(usize, PathBuf)>,
}

/// JPEG quality, from 1 to 100: little visible loss, a fraction of a PNG's size.
const JPEG_QUALITY: u8 = 88;

/// Text of `pages` of `doc`, a blank line between pages.
pub(crate) fn text(doc: &Document, pages: &[usize]) -> Result<String, SaveError> {
    let mut out = String::new();
    for &index in pages {
        let text = doc.page_text(index).map_err(|_| SaveError::NoSuchPage(index))?;
        let text = text.slice(0..text.len());
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(text.trim_end());
    }
    out.push('\n');
    Ok(out)
}

/// `page` encoded as `format` (PNG or JPEG), at `dpi` recorded in the PNG.
pub(crate) fn picture(page: &RenderedPage, format: ExportFormat) -> Result<Vec<u8>, SaveError> {
    use image::ImageEncoder;
    let fail = |e: image::ImageError| SaveError::Io(e.to_string());
    let mut out = Vec::new();
    match format {
        ExportFormat::Jpeg => {
            // JPEG has no transparency; pages are drawn on white, so the colour is all there is.
            let rgb: Vec<u8> = page
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|[r, g, b, _]| [*r, *g, *b])
                .collect();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
                .write_image(&rgb, page.width, page.height, image::ExtendedColorType::Rgb8)
                .map_err(fail)?;
        }
        _ => {
            image::codecs::png::PngEncoder::new(&mut out)
                .write_image(
                    &page.rgba,
                    page.width,
                    page.height,
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(fail)?;
        }
    }
    Ok(out)
}
