//! Panna: an offline, memory-safe PDF viewer.

#![forbid(unsafe_code)]
// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod add;
mod annotate;
mod clipboard;
mod colors;
mod comments;
mod compare;
mod export;
mod find;
mod form;
mod icons;
mod layout;
mod links;
mod measure;
mod pages;
mod pictures;
mod preflight;
mod prepare;
mod print;
mod protect;
mod redact;
mod selection;
mod settings;
mod shortcuts;
mod sidebar;
mod sign;
mod signatures;
mod signing;
mod speech;
mod toolbar;
mod tools;
mod trust;

use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::scrollable::{AbsoluteOffset, Viewport};
use iced::widget::{
    Stack, button, center, column, container, image, mouse_area, opaque, operation, pin, row, scrollable,
    space, stack, text, text_input,
};
use iced::{Center, Element, Fill, Length, Point, Size, Subscription, Task, event, mouse, window};
use panna_core::{
    Changes, Comment, DocumentInfo, Limits, LinkTarget, OpenError, PageEdit, PageSize, PageText, RenderError,
    RenderWorker, RenderedPage, SaveError, TextBox,
};

use icons::Icon;
use layout::{PAGE_GAP, PageLayout, Spread};
use selection::{Caret, PageMap, Selection, TextSlot};
use settings::Settings;
use sidebar::{Sidebar, ThumbLayout};
use toolbar::{Dock, Toolbar};

const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 8.0;
const ZOOM_STEPS: &[f32] = &[
    0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 6.0, 8.0,
];
/// Rendered pages kept around this many pages beyond the visible range.
const KEEP_MARGIN: usize = 2;
/// Size of a selected comment's corner handles, in logical pixels.
const HANDLE: f32 = 8.0;

fn main() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .window(window_settings())
        .centered()
        .run()
}

/// Reverse-DNS app ID. On Linux it must match the `.desktop` file name so GNOME and KDE show
/// the right icon and group windows correctly.
const INITIAL_SIZE: Size = Size::new(1100.0, 850.0);

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const APP_ID: &str = "io.github.nishantnkumar_cell.Panna";

/// The app icon as 64×64 RGBA, for the title bar and task switcher. Generated from the SVG
/// icon; see `packaging/icon/README.md`.
const WINDOW_ICON: &[u8] = include_bytes!("../assets/icon-64.rgba");

fn window_settings() -> window::Settings {
    window::Settings {
        size: INITIAL_SIZE,
        icon: window::icon::from_rgba(WINDOW_ICON.to_vec(), 64, 64).ok(),
        // Closing asks first when there are unsaved annotations.
        exit_on_close_request: false,
        min_size: Some(iced::Size::new(480.0, 360.0)),
        #[cfg(target_os = "linux")]
        platform_specific: window::settings::PlatformSpecific {
            application_id: APP_ID.to_owned(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// An area dragged out with the snapshot tool, in view points (view pixels at 100%).
#[derive(Debug, Clone, Copy)]
struct Snapshot {
    page: usize,
    from: Point,
    to: Point,
    dragging: bool,
    /// Its picture is on the clipboard.
    copied: bool,
}

impl Snapshot {
    /// (x, y, width, height) in view points.
    fn rect(self) -> (f32, f32, f32, f32) {
        let (x0, x1) = (self.from.x.min(self.to.x), self.from.x.max(self.to.x));
        let (y0, y1) = (self.from.y.min(self.to.y), self.from.y.max(self.to.y));
        (x0, y0, x1 - x0, y1 - y0)
    }
}

#[derive(Debug, Clone)]
enum Message {
    OpenDialog,
    FileChosen(Option<PathBuf>),
    Opened(u64, Result<DocumentInfo, OpenError>),
    Rendered {
        doc: u64,
        index: usize,
        scale: f32,
        rotation: u8,
        result: Result<RenderedPage, RenderError>,
    },
    ScaleFactor(f32),
    Scrolled(Viewport),
    ZoomIn,
    ZoomOut,
    ZoomReset,
    FitWidth,
    FitPage,
    RotateCw,
    NextSpread,
    NextColors,
    /// Full screen with the toolbars and panels still showing (F11).
    ToggleFullScreen,
    /// Full screen, one page at a time, nothing else on screen (Ctrl+L, as in Acrobat).
    Present,
    /// Next or previous page while presenting; ignored otherwise.
    Advance(bool),
    /// A page tool from the Pages tab.
    Pages(pages::Op),
    /// The PDF to insert pages from, once chosen.
    InsertFileChosen(Option<PathBuf>),
    /// The Extract pages card.
    Extract(pages::ExtractEvent),
    /// The Crop pages card.
    Crop(pages::CropEvent),
    /// The Headers and footers card.
    Mark(pages::MarkEvent),
    /// The Watermark card.
    Watermark(pages::WatermarkEvent),
    /// The Export card.
    Export(export::Event),
    /// The Flatten card.
    Flatten(pages::FlattenEvent),
    /// The Password protection card.
    Protect(protect::Event),
    /// The Remove hidden information card.
    Clean(protect::CleanEvent),
    /// Shows or hides the All tools card.
    AllTools(bool),
    /// Where to export pages to: a text file, or a folder for pictures.
    ExportChosen(Option<PathBuf>),
    /// Asks for pictures to make a new PDF of.
    FromPictures,
    PicturesChosen(Option<Vec<PathBuf>>),
    /// Where to save the PDF made of the pictures.
    PicturesSaveTo(Option<PathBuf>),
    /// The PDF made of pictures was saved here, or why not.
    PicturesMade(Result<PathBuf, String>),
    /// Where to save extracted pages as one file, once chosen.
    ExtractChosen(Option<PathBuf>),
    /// The folder to save extracted pages to, a file each, once chosen.
    ExtractFolder(Option<PathBuf>),
    PagesChanged {
        doc: u64,
        /// The page to show.
        show: usize,
        undo: bool,
        result: Result<DocumentInfo, SaveError>,
    },
    Extracted(Result<(), SaveError>),
    /// Save a copy of attached file n.
    SaveAttachment(usize),
    AttachmentChosen(usize, Option<PathBuf>),
    /// Asks for a picture to stamp.
    StampPicture,
    StampPictureChosen(Option<PathBuf>),
    StampPictureLoaded(Result<(panna_core::StampImage, String), String>),
    /// Prints the document with its unsaved changes (Ctrl+P).
    Print,
    /// The copy to print is ready.
    PrintCopy(Result<Vec<u8>, SaveError>),
    Printed(Result<(), String>),
    RotateCcw,
    PrevPage,
    NextPage,
    PageInput(String),
    /// Puts the cursor in the page number box.
    FocusPageBox,
    /// A comment tool's key was pressed.
    ToolKey(annotate::Tool),
    PageSubmit,
    Toolbar(toolbar::Event),
    WindowResized(Size),
    PasswordInput(String),
    PasswordSubmit,
    ToggleProperties,
    CloseOverlays,
    Sidebar(sidebar::Event),
    GoToPage(usize),
    /// Scroll to a comment, from the comments list.
    /// Scrolls to a comment, and selects it when it can be changed.
    ShowComment(usize, TextBox, Option<annotate::Picked>),
    ThumbRendered {
        doc: u64,
        index: usize,
        rotation: u8,
        result: Result<RenderedPage, RenderError>,
    },
    TextExtracted {
        doc: u64,
        index: usize,
        result: Result<Arc<PageText>, RenderError>,
    },
    Find(find::Event),
    PagePointer(usize, Point),
    ExportFormData,
    FormDataExportTo(Option<PathBuf>),
    ImportFormData,
    FormDataChosen(Option<PathBuf>),
    /// Save a smaller copy of the document.
    Compact,
    CompactTo(Option<PathBuf>),
    Compacted(Result<(u64, u64), SaveError>),
    ExportComments,
    CommentsExportTo(Option<PathBuf>),
    ImportComments,
    CommentsChosen(Option<PathBuf>),
    /// Comments read from a comments file, and how many in it couldn't be placed.
    CommentsRead {
        doc: u64,
        result: Result<(Vec<panna_core::Annotation>, usize), String>,
    },
    /// Show or hide layer n.
    ShowLayer(usize, bool),
    Measure(measure::Event),
    /// Adding text, pictures and links to pages.
    Add(add::Event),
    Redact(redact::Event),
    Prepare(prepare::Event),
    EditPictures(pictures::Event),
    Preflight(preflight::Event),
    Preflighted {
        doc: u64,
        standard: panna_core::Standard,
        findings: Vec<panna_core::Finding>,
    },
    AccessChecked {
        doc: u64,
        findings: Vec<panna_core::AccessFinding>,
    },
    ArchiveTo(Option<PathBuf>),
    Signing(signing::Event),
    Trust(trust::Event),
    SignTo(Option<PathBuf>),
    SignSaved {
        path: PathBuf,
        result: Result<(), SaveError>,
    },
    Archived {
        doc: u64,
        name: String,
        result: Result<Vec<panna_core::Finding>, SaveError>,
    },
    FieldsDetected {
        doc: u64,
        result: Result<Vec<panna_core::NewField>, String>,
    },
    ReadAloud(speech::Event),
    /// A reading aloud ended: whether all was said, or why not.
    SpeechDone {
        run: u64,
        result: Result<bool, String>,
    },
    PicturesFound {
        doc: u64,
        page: usize,
        boxes: Vec<TextBox>,
    },
    Compare(compare::Event),
    CompareChosen(Option<PathBuf>),
    Compared {
        doc: u64,
        name: String,
        result: Result<Vec<panna_core::Change>, String>,
    },
    AddPictureChosen(Option<PathBuf>),
    AddPictureLoaded(Result<panna_core::StampImage, String>),
    /// Turn the snapshot tool on or off.
    ToggleSnapshot,
    SnapshotTaken(Result<panna_core::RenderedPage, String>),
    PagePress(usize),
    PageDoubleClick(usize),
    PointerReleased,
    Copy,
    Annotate(annotate::Event),
    Form(form::Event),
    /// Tab and Shift+Tab: the next or previous form field.
    FocusNext,
    FocusPrevious,
    /// The Reset form button, then the answer to its confirmation.
    ResetForm,
    ConfirmReset(bool),
    /// The answer to "Open this link?": open it, copy it, or neither.
    OpenLink(LinkChoice),
    AnnotationsApplied {
        doc: u64,
        result: Result<(), SaveError>,
    },
    Save,
    SaveAs,
    SaveAsChosen(Option<PathBuf>),
    Saved {
        doc: u64,
        path: PathBuf,
        /// The saved file's comments.
        result: Result<Vec<Comment>, SaveError>,
    },
    CloseRequested(window::Id),
    /// Show the document in tab n.
    SwitchTab(usize),
    /// Show the next (or previous) tab.
    NextTab(bool),
    /// Close the document shown.
    CloseTab,
    /// Close the document in tab n.
    CloseTabAt(usize),
    Confirm(Choice),
    DismissNotice,
    Ignore,
}

struct App {
    next_doc_id: u64,
    doc: Option<OpenDoc>,
    /// The other open documents, in tab order with the shown one left out.
    parked: Vec<Parked>,
    /// Where the shown document's tab sits among them.
    active_tab: usize,
    /// Files still to open in tabs of their own, last first.
    to_open: Vec<PathBuf>,
    status: Status,
    zoom: f32,
    /// View rotation in clockwise quarter turns (0–3), applied to every page.
    rotation: u8,
    scale_factor: f32,
    viewport: ViewportState,
    page_input: String,
    settings: Settings,
    trusted: trust::Trusted,
    toolbar: Toolbar,
    sidebar: Sidebar,
    window_size: Size,
    show_properties: bool,
    find: find::Find,
    selection: Option<Selection>,
    /// The page under the pointer and the pointer's position on it, in view pixels.
    hover: Option<(usize, Point)>,
    /// A drag-selection is in progress.
    selecting: bool,
    /// The snapshot tool is on: dragging over a page copies a picture of the area.
    snapping: bool,
    snapshot: Option<Snapshot>,
    measure: measure::Measure,
    adder: add::Adder,
    redactor: redact::Redactor,
    preparer: prepare::Preparer,
    picture_editor: pictures::PictureEditor,
    reader: speech::Reader,
    preflight: Option<preflight::Report>,
    /// The comparison with another file, while it's shown.
    comparison: Option<compare::Comparison>,
    annotate: annotate::Annotate,
    form: form::Form,
    /// Asking whether to reset the form.
    confirm_reset: bool,
    /// A link pressed, followed when the button is released over it.
    pending_link: Option<usize>,
    /// A web address waiting for the user's go-ahead.
    confirm_link: Option<String>,
    saving: bool,
    /// Making the copy to print.
    printing: bool,
    /// Choosing pages to extract.
    extracting: Option<pages::Extracting>,
    /// Choosing how much to crop.
    cropping: Option<pages::Cropping>,
    /// Choosing headers and footers.
    marking: Option<pages::Marking>,
    /// Choosing a watermark or page colour.
    watermarking: Option<pages::Watermarking>,
    /// Choosing pages to export, then where.
    exporting: Option<export::Exporting>,
    /// Choosing what to flatten.
    flattening: Option<pages::Flattening>,
    /// Choosing password protection.
    protecting: Option<protect::Protecting>,
    signing: Option<signing::Signing>,
    /// Choosing hidden information to remove.
    cleaning: Option<panna_core::Cleanup>,
    /// The All tools card is showing.
    all_tools: bool,
    /// Pictures chosen for a new PDF, while asking where to save it.
    pictures: Vec<PathBuf>,
    /// The pages being extracted, while their file or folder is chosen.
    extract_pages: Vec<usize>,
    /// What to do once unsaved changes are saved or discarded.
    confirm: Option<Pending>,
    after_save: Option<Pending>,
    /// A problem to show the user, such as a failed save.
    notice: Option<String>,
    screen: Screen,
    /// Unsaved page changes, for undo and "Save changes?".
    page_edits: usize,
    /// A page change is being made; the tools wait for it.
    pages_busy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Screen {
    Windowed,
    Full,
    /// Presenting `page`; `zoom` and `was_full` are restored afterwards.
    Present {
        page: usize,
        zoom: f32,
        was_full: bool,
    },
}

/// An action that would lose unsaved annotations.
#[derive(Debug, Clone)]
enum Pending {
    Close,
    /// Close the document shown, keeping the others open.
    CloseTab,
}

/// Reads `files`, makes a PDF of them and writes it to `target`, on a thread of its own.
async fn make_pdf_from_pictures(files: Vec<PathBuf>, target: PathBuf) -> Result<PathBuf, String> {
    let (tx, rx) = futures_channel::oneshot::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let mut pictures = Vec::with_capacity(files.len());
            for file in &files {
                let name = file.file_name().unwrap_or_default().to_string_lossy();
                if std::fs::metadata(file)
                    .map_err(|e| format!("Couldn't read “{name}”: {e}"))?
                    .len()
                    > 200 * 1024 * 1024
                {
                    return Err(format!("“{name}” is too large."));
                }
                pictures.push(std::fs::read(file).map_err(|e| format!("Couldn't read “{name}”: {e}"))?);
            }
            let pdf = panna_core::pictures_to_pdf(pictures).map_err(|e| {
                // Name the picture rather than its place in the list.
                match e
                    .strip_prefix("Picture ")
                    .and_then(|r| r.split_once(':'))
                    .and_then(|(i, rest)| Some((files.get(i.parse::<usize>().ok()?.checked_sub(1)?)?, rest)))
                {
                    Some((file, rest)) => format!(
                        "“{}”:{rest}",
                        file.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    None => e,
                }
            })?;
            std::fs::write(&target, pdf).map_err(|e| format!("Couldn't write the PDF: {e}"))?;
            Ok(target)
        })();
        let _ = tx.send(result);
    });
    rx.await
        .unwrap_or_else(|_| Err("Making the PDF stopped unexpectedly.".into()))
}

/// The answer to "Save changes?".
#[derive(Debug, Clone, Copy)]
enum Choice {
    Save,
    Discard,
    Cancel,
}

enum Status {
    Idle,
    Loading(PathBuf),
    Error(String),
    Password {
        path: PathBuf,
        input: String,
        /// A password was tried and rejected.
        wrong: bool,
    },
}

/// An open document in a tab that isn't shown, with how it was being viewed.
struct Parked {
    doc: OpenDoc,
    zoom: f32,
    rotation: u8,
    offset: (f32, f32),
    find: find::Find,
    annotate: annotate::Annotate,
    form: form::Form,
    comparison: Option<compare::Comparison>,
    page_edits: usize,
    dirty: bool,
}

impl Parked {
    fn name(&self) -> String {
        tab_name(&self.doc)
    }
}

fn tab_name(doc: &OpenDoc) -> String {
    doc.path.file_name().map_or_else(
        || doc.path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

struct OpenDoc {
    id: u64,
    path: PathBuf,
    used_password: bool,
    info: DocumentInfo,
    worker: RenderWorker,
    layout: PageLayout,
    pages: Vec<Slot>,
    thumb_layout: ThumbLayout,
    thumbs: Vec<Thumb>,
    texts: Vec<TextSlot>,
    /// Whether each layer is shown.
    layers_shown: Vec<bool>,
}

#[derive(Default)]
pub(crate) struct Thumb {
    image: Option<image::Handle>,
    pending: bool,
}

#[derive(Default)]
struct Slot {
    image: Option<image::Handle>,
    /// Render scale of `image`, or of the request in flight.
    rendered_scale: Option<f32>,
    pending_scale: Option<f32>,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
struct ViewportState {
    offset_x: f32,
    offset_y: f32,
    width: f32,
    height: f32,
}

fn scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("pages")
}

fn page_box_id() -> iced::widget::Id {
    iced::widget::Id::new("page-number")
}

/// A card listing document properties, centred over the page view. Clicking outside closes it.
fn properties_view(info: &DocumentInfo) -> Element<'_, Message> {
    let rows = info.properties.iter().map(|(label, value)| {
        row![
            text(*label).width(110).style(text::secondary),
            text(value.as_str()).width(Fill)
        ]
        .spacing(12)
        .into()
    });
    let card = container(
        column![
            text("Document properties").size(18),
            iced::widget::Column::with_children(rows).spacing(6),
            button("Close").on_press(Message::ToggleProperties),
        ]
        .spacing(14)
        .width(460),
    )
    .padding(20)
    .style(container::rounded_box);
    opaque(
        mouse_area(
            center(opaque(card)).style(|_| {
                container::Style::default().background(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.35))
            }),
        )
        .on_press(Message::ToggleProperties),
    )
}

/// Centres `card` over a dimmed backdrop. Clicking the backdrop sends `dismiss`.
fn modal<'a>(card: Element<'a, Message>, dismiss: Message) -> Element<'a, Message> {
    opaque(
        mouse_area(
            center(opaque(card)).style(|_| {
                container::Style::default().background(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.35))
            }),
        )
        .on_press(dismiss),
    )
}

#[derive(Debug, Clone, Copy)]
enum LinkChoice {
    Open,
    Copy,
    Cancel,
}

fn link_view(uri: &str) -> Element<'_, Message> {
    let mail = uri.to_ascii_lowercase().starts_with("mailto:");
    container(
        column![
            text(if mail {
                "Write an email?"
            } else {
                "Open this link in your browser?"
            })
            .size(16),
            text(uri).size(13).style(text::secondary),
            text(if mail {
                "Your mail program will open."
            } else {
                "Panna stays offline; your browser opens the page."
            })
            .size(13)
            .style(text::secondary),
            row![
                button("Copy link")
                    .style(button::secondary)
                    .on_press(Message::OpenLink(LinkChoice::Copy)),
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(Message::OpenLink(LinkChoice::Cancel)),
                button("Open").on_press(Message::OpenLink(LinkChoice::Open)),
            ]
            .spacing(8),
        ]
        .spacing(12)
        .width(440),
    )
    .padding(18)
    .style(container::rounded_box)
    .into()
}

fn reset_view<'a>() -> Element<'a, Message> {
    container(
        column![
            text("Reset the form?").size(16),
            text("Every field goes back to its starting value. You can still close without saving to keep what the file has.")
                .style(text::secondary),
            row![
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(Message::ConfirmReset(false)),
                button("Reset").on_press(Message::ConfirmReset(true)),
            ]
            .spacing(8),
        ]
        .spacing(12)
        .width(400),
    )
    .padding(18)
    .style(container::rounded_box)
    .into()
}

fn password_id() -> iced::widget::Id {
    iced::widget::Id::new("password")
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let settings = Settings::load();
        let mut app = Self {
            parked: Vec::new(),
            active_tab: 0,
            to_open: Vec::new(),
            next_doc_id: 0,
            doc: None,
            status: Status::Idle,
            zoom: 1.0,
            rotation: 0,
            scale_factor: 1.0,
            viewport: ViewportState {
                offset_x: 0.0,
                offset_y: 0.0,
                width: 1000.0,
                height: 800.0,
            },
            page_input: String::new(),
            toolbar: Toolbar::new(settings.toolbar_dock, settings.toolbar_pos),
            sidebar: Sidebar::new(settings.sidebar_open, settings.sidebar_side, settings.sidebar_tab),
            settings,
            trusted: trust::Trusted::load(),
            window_size: INITIAL_SIZE,
            screen: Screen::Windowed,
            page_edits: 0,
            pages_busy: false,
            show_properties: false,
            find: find::Find::default(),
            selection: None,
            hover: None,
            selecting: false,
            annotate: annotate::Annotate::new(),
            form: form::Form::default(),
            confirm_reset: false,
            pending_link: None,
            confirm_link: None,
            saving: false,
            printing: false,
            extracting: None,
            cropping: None,
            marking: None,
            watermarking: None,
            exporting: None,
            flattening: None,
            protecting: None,
            signing: None,
            cleaning: None,
            all_tools: false,
            pictures: Vec::new(),
            extract_pages: Vec::new(),
            confirm: None,
            after_save: None,
            notice: None,
            snapping: false,
            snapshot: None,
            measure: measure::Measure::default(),
            adder: add::Adder::default(),
            redactor: redact::Redactor::default(),
            preparer: prepare::Preparer::default(),
            picture_editor: pictures::PictureEditor::default(),
            reader: speech::Reader::default(),
            preflight: None,
            comparison: None,
        };
        let scale = window::oldest()
            .and_then(window::scale_factor)
            .map(Message::ScaleFactor);
        // Each file named opens in a tab of its own, one after another.
        let mut files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        files.reverse();
        let open = match files.pop() {
            Some(first) => {
                app.to_open = files;
                app.open(first, None)
            }
            None => Task::none(),
        };
        (app, Task::batch([scale, open]))
    }

    fn title(&self) -> String {
        match &self.doc {
            Some(doc) => {
                let name = doc
                    .info
                    .title
                    .clone()
                    .or_else(|| doc.path.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_default();
                format!("{name} — Panna")
            }
            None => "Panna".into(),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let base = event::listen_with(|event, status, _window| match event {
            iced::Event::Window(window::Event::FileDropped(path)) => Some(Message::FileChosen(Some(path))),
            iced::Event::Window(window::Event::Rescaled(factor)) => Some(Message::ScaleFactor(factor)),
            iced::Event::Window(window::Event::Resized(size)) => Some(Message::WindowResized(size)),
            iced::Event::Keyboard(key) if status == event::Status::Ignored => shortcuts::map(key),
            _ => None,
        });
        let mut subs = vec![base, window::close_requests().map(Message::CloseRequested)];
        if self.selecting || self.annotate.busy() {
            // Finish a text selection even when the button is released outside the page.
            subs.push(event::listen_with(|event, _status, _window| match event {
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    Some(Message::PointerReleased)
                }
                _ => None,
            }));
        }
        if !self.toolbar.dragging() {
            return Subscription::batch(subs);
        }
        // Follow the cursor anywhere in the window while a toolbar is being dragged.
        let drag = event::listen_with(|event, _status, _window| match event {
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                Some(Message::Toolbar(toolbar::Event::CursorMoved(position)))
            }
            iced::Event::Mouse(
                mouse::Event::ButtonReleased(mouse::Button::Left) | mouse::Event::CursorLeft,
            ) => Some(Message::Toolbar(toolbar::Event::Released)),
            _ => None,
        });
        subs.push(drag);
        Subscription::batch(subs)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        // A tool picked from the All tools card takes its place.
        if matches!(
            message,
            Message::FromPictures
                | Message::Export(export::Event::Open)
                | Message::Protect(protect::Event::Open)
                | Message::Signing(signing::Event::Open)
                | Message::Clean(protect::CleanEvent::Open)
        ) {
            self.all_tools = false;
        }
        match message {
            Message::OpenDialog => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Open PDF")
                        .add_filter("PDF documents", &["pdf", "PDF"])
                        .pick_file()
                        .await
                        .map(|f| f.path().to_path_buf())
                },
                Message::FileChosen,
            ),
            Message::FileChosen(Some(path)) => self.open(path, None),
            Message::FileChosen(None) | Message::Ignore => Task::none(),
            Message::Opened(id, result) => {
                let Some(doc) = self.doc.as_mut().filter(|d| d.id == id) else {
                    return Task::none();
                };
                match result {
                    Ok(info) => {
                        self.settings.add_recent(doc.path.clone());
                        self.settings.save();
                        self.rotation = 0;
                        doc.layout = PageLayout::new(&info.pages, self.settings.spread);
                        doc.pages = info.pages.iter().map(|_| Slot::default()).collect();
                        doc.thumb_layout = ThumbLayout::new(&info.pages);
                        doc.thumbs = info.pages.iter().map(|_| Thumb::default()).collect();
                        doc.texts = info.pages.iter().map(|_| TextSlot::default()).collect();
                        doc.layers_shown = info.layers.iter().map(|l| l.shown).collect();
                        // Signed documents open on their signatures, as in Acrobat.
                        if !info.signatures.is_empty() {
                            self.sidebar.open = true;
                            self.sidebar.tab = sidebar::Tab::Signatures;
                        }
                        doc.info = info;
                        self.selection = None;
                        self.hover = None;
                        self.annotate.saved(&doc.info.comments);
                        self.form.load(&doc.info.fields);
                        self.find.rescan(&doc.texts);
                        self.status = Status::Idle;
                        self.zoom = 1.0;
                        self.viewport.offset_y = 0.0;
                        let shown = Task::batch([
                            operation::scroll_to(scroll_id(), AbsoluteOffset { x: 0.0, y: 0.0 }),
                            self.request_visible(),
                            self.request_thumbnails(),
                            self.request_search_text(),
                            self.request_comment_text(),
                        ]);
                        match self.to_open.pop() {
                            Some(next) => Task::batch([shown, self.open(next, None)]),
                            None => shown,
                        }
                    }
                    Err(OpenError::PasswordRequired) => {
                        self.status = Status::Password {
                            path: doc.path.clone(),
                            input: String::new(),
                            wrong: doc.used_password,
                        };
                        self.doc = None;
                        operation::focus(password_id())
                    }
                    Err(e) => {
                        self.doc = None;
                        self.status = Status::Error(e.to_string());
                        Task::none()
                    }
                }
            }
            Message::Rendered {
                doc,
                index,
                scale,
                rotation,
                result,
            } => {
                if let Some(slot) = self
                    .doc
                    .as_mut()
                    .filter(|d| d.id == doc && rotation == self.rotation)
                    .and_then(|d| d.pages.get_mut(index))
                {
                    if slot.pending_scale == Some(scale) {
                        slot.pending_scale = None;
                    }
                    match result {
                        Ok(page) => {
                            slot.image = Some(image::Handle::from_rgba(page.width, page.height, page.rgba));
                            slot.rendered_scale = Some(scale);
                            slot.error = None;
                        }
                        Err(e) => slot.error = Some(e.to_string()),
                    }
                }
                Task::none()
            }
            Message::ScaleFactor(factor) => {
                if factor.is_finite() && factor > 0.0 {
                    self.scale_factor = factor;
                    return self.request_visible();
                }
                Task::none()
            }
            Message::Scrolled(viewport) => {
                let bounds = viewport.bounds();
                self.viewport = ViewportState {
                    offset_x: viewport.absolute_offset().x,
                    offset_y: viewport.absolute_offset().y,
                    width: bounds.width,
                    height: bounds.height,
                };
                self.page_input.clear();
                self.request_visible()
            }
            Message::ZoomIn => {
                let next = ZOOM_STEPS.iter().copied().find(|z| *z > self.zoom + 0.001);
                self.set_zoom(next.unwrap_or(self.zoom))
            }
            Message::ZoomOut => {
                let next = ZOOM_STEPS.iter().rev().copied().find(|z| *z < self.zoom - 0.001);
                self.set_zoom(next.unwrap_or(self.zoom))
            }
            Message::ZoomReset => self.set_zoom(1.0),
            Message::FitWidth => {
                let available = (self.viewport.width - 2.0 * PAGE_GAP - 16.0).max(100.0);
                match self.doc.as_ref().and_then(|d| d.layout.fit_width(available)) {
                    Some(zoom) => self.set_zoom(zoom),
                    None => Task::none(),
                }
            }
            Message::FitPage => {
                let Some(doc) = &self.doc else { return Task::none() };
                let page = self.current_page();
                let Some((row_width, pages, row_height)) = doc.layout.row_size(page) else {
                    return Task::none();
                };
                let room =
                    self.viewport.width - 2.0 * PAGE_GAP - 16.0 - pages.saturating_sub(1) as f32 * PAGE_GAP;
                let width = room.max(100.0) / row_width.max(1.0);
                let height = (self.viewport.height - 2.0 * PAGE_GAP).max(100.0) / row_height.max(1.0);
                let zoom_task = self.set_zoom(width.min(height));
                Task::batch([zoom_task, self.go_to_page(page)])
            }
            Message::RotateCw => self.rotate(1),
            Message::NextColors => {
                self.settings.colors = self.settings.colors.next();
                self.settings.save();
                // Every page and thumbnail is drawn again in the new colours.
                if let Some(doc) = &mut self.doc {
                    doc.pages.iter_mut().for_each(|slot| *slot = Slot::default());
                    doc.thumbs.iter_mut().for_each(|thumb| *thumb = Thumb::default());
                }
                Task::batch([self.request_visible(), self.request_thumbnails()])
            }
            Message::NextSpread => {
                self.settings.spread = self.settings.spread.next();
                self.settings.save();
                self.relayout()
            }
            Message::RotateCcw => self.rotate(3),
            Message::PrevPage | Message::NextPage if matches!(self.screen, Screen::Present { .. }) => {
                self.update(Message::Advance(matches!(message, Message::NextPage)))
            }
            Message::PrevPage => self.step_page(false),
            Message::NextPage => self.step_page(true),
            Message::FocusPageBox => {
                operation::focus(page_box_id()).chain(operation::select_all(page_box_id()))
            }
            Message::ToolKey(tool) => {
                if self.annotate.open {
                    return self.update(Message::Annotate(annotate::Event::Tool(tool)));
                }
                Task::none()
            }
            Message::PageInput(value) => {
                self.page_input = value.chars().filter(char::is_ascii_digit).take(6).collect();
                Task::none()
            }
            Message::PageSubmit => {
                let target = self
                    .page_input
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(1));
                self.page_input.clear();
                // Hand the keyboard back to the page, so Home, End and the tool keys work again.
                // Focusing the (unfocusable) page view takes focus off the box.
                let release = operation::focus(scroll_id());
                match target {
                    Some(index) => Task::batch([self.go_to_page(index), release]),
                    None => release,
                }
            }
            Message::Toolbar(event) => {
                if self.toolbar.update(event, self.window_size) {
                    self.settings.toolbar_dock = self.toolbar.dock;
                    self.settings.toolbar_pos = self.toolbar.pos;
                    self.settings.save();
                }
                Task::none()
            }
            Message::WindowResized(size) => {
                self.window_size = size;
                // Refit the page once the window has gone full screen.
                match self.screen {
                    Screen::Present { page, .. } => self.present_page(page),
                    _ => Task::none(),
                }
            }
            Message::ToggleFullScreen => match self.screen {
                Screen::Windowed => {
                    self.screen = Screen::Full;
                    set_window_mode(window::Mode::Fullscreen)
                }
                Screen::Full => {
                    self.screen = Screen::Windowed;
                    set_window_mode(window::Mode::Windowed)
                }
                Screen::Present { .. } => self.stop_presenting(),
            },
            Message::Present => match self.screen {
                Screen::Present { .. } => self.stop_presenting(),
                _ if self.doc.as_ref().is_some_and(|d| !d.pages.is_empty()) => {
                    self.screen = Screen::Present {
                        page: self.current_page(),
                        zoom: self.zoom,
                        was_full: self.screen == Screen::Full,
                    };
                    self.selection = None;
                    self.annotate.unpick();
                    let page = self.current_page();
                    Task::batch([set_window_mode(window::Mode::Fullscreen), self.present_page(page)])
                }
                _ => Task::none(),
            },
            Message::Advance(forward) => match self.screen {
                Screen::Present { page, .. } => {
                    self.present_page(if forward { page + 1 } else { page.saturating_sub(1) })
                }
                _ => Task::none(),
            },
            Message::PasswordInput(value) => {
                if let Status::Password { input, .. } = &mut self.status {
                    *input = value;
                }
                Task::none()
            }
            Message::PasswordSubmit => {
                let Status::Password { path, input, .. } = &mut self.status else {
                    return Task::none();
                };
                let (path, password) = (path.clone(), std::mem::take(input));
                self.open(path, Some(password))
            }
            Message::ToggleProperties => {
                self.show_properties = !self.show_properties && self.doc.is_some();
                Task::none()
            }
            Message::CloseOverlays => {
                if matches!(self.screen, Screen::Present { .. }) {
                    return self.stop_presenting();
                }
                if self.confirm_link.is_some() {
                    self.confirm_link = None;
                } else if self.snapping {
                    self.snapping = false;
                    self.snapshot = None;
                } else if self.picture_editor.on {
                    if self.picture_editor.busy() {
                        self.picture_editor.cancel();
                    } else {
                        self.picture_editor.stop();
                    }
                } else if self.preparer.on {
                    if self.preparer.busy() {
                        self.preparer.cancel();
                    } else {
                        self.preparer.stop();
                    }
                } else if self.redactor.on {
                    if self.redactor.busy() {
                        self.redactor.cancel();
                    } else {
                        self.redactor.stop();
                    }
                } else if self.adder.on {
                    if self.adder.busy() {
                        self.adder.cancel();
                    } else {
                        self.adder.on = false;
                    }
                } else if self.measure.on {
                    if self.measure.active() {
                        self.measure.clear();
                    } else {
                        self.measure.update(measure::Event::Toggle);
                    }
                } else if self.extracting.is_some() {
                    self.extracting = None;
                } else if self.cropping.is_some() {
                    self.cropping = None;
                } else if self.marking.is_some() {
                    self.marking = None;
                } else if self.watermarking.is_some() {
                    self.watermarking = None;
                } else if self.exporting.is_some() {
                    self.exporting = None;
                } else if self.flattening.is_some() {
                    self.flattening = None;
                } else if self.protecting.is_some() {
                    self.protecting = None;
                } else if self.signing.is_some() {
                    self.signing = None;
                } else if self.cleaning.is_some() {
                    self.cleaning = None;
                } else if self.all_tools {
                    self.all_tools = false;
                } else if self.confirm_reset {
                    self.confirm_reset = false;
                } else if self.show_properties {
                    self.show_properties = false;
                } else if self.find.open {
                    self.find.open = false;
                    self.find.rescan(&[]);
                } else if self.annotate.stamping.is_some() {
                    self.annotate.stamping = None;
                } else if self.annotate.draft.is_some() {
                    self.annotate.update(annotate::Event::CancelShape);
                } else if self.screen == Screen::Full
                    && self.selection.is_none()
                    && self.annotate.picked.is_none()
                {
                    return self.update(Message::ToggleFullScreen);
                } else {
                    self.selection = None;
                    self.annotate.unpick();
                }
                Task::none()
            }
            Message::Sidebar(event) => {
                if self.sidebar.update(event) {
                    self.save_sidebar();
                }
                Task::batch([self.request_thumbnails(), self.request_comment_text()])
            }
            Message::GoToPage(index) => match self.screen {
                Screen::Present { .. } => self.present_page(index),
                _ => self.go_to_page(index),
            },
            Message::ShowComment(page, at, picked) => {
                // Selecting a comment in the list selects it on the page too, ready to move
                // or restyle.
                if let Some(picked) = picked
                    && !self.saving
                {
                    if !self.annotate.open {
                        self.annotate.update(annotate::Event::Toggle);
                    }
                    self.annotate
                        .update(annotate::Event::Tool(annotate::Tool::Select));
                    self.annotate.pick(picked);
                }
                self.reveal(page, at)
            }
            Message::ThumbRendered {
                doc,
                index,
                rotation,
                result,
            } => {
                if let Some(thumb) = self
                    .doc
                    .as_mut()
                    .filter(|d| d.id == doc && rotation == self.rotation)
                    .and_then(|d| d.thumbs.get_mut(index))
                {
                    thumb.pending = false;
                    // A failed thumbnail stays blank; the page view shows the error.
                    if let Ok(page) = result {
                        thumb.image = Some(image::Handle::from_rgba(page.width, page.height, page.rgba));
                    }
                }
                Task::none()
            }
            Message::TextExtracted { doc, index, result } => {
                let Some(open) = self.doc.as_mut().filter(|d| d.id == doc) else {
                    return Task::none();
                };
                let Some(slot) = open.texts.get_mut(index) else {
                    return Task::none();
                };
                slot.pending = false;
                // A page whose text can't be read is treated as having none.
                let text = result.unwrap_or_default();
                self.find.add_page(index, &text);
                slot.text = Some(text);
                if self.reader.waiting_for() == Some(index) {
                    return self.go_on_reading();
                }
                if self.find.needs_first() {
                    return self.step_find(true);
                }
                Task::none()
            }
            Message::Find(event) => self.find_event(event),
            Message::PagePointer(page, position) => {
                self.hover = Some((page, position));
                if self.adder.on {
                    self.adder.hover(page, self.page_map(page).to_page(position));
                    return Task::none();
                }
                if self.redactor.on {
                    self.redactor.hover(page, self.page_map(page).to_page(position));
                    return Task::none();
                }
                if self.preparer.on {
                    self.preparer.hover(page, self.page_map(page).to_page(position));
                    return Task::none();
                }
                if self.picture_editor.on {
                    self.picture_editor
                        .hover(page, self.page_map(page).to_page(position));
                    return self.ask_pictures(page);
                }
                if self.measure.on {
                    self.measure.hover(page, self.page_map(page).to_page(position));
                    return Task::none();
                }
                if let Some(snap) = self.snapshot.as_mut().filter(|s| s.dragging && s.page == page) {
                    snap.to = Point::new(position.x / self.zoom, position.y / self.zoom);
                    return Task::none();
                }
                if self.annotate.busy() {
                    let at = self.page_map(page).to_page(position);
                    self.annotate.drag(page, at);
                    return Task::none();
                }
                if self.selecting
                    && let Some(caret) = self.caret_at(page, position)
                    && let Some(selection) = &mut self.selection
                {
                    selection.focus = caret;
                }
                Task::none()
            }
            Message::PagePress(page) if self.adder.on => {
                if let Some((_, position)) = self.hover.filter(|(p, _)| *p == page) {
                    self.adder.press(page, self.page_map(page).to_page(position));
                }
                Task::none()
            }
            Message::PagePress(page) if self.preparer.on => {
                if let Some((_, position)) = self.hover.filter(|(p, _)| *p == page) {
                    self.preparer.press(page, self.page_map(page).to_page(position));
                }
                Task::none()
            }
            Message::PagePress(page) if self.picture_editor.on => {
                if let Some((_, position)) = self.hover.filter(|(p, _)| *p == page) {
                    let slop = pictures::HANDLE / self.zoom.max(0.01);
                    self.picture_editor
                        .press(page, self.page_map(page).to_page(position), slop);
                }
                Task::none()
            }
            Message::PagePress(page) if self.redactor.on => {
                if let Some((_, position)) = self.hover.filter(|(p, _)| *p == page) {
                    self.redactor.press(page, self.page_map(page).to_page(position));
                }
                Task::none()
            }
            Message::PagePress(page) if self.measure.on => {
                if let Some((_, position)) = self.hover.filter(|(p, _)| *p == page) {
                    self.measure.press(page, self.page_map(page).to_page(position));
                }
                Task::none()
            }
            Message::PagePress(page) if self.snapping => {
                self.selection = None;
                self.snapshot = self.hover.filter(|(p, _)| *p == page).map(|(_, position)| {
                    let at = Point::new(position.x / self.zoom, position.y / self.zoom);
                    Snapshot {
                        page,
                        from: at,
                        to: at,
                        dragging: true,
                        copied: false,
                    }
                });
                Task::none()
            }
            Message::ExportFormData => {
                self.all_tools = false;
                let name = format!("{}.fdf", self.file_stem());
                Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Save form data as")
                            .add_filter("Form data (FDF)", &["fdf"])
                            .add_filter("Form data (XFDF)", &["xfdf"])
                            .set_file_name(name)
                            .save_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::FormDataExportTo,
                )
            }
            Message::FormDataExportTo(None) | Message::FormDataChosen(None) => Task::none(),
            Message::FormDataExportTo(Some(path)) => {
                let Some(doc) = &self.doc else { return Task::none() };
                let xml = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xfdf"));
                let format = if xml {
                    panna_core::DataFormat::Xfdf
                } else {
                    panna_core::DataFormat::Fdf
                };
                let pdf_name = doc
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let bytes = panna_core::export_form_data(&self.form.data(), format, &pdf_name);
                Task::perform(
                    async move { std::fs::write(&path, bytes).map_err(|e| SaveError::Io(e.to_string())) },
                    Message::Extracted,
                )
            }
            Message::ImportFormData => {
                self.all_tools = false;
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Fill in the form from")
                            .add_filter("Form data", &["fdf", "xfdf"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::FormDataChosen,
                )
            }
            Message::FormDataChosen(Some(path)) => {
                if self.saving {
                    return Task::none();
                }
                // Form data files are small; a big one isn't one.
                const MAX: u64 = 16 * 1024 * 1024;
                let read = std::fs::metadata(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|m| {
                        if m.len() > MAX {
                            Err("the file is too large to be form data".into())
                        } else {
                            std::fs::read(&path).map_err(|e| e.to_string())
                        }
                    })
                    .and_then(|bytes| panna_core::import_form_data(&bytes));
                match read {
                    Ok(data) => {
                        if self.form.import(&data) == 0 {
                            self.notice = Some(
                                "None of the fields in that file could be filled in on this form.".into(),
                            );
                        }
                    }
                    Err(e) => self.notice = Some(format!("Couldn't read the form data: {e}")),
                }
                Task::none()
            }
            Message::Compact => {
                self.all_tools = false;
                let name = format!("{} (smaller).pdf", self.file_stem());
                Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Save a smaller copy as")
                            .add_filter("PDF", &["pdf"])
                            .set_file_name(name)
                            .save_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::CompactTo,
                )
            }
            Message::CompactTo(None) => Task::none(),
            Message::CompactTo(Some(path)) => {
                let Some(doc) = &self.doc else { return Task::none() };
                // Unsaved changes go into the copy too.
                if self.dirty() {
                    doc.worker.set_changes(self.changes(), |_| {});
                }
                let (tx, rx) = futures_channel::oneshot::channel();
                doc.worker.save_compacted(path, move |result| {
                    let _ = tx.send(result);
                });
                Task::perform(rx, |result| {
                    Message::Compacted(result.unwrap_or(Err(SaveError::EngineCrashed)))
                })
            }
            Message::Compacted(result) => {
                self.notice = Some(match result {
                    Ok((before, after)) if after < before => format!(
                        "Saved a copy {} smaller: {} instead of {}.",
                        sidebar::size_text(before - after),
                        sidebar::size_text(after),
                        sidebar::size_text(before)
                    ),
                    Ok(_) => "Saved a copy. This file was already about as small as it gets.".into(),
                    Err(e) => e.to_string(),
                });
                Task::none()
            }
            Message::ExportComments => {
                self.all_tools = false;
                let name = format!("{}.xfdf", self.file_stem());
                Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Save comments as")
                            .add_filter("Comments (XFDF)", &["xfdf"])
                            .add_filter("Comments (FDF)", &["fdf"])
                            .set_file_name(name)
                            .save_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::CommentsExportTo,
                )
            }
            Message::CommentsExportTo(None) | Message::CommentsChosen(None) => Task::none(),
            Message::CommentsExportTo(Some(path)) => {
                let Some(doc) = &self.doc else { return Task::none() };
                let fdf = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("fdf"));
                let format = if fdf {
                    panna_core::DataFormat::Fdf
                } else {
                    panna_core::DataFormat::Xfdf
                };
                let pdf_name = doc
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // The worker already has every unsaved comment: each change is sent to it.
                let (tx, rx) = futures_channel::oneshot::channel();
                doc.worker.export_comments(path, format, pdf_name, move |result| {
                    let _ = tx.send(result);
                });
                Task::perform(rx, |result| {
                    Message::Extracted(result.unwrap_or(Err(SaveError::EngineCrashed)))
                })
            }
            Message::Compare(event) => self.compare_event(event),
            Message::CompareChosen(None) => Task::none(),
            Message::CompareChosen(Some(path)) => {
                let Some(doc) = &self.doc else { return Task::none() };
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                let (tx, rx) = futures_channel::oneshot::channel();
                doc.worker.compare(path, move |result| {
                    let _ = tx.send(result);
                });
                self.notice = Some(format!("Comparing with {name}…"));
                let id = doc.id;
                Task::perform(rx, move |result| Message::Compared {
                    doc: id,
                    name,
                    result: result.unwrap_or_else(|_| Err("The files couldn't be compared.".into())),
                })
            }
            Message::Compared { doc, name, result } => {
                if self.doc.as_ref().is_none_or(|d| d.id != doc) {
                    return Task::none();
                }
                match result {
                    Ok(changes) => {
                        self.notice = None;
                        self.comparison = Some(compare::Comparison::new(name, changes));
                        self.sidebar.open = true;
                        self.sidebar.tab = sidebar::Tab::Changes;
                    }
                    Err(e) => self.notice = Some(e),
                }
                Task::none()
            }
            Message::ImportComments => {
                self.all_tools = false;
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Add comments from")
                            .add_filter("Comments", &["xfdf", "fdf"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::CommentsChosen,
                )
            }
            Message::CommentsChosen(Some(path)) => {
                let Some(doc) = &self.doc else { return Task::none() };
                // Comments files are small; a big one isn't one.
                const MAX: u64 = 16 * 1024 * 1024;
                let read = std::fs::metadata(&path).map_err(|e| e.to_string()).and_then(|m| {
                    if m.len() > MAX {
                        Err("the file is too large to be a comments file".into())
                    } else {
                        std::fs::read(&path).map_err(|e| e.to_string())
                    }
                });
                let bytes = match read {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        self.notice = Some(format!("Couldn't read the comments: {e}"));
                        return Task::none();
                    }
                };
                let (tx, rx) = futures_channel::oneshot::channel();
                doc.worker.read_comments(bytes, move |result| {
                    let _ = tx.send(result);
                });
                let id = doc.id;
                Task::perform(rx, move |result| Message::CommentsRead {
                    doc: id,
                    result: result.unwrap_or_else(|_| Err("the file couldn't be read".into())),
                })
            }
            Message::CommentsRead { doc, result } => {
                if self.doc.as_ref().is_none_or(|d| d.id != doc) || self.saving {
                    return Task::none();
                }
                match result {
                    Ok((list, skipped)) => {
                        let added = list.len();
                        if added == 0 {
                            self.notice = Some("None of the comments in that file fit this document.".into());
                        } else if skipped > 0 {
                            self.notice = Some(format!(
                                "Added {added} comments. {skipped} didn't fit this document and were left out."
                            ));
                        }
                        let outcome = self.annotate.add_imported(list);
                        return self.after_annotate(outcome);
                    }
                    Err(e) => self.notice = Some(format!("Couldn't read the comments: {e}")),
                }
                Task::none()
            }
            Message::ShowLayer(index, show) => {
                let Some(open) = self.doc.as_mut() else {
                    return Task::none();
                };
                let Some(slot) = open.layers_shown.get_mut(index) else {
                    return Task::none();
                };
                *slot = show;
                let hidden = open
                    .info
                    .layers
                    .iter()
                    .zip(&open.layers_shown)
                    .filter(|(_, shown)| !**shown)
                    .map(|(layer, _)| layer.id)
                    .collect();
                open.worker.set_layers(Some(hidden));
                // Re-render; old bitmaps stay up until the new ones arrive.
                for slot in &mut open.pages {
                    slot.rendered_scale = None;
                    slot.pending_scale = None;
                }
                for thumb in &mut open.thumbs {
                    *thumb = Thumb::default();
                }
                Task::batch([self.request_visible(), self.request_thumbnails()])
            }
            Message::Add(event) => {
                if self.saving {
                    return Task::none();
                }
                self.all_tools = false;
                let pages = self.doc.as_ref().map_or(0, |d| d.pages.len());
                let outcome = self.adder.update(event, pages);
                if self.adder.on {
                    self.redactor.stop();
                    self.preparer.stop();
                    self.picture_editor.stop();
                    self.measure.on = false;
                    self.measure.clear();
                    self.snapping = false;
                    self.snapshot = None;
                    self.selection = None;
                    if self.annotate.open {
                        self.annotate.update(annotate::Event::Toggle);
                    }
                }
                self.after_adding(outcome)
            }
            Message::Prepare(event) => {
                if self.saving || self.doc.is_none() {
                    return Task::none();
                }
                self.all_tools = false;
                let outcome = self.preparer.update(event);
                if self.preparer.on {
                    self.redactor.stop();
                    self.picture_editor.stop();
                    self.adder.on = false;
                    self.adder.cancel();
                    self.measure.on = false;
                    self.measure.clear();
                    self.snapping = false;
                    self.snapshot = None;
                    self.selection = None;
                    if self.annotate.open {
                        self.annotate.update(annotate::Event::Toggle);
                    }
                }
                match outcome {
                    prepare::Outcome::Nothing => Task::none(),
                    prepare::Outcome::Edit { add, remove } => {
                        let show = add.first().map_or(self.current_page(), |f| f.page);
                        self.edit_pages(PageEdit::FormFields { add, remove }, show, false)
                    }
                    prepare::Outcome::Detect => {
                        let Some(doc) = &self.doc else { return Task::none() };
                        let (tx, rx) = futures_channel::oneshot::channel();
                        doc.worker.detect_fields(move |result| {
                            let _ = tx.send(result);
                        });
                        self.notice = Some("Looking for fields…".into());
                        let id = doc.id;
                        Task::perform(rx, move |result| Message::FieldsDetected {
                            doc: id,
                            result: match result {
                                Ok(Ok(fields)) => Ok(fields),
                                Ok(Err(e)) => Err(format!("Fields couldn't be looked for: {e}")),
                                Err(_) => Err("Fields couldn't be looked for.".into()),
                            },
                        })
                    }
                }
            }
            Message::EditPictures(event) => {
                if self.saving || self.doc.is_none() {
                    return Task::none();
                }
                self.all_tools = false;
                let outcome = self.picture_editor.update(event);
                if self.picture_editor.on {
                    self.redactor.stop();
                    self.preparer.stop();
                    self.adder.on = false;
                    self.adder.cancel();
                    self.measure.on = false;
                    self.measure.clear();
                    self.snapping = false;
                    self.snapshot = None;
                    self.selection = None;
                    if self.annotate.open {
                        self.annotate.update(annotate::Event::Toggle);
                    }
                }
                let ask = self.ask_pictures(self.current_page());
                Task::batch([ask, self.after_pictures(outcome)])
            }
            Message::ReadAloud(event) => {
                self.all_tools = false;
                match event {
                    speech::Event::Stop => {
                        self.reader.stop();
                        Task::none()
                    }
                    speech::Event::Page | speech::Event::ToEnd if self.doc.is_some() => {
                        if matches!(event, speech::Event::Page)
                            && let Some(text) = self.selected_text()
                        {
                            return match self.reader.read_text(&text) {
                                Ok(task) => task,
                                Err(e) => {
                                    self.notice = Some(e);
                                    Task::none()
                                }
                            };
                        }
                        let to_end = matches!(event, speech::Event::ToEnd);
                        self.reader.read_page(self.current_page(), to_end);
                        self.go_on_reading()
                    }
                    _ => Task::none(),
                }
            }
            Message::SpeechDone { run, result } => {
                let pages = self.doc.as_ref().map_or(0, |d| d.texts.len());
                match result {
                    Ok(all_said) => {
                        if self.reader.done(run, all_said, pages) {
                            return self.go_on_reading();
                        }
                    }
                    Err(e) => {
                        if self.reader.done(run, false, pages) || !self.reader.active() {
                            self.notice = Some(e);
                        }
                    }
                }
                Task::none()
            }
            Message::Preflight(preflight::Event::Close) => {
                self.preflight = None;
                Task::none()
            }
            Message::Preflight(preflight::Event::Convert) => {
                self.preflight = None;
                let name = format!("{} (PDF-A).pdf", self.file_stem());
                Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Save a PDF/A copy as")
                            .add_filter("PDF", &["pdf"])
                            .set_file_name(name)
                            .save_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::ArchiveTo,
                )
            }
            Message::ArchiveTo(None) => Task::none(),
            Message::ArchiveTo(Some(path)) => {
                let Some(doc) = &self.doc else { return Task::none() };
                // Unsaved changes go into the copy too.
                if self.dirty() {
                    doc.worker.set_changes(self.changes(), |_| {});
                }
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                let (tx, rx) = futures_channel::oneshot::channel();
                doc.worker.save_archived(path, move |result| {
                    let _ = tx.send(result);
                });
                let id = doc.id;
                Task::perform(rx, move |result| Message::Archived {
                    doc: id,
                    name,
                    result: result.unwrap_or(Err(SaveError::EngineCrashed)),
                })
            }
            Message::Archived { doc, name, result } => {
                if self.doc.as_ref().is_some_and(|d| d.id == doc) {
                    match result {
                        Ok(findings) => self.preflight = Some(preflight::Report::saved(name, findings)),
                        Err(e) => self.notice = Some(e.to_string()),
                    }
                }
                Task::none()
            }
            Message::Preflight(preflight::Event::Show(page)) => {
                self.preflight = None;
                self.go_to_page(page)
            }
            Message::Preflight(event @ (preflight::Event::Run | preflight::Event::RunPrint)) => {
                self.all_tools = false;
                let standard = match event {
                    preflight::Event::RunPrint => panna_core::Standard::PdfX,
                    _ => panna_core::Standard::PdfA,
                };
                let Some(doc) = &self.doc else { return Task::none() };
                if self.dirty() {
                    doc.worker.set_changes(self.changes(), |_| {});
                }
                let (tx, rx) = futures_channel::oneshot::channel();
                doc.worker.preflight(standard, move |findings| {
                    let _ = tx.send(findings);
                });
                let id = doc.id;
                Task::perform(rx, move |findings| Message::Preflighted {
                    doc: id,
                    standard,
                    findings: findings.unwrap_or_default(),
                })
            }
            Message::Preflight(preflight::Event::RunAccessibility) => {
                self.all_tools = false;
                let Some(doc) = &self.doc else { return Task::none() };
                if self.dirty() {
                    doc.worker.set_changes(self.changes(), |_| {});
                }
                let (tx, rx) = futures_channel::oneshot::channel();
                doc.worker.accessibility(move |findings| {
                    let _ = tx.send(findings);
                });
                let id = doc.id;
                Task::perform(rx, move |findings| Message::AccessChecked {
                    doc: id,
                    findings: findings.unwrap_or_default(),
                })
            }
            Message::AccessChecked { doc, findings } => {
                if self.doc.as_ref().is_some_and(|d| d.id == doc) {
                    self.preflight = Some(preflight::Report::accessibility(findings));
                }
                Task::none()
            }
            Message::Preflighted {
                doc,
                standard,
                findings,
            } => {
                if self.doc.as_ref().is_some_and(|d| d.id == doc) {
                    self.preflight = Some(match standard {
                        panna_core::Standard::PdfA => preflight::Report::new(findings),
                        panna_core::Standard::PdfX => preflight::Report::print(findings),
                    });
                }
                Task::none()
            }
            Message::FieldsDetected { doc, result } => {
                if self.doc.as_ref().is_none_or(|d| d.id != doc) {
                    return Task::none();
                }
                match result {
                    Ok(add) if add.is_empty() => {
                        self.notice = Some("No more places for fields were found.".into());
                        Task::none()
                    }
                    Ok(add) => {
                        self.notice = Some(match add.len() {
                            1 => "Added 1 field. Undo it from the page tools if it isn't right.".into(),
                            n => format!(
                                "Added {n} fields. Undo them from the page tools if they aren't right."
                            ),
                        });
                        let show = add.first().map_or(0, |f| f.page);
                        self.edit_pages(
                            PageEdit::FormFields {
                                add,
                                remove: Vec::new(),
                            },
                            show,
                            false,
                        )
                    }
                    Err(e) => {
                        self.notice = Some(e);
                        Task::none()
                    }
                }
            }
            Message::PicturesFound { doc, page, boxes } => {
                if self.doc.as_ref().is_some_and(|d| d.id == doc) {
                    self.picture_editor.found(page, boxes);
                }
                Task::none()
            }
            Message::Redact(event) => {
                if self.saving || self.doc.is_none() {
                    return Task::none();
                }
                self.all_tools = false;
                match self.redactor.update(event) {
                    redact::Outcome::Nothing => {}
                    redact::Outcome::MarkResults => self.mark_find_results(),
                    redact::Outcome::Apply(areas) => {
                        let show = areas.first().map_or(0, |(page, _)| *page);
                        return self.edit_pages(PageEdit::Redact(areas), show, false);
                    }
                }
                if self.redactor.on {
                    self.preparer.stop();
                    self.picture_editor.stop();
                    self.adder.on = false;
                    self.adder.cancel();
                    self.measure.on = false;
                    self.measure.clear();
                    self.snapping = false;
                    self.snapshot = None;
                    self.selection = None;
                    if self.annotate.open {
                        self.annotate.update(annotate::Event::Toggle);
                    }
                }
                Task::none()
            }
            Message::AddPictureChosen(None) => {
                if self.picture_editor.replacing() {
                    self.picture_editor.picture(None);
                    return Task::none();
                }
                self.adder.cancel();
                Task::none()
            }
            Message::AddPictureChosen(Some(path)) => {
                let (tx, rx) = futures_channel::oneshot::channel();
                std::thread::spawn(move || {
                    let result = std::fs::read(&path)
                        .map_err(|e| format!("Couldn't open the picture: {e}"))
                        .and_then(|bytes| panna_core::StampImage::decode(&bytes));
                    let _ = tx.send(result);
                });
                Task::perform(rx, |result| {
                    Message::AddPictureLoaded(
                        result.unwrap_or_else(|_| Err("The picture couldn't be read.".into())),
                    )
                })
            }
            Message::AddPictureLoaded(result) => {
                let image = match result {
                    Ok(image) => Some(image),
                    Err(e) => {
                        self.notice = Some(e);
                        None
                    }
                };
                if self.picture_editor.replacing() {
                    let outcome = self.picture_editor.picture(image);
                    return self.after_pictures(outcome);
                }
                let outcome = self.adder.picture(image);
                self.after_adding(outcome)
            }
            Message::Measure(event) => {
                self.measure.update(event);
                if self.measure.on {
                    self.redactor.stop();
                    self.preparer.stop();
                    self.picture_editor.stop();
                    self.adder.on = false;
                    self.adder.cancel();
                    self.snapping = false;
                    self.snapshot = None;
                    self.selection = None;
                }
                Task::none()
            }
            Message::ToggleSnapshot => {
                self.redactor.stop();
                self.preparer.stop();
                self.picture_editor.stop();
                self.adder.on = false;
                self.adder.cancel();
                self.measure.on = false;
                self.measure.clear();
                self.snapping = !self.snapping;
                self.snapshot = None;
                Task::none()
            }
            Message::SnapshotTaken(result) => {
                let copied = result.and_then(|p| clipboard::copy_picture(p.width, p.height, p.rgba));
                match copied {
                    Ok(()) => {
                        if let Some(snap) = &mut self.snapshot {
                            snap.copied = true;
                        }
                    }
                    Err(e) => {
                        self.snapshot = None;
                        self.notice = Some(format!("Couldn't copy the picture: {e}"));
                    }
                }
                Task::none()
            }
            Message::PagePress(page) if self.annotate.open && !self.annotate.tool.marks_text() => {
                self.selection = None;
                self.annotate.unpick();
                if let Some((_, position)) = self.hover.filter(|(p, _)| *p == page) {
                    let at = self.page_map(page).to_page(position);
                    let outcome = self.annotate.press(page, at);
                    return self.after_annotate(outcome);
                }
                Task::none()
            }
            Message::PagePress(page) => {
                self.selecting = false;
                self.form.leave();
                // With the comment tools open, pressing on a comment selects it for moving.
                if self.annotate.open
                    && !self.saving
                    && let Some((_, position)) = self.hover.filter(|(p, _)| *p == page)
                {
                    let map = self.page_map(page);
                    let at = map.to_page(position);
                    let size = map.page_size();
                    if self.annotate.grab(page, at, size, HANDLE / self.zoom) {
                        self.selection = None;
                        return Task::none();
                    }
                }
                // Pressing a link follows it when the button comes up over it.
                self.pending_link = self.link_under(page);
                if self.pending_link.is_some() {
                    self.selection = None;
                    return Task::none();
                }
                let caret = self
                    .hover
                    .filter(|(p, _)| *p == page)
                    .and_then(|(_, position)| self.caret_at(page, position));
                self.selection = caret.map(Selection::at);
                self.selecting = caret.is_some();
                Task::none()
            }
            Message::PageDoubleClick(page) => {
                let Some((_, position)) = self.hover.filter(|(p, _)| *p == page) else {
                    return Task::none();
                };
                if self.measure.on {
                    self.measure.finish();
                    return Task::none();
                }
                // A double-click ends a polygon or cloud.
                if self.annotate.open && self.annotate.tool.by_corners() {
                    let outcome = self.annotate.update(annotate::Event::FinishShape);
                    return self.after_annotate(outcome);
                }
                // With the select tool, double-clicking a comment edits its text.
                if self.annotate.open
                    && self.annotate.tool == annotate::Tool::Select
                    && !self.saving
                    && self
                        .annotate
                        .edit_text(page, self.page_map(page).to_page(position))
                {
                    return operation::focus(annotate::editor_id());
                }
                let Some(text) = self.page_text(page) else {
                    return Task::none();
                };
                let map = self.page_map(page);
                let (x, y) = map.to_page(position);
                let Some(index) = text.char_at(x, y) else {
                    return Task::none();
                };
                let word = selection::word_at(text, index);
                self.selecting = false;
                self.selection = Some(Selection {
                    anchor: Caret {
                        page,
                        index: word.start,
                    },
                    focus: Caret {
                        page,
                        index: word.end,
                    },
                });
                self.copy_primary()
            }
            Message::PointerReleased => {
                if self.adder.on {
                    let outcome = self.adder.release();
                    return self.after_adding(outcome);
                }
                if self.redactor.on {
                    self.redactor.release();
                    return Task::none();
                }
                if self.picture_editor.on {
                    let outcome = self.picture_editor.release();
                    return self.after_pictures(outcome);
                }
                if self.preparer.on {
                    self.preparer.release(self.form.fields());
                    return self.preparer.focus().map_or_else(Task::none, operation::focus);
                }
                if self.snapshot.is_some_and(|s| s.dragging) {
                    return self.take_snapshot();
                }
                if let Some(index) = self.pending_link.take()
                    && let Some(page) = self.hover.map(|(p, _)| p)
                    && self.link_under(page) == Some(index)
                {
                    return self.follow_link(index);
                }
                if self.annotate.busy() {
                    let outcome = self.annotate.release();
                    if self.annotate.editing.is_some() {
                        return operation::focus(annotate::editor_id());
                    }
                    return self.after_annotate(outcome);
                }
                if !self.selecting {
                    return Task::none();
                }
                self.selecting = false;
                if self.selection.is_some_and(|s| s.is_empty()) {
                    self.selection = None;
                }
                if self.annotate.open && self.annotate.tool != annotate::Tool::Select {
                    return self.mark_selection();
                }
                self.copy_primary()
            }
            Message::Form(event) => {
                if !self.saving {
                    self.form.update(event);
                }
                Task::none()
            }
            Message::OpenLink(choice) => {
                let Some(uri) = self.confirm_link.take() else {
                    return Task::none();
                };
                match choice {
                    LinkChoice::Open => {
                        if let Err(e) = links::open(&uri) {
                            self.notice = Some(format!("Couldn't open the link: {e}"));
                        }
                        Task::none()
                    }
                    LinkChoice::Copy => iced::clipboard::write(uri),
                    LinkChoice::Cancel => Task::none(),
                }
            }
            Message::ResetForm => {
                self.confirm_reset = self.form.can_reset() && !self.saving;
                Task::none()
            }
            Message::ConfirmReset(reset) => {
                if std::mem::take(&mut self.confirm_reset) && reset {
                    self.form.reset();
                }
                Task::none()
            }
            Message::FocusNext => {
                self.form.leave();
                operation::focus_next()
            }
            Message::FocusPrevious => {
                self.form.leave();
                operation::focus_previous()
            }
            Message::Annotate(annotate::Event::Undo) if !self.annotate.can_undo() && self.page_edits > 0 => {
                self.page_tool(pages::Op::Undo)
            }
            Message::Annotate(event) => {
                // Choosing a markup tool with text selected marks it straight away.
                let marks_now = matches!(
                    event,
                    annotate::Event::Tool(
                        annotate::Tool::Highlight | annotate::Tool::Underline | annotate::Tool::StrikeOut
                    )
                ) && self.selection.is_some_and(|s| !s.is_empty());
                let opening = matches!(event, annotate::Event::Toggle);
                if self.saving && !opening {
                    return Task::none();
                }
                let was_editing = self.annotate.editing.is_some();
                let typing = matches!(event, annotate::Event::Sign(sign::Event::Mode(sign::Mode::Type)));
                let outcome = self.annotate.update(event);
                if typing {
                    return operation::focus(sign::input_id());
                }
                if marks_now {
                    return self.mark_selection();
                }
                if !was_editing && self.annotate.editing.is_some() {
                    return Task::batch([
                        self.after_annotate(outcome),
                        operation::focus(annotate::editor_id()),
                    ]);
                }
                self.after_annotate(outcome)
            }
            Message::AnnotationsApplied { doc, result } => {
                let Some(open) = self.doc.as_mut().filter(|d| d.id == doc) else {
                    return Task::none();
                };
                if let Err(e) = result {
                    self.notice = Some(e.to_string());
                    return Task::none();
                }
                // Re-render with the change; old bitmaps stay up until the new ones arrive.
                for slot in &mut open.pages {
                    slot.rendered_scale = None;
                    slot.pending_scale = None;
                }
                for thumb in &mut open.thumbs {
                    *thumb = Thumb::default();
                }
                Task::batch([self.request_visible(), self.request_thumbnails()])
            }
            Message::Pages(op) => self.page_tool(op),
            Message::InsertFileChosen(Some(path)) => {
                let at = self.current_page() + 1;
                self.edit_pages(PageEdit::InsertFile { at, path }, at, false)
            }
            Message::Extract(event) => self.extract(event),
            Message::Export(event) => self.export(event),
            Message::AllTools(show) => {
                self.all_tools = show;
                Task::none()
            }
            Message::Protect(event) => self.protect(event),
            Message::Signing(event) => self.signing(event),
            Message::Trust(event) => self.trust(event),
            Message::SignTo(None) => Task::none(),
            Message::SignTo(Some(path)) => {
                let (Some(doc), Some(state)) = (&self.doc, &self.signing) else {
                    return Task::none();
                };
                let Some((_, file)) = &state.file else {
                    return Task::none();
                };
                // Unsaved changes are signed too.
                if self.dirty() {
                    doc.worker.set_changes(self.changes(), |_| {});
                }
                let (tx, rx) = futures_channel::oneshot::channel();
                doc.worker.sign(
                    path.clone(),
                    Arc::clone(file),
                    state.password.clone(),
                    state.options(),
                    move |result| {
                        let _ = tx.send(result);
                    },
                );
                Task::perform(rx, move |result| Message::SignSaved {
                    path: path.clone(),
                    result: result.unwrap_or(Err(SaveError::EngineCrashed)),
                })
            }
            Message::SignSaved { path, result } => match result {
                Ok(()) => {
                    self.signing = None;
                    self.open(path, None)
                }
                Err(e) => {
                    match &mut self.signing {
                        Some(state) => state.error = Some(e.to_string()),
                        None => self.notice = Some(e.to_string()),
                    }
                    Task::none()
                }
            },
            Message::Clean(protect::CleanEvent::Open) => {
                if self.doc.is_some() {
                    self.cleaning = Some(protect::default_cleanup());
                }
                Task::none()
            }
            Message::Clean(event) => {
                let Some(state) = &mut self.cleaning else {
                    return Task::none();
                };
                match event {
                    protect::CleanEvent::Open => {}
                    protect::CleanEvent::Metadata(v) => state.metadata = v,
                    protect::CleanEvent::Scripts(v) => state.scripts = v,
                    protect::CleanEvent::Attachments(v) => state.attachments = v,
                    protect::CleanEvent::Comments(v) => state.comments = v,
                    protect::CleanEvent::Cancel => self.cleaning = None,
                    protect::CleanEvent::Go => {
                        let clean = *state;
                        self.cleaning = None;
                        if self.pages_busy || self.saving {
                            return Task::none();
                        }
                        let current = self.current_page();
                        return self.edit_pages(PageEdit::Clean(clean), current, false);
                    }
                }
                Task::none()
            }
            Message::Flatten(event) => {
                let Some(state) = &mut self.flattening else {
                    return Task::none();
                };
                match event {
                    pages::FlattenEvent::Comments(v) => state.comments = v,
                    pages::FlattenEvent::Fields(v) => state.fields = v,
                    pages::FlattenEvent::Cancel => self.flattening = None,
                    pages::FlattenEvent::Go => {
                        let pages::Flattening { comments, fields } = *state;
                        self.flattening = None;
                        if !(comments || fields) || self.pages_busy || self.saving {
                            return Task::none();
                        }
                        let current = self.current_page();
                        return self.edit_pages(PageEdit::Flatten { comments, fields }, current, false);
                    }
                }
                Task::none()
            }
            Message::FromPictures => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Make a PDF from pictures")
                        .add_filter(
                            "Pictures",
                            &[
                                "png", "PNG", "jpg", "JPG", "jpeg", "JPEG", "tif", "TIF", "tiff", "TIFF",
                            ],
                        )
                        .pick_files()
                        .await
                        .map(|files| files.iter().map(|f| f.path().to_path_buf()).collect())
                },
                Message::PicturesChosen,
            ),
            Message::PicturesChosen(Some(mut files)) if !files.is_empty() => {
                // In name order, as scans and photos are usually numbered.
                files.sort();
                let name = files
                    .first()
                    .and_then(|f| f.file_stem())
                    .map_or_else(|| "pictures".to_owned(), |s| s.to_string_lossy().into_owned());
                let dir = files
                    .first()
                    .and_then(|f| f.parent())
                    .map(std::path::Path::to_path_buf);
                self.pictures = files;
                Task::perform(
                    async move {
                        let mut dialog = rfd::AsyncFileDialog::new()
                            .set_title("Save the new PDF as")
                            .add_filter("PDF documents", &["pdf"])
                            .set_file_name(format!("{name}.pdf"));
                        if let Some(dir) = dir {
                            dialog = dialog.set_directory(dir);
                        }
                        dialog.save_file().await.map(|f| f.path().to_path_buf())
                    },
                    Message::PicturesSaveTo,
                )
            }
            Message::PicturesChosen(_) | Message::PicturesSaveTo(None) => {
                self.pictures.clear();
                Task::none()
            }
            Message::PicturesSaveTo(Some(target)) => {
                let files = std::mem::take(&mut self.pictures);
                self.notice = Some(format!(
                    "Making a PDF of {}…",
                    if files.len() == 1 {
                        "1 picture".into()
                    } else {
                        format!("{} pictures", files.len())
                    }
                ));
                Task::perform(make_pdf_from_pictures(files, target), Message::PicturesMade)
            }
            Message::PicturesMade(Ok(path)) => {
                self.notice = None;
                self.update(Message::FileChosen(Some(path)))
            }
            Message::PicturesMade(Err(e)) => {
                self.notice = Some(e);
                Task::none()
            }
            Message::ExportChosen(None) => {
                self.exporting = None;
                Task::none()
            }
            Message::ExportChosen(Some(target)) => {
                let Some(state) = self.exporting.take() else {
                    return Task::none();
                };
                let count = self.doc.as_ref().map_or(0, |d| d.pages.len());
                let Ok((pages, dpi)) = export::settings(&state, count) else {
                    return Task::none();
                };
                let pages: Vec<(usize, PathBuf)> = if state.format.one_file() {
                    pages.into_iter().map(|i| (i, target.clone())).collect()
                } else {
                    let stem = self.file_stem();
                    let ext = export::extension(state.format);
                    let files: Vec<_> = pages
                        .into_iter()
                        .map(|i| (i, target.join(format!("{stem} page {}.{ext}", i + 1))))
                        .collect();
                    if let Some((_, taken)) = files.iter().find(|(_, p)| p.exists()) {
                        self.notice = Some(format!(
                            "“{}” is already in that folder. Choose another folder, or move it first.",
                            taken.file_name().unwrap_or_default().to_string_lossy()
                        ));
                        return Task::none();
                    }
                    files
                };
                let Some(doc) = &self.doc else { return Task::none() };
                if self.dirty() {
                    doc.worker.set_changes(self.changes(), |_| {});
                }
                let (tx, rx) = futures_channel::oneshot::channel();
                doc.worker.export(
                    panna_core::Export {
                        format: state.format,
                        dpi,
                        pages,
                    },
                    move |result| {
                        let _ = tx.send(result);
                    },
                );
                Task::perform(rx, |result| {
                    Message::Extracted(result.unwrap_or(Err(SaveError::EngineCrashed)))
                })
            }
            Message::Watermark(event) => {
                let Some(state) = &mut self.watermarking else {
                    return Task::none();
                };
                let clean = |v: String, max: usize| v.chars().filter(|c| !c.is_control()).take(max).collect();
                match event {
                    pages::WatermarkEvent::Text(v) => state.text = clean(v, 200),
                    pages::WatermarkEvent::Ink(i) => state.ink = i,
                    pages::WatermarkEvent::Size(v) => state.size = clean(v, 6),
                    pages::WatermarkEvent::Angle(v) => state.angle = clean(v, 6),
                    pages::WatermarkEvent::Opacity(v) => state.opacity = v,
                    pages::WatermarkEvent::Behind(v) => state.behind = v,
                    pages::WatermarkEvent::UsePaper(on) => {
                        state.paper = on.then_some(state.paper.unwrap_or(0))
                    }
                    pages::WatermarkEvent::Paper(i) => state.paper = Some(i),
                    pages::WatermarkEvent::Range(v) => state.range = clean(v, 200),
                    pages::WatermarkEvent::Scale(v) => state.scale = v,
                    pages::WatermarkEvent::RemovePicture => state.picture = None,
                    pages::WatermarkEvent::ChoosePicture => {
                        return Task::perform(
                            async {
                                rfd::AsyncFileDialog::new()
                                    .set_title("Choose a picture for the watermark")
                                    .add_filter(
                                        "Pictures",
                                        &[
                                            "png", "PNG", "jpg", "JPG", "jpeg", "JPEG", "tif", "TIF", "tiff",
                                            "TIFF",
                                        ],
                                    )
                                    .pick_file()
                                    .await
                                    .map(|f| f.path().to_path_buf())
                            },
                            |path| Message::Watermark(pages::WatermarkEvent::PictureChosen(path)),
                        );
                    }
                    pages::WatermarkEvent::PictureChosen(None) => {}
                    pages::WatermarkEvent::PictureChosen(Some(path)) => {
                        let (tx, rx) = futures_channel::oneshot::channel();
                        std::thread::spawn(move || {
                            let name = path.file_name().map_or_else(
                                || path.display().to_string(),
                                |n| n.to_string_lossy().into_owned(),
                            );
                            let result = std::fs::read(&path)
                                .map_err(|e| format!("Couldn't open the picture: {e}"))
                                .and_then(|bytes| panna_core::StampImage::decode(&bytes))
                                .map(|image| (name, image));
                            let _ = tx.send(result);
                        });
                        return Task::perform(rx, |result| {
                            Message::Watermark(pages::WatermarkEvent::PictureLoaded(
                                result.unwrap_or_else(|_| Err("The picture couldn't be read.".into())),
                            ))
                        });
                    }
                    pages::WatermarkEvent::PictureLoaded(Ok(picture)) => state.picture = Some(picture),
                    pages::WatermarkEvent::PictureLoaded(Err(e)) => self.notice = Some(e),
                    pages::WatermarkEvent::Cancel => self.watermarking = None,
                    pages::WatermarkEvent::Go => {
                        let count = self.doc.as_ref().map_or(0, |d| d.pages.len());
                        let Ok((pages, mark)) = pages::watermark_settings(state, count) else {
                            return Task::none();
                        };
                        self.watermarking = None;
                        if self.pages_busy || self.saving {
                            return Task::none();
                        }
                        let current = self.current_page();
                        return self.edit_pages(PageEdit::Watermark { pages, mark }, current, false);
                    }
                }
                Task::none()
            }
            Message::Mark(event) => {
                let Some(state) = &mut self.marking else {
                    return Task::none();
                };
                let clean = |v: String, max: usize| v.chars().filter(|c| !c.is_control()).take(max).collect();
                match event {
                    pages::MarkEvent::Text(i, value) => {
                        if let Some(slot) = state.text.get_mut(i) {
                            *slot = clean(value, 200);
                        }
                    }
                    pages::MarkEvent::Size(v) => state.size = clean(v, 6),
                    pages::MarkEvent::Margin(v) => state.margin = clean(v, 6),
                    pages::MarkEvent::First(v) => state.first = clean(v, 9),
                    pages::MarkEvent::Range(v) => state.range = clean(v, 200),
                    pages::MarkEvent::Cancel => self.marking = None,
                    pages::MarkEvent::Go => {
                        let count = self.doc.as_ref().map_or(0, |d| d.pages.len());
                        let Ok((pages, marks)) = pages::mark_settings(state, count) else {
                            return Task::none();
                        };
                        self.marking = None;
                        if self.pages_busy || self.saving {
                            return Task::none();
                        }
                        let current = self.current_page();
                        return self.edit_pages(PageEdit::HeaderFooter { pages, marks }, current, false);
                    }
                }
                Task::none()
            }
            Message::Crop(event) => {
                let Some(state) = &mut self.cropping else {
                    return Task::none();
                };
                match event {
                    pages::CropEvent::Margin(i, value) => {
                        if let Some(slot) = state.margins.get_mut(i) {
                            *slot = value.chars().filter(|c| !c.is_control()).take(8).collect();
                        }
                    }
                    pages::CropEvent::All(all) => state.all = all,
                    pages::CropEvent::Cancel => self.cropping = None,
                    pages::CropEvent::Go => {
                        let Ok(margins) = pages::crop_margins(state) else {
                            return Task::none();
                        };
                        let all = state.all;
                        let current = self.current_page();
                        let pages = if all {
                            (0..self.doc.as_ref().map_or(0, |d| d.pages.len())).collect()
                        } else {
                            vec![current]
                        };
                        self.cropping = None;
                        if self.pages_busy || self.saving {
                            return Task::none();
                        }
                        return self.edit_pages(PageEdit::Crop { pages, margins }, current, false);
                    }
                }
                Task::none()
            }
            Message::ExtractChosen(Some(path)) => {
                let pages = std::mem::take(&mut self.extract_pages)
                    .into_iter()
                    .map(|index| panna_core::PageSource::Page {
                        doc: 0,
                        index,
                        turns: 0,
                    })
                    .collect();
                self.write_extracted(vec![(pages, path)])
            }
            Message::ExtractFolder(Some(dir)) => {
                let stem = self.file_stem();
                let mut files: Vec<(Vec<panna_core::PageSource>, PathBuf)> = Vec::new();
                for index in std::mem::take(&mut self.extract_pages) {
                    let path = dir.join(format!("{stem} page {}.pdf", index + 1));
                    if files.iter().any(|(_, p)| *p == path) {
                        continue;
                    }
                    if path.exists() {
                        self.notice = Some(format!(
                            "“{}” is already in that folder. Choose another folder, or move it first.",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ));
                        return Task::none();
                    }
                    files.push((
                        vec![panna_core::PageSource::Page {
                            doc: 0,
                            index,
                            turns: 0,
                        }],
                        path,
                    ));
                }
                self.write_extracted(files)
            }
            Message::InsertFileChosen(None) | Message::ExtractChosen(None) | Message::ExtractFolder(None) => {
                Task::none()
            }
            Message::StampPicture => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Stamp a picture")
                        .add_filter(
                            "Pictures",
                            &[
                                "png", "PNG", "jpg", "JPG", "jpeg", "JPEG", "tif", "TIF", "tiff", "TIFF",
                            ],
                        )
                        .pick_file()
                        .await
                        .map(|f| f.path().to_path_buf())
                },
                Message::StampPictureChosen,
            ),
            Message::StampPictureChosen(None) => Task::none(),
            Message::StampPictureChosen(Some(path)) => {
                let (tx, rx) = futures_channel::oneshot::channel();
                std::thread::spawn(move || {
                    let result = std::fs::read(&path)
                        .map_err(|e| format!("Couldn't open the picture: {e}"))
                        .and_then(|bytes| panna_core::StampImage::decode(&bytes))
                        .map(|image| {
                            let name = path
                                .file_stem()
                                .map_or_else(|| "Image".into(), |s| s.to_string_lossy().into_owned());
                            (image, name)
                        });
                    let _ = tx.send(result);
                });
                Task::perform(rx, |result| {
                    Message::StampPictureLoaded(
                        result.unwrap_or_else(|_| Err("The picture couldn't be read.".into())),
                    )
                })
            }
            Message::StampPictureLoaded(Err(e)) => {
                self.notice = Some(e);
                Task::none()
            }
            Message::StampPictureLoaded(Ok((image, name))) => {
                self.update(Message::Annotate(annotate::Event::StampPicture(image, name)))
            }
            Message::Print => {
                let Some(doc) = &self.doc else { return Task::none() };
                if self.printing {
                    return Task::none();
                }
                self.printing = true;
                if self.dirty() {
                    doc.worker.set_changes(self.changes(), |_| {});
                }
                let (tx, rx) = futures_channel::oneshot::channel();
                doc.worker.print_copy(move |result| {
                    let _ = tx.send(result);
                });
                Task::perform(rx, |result| {
                    Message::PrintCopy(result.unwrap_or(Err(SaveError::EngineCrashed)))
                })
            }
            Message::PrintCopy(result) => {
                self.printing = false;
                match result {
                    Ok(pdf) => {
                        let title = self.title().trim_end_matches(" — Panna").to_owned();
                        print::start(pdf, title, self.current_page())
                    }
                    Err(e) => {
                        self.notice = Some(e.to_string());
                        Task::none()
                    }
                }
            }
            Message::Printed(result) => {
                if let Err(e) = result {
                    self.notice = Some(e);
                }
                Task::none()
            }
            Message::Extracted(result) => {
                if let Err(e) = result {
                    self.notice = Some(e.to_string());
                }
                Task::none()
            }
            Message::SaveAttachment(index) => {
                let Some(file) = self.doc.as_ref().and_then(|d| d.info.attachments.get(index)) else {
                    return Task::none();
                };
                let name = file.name.clone();
                Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Save attached file as")
                            .set_file_name(name)
                            .save_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    move |path| Message::AttachmentChosen(index, path),
                )
            }
            Message::AttachmentChosen(_, None) => Task::none(),
            Message::AttachmentChosen(index, Some(path)) => {
                let Some(doc) = &self.doc else { return Task::none() };
                let (tx, rx) = futures_channel::oneshot::channel();
                doc.worker.save_attachment(index, path, move |result| {
                    let _ = tx.send(result);
                });
                Task::perform(rx, |result| {
                    Message::Extracted(result.unwrap_or(Err(SaveError::EngineCrashed)))
                })
            }
            Message::PagesChanged {
                doc,
                show,
                undo,
                result,
            } => {
                self.pages_busy = false;
                let Some(open) = self.doc.as_mut().filter(|d| d.id == doc) else {
                    return Task::none();
                };
                let info = match result {
                    Ok(info) => info,
                    Err(e) => {
                        self.notice = Some(format!("The pages couldn't be changed: {e}"));
                        return Task::none();
                    }
                };
                self.page_edits = if undo {
                    self.page_edits.saturating_sub(1)
                } else {
                    self.page_edits + 1
                };
                // Unsaved comments and form entries are now part of the document.
                let sizes = rotated_sizes(&info.pages, self.rotation);
                open.layout = PageLayout::new(&sizes, self.settings.spread);
                open.pages = info.pages.iter().map(|_| Slot::default()).collect();
                open.thumb_layout = ThumbLayout::new(&sizes);
                open.thumbs = info.pages.iter().map(|_| Thumb::default()).collect();
                open.texts = info.pages.iter().map(|_| TextSlot::default()).collect();
                // Layers start as the document has them again, as their ids may have changed.
                open.worker.set_layers(None);
                open.layers_shown = info.layers.iter().map(|l| l.shown).collect();
                open.info = info;
                self.selection = None;
                self.hover = None;
                // Marks to redact, and differences found, were for the pages as they were.
                self.redactor.stop();
                self.picture_editor.reload();
                self.comparison = None;
                self.annotate.saved(&open.info.comments);
                self.form.load(&open.info.fields);
                self.find.rescan(&open.texts);
                Task::batch([
                    self.ask_pictures(show),
                    self.go_to_page(show),
                    self.request_thumbnails(),
                    self.request_search_text(),
                    self.request_comment_text(),
                ])
            }
            Message::Save => {
                let Some(path) = self.doc.as_ref().map(|d| d.path.clone()) else {
                    return Task::none();
                };
                self.save_to(path)
            }
            Message::SaveAs => {
                if self.doc.is_none() {
                    return Task::none();
                }
                let name = self
                    .doc
                    .as_ref()
                    .and_then(|d| d.path.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "document.pdf".into());
                Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Save PDF as")
                            .add_filter("PDF documents", &["pdf"])
                            .set_file_name(name)
                            .save_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::SaveAsChosen,
                )
            }
            Message::SaveAsChosen(Some(path)) => self.save_to(path),
            Message::SaveAsChosen(None) => {
                self.after_save = None;
                Task::none()
            }
            Message::Saved { doc, path, result } => {
                self.saving = false;
                let Some(open) = self.doc.as_mut().filter(|d| d.id == doc) else {
                    return Task::none();
                };
                match result {
                    Ok(comments) => {
                        self.page_edits = 0;
                        open.path = path.clone();
                        // Saved comments are listed from the file, so new ones can now be
                        // removed like any other.
                        open.info.comments = comments;
                        self.annotate.saved(&open.info.comments);
                        self.form.saved();
                        self.settings.add_recent(path);
                        self.settings.save();
                        match self.after_save.take() {
                            Some(next) => self.proceed(next),
                            None => Task::none(),
                        }
                    }
                    Err(e) => {
                        self.after_save = None;
                        self.notice = Some(e.to_string());
                        Task::none()
                    }
                }
            }
            Message::CloseRequested(id) => {
                if self.dirty() {
                    self.confirm = Some(Pending::Close);
                    return Task::none();
                }
                // Another tab with unsaved changes is shown and asked about first.
                if let Some(i) = self.parked.iter().position(|t| t.dirty) {
                    let tab = if i < self.active_tab { i } else { i + 1 };
                    let shown = self.switch_tab(tab);
                    self.confirm = Some(Pending::Close);
                    return shown;
                }
                self.reader.stop();
                window::close(id)
            }
            Message::SwitchTab(tab) => self.switch_tab(tab),
            Message::NextTab(forward) => {
                let count = self.parked.len() + 1;
                if count < 2 {
                    return Task::none();
                }
                let tab = if forward {
                    (self.active_tab + 1) % count
                } else {
                    (self.active_tab + count - 1) % count
                };
                self.switch_tab(tab)
            }
            Message::CloseTabAt(tab) => {
                if tab == self.active_tab {
                    return self.update(Message::CloseTab);
                }
                let shown = self.switch_tab(tab);
                if self.active_tab == tab {
                    return Task::batch([shown, self.update(Message::CloseTab)]);
                }
                shown
            }
            Message::CloseTab => {
                if self.doc.is_none() || self.saving || self.pages_busy || self.printing {
                    return Task::none();
                }
                if self.dirty() {
                    self.confirm = Some(Pending::CloseTab);
                    return Task::none();
                }
                self.close_tab()
            }
            Message::Confirm(choice) => {
                let Some(pending) = self.confirm.take() else {
                    return Task::none();
                };
                match choice {
                    Choice::Cancel => Task::none(),
                    Choice::Discard => {
                        self.page_edits = 0;
                        self.annotate
                            .saved(self.doc.as_ref().map_or(&[], |d| d.info.comments.as_slice()));
                        self.form.discard();
                        self.proceed(pending)
                    }
                    Choice::Save => {
                        self.after_save = Some(pending);
                        self.update(Message::Save)
                    }
                }
            }
            Message::DismissNotice => {
                self.notice = None;
                Task::none()
            }
            Message::Copy => match self.selected_text() {
                Some(text) => iced::clipboard::write(text),
                None => Task::none(),
            },
        }
    }

    /// Whether there are annotations that haven't been saved.
    fn dirty(&self) -> bool {
        self.doc.is_some() && (self.annotate.dirty() || self.form.dirty() || self.page_edits > 0)
    }

    fn page_tool(&mut self, op: pages::Op) -> Task<Message> {
        let Some(doc) = &self.doc else { return Task::none() };
        if self.pages_busy || self.saving {
            return Task::none();
        }
        let current = self.current_page();
        match op {
            pages::Op::Undo if self.page_edits > 0 => self.edit_pages(PageEdit::Undo, current, true),
            pages::Op::Undo => Task::none(),
            pages::Op::InsertFile => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Insert pages from")
                        .add_filter("PDF documents", &["pdf", "PDF"])
                        .pick_file()
                        .await
                        .map(|f| f.path().to_path_buf())
                },
                Message::InsertFileChosen,
            ),
            pages::Op::Flatten => {
                self.flattening = Some(pages::Flattening {
                    comments: true,
                    fields: true,
                });
                Task::none()
            }
            pages::Op::Watermark => {
                let count = self.doc.as_ref().map_or(0, |d| d.pages.len());
                self.watermarking = Some(pages::Watermarking::new(count));
                operation::focus(pages::watermark_input_id())
            }
            pages::Op::HeaderFooter => {
                let count = self.doc.as_ref().map_or(0, |d| d.pages.len());
                self.marking = Some(pages::Marking::new(count));
                operation::focus(pages::mark_input_id())
                    .chain(operation::move_cursor_to_end(pages::mark_input_id()))
            }
            pages::Op::Crop => {
                self.cropping = Some(pages::Cropping::default());
                operation::focus(pages::crop_input_id()).chain(operation::select_all(pages::crop_input_id()))
            }
            pages::Op::Extract => {
                self.extracting = Some(pages::Extracting {
                    range: (current + 1).to_string(),
                    split: false,
                });
                operation::focus(pages::extract_input_id())
                    .chain(operation::select_all(pages::extract_input_id()))
            }
            _ => {
                let size = doc.info.pages.get(current).copied().unwrap_or(PageSize {
                    width: 612.0,
                    height: 792.0,
                });
                match pages::plan(op, doc.pages.len(), current, size) {
                    Some((plan, show)) => self.edit_pages(PageEdit::Plan(plan), show, false),
                    None => Task::none(),
                }
            }
        }
    }

    /// The document's file name without its extension, for naming files made from it.
    fn file_stem(&self) -> String {
        self.doc
            .as_ref()
            .and_then(|d| d.path.file_stem())
            .map_or_else(|| "document".into(), |s| s.to_string_lossy().into_owned())
    }

    fn extract(&mut self, event: pages::ExtractEvent) -> Task<Message> {
        let Some(state) = &mut self.extracting else {
            return Task::none();
        };
        match event {
            pages::ExtractEvent::Range(range) => state.range = range,
            pages::ExtractEvent::Split(split) => state.split = split,
            pages::ExtractEvent::Cancel => self.extracting = None,
            pages::ExtractEvent::Go => {
                let count = self.doc.as_ref().map_or(0, |d| d.pages.len());
                let Ok(pages) = pages::parse_range(&state.range, count) else {
                    return Task::none();
                };
                let split = state.split;
                let label = if pages.len() == 1 {
                    format!("page {}", pages.first().map_or(1, |p| p + 1))
                } else {
                    format!("pages {}", state.range.split_whitespace().collect::<String>())
                };
                self.extracting = None;
                self.extract_pages = pages;
                let name = format!("{} {label}.pdf", self.file_stem());
                return if split {
                    Task::perform(
                        async {
                            rfd::AsyncFileDialog::new()
                                .set_title("Save each page in")
                                .pick_folder()
                                .await
                                .map(|f| f.path().to_path_buf())
                        },
                        Message::ExtractFolder,
                    )
                } else {
                    Task::perform(
                        async move {
                            rfd::AsyncFileDialog::new()
                                .set_title("Save pages as")
                                .add_filter("PDF documents", &["pdf"])
                                .set_file_name(name)
                                .save_file()
                                .await
                                .map(|f| f.path().to_path_buf())
                        },
                        Message::ExtractChosen,
                    )
                };
            }
        }
        Task::none()
    }

    /// Writes extracted pages, with unsaved changes, each page list to its file.
    fn write_extracted(&mut self, files: Vec<(Vec<panna_core::PageSource>, PathBuf)>) -> Task<Message> {
        let Some(doc) = &self.doc else { return Task::none() };
        if files.is_empty() {
            return Task::none();
        }
        if self.dirty() {
            doc.worker.set_changes(self.changes(), |_| {});
        }
        let (tx, rx) = futures_channel::oneshot::channel();
        doc.worker.extract(files, move |result| {
            let _ = tx.send(result);
        });
        Task::perform(rx, |result| {
            Message::Extracted(result.unwrap_or(Err(SaveError::EngineCrashed)))
        })
    }

    /// Trusting a certificate, or no longer trusting it.
    fn trust(&mut self, event: trust::Event) -> Task<Message> {
        let result = match event {
            trust::Event::Add(cert) => self.trusted.add(cert),
            trust::Event::Remove(fingerprint) => self.trusted.remove(&fingerprint),
            trust::Event::Import => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Choose a certificate to trust")
                            .add_filter(
                                "Certificates",
                                &["cer", "crt", "pem", "der", "CER", "CRT", "PEM", "DER"],
                            )
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    |path| Message::Trust(trust::Event::Imported(path)),
                );
            }
            trust::Event::Imported(None) => Ok(()),
            trust::Event::Imported(Some(path)) => trust::read(&path)
                .and_then(|bytes| panna_core::Cert::from_file(&bytes))
                .and_then(|cert| self.trusted.add(cert)),
        };
        if let Err(e) = result {
            self.notice = Some(format!("Couldn't change the trusted certificates: {e}"));
        }
        Task::none()
    }

    /// The Sign with a certificate card's events.
    fn signing(&mut self, event: signing::Event) -> Task<Message> {
        if let signing::Event::Open = event {
            let page = self.current_page();
            let Some(doc) = &self.doc else { return Task::none() };
            self.signing = Some(signing::Signing {
                can_certify: doc.info.signatures.is_empty(),
                page,
                ..signing::Signing::default()
            });
            return Task::none();
        }
        let Some(state) = &mut self.signing else {
            return Task::none();
        };
        let clean = |v: String| v.chars().filter(|c| !c.is_control()).take(200).collect();
        match event {
            signing::Event::Open => Task::none(),
            signing::Event::Choose => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Choose a certificate file")
                        .add_filter("Certificate files", &["p12", "pfx", "P12", "PFX"])
                        .pick_file()
                        .await
                        .map(|f| f.path().to_path_buf())
                },
                |path| Message::Signing(signing::Event::Chosen(path)),
            ),
            signing::Event::Chosen(None) => Task::none(),
            signing::Event::Chosen(Some(path)) => {
                let read = std::fs::metadata(&path).map_err(|e| e.to_string()).and_then(|m| {
                    if m.len() > signing::MAX_FILE {
                        Err("That file is too large to be a certificate file.".into())
                    } else {
                        std::fs::read(&path).map_err(|e| e.to_string())
                    }
                });
                match read {
                    Ok(bytes) => {
                        let name = path.file_name().map_or_else(
                            || path.display().to_string(),
                            |n| n.to_string_lossy().into_owned(),
                        );
                        state.file = Some((name, Arc::new(bytes)));
                        state.error = None;
                        iced::widget::operation::focus(signing::input_id())
                    }
                    Err(e) => {
                        state.error = Some(format!("Couldn't read the certificate file: {e}"));
                        Task::none()
                    }
                }
            }
            signing::Event::Password(v) => {
                state.password = clean(v);
                Task::none()
            }
            signing::Event::Reason(v) => {
                state.reason = clean(v);
                Task::none()
            }
            signing::Event::Location(v) => {
                state.location = clean(v);
                Task::none()
            }
            signing::Event::Certify(v) => {
                state.certify = v;
                Task::none()
            }
            signing::Event::Shown(v) => {
                state.shown = v;
                Task::none()
            }
            signing::Event::Corner(v) => {
                state.corner = v;
                Task::none()
            }
            signing::Event::Cancel => {
                self.signing = None;
                Task::none()
            }
            signing::Event::Go => {
                if state.file.is_none() {
                    return Task::none();
                }
                state.error = None;
                let name = format!("{} (signed).pdf", self.file_stem());
                Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Save the signed copy as")
                            .add_filter("PDF", &["pdf"])
                            .set_file_name(name)
                            .save_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::SignTo,
                )
            }
        }
    }

    /// The Password protection card's events.
    fn protect(&mut self, event: protect::Event) -> Task<Message> {
        if let protect::Event::Open = event {
            let Some(doc) = &self.doc else { return Task::none() };
            self.protecting = Some(protect::Protecting::new(doc.info.protected));
            return iced::widget::operation::focus(protect::input_id());
        }
        let Some(state) = &mut self.protecting else {
            return Task::none();
        };
        let clean = |v: String| v.chars().filter(|c| !c.is_control()).take(127).collect();
        let protection = match event {
            protect::Event::Open => return Task::none(),
            protect::Event::OpenPassword(v) => {
                state.open = clean(v);
                return Task::none();
            }
            protect::Event::OpenAgain(v) => {
                state.open_again = clean(v);
                return Task::none();
            }
            protect::Event::Restrict(v) => {
                state.restrict = v;
                return Task::none();
            }
            protect::Event::Owner(v) => {
                state.owner = clean(v);
                return Task::none();
            }
            protect::Event::OwnerAgain(v) => {
                state.owner_again = clean(v);
                return Task::none();
            }
            protect::Event::Print(v) => {
                state.print = v;
                return Task::none();
            }
            protect::Event::Change(v) => {
                state.change = v;
                return Task::none();
            }
            protect::Event::Copy(v) => {
                state.copy = v;
                return Task::none();
            }
            protect::Event::Cancel => {
                self.protecting = None;
                return Task::none();
            }
            protect::Event::Remove => None,
            protect::Event::Go => match protect::settings(state) {
                Ok(p) => Some(p),
                Err(_) => return Task::none(),
            },
        };
        self.protecting = None;
        if self.pages_busy || self.saving {
            return Task::none();
        }
        let current = self.current_page();
        self.edit_pages(PageEdit::Protect(protection), current, false)
    }

    /// The Export card's events.
    fn export(&mut self, event: export::Event) -> Task<Message> {
        let count = self.doc.as_ref().map_or(0, |d| d.pages.len());
        if let export::Event::Open = event {
            if count > 0 {
                self.exporting = Some(export::Exporting::new(count));
            }
            return iced::widget::operation::focus(export::input_id());
        }
        let Some(state) = &mut self.exporting else {
            return Task::none();
        };
        let clean = |v: String, max: usize| v.chars().filter(|c| !c.is_control()).take(max).collect();
        match event {
            export::Event::Open => {}
            export::Event::Format(f) => state.format = f,
            export::Event::Dpi(v) => state.dpi = clean(v, 6),
            export::Event::Range(v) => state.range = clean(v, 200),
            export::Event::Cancel => self.exporting = None,
            export::Event::Go => {
                if export::settings(state, count).is_err() {
                    return Task::none();
                }
                let format = state.format;
                let stem = self.file_stem();
                return if format.one_file() {
                    let (title, kind, ext) = if format == panna_core::ExportFormat::Word {
                        ("Export as a Word document", "Word documents", "docx")
                    } else {
                        ("Export text as", "Text files", "txt")
                    };
                    Task::perform(
                        async move {
                            rfd::AsyncFileDialog::new()
                                .set_title(title)
                                .add_filter(kind, &[ext])
                                .set_file_name(format!("{stem}.{ext}"))
                                .save_file()
                                .await
                                .map(|f| f.path().to_path_buf())
                        },
                        Message::ExportChosen,
                    )
                } else {
                    Task::perform(
                        async {
                            rfd::AsyncFileDialog::new()
                                .set_title("Save the pictures in")
                                .pick_folder()
                                .await
                                .map(|f| f.path().to_path_buf())
                        },
                        Message::ExportChosen,
                    )
                };
            }
        }
        Task::none()
    }

    fn compare_event(&mut self, event: compare::Event) -> Task<Message> {
        let spot = match event {
            compare::Event::Open => {
                self.all_tools = false;
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Compare with")
                            .add_filter("PDF documents", &["pdf", "PDF"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::CompareChosen,
                );
            }
            compare::Event::Close => {
                self.comparison = None;
                if self.sidebar.tab == sidebar::Tab::Changes {
                    self.sidebar.tab = sidebar::Tab::Thumbnails;
                }
                return Task::none();
            }
            compare::Event::Goto(index) => self.comparison.as_mut().and_then(|c| c.select(index)),
            compare::Event::Step(forward) => self.comparison.as_mut().and_then(|c| c.step(forward)),
        };
        let Some((page, range)) = spot else {
            return Task::none();
        };
        self.reveal_hit(&find::Hit { page, range })
    }

    /// Marks every match Find has found so far for redacting.
    fn mark_find_results(&mut self) {
        let Some(doc) = &self.doc else { return };
        let (mut page, mut rects) = (None, Vec::new());
        for hit in self.find.hits() {
            if page != Some(hit.page) {
                if let Some(p) = page {
                    self.redactor.mark(p, std::mem::take(&mut rects));
                }
                page = Some(hit.page);
            }
            if let Some(text) = doc.texts.get(hit.page).and_then(|t| t.text.as_ref()) {
                rects.extend(text.rects(hit.range.clone()));
            }
        }
        if let Some(p) = page {
            self.redactor.mark(p, rects);
        }
    }

    /// Carries out what adding to a page asked for: the change itself, a picture to choose,
    /// or the card's input to focus.
    fn after_adding(&mut self, outcome: add::Outcome) -> Task<Message> {
        match outcome {
            add::Outcome::Add(page, addition) => self.edit_pages(
                PageEdit::Add {
                    page,
                    additions: vec![addition],
                },
                page,
                false,
            ),
            add::Outcome::PickPicture => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Add a picture")
                        .add_filter(
                            "Pictures",
                            &[
                                "png", "PNG", "jpg", "JPG", "jpeg", "JPEG", "tif", "TIF", "tiff", "TIFF",
                            ],
                        )
                        .pick_file()
                        .await
                        .map(|f| f.path().to_path_buf())
                },
                Message::AddPictureChosen,
            ),
            add::Outcome::Nothing => self.adder.focus().map_or_else(Task::none, operation::focus),
        }
    }

    /// Reads aloud the page the reader is on, once its text is known.
    fn go_on_reading(&mut self) -> Task<Message> {
        let Some(doc) = &self.doc else { return Task::none() };
        let texts = &doc.texts;
        let result = self.reader.go_on(texts.len(), |i| {
            let slot = texts.get(i)?;
            slot.text.as_ref().map(|t| t.slice(0..t.len()))
        });
        match result {
            Ok((speech::Next::NeedText(page), _)) => self.request_text(page..page + 1),
            Ok((speech::Next::Show(page), task)) => {
                let task = task.unwrap_or_else(Task::none);
                if page == self.current_page() {
                    task
                } else {
                    Task::batch([task, self.go_to_page(page)])
                }
            }
            Ok((speech::Next::Nothing, task)) => task.unwrap_or_else(Task::none),
            Err(e) => {
                self.notice = Some(e);
                Task::none()
            }
        }
    }

    /// Asks the worker where page `page`'s pictures are, unless already asked.
    fn ask_pictures(&mut self, page: usize) -> Task<Message> {
        let Some(doc) = &self.doc else { return Task::none() };
        if page >= doc.pages.len() || !self.picture_editor.should_ask(page) {
            return Task::none();
        }
        let (tx, rx) = futures_channel::oneshot::channel();
        doc.worker.pictures(page, move |boxes| {
            let _ = tx.send(boxes);
        });
        let id = doc.id;
        Task::perform(rx, move |boxes| Message::PicturesFound {
            doc: id,
            page,
            boxes: boxes.unwrap_or_default(),
        })
    }

    fn after_pictures(&mut self, outcome: pictures::Outcome) -> Task<Message> {
        match outcome {
            pictures::Outcome::Nothing => Task::none(),
            pictures::Outcome::PickReplacement => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Replace the picture with")
                        .add_filter(
                            "Pictures",
                            &[
                                "png", "PNG", "jpg", "JPG", "jpeg", "JPEG", "tif", "TIF", "tiff", "TIFF",
                            ],
                        )
                        .pick_file()
                        .await
                        .map(|f| f.path().to_path_buf())
                },
                Message::AddPictureChosen,
            ),
            pictures::Outcome::Change { page, changes } => {
                self.edit_pages(PageEdit::Pictures { page, changes }, page, false)
            }
        }
    }

    /// Sends a page change to the worker, with unsaved comments and form entries first so
    /// they go along.
    fn edit_pages(&mut self, edit: PageEdit, show: usize, undo: bool) -> Task<Message> {
        let Some(doc) = &self.doc else { return Task::none() };
        if self.dirty() {
            doc.worker.set_changes(self.changes(), |_| {});
        }
        self.pages_busy = true;
        let (tx, rx) = futures_channel::oneshot::channel();
        doc.worker.edit_pages(edit, move |result| {
            let _ = tx.send(result);
        });
        let id = doc.id;
        Task::perform(rx, move |result| Message::PagesChanged {
            doc: id,
            show,
            undo,
            result: result.unwrap_or(Err(SaveError::EngineCrashed)),
        })
    }

    /// Carries out an action that was waiting on "Save changes?".
    fn proceed(&mut self, pending: Pending) -> Task<Message> {
        match pending {
            Pending::Close => {
                // Any other tab with unsaved changes is asked about in turn.
                self.doc = None;
                match self.parked.iter().position(|t| t.dirty) {
                    Some(i) => {
                        let tab = self.parked.remove(i);
                        self.active_tab = i;
                        let shown = self.unpark(tab);
                        self.confirm = Some(Pending::Close);
                        shown
                    }
                    None => {
                        self.reader.stop();
                        iced::exit()
                    }
                }
            }
            Pending::CloseTab => self.close_tab(),
        }
    }

    fn save_to(&mut self, path: PathBuf) -> Task<Message> {
        let Some(doc) = &self.doc else { return Task::none() };
        if self.saving {
            return Task::none();
        }
        self.saving = true;
        // Field values reach the worker only now, since the inputs on the page already show
        // them; re-rendering every keystroke would be wasted work.
        if self.form.dirty() {
            doc.worker.set_changes(self.changes(), |_| {});
        }
        let (tx, rx) = futures_channel::oneshot::channel();
        doc.worker.save(path.clone(), move |result| {
            let _ = tx.send(result);
        });
        let id = doc.id;
        Task::perform(rx, move |result| Message::Saved {
            doc: id,
            path: path.clone(),
            result: result.unwrap_or(Err(SaveError::EngineCrashed)),
        })
    }

    /// The link under the pointer on `page`, unless a drawing tool is in use.
    fn link_under(&self, page: usize) -> Option<usize> {
        if self.annotate.open && self.annotate.tool != annotate::Tool::Select {
            return None;
        }
        let (_, position) = self.hover.filter(|(p, _)| *p == page)?;
        let doc = self.doc.as_ref()?;
        links::at(&doc.info.links, page, self.page_map(page).to_page(position))
    }

    fn follow_link(&mut self, index: usize) -> Task<Message> {
        let Some(target) = self
            .doc
            .as_ref()
            .and_then(|d| d.info.links.get(index))
            .map(|l| l.target.clone())
        else {
            return Task::none();
        };
        match target {
            LinkTarget::Page(page) => self.go_to_page(page),
            LinkTarget::Uri(uri) => {
                self.confirm_link = Some(uri);
                Task::none()
            }
        }
    }

    /// Everything not yet saved: comments, and filled-in fields.
    fn changes(&self) -> Changes {
        Changes {
            fields: self.form.changes(),
            ..self.annotate.changes()
        }
    }

    /// Sends the current annotation list to the worker after a change.
    fn after_annotate(&mut self, outcome: annotate::Outcome) -> Task<Message> {
        if !matches!(outcome, annotate::Outcome::Changed) {
            return Task::none();
        }
        let Some(doc) = &self.doc else { return Task::none() };
        let (tx, rx) = futures_channel::oneshot::channel();
        doc.worker.set_changes(self.changes(), move |result| {
            let _ = tx.send(result);
        });
        let id = doc.id;
        Task::perform(rx, move |result| match result {
            Ok(result) => Message::AnnotationsApplied { doc: id, result },
            // Superseded by a newer list.
            Err(_) => Message::Ignore,
        })
    }

    /// Highlights, underlines or strikes through the selected text with the current tool.
    fn mark_selection(&mut self) -> Task<Message> {
        let Some((selection, doc)) = self.selection.zip(self.doc.as_ref()) else {
            return Task::none();
        };
        let per_page = selection
            .pages()
            .filter_map(|page| {
                let text = doc.texts.get(page)?.text.as_ref()?;
                let range = selection.range_on(page, text.len())?;
                Some((page, text.rects(range)))
            })
            .collect();
        self.selection = None;
        let outcome = self.annotate.mark_text(per_page);
        self.after_annotate(outcome)
    }

    fn selected_text(&self) -> Option<String> {
        let doc = self.doc.as_ref()?;
        let text = self.selection.filter(|s| !s.is_empty())?.text(&doc.texts);
        (!text.is_empty()).then_some(text)
    }

    /// Copies a picture of the area dragged out with the snapshot tool, as shown and with
    /// unsaved changes, drawn sharper than the screen when the page is small enough.
    fn take_snapshot(&mut self) -> Task<Message> {
        let Some(snap) = self.snapshot.as_mut() else {
            return Task::none();
        };
        snap.dragging = false;
        let snap = *snap;
        let (x, y, w, h) = snap.rect();
        // A click rather than a drag.
        if w * self.zoom < 4.0 || h * self.zoom < 4.0 {
            self.snapshot = None;
            return Task::none();
        }
        let Some(doc) = &self.doc else { return Task::none() };
        let Some(size) = doc.info.pages.get(snap.page).copied() else {
            return Task::none();
        };
        let (vw, vh) = if self.rotation % 2 == 1 {
            (size.height, size.width)
        } else {
            (size.width, size.height)
        };
        if vw <= 0.0 || vh <= 0.0 {
            return Task::none();
        }
        let area = [x / vw, y / vh, (x + w) / vw, (y + h) / vh];
        // At least 144 dpi, or the screen's sharpness, but never a page over 40 megapixels.
        let scale = (self.zoom * self.scale_factor)
            .max(2.0)
            .min((40.0e6 / (vw * vh)).sqrt());
        if self.dirty() {
            doc.worker.set_changes(self.changes(), |_| {});
        }
        let (tx, rx) = futures_channel::oneshot::channel();
        doc.worker
            .snapshot(snap.page, scale, self.rotation, area, move |result| {
                let _ = tx.send(result.map_err(|e| e.to_string()));
            });
        Task::perform(rx, |result| {
            Message::SnapshotTaken(result.unwrap_or_else(|_| Err("the page couldn't be drawn".into())))
        })
    }

    /// On Linux, selected text also goes to the primary selection for middle-click paste.
    fn copy_primary(&self) -> Task<Message> {
        match self.selected_text() {
            Some(text) if cfg!(target_os = "linux") => iced::clipboard::write_primary(text),
            _ => Task::none(),
        }
    }

    fn page_text(&self, page: usize) -> Option<&PageText> {
        self.doc.as_ref()?.texts.get(page)?.text.as_deref()
    }

    fn page_map(&self, page: usize) -> PageMap {
        let size = self
            .doc
            .as_ref()
            .and_then(|d| d.info.pages.get(page).copied())
            .unwrap_or(PageSize {
                width: 1.0,
                height: 1.0,
            });
        PageMap {
            size,
            rotation: self.rotation,
            zoom: self.zoom,
        }
    }

    fn caret_at(&self, page: usize, position: Point) -> Option<Caret> {
        let (x, y) = self.page_map(page).to_page(position);
        let index = self.page_text(page)?.caret_at(x, y)?;
        Some(Caret { page, index })
    }

    fn find_event(&mut self, event: find::Event) -> Task<Message> {
        match event {
            find::Event::Open => {
                // Find lives in the navigation pane, as in Word: Ctrl+F opens the pane.
                self.find.open = true;
                if !self.sidebar.open {
                    self.sidebar.open = true;
                    self.save_sidebar();
                }
                // Start with the selected word, as most viewers do.
                if let Some(text) = self
                    .selected_text()
                    .filter(|t| !t.contains('\n') && t.len() < 100)
                {
                    self.find.query = text;
                }
                self.rescan_find();
                Task::batch([
                    operation::focus(find::input_id()),
                    operation::select_all(find::input_id()),
                    self.request_search_text(),
                    self.step_find(true),
                ])
            }
            find::Event::Close => {
                self.find.open = false;
                self.find.query.clear();
                self.rescan_find();
                Task::none()
            }
            find::Event::Query(query) => {
                self.find.open = true;
                self.find.query = query;
                if self.find.active() && self.sidebar.tab != sidebar::Tab::Results {
                    // Typing a search shows the results list, like Word.
                    self.sidebar.tab = sidebar::Tab::Results;
                    self.save_sidebar();
                }
                self.rescan_find();
                Task::batch([self.request_search_text(), self.step_find(true)])
            }
            find::Event::MatchCase | find::Event::WholeWord => {
                self.find.update_options(&event);
                self.rescan_find();
                self.step_find(true)
            }
            find::Event::Next => {
                if !self.find.open {
                    return self.find_event(find::Event::Open);
                }
                self.step_find(true)
            }
            find::Event::Prev => {
                if !self.find.open {
                    return self.find_event(find::Event::Open);
                }
                self.step_find(false)
            }
            find::Event::Goto(index) => match self.find.select(index).cloned() {
                Some(hit) => self.reveal_hit(&hit),
                None => Task::none(),
            },
        }
    }

    fn save_sidebar(&mut self) {
        self.settings.sidebar_open = self.sidebar.open;
        self.settings.sidebar_side = self.sidebar.side;
        self.settings.sidebar_tab = self.sidebar.tab;
        self.settings.save();
    }

    fn rescan_find(&mut self) {
        match &self.doc {
            Some(doc) => self.find.rescan(&doc.texts),
            None => self.find.rescan(&[]),
        }
    }

    /// Moves to the next or previous match and scrolls it into view.
    fn step_find(&mut self, forward: bool) -> Task<Message> {
        let from = self.current_page();
        match self.find.step(forward, from).cloned() {
            Some(hit) => self.reveal_hit(&hit),
            None => Task::none(),
        }
    }

    /// Scrolls a search match into view unless it is already comfortably on screen.
    fn reveal_hit(&mut self, hit: &find::Hit) -> Task<Message> {
        let Some(text) = self.page_text(hit.page) else {
            return Task::none();
        };
        let Some(first) = text.rects(hit.range.clone()).first().copied() else {
            return self.go_to_page(hit.page);
        };
        self.reveal(hit.page, first)
    }

    /// Scrolls a spot on a page (in page points) into view unless it is already on screen.
    fn reveal(&mut self, page: usize, spot: TextBox) -> Task<Message> {
        let Some(doc) = &self.doc else { return Task::none() };
        let (x, y, _, _) = self.page_map(page).rect(&spot);
        let content_w = doc.layout.content_width(self.zoom, self.viewport.width);
        let page_left = doc.layout.page_left(page, self.zoom, content_w);
        let top = doc.layout.page_top(page, self.zoom) + PAGE_GAP + y;
        // Leave the match where it is if it is already comfortably on screen.
        let on_screen = top > self.viewport.offset_y + 40.0
            && top < self.viewport.offset_y + self.viewport.height - 60.0
            && page_left + x > self.viewport.offset_x
            && page_left + x < self.viewport.offset_x + self.viewport.width - 40.0;
        if on_screen {
            return Task::none();
        }
        let target_y = (top - self.viewport.height / 3.0).max(0.0);
        let target_x = (page_left + x - self.viewport.width / 3.0).max(0.0);
        let target_x = if content_w <= self.viewport.width {
            0.0
        } else {
            target_x
        };
        self.viewport.offset_y = target_y;
        self.viewport.offset_x = target_x;
        Task::batch([
            operation::scroll_to(
                scroll_id(),
                AbsoluteOffset {
                    x: target_x,
                    y: target_y,
                },
            ),
            self.request_visible(),
        ])
    }

    /// Requests text for every page not yet extracted, while the find bar has a query.
    fn request_search_text(&mut self) -> Task<Message> {
        if !self.find.active() {
            return Task::none();
        }
        let count = self.doc.as_ref().map_or(0, |d| d.texts.len());
        self.request_text(0..count)
    }

    fn request_text(&mut self, pages: std::ops::Range<usize>) -> Task<Message> {
        let Some(doc) = &mut self.doc else {
            return Task::none();
        };
        let mut tasks = Vec::new();
        for index in pages {
            let Some(slot) = doc.texts.get_mut(index) else {
                break;
            };
            if slot.text.is_some() || slot.pending {
                continue;
            }
            slot.pending = true;
            let (tx, rx) = futures_channel::oneshot::channel();
            doc.worker.extract_text(index, move |result| {
                let _ = tx.send(result);
            });
            let doc_id = doc.id;
            tasks.push(Task::perform(rx, move |result| match result {
                Ok(result) => Message::TextExtracted {
                    doc: doc_id,
                    index,
                    result: result.map(Arc::new),
                },
                // Dropped unanswered (superseded); allow a retry.
                Err(_) => Message::TextExtracted {
                    doc: doc_id,
                    index,
                    result: Err(RenderError::EngineCrashed),
                },
            }));
        }
        Task::batch(tasks)
    }

    /// Takes the shown document out of view, with how it was being viewed, ending any tool
    /// in use on it.
    fn park(&mut self) -> Option<Parked> {
        let dirty = self.dirty();
        let doc = self.doc.take()?;
        self.measure.on = false;
        self.measure.clear();
        self.adder.on = false;
        self.adder.cancel();
        self.redactor.stop();
        self.preparer.stop();
        self.picture_editor.stop();
        self.reader.stop();
        self.preflight = None;
        self.snapping = false;
        self.snapshot = None;
        self.selection = None;
        self.hover = None;
        self.show_properties = false;
        Some(Parked {
            doc,
            zoom: self.zoom,
            rotation: self.rotation,
            offset: (self.viewport.offset_x, self.viewport.offset_y),
            find: std::mem::take(&mut self.find),
            annotate: std::mem::replace(&mut self.annotate, annotate::Annotate::new()),
            form: std::mem::take(&mut self.form),
            comparison: self.comparison.take(),
            page_edits: std::mem::take(&mut self.page_edits),
            dirty,
        })
    }

    /// Shows a parked document again where it was left.
    fn unpark(&mut self, tab: Parked) -> Task<Message> {
        let mut doc = tab.doc;
        // Replies for it while it was out of view were dropped, so it asks again.
        for slot in &mut doc.pages {
            slot.pending_scale = None;
        }
        for thumb in &mut doc.thumbs {
            thumb.pending = false;
        }
        for text in &mut doc.texts {
            text.pending = false;
        }
        self.doc = Some(doc);
        self.zoom = tab.zoom;
        self.rotation = tab.rotation;
        self.viewport.offset_x = tab.offset.0;
        self.viewport.offset_y = tab.offset.1;
        self.find = tab.find;
        self.annotate = tab.annotate;
        self.form = tab.form;
        self.comparison = tab.comparison;
        self.page_edits = tab.page_edits;
        self.status = Status::Idle;
        self.page_input = (self.current_page() + 1).to_string();
        Task::batch([
            operation::scroll_to(
                scroll_id(),
                AbsoluteOffset {
                    x: tab.offset.0,
                    y: tab.offset.1,
                },
            ),
            self.request_visible(),
            self.request_thumbnails(),
            self.request_search_text(),
        ])
    }

    /// Shows tab `tab` (counting the shown one).
    fn switch_tab(&mut self, tab: usize) -> Task<Message> {
        if tab == self.active_tab
            || tab > self.parked.len()
            || self.saving
            || self.pages_busy
            || self.printing
        {
            return Task::none();
        }
        let wanted = if tab < self.active_tab { tab } else { tab - 1 };
        if wanted >= self.parked.len() {
            return Task::none();
        }
        let next = self.parked.remove(wanted);
        if let Some(current) = self.park() {
            self.parked
                .insert(self.active_tab.min(self.parked.len()), current);
        }
        self.active_tab = tab;
        self.unpark(next)
    }

    /// Closes the shown document, showing the next tab (or the start screen).
    fn close_tab(&mut self) -> Task<Message> {
        let _ = self.park();
        self.find = find::Find::default();
        self.annotate = annotate::Annotate::new();
        self.form = form::Form::default();
        self.comparison = None;
        self.page_edits = 0;
        self.status = Status::Idle;
        if self.parked.is_empty() {
            self.active_tab = 0;
            return Task::none();
        }
        let at = self.active_tab.min(self.parked.len() - 1);
        let next = self.parked.remove(at);
        self.active_tab = at;
        self.unpark(next)
    }

    /// The row of tabs, when more than one document is open.
    fn tab_bar(&self) -> Element<'_, Message> {
        if self.parked.is_empty() {
            return space().height(0).into();
        }
        let shown = self
            .doc
            .as_ref()
            .map(tab_name)
            .unwrap_or_else(|| match &self.status {
                Status::Loading(path) | Status::Password { path, .. } => path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                _ => "New tab".to_owned(),
            });
        let mut names: Vec<(String, bool)> = self.parked.iter().map(|t| (t.name(), t.dirty)).collect();
        names.insert(self.active_tab.min(names.len()), (shown, self.dirty()));
        let tabs = names.into_iter().enumerate().map(|(i, (name, dirty))| {
            let active = i == self.active_tab;
            let label = if dirty { format!("{name} •") } else { name };
            let tab = row![
                button(text(label).size(13).wrapping(text::Wrapping::None))
                    .padding([4, 8])
                    .style(button::text)
                    .on_press(Message::SwitchTab(i)),
                icons::button(
                    Icon::Close,
                    "Close this document (Ctrl+W)",
                    Some(Message::CloseTabAt(i))
                ),
            ]
            .align_y(Center);
            container(tab)
                .max_width(240)
                .clip(true)
                .style(move |theme: &iced::Theme| {
                    let palette = theme.extended_palette();
                    let style = container::Style::default().border(iced::Border {
                        color: palette.background.strong.color,
                        width: 1.0,
                        radius: 6.0.into(),
                    });
                    if active {
                        style.background(palette.background.base.color)
                    } else {
                        style.background(palette.background.weak.color)
                    }
                })
                .into()
        });
        container(scrollable(row(tabs).spacing(4).padding([4, 8])).direction(
            scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(4).scroller_width(4)),
        ))
        .width(Fill)
        .into()
    }

    fn open(&mut self, path: PathBuf, password: Option<String>) -> Task<Message> {
        // A file already open in a tab is shown rather than opened again.
        if password.is_none() {
            if self
                .doc
                .as_ref()
                .is_some_and(|d| d.path == path && !d.info.pages.is_empty())
            {
                return Task::none();
            }
            if let Some(i) = self.parked.iter().position(|t| t.doc.path == path) {
                let tab = if i < self.active_tab { i } else { i + 1 };
                return self.switch_tab(tab);
            }
        }
        // The document shown keeps its tab; the new one opens in a tab after it.
        if matches!(self.status, Status::Idle)
            && self.doc.as_ref().is_some_and(|d| !d.info.pages.is_empty())
            && let Some(tab) = self.park()
        {
            self.parked.insert(self.active_tab.min(self.parked.len()), tab);
            self.active_tab += 1;
        }
        let id = self.next_doc_id;
        self.next_doc_id += 1;
        self.show_properties = false;
        let used_password = password.is_some();
        let (tx, rx) = futures_channel::oneshot::channel();
        let worker = match RenderWorker::open(path.clone(), password, Limits::default(), move |result| {
            let _ = tx.send(result);
        }) {
            Ok(worker) => worker,
            Err(e) => {
                self.status = Status::Error(format!("Couldn't start the renderer: {e}"));
                return Task::none();
            }
        };
        self.status = Status::Loading(path.clone());
        self.comparison = None;
        self.redactor.stop();
        self.doc = Some(OpenDoc {
            id,
            path,
            used_password,
            info: DocumentInfo {
                pages: Vec::new(),
                title: None,
                properties: Vec::new(),
                outline: Vec::new(),
                comments: Vec::new(),
                fields: Vec::new(),
                links: Vec::new(),
                protected: false,
                attachments: Vec::new(),
                layers: Vec::new(),
                signatures: Vec::new(),
            },
            worker,
            layout: PageLayout::default(),
            pages: Vec::new(),
            thumb_layout: ThumbLayout::default(),
            thumbs: Vec::new(),
            texts: Vec::new(),
            layers_shown: Vec::new(),
        });
        Task::perform(rx, move |result| {
            Message::Opened(id, result.unwrap_or(Err(OpenError::EngineCrashed)))
        })
    }

    fn set_zoom(&mut self, zoom: f32) -> Task<Message> {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let Some(doc) = &self.doc else {
            self.zoom = zoom;
            return Task::none();
        };
        // Keep the point at the top of the viewport in place while zooming.
        let anchor = doc.layout.unzoomed_y(self.viewport.offset_y, self.zoom);
        self.zoom = zoom;
        self.viewport.offset_y = doc.layout.zoomed_y(anchor, zoom);
        let scroll = operation::scroll_to(
            scroll_id(),
            AbsoluteOffset {
                x: 0.0,
                y: self.viewport.offset_y,
            },
        );
        Task::batch([scroll, self.request_visible()])
    }

    /// Turns the view clockwise by `turns` quarter turns, keeping the current page in view.
    /// Regroups pages into rows after the spread changes, staying on the current page.
    fn relayout(&mut self) -> Task<Message> {
        let page = self.current_page();
        let Some(doc) = &mut self.doc else {
            return Task::none();
        };
        let sizes = doc.layout.sizes().to_vec();
        doc.layout = PageLayout::new(&sizes, self.settings.spread);
        self.go_to_page(page)
    }

    fn rotate(&mut self, turns: u8) -> Task<Message> {
        let page = self.current_page();
        let Some(doc) = &mut self.doc else {
            return Task::none();
        };
        self.rotation = (self.rotation + turns) % 4;
        let sizes = rotated_sizes(&doc.info.pages, self.rotation);
        doc.layout = PageLayout::new(&sizes, self.settings.spread);
        doc.thumb_layout = ThumbLayout::new(&sizes);
        doc.pages.iter_mut().for_each(|slot| *slot = Slot::default());
        doc.thumbs.iter_mut().for_each(|thumb| *thumb = Thumb::default());
        Task::batch([self.go_to_page(page), self.request_thumbnails()])
    }

    /// Shows `index` fitted to the whole window, for presenting.
    fn present_page(&mut self, index: usize) -> Task<Message> {
        let Some(doc) = &self.doc else { return Task::none() };
        let index = index.min(doc.pages.len().saturating_sub(1));
        let Some(size) = doc.layout.sizes().get(index).copied() else {
            return Task::none();
        };
        if let Screen::Present { page, .. } = &mut self.screen {
            *page = index;
        }
        let fit = (self.window_size.width / size.width.max(1.0))
            .min(self.window_size.height / size.height.max(1.0));
        self.zoom = fit.clamp(MIN_ZOOM, MAX_ZOOM);
        self.go_to_page(index)
    }

    fn stop_presenting(&mut self) -> Task<Message> {
        let Screen::Present { page, zoom, was_full } = self.screen else {
            return Task::none();
        };
        self.screen = if was_full { Screen::Full } else { Screen::Windowed };
        self.zoom = zoom;
        let mode = if was_full {
            Task::none()
        } else {
            set_window_mode(window::Mode::Windowed)
        };
        Task::batch([mode, self.go_to_page(page)])
    }

    fn step_page(&mut self, forward: bool) -> Task<Message> {
        let page = self.current_page();
        let target = self.doc.as_ref().map_or(page, |d| d.layout.step(page, forward));
        self.go_to_page(target)
    }

    fn go_to_page(&mut self, index: usize) -> Task<Message> {
        let Some(doc) = &self.doc else { return Task::none() };
        if doc.pages.is_empty() {
            return Task::none();
        }
        let index = index.min(doc.pages.len() - 1);
        let y = doc.layout.page_top(index, self.zoom);
        self.viewport.offset_y = y;
        Task::batch([
            operation::scroll_to(scroll_id(), AbsoluteOffset { x: 0.0, y }),
            self.request_visible(),
        ])
    }

    fn current_page(&self) -> usize {
        self.doc.as_ref().map_or(0, |d| {
            d.layout
                .page_at(self.viewport.offset_y + self.viewport.height * 0.3, self.zoom)
        })
    }

    /// Requests sharp bitmaps for pages in view and drops bitmaps far outside it.
    fn request_visible(&mut self) -> Task<Message> {
        let Some(doc) = &mut self.doc else {
            return Task::none();
        };
        if doc.pages.is_empty() {
            return Task::none();
        }
        let visible = doc
            .layout
            .visible_range(self.viewport.offset_y, self.viewport.height, self.zoom);
        let keep_start = visible.start.saturating_sub(KEEP_MARGIN);
        let keep_end = (visible.end + KEEP_MARGIN).min(doc.pages.len());
        let scale = self.zoom * self.scale_factor;
        let mut tasks = Vec::new();
        let text_range = visible.clone();

        for (index, slot) in doc.pages.iter_mut().enumerate() {
            if index < keep_start || index >= keep_end {
                *slot = Slot::default();
                continue;
            }
            let fresh = slot.rendered_scale.is_some_and(|s| (s - scale).abs() < 0.01);
            let queued = slot.pending_scale.is_some_and(|s| (s - scale).abs() < 0.01);
            if fresh || queued || slot.error.is_some() {
                continue;
            }
            slot.pending_scale = Some(scale);
            let (tx, rx) = futures_channel::oneshot::channel();
            let rotation = self.rotation;
            let colors = self.settings.colors;
            doc.worker.render(index, scale, rotation, move |mut result| {
                if let Ok(page) = &mut result {
                    colors.apply(&mut page.rgba);
                }
                let _ = tx.send(result);
            });
            let doc_id = doc.id;
            // The task resolves when the worker answers. Superseded jobs drop their sender,
            // which resolves to Err and is ignored.
            tasks.push(Task::perform(rx, move |result| match result {
                Ok(result) => Message::Rendered {
                    doc: doc_id,
                    index,
                    scale,
                    rotation,
                    result,
                },
                Err(_) => Message::Ignore,
            }));
        }
        // Visible pages get their text too, so it's ready to select.
        tasks.push(self.request_text(text_range));
        Task::batch(tasks)
    }

    /// Requests thumbnails for the sidebar's visible range and drops ones far outside it.
    /// Loads the text of pages with highlights, so the comments list can quote what they mark.
    fn request_comment_text(&mut self) -> Task<Message> {
        if !(self.sidebar.open && self.sidebar.tab == sidebar::Tab::Comments) {
            return Task::none();
        }
        let Some(doc) = &self.doc else { return Task::none() };
        let mut pages: Vec<usize> = doc
            .info
            .comments
            .iter()
            .filter(|c| !c.marked.is_empty())
            .map(|c| c.page)
            .collect();
        pages.dedup();
        Task::batch(
            pages
                .into_iter()
                .map(|p| self.request_text(p..p + 1))
                .collect::<Vec<_>>(),
        )
    }

    fn request_thumbnails(&mut self) -> Task<Message> {
        if !self.sidebar.showing_thumbnails() {
            return Task::none();
        }
        let Some(doc) = &mut self.doc else {
            return Task::none();
        };
        let wanted = self.sidebar.wanted(&doc.thumb_layout);
        let rotation = self.rotation;
        let mut tasks = Vec::new();
        for (index, thumb) in doc.thumbs.iter_mut().enumerate() {
            if !wanted.contains(&index) {
                *thumb = Thumb::default();
                continue;
            }
            if thumb.image.is_some() || thumb.pending {
                continue;
            }
            let Some(size) = doc.layout.sizes().get(index) else {
                continue;
            };
            let scale = doc.thumb_layout.size(index).0 / size.width.max(1.0) * self.scale_factor;
            thumb.pending = true;
            let (tx, rx) = futures_channel::oneshot::channel();
            let colors = self.settings.colors;
            doc.worker
                .render_thumbnail(index, scale, rotation, move |mut result| {
                    if let Ok(page) = &mut result {
                        colors.apply(&mut page.rgba);
                    }
                    let _ = tx.send(result);
                });
            let doc_id = doc.id;
            tasks.push(Task::perform(rx, move |result| match result {
                Ok(result) => Message::ThumbRendered {
                    doc: doc_id,
                    index,
                    rotation,
                    result,
                },
                Err(_) => Message::Ignore,
            }));
        }
        Task::batch(tasks)
    }

    fn view(&self) -> Element<'_, Message> {
        if let Screen::Present { page, .. } = self.screen {
            return self.present_view(page);
        }
        let page_count = self.doc.as_ref().map_or(0, |d| d.pages.len());
        let has_doc = page_count > 0;
        let current = self.current_page() + 1;

        let page_box = text_input(&current.to_string(), &self.page_input)
            .id(page_box_id())
            .on_input(Message::PageInput)
            .on_submit(Message::PageSubmit)
            .width(56);

        let gap = || space().width(10).height(10).into();
        let on = |m: Message| has_doc.then_some(m);
        let m = icons::MOD;
        let items: Vec<Element<'_, Message>> = vec![
            icons::button(
                Icon::Sidebar,
                if self.sidebar.open {
                    "Hide sidebar (F4)"
                } else {
                    "Show sidebar (F4)"
                },
                on(Message::Sidebar(sidebar::Event::Toggle)),
            ),
            icons::button(Icon::Open, format!("Open… ({m}O)"), Some(Message::OpenDialog)),
            icons::button(
                Icon::Save,
                format!("Save ({m}S)"),
                (self.dirty() && !self.saving).then_some(Message::Save),
            ),
            icons::button(
                Icon::Print,
                format!("Print… ({m}P)"),
                (has_doc && !self.printing).then_some(Message::Print),
            ),
            icons::button(Icon::AllTools, "All tools…", Some(Message::AllTools(true))),
            gap(),
            icons::button(Icon::PrevPage, "Previous page (Page Up)", on(Message::PrevPage)),
            icons::with_tip(page_box, format!("Go to page ({m}Shift+N)")),
            text(format!("of {page_count}")).into(),
            icons::button(Icon::NextPage, "Next page (Page Down)", on(Message::NextPage)),
            gap(),
            icons::button(
                Icon::ZoomOut,
                format!("Zoom out ({m}Minus)"),
                on(Message::ZoomOut),
            ),
            icons::with_tip(
                button(text(format!("{:.0}%", self.zoom * 100.0)).size(13))
                    .style(button::text)
                    .on_press_maybe(on(Message::ZoomReset)),
                format!("Actual size ({m}1)"),
            ),
            icons::button(Icon::ZoomIn, format!("Zoom in ({m}Plus)"), on(Message::ZoomIn)),
            icons::button(Icon::FitWidth, format!("Fit width ({m}2)"), on(Message::FitWidth)),
            icons::button(Icon::FitPage, format!("Fit page ({m}0)"), on(Message::FitPage)),
            gap(),
            icons::button(Icon::RotateCcw, "Rotate left", on(Message::RotateCcw)),
            icons::button(Icon::RotateCw, "Rotate right", on(Message::RotateCw)),
            icons::button(
                match self.settings.spread {
                    Spread::Single => Icon::SinglePage,
                    Spread::Two => Icon::TwoPages,
                    Spread::TwoCover => Icon::CoverPage,
                },
                match self.settings.spread {
                    Spread::Single => format!("Single page: click for two pages side by side ({m}Shift+T)"),
                    Spread::Two => format!("Two pages: click to show the cover page alone ({m}Shift+T)"),
                    Spread::TwoCover => format!("Two pages with cover: click for single page ({m}Shift+T)"),
                },
                on(Message::NextSpread),
            ),
            icons::button(
                Icon::Present,
                format!("Present, one page at a time ({m}L)"),
                on(Message::Present),
            ),
            icons::toggle(
                Icon::FullScreen,
                if self.screen == Screen::Full {
                    "Leave full screen (F11)"
                } else {
                    "Full screen (F11)"
                },
                self.screen == Screen::Full,
                Message::ToggleFullScreen,
            ),
            icons::toggle(
                Icon::Moon,
                format!(
                    "Page colours: {} (click for {}, {m}Shift+D)",
                    self.settings.colors.label(),
                    self.settings.colors.next().label()
                ),
                self.settings.colors != colors::PageColors::Normal,
                Message::NextColors,
            ),
            gap(),
            icons::button(
                Icon::Search,
                format!("Find ({m}F)"),
                on(Message::Find(find::Event::Open)),
            ),
            icons::toggle(
                Icon::Annotate,
                if self.annotate.open {
                    format!("Hide comment tools ({m}Shift+C)")
                } else {
                    format!("Comment and draw ({m}Shift+C)")
                },
                self.annotate.open,
                Message::Annotate(annotate::Event::Toggle),
            ),
            icons::toggle(
                Icon::Ruler,
                if self.measure.on {
                    "Stop measuring (Esc)"
                } else {
                    "Measure distance, perimeter and area"
                },
                self.measure.on,
                Message::Measure(measure::Event::Toggle),
            ),
            icons::toggle(
                Icon::Camera,
                if self.snapping {
                    "Stop taking snapshots (Esc)"
                } else {
                    "Snapshot: drag over a page to copy a picture of it"
                },
                self.snapping,
                Message::ToggleSnapshot,
            ),
            icons::button(
                Icon::Eraser,
                "Reset form",
                self.form.can_reset().then_some(Message::ResetForm),
            ),
            gap(),
            icons::button(
                Icon::Properties,
                format!("Document properties ({m}D)"),
                on(Message::ToggleProperties),
            ),
        ];
        let toolbar = self.toolbar.view(items, Message::Toolbar);

        let body: Element<'_, Message> = match (&self.status, &self.doc) {
            (Status::Error(e), _) => center(
                column![
                    text(e.as_str()),
                    button("Open another file").on_press(Message::OpenDialog)
                ]
                .spacing(12)
                .align_x(Center),
            )
            .into(),
            (Status::Loading(path), _) => center(text(format!("Opening {}…", path.display()))).into(),
            (Status::Password { path, input, wrong }, _) => center(
                column![
                    text(format!(
                        "“{}” is password-protected.",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    )),
                    text(if *wrong {
                        "That password didn't work. Try again."
                    } else {
                        "Enter the password to open it."
                    }),
                    text_input("Password", input)
                        .id(password_id())
                        .secure(true)
                        .on_input(Message::PasswordInput)
                        .on_submit(Message::PasswordSubmit)
                        .width(280),
                    row![
                        button("Open").on_press(Message::PasswordSubmit),
                        button("Choose another file").on_press(Message::OpenDialog),
                    ]
                    .spacing(8),
                ]
                .spacing(12)
                .align_x(Center),
            )
            .into(),
            (Status::Idle, Some(doc)) if has_doc => {
                let pages = self.pages_view(doc);
                // The row is there either way, so opening the rail doesn't move the pages in
                // the widget tree and reset their scroll position.
                let rail: Element<'_, Message> = if self.annotate.open {
                    container(self.annotate.rail()).padding(6).into()
                } else {
                    space().into()
                };
                let pages = row![rail, pages].into();
                self.with_sidebar(doc, pages)
            }
            _ => self.welcome_view(),
        };
        // Overlays always go in one stack above the body, so opening or closing one doesn't
        // move the body in the widget tree and reset its scroll position.
        let body = column![self.tab_bar(), body].into();
        let mut layers = vec![body];
        if self.adder.on && self.doc.is_some() {
            layers.push(
                container(self.adder.bar())
                    .width(Fill)
                    .height(Fill)
                    .padding(16)
                    .align_x(Center)
                    .align_y(iced::Bottom)
                    .into(),
            );
        }
        if let Some(card) = self.adder.card() {
            layers.push(modal(card, Message::Add(add::Event::Cancel)));
        }
        if self.redactor.on && self.doc.is_some() {
            layers.push(
                container(self.redactor.bar(!self.find.hits().is_empty()))
                    .width(Fill)
                    .height(Fill)
                    .padding(16)
                    .align_x(Center)
                    .align_y(iced::Bottom)
                    .into(),
            );
        }
        if let Some(card) = self.redactor.card() {
            layers.push(modal(card, Message::Redact(redact::Event::Cancel)));
        }
        if self.preparer.on && self.doc.is_some() {
            layers.push(
                container(self.preparer.bar())
                    .width(Fill)
                    .height(Fill)
                    .padding(16)
                    .align_x(Center)
                    .align_y(iced::Bottom)
                    .into(),
            );
        }
        if let Some(bar) = self.reader.bar() {
            layers.push(
                container(bar)
                    .width(Fill)
                    .height(Fill)
                    .padding(16)
                    .align_x(iced::Right)
                    .align_y(iced::Bottom)
                    .into(),
            );
        }
        if self.picture_editor.on && self.doc.is_some() {
            layers.push(
                container(self.picture_editor.bar())
                    .width(Fill)
                    .height(Fill)
                    .padding(16)
                    .align_x(Center)
                    .align_y(iced::Bottom)
                    .into(),
            );
        }
        if let Some(report) = &self.preflight {
            layers.push(modal(report.card(), Message::Preflight(preflight::Event::Close)));
        }
        if let Some(card) = self.preparer.card() {
            layers.push(modal(card, Message::Prepare(prepare::Event::Cancel)));
        }
        if self.measure.on && self.doc.is_some() {
            layers.push(
                container(self.measure.bar())
                    .width(Fill)
                    .height(Fill)
                    .padding(16)
                    .align_x(Center)
                    .align_y(iced::Bottom)
                    .into(),
            );
        }
        if let (Some(doc), true) = (&self.doc, self.show_properties) {
            layers.push(properties_view(&doc.info));
        }
        if let Some(card) = self.annotate.editor() {
            layers.push(modal(card, Message::Annotate(annotate::Event::EditCancel)));
        }
        if let (Some(state), Some(doc)) = (&self.extracting, &self.doc) {
            layers.push(modal(
                pages::extract_card(state, doc.pages.len()),
                Message::Extract(pages::ExtractEvent::Cancel),
            ));
        }
        if let Some(state) = &self.cropping {
            layers.push(modal(
                pages::crop_card(state),
                Message::Crop(pages::CropEvent::Cancel),
            ));
        }
        if self.all_tools {
            layers.push(modal(
                tools::card(self.form.exists(), self.doc.is_some()),
                Message::AllTools(false),
            ));
        }
        if let Some(state) = self.cleaning {
            layers.push(modal(
                protect::clean_card(state),
                Message::Clean(protect::CleanEvent::Cancel),
            ));
        }
        if let Some(state) = &self.protecting {
            layers.push(modal(
                protect::card(state),
                Message::Protect(protect::Event::Cancel),
            ));
        }
        if let Some(state) = &self.signing {
            layers.push(modal(
                signing::card(state),
                Message::Signing(signing::Event::Cancel),
            ));
        }
        if let Some(state) = self.flattening {
            layers.push(modal(
                pages::flatten_card(state),
                Message::Flatten(pages::FlattenEvent::Cancel),
            ));
        }
        if let (Some(state), Some(doc)) = (&self.exporting, &self.doc) {
            layers.push(modal(
                export::card(state, doc.pages.len()),
                Message::Export(export::Event::Cancel),
            ));
        }
        if let (Some(state), Some(doc)) = (&self.watermarking, &self.doc) {
            layers.push(modal(
                pages::watermark_card(state, doc.pages.len()),
                Message::Watermark(pages::WatermarkEvent::Cancel),
            ));
        }
        if let (Some(state), Some(doc)) = (&self.marking, &self.doc) {
            layers.push(modal(
                pages::mark_card(state, doc.pages.len()),
                Message::Mark(pages::MarkEvent::Cancel),
            ));
        }
        if self.confirm_reset {
            layers.push(modal(reset_view(), Message::ConfirmReset(false)));
        }
        if let Some(uri) = &self.confirm_link {
            layers.push(modal(link_view(uri), Message::OpenLink(LinkChoice::Cancel)));
        }
        if self.confirm.is_some() {
            layers.push(modal(self.confirm_view(), Message::Confirm(Choice::Cancel)));
        }
        if let Some(notice) = &self.notice {
            layers.push(modal(
                container(
                    column![
                        text(notice.as_str()),
                        button("OK").on_press(Message::DismissNotice)
                    ]
                    .spacing(12)
                    .width(360),
                )
                .padding(16)
                .style(container::rounded_box)
                .into(),
                Message::DismissNotice,
            ));
        }
        let body: Element<'_, Message> = Stack::with_children(layers).into();

        match self.toolbar.dock {
            Dock::Top => column![toolbar, body].into(),
            Dock::Bottom => column![body, toolbar].into(),
            Dock::Left => row![toolbar, body].into(),
            Dock::Right => row![body, toolbar].into(),
            Dock::Floating => stack![
                body,
                pin(opaque(toolbar)).x(self.toolbar.pos.x).y(self.toolbar.pos.y)
            ]
            .into(),
        }
    }

    /// One page on black, nothing else. A click goes forward, a right-click back.
    fn present_view(&self, index: usize) -> Element<'_, Message> {
        let page = self.doc.as_ref().and_then(|doc| {
            let size = doc.layout.sizes().get(index)?;
            let handle = doc.pages.get(index)?.image.as_ref()?;
            Some(
                image(handle.clone())
                    .width((size.width * self.zoom).round())
                    .height((size.height * self.zoom).round()),
            )
        });
        let page: Element<'_, Message> = match page {
            Some(page) => page.into(),
            None => space().into(),
        };
        mouse_area(center(page).style(|_| container::Style::default().background(iced::Color::BLACK)))
            .on_press(Message::Advance(true))
            .on_right_press(Message::Advance(false))
            .into()
    }

    fn confirm_view(&self) -> Element<'_, Message> {
        let name = self
            .doc
            .as_ref()
            .and_then(|d| d.path.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        container(
            column![
                text(format!("Save your changes to “{name}”?")).size(16),
                text("Your comments, drawings and form entries will be lost if you don't save them.")
                    .style(text::secondary),
                row![
                    button("Don't save")
                        .style(button::secondary)
                        .on_press(Message::Confirm(Choice::Discard)),
                    space::horizontal(),
                    button("Cancel")
                        .style(button::secondary)
                        .on_press(Message::Confirm(Choice::Cancel)),
                    button("Save").on_press(Message::Confirm(Choice::Save)),
                ]
                .spacing(8),
            ]
            .spacing(12)
            .width(400),
        )
        .padding(18)
        .style(container::rounded_box)
        .into()
    }

    fn welcome_view(&self) -> Element<'_, Message> {
        let mut content = column![
            text("Open a PDF to get started").size(20),
            text("Use the Open button, press Ctrl+O, or drop a file on this window."),
        ]
        .spacing(8)
        .align_x(Center);
        if !self.settings.recent.is_empty() {
            content = content
                .push(space().height(16))
                .push(text("Recent files").size(16));
            for path in &self.settings.recent {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                content = content.push(
                    button(column![
                        text(name).size(15),
                        text(path.display().to_string()).size(11)
                    ])
                    .style(button::text)
                    .on_press(Message::FileChosen(Some(path.clone()))),
                );
            }
        }
        center(scrollable(content)).into()
    }

    fn with_sidebar<'a>(&'a self, doc: &'a OpenDoc, pages: Element<'a, Message>) -> Element<'a, Message> {
        // Closed, the bar is an empty space rather than missing, so toggling it doesn't move the
        // pages in the widget tree and reset their scroll position.
        let bar = if !self.sidebar.open {
            space().into()
        } else {
            self.sidebar.view(sidebar::ViewData {
                layout: &doc.thumb_layout,
                thumbs_images: &doc.thumbs,
                outline: &doc.info.outline,
                current: self.current_page(),
                find: &self.find,
                texts: &doc.texts,
                comments: &doc.info.comments,
                unsaved: &self.annotate.items,
                removed: &self.annotate.removed,
                attachments: &doc.info.attachments,
                signatures: &doc.info.signatures,
                trusted: &self.trusted.certs,
                layers: &doc.info.layers,
                layers_shown: &doc.layers_shown,
                can_undo_pages: self.page_edits > 0,
                pages_busy: self.pages_busy,
                comparison: self.comparison.as_ref(),
            })
        };
        match self.sidebar.side {
            sidebar::Side::Left => row![bar, pages].into(),
            sidebar::Side::Right => row![pages, bar].into(),
        }
    }

    /// Search-match and selection highlights for one page, as positioned layers.
    fn text_marks<'a>(&self, doc: &'a OpenDoc, index: usize) -> Vec<Element<'a, Message>> {
        let Some(text) = doc.texts.get(index).and_then(|t| t.text.as_ref()) else {
            return Vec::new();
        };
        let map = self.page_map(index);
        let mut out = Vec::new();
        let mut mark = |range: std::ops::Range<usize>, color: iced::Color| {
            for b in text.rects(range) {
                let (x, y, w, h) = map.rect(&b);
                out.push(
                    pin(container(space())
                        .width(w.max(1.0))
                        .height(h.max(1.0))
                        .style(move |_| container::Style::default().background(color)))
                    .x(x)
                    .y(y)
                    .into(),
                );
            }
        };
        if self.find.active() {
            let (hits, current) = self.find.hits_on(index);
            for (i, hit) in hits.iter().enumerate() {
                let color = if Some(i) == current {
                    iced::Color::from_rgba(1.0, 0.55, 0.0, 0.5)
                } else {
                    iced::Color::from_rgba(1.0, 0.85, 0.0, 0.4)
                };
                mark(hit.range.clone(), color);
            }
        }
        if let Some(range) = self.selection.and_then(|s| s.range_on(index, text.len())) {
            mark(range, iced::Color::from_rgba(0.2, 0.45, 0.95, 0.35));
        }
        if let Some(comparison) = &self.comparison {
            let (tints, bars) = comparison.marks(index, text);
            let bar = compare::Comparison::bar_color();
            for (b, color) in tints.into_iter().chain(bars.into_iter().map(|b| (b, bar))) {
                let (x, y, w, h) = map.rect(&b);
                out.push(
                    pin(container(space())
                        .width(w.max(2.0))
                        .height(h.max(1.0))
                        .style(move |_| container::Style::default().background(color)))
                    .x(x)
                    .y(y)
                    .into(),
                );
            }
        }
        out
    }

    fn pages_view<'a>(&'a self, doc: &'a OpenDoc) -> Element<'a, Message> {
        let pages = doc
            .layout
            .sizes()
            .iter()
            .zip(&doc.pages)
            .enumerate()
            .map(|(index, (size, slot))| {
                let width = Length::Fixed((size.width * self.zoom).round());
                let height = Length::Fixed((size.height * self.zoom).round());
                let content: Element<'_, Message> = match (&slot.image, &slot.error) {
                    (Some(handle), _) => {
                        let marks = self.text_marks(doc, index);
                        let preview = annotate::preview(
                            &self.annotate,
                            index,
                            self.page_map(index),
                            Size::new(size.width * self.zoom, size.height * self.zoom),
                        );
                        let outline = self.annotate.picked_outline(index).map(|b| {
                            let (x, y, w, h) = self.page_map(index).rect(&b);
                            Element::from(
                                pin(container(space()).width(w.max(1.0)).height(h.max(1.0)).style(
                                    |theme: &iced::Theme| {
                                        container::Style::default().border(iced::Border {
                                            color: theme.extended_palette().primary.base.color,
                                            width: 2.0,
                                            radius: 2.0.into(),
                                        })
                                    },
                                ))
                                .x(x)
                                .y(y),
                            )
                        });
                        let handles = self
                            .annotate
                            .resizable(index)
                            .filter(|_| !self.annotate.busy())
                            .map(|b| {
                                let map = self.page_map(index);
                                [(b.x0, b.y0), (b.x1, b.y0), (b.x0, b.y1), (b.x1, b.y1)].map(|(x, y)| {
                                    let (sx, sy, _, _) = map.rect(&TextBox {
                                        x0: x,
                                        y0: y,
                                        x1: x,
                                        y1: y,
                                    });
                                    Element::from(
                                        pin(container(space()).width(HANDLE).height(HANDLE).style(
                                            |theme: &iced::Theme| {
                                                let palette = theme.extended_palette();
                                                container::Style::default()
                                                    .background(iced::Color::WHITE)
                                                    .border(iced::Border {
                                                        color: palette.primary.base.color,
                                                        width: 1.5,
                                                        radius: 1.0.into(),
                                                    })
                                            },
                                        ))
                                        .x(sx - HANDLE / 2.0)
                                        .y(sy - HANDLE / 2.0),
                                    )
                                })
                            })
                            .into_iter()
                            .flatten();
                        let fields = if self.preparer.on {
                            self.preparer
                                .overlays(index, self.page_map(index), self.form.fields())
                        } else {
                            self.form.overlays(index, self.page_map(index))
                        };
                        let measured = self.measure.overlay(
                            index,
                            self.page_map(index),
                            Size::new(size.width * self.zoom, size.height * self.zoom),
                        );
                        let adding = self.adder.overlay(index, self.page_map(index));
                        let mut redacting = if self.redactor.on {
                            self.redactor.overlays(index, self.page_map(index))
                        } else {
                            Vec::new()
                        };
                        if self.picture_editor.on {
                            redacting.extend(self.picture_editor.overlays(index, self.page_map(index)));
                        }
                        let snapshot = self.snapshot.filter(|s| s.page == index).map(|s| {
                            let (x, y, w, h) = s.rect();
                            let zoom = self.zoom;
                            let label: Element<'_, Message> = if s.copied {
                                container(text("Copied").size(12))
                                    .padding([2, 6])
                                    .style(|theme: &iced::Theme| {
                                        let palette = theme.extended_palette();
                                        container::Style::default()
                                            .background(palette.primary.strong.color)
                                            .color(palette.primary.strong.text)
                                    })
                                    .into()
                            } else {
                                space().into()
                            };
                            Element::from(
                                pin(container(label)
                                    .width((w * zoom).max(1.0))
                                    .height((h * zoom).max(1.0))
                                    .style(|theme: &iced::Theme| {
                                        let palette = theme.extended_palette();
                                        container::Style::default()
                                            .background(palette.primary.base.color.scale_alpha(0.12))
                                            .border(iced::Border {
                                                color: palette.primary.base.color,
                                                width: 1.5,
                                                radius: 0.0.into(),
                                            })
                                    }))
                                .x(x * zoom)
                                .y(y * zoom),
                            )
                        });
                        if marks.is_empty()
                            && preview.is_none()
                            && outline.is_none()
                            && fields.is_empty()
                            && snapshot.is_none()
                            && measured.is_none()
                            && adding.is_none()
                            && redacting.is_empty()
                        {
                            image(handle.clone()).width(width).height(height).into()
                        } else {
                            let mut layers = vec![image(handle.clone()).width(width).height(height).into()];
                            layers.extend(marks);
                            layers.extend(preview);
                            layers.extend(outline);
                            layers.extend(handles);
                            layers.extend(fields);
                            layers.extend(snapshot);
                            layers.extend(measured);
                            layers.extend(adding);
                            layers.extend(redacting);
                            iced::widget::Stack::with_children(layers).into()
                        }
                    }
                    (None, Some(e)) => center(text(e.as_str())).into(),
                    (None, None) => space::horizontal().into(),
                };
                let over_text = self.hover.is_some_and(|(page, position)| {
                    page == index
                        && doc
                            .texts
                            .get(index)
                            .and_then(|t| t.text.as_ref())
                            .is_some_and(|t| {
                                let (x, y) = self.page_map(index).to_page(position);
                                t.hit(x, y)
                            })
                });
                let over_comment = self.annotate.open
                    && self.annotate.tool == annotate::Tool::Select
                    && self.hover.is_some_and(|(page, position)| {
                        page == index
                            && self
                                .annotate
                                .hit(index, self.page_map(index).to_page(position))
                                .is_some()
                    });
                let over_link = !self.selecting && self.link_under(index).is_some();
                // Over a corner handle: which way it stretches.
                let over_handle = self.hover.and_then(|(page, position)| {
                    if page != index || self.annotate.busy() && !self.annotate.is_resizing() {
                        return None;
                    }
                    let at = self.page_map(index).to_page(position);
                    let (ax, ay) = self.annotate.handle_at(index, at, HANDLE / self.zoom)?;
                    Some((at.0 - ax) * (at.1 - ay) > 0.0)
                });
                let content = mouse_area(content)
                    .on_move(move |p| Message::PagePointer(index, p))
                    .on_press(Message::PagePress(index))
                    .on_double_click(Message::PageDoubleClick(index))
                    .on_release(Message::PointerReleased)
                    .interaction(if self.picture_editor.on {
                        let slop = pictures::HANDLE / self.zoom.max(0.01);
                        let at_handle = self.hover.is_some_and(|(page, position)| {
                            page == index
                                && self.picture_editor.over_handle(
                                    page,
                                    self.page_map(page).to_page(position),
                                    slop,
                                )
                        });
                        if at_handle {
                            mouse::Interaction::ResizingDiagonallyDown
                        } else {
                            mouse::Interaction::Grab
                        }
                    } else if let Some(down) = over_handle {
                        if down {
                            mouse::Interaction::ResizingDiagonallyDown
                        } else {
                            mouse::Interaction::ResizingDiagonallyUp
                        }
                    } else if self.snapping
                        || self.measure.on
                        || self.adder.on
                        || self.redactor.on
                        || self.preparer.on
                    {
                        mouse::Interaction::Crosshair
                    } else if over_link {
                        mouse::Interaction::Pointer
                    } else if self.annotate.is_moving() {
                        mouse::Interaction::Grabbing
                    } else if over_comment {
                        mouse::Interaction::Grab
                    } else if self.annotate.open && !self.annotate.tool.marks_text() {
                        mouse::Interaction::Crosshair
                    } else if over_text || self.selecting {
                        mouse::Interaction::Text
                    } else {
                        mouse::Interaction::Idle
                    });
                container(content)
                    .width(width)
                    .height(height)
                    // A border rather than a drop shadow: the software renderer draws shadows
                    // over page images when an overlay such as the find bar is open.
                    .style(|_| {
                        container::Style::default()
                            .background(iced::Color::WHITE)
                            .border(iced::Border {
                                color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.25),
                                width: 1.0,
                                radius: 0.0.into(),
                            })
                    })
                    .into()
            });
        // Fixed content width: at least the viewport (so pages stay centred), wider when zoomed in.
        let content_width = doc.layout.content_width(self.zoom, self.viewport.width);
        let mut pages = pages;
        let rows = doc.layout.rows().iter().map(|row| {
            iced::widget::Row::with_children(pages.by_ref().take(row.len()))
                .spacing(PAGE_GAP)
                .align_y(iced::alignment::Vertical::Top)
                .into()
        });
        let rows: Vec<Element<'_, Message>> = rows.collect();
        let column = iced::widget::Column::with_children(rows)
            .spacing(PAGE_GAP)
            .padding(PAGE_GAP)
            .align_x(Center)
            .width(Length::Fixed(content_width.round()));
        let pages = scrollable(column)
            .id(scroll_id())
            .direction(scrollable::Direction::Both {
                vertical: scrollable::Scrollbar::default(),
                horizontal: scrollable::Scrollbar::default(),
            })
            .on_scroll(Message::Scrolled)
            .width(Fill)
            .height(Fill);
        // A darker backdrop so page edges stay visible in light and dark themes.
        container(pages)
            .style(|theme: &iced::Theme| {
                container::Style::default().background(theme.extended_palette().background.strong.color)
            })
            .into()
    }
}

fn set_window_mode(mode: window::Mode) -> Task<Message> {
    window::oldest().and_then(move |id| window::set_mode(id, mode))
}

/// Page sizes as shown, turned by the view's rotation.
fn rotated_sizes(pages: &[PageSize], rotation: u8) -> Vec<PageSize> {
    pages
        .iter()
        .map(|s| {
            if rotation % 2 == 1 {
                PageSize {
                    width: s.height,
                    height: s.width,
                }
            } else {
                *s
            }
        })
        .collect()
}
