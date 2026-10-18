//! Panna: an offline, memory-safe PDF viewer.

#![forbid(unsafe_code)]
// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod annotate;
mod comments;
mod find;
mod form;
mod icons;
mod layout;
mod links;
mod selection;
mod settings;
mod shortcuts;
mod sidebar;
mod toolbar;

use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::scrollable::{AbsoluteOffset, Viewport};
use iced::widget::{
    Stack, button, center, column, container, image, mouse_area, opaque, operation, pin, row, scrollable,
    space, stack, text, text_input,
};
use iced::{Center, Element, Fill, Length, Point, Size, Subscription, Task, event, mouse, window};
use panna_core::{
    Changes, Comment, DocumentInfo, Limits, LinkTarget, OpenError, PageSize, PageText, RenderError,
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
    RotateCcw,
    PrevPage,
    NextPage,
    PageInput(String),
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
    Confirm(Choice),
    DismissNotice,
    Ignore,
}

struct App {
    next_doc_id: u64,
    doc: Option<OpenDoc>,
    status: Status,
    zoom: f32,
    /// View rotation in clockwise quarter turns (0–3), applied to every page.
    rotation: u8,
    scale_factor: f32,
    viewport: ViewportState,
    page_input: String,
    settings: Settings,
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
    annotate: annotate::Annotate,
    form: form::Form,
    /// Asking whether to reset the form.
    confirm_reset: bool,
    /// A link pressed, followed when the button is released over it.
    pending_link: Option<usize>,
    /// A web address waiting for the user's go-ahead.
    confirm_link: Option<String>,
    saving: bool,
    /// What to do once unsaved changes are saved or discarded.
    confirm: Option<Pending>,
    after_save: Option<Pending>,
    /// A problem to show the user, such as a failed save.
    notice: Option<String>,
}

/// An action that would lose unsaved annotations.
#[derive(Debug, Clone)]
enum Pending {
    Close,
    Open(PathBuf),
    OpenDialog,
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
            window_size: INITIAL_SIZE,
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
            confirm: None,
            after_save: None,
            notice: None,
        };
        let scale = window::oldest()
            .and_then(window::scale_factor)
            .map(Message::ScaleFactor);
        let open = match std::env::args_os().nth(1) {
            Some(arg) => app.open(PathBuf::from(arg), None),
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
        match message {
            Message::OpenDialog if self.dirty() => {
                self.confirm = Some(Pending::OpenDialog);
                Task::none()
            }
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
            Message::FileChosen(Some(path)) => {
                if self.dirty() {
                    self.confirm = Some(Pending::Open(path));
                    return Task::none();
                }
                self.open(path, None)
            }
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
                        doc.info = info;
                        self.selection = None;
                        self.hover = None;
                        self.annotate.saved(&doc.info.comments);
                        self.form.load(&doc.info.fields);
                        self.find.rescan(&doc.texts);
                        self.status = Status::Idle;
                        self.zoom = 1.0;
                        self.viewport.offset_y = 0.0;
                        Task::batch([
                            operation::scroll_to(scroll_id(), AbsoluteOffset { x: 0.0, y: 0.0 }),
                            self.request_visible(),
                            self.request_thumbnails(),
                            self.request_search_text(),
                            self.request_comment_text(),
                        ])
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
            Message::NextSpread => {
                self.settings.spread = self.settings.spread.next();
                self.settings.save();
                self.relayout()
            }
            Message::RotateCcw => self.rotate(3),
            Message::PrevPage => self.step_page(false),
            Message::NextPage => self.step_page(true),
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
                match target {
                    Some(index) => self.go_to_page(index),
                    None => Task::none(),
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
                Task::none()
            }
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
                if self.confirm_link.is_some() {
                    self.confirm_link = None;
                } else if self.confirm_reset {
                    self.confirm_reset = false;
                } else if self.show_properties {
                    self.show_properties = false;
                } else if self.find.open {
                    self.find.open = false;
                    self.find.rescan(&[]);
                } else if self.annotate.draft.is_some() {
                    self.annotate.update(annotate::Event::CancelShape);
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
            Message::GoToPage(index) => self.go_to_page(index),
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
                if self.find.needs_first() {
                    return self.step_find(true);
                }
                Task::none()
            }
            Message::Find(event) => self.find_event(event),
            Message::PagePointer(page, position) => {
                self.hover = Some((page, position));
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
                let outcome = self.annotate.update(event);
                if marks_now {
                    return self.mark_selection();
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
                window::close(id)
            }
            Message::Confirm(choice) => {
                let Some(pending) = self.confirm.take() else {
                    return Task::none();
                };
                match choice {
                    Choice::Cancel => Task::none(),
                    Choice::Discard => {
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
        self.doc.is_some() && (self.annotate.dirty() || self.form.dirty())
    }

    /// Carries out an action that was waiting on "Save changes?".
    fn proceed(&mut self, pending: Pending) -> Task<Message> {
        match pending {
            Pending::Close => iced::exit(),
            Pending::Open(path) => self.open(path, None),
            Pending::OpenDialog => self.update(Message::OpenDialog),
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

    fn open(&mut self, path: PathBuf, password: Option<String>) -> Task<Message> {
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
            },
            worker,
            layout: PageLayout::default(),
            pages: Vec::new(),
            thumb_layout: ThumbLayout::default(),
            thumbs: Vec::new(),
            texts: Vec::new(),
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
        let sizes: Vec<PageSize> = doc
            .info
            .pages
            .iter()
            .map(|s| {
                if self.rotation % 2 == 1 {
                    PageSize {
                        width: s.height,
                        height: s.width,
                    }
                } else {
                    *s
                }
            })
            .collect();
        doc.layout = PageLayout::new(&sizes, self.settings.spread);
        doc.thumb_layout = ThumbLayout::new(&sizes);
        doc.pages.iter_mut().for_each(|slot| *slot = Slot::default());
        doc.thumbs.iter_mut().for_each(|thumb| *thumb = Thumb::default());
        Task::batch([self.go_to_page(page), self.request_thumbnails()])
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
            doc.worker.render(index, scale, rotation, move |result| {
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
            doc.worker
                .render_thumbnail(index, scale, rotation, move |result| {
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
        let page_count = self.doc.as_ref().map_or(0, |d| d.pages.len());
        let has_doc = page_count > 0;
        let current = self.current_page() + 1;

        let page_box = text_input(&current.to_string(), &self.page_input)
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
            gap(),
            icons::button(Icon::PrevPage, "Previous page (Page Up)", on(Message::PrevPage)),
            icons::with_tip(page_box, "Go to page"),
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
                    Spread::Single => "Single page: click for two pages side by side",
                    Spread::Two => "Two pages: click to show the cover page alone",
                    Spread::TwoCover => "Two pages with cover: click for single page",
                },
                on(Message::NextSpread),
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
                    "Hide comment tools"
                } else {
                    "Comment and draw"
                },
                self.annotate.open,
                Message::Annotate(annotate::Event::Toggle),
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
        let mut layers = vec![body];
        if let (Some(doc), true) = (&self.doc, self.show_properties) {
            layers.push(properties_view(&doc.info));
        }
        if let Some(card) = self.annotate.editor() {
            layers.push(modal(card, Message::Annotate(annotate::Event::EditCancel)));
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
                        let fields = self.form.overlays(index, self.page_map(index));
                        if marks.is_empty() && preview.is_none() && outline.is_none() && fields.is_empty() {
                            image(handle.clone()).width(width).height(height).into()
                        } else {
                            let mut layers = vec![image(handle.clone()).width(width).height(height).into()];
                            layers.extend(marks);
                            layers.extend(preview);
                            layers.extend(outline);
                            layers.extend(handles);
                            layers.extend(fields);
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
                    .interaction(if let Some(down) = over_handle {
                        if down {
                            mouse::Interaction::ResizingDiagonallyDown
                        } else {
                            mouse::Interaction::ResizingDiagonallyUp
                        }
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
