//! Saving page labels (see [`crate::LabelRange`]) into the catalog, as an incremental update.

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::Dict;

use super::{SaveError, annot, lex, update};
use crate::LabelRange;

pub(crate) fn with_page_labels(
    pdf: &Pdf,
    original: &[u8],
    password: &str,
    ranges: &[LabelRange],
) -> Result<Vec<u8>, SaveError> {
    let trailer = update::read_trailer(original)?;
    let crypt = match &trailer.encrypt {
        Some(raw) => Some(super::encryption(pdf, raw, trailer.id.as_deref(), password)?),
        None => None,
    };
    let mut out = update::Update::new(trailer);
    if let Some(crypt) = crypt {
        out.encrypt_with(crypt);
    }
    let xref = pdf.xref();
    let root = xref.root_id();
    let catalog = xref.get::<Dict<'_>>(root).ok_or(SaveError::Damaged)?;
    let (num, generation) = (
        u32::try_from(root.obj_number).map_err(|_| SaveError::Damaged)?,
        u16::try_from(root.gen_number).map_err(|_| SaveError::Damaged)?,
    );
    let data = catalog.data();
    let mut dict = b"<<".to_vec();
    for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
        if key == b"PageLabels" {
            continue;
        }
        dict.push(b'/');
        dict.extend_from_slice(key);
        dict.push(b' ');
        dict.extend_from_slice(data.get(value).ok_or(SaveError::Damaged)?);
        dict.push(b'\n');
    }
    // Pages that are just numbered need no labels.
    let plain = ranges.is_empty()
        || (ranges.len() == 1
            && ranges.first().is_some_and(|r| {
                r.start == 0 && r.style == crate::LabelStyle::Decimal && r.prefix.is_empty() && r.first == 1
            }));
    if !plain {
        let entry = crate::labels::entry(ranges, |prefix| {
            out.seal_value(num, generation, annot::text_string(prefix))
        })?;
        dict.extend_from_slice(format!("/PageLabels {entry}\n").as_bytes());
    }
    dict.extend_from_slice(b">>");
    out.put_copied(num, generation, dict);
    Ok(out.finish(original))
}
