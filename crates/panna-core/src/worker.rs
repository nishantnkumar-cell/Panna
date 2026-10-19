use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::remote::{Answer, Doc, Engine, Op, Revision};
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
type ArchiveReply = Box<dyn FnOnce(Result<Vec<crate::Finding>, SaveError>) + Send>;
type TagReply = Box<dyn FnOnce(Result<Vec<crate::AccessFinding>, SaveError>) + Send>;
/// What signing needs.
pub(crate) struct SignJob {
    path: PathBuf,
    id: crate::DigitalId,
    options: crate::SignOptions,
}

type SignReply = Box<dyn FnOnce(Result<(), SaveError>) + Send>;
type PreflightReply = Box<dyn FnOnce(Vec<crate::Finding>) + Send>;
type AccessReply = Box<dyn FnOnce(Vec<crate::AccessFinding>) + Send>;
type DetectReply = Box<dyn FnOnce(Result<Vec<crate::NewField>, RenderError>) + Send>;
type PicturesReply = Box<dyn FnOnce(Vec<crate::TextBox>) + Send>;
type LeftoversReply = Box<dyn FnOnce(crate::Leftovers) + Send>;
type CompareReply = Box<dyn FnOnce(Result<Vec<crate::Change>, String>) + Send>;
type PrintReply = Box<dyn FnOnce(Result<Vec<u8>, SaveError>) + Send>;

/// Earlier versions kept for undoing page changes.
const MAX_PAGE_UNDO: usize = 20;

/// A change to the document's pages. Unsaved changes are made part of the document first.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    /// Replaces the bookmarks with these, in reading order.
    Bookmarks(Vec<crate::OutlineItem>),
    /// Sets how the document opens.
    InitialView(crate::InitialView),
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
    /// Moves, replaces or takes out pictures on a page, each named by its place in
    /// [`Worker::pictures`].
    Pictures {
        page: usize,
        changes: Vec<(usize, crate::PictureChange)>,
    },
    /// Takes everything inside these areas (as page text boxes, per page) out for good, and
    /// covers them in black; and the comments and bookmark titles that mention one of
    /// `scrub` (see [`Document::leftovers`]).
    Redact {
        areas: Vec<(usize, Vec<crate::TextBox>)>,
        scrub: Vec<String>,
    },
    /// Adds new form fields and takes out these ones.
    FormFields {
        add: Vec<crate::NewField>,
        remove: Vec<crate::ObjRef>,
    },
    /// Removes hidden information.
    Clean(crate::Cleanup),
    /// Gives the document new password protection, or removes it.
    Protect(Option<crate::Protection>),
    /// Redoes RC4 or AES-128 protection with AES-256 (see [`Document::strengthened`]).
    Strengthen,
    /// Draws comments, form fields or both into the pages.
    Flatten { comments: bool, fields: bool },
    /// Recognises the text on these pages (those without text of their own), and lays it over
    /// them invisibly.
    Recognise(Vec<usize>),
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
    Compact(PathBuf, Option<u32>, CompactReply),
    /// Writes the document's comments to a comments file.
    ExportComments(PathBuf, crate::DataFormat, String, ExtractReply),
    /// Reads a comments file into annotations for this document.
    ReadComments(Vec<u8>, CommentsReply),
    /// Compares the document's text with another file's.
    Compare(PathBuf, CompareReply),
    /// Where the pictures on a page are.
    Pictures(usize, PicturesReply),
    /// What redacting these areas would leave in bookmarks and comments.
    Leftovers(Vec<(usize, Vec<crate::TextBox>)>, LeftoversReply),
    /// Where form fields could go.
    DetectFields(DetectReply),
    /// What keeps the document from being PDF/A.
    Preflight(crate::Standard, PreflightReply),
    Accessibility(AccessReply),
    /// Writes a PDF/A copy.
    Archive(PathBuf, ArchiveReply),
    Tag(PathBuf, TagReply),
    Sign(Box<SignJob>, SignReply),
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
    /// with pictures sharper than `pictures` dots per inch scaled down when given, and replies
    /// with the sizes before and after, in bytes.
    pub fn save_compacted(
        &self,
        path: PathBuf,
        pictures: Option<u32>,
        reply: impl FnOnce(Result<(u64, u64), SaveError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Compact(path, pictures, Box::new(reply)),
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

    /// What keeps the document, with unsaved changes, from being PDF/A or PDF/X (see
    /// [`Document::preflight`] and [`Document::preflight_x`]).
    pub fn preflight(
        &self,
        standard: crate::Standard,
        reply: impl FnOnce(Vec<crate::Finding>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Preflight(standard, Box::new(reply)),
        });
    }

    /// What makes the document, with unsaved changes, harder to use with assistive
    /// technology (see [`Document::accessibility`]).
    pub fn accessibility(&self, reply: impl FnOnce(Vec<crate::AccessFinding>) + Send + 'static) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Accessibility(Box::new(reply)),
        });
    }

    /// Writes a PDF/A copy of the document (with the changes last given to it) to `path`,
    /// and replies with what still keeps the copy from being PDF/A.
    pub fn save_archived(
        &self,
        path: PathBuf,
        reply: impl FnOnce(Result<Vec<crate::Finding>, SaveError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Archive(path, Box::new(reply)),
        });
    }

    /// Writes a copy of the document (with the changes last given to it) with tags added for
    /// screen readers to `path`, and replies with the accessibility check of the copy.
    pub fn save_tagged(
        &self,
        path: PathBuf,
        reply: impl FnOnce(Result<Vec<crate::AccessFinding>, SaveError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Tag(path, Box::new(reply)),
        });
    }

    /// Signs the document, with unsaved changes, and writes the signed copy to `path` (see
    /// [`Document::signed`]).
    pub fn sign(
        &self,
        path: PathBuf,
        id: crate::DigitalId,
        options: crate::SignOptions,
        reply: impl FnOnce(Result<(), SaveError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Sign(Box::new(SignJob { path, id, options }), Box::new(reply)),
        });
    }

    /// Where form fields could go (see [`Document::detect_fields`]).
    pub fn detect_fields(
        &self,
        reply: impl FnOnce(Result<Vec<crate::NewField>, RenderError>) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::DetectFields(Box::new(reply)),
        });
    }

    /// Where the pictures on page `page` are, as shown.
    pub fn pictures(&self, page: usize, reply: impl FnOnce(Vec<crate::TextBox>) + Send + 'static) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Pictures(page, Box::new(reply)),
        });
    }

    /// What redacting `areas` would leave in bookmarks and comments (see
    /// [`Document::leftovers`]).
    pub fn leftovers(
        &self,
        areas: Vec<(usize, Vec<crate::TextBox>)>,
        reply: impl FnOnce(crate::Leftovers) + Send + 'static,
    ) {
        let _ = self.jobs.send(Job {
            index: self.serial.fetch_add(1, Ordering::Relaxed),
            kind: Kind::Pages,
            reply: Reply::Leftovers(areas, Box::new(reply)),
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

/// The files the pages `export` asks for make, each with where it goes.
pub(crate) fn export_files<'a>(
    doc: &'a Document,
    cache: &hayro::RenderCache<'a>,
    export: &crate::Export,
) -> Result<Vec<(PathBuf, Vec<u8>)>, SaveError> {
    use crate::ExportFormat;
    match export.format {
        ExportFormat::Text => {
            let Some((_, path)) = export.pages.first() else {
                return Ok(Vec::new());
            };
            let pages: Vec<usize> = export.pages.iter().map(|(i, _)| *i).collect();
            Ok(vec![(
                path.clone(),
                crate::export::text(doc, &pages)?.into_bytes(),
            )])
        }
        ExportFormat::Word => {
            let Some((_, path)) = export.pages.first() else {
                return Ok(Vec::new());
            };
            let pages: Vec<usize> = export.pages.iter().map(|(i, _)| *i).collect();
            Ok(vec![(path.clone(), crate::office::docx(doc, &pages)?)])
        }
        ExportFormat::Excel => {
            let Some((_, path)) = export.pages.first() else {
                return Ok(Vec::new());
            };
            let pages: Vec<usize> = export.pages.iter().map(|(i, _)| *i).collect();
            Ok(vec![(path.clone(), crate::office::xlsx(doc, &pages)?)])
        }
        ExportFormat::PowerPoint => {
            let Some((_, path)) = export.pages.first() else {
                return Ok(Vec::new());
            };
            let mut slides = Vec::with_capacity(export.pages.len());
            for (index, _) in &export.pages {
                let page = doc
                    .render_page(cache, *index, export.dpi / 72.0)
                    .map_err(|_| SaveError::NoSuchPage(*index))?;
                let size = doc.page_size(*index).ok_or(SaveError::NoSuchPage(*index))?;
                let png = crate::export::picture(&page, ExportFormat::Png)?;
                slides.push((png, f64::from(size.width), f64::from(size.height)));
            }
            Ok(vec![(path.clone(), crate::office::pptx(&slides)?)])
        }
        format => export
            .pages
            .iter()
            .map(|(index, path)| {
                let page = doc
                    .render_page(cache, *index, export.dpi / 72.0)
                    .map_err(|_| SaveError::NoSuchPage(*index))?;
                Ok((path.clone(), crate::export::picture(&page, format)?))
            })
            .collect(),
    }
}

/// Gives figures in the file at `source` descriptions (see [`Document::described`]),
/// writes the result to `target` (which may be the same file), and gives the accessibility
/// check of what was written. Blocks; call it off the interface's thread.
pub fn describe_file(
    source: &Path,
    target: &Path,
    alts: &[((u32, u16), String)],
) -> Result<Vec<crate::AccessFinding>, SaveError> {
    let limits = crate::Limits::default();
    let bytes = Document::read_file(source, limits).map_err(|_| SaveError::Damaged)?;
    let (mut doc, _) = crate::Isolated::open(bytes, None, limits).map_err(|_| SaveError::Damaged)?;
    match doc.call(Op::Described(alts.to_vec())) {
        Answer::Tagged(result) => result.and_then(|(bytes, findings)| {
            write_atomically(target, &bytes)?;
            Ok(findings)
        }),
        _ => Err(SaveError::EngineCrashed),
    }
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
    // Only the bytes are kept here; they're parsed in a locked-down process when there is
    // one (see `remote`).
    let bytes = match Document::read_file(&path, limits) {
        Ok(bytes) => bytes,
        Err(e) => return on_open(Err(e)),
    };
    let mut engine = Engine::default();
    let opened = Doc::new(
        bytes,
        zeroize::Zeroizing::new(password.unwrap_or_default()),
        limits,
    );
    match engine.call(&opened, None, Op::Info) {
        Answer::Info(Ok(info)) => on_open(Ok(info)),
        Answer::Info(Err(e)) => return on_open(Err(e)),
        _ => return on_open(Err(OpenError::EngineCrashed)),
    }

    // `base` is the document as last saved; `live` adds the unsaved annotations to it.
    let mut base = opened;
    let mut live: Option<Doc> = None;
    let mut pending: Vec<Job> = Vec::new();
    let mut history: Vec<Doc> = Vec::new();
    let mut hidden: Option<Vec<crate::ObjRef>> = None;
    loop {
        let current = live.as_ref().unwrap_or(&base).clone();
        let shown = hidden.as_deref();
        // Asks about the document as it is now; the answer is always the kind asked for.
        macro_rules! ask {
            ($op:expr, $kind:ident) => {
                match engine.call(&current, shown, $op) {
                    Answer::$kind(answer) => answer,
                    _ => unreachable!("an answer of another kind"),
                }
            };
        }
        // Serve from `current` until an edit or save needs to swap documents.
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
                    let page = engine.render(&current, shown, job.index, scale);
                    reply(page.map(|p| p.rotated(rotation)));
                }
                Reply::Text(reply) => reply(ask!(Op::Text(job.index), Text)),
                Reply::Save(path, reply) => {
                    let result =
                        write_atomically(&path, &current.bytes).map(|()| ask!(Op::Comments, Comments));
                    let saved = result.is_ok();
                    reply(result);
                    if saved && live.is_some() {
                        break Swap::Promote;
                    }
                }
                Reply::Print(reply) => reply(ask!(Op::PrintCopy, Bytes)),
                Reply::ExportComments(path, format, name, reply) => {
                    reply(
                        ask!(Op::CommentsFile(format, name), Bytes)
                            .and_then(|bytes| write_atomically(&path, &bytes)),
                    );
                }
                Reply::Compact(path, pictures, reply) => {
                    reply(ask!(Op::Compacted(pictures), Bytes).and_then(|bytes| {
                        write_atomically(&path, &bytes)?;
                        Ok((current.bytes.len() as u64, bytes.len() as u64))
                    }))
                }
                Reply::Pictures(page, reply) => reply(ask!(Op::Pictures(page), Boxes)),
                Reply::Leftovers(areas, reply) => reply(ask!(Op::Leftovers(areas), Leftovers)),
                Reply::DetectFields(reply) => reply(ask!(Op::DetectFields, Fields)),
                Reply::Preflight(standard, reply) => reply(ask!(Op::Preflight(standard), Findings)),
                Reply::Accessibility(reply) => reply(ask!(Op::Accessibility, Access)),
                Reply::Archive(path, reply) => {
                    reply(ask!(Op::Archived, Archived).and_then(|(bytes, findings)| {
                        write_atomically(&path, &bytes)?;
                        Ok(findings)
                    }))
                }
                Reply::Tag(path, reply) => reply(ask!(Op::Tagged, Tagged).and_then(|(bytes, findings)| {
                    write_atomically(&path, &bytes)?;
                    Ok(findings)
                })),
                // Signing needs the key, which may be in the system's certificate store, so
                // it's done here. Only on the user's request, for a file they chose to sign.
                Reply::Sign(job, reply) => reply(
                    current
                        .parse()
                        .map_err(|_| SaveError::Damaged)
                        .and_then(|doc| doc.signed_by(&job.id, &job.options))
                        .and_then(|bytes| write_atomically(&job.path, &bytes)),
                ),
                Reply::ReadComments(bytes, reply) => reply(ask!(Op::CommentsFromFile(bytes), Annotations)),
                Reply::Compare(path, reply) => reply(match Document::read_file(&path, current.limits) {
                    Ok(bytes) => ask!(Op::Compare(bytes), Changes),
                    Err(e) => Err(format!("That file couldn't be opened: {e}")),
                }),
                Reply::Snapshot {
                    page,
                    scale,
                    rotation,
                    area,
                    reply,
                } => {
                    let page = engine.render(&current, shown, page, scale);
                    reply(page.map(|p| p.rotated(rotation).cropped(area)));
                }
                Reply::Edit(list, reply) => break Swap::Edit(list, reply),
                Reply::Layers(list) => break Swap::Layers(list),
                Reply::Pages(edit, reply) => break Swap::Pages(edit, reply),
                Reply::Export(export, reply) => {
                    let paths = crate::remote::destinations(&export);
                    reply(ask!(Op::Export(export), Files).and_then(|files| {
                        if files.len() != paths.len() {
                            return Err(SaveError::EngineCrashed);
                        }
                        paths
                            .iter()
                            .zip(&files)
                            .try_for_each(|(path, bytes)| write_atomically(path, bytes))
                    }));
                }
                Reply::Attachment(index, path, reply) => {
                    reply(
                        ask!(Op::Attachment(index), Bytes).and_then(|bytes| write_atomically(&path, &bytes)),
                    );
                }
                Reply::Extract(files, reply) => {
                    reply(files.into_iter().try_for_each(|(pages, path)| {
                        ask!(Op::Rearranged(pages), Bytes).and_then(|bytes| write_atomically(&path, &bytes))
                    }));
                }
            }
        };
        match swap {
            Swap::Layers(list) => hidden = list,
            Swap::Promote => {
                if let Some(saved) = live.take() {
                    base = saved;
                }
            }
            Swap::Pages(edit, reply) => {
                let undoing = matches!(edit, PageEdit::Undo);
                let result = match edit {
                    PageEdit::Undo => {
                        history.pop().ok_or(SaveError::Damaged).and_then(|earlier| {
                            match engine.call(&earlier, None, Op::Info) {
                                Answer::Info(Ok(info)) => Ok((earlier, info)),
                                _ => Err(SaveError::Damaged),
                            }
                        })
                    }
                    PageEdit::InsertFile { at, path } => {
                        let name = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        match Document::read_file(&path, current.limits) {
                            Ok(bytes) => revised(&current, ask!(Op::InsertFile { at, bytes, name }, Revised)),
                            Err(e) => Err(SaveError::Io(format!("couldn't open “{name}”: {e}"))),
                        }
                    }
                    edit => revised(&current, ask!(Op::Edit(edit), Revised)),
                };
                match result {
                    Ok((doc, info)) => {
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
                match engine.call(&base, None, Op::WithChanges(list)) {
                    Answer::Revised(Ok(revision)) => {
                        live = Some(base.revised(revision.bytes));
                        reply(Ok(()));
                    }
                    Answer::Revised(Err(e)) => reply(Err(e)),
                    _ => reply(Err(SaveError::EngineCrashed)),
                }
            }
        }
    }
}

/// The new version an edit of `current` made, and how it's laid out.
fn revised(current: &Doc, revision: Result<Revision, SaveError>) -> Result<(Doc, DocumentInfo), SaveError> {
    let revision = revision?;
    let doc = Doc::new(revision.bytes, revision.password, current.limits);
    Ok((doc, revision.info))
}
