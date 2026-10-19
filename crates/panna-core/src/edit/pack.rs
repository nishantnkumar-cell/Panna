//! Making a rebuilt file smaller: streams compressed (or compressed harder), identical streams
//! kept once, and small objects packed together into compressed object streams with a
//! compressed cross-reference stream. Nothing that changes how pages look is touched, unless
//! pictures may be scaled down too.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read as _, Write as _};

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

use super::lex;

/// What a smaller copy may change.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Compact {
    /// Scale pictures sharper than this many dots per inch, as drawn, down to it.
    pub pictures: Option<u32>,
}

/// Pictures are scaled down only when they're at least this much sharper than needed.
const WORTH_SCALING: f64 = 1.5;
/// Quality of pictures written again as JPEG.
const JPEG_QUALITY: u8 = 80;
/// Pictures with more pixels than this are left as they are.
const MAX_PIXELS: u64 = 100_000_000;

/// Streams are inflated up to this size to compress them again; bigger ones stay as they are.
const MAX_INFLATED: u64 = 256 * 1024 * 1024;
/// Smaller streams aren't worth a filter.
const MIN_STREAM: usize = 64;
/// Objects per object stream.
const PER_STREAM: usize = 200;

/// An object body split into its dictionary and stream data, when it's a stream.
fn split(body: &[u8]) -> Option<(&[u8], &[u8])> {
    let at = find(body, b"\nstream\n")?;
    let start = at + b"\nstream\n".len();
    let end = rfind(body, b"\nendstream").filter(|e| *e >= start)?;
    Some((body.get(..at)?, body.get(start..end)?))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

pub(crate) fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    // Writing to a Vec can't fail.
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    ZlibDecoder::new(data)
        .take(MAX_INFLATED + 1)
        .read_to_end(&mut out)
        .ok()?;
    (out.len() as u64 <= MAX_INFLATED).then_some(out)
}

/// A stream with `dict` (without its `/Length`, `/Filter` and `/DecodeParms`) and `data`.
fn stream(dict: &[u8], filter: bool, data: &[u8]) -> Vec<u8> {
    let mut body = b"<<".to_vec();
    for (key, value) in lex::dict_entries(dict).unwrap_or_default() {
        if matches!(key, b"Length" | b"Filter" | b"DecodeParms") {
            continue;
        }
        body.push(b'/');
        body.extend_from_slice(key);
        body.push(b' ');
        body.extend_from_slice(dict.get(value).unwrap_or_default());
        body.push(b'\n');
    }
    if filter {
        body.extend_from_slice(b"/Filter /FlateDecode\n");
    }
    body.extend_from_slice(format!("/Length {}\n>>\nstream\n", data.len()).as_bytes());
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}

/// The stream compressed, or compressed harder, when that makes it smaller.
fn compress(body: &[u8]) -> Option<Vec<u8>> {
    let (dict, data) = split(body)?;
    if data.len() < MIN_STREAM {
        return None;
    }
    let entries = lex::dict_entries(dict)?;
    let value = |key: &[u8]| {
        entries
            .iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| dict.get(v.clone()))
            .map(<[u8]>::trim_ascii)
    };
    let plain = match (value(b"Filter"), value(b"DecodeParms")) {
        (None, None) => data.to_vec(),
        // Only plain Flate is undone; predictors and other filters stay as they are.
        (Some(b"/FlateDecode" | b"[/FlateDecode]" | b"[ /FlateDecode ]"), None) => inflate(data)?,
        _ => return None,
    };
    let packed = deflate(&plain);
    (packed.len() + 20 < data.len()).then(|| stream(dict, true, &packed))
}

/// Compresses every stream that gets smaller for it.
pub(crate) fn compress_streams(objects: &mut [Option<Vec<u8>>]) {
    for body in objects.iter_mut().flatten() {
        if let Some(smaller) = compress(body) {
            *body = smaller;
        }
    }
}

/// The colour components of a picture's colour space, when it's one that can be scaled
/// pixel by pixel: grey or RGB, plain or with a profile.
fn components(space: &hayro::hayro_syntax::object::Object<'_>) -> Option<usize> {
    use hayro::hayro_syntax::object::{Array, Name, Object, Stream};
    match space {
        Object::Name(name) => match name.as_str() {
            "DeviceGray" | "G" | "CalGray" => Some(1),
            "DeviceRGB" | "RGB" | "CalRGB" => Some(3),
            _ => None,
        },
        Object::Array(array) => {
            let array: &Array<'_> = array;
            let mut items = array.flex_iter();
            let kind = items.next::<Name<'_>>()?;
            match kind.as_str() {
                "CalGray" => Some(1),
                "CalRGB" => Some(3),
                "ICCBased" => match items.next::<Stream<'_>>()?.dict().get::<u8>(b"N")? {
                    1 => Some(1),
                    3 => Some(3),
                    _ => None,
                },
                _ => None,
            }
        }
        _ => None,
    }
}

/// `body`, the copy of picture `source`, scaled down to `dpi` dots per inch at `drawn`, the
/// largest size it's drawn at (in points), when it's that much sharper and gets smaller for
/// it. JPEG pictures stay JPEG; others are compressed without loss.
pub(crate) fn shrink_picture(
    source: &hayro::hayro_syntax::object::Stream<'_>,
    body: &[u8],
    drawn: (f64, f64),
    dpi: u32,
) -> Option<Vec<u8>> {
    use hayro::hayro_syntax::Filter;
    use hayro::hayro_syntax::object::Object;
    let dict = source.dict();
    if dict.get::<bool>(b"ImageMask") == Some(true)
        || dict.contains_key(b"Mask")
        || dict.get::<u8>(b"BitsPerComponent") != Some(8)
    {
        return None;
    }
    let (width, height) = (dict.get::<u32>(b"Width")?, dict.get::<u32>(b"Height")?);
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return None;
    }
    let channels = components(&dict.get::<Object<'_>>(b"ColorSpace")?)?;
    let filters = source.filters();
    let lossless = |f: &Filter| {
        matches!(
            f,
            Filter::FlateDecode
                | Filter::LzwDecode
                | Filter::RunLengthDecode
                | Filter::AsciiHexDecode
                | Filter::Ascii85Decode
        )
    };
    let jpeg = match filters.as_slice() {
        [rest @ .., Filter::DctDecode] if rest.iter().all(lossless) => true,
        all if all.iter().all(lossless) => false,
        _ => return None,
    };
    // How many times sharper than needed, along the side that needs the most.
    let needed = |points: f64, pixels: u32| points / 72.0 * f64::from(dpi) / f64::from(pixels);
    let scale = needed(drawn.0, width).max(needed(drawn.1, height));
    if !(scale > 0.0 && scale * WORTH_SCALING <= 1.0) {
        return None;
    }
    let new_size = |pixels: u32| ((f64::from(pixels) * scale).ceil() as u32).max(1);
    let (new_width, new_height) = (new_size(width), new_size(height));
    let params = hayro::hayro_syntax::object::stream::ImageDecodeParams {
        is_indexed: false,
        bpc: Some(8),
        num_components: u8::try_from(channels).ok(),
        target_dimension: None,
        width,
        height,
    };
    let pixels = source.decoded_image(&params).ok()?.data;
    let len = width as usize * height as usize * channels;
    // Anything but one byte a component (such as a JPEG with other colours than its colour
    // space says) is left alone.
    if pixels.len() != len {
        return None;
    }
    let pixels = pixels.into_owned();
    let filter = image::imageops::FilterType::Triangle;
    let scaled = if channels == 1 {
        let picture = image::GrayImage::from_raw(width, height, pixels)?;
        image::imageops::resize(&picture, new_width, new_height, filter).into_raw()
    } else {
        let picture = image::RgbImage::from_raw(width, height, pixels)?;
        image::imageops::resize(&picture, new_width, new_height, filter).into_raw()
    };
    let data = if jpeg {
        use image::ImageEncoder as _;
        let mut out = Vec::new();
        let color = if channels == 1 {
            image::ExtendedColorType::L8
        } else {
            image::ExtendedColorType::Rgb8
        };
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
            .write_image(&scaled, new_width, new_height, color)
            .ok()?;
        out
    } else {
        deflate(&scaled)
    };
    let (old_dict, old_data) = split(body)?;
    if data.len() >= old_data.len() {
        return None;
    }
    let mut new = b"<<".to_vec();
    for (key, value) in lex::dict_entries(old_dict)? {
        if matches!(
            key,
            b"Length" | b"Filter" | b"DecodeParms" | b"Width" | b"Height" | b"BitsPerComponent"
        ) {
            continue;
        }
        new.push(b'/');
        new.extend_from_slice(key);
        new.push(b' ');
        new.extend_from_slice(old_dict.get(value)?);
        new.push(b'\n');
    }
    let filter = if jpeg { "DCTDecode" } else { "FlateDecode" };
    new.extend_from_slice(
        format!(
            "/Width {new_width}\n/Height {new_height}\n/BitsPerComponent 8\n/Filter /{filter}\n\
             /Length {}\n>>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    new.extend_from_slice(&data);
    new.extend_from_slice(b"\nendstream");
    Some(new)
}

/// Keeps one copy of streams that are byte for byte the same (fonts and pictures repeated in
/// combined files), pointing every reference at it. `keep` objects are never merged away.
pub(crate) fn merge_duplicates(objects: &mut [Option<Vec<u8>>], keep: &[u32]) {
    let mut first: HashMap<u64, Vec<u32>> = HashMap::new();
    let mut moved: HashMap<u32, u32> = HashMap::new();
    for (i, body) in objects.iter().enumerate() {
        let num = i as u32 + 1;
        let Some(body) = body.as_deref().filter(|b| split(b).is_some()) else {
            continue;
        };
        if keep.contains(&num) {
            continue;
        }
        let mut hasher = DefaultHasher::new();
        body.hash(&mut hasher);
        let candidates = first.entry(hasher.finish()).or_default();
        let same = candidates.iter().copied().find(|&c| {
            objects
                .get(c as usize - 1)
                .and_then(Option::as_deref)
                .is_some_and(|other| other == body)
        });
        match same {
            Some(original) => {
                moved.insert(num, original);
            }
            None => candidates.push(num),
        }
    }
    if moved.is_empty() {
        return;
    }
    for &num in moved.keys() {
        if let Some(slot) = objects.get_mut(num as usize - 1) {
            *slot = None;
        }
    }
    let mut remap = |r: crate::ObjRef| Some(moved.get(&r.number).copied().unwrap_or(r.number));
    let mut same = |s: &[u8]| Ok::<_, ()>(s.to_vec());
    for body in objects.iter_mut().flatten() {
        let (dict, data) = match split(body) {
            Some((dict, data)) => (dict, Some(data)),
            None => (&body[..], None),
        };
        let Some(Ok(mut new)) = lex::transform(dict, &mut remap, &mut same) else {
            continue;
        };
        if let Some(data) = data {
            new.extend_from_slice(b"\nstream\n");
            new.extend_from_slice(data);
            new.extend_from_slice(b"\nendstream");
        }
        *body = new;
    }
}

/// Where each packed object went: which object stream (by its place in the list), and its
/// index in it.
pub(crate) type Places = HashMap<u32, (usize, usize)>;

/// The object streams holding every object that isn't a stream itself (nor in `apart`), and
/// for each packed object, which object stream (by its place in the list) and index it's at.
pub(crate) fn object_streams(objects: &[Option<Vec<u8>>], apart: &[u32]) -> (Vec<Vec<u8>>, Places) {
    let packable: Vec<u32> = objects
        .iter()
        .enumerate()
        .map(|(i, b)| (i as u32 + 1, b))
        .filter(|(num, b)| !apart.contains(num) && b.as_deref().is_none_or(|b| split(b).is_none()))
        .map(|(num, _)| num)
        .collect();
    let mut streams = Vec::new();
    let mut places = HashMap::new();
    for (s, chunk) in packable.chunks(PER_STREAM).enumerate() {
        let mut header = String::new();
        let mut data = Vec::new();
        for (index, &num) in chunk.iter().enumerate() {
            header.push_str(&format!("{num} {} ", data.len()));
            let body = objects
                .get(num as usize - 1)
                .and_then(Option::as_deref)
                .unwrap_or(b"null");
            data.extend_from_slice(body);
            data.push(b'\n');
            places.insert(num, (s, index));
        }
        header.push('\n');
        let mut plain = header.clone().into_bytes();
        plain.extend_from_slice(&data);
        let packed = deflate(&plain);
        let mut body = format!(
            "<< /Type /ObjStm /N {} /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
            chunk.len(),
            header.len(),
            packed.len()
        )
        .into_bytes();
        body.extend_from_slice(&packed);
        body.extend_from_slice(b"\nendstream");
        streams.push(body);
    }
    (streams, places)
}

/// One cross-reference stream entry: type, then field 2 (offset or object stream number)
/// and field 3 (index in it), in 1, 4 and 2 bytes.
pub(crate) fn xref_entry(kind: u8, field: u32, index: u16) -> [u8; 7] {
    let [a, b, c, d] = field.to_be_bytes();
    let [e, f] = index.to_be_bytes();
    [kind, a, b, c, d, e, f]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut b = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        b.extend_from_slice(data);
        b.extend_from_slice(b"\nendstream");
        b
    }

    #[test]
    fn plain_streams_are_compressed_and_others_left() {
        let text = b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET\n".repeat(50);
        let packed = compress(&body("", &text)).unwrap();
        let (dict, data) = split(&packed).unwrap();
        assert!(find(dict, b"/Filter /FlateDecode").is_some());
        assert_eq!(inflate(data).unwrap(), text);
        // Already compressed at a low level: compressed harder.
        let mut low = ZlibEncoder::new(Vec::new(), Compression::fast());
        low.write_all(&text).unwrap();
        let low = low.finish().unwrap();
        let again = compress(&body("/Filter /FlateDecode", &low));
        assert!(again.is_none_or(|b| split(&b).unwrap().1.len() < low.len()));
        // Pictures and predictors are left alone.
        assert!(compress(&body("/Filter /DCTDecode", &text)).is_none());
        assert!(
            compress(&body(
                "/Filter /FlateDecode /DecodeParms << /Predictor 12 >>",
                &low
            ))
            .is_none()
        );
        assert!(compress(&body("", b"short")).is_none());
    }

    #[test]
    fn identical_streams_are_kept_once() {
        let font = body("/Subtype /Type1C", &[7u8; 300]);
        let mut objects = vec![
            Some(b"<< /F1 2 0 R /F2 3 0 R >>".to_vec()),
            Some(font.clone()),
            Some(font),
            Some(body("/Res 3 0 R", b"q Q")),
        ];
        merge_duplicates(&mut objects, &[]);
        assert!(objects[2].is_none());
        let first = String::from_utf8_lossy(objects[0].as_deref().unwrap()).into_owned();
        assert!(first.contains("2 0 R") && !first.contains("3 0 R"), "{first}");
        let last = objects[3].as_deref().unwrap();
        assert!(find(last, b"3 0 R").is_none() && last.ends_with(b"q Q\nendstream"));
    }

    #[test]
    fn small_objects_are_packed_and_streams_stay_out() {
        let objects = vec![
            Some(b"<< /Type /Catalog >>".to_vec()),
            Some(body("", b"q Q")),
            None,
            Some(b"[1 2 3]".to_vec()),
        ];
        let (streams, places) = object_streams(&objects, &[4]);
        assert_eq!(streams.len(), 1);
        assert_eq!(places.get(&1), Some(&(0, 0)));
        assert_eq!(places.get(&3), Some(&(0, 1)));
        assert!(!places.contains_key(&2) && !places.contains_key(&4));
        let (_, data) = split(&streams[0]).unwrap();
        let plain = String::from_utf8(inflate(data).unwrap()).unwrap();
        assert!(
            plain.starts_with("1 0 3 21 \n<< /Type /Catalog >>\nnull\n"),
            "{plain:?}"
        );
    }
}
