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
type CommentsReply = Box<dyn FnOnce(Result<(Vec<crate::Annotation>, usize), String>) + Send>;
type CompactReply = Box<dyn FnOnce(Result<(u64, u64), SaveError>) + Send>;
type CompareReply = Box<dyn FnOnce(Result<Vec<crate::Change>, String>) + Send>;
type PrintReply = Box<dyn FnOnce(Result<Vec<u8>, SaveError>) + Send>;

/// Earlier versions kept for undoing page changes.
const MAX_PAGE_UNDO: usize = 20;

/// A change to the document's pages. Unsaved changes are made part of the document first.
#[derive(Debug, Clone)]
pub enum PageEdit {
    /// The new page list (see [`Document::rearranged`]).
    Plan(Vec<PageSource>),
    /// Every page of another file, inserted before page `at`.
    InsertFile { at: usize, path: PathBuf },
    /// Trims margins (top, right, bottom, left as shown, in points) from these pages.
    Crop {
        pages: Vec<usize>,
        margins: crate::Margins,
    },
    /// Draws header and footer text on these pages, numbered in this order.
    HeaderFooter {
        pages: Vec<usize>,
        marks: crate::HeaderFooter,
    },
    /// Draws a watermark or background on these pages.
    Watermark {
        pages: Vec<usize>,
        mark: crate::Watermark,
    },
    /// Adds text, pictures or links to a page.
    Add {
        page: usize,
        additions: Vec<crate::Addition>,
    },
    /// Takes everything inside these areas (as page text boxes, per page) out for good, and
    /// covers them in black.
    Redact(Vec<(usize, Vec<crate::TextBox>)>),
    /// Adds new form fields and takes out these ones.
    FormFields {
        add: Vec<crate::NewField>,
        remove: Vec<crate::ObjRef>,
    },
    /// Removes hidden information.
    Clean(crate::Cleanup),
    /// Gives the document new password protection, or removes it.
    Protect(Option<crate::Protection>),
    /// Draws comments, form fields or both into the pages.
    Flatten { comments: bool, fields: bool },
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
    Extract(Vec<(Vec<PageSource>, PathBuf)>, ExtractReply),
    Export(crate::Export, ExtractReply),
    Attachment(usize, PathBuf, ExtractReply),
    Print(PrintReply),
    /// Writes a smaller copy of the document.
    Compact(PathBuf, CompactReply),
    /// Writes the document's comments to a comments file.
    ExportComments(PathBuf, crate::DataFormat, String, ExtractReply),
    /// Reads a comments file into annotations for this document.
    ReadComments(Vec<u8>, CommentsReply),
    /// Compares the document's text with another file's.
    Compare(PathBuf, CompareReply),
    /// Hide these layers and show the rest, or `None` for the document's own choice.
    Layers(Option<Vec<crate::ObjRef>>),
    /// A page drawn at a scale and turned, then cut to an area.
    Snapshot {
        page: usize,
        scale: f32,
        rotation: u8,
        area: [f32; 4],
        reply: RenderReply,
    },
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

    /// Writes pages (with unsaved changes) to new files, each page list to its path, leaving
    /// this document as it is. Stops at the first file that can't be written.
    pub fn extract(
        &self,
        files: Vec<(Vec<PageSource>, PathBuf)>,
        reply: impl FnOnce(Result<(), SaveError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Extract(files, Box::new(reply)),
        });
    }

    /// Saves attached file `index` (in [`DocumentInfo::attachments`]) to `path`.
    pub fn save_attachment(
        &self,
        index: usize,
        path: PathBuf,
        reply: impl FnOnce(Result<(), SaveError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Attachment(index, path, Box::new(reply)),
        });
    }

    /// Saves pages (with unsaved changes) as text or pictures. Stops at the first file that
    /// can't be written.
    pub fn export(&self, export: crate::Export, reply: impl FnOnce(Result<(), SaveError>) + Send + 'static) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Export(export, Box::new(reply)),
        });
    }

    /// A picture of part of page `index` as shown, with unsaved changes: the page drawn at
    /// `scale`, turned `rotation` quarter turns, then cut to `area` (see
    /// [`RenderedPage::cropped`]). Never replaced by other requests.
    pub fn snapshot(
        &self,
        index: usize,
        scale: f32,
        rotation: u8,
        area: [f32; 4],
        reply: impl FnOnce(Result<RenderedPage, RenderError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Snapshot {
                page: index,
                scale,
                rotation,
                area,
                reply: Box::new(reply),
            },
        });
    }

    /// Shows the document with the layers in `hidden` hidden and the rest shown, or as the
    /// document itself chooses with `None`. Only the view changes; saved files don't.
    pub fn set_layers(&self, hidden: Option<Vec<crate::ObjRef>>) {
        // Its own number, so a newer choice replaces an older one but never other changes.
        let _ = self.jobs.send(Job {
            index: usize::MAX,
            kind: Kind::Edit,
            reply: Reply::Layers(hidden),
        });
    }

    /// Writes a smaller copy of the document (with the changes last given to it) to `path`,
    /// and replies with the sizes before and after, in bytes.
    pub fn save_compacted(
        &self,
        path: PathBuf,
        reply: impl FnOnce(Result<(u64, u64), SaveError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Compact(path, Box::new(reply)),
        });
    }

    /// Writes the document's comments (as last given to it) to a comments file at `path`.
    pub fn export_comments(
        &self,
        path: PathBuf,
        format: crate::DataFormat,
        pdf_name: String,
        reply: impl FnOnce(Result<(), SaveError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::ExportComments(path, format, pdf_name, Box::new(reply)),
        });
    }

    /// Reads a comments file into annotations placed on this document's pages, with how
    /// many comments in it couldn't be placed (see [`Document::comments_from_file`]).
    pub fn read_comments(
        &self,
        bytes: Vec<u8>,
        reply: impl FnOnce(Result<(Vec<crate::Annotation>, usize), String>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::ReadComments(bytes, Box::new(reply)),
        });
    }

    /// Compares the document's text with the file at `path`, word by word.
    pub fn compare(
        &self,
        path: PathBuf,
        reply: impl FnOnce(Result<Vec<crate::Change>, String>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Compare(path, Box::new(reply)),
        });
    }

    /// The whole document with unsaved changes, without password protection, for printing.
    pub fn print_copy(&self, reply: impl FnOnce(Result<Vec<u8>, SaveError>) + Send + 'static) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Print(Box::new(reply)),
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
    Layers(Option<Vec<crate::ObjRef>>),
    Edit(Changes, EditReply),
    Pages(PageEdit, PagesReply),
    /// The live document was saved and becomes the base.
    Promote,
}

/// Writes to a temporary file next to `path`, then renames it over `path`, so a failed write
/// never leaves a half-written PDF behind.
/// Writes the pages `export` asks for.
fn export_pages<'a>(
    doc: &'a Document,
    cache: &hayro::RenderCache<'a>,
    export: &crate::Export,
) -> Result<(), SaveError> {
    use crate::ExportFormat;
    match export.format {
        ExportFormat::Text => {
            let Some((_, path)) = export.pages.first() else {
                return Ok(());
            };
            let pages: Vec<usize> = export.pages.iter().map(|(i, _)| *i).collect();
            write_atomically(path, crate::export::text(doc, &pages)?.as_bytes())
        }
        format => export.pages.iter().try_for_each(|(index, path)| {
            let page = doc
                .render_page(cache, *index, export.dpi / 72.0)
                .map_err(|_| SaveError::NoSuchPage(*index))?;
            write_atomically(path, &crate::export::picture(&page, format)?)
        }),
    }
}

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
    let mut hidden: Option<Vec<crate::ObjRef>> = None;
    loop {
        let current = live.as_ref().unwrap_or(&base);
        // What's shown: the document with the layers the user hid, when they've chosen.
        let with_layers = hidden.as_ref().and_then(|h| current.with_layers_hidden(h).ok());
        let view = with_layers.as_ref().unwrap_or(current);
        let cache = view.new_cache();
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
                    let page = view.render_page(&cache, job.index, scale);
                    reply(page.map(|p| p.rotated(rotation)));
                }
                Reply::Text(reply) => reply(view.page_text(job.index)),
                Reply::Save(path, reply) => {
                    let result = write_atomically(&path, current.bytes()).map(|()| current.comments());
                    let saved = result.is_ok();
                    reply(result);
                    if saved && live.is_some() {
                        break Swap::Promote;
                    }
                }
                Reply::Print(reply) => reply(current.print_copy()),
                Reply::ExportComments(path, format, name, reply) => {
                    reply(write_atomically(&path, &current.comments_file(format, &name)));
                }
                Reply::Compact(path, reply) => reply(current.compacted().and_then(|bytes| {
                    write_atomically(&path, &bytes)?;
                    Ok((current.bytes().len() as u64, bytes.len() as u64))
                })),
                Reply::ReadComments(bytes, reply) => reply(current.comments_from_file(&bytes)),
                Reply::Compare(path, reply) => reply(match Document::open(&path, None, current.limits()) {
                    Ok(other) => current
                        .compare_with(&other)
                        .map_err(|e| format!("The files couldn't be compared: {e}")),
                    Err(OpenError::PasswordRequired) => {
                        Err("That file is password protected, so it can't be compared yet.".into())
                    }
                    Err(e) => Err(format!("That file couldn't be opened: {e}")),
                }),
                Reply::Snapshot {
                    page,
                    scale,
                    rotation,
                    area,
                    reply,
                } => {
                    let page = view.render_page(&cache, page, scale);
                    reply(page.map(|p| p.rotated(rotation).cropped(area)));
                }
                Reply::Edit(list, reply) => break Swap::Edit(list, reply),
                Reply::Layers(list) => break Swap::Layers(list),
                Reply::Pages(edit, reply) => break Swap::Pages(edit, reply),
                Reply::Export(export, reply) => reply(export_pages(view, &cache, &export)),
                Reply::Attachment(index, path, reply) => reply(match current.attachment(index) {
                    Some(bytes) => write_atomically(&path, &bytes),
                    None => Err(SaveError::Io("the attached file couldn't be read".into())),
                }),
                Reply::Extract(files, reply) => {
                    reply(files.iter().try_for_each(|(pages, path)| {
                        current
                            .rearranged(pages, &[])
                            .and_then(|bytes| write_atomically(path, &bytes))
                    }));
                }
            }
        };
        drop(cache);
        match swap {
            Swap::Layers(list) => hidden = list,
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
                    PageEdit::Crop { pages, margins } => current
                        .cropped(&pages, margins)
                        .and_then(|bytes| current.reopen(bytes).map_err(|_| SaveError::Damaged)),
                    PageEdit::HeaderFooter { pages, marks } => current
                        .with_header_footer(&pages, &marks)
                        .and_then(|bytes| current.reopen(bytes).map_err(|_| SaveError::Damaged)),
                    PageEdit::Protect(protection) => {
                        current
                            .protected(protection.as_ref())
                            .and_then(|(bytes, password)| {
                                Document::from_bytes(bytes, Some(&password), current.limits())
                                    .map_err(|_| SaveError::Damaged)
                            })
                    }
                    PageEdit::Clean(clean) => current
                        .cleaned(clean)
                        .and_then(|bytes| current.reopen(bytes).map_err(|_| SaveError::Damaged)),
                    PageEdit::Flatten { comments, fields } => current
                        .flattened(comments, fields)
                        .and_then(|bytes| current.reopen(bytes).map_err(|_| SaveError::Damaged)),
                    PageEdit::FormFields { add, remove } => current
                        .with_form_fields(&add, &remove)
                        .and_then(|bytes| current.reopen(bytes).map_err(|_| SaveError::Damaged)),
                    PageEdit::Redact(areas) => current
                        .redacted(&areas)
                        .and_then(|bytes| current.reopen(bytes).map_err(|_| SaveError::Damaged)),
                    PageEdit::Add { page, additions } => current
                        .with_additions(page, &additions)
                        .and_then(|bytes| current.reopen(bytes).map_err(|_| SaveError::Damaged)),
                    PageEdit::Watermark { pages, mark } => current
                        .with_watermark(&pages, &mark)
                        .and_then(|bytes| current.reopen(bytes).map_err(|_| SaveError::Damaged)),
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
