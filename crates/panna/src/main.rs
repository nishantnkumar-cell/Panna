//! Panna: an offline, memory-safe PDF viewer.

#![forbid(unsafe_code)]
// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod layout;
mod shortcuts;

use std::path::PathBuf;

use iced::widget::scrollable::{AbsoluteOffset, Viewport};
use iced::widget::{
    button, center, column, container, image, operation, row, scrollable, space, text, text_input,
};
use iced::{Center, Element, Fill, Length, Subscription, Task, event, window};
use panna_core::{DocumentInfo, Limits, OpenError, RenderError, RenderWorker, RenderedPage};

use layout::{PAGE_GAP, PageLayout};

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
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const APP_ID: &str = "io.github.nishantnkumar_cell.Panna";

fn window_settings() -> window::Settings {
    window::Settings {
        size: iced::Size::new(1100.0, 850.0),
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
        result: Result<RenderedPage, RenderError>,
    },
    ScaleFactor(f32),
    Scrolled(Viewport),
    ZoomIn,
    ZoomOut,
    ZoomReset,
    FitWidth,
    PrevPage,
    NextPage,
    PageInput(String),
    PageSubmit,
    Ignore,
}

struct App {
    next_doc_id: u64,
    doc: Option<OpenDoc>,
    status: Status,
    zoom: f32,
    scale_factor: f32,
    viewport: ViewportState,
    page_input: String,
}

enum Status {
    Idle,
    Loading(PathBuf),
    Error(String),
}

struct OpenDoc {
    id: u64,
    path: PathBuf,
    info: DocumentInfo,
    worker: RenderWorker,
    layout: PageLayout,
    pages: Vec<Slot>,
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
    offset_y: f32,
    width: f32,
    height: f32,
}

fn scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("pages")
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let mut app = Self {
            next_doc_id: 0,
            doc: None,
            status: Status::Idle,
            zoom: 1.0,
            scale_factor: 1.0,
            viewport: ViewportState {
                offset_y: 0.0,
                width: 1000.0,
                height: 800.0,
            },
            page_input: String::new(),
        };
        let scale = window::oldest()
            .and_then(window::scale_factor)
            .map(Message::ScaleFactor);
        let open = match std::env::args_os().nth(1) {
            Some(arg) => app.open(PathBuf::from(arg)),
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
        event::listen_with(|event, status, _window| match event {
            iced::Event::Window(window::Event::FileDropped(path)) => Some(Message::FileChosen(Some(path))),
            iced::Event::Window(window::Event::Rescaled(factor)) => Some(Message::ScaleFactor(factor)),
            iced::Event::Keyboard(key) if status == event::Status::Ignored => shortcuts::map(key),
            _ => None,
        })
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
            Message::FileChosen(Some(path)) => self.open(path),
            Message::FileChosen(None) | Message::Ignore => Task::none(),
            Message::Opened(id, result) => {
                let Some(doc) = self.doc.as_mut().filter(|d| d.id == id) else {
                    return Task::none();
                };
                match result {
                    Ok(info) => {
                        doc.layout = PageLayout::new(&info.pages);
                        doc.pages = info.pages.iter().map(|_| Slot::default()).collect();
                        doc.info = info;
                        self.status = Status::Idle;
                        self.zoom = 1.0;
                        self.viewport.offset_y = 0.0;
                        Task::batch([
                            operation::scroll_to(scroll_id(), AbsoluteOffset { x: 0.0, y: 0.0 }),
                            self.request_visible(),
                        ])
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
                result,
            } => {
                if let Some(slot) = self
                    .doc
                    .as_mut()
                    .filter(|d| d.id == doc)
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
        }
    }

    fn open(&mut self, path: PathBuf) -> Task<Message> {
        let id = self.next_doc_id;
        self.next_doc_id += 1;
        let (tx, rx) = futures_channel::oneshot::channel();
        let worker = match RenderWorker::open(path.clone(), None, Limits::default(), move |result| {
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
            info: DocumentInfo {
                pages: Vec::new(),
                title: None,
            },
            worker,
            layout: PageLayout::default(),
            pages: Vec::new(),
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
            doc.worker.render(index, scale, move |result| {
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

        let toolbar = row![
            button("Open").on_press(Message::OpenDialog),
            space::horizontal().width(16),
            button("‹").on_press_maybe(has_doc.then_some(Message::PrevPage)),
            page_box,
            text(format!("of {page_count}")),
            button("›").on_press_maybe(has_doc.then_some(Message::NextPage)),
            space::horizontal().width(16),
            button("−").on_press_maybe(has_doc.then_some(Message::ZoomOut)),
            button(text(format!("{:.0}%", self.zoom * 100.0)))
                .on_press_maybe(has_doc.then_some(Message::ZoomReset)),
            button("+").on_press_maybe(has_doc.then_some(Message::ZoomIn)),
            button("Fit width").on_press_maybe(has_doc.then_some(Message::FitWidth)),
        ]
        .spacing(6)
        .align_y(Center)
        .padding(8);

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
            (Status::Idle, Some(doc)) if has_doc => self.pages_view(doc),
            _ => center(
                column![
                    text("Open a PDF to get started").size(20),
                    text("Use the Open button, press Ctrl+O, or drop a file on this window."),
                ]
                .spacing(8)
                .align_x(Center),
            )
            .into(),
        };

        column![toolbar, body].into()
    }

    fn pages_view<'a>(&'a self, doc: &'a OpenDoc) -> Element<'a, Message> {
        let pages = doc.layout.sizes().iter().zip(&doc.pages).map(|(size, slot)| {
            let width = Length::Fixed((size.width * self.zoom).round());
            let height = Length::Fixed((size.height * self.zoom).round());
            let content: Element<'_, Message> = match (&slot.image, &slot.error) {
                (Some(handle), _) => image(handle.clone()).width(width).height(height).into(),
                (None, Some(e)) => center(text(e.as_str())).into(),
                (None, None) => space::horizontal().into(),
            };
            container(content)
                .width(width)
                .height(height)
                .style(|_| {
                    container::Style::default()
                        .background(iced::Color::WHITE)
                        .shadow(iced::Shadow {
                            color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.35),
                            offset: iced::Vector::new(0.0, 2.0),
                            blur_radius: 8.0,
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
