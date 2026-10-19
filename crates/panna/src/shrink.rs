//! The Save a smaller copy card: whether pictures sharper than needed may be scaled down too,
//! and to what resolution. Without that, the copy looks exactly the same.

use iced::widget::{button, checkbox, column, container, pick_list, row, space, text};
use iced::{Element, Theme};

use crate::Message;

#[derive(Debug, Clone, Copy, Default)]
pub struct Shrinking {
    pub pictures: bool,
    pub dpi: Dpi,
}

impl Shrinking {
    /// The resolution pictures are scaled down to, when they may be.
    pub fn pictures(self) -> Option<u32> {
        self.pictures.then_some(self.dpi.0)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Event {
    Open,
    Pictures(bool),
    Dpi(Dpi),
    Go,
    Cancel,
}

/// Dots per inch, as the list names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dpi(pub u32);

impl Default for Dpi {
    fn default() -> Self {
        Self(150)
    }
}

impl Dpi {
    const ALL: [Dpi; 3] = [Dpi(300), Dpi(150), Dpi(96)];
}

impl std::fmt::Display for Dpi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} dpi", self.0)
    }
}

pub fn card<'a>(state: Shrinking) -> Element<'a, Message> {
    let ev = Message::Shrink;
    let mut body = column![
        text("Save a smaller copy").size(16),
        text(
            "Compresses the file harder, keeps one copy of fonts and pictures used more than \
             once, and leaves out what nothing uses. Pages look exactly the same."
        )
        .size(13),
        row![
            checkbox(state.pictures)
                .label("Also make sharp pictures smaller, to")
                .on_toggle(move |v| ev(Event::Pictures(v))),
            pick_list(Dpi::ALL, Some(state.dpi), move |v| ev(Event::Dpi(v)))
                .text_size(13)
                .padding([2, 6]),
        ]
        .spacing(6)
        .align_y(iced::Center),
    ]
    .spacing(10)
    .width(420);
    if state.pictures {
        let hint = match state.dpi.0 {
            300 => "300 dpi suits professional printing.",
            150 => "150 dpi suits screens and office printers.",
            _ => "96 dpi suits screens only.",
        };
        body = body.push(
            text(format!(
                "{hint} Pictures look a little softer when zoomed in; photos stay JPEG, and \
                 other pictures are kept without loss."
            ))
            .size(12)
            .style(text::secondary),
        );
    }
    body = body.push(
        row![
            space::horizontal(),
            button("Cancel")
                .style(button::secondary)
                .on_press(ev(Event::Cancel)),
            button("Save as…").on_press(ev(Event::Go)),
        ]
        .spacing(8),
    );
    container(body)
        .padding(16)
        .style(|theme: &Theme| container::rounded_box(theme))
        .into()
}
