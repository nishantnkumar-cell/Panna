use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use crate::{Document, DocumentInfo, Limits, OpenError, PageText, RenderError, RenderedPage};

type OpenReply = Box<dyn FnOnce(Result<DocumentInfo, OpenError>) + Send>;
type RenderReply = Box<dyn FnOnce(Result<RenderedPage, RenderError>) + Send>;
type TextReply = Box<dyn FnOnce(Result<PageText, RenderError>) + Send>;

/// Work kinds, in the order a batch runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
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

fn run(path: PathBuf, password: Option<String>, limits: Limits, on_open: OpenReply, queue: Receiver<Job>) {
    let document = match Document::open(&path, password.as_deref(), limits) {
        Ok(document) => document,
        Err(e) => return on_open(Err(e)),
    };
    match document.info() {
        Ok(info) => on_open(Ok(info)),
        Err(e) => return on_open(Err(e)),
    }

    let cache = document.new_cache();
    let mut pending: Vec<Job> = Vec::new();
    loop {
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
        // Stable sort: pages, then thumbnails, then text; otherwise in request order. Only one
        // job runs before the queue is checked again, so new page renders overtake text work.
        pending.sort_by_key(|job| job.kind);
        let job = pending.remove(0);
        match job.reply {
            Reply::Render {
                scale,
                rotation,
                reply,
            } => {
                let page = document.render_page(&cache, job.index, scale);
                reply(page.map(|p| p.rotated(rotation)));
            }
            Reply::Text(reply) => reply(document.page_text(job.index)),
        }
    }
}
