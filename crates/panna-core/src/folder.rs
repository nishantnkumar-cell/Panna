//! Searching every PDF in a folder, as Acrobat's Advanced Search: each file is read in the
//! locked-down process, like an open one, and its pages' text searched. The text can be kept
//! in an [`Index`] so a later search of an unchanged file needn't read it again, as Acrobat
//! Catalog's indexes do.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::{FindOptions, Isolated, Limits, OpenError, PageText};

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
    search_indexed(file, query, options, limits, None).map(|(found, _)| found)
}

/// As [`search`], reading the file's text from `index` when it holds it for the file as it
/// is now, and keeping it there when not. Also says whether the index answered.
pub fn search_indexed(
    file: &Path,
    query: &str,
    options: FindOptions,
    limits: Limits,
    index: Option<&Index>,
) -> Result<(Vec<Found>, bool), String> {
    if let Some(pages) = index.and_then(|i| i.load(file)) {
        let pages: Vec<PageText> = pages.iter().map(|p| PageText::plain(p)).collect();
        return Ok((matches(&pages, query, options), true));
    }
    let pages = read_text(file, limits)?;
    if let Some(index) = index {
        // A failed save only means the next search reads the file again.
        let _ = index.store(file, &pages.iter().map(PageText::text).collect::<Vec<_>>());
    }
    Ok((matches(&pages, query, options), false))
}

/// Every page's text, read in the locked-down process.
fn read_text(file: &Path, limits: Limits) -> Result<Vec<PageText>, String> {
    let size = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
    if size > limits.max_file_bytes {
        return Err("It's bigger than Panna opens.".into());
    }
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let (mut doc, info) = Isolated::open(bytes, None, limits).map_err(|e| match e {
        OpenError::PasswordRequired => "It needs a password, so it wasn't searched.".to_owned(),
        e => e.to_string(),
    })?;
    Ok((0..info.pages.len())
        .map(|page| doc.page_text(page).unwrap_or_default())
        .collect())
}

fn matches(pages: &[PageText], query: &str, options: FindOptions) -> Vec<Found> {
    let mut found = Vec::new();
    for (page, text) in pages.iter().enumerate() {
        for range in text.find(query, options) {
            found.push(Found {
                page,
                before: flat(text.slice(range.start.saturating_sub(CONTEXT)..range.start)),
                matched: flat(text.slice(range.clone())),
                after: flat(text.slice(range.end..range.end + CONTEXT)),
            });
            if found.len() >= MAX_PER_FILE {
                return found;
            }
        }
    }
    found
}

/// Text kept for each file: one file per PDF, named from a hash of its path, holding the
/// path, size and modified time it was read at, and each page's text. An entry whose file
/// has since changed, moved or gone is ignored and replaced.
#[derive(Debug, Clone)]
pub struct Index {
    dir: PathBuf,
}

const MAGIC: &str = "panna-search-index 1";
/// Text kept for one file, at most; bigger files are searched each time.
const MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;

impl Index {
    /// An index kept in `dir`, made when first written.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The file's identity now: its full path, size and modified time.
    fn stamp(file: &Path) -> Option<(String, u64, u128)> {
        let path = std::fs::canonicalize(file).ok()?;
        let meta = std::fs::metadata(&path).ok()?;
        let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
        // A path that isn't text or has a line break can't be kept in the entry.
        let path = path.to_str().filter(|p| !p.contains(['\n', '\r']))?.to_owned();
        Some((path, meta.len(), modified))
    }

    fn entry(&self, path: &str) -> PathBuf {
        // FNV-1a: stable across Rust versions, unlike the standard library's hasher.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for b in path.bytes() {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.dir.join(format!("{hash:016x}.txt"))
    }

    /// The pages' text kept for `file`, if it was kept for the file as it is now.
    pub fn load(&self, file: &Path) -> Option<Vec<String>> {
        let (path, size, modified) = Self::stamp(file)?;
        let entry = self.entry(&path);
        if std::fs::metadata(&entry).ok()?.len() > MAX_ENTRY_BYTES as u64 {
            return None;
        }
        let data = std::fs::read_to_string(entry).ok()?;
        let mut rest = data.as_str();
        if next_line(&mut rest)? != MAGIC || next_line(&mut rest)? != path {
            return None;
        }
        if next_line(&mut rest)?.parse::<u64>().ok()? != size
            || next_line(&mut rest)?.parse::<u128>().ok()? != modified
        {
            return None;
        }
        let count: usize = next_line(&mut rest)?.parse().ok()?;
        let mut pages = Vec::with_capacity(count.min(100_000));
        for _ in 0..count {
            // Each page: its length in bytes on a line, then the text and a line break.
            let len: usize = next_line(&mut rest)?.parse().ok()?;
            let text = rest.get(..len)?;
            rest = rest.get(len..)?.strip_prefix('\n')?;
            pages.push(text.to_owned());
        }
        rest.is_empty().then_some(pages)
    }

    /// Keeps the pages' text for `file`.
    pub fn store(&self, file: &Path, pages: &[String]) -> std::io::Result<()> {
        let Some((path, size, modified)) = Self::stamp(file) else {
            return Ok(());
        };
        let mut out = format!("{MAGIC}\n{path}\n{size}\n{modified}\n{}\n", pages.len());
        for page in pages {
            out.push_str(&format!("{}\n{page}\n", page.len()));
            if out.len() > MAX_ENTRY_BYTES {
                return Ok(());
            }
        }
        // The text of the user's documents: only they may read it.
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&self.dir)?;
        let entry = self.entry(&path);
        // Write then rename, so a search running alongside never reads half an entry.
        let tmp = entry.with_extension(format!("tmp{}", std::process::id()));
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(out.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &entry)
    }

    /// How many files' text it holds, and their size in bytes.
    pub fn size(&self) -> (usize, u64) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return (0, 0);
        };
        entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
            .fold((0, 0), |(n, bytes), e| {
                (n + 1, bytes + e.metadata().map(|m| m.len()).unwrap_or(0))
            })
    }

    /// Removes everything it holds.
    pub fn clear(&self) -> std::io::Result<()> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ours = path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x == "txt" || x.starts_with("tmp"));
            if ours && entry.file_type().is_ok_and(|t| t.is_file()) {
                std::fs::remove_file(path)?;
            }
        }
        Ok(())
    }
}

/// The text up to the next line break, moving `rest` past it.
fn next_line<'a>(rest: &mut &'a str) -> Option<&'a str> {
    let (head, tail) = rest.split_once('\n')?;
    *rest = tail;
    Some(head)
}
