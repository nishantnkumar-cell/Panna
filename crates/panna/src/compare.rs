//! Comparing the open document's text with another file's: the changes listed in the
//! sidebar and marked on the pages.

use std::ops::Range;

use iced::widget::{Column, button, column, container, rich_text, row, scrollable, span, text};
use iced::{Center, Color, Element, Fill, Theme};
use panna_core::{Change, ChangeKind, PageText, TextBox};

use crate::Message;
use crate::icons::{self, Icon};

/// Changes listed at most; stepping still covers every one.
const MAX_LISTED: usize = 500;
/// Characters of a change's text shown in the list.
const MAX_SHOWN: usize = 160;

const ADDED: Color = Color::from_rgb8(0x1E, 0x9E, 0x4A);
const REMOVED: Color = Color::from_rgb8(0xD6, 0x33, 0x3F);

#[derive(Debug, Clone)]
pub enum Event {
    /// Choose a file to compare with.
    Open,
    Goto(usize),
    Step(bool),
    Close,
}

#[derive(Debug)]
pub struct Comparison {
    /// The other file's name.
    pub name: String,
    changes: Vec<Change>,
    current: Option<usize>,
}

fn shorten(s: &str) -> String {
    if s.chars().count() <= MAX_SHOWN {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(MAX_SHOWN).collect();
    out.push('…');
    out
}

fn label(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "Added",
        ChangeKind::Removed => "Removed",
        ChangeKind::Replaced => "Changed",
    }
}

impl Comparison {
    pub fn new(name: String, changes: Vec<Change>) -> Self {
        Self {
            name,
            changes,
            current: None,
        }
    }

    /// Makes change `index` the current one, returning its page and characters to show (the
    /// one after it, where text was taken out).
    pub fn select(&mut self, index: usize) -> Option<(usize, Range<usize>)> {
        let change = self.changes.get(index)?;
        self.current = Some(index);
        let range = change.range.clone();
        let range = if range.is_empty() {
            range.start.saturating_sub(1)..range.start + 1
        } else {
            range
        };
        Some((change.page, range))
    }

    /// Moves to the next or previous change, wrapping around.
    pub fn step(&mut self, forward: bool) -> Option<(usize, Range<usize>)> {
        let n = self.changes.len();
        if n == 0 {
            return None;
        }
        let next = match (self.current, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(c), true) => (c + 1) % n,
            (Some(c), false) => (c + n - 1) % n,
        };
        self.select(next)
    }

    /// The marks for page `page`: text to tint, and thin bars where text was taken out.
    pub fn marks(&self, page: usize, text: &PageText) -> (Vec<(TextBox, Color)>, Vec<TextBox>) {
        let start = self.changes.partition_point(|c| c.page < page);
        let end = self.changes.partition_point(|c| c.page <= page);
        let (mut tints, mut bars) = (Vec::new(), Vec::new());
        for (i, change) in self.changes.iter().enumerate().take(end).skip(start) {
            let current = self.current == Some(i);
            let alpha = if current { 0.55 } else { 0.3 };
            match change.kind() {
                ChangeKind::Removed => {
                    let at = change.range.start;
                    let bar = if at < text.len() {
                        text.rects(at..at + 1).first().map(|b| (b.x0, *b))
                    } else {
                        text.rects(at.saturating_sub(1)..at).last().map(|b| (b.x1, *b))
                    };
                    if let Some((x, b)) = bar {
                        let half = if current { 1.5 } else { 1.0 };
                        bars.push(TextBox {
                            x0: x - half,
                            x1: x + half,
                            y0: b.y0 - 1.0,
                            y1: b.y1 + 1.0,
                        });
                    }
                }
                kind => {
                    let color = if kind == ChangeKind::Added {
                        ADDED
                    } else {
                        Color::from_rgb8(0xF0, 0xA0, 0x20)
                    };
                    for b in text.rects(change.range.clone()) {
                        tints.push((b, Color { a: alpha, ..color }));
                    }
                }
            }
        }
        (tints, bars)
    }

    pub fn bar_color() -> Color {
        REMOVED
    }

    /// The sidebar's list of changes.
    pub fn panel(&self) -> Element<'_, Message> {
        let ev = Message::Compare;
        let count = match self.changes.len() {
            0 => "No differences in the text.".to_owned(),
            1 => "1 difference".to_owned(),
            n => format!("{n} differences"),
        };
        let any = !self.changes.is_empty();
        let header = column![
            row![
                text(format!("Compared with {}", self.name))
                    .size(13)
                    .width(Fill)
                    .wrapping(text::Wrapping::WordOrGlyph),
                icons::button(Icon::Close, "Stop comparing", Some(ev(Event::Close))),
            ]
            .align_y(Center),
            row![
                text(count).size(12).style(text::secondary).width(Fill),
                icons::button(
                    Icon::FindPrev,
                    "Previous difference",
                    any.then_some(ev(Event::Step(false)))
                ),
                icons::button(
                    Icon::FindNext,
                    "Next difference",
                    any.then_some(ev(Event::Step(true)))
                ),
            ]
            .align_y(Center),
        ]
        .spacing(2)
        .padding([6, 8]);
        let entries = self
            .changes
            .iter()
            .enumerate()
            .take(MAX_LISTED)
            .map(|(i, change)| {
                let snippet: text::Rich<'_, (), Message> = rich_text![
                    span(shorten(&change.other_text))
                        .color(REMOVED)
                        .strikethrough(true),
                    span(if change.kind() == ChangeKind::Replaced {
                        " "
                    } else {
                        ""
                    }),
                    span(shorten(&change.text)).color(ADDED),
                ]
                .size(13)
                .wrapping(text::Wrapping::WordOrGlyph);
                let current = self.current == Some(i);
                button(
                    column![
                        text(format!("Page {} · {}", change.page + 1, label(change.kind())))
                            .size(11)
                            .style(text::secondary),
                        snippet,
                    ]
                    .spacing(2),
                )
                .width(Fill)
                .padding([6, 8])
                .style(move |theme: &Theme, status| {
                    let mut style = button::text(theme, status);
                    if current {
                        let palette = theme.extended_palette();
                        style.background = Some(palette.primary.weak.color.into());
                        style.text_color = palette.primary.weak.text;
                    }
                    style
                })
                .on_press(ev(Event::Goto(i)))
                .into()
            });
        let mut list = Column::with_children(entries)
            .spacing(2)
            .padding([4, 6])
            .width(Fill);
        if self.changes.len() > MAX_LISTED {
            list = list.push(
                text(format!(
                    "Showing the first {MAX_LISTED} of {} differences.",
                    self.changes.len()
                ))
                .size(12)
                .style(text::secondary),
            );
        }
        column![
            header,
            container(scrollable(list).height(Fill)).height(Fill),
            container(
                text("Green is only in this file; red, struck through, only in the other.")
                    .size(11)
                    .style(text::secondary)
            )
            .padding([6, 8]),
        ]
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(page: usize) -> Change {
        Change {
            page,
            range: 0..1,
            text: "a".into(),
            other_page: None,
            other_text: String::new(),
        }
    }

    #[test]
    fn stepping_wraps_around() {
        let mut c = Comparison::new("x.pdf".into(), vec![change(0), change(2), change(5)]);
        assert_eq!(c.step(false), Some((5, 0..1)));
        assert_eq!(c.step(true), Some((0, 0..1)));
        assert_eq!(c.step(true), Some((2, 0..1)));
        assert_eq!(c.select(9), None);
        assert_eq!(c.current, Some(1));
        let mut none = Comparison::new("y.pdf".into(), Vec::new());
        assert_eq!(none.step(true), None);
    }
}
