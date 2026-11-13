//! Combining files into one PDF, as Acrobat's Combine Files: PDFs and pictures, in the order
//! given, each file's pages after the last, with a bookmark for each file and that file's own
//! bookmarks under it. Each PDF is read in the locked-down process, like an open one.

use std::path::{Path, PathBuf};

use crate::{Isolated, Limits, OpenError, OutlineItem, PageEdit};

/// Most files combined in one go.
pub const MAX_FILES: usize = 500;

/// Picture files taken, by extension.
pub const PICTURES: [&str; 5] = ["png", "jpg", "jpeg", "tif", "tiff"];

/// Whether `path` names a picture rather than a PDF, by its extension.
pub fn is_picture(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PICTURES.iter().any(|p| p.eq_ignore_ascii_case(e)))
}

/// The file's name without its extension, for its bookmark.
fn stem(path: &Path) -> String {
    path.file_stem()
        .map_or_else(|| "File".to_owned(), |s| s.to_string_lossy().into_owned())
}

fn name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// A file as a PDF: read, and a picture made into a page.
fn as_pdf(path: &Path, limits: Limits) -> Result<Vec<u8>, String> {
    let name = name(path);
    let size = std::fs::metadata(path)
        .map_err(|e| format!("Couldn't read “{name}”: {e}"))?
        .len();
    if size > limits.max_file_bytes {
        return Err(format!("“{name}” is bigger than Panna opens."));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't read “{name}”: {e}"))?;
    if is_picture(path) {
        crate::pictures_to_pdf(vec![bytes]).map_err(|e| format!("“{name}”: {e}"))
    } else {
        Ok(bytes)
    }
}

/// Combines `files`, in order, into one PDF and gives its bytes. `progress` is told each
/// file's index as it's started.
pub fn combine(
    files: &[PathBuf],
    limits: Limits,
    mut progress: impl FnMut(usize),
) -> Result<Vec<u8>, String> {
    if files.len() < 2 {
        return Err("Choose at least two files to combine.".into());
    }
    if files.len() > MAX_FILES {
        return Err(format!("Combine at most {MAX_FILES} files at a time."));
    }
    let mut combined: Option<Isolated> = None;
    let mut outline: Vec<OutlineItem> = Vec::new();
    let mut count = 0usize;
    for (index, path) in files.iter().enumerate() {
        progress(index);
        let name = name(path);
        let bytes = as_pdf(path, limits)?;
        // Read on its own first: it's checked, and its bookmarks and pages are known.
        let (one, info) = Isolated::open(bytes, None, limits).map_err(|e| match e {
            OpenError::PasswordRequired => {
                format!("“{name}” needs a password. Open it, remove the password, and combine the copy.")
            }
            e => format!("Couldn't open “{name}”: {e}"),
        })?;
        let pages = info.pages.len();
        if pages == 0 {
            continue;
        }
        outline.push(OutlineItem {
            title: stem(path),
            depth: 0,
            page: Some(count),
            ..OutlineItem::default()
        });
        outline.extend(info.outline.into_iter().map(|item| OutlineItem {
            depth: item.depth.saturating_add(1),
            page: item.page.map(|p| p + count),
            id: None,
            ..item
        }));
        match &mut combined {
            None => combined = Some(one),
            Some(doc) => {
                doc.insert_file(count, one.bytes().to_vec(), &name)
                    .map_err(|e| format!("Couldn't add “{name}”: {e}"))?;
            }
        }
        count += pages;
    }
    let mut doc = combined.ok_or("None of the files has any pages.")?;
    doc.edit(PageEdit::Bookmarks(outline))
        .map_err(|e| format!("Couldn't add the bookmarks: {e}"))?;
    Ok(doc.bytes().to_vec())
}
