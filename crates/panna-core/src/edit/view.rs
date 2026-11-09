//! Saving how a document opens (see [`InitialView`]) into its catalog, as an incremental
//! update.

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::Dict;

use super::{SaveError, lex, update};
use crate::InitialView;

/// Catalog entries this replaces.
const REPLACED: &[&[u8]] = &[b"OpenAction", b"PageMode", b"PageLayout", b"ViewerPreferences"];

pub(crate) fn with_initial_view(
    pdf: &Pdf,
    original: &[u8],
    view: &InitialView,
) -> Result<Vec<u8>, SaveError> {
    let xref = pdf.xref();
    let root = xref.root_id();
    let catalog = xref.get::<Dict<'_>>(root).ok_or(SaveError::Damaged)?;
    let (num, generation) = (
        u32::try_from(root.obj_number).map_err(|_| SaveError::Damaged)?,
        u16::try_from(root.gen_number).map_err(|_| SaveError::Damaged)?,
    );
    let pages: Vec<String> = pdf
        .pages()
        .iter()
        .filter_map(|page| page.raw().obj_id())
        .map(|id| format!("{} {} R", id.obj_number, id.gen_number))
        .collect();

    let data = catalog.data();
    let mut dict = b"<<".to_vec();
    for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
        if REPLACED.contains(&key) {
            continue;
        }
        dict.push(b'/');
        dict.extend_from_slice(key);
        dict.push(b' ');
        dict.extend_from_slice(data.get(value).ok_or(SaveError::Damaged)?);
        dict.push(b'\n');
    }
    for (key, value) in view.entries(&pages) {
        dict.extend_from_slice(format!("/{key} {value}\n").as_bytes());
    }
    // The viewer preferences keep everything but whether the title is shown.
    let mut prefs = Vec::new();
    if let Some(old) = catalog.get::<Dict<'_>>(b"ViewerPreferences") {
        let data = old.data();
        for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
            if key == b"DisplayDocTitle" {
                continue;
            }
            prefs.push(b'/');
            prefs.extend_from_slice(key);
            prefs.push(b' ');
            prefs.extend_from_slice(data.get(value).ok_or(SaveError::Damaged)?);
            prefs.push(b' ');
        }
    }
    if view.show_title {
        prefs.extend_from_slice(b"/DisplayDocTitle true");
    }
    if !prefs.is_empty() {
        dict.extend_from_slice(b"/ViewerPreferences << ");
        dict.extend_from_slice(&prefs);
        dict.extend_from_slice(b" >>\n");
    }
    dict.extend_from_slice(b">>");
    let mut out = update::Update::new(update::read_trailer(original)?);
    out.put_copied(num, generation, dict);
    Ok(out.finish(original))
}
