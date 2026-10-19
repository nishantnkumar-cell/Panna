//! Checks run once when a document opens, before anything is decoded for real, so that small
//! files built to expand into gigabytes ("decompression bombs") or to make an image decoder
//! allocate an enormous bitmap are refused instead of exhausting memory.
//!
//! The engine decodes streams and images in full and has no output cap of its own, so these
//! checks measure first: compressed data is inflated through a counting reader that keeps
//! nothing, and image sizes are read from the image dictionary and from the JPEG or JPEG 2000
//! header the decoder itself trusts.

use std::collections::HashMap;
use std::io::Read;

use flate2::read::{DeflateDecoder, ZlibDecoder};
use hayro::hayro_syntax::Filter;
use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Name, Object, ObjectIdentifier, Stream};

use crate::Limits;

/// Why a document was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// One stream decodes to more than the limit.
    Stream { bytes: u64, limit: u64 },
    /// One page's content streams together decode to more than the limit.
    Page { page: usize, bytes: u64, limit: u64 },
    /// An image claims more pixels than the limit.
    Image { width: u64, height: u64, limit: u64 },
}

/// Checks every stream and image in `pdf` against `limits`.
pub(crate) fn check(pdf: &Pdf, limits: &Limits) -> Result<(), Refusal> {
    let mut sizes: HashMap<ObjectIdentifier, u64> = HashMap::new();
    for object in pdf.objects() {
        let Object::Stream(stream) = object else { continue };
        let dict = stream.dict();
        let kind = |key: &[u8]| dict.get::<Name<'_>>(key).map(|n| n.as_str().to_owned());
        // Attached files may legitimately be large and are only ever saved, so their size is
        // checked when saved instead (see `decoded_size`).
        if kind(b"Type").as_deref() == Some("EmbeddedFile") {
            continue;
        }
        if kind(b"Subtype").as_deref() == Some("Image") {
            check_image(&stream, limits)?;
        }
        let bytes = decoded_size(&stream, limits.max_decoded_bytes)?;
        sizes.insert(stream.obj_id(), bytes);
    }
    // The engine joins a page's content streams before running them.
    for (index, page) in pdf.pages().iter().enumerate() {
        let Some(contents) = page.raw().get::<Array<'_>>(b"Contents") else {
            continue;
        };
        let mut total = 0u64;
        for item in contents.iter::<Stream<'_>>() {
            total = total.saturating_add(sizes.get(&item.obj_id()).copied().unwrap_or(0));
        }
        if total > limits.max_decoded_bytes {
            return Err(Refusal::Page {
                page: index,
                bytes: total,
                limit: limits.max_decoded_bytes,
            });
        }
    }
    Ok(())
}

/// How many bytes `stream` decodes to, counted without keeping them. Flate stages, the only
/// filter with unbounded expansion, are inflated through a counter that stops at `limit`.
/// Stages after the leading text and Flate filters can only shrink or expand linearly, so
/// they're not followed.
pub(crate) fn decoded_size(stream: &Stream<'_>, limit: u64) -> Result<u64, Refusal> {
    let mut data = stream.raw_data().into_owned();
    let filters: Vec<Filter> = stream
        .filters()
        .into_iter()
        .filter(|f| *f != Filter::Crypt)
        .collect();
    let mut stages = filters.iter().peekable();
    // ASCII stages come first in some old files; their output is never larger than their input.
    while let Some(f) = stages.peek() {
        data = match f {
            Filter::AsciiHexDecode => ascii_hex(&data),
            Filter::Ascii85Decode => ascii85(&data),
            _ => break,
        };
        stages.next();
    }
    let flates = stages.take_while(|f| **f == Filter::FlateDecode).count();
    if flates == 0 {
        return Ok(data.len() as u64);
    }
    // Each Flate stage feeds the next, so nothing is held in memory but the input.
    let mut reader: Box<dyn Read + '_> = Box::new(&data[..]);
    for _ in 0..flates {
        reader = inflater(reader);
    }
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        match reader.read(&mut buf) {
            // Damaged data ends the count, as it ends the engine's decoding.
            Ok(0) | Err(_) => return Ok(total),
            Ok(n) => {
                total += n as u64;
                if total > limit {
                    return Err(Refusal::Stream { bytes: total, limit });
                }
            }
        }
    }
}

/// A Flate reader that accepts zlib data, and raw deflate data as the engine also does.
fn inflater<'a>(mut inner: Box<dyn Read + 'a>) -> Box<dyn Read + 'a> {
    let mut head = [0u8; 2];
    let n = read_up_to(&mut inner, &mut head);
    let head = head.get(..n).unwrap_or_default().to_vec();
    // A zlib header: compression method 8, and the two bytes form a multiple of 31.
    let zlib = be16(&head, 0).is_some_and(|h| (h >> 8) & 0x0F == 8 && h % 31 == 0);
    let chained: Box<dyn Read + 'a> = Box::new(std::io::Cursor::new(head).chain(inner));
    if zlib {
        Box::new(ZlibDecoder::new(chained))
    } else {
        Box::new(DeflateDecoder::new(chained))
    }
}

fn read_up_to(r: &mut dyn Read, buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        match r.read(buf.get_mut(n..).unwrap_or_default()) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    n
}

fn check_image(stream: &Stream<'_>, limits: &Limits) -> Result<(), Refusal> {
    let dict = stream.dict();
    let dims = |w: Option<i64>, h: Option<i64>| (w.unwrap_or(0).max(0) as u64, h.unwrap_or(0).max(0) as u64);
    let mut claims = vec![dims(dict.get::<i64>(b"Width"), dict.get::<i64>(b"Height"))];
    // The JPEG and JPEG 2000 decoders size their output from the data's own header.
    let filters = stream.filters();
    if filters.contains(&Filter::DctDecode) || filters.contains(&Filter::JpxDecode) {
        let data = stream.raw_data();
        claims.extend(jpeg_size(&data));
        claims.extend(jpx_size(&data));
    }
    for (width, height) in claims {
        if width.saturating_mul(height) > limits.max_image_pixels {
            return Err(Refusal::Image {
                width,
                height,
                limit: limits.max_image_pixels,
            });
        }
    }
    Ok(())
}

/// Width and height from a JPEG's start-of-frame segment.
fn jpeg_size(data: &[u8]) -> Option<(u64, u64)> {
    let mut i = 2;
    while let (Some(&lead), Some(&marker)) = (data.get(i), data.get(i + 1)) {
        if lead != 0xFF {
            i += 1;
            continue;
        }
        // SOF0 to SOF15, except DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let (h, w) = (be16(data, i + 5)?, be16(data, i + 7)?);
            return Some((u64::from(w), u64::from(h)));
        }
        if matches!(marker, 0xD8 | 0x01 | 0xD0..=0xD7 | 0xFF) {
            i += 2;
            continue;
        }
        i += 2 + usize::from(be16(data, i + 2)?).max(2);
    }
    None
}

/// Width and height from a JPEG 2000 codestream's SIZ marker (FF 51), inside a JP2 box or bare.
fn jpx_size(data: &[u8]) -> Option<(u64, u64)> {
    let at = data.windows(2).take(64 * 1024).position(|w| w == [0xFF, 0x51])?;
    let field = |k: usize| {
        let b: [u8; 4] = data.get(at + k..at + k + 4)?.try_into().ok()?;
        Some(u64::from(u32::from_be_bytes(b)))
    };
    // Lsiz (2), Rsiz (2), Xsiz (4), Ysiz (4), XOsiz (4), YOsiz (4).
    let (xsiz, ysiz, xo, yo) = (field(6)?, field(10)?, field(14)?, field(18)?);
    Some((xsiz.saturating_sub(xo), ysiz.saturating_sub(yo)))
}

fn be16(data: &[u8], at: usize) -> Option<u16> {
    let b: [u8; 2] = data.get(at..at + 2)?.try_into().ok()?;
    Some(u16::from_be_bytes(b))
}

fn ascii_hex(data: &[u8]) -> Vec<u8> {
    let digits: Vec<u8> = data
        .iter()
        .take_while(|&&b| b != b'>')
        .filter_map(|&b| (b as char).to_digit(16).map(|d| d as u8))
        .collect();
    digits
        .chunks(2)
        .map(|p| p.first().copied().unwrap_or(0) << 4 | p.get(1).copied().unwrap_or(0))
        .collect()
}

fn ascii85(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut group = [0u8; 5];
    let mut n = 0;
    let flush = |group: &[u8; 5], n: usize, out: &mut Vec<u8>| {
        let mut padded = *group;
        for b in padded.iter_mut().skip(n) {
            *b = 84;
        }
        let v = padded.iter().fold(0u64, |acc, &d| acc * 85 + u64::from(d));
        let bytes = (v as u32).to_be_bytes();
        out.extend_from_slice(bytes.get(..n.saturating_sub(1)).unwrap_or_default());
    };
    for &b in data {
        match b {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                if let Some(slot) = group.get_mut(n) {
                    *slot = b - b'!';
                }
                n += 1;
                if n == 5 {
                    flush(&group, 5, &mut out);
                    n = 0;
                }
            }
            _ => {}
        }
    }
    if n > 1 {
        flush(&group, n, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_decoders() {
        assert_eq!(ascii_hex(b"48 65 6C6C6F>"), b"Hello");
        assert_eq!(ascii_hex(b"7>"), [0x70]);
        assert_eq!(ascii85(b"87cURD]i,\"Ebo80~>"), b"Hello World!");
        assert_eq!(ascii85(b"z~>"), [0, 0, 0, 0]);
    }

    #[test]
    fn reads_jpeg_and_jpx_headers() {
        // SOI, then SOF0 with height 0x1234 and width 0x5678.
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 0x12, 0x34, 0x56, 0x78, 3, 0, 0, 0, 0,
        ];
        assert_eq!(jpeg_size(&jpeg), Some((0x5678, 0x1234)));
        let mut jpx = vec![0xFF, 0x4F, 0xFF, 0x51, 0, 47, 0, 0];
        jpx.extend_from_slice(&100_000u32.to_be_bytes());
        jpx.extend_from_slice(&200_000u32.to_be_bytes());
        jpx.extend_from_slice(&[0; 8]);
        assert_eq!(jpx_size(&jpx), Some((100_000, 200_000)));
        assert_eq!(jpeg_size(&[0xFF, 0xD8]), None);
    }
}
