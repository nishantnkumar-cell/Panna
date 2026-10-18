//! Writing changes back into a PDF as an incremental update.

mod annot;
mod crypt;
mod field;
pub(crate) mod lex;
mod update;

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use hayro::hayro_interpret::TransformExt;
use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, ObjectIdentifier};

pub use annot::{Annotation, Color, Kind, LINE_HEIGHT, NOTE_SIZE, text_width, wrap};

use crate::{CommentId, FieldValue, ObjRef};

/// Unsaved changes to a document's comments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    /// New annotations, oldest first.
    pub add: Vec<Annotation>,
    /// Saved comments to take out.
    pub remove: Vec<CommentId>,
    /// Form fields filled in, by the field's object, with their new values.
    pub fields: Vec<(ObjRef, FieldValue)>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty() && self.fields.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The file's structure is too damaged to append to safely.
    Damaged,
    /// The file is encrypted in a way changes can't be saved into.
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
                "This PDF's password protection uses a method Panna can't save changes into."
            ),
            Self::NoSuchPage(i) => write!(f, "Page {} doesn't exist.", i + 1),
            Self::EngineCrashed => write!(f, "The PDF engine failed while preparing the changes."),
            Self::Io(e) => write!(f, "Couldn't write the file: {e}"),
        }
    }
}

impl std::error::Error for SaveError {}

/// Returns `original` with `changes` appended as an incremental update. With no changes the
/// bytes are returned unchanged.
///
/// `password` is the one the file was opened with; for encrypted files it is needed again to
/// encrypt what is added.
pub(crate) fn apply(
    pdf: &Pdf,
    original: &[u8],
    changes: &Changes,
    password: &str,
) -> Result<Vec<u8>, SaveError> {
    if changes.is_empty() {
        return Ok(original.to_vec());
    }
    let trailer = update::read_trailer(original)?;
    let crypt = match &trailer.encrypt {
        Some(raw) => Some(encryption(pdf, raw, trailer.id.as_deref(), password)?),
        None => None,
    };
    let mut out = update::Update::new(trailer);
    if let Some(crypt) = crypt {
        out.encrypt_with(crypt);
    }
    let date = pdf_date(SystemTime::now());
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());

    let mut by_page: BTreeMap<usize, Vec<&Annotation>> = BTreeMap::new();
    for a in &changes.add {
        by_page.entry(a.page).or_default().push(a);
    }
    let mut removed: BTreeMap<usize, HashSet<ObjRef>> = BTreeMap::new();
    for id in &changes.remove {
        by_page.entry(id.page).or_default();
        let refs = removed.entry(id.page).or_default();
        refs.insert(id.annot);
        refs.extend(id.popup);
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
            let ap_id = a.kind.drawn().then(|| out.alloc());
            serial += 1;
            let built = a.build(
                &info,
                &ap_id.map_or_else(String::new, |id| format!("{id} 0 R")),
                &format!("panna-{stamp:x}-{serial}"),
                &date,
            );
            out.put(annot_id, 0, built.dict)?;
            if let (Some(id), Some(appearance)) = (ap_id, built.appearance) {
                out.put(id, 0, appearance)?;
            }
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
            let gone = removed.get(&index);
            let data = existing.data();
            for item in lex::array_items(data).ok_or(SaveError::Damaged)? {
                let value = data.get(item).ok_or(SaveError::Damaged)?;
                if gone.is_some_and(|gone| lex::reference(value).is_some_and(|r| gone.contains(&r))) {
                    continue;
                }
                dict.extend_from_slice(value);
                dict.push(b' ');
            }
        }
        dict.extend_from_slice(refs.join(" ").as_bytes());
        dict.extend_from_slice(b"]\n>>");
        out.put_copied(num, generation, dict);
    }
    fill_fields(pdf, &mut out, &changes.fields)?;
    Ok(out.finish(original))
}

/// Writes filled-in field values: each field's /V, a new appearance for text and choice
/// widgets, and the on or off state of checkbox and radio widgets.
fn fill_fields(
    pdf: &Pdf,
    out: &mut update::Update,
    values: &[(ObjRef, FieldValue)],
) -> Result<(), SaveError> {
    if values.is_empty() {
        return Ok(());
    }
    let fields = crate::forms::read(pdf);
    let xref = pdf.xref();
    let get = |r: ObjRef| {
        xref.get::<Dict<'_>>(ObjectIdentifier::new(
            i32::try_from(r.number).unwrap_or(-1),
            i32::from(r.generation),
        ))
    };
    // Every edit to one object, so a field that is also its own widget is written once.
    let mut edits: BTreeMap<(u32, u16), field::Entries> = BTreeMap::new();
    for (id, value) in values {
        let Some(f) = fields.iter().find(|f| f.id == *id) else {
            continue;
        };
        edits
            .entry((id.number, id.generation))
            .or_default()
            .push(("V", field::value(f, value)));
        for w in &f.widgets {
            let entry = match value {
                FieldValue::State(state) => {
                    let on = state.is_some() && w.on == *state;
                    let shown = if on {
                        w.on.as_deref().unwrap_or("Off")
                    } else {
                        "Off"
                    };
                    ("AS", field::name(shown))
                }
                FieldValue::Text(text) => {
                    let Some(widget) = get(w.annot) else { continue };
                    let Some(stream) = field::appearance(f, &widget, text) else {
                        continue;
                    };
                    let ap = out.alloc();
                    out.put(ap, 0, stream)?;
                    ("AP", format!("<< /N {ap} 0 R >>"))
                }
            };
            edits
                .entry((w.annot.number, w.annot.generation))
                .or_default()
                .push(entry);
        }
    }
    for ((number, generation), set) in edits {
        let dict = get(ObjRef { number, generation }).ok_or(SaveError::Damaged)?;
        let set = set
            .into_iter()
            .map(|(key, value)| Ok((key, out.seal_value(number, generation, value)?)))
            .collect::<Result<field::Entries, SaveError>>()?;
        out.put_copied(number, generation, rewrite(dict.data(), &set)?);
    }
    Ok(())
}

/// The file key for an encrypted file, from its /Encrypt dictionary (inline or referenced),
/// the first part of its /ID, and the password it was opened with.
fn encryption(pdf: &Pdf, raw: &[u8], id: Option<&[u8]>, password: &str) -> Result<crypt::Crypt, SaveError> {
    let first_id = id
        .and_then(|id| {
            let inner = id.trim_ascii().strip_prefix(b"[")?.strip_suffix(b"]")?;
            let items = lex::array_items(inner)?;
            crypt::parse_string(inner.get(items.first()?.clone())?)
        })
        .unwrap_or_default();
    let raw = raw.trim_ascii();
    if raw.starts_with(b"<<") {
        return crypt::Crypt::new(raw, &first_id, password);
    }
    let r = lex::reference(raw).ok_or(SaveError::Damaged)?;
    let dict = pdf
        .xref()
        .get::<Dict<'_>>(ObjectIdentifier::new(
            i32::try_from(r.number).map_err(|_| SaveError::Damaged)?,
            i32::from(r.generation),
        ))
        .ok_or(SaveError::Damaged)?;
    crypt::Crypt::new(dict.data(), &first_id, password)
}

/// A dictionary with `set` replacing (or adding) those keys, everything else copied byte for
/// byte.
fn rewrite(data: &[u8], set: &field::Entries) -> Result<Vec<u8>, SaveError> {
    let entries = lex::dict_entries(data).ok_or(SaveError::Damaged)?;
    let mut dict = b"<<".to_vec();
    for (key, value) in &entries {
        if set.iter().any(|(k, _)| k.as_bytes() == *key) {
            continue;
        }
        dict.push(b'/');
        dict.extend_from_slice(key);
        dict.push(b' ');
        dict.extend_from_slice(data.get(value.clone()).ok_or(SaveError::Damaged)?);
        dict.push(b'\n');
    }
    for (key, value) in set {
        dict.extend_from_slice(format!("/{key} {value}\n").as_bytes());
    }
    dict.extend_from_slice(b">>");
    Ok(dict)
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
