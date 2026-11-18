//! Making a PDF/X-4 copy for printing, as an incremental update: an output intent with the
//! print shop's colour profile, metadata naming the file PDF/X-4, a file identifier, the
//! document marked as not trapped, and a trim box on every page that has none (its crop box,
//! so nothing is cut). Fonts that aren't embedded can't be fixed this way; the copy is checked
//! afterwards.

use std::time::SystemTime;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Dict, ObjectIdentifier};

use super::SaveError;
use super::update::{Update, read_trailer};

/// Largest colour profile accepted.
pub const MAX_PROFILE: usize = 16 * 1024 * 1024;

/// A colour profile checked for use as a PDF/X output intent.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrintProfile {
    pub bytes: Vec<u8>,
    /// Its own description, such as "Coated FOGRA39 (ISO 12647-2:2004)".
    pub name: String,
    /// Colour components: 4 for CMYK, 3 for RGB, 1 for grey.
    pub components: u8,
}

/// Why a profile can't be a PDF/X output intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    NotIcc,
    TooBig,
    /// A display or other profile rather than one for a printer.
    NotOutput,
    /// A colour space other than CMYK, RGB or grey.
    Colours,
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ProfileError::NotIcc => "That isn't an ICC colour profile.",
            ProfileError::TooBig => "That colour profile is too large.",
            ProfileError::NotOutput => {
                "That's a profile for screens or other devices. PDF/X needs the printing \
                 condition's output profile, such as one from the print shop."
            }
            ProfileError::Colours => "PDF/X needs a CMYK, RGB or grey profile.",
        })
    }
}

impl PrintProfile {
    /// The largest profile [`PrintProfile::read`] takes.
    pub const MAX_BYTES: usize = MAX_PROFILE;

    /// Checks `bytes` as an ICC output profile; `fallback` names it when it has no description.
    pub fn read(bytes: Vec<u8>, fallback: &str) -> Result<Self, ProfileError> {
        if bytes.len() > MAX_PROFILE {
            return Err(ProfileError::TooBig);
        }
        let field = |at: usize| bytes.get(at..at + 4);
        let declared = be32(&bytes, 0);
        if bytes.len() < 132 || field(36) != Some(b"acsp") || declared.is_none_or(|d| d > bytes.len()) {
            return Err(ProfileError::NotIcc);
        }
        if field(12) != Some(b"prtr") {
            return Err(ProfileError::NotOutput);
        }
        let components = match field(16) {
            Some(b"CMYK") => 4,
            Some(b"RGB ") => 3,
            Some(b"GRAY") => 1,
            _ => return Err(ProfileError::Colours),
        };
        let name = description(&bytes)
            .map(|n| {
                n.chars()
                    .filter(|c| !c.is_control())
                    .take(200)
                    .collect::<String>()
            })
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| fallback.chars().filter(|c| !c.is_control()).take(200).collect());
        Ok(Self {
            bytes,
            name,
            components,
        })
    }
}

/// The big-endian 32-bit number at `at`.
fn be32(bytes: &[u8], at: usize) -> Option<usize> {
    let b: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
    usize::try_from(u32::from_be_bytes(b)).ok()
}

/// The profile's `desc` tag: ASCII in version 2 profiles, the first UTF-16 name in version 4.
fn description(icc: &[u8]) -> Option<String> {
    let u32_at = |at: usize| be32(icc, at);
    let count = u32_at(128)?.min(200);
    let (offset, size) = (0..count).find_map(|i| {
        let at = 132 + i * 12;
        (icc.get(at..at + 4)? == b"desc").then(|| Some((u32_at(at + 4)?, u32_at(at + 8)?)))?
    })?;
    let tag = icc.get(offset..offset.checked_add(size)?)?;
    match tag.get(..4)? {
        b"desc" => {
            let len = u32_at(offset + 8)?.min(size.saturating_sub(12));
            let text = tag.get(12..12 + len)?;
            Some(String::from_utf8_lossy(text).trim_end_matches('\0').to_owned())
        }
        b"mluc" => {
            let len = u32_at(offset + 20)?;
            let start = u32_at(offset + 24)?;
            let units: Vec<u16> = tag
                .get(start..start.checked_add(len)?)?
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_be_bytes(*p))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

fn escape_xml(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .fold(String::new(), |mut out, c| {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                c => out.push(c),
            }
            out
        })
}

/// An XMP date (`2026-11-17T09:30:00Z`) from a PDF date (`D:20261117093000Z`).
fn xmp_date(pdf: &str) -> String {
    let d = pdf.trim_start_matches("D:");
    let part = |r: std::ops::Range<usize>| d.get(r).unwrap_or("00");
    format!(
        "{}-{}-{}T{}:{}:{}Z",
        d.get(0..4).unwrap_or("1970"),
        part(4..6),
        part(6..8),
        part(8..10),
        part(10..12),
        part(12..14)
    )
}

fn xmp(title: &str, author: Option<&str>, document_id: &str, now: &str) -> Vec<u8> {
    let mut extra = String::new();
    if let Some(a) = author.filter(|a| !a.trim().is_empty()) {
        extra.push_str(&format!(
            "<dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>\n",
            escape_xml(a)
        ));
    }
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
         <rdf:Description rdf:about=\"\" xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" \
         xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmlns:xmpMM=\"http://ns.adobe.com/xap/1.0/mm/\">\n\
         <pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>\n\
         <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\n\
         {extra}<pdf:Producer>Panna</pdf:Producer>\n<pdf:Trapped>False</pdf:Trapped>\n\
         <xmp:CreateDate>{now}</xmp:CreateDate>\n<xmp:ModifyDate>{now}</xmp:ModifyDate>\n\
         <xmp:MetadataDate>{now}</xmp:MetadataDate>\n\
         <xmpMM:DocumentID>uuid:{document_id}</xmpMM:DocumentID>\n\
         <xmpMM:InstanceID>uuid:{document_id}</xmpMM:InstanceID>\n\
         <xmpMM:VersionID>1</xmpMM:VersionID>\n<xmpMM:RenditionClass>default</xmpMM:RenditionClass>\n\
         </rdf:Description>\n</rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>",
        escape_xml(title)
    )
    .into_bytes()
}

/// 16 bytes for a new file identifier, from the file's own bytes and the time: unique enough
/// to tell files apart, which is all an identifier is for.
fn new_id(original: &[u8], now: SystemTime) -> [u8; 16] {
    let nanos = now
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut out = [0u8; 16];
    for (half, seed) in out
        .as_chunks_mut::<8>()
        .0
        .iter_mut()
        .zip([0xcbf2_9ce4_8422_2325u64, 0x8422_2325_cbf2_9ce4])
    {
        let mut hash = seed ^ (nanos as u64);
        for b in original.iter().step_by((original.len() / 65_536).max(1)) {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= original.len() as u64;
        *half = hash.to_be_bytes();
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// The PDF/X-4 copy of `original`.
pub(crate) fn to_pdfx(pdf: &Pdf, original: &[u8], profile: &PrintProfile) -> Result<Vec<u8>, SaveError> {
    let mut trailer = read_trailer(original)?;
    if trailer.encrypt.is_some() {
        return Err(SaveError::PrintProtected);
    }
    let now_time = SystemTime::now();
    let now = super::pdf_date(now_time);
    let id = new_id(original, now_time);
    let document_id = {
        let h = hex(&id).to_ascii_lowercase();
        format!(
            "{}-{}-{}-{}-{}",
            h.get(0..8).unwrap_or_default(),
            h.get(8..12).unwrap_or_default(),
            h.get(12..16).unwrap_or_default(),
            h.get(16..20).unwrap_or_default(),
            h.get(20..32).unwrap_or_default()
        )
    };
    if trailer.id.is_none() {
        trailer.id = Some(format!("[<{0}> <{0}>]", hex(&id)).into_bytes());
    }
    let root = super::lex::reference(&trailer.root).ok_or(SaveError::Damaged)?;
    let info_ref = trailer.info.as_deref().and_then(super::lex::reference);
    let catalog = pdf
        .xref()
        .get::<Dict<'_>>(ObjectIdentifier::new(root.number, i32::from(root.generation)))
        .ok_or(SaveError::Damaged)?;

    let meta = pdf.metadata();
    let text = |v: &Option<Box<[u8]>>| v.as_deref().map(crate::document::decode_text_string);
    let title = text(&meta.title)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Untitled".to_owned());
    let author = text(&meta.author);

    let mut out = Update::new(trailer);

    // The document information: as it was, plus the dates and trapping PDF/X asks for.
    let info_num = out.alloc();
    let mut info: Vec<(&'static str, String)> = vec![
        ("Title", super::annot::text_string(&title)),
        ("Trapped", "/False".to_owned()),
        ("ModDate", format!("({now})")),
        ("GTS_PDFXVersion", "(PDF/X-4)".to_owned()),
        ("Producer", "(Panna)".to_owned()),
    ];
    let old_info = info_ref.and_then(|r| {
        pdf.xref()
            .get::<Dict<'_>>(ObjectIdentifier::new(r.number, i32::from(r.generation)))
    });
    if old_info.as_ref().is_none_or(|d| !d.contains_key(b"CreationDate")) {
        info.push(("CreationDate", format!("({now})")));
    }
    let info_body = match &old_info {
        Some(old) => super::rewrite(old.data(), &info)?,
        None => super::rewrite(b"<< >>", &info)?,
    };
    out.put(info_num, 0, info_body)?;

    let xmp = xmp(&title, author.as_deref(), &document_id, &xmp_date(&now));
    let meta_num = out.alloc();
    let mut stream = format!(
        "<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n",
        xmp.len()
    )
    .into_bytes();
    stream.extend(xmp);
    stream.extend_from_slice(b"\nendstream");
    out.put(meta_num, 0, stream)?;

    let profile_num = out.alloc();
    out.put(
        profile_num,
        0,
        super::rebuild::flate_stream(&format!("/N {} ", profile.components), &profile.bytes),
    )?;
    let name = super::annot::text_string(&profile.name);
    let mut set: Vec<(&'static str, String)> = vec![
        (
            "OutputIntents",
            format!(
                "[<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier {name} \
                 /OutputCondition {name} /Info {name} /DestOutputProfile {profile_num} 0 R >>]"
            ),
        ),
        ("Metadata", format!("{meta_num} 0 R")),
    ];
    // PDF/X-4 is PDF 1.6 or later.
    let header = original.get(5..8).unwrap_or_default();
    if header < b"1.6".as_slice() && header.starts_with(b"1.") {
        set.push(("Version", "/1.6".to_owned()));
    }
    out.put_copied(
        root.number,
        root.generation,
        super::rewrite(catalog.data(), &set)?,
    );

    // Each page gets a trim box: its crop box, so nothing more is cut than is now.
    for page in pdf.pages().iter() {
        let raw = page.raw();
        if raw.contains_key(b"TrimBox") || raw.contains_key(b"ArtBox") {
            continue;
        }
        let Some(page_id) = raw.obj_id() else {
            return Err(SaveError::Damaged);
        };
        let b = page.intersected_crop_box();
        let r = |v: f64| format!("{}", (v * 1000.0).round() / 1000.0);
        let trim = format!("[{} {} {} {}]", r(b.x0), r(b.y0), r(b.x1), r(b.y1));
        let generation = u16::try_from(page_id.gen_number).map_err(|_| SaveError::Damaged)?;
        out.put_copied(
            page_id.obj_number,
            generation,
            super::rewrite(raw.data(), &vec![("TrimBox", trim)])?,
        );
    }

    // The update points the trailer at the new information dictionary.
    out.set_info(format!("{info_num} 0 R").into_bytes());
    Ok(out.finish(original))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(class: &[u8; 4], space: &[u8; 4], desc: &str) -> Vec<u8> {
        let mut tag = b"desc\0\0\0\0".to_vec();
        tag.extend(((desc.len() + 1) as u32).to_be_bytes());
        tag.extend(desc.as_bytes());
        tag.push(0);
        let mut icc = vec![0u8; 128];
        icc[12..16].copy_from_slice(class);
        icc[16..20].copy_from_slice(space);
        icc[36..40].copy_from_slice(b"acsp");
        icc.extend(1u32.to_be_bytes());
        icc.extend(b"desc");
        icc.extend((144u32).to_be_bytes());
        icc.extend((tag.len() as u32).to_be_bytes());
        icc.extend(tag);
        let size = icc.len() as u32;
        icc[0..4].copy_from_slice(&size.to_be_bytes());
        icc
    }

    #[test]
    fn reads_output_profiles_only() {
        let p = PrintProfile::read(profile(b"prtr", b"CMYK", "Coated FOGRA39"), "file")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((p.name.as_str(), p.components), ("Coated FOGRA39", 4));
        assert_eq!(
            PrintProfile::read(profile(b"mntr", b"RGB ", "sRGB"), "x"),
            Err(ProfileError::NotOutput)
        );
        assert_eq!(
            PrintProfile::read(profile(b"prtr", b"Lab ", "lab"), "x"),
            Err(ProfileError::Colours)
        );
        assert_eq!(
            PrintProfile::read(b"not a profile".to_vec(), "x"),
            Err(ProfileError::NotIcc)
        );
        let unnamed =
            PrintProfile::read(profile(b"prtr", b"GRAY", ""), "press.icc").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((unnamed.name.as_str(), unnamed.components), ("press.icc", 1));
    }

    #[test]
    fn xmp_dates() {
        assert_eq!(xmp_date("D:20261117093005Z"), "2026-11-17T09:30:05Z");
    }
}
