//! The find bar: searches every page's text as it is extracted, and steps through matches.

use std::ops::Range;

use iced::widget::{container, row, text, text_input};
use iced::{Center, Element, Theme};
use panna_core::{FindOptions, PageText};

use crate::Message;
use crate::icons::{self, Icon};
use crate::selection::TextSlot;

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Close,
    Query(String),
    MatchCase,
    WholeWord,
    Next,
    Prev,
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

    pub fn view(&self, searched: usize, total: usize) -> Element<'_, Message> {
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
                Some(c) => format!("{} of {}{more}", c + 1, self.hits.len()),
                None => format!("{}{more} matches", self.hits.len()),
            }
        };
        let has_hits = !self.hits.is_empty();
        let bar = row![
            icons::glyph::<Message>(Icon::Search, true),
            text_input("Find in document", &self.query)
                .id(input_id())
                .on_input(move |q| ev(Event::Query(q)))
                .on_submit(ev(Event::Next))
                .width(200),
            text(status).size(12).width(90).style(text::secondary),
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
            icons::button(
                Icon::FindPrev,
                "Previous match (Shift+F3)",
                has_hits.then(|| ev(Event::Prev))
            ),
            icons::button(
                Icon::FindNext,
                "Next match (Enter or F3)",
                has_hits.then(|| ev(Event::Next))
            ),
            icons::button(Icon::Close, "Close (Esc)", Some(ev(Event::Close))),
        ]
        .spacing(4)
        .align_y(Center);
        container(bar)
            .padding([4, 8])
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style::default()
                    .background(palette.background.base.color)
                    .border(iced::Border {
                        color: palette.background.strong.color,
                        width: 1.0,
                        radius: 8.0.into(),
                    })
            })
            .into()
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
