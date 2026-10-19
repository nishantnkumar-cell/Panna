//! Pictures for image stamps: a PNG, JPEG or TIFF, decoded and kept ready to write into a PDF.

use std::io::Write as _;
use std::sync::Arc;

use flate2::Compression;
use flate2::write::ZlibEncoder;

/// The largest picture accepted, in pixels a side. A stamp needs nothing near this.
const MAX_SIDE: u32 = 8000;
/// The largest file accepted.
const MAX_BYTES: usize = 50 * 1024 * 1024;

/// A picture for an image stamp, decoded and compressed for the PDF.
#[derive(Debug, Clone)]
pub struct StampImage {
    pub width: u32,
    pub height: u32,
    data: Arc<Encoded>,
}

#[derive(Debug)]
struct Encoded {
    /// RGB samples, zlib-compressed.
    rgb: Vec<u8>,
    /// Alpha samples, zlib-compressed, when any pixel isn't opaque.
    alpha: Option<Vec<u8>>,
}

impl PartialEq for StampImage {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }
}

impl StampImage {
    /// Decodes a PNG, JPEG or TIFF file's bytes (a TIFF's first page).
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("That picture is too large.".into());
        }
        let (width, height, rgba) = if bytes.starts_with(&[0xFF, 0xD8]) {
            jpeg(bytes)?
        } else if bytes.starts_with(b"\x89PNG") {
            decoded(bytes, image::ImageFormat::Png, "PNG")?
        } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
            decoded(bytes, image::ImageFormat::Tiff, "TIFF")?
        } else {
            return Err("Choose a PNG, JPEG or TIFF picture.".into());
        };
        Self::from_rgba(width, height, &rgba)
    }

    /// A picture from RGBA samples, row by row, such as a drawn signature.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("That picture is empty.".into());
        }
        if too_big(width, height) || rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
            return Err("That picture is too large.".into());
        }
        let (pixels, _) = rgba.as_chunks::<4>();
        let rgb: Vec<u8> = pixels.iter().flat_map(|&[r, g, b, _]| [r, g, b]).collect();
        let alpha: Vec<u8> = pixels.iter().map(|&[.., a]| a).collect();
        let opaque = alpha.iter().all(|&a| a == 255);
        Ok(Self {
            width,
            height,
            data: Arc::new(Encoded {
                rgb: deflate(&rgb),
                alpha: (!opaque).then(|| deflate(&alpha)),
            }),
        })
    }

    /// The image XObject, and its soft mask when it has transparency: (image, mask) bodies,
    /// with the mask referred to as `mask_ref`.
    pub(crate) fn objects(&self, mask_ref: Option<&str>) -> (Vec<u8>, Option<Vec<u8>>) {
        let stream = |extra: &str, color: &str, data: &[u8]| {
            let mut body = format!(
                "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /{color} \
                 /BitsPerComponent 8 /Filter /FlateDecode /Length {}{extra} >>\nstream\n",
                self.width,
                self.height,
                data.len()
            )
            .into_bytes();
            body.extend_from_slice(data);
            body.extend_from_slice(b"\nendstream");
            body
        };
        let mask = match (&self.data.alpha, mask_ref) {
            (Some(alpha), Some(_)) => Some(stream("", "DeviceGray", alpha)),
            _ => None,
        };
        let extra = match (&mask, mask_ref) {
            (Some(_), Some(r)) => format!(" /SMask {r}"),
            _ => String::new(),
        };
        (stream(&extra, "DeviceRGB", &self.data.rgb), mask)
    }

    /// Whether it has transparent parts, which need a mask object.
    /// Identifies the decoded picture, so one used on many pages is written once.
    pub(crate) fn key(&self) -> usize {
        Arc::as_ptr(&self.data) as usize
    }

    pub(crate) fn has_mask(&self) -> bool {
        self.data.alpha.is_some()
    }
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    // Writing to a Vec can't fail.
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}

fn too_big(width: u32, height: u32) -> bool {
    width > MAX_SIDE || height > MAX_SIDE
}

fn decoded(bytes: &[u8], format: image::ImageFormat, kind: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let bad = |_| format!("That {kind} picture couldn't be read.");
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let picture = reader.decode().map_err(bad)?.into_rgba8();
    let (width, height) = picture.dimensions();
    Ok((width, height, picture.into_raw()))
}

fn jpeg(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    use zune_core::bytestream::ZCursor;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;

    let bad = |_| "That JPEG picture couldn't be read.".to_owned();
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(MAX_SIDE as usize)
        .set_max_height(MAX_SIDE as usize);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let pixels = decoder.decode().map_err(bad)?;
    let info = decoder
        .info()
        .ok_or_else(|| "That JPEG picture couldn't be read.".to_owned())?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    if too_big(width, height) || pixels.len() != width as usize * height as usize * 4 {
        return Err("That JPEG picture couldn't be read.".into());
    }
    Ok((width, height, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let picture = image::RgbaImage::from_raw(width, height, rgba.to_vec()).unwrap_or_default();
        let _ = picture.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
        out
    }

    #[test]
    fn pngs_keep_their_transparency() {
        let opaque = StampImage::decode(&png_bytes(&[255, 0, 0, 255, 0, 0, 255, 255], 2, 1));
        assert!(
            opaque
                .as_ref()
                .is_ok_and(|p| (p.width, p.height) == (2, 1) && !p.has_mask())
        );
        let clear = StampImage::decode(&png_bytes(&[255, 0, 0, 0, 0, 0, 255, 255], 2, 1));
        assert!(clear.as_ref().is_ok_and(StampImage::has_mask));
        if let Ok(p) = clear {
            let (image, mask) = p.objects(Some("9 0 R"));
            assert!(String::from_utf8_lossy(&image).contains("/SMask 9 0 R"));
            assert!(mask.is_some());
        }
    }

    #[test]
    fn other_files_are_refused() {
        assert!(StampImage::decode(b"GIF89a").is_err());
        assert!(StampImage::decode(b"\x89PNG broken").is_err());
        assert!(StampImage::decode(&[0xFF, 0xD8, 0xFF, 0x00]).is_err());
    }
}
