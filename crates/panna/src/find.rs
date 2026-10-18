//! The find bar: searches every page's text as it is extracted, and steps through matches.

use std::ops::Range;

use iced::widget::{
    Column, button, column, container, rich_text, row, scrollable, space, span, text, text_input,
};
use iced::{Center, Color, Element, Fill, Theme};
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
    /// Jump to match number `n` (from the Results list).
    Goto(usize),
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

/// Highlight behind matched text in the Results list, like a highlighter pen.
const MATCH: Color = Color::from_rgb8(0xFF, 0xD8, 0x4D);
/// Characters of context shown on each side of a match in the Results list.
const CONTEXT: usize = 40;
/// Results listed at most; the count and stepping still cover every match.
const MAX_LISTED: usize = 500;

pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("find")
}

fn results_id() -> iced::widget::Id {
    iced::widget::Id::new("find-results")
}

fn note(message: &str) -> Element<'_, Message> {
    container(text(message).size(13).style(text::secondary))
        .padding(12)
        .into()
}

/// Joins lines with spaces and trims context to whole words, with an ellipsis where cut.
fn tidy(s: &str, leading: bool) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    if s.chars().count() < CONTEXT {
        return flat;
    }
    match (leading, flat.find(' '), flat.rfind(' ')) {
        (true, Some(cut), _) => format!("…{}", &flat[cut..]),
        (false, _, Some(cut)) if cut > 0 => format!("{}…", &flat[..cut]),
        _ => flat,
    }
}

impl Find {
    pub fn active(&self) -> bool {
        self.open && !self.query.trim().is_empty()
    }

    /// Every match found so far, by page, then position.
    pub fn hits(&self) -> &[Hit] {
        if self.active() { &self.hits } else { &[] }
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

    /// Makes match `index` current. Returns it, or `None` if there is no such match.
    pub fn select(&mut self, index: usize) -> Option<&Hit> {
        let hit = self.hits.get(index)?;
        self.current = Some(index);
        Some(hit)
    }

    /// Number of matches on pages in `pages`.
    pub fn count_in(&self, pages: Range<usize>) -> usize {
        let start = self.hits.partition_point(|h| h.page < pages.start);
        let end = self.hits.partition_point(|h| h.page < pages.end);
        end.saturating_sub(start)
    }

    /// "Result 3 of 27", "Searching… 40%", "No matches", as in Word's navigation pane.
    pub fn status(&self, searched: usize, total: usize) -> String {
        if !self.active() {
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
        }
    }

    /// The search box at the top of the navigation pane, with the match count, options and
    /// previous/next buttons underneath.
    pub fn search_box(&self, searched: usize, total: usize) -> Element<'_, Message> {
        let ev = |e: Event| Message::Find(e);
        let has_hits = !self.hits.is_empty();
        let input = text_input("Search document", &self.query)
            .id(input_id())
            .on_input(move |q| ev(Event::Query(q)))
            .on_submit(ev(Event::Next))
            .padding([5, 8])
            .width(Fill);
        let clear = (!self.query.is_empty())
            .then(|| icons::button(Icon::Close, "Clear search (Esc)", Some(ev(Event::Close))));
        let field = row![icons::glyph::<Message>(Icon::Search, true), input]
            .push(clear.unwrap_or_else(|| space().into()))
            .spacing(4)
            .align_y(Center);
        let controls = row![
            text(self.status(searched, total))
                .size(12)
                .style(text::secondary)
                .width(Fill),
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
        column![field, controls].spacing(4).padding([6, 8]).into()
    }

    /// The Results tab: one entry per match with surrounding text, the match highlighted as on the page.
    pub fn results<'a>(&'a self, texts: &'a [TextSlot]) -> Element<'a, Message> {
        if !self.active() {
            return note("Type in the search box to find text in this document.");
        }
        if self.hits.is_empty() {
            return note("No results yet.");
        }
        // ponytail: shows the first MAX_LISTED results; a virtualised list if people need more.
        let shown = self.hits.iter().enumerate().take(MAX_LISTED).map(|(i, hit)| {
            let Some(page) = texts.get(hit.page).and_then(|t| t.text.as_ref()) else {
                return space().into();
            };
            let before = page.slice(hit.range.start.saturating_sub(CONTEXT)..hit.range.start);
            let after = page.slice(hit.range.end..hit.range.end + CONTEXT);
            let snippet: text::Rich<'_, (), Message> = rich_text![
                span(tidy(&before, true)),
                span(page.slice(hit.range.clone()).replace('\n', " "))
                    .background(MATCH)
                    .color(Color::BLACK),
                span(tidy(&after, false)),
            ]
            .size(13)
            .wrapping(text::Wrapping::WordOrGlyph);
            let current = self.current == Some(i);
            button(
                column![
                    text(format!("Page {}", hit.page + 1))
                        .size(11)
                        .style(text::secondary),
                    snippet
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
            .on_press(Message::Find(Event::Goto(i)))
            .into()
        });
        let mut list = Column::with_children(shown)
            .spacing(2)
            .padding([4, 6])
            .width(Fill);
        if self.hits.len() > MAX_LISTED {
            list = list.push(
                text(format!(
                    "Showing the first {MAX_LISTED} of {} results.",
                    self.hits.len()
                ))
                .size(12)
                .style(text::secondary),
            );
        }
        scrollable(list).id(results_id()).height(Fill).into()
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
    fn counts_matches_in_a_page_range() {
        let f = find_with(&[(1, 0), (4, 0), (4, 9), (8, 2)]);
        assert_eq!(f.count_in(0..4), 1);
        assert_eq!(f.count_in(4..5), 2);
        assert_eq!(f.count_in(2..100), 3);
        assert_eq!(f.count_in(5..5), 0);
    }

    #[test]
    fn snippets_cut_at_word_boundaries() {
        let long = "alpha beta gamma delta epsilon zeta eta theta";
        assert_eq!(tidy(long, true), "… beta gamma delta epsilon zeta eta theta");
        assert_eq!(tidy(long, false), "alpha beta gamma delta epsilon zeta eta…");
        assert_eq!(tidy("short\nline", true), "short line");
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
