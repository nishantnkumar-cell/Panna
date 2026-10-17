//! Document search: matches every page's text as it is extracted, steps through the matches,
//! and lists them with their context in the sidebar.

use std::ops::Range;

use iced::widget::{Column, button, column, rich_text, row, scrollable, space, span, text, text_input};
use iced::{Center, Element, Fill, Font, Theme, font};
use panna_core::{FindOptions, PageText};

use crate::Message;
use crate::icons::{self, Icon};
use crate::selection::TextSlot;

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Query(String),
    MatchCase,
    WholeWord,
    Next,
    Prev,
    Select(usize),
}

/// Results listed in the panel. More are still highlighted and reachable with Next.
const MAX_LISTED: usize = 500;
/// Characters of context shown on each side of a match.
const CONTEXT: usize = 32;

/// The text around a match, on one line: (before, match, after).
fn snippet(text: &PageText, range: Range<usize>) -> (String, String, String) {
    let clean = |s: String| s.replace('\n', " ");
    let start = range.start.saturating_sub(CONTEXT);
    let mut before = clean(text.slice(start..range.start));
    if start > 0 {
        // Start at a word boundary.
        if let Some(space) = before.find(' ') {
            before = format!("…{}", &before[space + 1..]);
        }
    }
    let end = range.end + CONTEXT;
    let mut after = clean(text.slice(range.end..end));
    if end < text.len()
        && let Some(space) = after.rfind(' ')
    {
        after = format!("{}…", &after[..space]);
    }
    (before, clean(text.slice(range)), after)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub page: usize,
    pub range: Range<usize>,
}

#[derive(Debug, Default)]
pub struct Find {
    pub open: bool,
    pub query: String,
    pub options: FindOptions,
    /// Sorted by page, then position.
    hits: Vec<Hit>,
    current: Option<usize>,
}

pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("find")
}

impl Find {
    pub fn active(&self) -> bool {
        self.open && !self.query.trim().is_empty()
    }

    /// Matches on `page`, and which of them (if any) is the current one.
    pub fn hits_on(&self, page: usize) -> (&[Hit], Option<usize>) {
        let start = self.hits.partition_point(|h| h.page < page);
        let end = self.hits.partition_point(|h| h.page <= page);
        let current = self
            .current
            .filter(|c| (start..end).contains(c))
            .map(|c| c - start);
        (self.hits.get(start..end).unwrap_or_default(), current)
    }

    /// Recomputes every match from the text extracted so far.
    pub fn rescan(&mut self, texts: &[TextSlot]) {
        self.hits.clear();
        self.current = None;
        if !self.active() {
            return;
        }
        for (page, slot) in texts.iter().enumerate() {
            if let Some(text) = &slot.text {
                self.hits.extend(self.matches(page, text));
            }
        }
    }

    /// Adds the matches of a newly extracted page, keeping the current match where it is.
    pub fn add_page(&mut self, page: usize, text: &PageText) {
        if !self.active() {
            return;
        }
        let new = self.matches(page, text);
        if new.is_empty() {
            return;
        }
        let at = self.hits.partition_point(|h| h.page < page);
        if let Some(c) = self.current.as_mut()
            && *c >= at
        {
            *c += new.len();
        }
        self.hits.splice(at..at, new);
    }

    fn matches(&self, page: usize, text: &PageText) -> Vec<Hit> {
        text.find(&self.query, self.options)
            .into_iter()
            .map(|range| Hit { page, range })
            .collect()
    }

    /// Moves to the next (or previous) match, starting from `from_page` when nothing is
    /// current yet. Returns the new current match.
    pub fn step(&mut self, forward: bool, from_page: usize) -> Option<&Hit> {
        if self.hits.is_empty() {
            self.current = None;
            return None;
        }
        let n = self.hits.len();
        let next = match self.current {
            Some(c) if forward => (c + 1) % n,
            Some(c) => (c + n - 1) % n,
            None if forward => {
                let i = self.hits.partition_point(|h| h.page < from_page);
                if i == n { 0 } else { i }
            }
            None => {
                let i = self.hits.partition_point(|h| h.page <= from_page);
                if i == 0 { n - 1 } else { i - 1 }
            }
        };
        self.current = Some(next);
        self.hits.get(next)
    }

    /// Whether a first match should be shown now: there are matches and none is current.
    pub fn needs_first(&self) -> bool {
        self.current.is_none() && !self.hits.is_empty()
    }

    /// The search panel in the sidebar: a search box, options, and every match with the text
    /// around it, as in Word's navigation pane.
    pub fn panel<'a>(&'a self, texts: &'a [TextSlot], searched: usize, total: usize) -> Element<'a, Message> {
        let ev = |e: Event| Message::Find(e);
        let status = if !self.active() {
            String::new()
        } else if self.hits.is_empty() && searched < total {
            format!("Searching… {}%", searched * 100 / total.max(1))
        } else if self.hits.is_empty() {
            "No matches".into()
        } else {
            let more = if searched < total { "+" } else { "" };
            match self.current {
                Some(c) => format!("Result {} of {}{more}", c + 1, self.hits.len()),
                None => format!("{}{more} results", self.hits.len()),
            }
        };
        let has_hits = !self.hits.is_empty();
        let input = text_input("Search document", &self.query)
            .id(input_id())
            .on_input(move |q| ev(Event::Query(q)))
            .on_submit(ev(Event::Next))
            .size(13)
            .padding([4, 6]);
        let controls = row![
            icons::toggle(
                Icon::MatchCase,
                "Match case",
                self.options.match_case,
                ev(Event::MatchCase)
            ),
            icons::toggle(
                Icon::WholeWord,
                "Whole words only",
                self.options.whole_word,
                ev(Event::WholeWord)
            ),
            space::horizontal(),
            icons::button(
                Icon::FindPrev,
                "Previous result (Shift+F3)",
                has_hits.then(|| ev(Event::Prev))
            ),
            icons::button(
                Icon::FindNext,
                "Next result (Enter or F3)",
                has_hits.then(|| ev(Event::Next))
            ),
        ]
        .spacing(2)
        .align_y(Center);

        let mut list = Column::new().spacing(2);
        for (i, hit) in self.hits.iter().enumerate().take(MAX_LISTED) {
            let Some(page_text) = texts.get(hit.page).and_then(|t| t.text.as_deref()) else {
                continue;
            };
            let (before, found, after) = snippet(page_text, hit.range.clone());
            let current = self.current == Some(i);
            let line: text::Rich<'_, (), Message> = rich_text![
                span(before),
                span(found).font(Font {
                    weight: font::Weight::Bold,
                    ..Font::DEFAULT
                }),
                span(after),
            ]
            .size(12);
            let entry = column![
                text(format!("Page {}", hit.page + 1))
                    .size(11)
                    .style(text::secondary),
                line
            ]
            .spacing(1);
            list = list.push(
                button(entry)
                    .padding([4, 6])
                    .width(Fill)
                    .style(move |theme: &Theme, status| {
                        let palette = theme.extended_palette();
                        let background = if current {
                            Some(palette.primary.weak.color.into())
                        } else if matches!(status, button::Status::Hovered) {
                            Some(palette.background.strong.color.into())
                        } else {
                            None
                        };
                        button::Style {
                            background,
                            text_color: palette.background.base.text,
                            border: iced::Border {
                                radius: 4.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    })
                    .on_press(ev(Event::Select(i))),
            );
        }
        if self.hits.len() > MAX_LISTED {
            list = list.push(
                text(format!("Showing the first {MAX_LISTED} results."))
                    .size(11)
                    .style(text::secondary),
            );
        }
        column![
            column![input, controls, text(status).size(12).style(text::secondary)]
                .spacing(4)
                .padding([4, 8]),
            scrollable(list.padding([0, 6])).height(Fill),
        ]
        .spacing(2)
        .into()
    }

    /// Makes result `i` current. Returns it.
    pub fn select(&mut self, i: usize) -> Option<&Hit> {
        let hit = self.hits.get(i)?;
        self.current = Some(i);
        Some(hit)
    }

    pub fn update_options(&mut self, event: &Event) {
        match event {
            Event::MatchCase => self.options.match_case = !self.options.match_case,
            Event::WholeWord => self.options.whole_word = !self.options.whole_word,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_with(hits: &[(usize, usize)]) -> Find {
        Find {
            open: true,
            query: "x".into(),
            hits: hits
                .iter()
                .map(|&(page, at)| Hit {
                    page,
                    range: at..at + 1,
                })
                .collect(),
            ..Find::default()
        }
    }

    #[test]
    fn first_step_starts_from_the_current_page_and_wraps() {
        let mut f = find_with(&[(1, 0), (4, 0), (4, 9), (8, 2)]);
        assert_eq!(f.step(true, 3).map(|h| h.page), Some(4));
        assert_eq!(f.step(true, 3).map(|h| h.range.start), Some(9));
        assert_eq!(f.step(true, 3).map(|h| h.page), Some(8));
        assert_eq!(f.step(true, 3).map(|h| h.page), Some(1));
        let mut f = find_with(&[(1, 0), (4, 0)]);
        assert_eq!(f.step(false, 0).map(|h| h.page), Some(4));
        let mut f = find_with(&[(1, 0)]);
        assert_eq!(f.step(true, 9).map(|h| h.page), Some(1));
    }

    #[test]
    fn hits_on_reports_the_current_match() {
        let mut f = find_with(&[(1, 0), (4, 0), (4, 9)]);
        f.current = Some(2);
        let (hits, current) = f.hits_on(4);
        assert_eq!((hits.len(), current), (2, Some(1)));
        assert_eq!(f.hits_on(2).0.len(), 0);
    }
}
