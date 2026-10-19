//! Batch actions: the same steps run over every PDF in a folder, each result saved to another
//! folder under the same name. Each file is read in the locked-down process, like an open one.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{Cleanup, HeaderFooter, Isolated, Limits, PageEdit, Watermark};

/// Most files taken from one folder.
pub const MAX_FILES: usize = 10_000;

/// One step, run on each file in turn.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Recognises the text on pages that have none of their own (scans), so it can be
    /// searched and copied.
    Recognise,
    /// Draws this text across every page, faintly.
    Watermark(String),
    /// "Page 1 of 9" at the foot of every page.
    PageNumbers,
    /// Draws comments and form fields into the pages, so they can't be changed.
    Flatten,
    /// Removes the document information, scripts, attachments and comments.
    Clean,
    /// Makes the file smaller, with pictures sharper than 150 dots per inch scaled down.
    Compact,
}

impl Step {
    /// What it does, for messages.
    pub fn label(&self) -> &'static str {
        match self {
            Step::Recognise => "making text searchable",
            Step::Watermark(_) => "adding the watermark",
            Step::PageNumbers => "adding page numbers",
            Step::Flatten => "flattening",
            Step::Clean => "removing hidden information",
            Step::Compact => "making it smaller",
        }
    }
}

/// The PDFs directly in `folder` (not in folders inside it), by name.
pub fn pdfs_in(folder: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        let is_pdf = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        if is_pdf && entry.file_type().is_ok_and(|t| t.is_file()) {
            files.push(path);
            if files.len() >= MAX_FILES {
                break;
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Runs `steps` on the PDF at `file`, saving the result under the same name in `out`. An
/// existing file there is left alone, and an error says so.
pub fn run(file: &Path, steps: &[Step], out: &Path, limits: Limits) -> Result<PathBuf, String> {
    let name = file.file_name().ok_or("This isn't a file.")?;
    let target = out.join(name);
    if same_file(file, &target) {
        return Err("The results folder can't be the folder the files come from.".into());
    }
    if target.exists() {
        return Err("A file with this name is already in the results folder.".into());
    }
    let size = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
    if size > limits.max_file_bytes {
        return Err("It's bigger than Panna opens.".into());
    }
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let (mut doc, info) = Isolated::open(bytes, None, limits).map_err(|e| match e {
        crate::OpenError::PasswordRequired => "It needs a password, so it was left out.".to_owned(),
        e => e.to_string(),
    })?;
    let mut count = info.pages.len();
    for step in steps {
        let all = || (0..count).collect::<Vec<usize>>();
        let done = match step {
            Step::Compact => doc.compact(Some(150)).map(|()| count),
            step => {
                let edit = match step {
                    Step::Recognise => PageEdit::Recognise(all()),
                    Step::Watermark(text) => PageEdit::Watermark {
                        pages: all(),
                        mark: Watermark {
                            text: text.clone(),
                            ..Watermark::default()
                        },
                    },
                    Step::PageNumbers => {
                        let mut marks = HeaderFooter::default();
                        if let Some(centre) = marks.text.get_mut(4) {
                            *centre = "Page {page} of {pages}".into();
                        }
                        PageEdit::HeaderFooter { pages: all(), marks }
                    }
                    Step::Flatten => PageEdit::Flatten {
                        comments: true,
                        fields: true,
                    },
                    Step::Clean | Step::Compact => PageEdit::Clean(Cleanup {
                        metadata: true,
                        scripts: true,
                        attachments: true,
                        comments: true,
                        archive: false,
                    }),
                };
                doc.edit(edit).map(|info| info.pages.len())
            }
        };
        count = done.map_err(|e| format!("Stopped while {}: {e}", step.label()))?;
    }
    let mut written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|e| e.to_string())?;
    if let Err(e) = written.write_all(doc.bytes()).and_then(|()| written.sync_all()) {
        drop(written);
        let _ = std::fs::remove_file(&target);
        return Err(e.to_string());
    }
    Ok(target)
}

/// Whether `a` and `b` name the same place, when both can be found.
fn same_file(a: &Path, b: &Path) -> bool {
    let parent = |p: &Path| p.parent().and_then(|d| std::fs::canonicalize(d).ok());
    parent(a).is_some_and(|d| Some(d) == parent(b)) && a.file_name() == b.file_name()
}
