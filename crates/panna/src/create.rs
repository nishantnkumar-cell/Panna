//! Making a new blank PDF: its page size, which way up, and how many pages.

use std::fmt;

use iced::widget::{button, column, container, pick_list, radio, row, space, text, text_input};
use iced::{Center, Element};

use crate::Message;

/// Paper sizes, in points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paper {
    A4,
    Letter,
    Legal,
    A3,
    A5,
}

impl Paper {
    const ALL: [Paper; 5] = [Paper::A4, Paper::Letter, Paper::Legal, Paper::A3, Paper::A5];

    /// Width and height, upright.
    fn size(self) -> (f32, f32) {
        match self {
            Paper::A4 => (595.0, 842.0),
            Paper::Letter => (612.0, 792.0),
            Paper::Legal => (612.0, 1008.0),
            Paper::A3 => (842.0, 1191.0),
            Paper::A5 => (420.0, 595.0),
        }
    }

    /// Letter for the US and Canada, A4 elsewhere, by the system's language setting.
    fn local() -> Self {
        let lang = std::env::var("LC_PAPER")
            .or_else(|_| std::env::var("LC_ALL"))
            .or_else(|_| std::env::var("LANG"))
            .unwrap_or_default();
        if ["_US", "_CA"].iter().any(|r| lang.contains(r)) {
            Paper::Letter
        } else {
            Paper::A4
        }
    }
}

impl fmt::Display for Paper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Paper::A4 => "A4 (210 × 297 mm)",
            Paper::Letter => "Letter (8.5 × 11 in)",
            Paper::Legal => "Legal (8.5 × 14 in)",
            Paper::A3 => "A3 (297 × 420 mm)",
            Paper::A5 => "A5 (148 × 210 mm)",
        })
    }
}

#[derive(Debug, Clone)]
pub struct Blank {
    paper: Paper,
    landscape: bool,
    /// Pages, as typed.
    pages: String,
}

impl Default for Blank {
    fn default() -> Self {
        Self {
            paper: Paper::local(),
            landscape: false,
            pages: "1".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Paper(Paper),
    Landscape(bool),
    Pages(String),
    Go,
    Cancel,
}

impl Blank {
    pub fn update(&mut self, event: Event) {
        match event {
            Event::Paper(paper) => self.paper = paper,
            Event::Landscape(on) => self.landscape = on,
            Event::Pages(pages) => self.pages = pages,
            Event::Open | Event::Go | Event::Cancel => {}
        }
    }

    /// Page width and height in points, and how many pages; `None` while the count typed
    /// isn't one.
    pub fn pages(&self) -> Option<(f32, f32, usize)> {
        let count: usize = self.pages.trim().parse().ok()?;
        let (w, h) = self.paper.size();
        let (w, h) = if self.landscape { (h, w) } else { (w, h) };
        (1..=panna_core::MAX_BLANK_PAGES)
            .contains(&count)
            .then_some((w, h, count))
    }
}

pub fn pages_input_id() -> iced::widget::Id {
    iced::widget::Id::new("blank-pages")
}

/// The New blank PDF card.
pub fn card(state: &Blank) -> Element<'_, Message> {
    let ev = Message::Blank;
    let label = |s: &'static str| text(s).width(90);
    let ok = state.pages().is_some();
    container(
        column![
            text("New blank PDF").size(16),
            row![
                label("Page size"),
                pick_list(Paper::ALL, Some(state.paper), move |p| ev(Event::Paper(p)))
            ]
            .spacing(8)
            .align_y(Center),
            row![
                label(""),
                radio("Portrait", false, Some(state.landscape), move |v| ev(
                    Event::Landscape(v)
                )),
                radio("Landscape", true, Some(state.landscape), move |v| ev(
                    Event::Landscape(v)
                )),
            ]
            .spacing(16)
            .align_y(Center),
            row![
                label("Pages"),
                text_input("1", &state.pages)
                    .id(pages_input_id())
                    .on_input(move |s| ev(Event::Pages(s)))
                    .on_submit(ev(Event::Go))
                    .width(70),
            ]
            .spacing(8)
            .align_y(Center),
            row![
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(ev(Event::Cancel)),
                button("Create…").on_press_maybe(ok.then_some(ev(Event::Go))),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .width(380),
    )
    .padding(16)
    .style(container::rounded_box)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landscape_turns_the_page_and_the_count_must_be_a_number() {
        let mut blank = Blank {
            paper: Paper::A4,
            ..Blank::default()
        };
        assert_eq!(blank.pages(), Some((595.0, 842.0, 1)));
        blank.update(Event::Landscape(true));
        blank.update(Event::Pages("3".into()));
        assert_eq!(blank.pages(), Some((842.0, 595.0, 3)));
        blank.update(Event::Pages("x".into()));
        assert_eq!(blank.pages(), None);
        blank.update(Event::Pages("0".into()));
        assert_eq!(blank.pages(), None);
    }
}
