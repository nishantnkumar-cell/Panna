//! The Page labels card: the numbering a document's pages show, such as i, ii, iii for a
//! preface then 1, 2, 3, in ranges that each start on a page.

use std::fmt;

use iced::widget::{button, column, container, pick_list, row, space, text, text_input};
use iced::{Center, Element};
use panna_core::{LabelRange, LabelStyle};

use crate::Message;
use crate::icons::{self, Icon};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleChoice(pub LabelStyle);

impl StyleChoice {
    const ALL: [StyleChoice; 6] = [
        StyleChoice(LabelStyle::Decimal),
        StyleChoice(LabelStyle::LowerRoman),
        StyleChoice(LabelStyle::UpperRoman),
        StyleChoice(LabelStyle::LowerLetters),
        StyleChoice(LabelStyle::UpperLetters),
        StyleChoice(LabelStyle::None),
    ];
}

impl fmt::Display for StyleChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            LabelStyle::Decimal => "1, 2, 3",
            LabelStyle::LowerRoman => "i, ii, iii",
            LabelStyle::UpperRoman => "I, II, III",
            LabelStyle::LowerLetters => "a, b, c",
            LabelStyle::UpperLetters => "A, B, C",
            LabelStyle::None => "No number",
        })
    }
}

/// One range as typed: its first page (from 1), style, prefix and first number.
#[derive(Debug, Clone)]
struct Row {
    start: String,
    style: LabelStyle,
    prefix: String,
    first: String,
}

#[derive(Debug, Clone)]
pub struct Labelling {
    rows: Vec<Row>,
    /// Pages in the document.
    count: usize,
}

#[derive(Debug, Clone)]
pub enum Event {
    Start(usize, String),
    Style(usize, StyleChoice),
    Prefix(usize, String),
    First(usize, String),
    /// A new range starting at the current page.
    Add,
    Remove(usize),
    Go,
    Cancel,
}

impl Labelling {
    /// Starts from the document's `ranges`, or plain numbers.
    pub fn new(ranges: &[LabelRange], count: usize) -> Self {
        let mut rows: Vec<Row> = ranges
            .iter()
            .map(|r| Row {
                start: (r.start + 1).to_string(),
                style: r.style,
                prefix: r.prefix.clone(),
                first: r.first.to_string(),
            })
            .collect();
        if rows.first().is_none_or(|r| r.start != "1") {
            rows.insert(
                0,
                Row {
                    start: "1".into(),
                    style: LabelStyle::Decimal,
                    prefix: String::new(),
                    first: "1".into(),
                },
            );
        }
        Self { rows, count }
    }

    pub fn update(&mut self, event: Event, current: usize) {
        match event {
            Event::Start(i, s) => {
                // The first range always starts on the first page.
                if i > 0
                    && let Some(row) = self.rows.get_mut(i)
                {
                    row.start = s;
                }
            }
            Event::Style(i, choice) => {
                if let Some(row) = self.rows.get_mut(i) {
                    row.style = choice.0;
                }
            }
            Event::Prefix(i, s) => {
                if let Some(row) = self.rows.get_mut(i) {
                    row.prefix = s;
                }
            }
            Event::First(i, s) => {
                if let Some(row) = self.rows.get_mut(i) {
                    row.first = s;
                }
            }
            Event::Add => {
                let start = (current + 1).max(2).min(self.count.max(1));
                self.rows.push(Row {
                    start: start.to_string(),
                    style: LabelStyle::Decimal,
                    prefix: String::new(),
                    first: "1".into(),
                });
            }
            Event::Remove(i) => {
                if i > 0 && i < self.rows.len() {
                    self.rows.remove(i);
                }
            }
            Event::Go | Event::Cancel => {}
        }
    }

    /// The ranges, by start page; `None` while a page or number typed isn't one, or two
    /// ranges start on the same page.
    pub fn ranges(&self) -> Option<Vec<LabelRange>> {
        let mut out = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            let start: usize = row.start.trim().parse().ok()?;
            let first: u32 = row.first.trim().parse().ok()?;
            if !(1..=self.count).contains(&start) || first == 0 {
                return None;
            }
            out.push(LabelRange {
                start: start - 1,
                style: row.style,
                prefix: row.prefix.clone(),
                first,
            });
        }
        out.sort_by_key(|r| r.start);
        let unique = out
            .windows(2)
            .all(|w| w.first().map(|r| r.start) != w.get(1).map(|r| r.start));
        unique.then_some(out)
    }
}

/// The Page labels card, with `current` the page shown (from 0).
pub fn card(state: &Labelling) -> Element<'_, Message> {
    let ev = Message::Labels;
    let ranges = state.ranges();
    let mut rows = column![
        row![
            text("From").width(80),
            text("Numbers").width(120),
            text("Prefix").width(90),
            text("Start at").width(70),
        ]
        .spacing(8)
    ]
    .spacing(6);
    for (i, r) in state.rows.iter().enumerate() {
        let start = text_input("", &r.start).width(80);
        let start = if i == 0 {
            start
        } else {
            start.on_input(move |s| ev(Event::Start(i, s)))
        };
        let remove: Element<'_, Message> = if i == 0 {
            space().width(28).into()
        } else {
            icons::button(Icon::Delete, "Remove this range", Some(ev(Event::Remove(i))))
        };
        rows = rows.push(
            row![
                start,
                pick_list(StyleChoice::ALL, Some(StyleChoice(r.style)), move |c| ev(
                    Event::Style(i, c)
                ))
                .width(120),
                text_input("", &r.prefix)
                    .on_input(move |s| ev(Event::Prefix(i, s)))
                    .width(90),
                text_input("1", &r.first)
                    .on_input(move |s| ev(Event::First(i, s)))
                    .width(70),
                remove,
            ]
            .spacing(8)
            .align_y(Center),
        );
    }
    // What the first pages will show.
    let sample = ranges.as_ref().map_or_else(String::new, |ranges| {
        let shown: Vec<String> = (0..state.count.min(6))
            .map(|i| panna_core::page_label(ranges, i))
            .collect();
        let more = if state.count > 6 { ", …" } else { "" };
        format!("Pages show: {}{more}", shown.join(", "))
    });
    container(
        column![
            text("Page labels").size(16),
            text(
                "How pages are numbered in the page box and thumbnails, as in a book whose \
                 preface runs i, ii, iii before page 1."
            )
            .size(13),
            rows,
            button(text("Add a range from this page"))
                .style(button::secondary)
                .on_press(ev(Event::Add)),
            text(sample).size(13).style(text::secondary),
            row![
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(ev(Event::Cancel)),
                button("Apply").on_press_maybe(ranges.is_some().then_some(ev(Event::Go))),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .width(460),
    )
    .padding(16)
    .style(container::rounded_box)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_sorted_and_must_be_pages() {
        let mut state = Labelling::new(&[], 10);
        state.update(Event::Style(0, StyleChoice(LabelStyle::LowerRoman)), 0);
        state.update(Event::Add, 3);
        assert_eq!(
            state
                .ranges()
                .unwrap()
                .iter()
                .map(|r| r.start)
                .collect::<Vec<_>>(),
            [0, 3]
        );
        state.update(Event::Start(1, "11".into()), 0);
        assert!(state.ranges().is_none());
        state.update(Event::Start(1, "1".into()), 0);
        assert!(state.ranges().is_none(), "two ranges on page 1");
        // The first range can't be moved off page 1.
        state.update(Event::Start(0, "5".into()), 0);
        state.update(Event::Remove(1), 0);
        assert_eq!(state.ranges().unwrap()[0].start, 0);
    }
}
