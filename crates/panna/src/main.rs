//! Panna: an offline, memory-safe PDF viewer.

#![forbid(unsafe_code)]
// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod find;
mod icons;
mod layout;
mod selection;
mod settings;
mod shortcuts;
mod sidebar;
mod toolbar;

use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::scrollable::{AbsoluteOffset, Viewport};
use iced::widget::{
    button, center, column, container, image, mouse_area, opaque, operation, pin, row, scrollable, space,
    stack, text, text_input,
};
use iced::{Center, Element, Fill, Length, Point, Size, Subscription, Task, event, mouse, window};
use panna_core::{
    DocumentInfo, Limits, OpenError, PageSize, PageText, RenderError, RenderWorker, RenderedPage,
};

use icons::Icon;
use layout::{PAGE_GAP, PageLayout};
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

fn window_settings() -> window::Settings {
    window::Settings {
        size: INITIAL_SIZE,
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
        let mut subs = vec![base];
        if self.selecting {
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
                        doc.layout = PageLayout::new(&info.pages);
                        doc.pages = info.pages.iter().map(|_| Slot::default()).collect();
                        doc.thumb_layout = ThumbLayout::new(&info.pages);
                        doc.thumbs = info.pages.iter().map(|_| Thumb::default()).collect();
                        doc.texts = info.pages.iter().map(|_| TextSlot::default()).collect();
                        doc.info = info;
                        self.selection = None;
                        self.hover = None;
                        self.find.rescan(&doc.texts);
                        self.status = Status::Idle;
                        self.zoom = 1.0;
                        self.viewport.offset_y = 0.0;
                        Task::batch([
                            operation::scroll_to(scroll_id(), AbsoluteOffset { x: 0.0, y: 0.0 }),
                            self.request_visible(),
                            self.request_thumbnails(),
                            self.request_search_text(),
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
                let widest = self.doc.as_ref().map_or(0.0, |d| d.layout.max_width());
                if widest > 0.0 {
                    let available = (self.viewport.width - 2.0 * PAGE_GAP - 16.0).max(100.0);
                    self.set_zoom(available / widest)
                } else {
                    Task::none()
                }
            }
            Message::FitPage => {
                let Some(doc) = &self.doc else { return Task::none() };
                let Some(size) = doc.layout.sizes().get(self.current_page()).copied() else {
                    return Task::none();
                };
                let page = self.current_page();
                let width = (self.viewport.width - 2.0 * PAGE_GAP - 16.0).max(100.0) / size.width;
                let height = (self.viewport.height - 2.0 * PAGE_GAP).max(100.0) / size.height;
                let zoom_task = self.set_zoom(width.min(height));
                Task::batch([zoom_task, self.go_to_page(page)])
            }
            Message::RotateCw => self.rotate(1),
            Message::RotateCcw => self.rotate(3),
            Message::PrevPage => self.go_to_page(self.current_page().saturating_sub(1)),
            Message::NextPage => self.go_to_page(self.current_page() + 1),
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
                if self.show_properties {
                    self.show_properties = false;
                } else if self.find.open {
                    self.find.open = false;
                    self.find.rescan(&[]);
                } else {
                    self.selection = None;
                }
                Task::none()
            }
            Message::Sidebar(event) => {
                if self.sidebar.update(event) {
                    self.settings.sidebar_open = self.sidebar.open;
                    self.settings.sidebar_side = self.sidebar.side;
                    self.settings.sidebar_tab = self.sidebar.tab;
                    self.settings.save();
                }
                self.request_thumbnails()
            }
            Message::GoToPage(index) => self.go_to_page(index),
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
                if self.selecting
                    && let Some(caret) = self.caret_at(page, position)
                    && let Some(selection) = &mut self.selection
                {
                    selection.focus = caret;
                }
                Task::none()
            }
            Message::PagePress(page) => {
                self.selecting = false;
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
                if !self.selecting {
                    return Task::none();
                }
                self.selecting = false;
                if self.selection.is_some_and(|s| s.is_empty()) {
                    self.selection = None;
                }
                self.copy_primary()
            }
            Message::Copy => match self.selected_text() {
                Some(text) => iced::clipboard::write(text),
                None => Task::none(),
            },
        }
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
                self.find.open = true;
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
                self.rescan_find();
                Task::none()
            }
            find::Event::Query(query) => {
                self.find.query = query;
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
        }
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
        let Some(hit) = self.find.step(forward, from).cloned() else {
            return Task::none();
        };
        let Some(text) = self.page_text(hit.page) else {
            return Task::none();
        };
        let Some(first) = text.rects(hit.range.clone()).first().copied() else {
            return self.go_to_page(hit.page);
        };
        let Some(doc) = &self.doc else { return Task::none() };
        let (x, y, _, _) = self.page_map(hit.page).rect(&first);
        let page_w = doc
            .layout
            .sizes()
            .get(hit.page)
            .map_or(0.0, |s| s.width * self.zoom);
        let content_w = (doc.layout.max_width() * self.zoom + 2.0 * PAGE_GAP).max(self.viewport.width);
        let page_left = (content_w - page_w) / 2.0;
        let top = doc.layout.page_top(hit.page, self.zoom) + PAGE_GAP + y;
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
        doc.layout = PageLayout::new(&sizes);
        doc.thumb_layout = ThumbLayout::new(&sizes);
        doc.pages.iter_mut().for_each(|slot| *slot = Slot::default());
        doc.thumbs.iter_mut().for_each(|thumb| *thumb = Thumb::default());
        Task::batch([self.go_to_page(page), self.request_thumbnails()])
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
            gap(),
            icons::button(
                Icon::Search,
                format!("Find ({m}F)"),
                on(Message::Find(find::Event::Open)),
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
                let pages = if self.find.open {
                    let total = doc.texts.len();
                    let searched = doc.texts.iter().filter(|t| t.text.is_some()).count();
                    stack![
                        pages,
                        container(opaque(self.find.view(searched, total)))
                            .width(Fill)
                            .align_x(iced::Right)
                            .padding([10, 28]),
                    ]
                    .into()
                } else {
                    pages
                };
                self.with_sidebar(doc, pages)
            }
            _ => self.welcome_view(),
        };
        let body = match (&self.doc, self.show_properties) {
            (Some(doc), true) => stack![body, properties_view(&doc.info)].into(),
            _ => body,
        };

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
        if !self.sidebar.open {
            return pages;
        }
        let bar = self.sidebar.view(sidebar::ViewData {
            layout: &doc.thumb_layout,
            thumbs_images: &doc.thumbs,
            outline: &doc.info.outline,
            current: self.current_page(),
        });
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
                        if marks.is_empty() {
                            image(handle.clone()).width(width).height(height).into()
                        } else {
                            let mut layers = vec![image(handle.clone()).width(width).height(height).into()];
                            layers.extend(marks);
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
                let content = mouse_area(content)
                    .on_move(move |p| Message::PagePointer(index, p))
                    .on_press(Message::PagePress(index))
                    .on_double_click(Message::PageDoubleClick(index))
                    .on_release(Message::PointerReleased)
                    .interaction(if over_text || self.selecting {
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
        let content_width = (doc.layout.max_width() * self.zoom + 2.0 * PAGE_GAP).max(self.viewport.width);
        let column = iced::widget::Column::with_children(pages)
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
