use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use crate::{
    Changes, Comment, Document, DocumentInfo, Limits, OpenError, PageText, RenderError, RenderedPage,
    SaveError,
};

type OpenReply = Box<dyn FnOnce(Result<DocumentInfo, OpenError>) + Send>;
type RenderReply = Box<dyn FnOnce(Result<RenderedPage, RenderError>) + Send>;
type TextReply = Box<dyn FnOnce(Result<PageText, RenderError>) + Send>;
type EditReply = Box<dyn FnOnce(Result<(), SaveError>) + Send>;
type SaveReply = Box<dyn FnOnce(Result<Vec<Comment>, SaveError>) + Send>;

/// Work kinds, in the order a batch runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// Edits go first so that later renders show them.
    Edit,
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
        Ok(Self { jobs })
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
            }
        };
        drop(cache);
        match swap {
            Swap::Promote => {
                if let Some(saved) = live.take() {
                    base = saved;
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
