use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    Changes, Comment, Document, DocumentInfo, Limits, OpenError, PageSource, PageText, RenderError,
    RenderedPage, SaveError,
};

type OpenReply = Box<dyn FnOnce(Result<DocumentInfo, OpenError>) + Send>;
type RenderReply = Box<dyn FnOnce(Result<RenderedPage, RenderError>) + Send>;
type TextReply = Box<dyn FnOnce(Result<PageText, RenderError>) + Send>;
type EditReply = Box<dyn FnOnce(Result<(), SaveError>) + Send>;
type SaveReply = Box<dyn FnOnce(Result<Vec<Comment>, SaveError>) + Send>;
type PagesReply = Box<dyn FnOnce(Result<DocumentInfo, SaveError>) + Send>;
type ExtractReply = Box<dyn FnOnce(Result<(), SaveError>) + Send>;

/// Earlier versions kept for undoing page changes.
const MAX_PAGE_UNDO: usize = 20;

/// A change to the document's pages. Unsaved changes are made part of the document first.
#[derive(Debug, Clone)]
pub enum PageEdit {
    /// The new page list (see [`Document::rearranged`]).
    Plan(Vec<PageSource>),
    /// Every page of another file, inserted before page `at`.
    InsertFile { at: usize, path: PathBuf },
    /// Goes back to before the last page change.
    Undo,
}

/// Work kinds, in the order a batch runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// Edits go first so that later renders show them.
    Edit,
    /// Page changes, each kept: a later one builds on the ones before.
    Pages,
    Save,
    Page,
    Thumbnail,
    Text,
}

enum Reply {
    Render {
        scale: f32,
        rotation: u8,
        reply: RenderReply,
    },
    Text(TextReply),
    Edit(Changes, EditReply),
    Save(PathBuf, SaveReply),
    Pages(PageEdit, PagesReply),
    Extract(Vec<PageSource>, PathBuf, ExtractReply),
}

struct Job {
    index: usize,
    kind: Kind,
    reply: Reply,
}

/// Owns one document on a dedicated thread so parsing and rendering never block the UI.
///
/// Dropping the worker closes its queue; the thread finishes its current page and exits.
pub struct RenderWorker {
    jobs: Sender<Job>,
    /// Numbers page jobs so that none replaces another in the queue.
    serial: AtomicUsize,
}

impl RenderWorker {
    /// Starts a worker that opens `path` and reports the outcome through `on_open`.
    pub fn open(
        path: PathBuf,
        password: Option<String>,
        limits: Limits,
        on_open: impl FnOnce(Result<DocumentInfo, OpenError>) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (jobs, queue) = mpsc::channel();
        let on_open: OpenReply = Box::new(on_open);
        thread::Builder::new()
            .name("pdf-render".into())
            .spawn(move || run(path, password, limits, on_open, queue))?;
        Ok(Self {
            jobs,
            serial: AtomicUsize::new(0),
        })
    }

    /// Queues a page render, turned clockwise by `rotation` quarter turns. If the worker has
    /// gone away, `reply` is never called; the caller sees its channel close instead.
    pub fn render(
        &self,
        index: usize,
        scale: f32,
        rotation: u8,
        reply: impl FnOnce(Result<RenderedPage, RenderError>) + Send + 'static,
    ) {
        self.send(index, Kind::Page, scale, rotation, Box::new(reply));
    }

    /// Like [`render`](Self::render), for a sidebar thumbnail. Pending thumbnails never replace
    /// pending page renders, and page renders go first.
    pub fn render_thumbnail(
        &self,
        index: usize,
        scale: f32,
        rotation: u8,
        reply: impl FnOnce(Result<RenderedPage, RenderError>) + Send + 'static,
    ) {
        self.send(index, Kind::Thumbnail, scale, rotation, Box::new(reply));
    }

    /// Queues text extraction for one page. It runs after any queued renders.
    pub fn extract_text(
        &self,
        index: usize,
        reply: impl FnOnce(Result<PageText, RenderError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index,
            kind: Kind::Text,
            reply: Reply::Text(Box::new(reply)),
        });
    }

    /// Replaces the unsaved changes. Pages render with them from then on; until saved, the
    /// file on disk is untouched. Only the newest pending set is applied.
    pub fn set_changes(&self, changes: Changes, reply: impl FnOnce(Result<(), SaveError>) + Send + 'static) {
        let _ = self.jobs.send(Job {
            index: 0,
            kind: Kind::Edit,
            reply: Reply::Edit(changes, Box::new(reply)),
        });
    }

    /// Writes the document, with its changes, to `path`, and replies with the comments the
    /// saved file now has. On success the changes become part of the document, so the next
    /// [`set_changes`](Self::set_changes) should start empty.
    pub fn save(&self, path: PathBuf, reply: impl FnOnce(Result<Vec<Comment>, SaveError>) + Send + 'static) {
        let _ = self.jobs.send(Job {
            index: 0,
            kind: Kind::Save,
            reply: Reply::Save(path, Box::new(reply)),
        });
    }

    /// Changes the pages, and replies with the document's new layout and contents. The
    /// change is in memory until saved.
    pub fn edit_pages(
        &self,
        edit: PageEdit,
        reply: impl FnOnce(Result<DocumentInfo, SaveError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Pages(edit, Box::new(reply)),
        });
    }

    /// Writes some pages (with unsaved changes) to a new file at `path`, leaving this document
    /// as it is.
    pub fn extract(
        &self,
        pages: Vec<PageSource>,
        path: PathBuf,
        reply: impl FnOnce(Result<(), SaveError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Extract(pages, path, Box::new(reply)),
        });
    }

    fn send(&self, index: usize, kind: Kind, scale: f32, rotation: u8, reply: RenderReply) {
        let _ = self.jobs.send(Job {
            index,
            kind,
            reply: Reply::Render {
                scale,
                rotation,
                reply,
            },
        });
    }
}

enum Swap {
    Edit(Changes, EditReply),
    Pages(PageEdit, PagesReply),
    /// The live document was saved and becomes the base.
    Promote,
}

/// Writes to a temporary file next to `path`, then renames it over `path`, so a failed write
/// never leaves a half-written PDF behind.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), SaveError> {
    let io = |e: std::io::Error| SaveError::Io(e.to_string());
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| SaveError::Io("No file name".into()))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(".panna-saving");
    let tmp = dir.join(tmp_name);
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(io)
}

fn run(path: PathBuf, password: Option<String>, limits: Limits, on_open: OpenReply, queue: Receiver<Job>) {
    let document = match Document::open(&path, password.as_deref(), limits) {
        Ok(document) => document,
        Err(e) => return on_open(Err(e)),
    };
    match document.info() {
        Ok(info) => on_open(Ok(info)),
        Err(e) => return on_open(Err(e)),
    }

    // `base` is the document as last saved; `live` adds the unsaved annotations to it.
    let mut base = document;
    let mut live: Option<Document> = None;
    let mut pending: Vec<Job> = Vec::new();
    let mut history: Vec<Document> = Vec::new();
    loop {
        let current = live.as_ref().unwrap_or(&base);
        let cache = current.new_cache();
        // Render from `current` until an edit or save needs to swap documents.
        let swap = loop {
            if pending.is_empty() {
                match queue.recv() {
                    Ok(job) => pending.push(job),
                    Err(_) => return,
                }
            }
            // Collapse a burst of requests (fast scrolling, repeated zooms) so that only the
            // newest request per page and kind is done. Superseded replies are dropped unanswered.
            loop {
                match queue.try_recv() {
                    Ok(job) => {
                        pending.retain(|queued| queued.index != job.index || queued.kind != job.kind);
                        pending.push(job);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            // Stable sort: edits, saves, pages, thumbnails, then text; otherwise in request
            // order. Only one job runs before the queue is checked again, so new page renders
            // overtake text work.
            pending.sort_by_key(|job| job.kind);
            let job = pending.remove(0);
            match job.reply {
                Reply::Render {
                    scale,
                    rotation,
                    reply,
                } => {
                    let page = current.render_page(&cache, job.index, scale);
                    reply(page.map(|p| p.rotated(rotation)));
                }
                Reply::Text(reply) => reply(current.page_text(job.index)),
                Reply::Save(path, reply) => {
                    let result = write_atomically(&path, current.bytes()).map(|()| current.comments());
                    let saved = result.is_ok();
                    reply(result);
                    if saved && live.is_some() {
                        break Swap::Promote;
                    }
                }
                Reply::Edit(list, reply) => break Swap::Edit(list, reply),
                Reply::Pages(edit, reply) => break Swap::Pages(edit, reply),
                Reply::Extract(pages, path, reply) => {
                    reply(
                        current
                            .rearranged(&pages, &[])
                            .and_then(|bytes| write_atomically(&path, &bytes)),
                    );
                }
            }
        };
        drop(cache);
        match swap {
            Swap::Promote => {
                if let Some(saved) = live.take() {
                    base = saved;
                }
            }
            Swap::Pages(edit, reply) => {
                let undoing = matches!(edit, PageEdit::Undo);
                let current = live.as_ref().unwrap_or(&base);
                let result = match edit {
                    PageEdit::Undo => match history.pop() {
                        Some(earlier) => Ok(earlier),
                        None => Err(SaveError::Damaged),
                    },
                    PageEdit::Plan(plan) => current
                        .rearranged(&plan, &[])
                        .and_then(|bytes| current.reopen(bytes).map_err(|_| SaveError::Damaged)),
                    PageEdit::InsertFile { at, path } => insert_file(current, at, &path),
                };
                match result.and_then(|doc| Ok((doc.info().map_err(|_| SaveError::Damaged)?, doc))) {
                    Ok((info, doc)) => {
                        // Unsaved changes are now part of the document; undo goes back to it
                        // as it was, those changes included.
                        let before = std::mem::replace(&mut base, doc);
                        let before = live.take().unwrap_or(before);
                        if !undoing {
                            history.push(before);
                            if history.len() > MAX_PAGE_UNDO {
                                history.remove(0);
                            }
                        }
                        reply(Ok(info));
                    }
                    Err(e) => reply(Err(e)),
                }
            }
            Swap::Edit(list, reply) => {
                if list.is_empty() {
                    live = None;
                    reply(Ok(()));
                    continue;
                }
                // The password goes along: an encrypted file stays encrypted with it.
                let result = base
                    .with_changes(&list)
                    .and_then(|bytes| base.reopen(bytes).map_err(|_| SaveError::Damaged));
                match result {
                    Ok(doc) => {
                        live = Some(doc);
                        reply(Ok(()));
                    }
                    Err(e) => reply(Err(e)),
                }
            }
        }
    }
}

/// `current` with every page of the file at `path` inserted before page `at`.
fn insert_file(current: &Document, at: usize, path: &Path) -> Result<Document, SaveError> {
    let other = Document::open(path, None, current.limits()).map_err(|e| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        SaveError::Io(format!("couldn't open “{name}”: {e}"))
    })?;
    let own = current.page_count();
    let at = at.min(own);
    let page = |doc, index| PageSource::Page { doc, index, turns: 0 };
    let plan: Vec<PageSource> = (0..at)
        .map(|i| page(0, i))
        .chain((0..other.page_count()).map(|i| page(1, i)))
        .chain((at..own).map(|i| page(0, i)))
        .collect();
    let bytes = current.rearranged(&plan, &[&other])?;
    current.reopen(bytes).map_err(|_| SaveError::Damaged)
}
