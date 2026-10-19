//! Everything the worker asks of a document, as messages, so it can be done in the
//! locked-down process (see [`isolate`](crate::isolate)) instead of the app's. The app keeps
//! only each version's bytes ([`Doc`]); an [`Engine`] parses them where it's safe to and
//! sends back what was asked for. When no locked-down process can be started, the engine
//! parses them in the app, as before.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{
    AccessFinding, Annotation, Change, Changes, Comment, DataFormat, Document, DocumentInfo, Export, Finding,
    Leftovers, Limits, NewField, ObjRef, OpenError, PageEdit, PageSource, PageText, RenderError,
    RenderedPage, SaveError, Standard, TextBox,
};

/// One version of the document: its bytes, and what opens them. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Doc {
    pub(crate) bytes: Arc<Vec<u8>>,
    pub(crate) password: Arc<Zeroizing<String>>,
    pub(crate) limits: Limits,
    /// Different for every version, so a parsed copy can be kept and reused.
    pub(crate) version: u64,
}

static VERSIONS: AtomicU64 = AtomicU64::new(1);

impl Doc {
    pub(crate) fn new(bytes: Vec<u8>, password: Zeroizing<String>, limits: Limits) -> Self {
        Self {
            bytes: Arc::new(bytes),
            password: Arc::new(password),
            limits,
            version: VERSIONS.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// A later revision of this file, opened the same way.
    pub(crate) fn revised(&self, bytes: Vec<u8>) -> Self {
        Self::new(
            bytes,
            Zeroizing::new(self.password.as_str().to_owned()),
            self.limits,
        )
    }

    pub(crate) fn parse(&self) -> Result<Document, OpenError> {
        Document::from_bytes(self.bytes.to_vec(), Some(self.password.as_str()), self.limits)
    }
}

/// What to do with the loaded document. Requests that make a new version answer with it
/// ([`Answer::Revised`]), already checked to open.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum Op {
    Info,
    /// Of the document as shown (with layers hidden).
    Text(usize),
    Comments,
    /// For printing, with each commented page followed by its comments if set.
    PrintCopy(bool),
    CommentsFile(DataFormat, String),
    /// The comments listed by page, under a heading naming the document.
    CommentSummary(String),
    Compacted(Option<u32>),
    Pictures(usize),
    Leftovers(Vec<(usize, Vec<TextBox>)>),
    DetectFields,
    Preflight(Standard),
    Accessibility,
    Archived,
    Tagged,
    /// Figures given these descriptions, by object number and generation.
    Described(Vec<((u32, u16), String)>),
    CommentsFromFile(Vec<u8>),
    /// Compared with another file's bytes.
    Compare(Vec<u8>),
    Attachment(usize),
    Rearranged(Vec<PageSource>),
    /// Of the document as shown.
    Export(Export),
    /// Every page of another file's bytes inserted before page `at`; `name` is for messages.
    InsertFile {
        at: usize,
        bytes: Vec<u8>,
        name: String,
    },
    /// `count` pages from `first` replaced by another file's pages from `from`.
    ReplaceFromFile {
        first: usize,
        count: usize,
        from: usize,
        bytes: Vec<u8>,
        name: String,
    },
    /// Any page edit but [`PageEdit::Undo`] and those that read another file.
    Edit(PageEdit),
    WithChanges(Changes),
}

/// A new version of the document.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Revision {
    pub(crate) bytes: Vec<u8>,
    pub(crate) password: Zeroizing<String>,
    pub(crate) info: DocumentInfo,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum Answer {
    Info(Result<DocumentInfo, OpenError>),
    Text(Result<PageText, RenderError>),
    Comments(Vec<Comment>),
    Bytes(Result<Vec<u8>, SaveError>),
    /// The files an export makes, in the order of [`destinations`].
    Files(Result<Vec<Vec<u8>>, SaveError>),
    Boxes(Vec<TextBox>),
    Leftovers(Leftovers),
    Fields(Result<Vec<NewField>, RenderError>),
    Findings(Vec<Finding>),
    Access(Vec<AccessFinding>),
    Archived(Result<(Vec<u8>, Vec<Finding>), SaveError>),
    Tagged(Result<(Vec<u8>, Vec<AccessFinding>), SaveError>),
    Annotations(Result<(Vec<Annotation>, usize), String>),
    Changes(Result<Vec<Change>, String>),
    Revised(Result<Revision, SaveError>),
}

impl Op {
    /// The answer when the document couldn't be read, or the process reading it stopped.
    pub(crate) fn failed(&self, crashed: bool) -> Answer {
        let save = if crashed {
            SaveError::EngineCrashed
        } else {
            SaveError::Damaged
        };
        // Pages have no error for an unreadable document: it opened, so it was readable.
        let render = RenderError::EngineCrashed;
        let message = || String::from("The document couldn't be read.");
        match self {
            Self::Info => Answer::Info(Err(if crashed {
                OpenError::EngineCrashed
            } else {
                OpenError::Invalid
            })),
            Self::Text(_) => Answer::Text(Err(render)),
            Self::Comments => Answer::Comments(Vec::new()),
            Self::PrintCopy(_)
            | Self::CommentsFile(..)
            | Self::CommentSummary(_)
            | Self::Compacted(_)
            | Self::Attachment(_)
            | Self::Rearranged(_) => Answer::Bytes(Err(save)),
            Self::Export(_) => Answer::Files(Err(save)),
            Self::Pictures(_) => Answer::Boxes(Vec::new()),
            Self::Leftovers(_) => Answer::Leftovers(Leftovers::default()),
            Self::DetectFields => Answer::Fields(Err(render)),
            Self::Preflight(_) => Answer::Findings(Vec::new()),
            Self::Accessibility => Answer::Access(Vec::new()),
            Self::Archived => Answer::Archived(Err(save)),
            Self::Tagged | Self::Described(_) => Answer::Tagged(Err(save)),
            Self::CommentsFromFile(_) => Answer::Annotations(Err(message())),
            Self::Compare(_) => Answer::Changes(Err(message())),
            Self::InsertFile { .. } | Self::ReplaceFromFile { .. } | Self::Edit(_) | Self::WithChanges(_) => {
                Answer::Revised(Err(save))
            }
        }
    }
}

/// Where the files `export` makes go, in order: the app writes them there itself, so the
/// process that made them can't choose where files are written.
pub(crate) fn destinations(export: &Export) -> Vec<PathBuf> {
    use crate::ExportFormat;
    match export.format {
        ExportFormat::Png | ExportFormat::Jpeg => export.pages.iter().map(|(_, path)| path.clone()).collect(),
        _ => export
            .pages
            .first()
            .map(|(_, path)| path.clone())
            .into_iter()
            .collect(),
    }
}

/// A new version from `bytes`, checked to open.
fn revision(doc: &Document, bytes: Result<Vec<u8>, SaveError>) -> Result<Revision, SaveError> {
    let new = doc.reopen(bytes?).map_err(|_| SaveError::Damaged)?;
    revised(new)
}

fn revised(new: Document) -> Result<Revision, SaveError> {
    let info = new.info().map_err(|_| SaveError::Damaged)?;
    Ok(Revision {
        password: Zeroizing::new(new.password().to_owned()),
        bytes: new.bytes().to_vec(),
        info,
    })
}

/// With a new password, as protecting the document gives.
fn protected(
    doc: &Document,
    made: Result<(Vec<u8>, Zeroizing<String>), SaveError>,
) -> Result<Revision, SaveError> {
    let (bytes, password) = made?;
    revised(Document::from_bytes(bytes, Some(&password), doc.limits()).map_err(|_| SaveError::Damaged)?)
}

/// Does `op` on `doc`, shown as `view` (the same document with layers hidden).
pub(crate) fn perform<'a>(
    doc: &Document,
    view: &'a Document,
    cache: &hayro::RenderCache<'a>,
    op: Op,
) -> Answer {
    match op {
        Op::Info => Answer::Info(doc.info()),
        Op::Text(index) => Answer::Text(view.page_text(index)),
        Op::Comments => Answer::Comments(doc.comments()),
        Op::PrintCopy(false) => Answer::Bytes(doc.print_copy()),
        Op::PrintCopy(true) => Answer::Bytes(doc.print_with_comments()),
        Op::CommentSummary(title) => Answer::Bytes(doc.comment_summary(&title)),
        Op::CommentsFile(format, name) => Answer::Bytes(Ok(doc.comments_file(format, &name))),
        Op::Compacted(pictures) => Answer::Bytes(doc.compacted(pictures)),
        Op::Pictures(page) => Answer::Boxes(doc.pictures(page)),
        Op::Leftovers(areas) => Answer::Leftovers(doc.leftovers(&areas)),
        Op::DetectFields => Answer::Fields(doc.detect_fields()),
        Op::Preflight(Standard::PdfA) => Answer::Findings(doc.preflight()),
        Op::Preflight(Standard::PdfX) => Answer::Findings(doc.preflight_x()),
        Op::Accessibility => Answer::Access(doc.accessibility()),
        Op::Archived => Answer::Archived(doc.archived().and_then(|bytes| {
            let copy = doc.reopen(bytes).map_err(|_| SaveError::Damaged)?;
            let findings = copy.preflight();
            Ok((copy.bytes().to_vec(), findings))
        })),
        Op::Tagged => Answer::Tagged(doc.tagged().and_then(|bytes| {
            let copy = doc.reopen(bytes).map_err(|_| SaveError::Damaged)?;
            let findings = copy.accessibility();
            Ok((copy.bytes().to_vec(), findings))
        })),
        Op::Described(alts) => Answer::Tagged(doc.described(&alts).and_then(|bytes| {
            let copy = doc.reopen(bytes).map_err(|_| SaveError::Damaged)?;
            let findings = copy.accessibility();
            Ok((copy.bytes().to_vec(), findings))
        })),
        Op::CommentsFromFile(bytes) => Answer::Annotations(doc.comments_from_file(&bytes)),
        Op::Compare(bytes) => Answer::Changes(match Document::from_bytes(bytes, None, doc.limits()) {
            Ok(other) => doc
                .compare_with(&other)
                .map_err(|e| format!("The files couldn't be compared: {e}")),
            Err(OpenError::PasswordRequired) => {
                Err("That file is password protected, so it can't be compared yet.".into())
            }
            Err(e) => Err(format!("That file couldn't be opened: {e}")),
        }),
        Op::Attachment(index) => Answer::Bytes(
            doc.attachment(index)
                .ok_or_else(|| SaveError::Io("the attached file couldn't be read".into())),
        ),
        Op::Rearranged(plan) => Answer::Bytes(doc.rearranged(&plan, &[])),
        Op::Export(export) => Answer::Files(
            crate::worker::export_files(view, cache, &export)
                .map(|files| files.into_iter().map(|(_, bytes)| bytes).collect()),
        ),
        Op::InsertFile { at, bytes, name } => Answer::Revised(insert_file(doc, at, bytes, &name)),
        Op::ReplaceFromFile {
            first,
            count,
            from,
            bytes,
            name,
        } => Answer::Revised(replace_from_file(doc, first, count, from, bytes, &name)),
        Op::WithChanges(changes) => Answer::Revised(revision(doc, doc.with_changes(&changes))),
        Op::Edit(edit) => Answer::Revised(match edit {
            PageEdit::Undo | PageEdit::InsertFile { .. } | PageEdit::ReplaceFromFile { .. } => {
                Err(SaveError::Damaged)
            }
            PageEdit::Plan(plan) => revision(doc, doc.rearranged(&plan, &[])),
            PageEdit::Crop { pages, margins } => revision(doc, doc.cropped(&pages, margins)),
            PageEdit::HeaderFooter { pages, marks } => revision(doc, doc.with_header_footer(&pages, &marks)),
            PageEdit::Protect(protection) => protected(doc, doc.protected(protection.as_ref())),
            PageEdit::Strengthen => protected(doc, doc.strengthened()),
            PageEdit::Clean(clean) => revision(doc, doc.cleaned(clean)),
            PageEdit::Flatten { comments, fields } => revision(doc, doc.flattened(comments, fields)),
            PageEdit::FormFields { add, remove } => revision(doc, doc.with_form_fields(&add, &remove)),
            PageEdit::Pictures { page, changes } => revision(doc, doc.with_pictures_changed(page, &changes)),
            PageEdit::Redact { areas, scrub } => revision(doc, doc.redacted(&areas, &scrub)),
            PageEdit::Add { page, additions } => revision(doc, doc.with_additions(page, &additions)),
            PageEdit::Recognise(pages) => revision(doc, doc.recognised(&pages).map(|(bytes, _, _)| bytes)),
            PageEdit::Bookmarks(items) => revision(doc, doc.with_bookmarks(&items)),
            PageEdit::InitialView(view) => revision(doc, doc.with_initial_view(&view)),
            PageEdit::PageLabels(ranges) => revision(doc, doc.with_page_labels(&ranges)),
            PageEdit::Watermark { pages, mark } => revision(doc, doc.with_watermark(&pages, &mark)),
        }),
    }
}

/// `doc` with every page of the file in `bytes` inserted before page `at`.
fn insert_file(doc: &Document, at: usize, bytes: Vec<u8>, name: &str) -> Result<Revision, SaveError> {
    let other = Document::from_bytes(bytes, None, doc.limits())
        .map_err(|e| SaveError::OtherFile(format!("Couldn't open “{name}”: {e}")))?;
    let own = doc.page_count();
    let at = at.min(own);
    let page = |doc, index| PageSource::Page { doc, index, turns: 0 };
    let plan: Vec<PageSource> = (0..at)
        .map(|i| page(0, i))
        .chain((0..other.page_count()).map(|i| page(1, i)))
        .chain((at..own).map(|i| page(0, i)))
        .collect();
    revision(doc, doc.rearranged(&plan, &[&other]))
}

/// `doc` with `count` of its pages from `first` replaced by the pages of the file in `bytes`
/// from `from`.
fn replace_from_file(
    doc: &Document,
    first: usize,
    count: usize,
    from: usize,
    bytes: Vec<u8>,
    name: &str,
) -> Result<Revision, SaveError> {
    let other = Document::from_bytes(bytes, None, doc.limits())
        .map_err(|e| SaveError::OtherFile(format!("Couldn't open “{name}”: {e}")))?;
    if count == 0 || first.saturating_add(count) > doc.page_count() {
        return Err(SaveError::NoSuchPage(first.saturating_add(count)));
    }
    if from.saturating_add(count) > other.page_count() {
        return Err(SaveError::OtherFile(format!(
            "“{name}” has only {} pages, so it hasn't got pages {} to {}.",
            other.page_count(),
            from + 1,
            from + count
        )));
    }
    revision(doc, doc.with_pages_replaced(&other, first, count, from))
}

/// A document parsed in the app, for when there's no locked-down process.
struct Local {
    key: (u64, Option<Vec<ObjRef>>),
    doc: Document,
    view: Option<Document>,
}

/// Does what's asked of documents: in a locked-down process when one can be started, in the
/// app otherwise.
#[derive(Default)]
pub(crate) struct Engine {
    child: Option<crate::isolate::Renderer>,
    /// A locked-down process couldn't be started, so documents are parsed here.
    in_app: bool,
    local: Option<Local>,
}

impl Engine {
    /// The locked-down process, started if need be; `None` when documents are parsed here.
    fn child(&mut self) -> Option<&mut crate::isolate::Renderer> {
        if self.in_app {
            return None;
        }
        if self.child.is_none() {
            self.child = crate::isolate::Renderer::start();
            self.in_app = self.child.is_none();
        }
        self.child.as_mut()
    }

    /// The document parsed here, as shown with `hidden` layers hidden.
    fn local(&mut self, doc: &Doc, hidden: Option<&[ObjRef]>) -> Result<&Local, OpenError> {
        let key = (doc.version, hidden.map(<[ObjRef]>::to_vec));
        if self.local.as_ref().is_none_or(|l| l.key != key) {
            self.local = None;
            let parsed = doc.parse()?;
            let view = hidden.and_then(|h| parsed.with_layers_hidden(h).ok());
            self.local = Some(Local {
                key,
                doc: parsed,
                view,
            });
        }
        self.local.as_ref().ok_or(OpenError::Invalid)
    }

    /// Does `op` on `doc`, shown with `hidden` layers hidden.
    pub(crate) fn call(&mut self, doc: &Doc, hidden: Option<&[ObjRef]>, op: Op) -> Answer {
        if let Some(child) = self.child() {
            return match child.call(doc, hidden, &op) {
                Ok(answer) => answer,
                Err(crashed) => {
                    if crashed {
                        // Stuck, out of memory or crashed: the next request starts a new one.
                        self.child = None;
                    }
                    op.failed(crashed)
                }
            };
        }
        match self.local(doc, hidden) {
            Ok(local) => {
                let view = local.view.as_ref().unwrap_or(&local.doc);
                perform(&local.doc, view, &view.new_cache(), op)
            }
            Err(_) => op.failed(false),
        }
    }

    /// Draws page `index` of `doc` as shown.
    pub(crate) fn render(
        &mut self,
        doc: &Doc,
        hidden: Option<&[ObjRef]>,
        index: usize,
        scale: f32,
    ) -> Result<RenderedPage, RenderError> {
        if let Some(child) = self.child() {
            return child.render(doc, hidden, index, scale).unwrap_or_else(|()| {
                self.child = None;
                Err(RenderError::EngineCrashed)
            });
        }
        let local = self.local(doc, hidden).map_err(|_| RenderError::EngineCrashed)?;
        let view = local.view.as_ref().unwrap_or(&local.doc);
        view.render_page(&view.new_cache(), index, scale)
    }
}

/// A document read in the locked-down process when there is one (as the app's open
/// documents are), for work outside a [`RenderWorker`](crate::RenderWorker).
pub struct Isolated {
    engine: Engine,
    doc: Doc,
}

impl Isolated {
    /// Reads `bytes`, and gives how the document is laid out.
    pub fn open(
        bytes: Vec<u8>,
        password: Option<&str>,
        limits: Limits,
    ) -> Result<(Self, DocumentInfo), OpenError> {
        let doc = Doc::new(
            bytes,
            Zeroizing::new(password.unwrap_or_default().to_owned()),
            limits,
        );
        let mut isolated = Self {
            engine: Engine::default(),
            doc,
        };
        match isolated.call(Op::Info) {
            Answer::Info(Ok(info)) => Ok((isolated, info)),
            Answer::Info(Err(e)) => Err(e),
            _ => Err(OpenError::EngineCrashed),
        }
    }

    /// Draws page `index` at `scale` pixels per point.
    pub fn render(&mut self, index: usize, scale: f32) -> Result<RenderedPage, RenderError> {
        self.engine.render(&self.doc, None, index, scale)
    }

    pub(crate) fn call(&mut self, op: Op) -> Answer {
        self.engine.call(&self.doc, None, op)
    }
}
