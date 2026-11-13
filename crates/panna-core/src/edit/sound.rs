//! Spoken comments (Acrobat's Record Audio tool): a sound recorded into the document,
//! shown as a speaker on the page, and the sounds in documents read back to be played.

use std::io::{Read as _, Write as _};
use std::sync::Arc;

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use hayro::hayro_syntax::object::{Name, Stream};

use crate::comments::ObjRef;

/// The longest recording kept, in seconds.
pub const MAX_SOUND_SECONDS: u32 = 10 * 60;
/// Samples read from a document's sound at most (ten minutes of stereo at 48 kHz).
const MAX_SAMPLES: usize = 48_000 * 2 * 600;

/// Sound to play: 16-bit samples, the channels interleaved.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sound {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl Sound {
    /// How long it plays, in seconds.
    pub fn seconds(&self) -> f32 {
        let frames = self.samples.len() / usize::from(self.channels.max(1));
        frames as f32 / self.rate.max(1) as f32
    }
}

/// A spoken comment's sound: a new recording, carried compressed, or one already in the file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Recording {
    pub rate: u32,
    pub channels: u16,
    /// How long it plays, in seconds.
    pub seconds: f32,
    source: Source,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum Source {
    /// The samples, as the PDF stores them (16-bit, most significant byte first),
    /// zlib-compressed.
    New(Arc<Vec<u8>>),
    /// The sound stream already in the document.
    Saved(ObjRef),
}

impl PartialEq for Recording {
    fn eq(&self, other: &Self) -> bool {
        self.rate == other.rate
            && self.channels == other.channels
            && match (&self.source, &other.source) {
                (Source::New(a), Source::New(b)) => Arc::ptr_eq(a, b),
                (Source::Saved(a), Source::Saved(b)) => a == b,
                _ => false,
            }
    }
}

impl Recording {
    /// A new recording of `sound`, cut to [`MAX_SOUND_SECONDS`].
    pub fn new(sound: &Sound) -> Result<Self, String> {
        if sound.rate == 0 || sound.channels == 0 || sound.samples.is_empty() {
            return Err("Nothing was recorded.".into());
        }
        let most = sound.rate as usize * usize::from(sound.channels) * MAX_SOUND_SECONDS as usize;
        let samples = sound.samples.get(..most).unwrap_or(&sound.samples);
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        for s in samples {
            encoder
                .write_all(&s.to_be_bytes())
                .map_err(|e| format!("The recording couldn't be compressed: {e}"))?;
        }
        let data = encoder
            .finish()
            .map_err(|e| format!("The recording couldn't be compressed: {e}"))?;
        let frames = samples.len() / usize::from(sound.channels);
        Ok(Self {
            rate: sound.rate,
            channels: sound.channels,
            seconds: frames as f32 / sound.rate as f32,
            source: Source::New(Arc::new(data)),
        })
    }

    /// A sound already in the document, by its stream.
    pub(crate) fn saved(rate: u32, channels: u16, seconds: f32, stream: ObjRef) -> Self {
        Self {
            rate,
            channels,
            seconds,
            source: Source::Saved(stream),
        }
    }

    /// The sound stream in the document, for one already saved.
    pub fn saved_stream(&self) -> Option<ObjRef> {
        match self.source {
            Source::Saved(r) => Some(r),
            Source::New(_) => None,
        }
    }

    /// A new recording's sound, to play before it's saved.
    pub fn sound(&self) -> Option<Sound> {
        let Source::New(data) = &self.source else {
            return None;
        };
        let mut raw = Vec::new();
        ZlibDecoder::new(data.as_slice())
            .take((MAX_SAMPLES * 2) as u64)
            .read_to_end(&mut raw)
            .ok()?;
        Some(Sound {
            rate: self.rate,
            channels: self.channels,
            samples: raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&b| i16::from_be_bytes(b))
                .collect(),
        })
    }

    /// The sound stream for a new recording; `None` for one already saved.
    pub(crate) fn stream(&self) -> Option<Vec<u8>> {
        let Source::New(data) = &self.source else {
            return None;
        };
        let mut body = format!(
            "<< /Type /Sound /R {} /C {} /B 16 /E /Signed /Filter /FlateDecode /Length {} >>\nstream\n",
            self.rate,
            self.channels,
            data.len(),
        )
        .into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        Some(body)
    }

    /// The value of the annotation's /Sound: `new`, the stream just written, or a reference to
    /// the one already saved.
    pub(crate) fn reference(&self, new: Option<&str>) -> String {
        match (&self.source, new) {
            (Source::Saved(r), _) => format!("{} {} R", r.number, r.generation),
            (Source::New(_), Some(stream)) => stream.to_owned(),
            (Source::New(_), None) => "null".to_owned(),
        }
    }
}

/// How a sound stream's samples are stored.
struct Format {
    rate: u32,
    channels: u16,
    bits: u8,
    encoding: Encoding,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Raw,
    Signed,
    MuLaw,
    ALaw,
}

fn format(stream: &Stream<'_>) -> Option<Format> {
    let dict = stream.dict();
    let rate = dict.get::<f64>(b"R").filter(|r| (1.0..=384_000.0).contains(r))? as u32;
    let channels = dict.get::<i64>(b"C").unwrap_or(1);
    let bits = dict.get::<i64>(b"B").unwrap_or(8);
    let encoding = match dict.get::<Name<'_>>(b"E").as_ref().map(Name::as_str) {
        None | Some("Raw") => Encoding::Raw,
        Some("Signed") => Encoding::Signed,
        Some("muLaw") => Encoding::MuLaw,
        Some("ALaw") => Encoding::ALaw,
        Some(_) => return None,
    };
    let bits = match (encoding, bits) {
        (Encoding::MuLaw | Encoding::ALaw, _) => 8,
        (_, 8 | 16) => bits as u8,
        _ => return None,
    };
    Some(Format {
        rate,
        channels: u16::try_from(channels).ok().filter(|c| (1..=8).contains(c))?,
        bits,
        encoding,
    })
}

/// How long the sound in `stream` plays, in seconds, from its size.
pub(crate) fn seconds(stream: &Stream<'_>) -> Option<(u32, u16, f32)> {
    let f = format(stream)?;
    let bytes = stream.decoded().ok()?.len();
    let frames = bytes / (usize::from(f.bits / 8) * usize::from(f.channels));
    Some((f.rate, f.channels, frames as f32 / f.rate as f32))
}

fn mu_law(byte: u8) -> i16 {
    let b = !byte;
    let sign = b & 0x80;
    let exponent = (b >> 4) & 0x07;
    let mantissa = i32::from(b & 0x0f);
    let magnitude = ((mantissa << 3) + 0x84) << exponent;
    let v = magnitude - 0x84;
    (if sign != 0 { -v } else { v }) as i16
}

fn a_law(byte: u8) -> i16 {
    let b = byte ^ 0x55;
    let sign = b & 0x80;
    let exponent = (b >> 4) & 0x07;
    let mantissa = i32::from(b & 0x0f);
    let v = if exponent == 0 {
        (mantissa << 4) + 8
    } else {
        ((mantissa << 4) + 0x108) << (exponent - 1)
    };
    (if sign != 0 { v } else { -v }) as i16
}

/// The sound in a document's sound stream, as 16-bit samples.
pub(crate) fn decode(stream: &Stream<'_>) -> Option<Sound> {
    let f = format(stream)?;
    let data = stream.decoded().ok()?;
    let samples: Vec<i16> = match (f.bits, f.encoding) {
        (16, Encoding::Raw) => data
            .as_chunks::<2>()
            .0
            .iter()
            .take(MAX_SAMPLES)
            .map(|&b| (i32::from(u16::from_be_bytes(b)) - 32768) as i16)
            .collect(),
        (16, _) => data
            .as_chunks::<2>()
            .0
            .iter()
            .take(MAX_SAMPLES)
            .map(|&b| i16::from_be_bytes(b))
            .collect(),
        (_, Encoding::Raw) => data
            .iter()
            .take(MAX_SAMPLES)
            .map(|&b| (i16::from(b) - 128) << 8)
            .collect(),
        (_, Encoding::Signed) => data
            .iter()
            .take(MAX_SAMPLES)
            .map(|&b| i16::from(b as i8) << 8)
            .collect(),
        (_, Encoding::MuLaw) => data.iter().take(MAX_SAMPLES).map(|&b| mu_law(b)).collect(),
        (_, Encoding::ALaw) => data.iter().take(MAX_SAMPLES).map(|&b| a_law(b)).collect(),
    };
    Some(Sound {
        rate: f.rate,
        channels: f.channels,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recording_keeps_its_samples() {
        let sound = Sound {
            rate: 8000,
            channels: 1,
            samples: vec![0, 1000, -1000, i16::MAX, i16::MIN],
        };
        let rec = Recording::new(&sound).unwrap();
        assert_eq!(rec.sound().unwrap(), sound);
        assert!((rec.seconds - 5.0 / 8000.0).abs() < 1e-6);
    }

    #[test]
    fn companded_samples_decode_near_their_values() {
        // Silence and the loudest codes.
        assert_eq!(mu_law(0xff), 0);
        assert_eq!(mu_law(0x80), 32124);
        assert_eq!(mu_law(0x00), -32124);
        assert_eq!(a_law(0xd5), 8);
        assert_eq!(a_law(0x55), -8);
    }
}
