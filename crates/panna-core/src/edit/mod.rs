//! Writing changes back into a PDF as an incremental update.

mod annot;
pub(crate) mod lex;
mod update;

use std::collections::BTreeMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use hayro::hayro_interpret::TransformExt;
use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::Array;

pub use annot::{Annotation, Color, Kind, LINE_HEIGHT, NOTE_SIZE, text_width, wrap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The file's structure is too damaged to append to safely.
    Damaged,
    /// Saving changes into encrypted files isn't supported yet.
    Encrypted,
    NoSuchPage(usize),
    EngineCrashed,
    Io(String),
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
                "Saving changes to password-protected PDFs isn't supported yet."
            ),
            Self::NoSuchPage(i) => write!(f, "Page {} doesn't exist.", i + 1),
            Self::EngineCrashed => write!(f, "The PDF engine failed while preparing the changes."),
            Self::Io(e) => write!(f, "Couldn't write the file: {e}"),
        }
    }
}

impl std::error::Error for SaveError {}

/// Returns `original` with `annotations` appended as an incremental update. With no
/// annotations the bytes are returned unchanged.
pub(crate) fn add_annotations(
    pdf: &Pdf,
    original: &[u8],
    annotations: &[Annotation],
) -> Result<Vec<u8>, SaveError> {
    if annotations.is_empty() {
        return Ok(original.to_vec());
    }
    let trailer = update::read_trailer(original)?;
    if trailer.encrypted {
        return Err(SaveError::Encrypted);
    }
    let mut out = update::Update::new(trailer);
    let date = pdf_date(SystemTime::now());
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());

    let mut by_page: BTreeMap<usize, Vec<&Annotation>> = BTreeMap::new();
    for a in annotations {
        by_page.entry(a.page).or_default().push(a);
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
            let ap_id = out.alloc();
            serial += 1;
            let built = a.build(
                &info,
                &format!("{ap_id} 0 R"),
                &format!("panna-{stamp:x}-{serial}"),
                &date,
            );
            out.put(annot_id, 0, built.dict);
            out.put(ap_id, 0, built.appearance);
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
            dict.extend_from_slice(existing.data());
            dict.push(b' ');
        }
        dict.extend_from_slice(refs.join(" ").as_bytes());
        dict.extend_from_slice(b"]\n>>");
        out.put(num, generation, dict);
    }
    Ok(out.finish(original))
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
