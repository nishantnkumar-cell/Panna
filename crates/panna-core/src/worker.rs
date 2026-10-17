use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use crate::{Document, DocumentInfo, Limits, OpenError, RenderError, RenderedPage};

type OpenReply = Box<dyn FnOnce(Result<DocumentInfo, OpenError>) + Send>;
type RenderReply = Box<dyn FnOnce(Result<RenderedPage, RenderError>) + Send>;

struct Job {
    index: usize,
    scale: f32,
    reply: RenderReply,
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

    /// Queues a page render. If the worker has gone away, `reply` is never called; the
    /// caller sees its channel close instead.
    pub fn render(
        &self,
        index: usize,
        scale: f32,
        reply: impl FnOnce(Result<RenderedPage, RenderError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index,
            scale,
            reply: Box::new(reply),
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
    while let Ok(first) = queue.recv() {
        // Collapse a burst of requests (fast scrolling, repeated zooms) so that only the
        // newest request per page is rendered. Superseded replies are dropped unanswered.
        let mut batch = vec![first];
        loop {
            match queue.try_recv() {
                Ok(job) => {
                    batch.retain(|queued| queued.index != job.index);
                    batch.push(job);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        for job in batch {
            (job.reply)(document.render_page(&cache, job.index, job.scale));
        }
    }
}
