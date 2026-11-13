//! Searching every PDF in a folder, as Acrobat's Advanced Search: each file is read in the
//! locked-down process, like an open one, and its pages' text searched.

use std::path::{Path, PathBuf};

use crate::{FindOptions, Isolated, Limits, OpenError};

/// Most files searched in one go.
pub const MAX_FILES: usize = 10_000;
/// Most matches kept from one file.
pub const MAX_PER_FILE: usize = 200;
/// Folders gone into below the one chosen, at most.
const MAX_DEPTH: usize = 16;
/// Characters of text kept on each side of a match.
const CONTEXT: usize = 40;

/// The PDFs in `folder`, and in the folders inside it when `subfolders`, by path. Links to
/// other places aren't followed.
pub fn pdfs_under(folder: &Path, subfolders: bool) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut to_read = vec![(folder.to_path_buf(), 0usize)];
    let mut first = true;
    while let Some((dir, depth)) = to_read.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            // The folder chosen must be readable; one inside it that isn't is skipped.
            Err(e) if first => return Err(e),
            Err(_) => continue,
        };
        first = false;
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                if subfolders && depth < MAX_DEPTH {
                    to_read.push((path, depth + 1));
                }
            } else if kind.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
                files.push(path);
                if files.len() >= MAX_FILES {
                    files.sort();
                    return Ok(files);
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

/// A match: its page, and the text before it, of it and after it, on one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub page: usize,
    pub before: String,
    pub matched: String,
    pub after: String,
}

fn flat(s: String) -> String {
    s.chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect()
}

/// Every match of `query` in the PDF at `file`, by page, at most [`MAX_PER_FILE`].
pub fn search(file: &Path, query: &str, options: FindOptions, limits: Limits) -> Result<Vec<Found>, String> {
    let size = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
    if size > limits.max_file_bytes {
        return Err("It's bigger than Panna opens.".into());
    }
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let (mut doc, info) = Isolated::open(bytes, None, limits).map_err(|e| match e {
        OpenError::PasswordRequired => "It needs a password, so it wasn't searched.".to_owned(),
        e => e.to_string(),
    })?;
    let mut found = Vec::new();
    for page in 0..info.pages.len() {
        let Ok(text) = doc.page_text(page) else { continue };
        for range in text.find(query, options) {
            found.push(Found {
                page,
                before: flat(text.slice(range.start.saturating_sub(CONTEXT)..range.start)),
                matched: flat(text.slice(range.clone())),
                after: flat(text.slice(range.end..range.end + CONTEXT)),
            });
            if found.len() >= MAX_PER_FILE {
                return Ok(found);
            }
        }
    }
    Ok(found)
}
