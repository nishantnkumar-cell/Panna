//! Making a new PDF from pictures, one picture a page.

use std::fmt::Write as _;

use super::annot::n;
use super::picture::StampImage;

/// The page a picture is fitted into, in points: A4, turned to suit the picture.
const PAGE: (f64, f64) = (595.0, 842.0);
/// Points per pixel at most, so small pictures aren't blown up (96 pixels an inch).
const MAX_SCALE: f64 = 0.75;
/// Pictures accepted at once.
pub const MAX_PICTURES: usize = 2000;

/// A picture as it goes into the PDF.
enum Picture {
    /// A JPEG file kept as it is, which PDF can show directly: width, height, colour
    /// components (1 or 3) and the file.
    Jpeg(u32, u32, u8, Vec<u8>),
    Decoded(StampImage),
}

impl Picture {
    fn read(bytes: Vec<u8>) -> Result<Self, String> {
        if let Some((w, h, components)) = jpeg_header(&bytes)
            && (components == 1 || components == 3)
        {
            return Ok(Picture::Jpeg(w, h, components, bytes));
        }
        StampImage::decode(&bytes).map(Picture::Decoded)
    }

    fn size(&self) -> (u32, u32) {
        match self {
            Picture::Jpeg(w, h, ..) => (*w, *h),
            Picture::Decoded(image) => (image.width, image.height),
        }
    }
}

/// A JPEG's width, height and colour components, read from its header.
fn jpeg_header(bytes: &[u8]) -> Option<(u32, u32, u8)> {
    use zune_core::bytestream::ZCursor;
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut decoder = zune_jpeg::JpegDecoder::new(ZCursor::new(bytes));
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    (w > 0 && h > 0 && w <= 20_000 && h <= 20_000).then_some((w, h, info.components))
}

/// A new PDF with each of `pictures` (PNG, JPEG or TIFF files' bytes) on a page of its own, fitted
/// to an A4 page turned to suit it. Fails with a message naming the first picture (counting
/// from 1) that can't be read.
pub fn pictures_to_pdf(pictures: Vec<Vec<u8>>) -> Result<Vec<u8>, String> {
    if pictures.is_empty() || pictures.len() > MAX_PICTURES {
        return Err(format!("Choose from 1 to {MAX_PICTURES} pictures."));
    }
    let count = pictures.len();
    let pictures = pictures
        .into_iter()
        .enumerate()
        .map(|(i, bytes)| {
            Picture::read(bytes).map_err(|e| {
                if count == 1 {
                    e
                } else {
                    format!("Picture {}: {e}", i + 1)
                }
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(pages_of(pictures))
}

/// A new PDF with an RGBA picture (from the clipboard, say) on its one page, fitted to an A4
/// page turned to suit it.
pub fn rgba_to_pdf(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let picture = StampImage::from_rgba(width, height, rgba)?;
    Ok(pages_of(vec![Picture::Decoded(picture)]))
}

/// Pages a new blank PDF can have at most.
pub const MAX_BLANK_PAGES: usize = 1000;

/// A new PDF of `count` empty pages, each `width` by `height` points.
pub fn blank_pdf(width: f32, height: f32, count: usize) -> Result<Vec<u8>, String> {
    if count == 0 || count > MAX_BLANK_PAGES {
        return Err(format!("Choose from 1 to {MAX_BLANK_PAGES} pages."));
    }
    // ISO 32000-2 limits pages to 3 to 14,400 units on a side.
    if !(3.0..=14_400.0).contains(&width) || !(3.0..=14_400.0).contains(&height) {
        return Err("Pages must be from 3 to 14,400 points on a side.".into());
    }
    let mut objects: Vec<Vec<u8>> = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), Vec::new()];
    let mut kids = String::new();
    for i in 0..count {
        let _ = write!(kids, "{} 0 R ", i + 3);
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << >> >>",
                n(f64::from(width)),
                n(f64::from(height))
            )
            .into_bytes(),
        );
    }
    if let Some(tree) = objects.get_mut(1) {
        *tree = format!("<< /Type /Pages /Kids [{}] /Count {count} >>", kids.trim_end()).into_bytes();
    }
    Ok(file_of(&objects))
}

/// A new PDF with each picture on a page of its own.
fn pages_of(pictures: Vec<Picture>) -> Vec<u8> {
    let count = pictures.len();
    // Objects: 1 catalog, 2 page tree, then for each picture a page, its content, its image
    // and (maybe) the image's mask.
    let mut objects: Vec<Vec<u8>> = vec![Vec::new(), Vec::new()];
    let mut kids = String::new();
    for picture in pictures {
        let (w, h) = picture.size();
        let (w, h) = (f64::from(w), f64::from(h));
        let (bw, bh) = if w > h { (PAGE.1, PAGE.0) } else { PAGE };
        let scale = (bw / w).min(bh / h).min(MAX_SCALE);
        let (pw, ph) = (w * scale, h * scale);
        let page = objects.len() + 1;
        let (content, image) = (page + 1, page + 2);
        let draw = format!("q {} 0 0 {} 0 0 cm /Im0 Do Q", n(pw), n(ph));
        let _ = write!(kids, "{page} 0 R ");
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents {content} 0 R \
                 /Resources << /XObject << /Im0 {image} 0 R >> >> >>",
                n(pw),
                n(ph)
            )
            .into_bytes(),
        );
        objects.push(format!("<< /Length {} >>\nstream\n{draw}\nendstream", draw.len()).into_bytes());
        match picture {
            Picture::Jpeg(w, h, components, data) => {
                let color = if components == 1 {
                    "DeviceGray"
                } else {
                    "DeviceRGB"
                };
                let mut body = format!(
                    "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /{color} \
                     /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
                    data.len()
                )
                .into_bytes();
                body.extend_from_slice(&data);
                body.extend_from_slice(b"\nendstream");
                objects.push(body);
            }
            Picture::Decoded(decoded) => {
                let mask_ref = decoded.has_mask().then(|| format!("{} 0 R", image + 1));
                let (body, mask) = decoded.objects(mask_ref.as_deref());
                objects.push(body);
                objects.extend(mask);
            }
        }
    }
    if let Some(catalog) = objects.get_mut(0) {
        *catalog = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    }
    if let Some(tree) = objects.get_mut(1) {
        *tree = format!("<< /Type /Pages /Kids [{}] /Count {count} >>", kids.trim_end()).into_bytes();
    }
    file_of(&objects)
}

/// A PDF file of numbered objects (from 1), the first being the catalog.
pub(super) fn file_of(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    let mut xref = format!("xref\n0 {}\n0000000000 65535 f\r\n", offsets.len() + 1);
    for offset in offsets {
        let _ = write!(xref, "{offset:010} 00000 n\r\n");
    }
    let _ = write!(
        xref,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
        objects.len() + 1
    );
    out.extend_from_slice(xref.as_bytes());
    out
}
