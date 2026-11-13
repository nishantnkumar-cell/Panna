//! The Check spelling card, as Acrobat's Check Spelling (F7): goes through the words in
//! comments and form fields that aren't in the dictionary, one at a time, offering words
//! that may have been meant. Each can be changed, ignored once or everywhere, or added to
//! the personal dictionary.

use std::collections::HashSet;
use std::ops::Range;

use iced::widget::{Column, button, column, container, rich_text, row, space, span, text, text_input};
use iced::{Center, Color, Element, Fill};
use panna_core::spell::{Dictionary, misspelled};

use crate::Message;
use crate::annotate::Picked;

/// Characters of text shown each side of the word.
const CONTEXT: usize = 40;
const MARK: Color = Color::from_rgb8(0xFF, 0xB4, 0xB4);

/// Where some text is: a comment on a page, or a form field with its name.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Comment(Picked, usize),
    Field(usize, String),
}

#[derive(Debug, Clone, PartialEq)]
struct Issue {
    /// Index into the sources.
    source: usize,
    range: Range<usize>,
    word: String,
}

#[derive(Debug, Clone)]
pub struct Spelling {
    sources: Vec<(Source, String)>,
    issues: Vec<Issue>,
    current: usize,
    pub change_to: String,
    suggestions: Vec<String>,
    /// Words ignored everywhere, in lower case: the personal dictionary's and those ignored
    /// here.
    ignored: HashSet<String>,
}

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    ChangeTo(String),
    Change,
    Ignore,
    IgnoreAll,
    Add,
    Close,
}

/// What the card asks of the rest of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Nothing,
    /// Give this text its new words.
    Replace(Source, String),
    /// Add this word to the personal dictionary.
    AddWord(String),
}

pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("spelling")
}

impl Spelling {
    pub fn new(sources: Vec<(Source, String)>, words: &[String]) -> Self {
        let mut state = Spelling {
            sources,
            issues: Vec::new(),
            current: 0,
            change_to: String::new(),
            suggestions: Vec::new(),
            ignored: words.iter().map(|w| w.to_lowercase()).collect(),
        };
        state.scan(None);
        state
    }

    /// Finds every misspelling again, then goes to the first after `after` (a source and a
    /// place in its text), or the first of all.
    fn scan(&mut self, after: Option<(usize, usize)>) {
        let dict = Dictionary::english();
        self.issues = self
            .sources
            .iter()
            .enumerate()
            .flat_map(|(source, (_, text))| {
                misspelled(text, dict, &self.ignored)
                    .into_iter()
                    .map(move |range| Issue {
                        source,
                        word: text.get(range.clone()).unwrap_or_default().to_owned(),
                        range,
                    })
            })
            .collect();
        self.current = after.map_or(0, |at| {
            self.issues
                .iter()
                .position(|i| (i.source, i.range.start) >= at)
                .unwrap_or(self.issues.len())
        });
        self.suggest();
    }

    fn suggest(&mut self) {
        match self.issues.get(self.current) {
            Some(issue) => {
                self.suggestions = Dictionary::english().suggest(&issue.word, 6);
                self.change_to = self
                    .suggestions
                    .first()
                    .cloned()
                    .unwrap_or_else(|| issue.word.clone());
            }
            None => {
                self.suggestions.clear();
                self.change_to.clear();
            }
        }
    }

    /// A comment edited by hand or here is now known as `now`.
    pub fn renumber(&mut self, was: Picked, now: Picked) {
        for (source, _) in &mut self.sources {
            if let Source::Comment(picked, _) = source
                && *picked == was
            {
                *picked = now;
            }
        }
    }

    /// Handles `event`; the card closes when this returns `None`.
    pub fn update(mut self, event: Event) -> (Option<Self>, Action) {
        match event {
            Event::Open => {}
            Event::ChangeTo(t) => self.change_to = t.chars().take(200).collect(),
            Event::Ignore => {
                self.current += 1;
                self.suggest();
            }
            Event::IgnoreAll | Event::Add => {
                let Some(issue) = self.issues.get(self.current).cloned() else {
                    return (Some(self), Action::Nothing);
                };
                let word = issue.word.to_lowercase();
                self.ignored.insert(word.clone());
                self.scan(Some((issue.source, issue.range.start)));
                if matches!(event, Event::Add) {
                    return (Some(self), Action::AddWord(word));
                }
            }
            Event::Change => {
                let Some(issue) = self.issues.get(self.current).cloned() else {
                    return (Some(self), Action::Nothing);
                };
                let Some((source, text)) = self.sources.get_mut(issue.source) else {
                    return (Some(self), Action::Nothing);
                };
                let mut new = text.clone();
                if new.get(issue.range.clone()).is_none() {
                    return (Some(self), Action::Nothing);
                }
                new.replace_range(issue.range.clone(), &self.change_to);
                *text = new.clone();
                let source = source.clone();
                // On from just after the new word.
                self.scan(Some((
                    issue.source,
                    issue.range.start + self.change_to.len().max(1),
                )));
                return (Some(self), Action::Replace(source, new));
            }
            Event::Close => return (None, Action::Nothing),
        }
        (Some(self), Action::Nothing)
    }
}

/// The Check spelling card.
pub fn card(state: &Spelling) -> Element<'_, Message> {
    let ev = Message::Spelling;
    let mut body = column![text("Check spelling").size(16)].spacing(10).width(540);
    let Some(issue) = state.issues.get(state.current) else {
        let done = if state.sources.is_empty() {
            "There's no text in comments or form fields to check."
        } else {
            "No more spelling mistakes in comments and form fields."
        };
        body = body.push(text(done).size(13));
        body = body.push(row![
            space::horizontal(),
            button("Done").on_press(ev(Event::Close))
        ]);
        return container(body).padding(16).style(container::rounded_box).into();
    };
    let Some((source, full)) = state.sources.get(issue.source) else {
        return space().into();
    };
    let place = match source {
        Source::Comment(_, page) => format!("In a comment on page {}", page + 1),
        Source::Field(_, name) if name.is_empty() => "In a form field".to_owned(),
        Source::Field(_, name) => format!("In the form field \u{201c}{name}\u{201d}"),
    };
    let flat = |s: &str| s.replace(['\n', '\r', '\t'], " ");
    let before: String = {
        let b = full.get(..issue.range.start).unwrap_or_default();
        let chars: Vec<char> = b.chars().collect();
        let cut = chars.len().saturating_sub(CONTEXT);
        let tail: String = chars.get(cut..).unwrap_or_default().iter().collect();
        if cut > 0 {
            format!("…{}", flat(&tail))
        } else {
            flat(&tail)
        }
    };
    let after: String = {
        let a = full.get(issue.range.end..).unwrap_or_default();
        let head: String = a.chars().take(CONTEXT).collect();
        if a.chars().count() > CONTEXT {
            format!("{}…", flat(&head))
        } else {
            flat(&head)
        }
    };
    let snippet: text::Rich<'_, (), Message> = rich_text![
        span(before),
        span(issue.word.as_str()).background(MARK).color(Color::BLACK),
        span(after),
    ]
    .size(14)
    .wrapping(text::Wrapping::WordOrGlyph);
    body = body.push(
        column![
            text(format!(
                "{place} ({} of {})",
                state.current + 1,
                state.issues.len()
            ))
            .size(12)
            .style(text::secondary),
            container(snippet)
                .padding(8)
                .width(Fill)
                .style(container::bordered_box),
        ]
        .spacing(4),
    );
    body = body.push(
        row![
            text("Change to").size(13).width(80),
            text_input("", &state.change_to)
                .id(input_id())
                .on_input(move |t| ev(Event::ChangeTo(t)))
                .on_submit(ev(Event::Change)),
        ]
        .spacing(6)
        .align_y(Center),
    );
    if state.suggestions.is_empty() {
        body = body.push(text("No suggestions.").size(12).style(text::secondary));
    } else {
        let list = state
            .suggestions
            .iter()
            .fold(Column::new().spacing(2), |list, s| {
                let picked = *s == state.change_to;
                list.push(
                    button(text(s.as_str()).size(13))
                        .width(Fill)
                        .padding([3, 8])
                        .style(if picked { button::secondary } else { button::text })
                        .on_press(ev(Event::ChangeTo(s.clone()))),
                )
            });
        body = body.push(column![text("Suggestions").size(12).style(text::secondary), list].spacing(4));
    }
    let can_change = !state.change_to.is_empty() && state.change_to != issue.word;
    body = body.push(
        row![
            button("Ignore")
                .style(button::secondary)
                .on_press(ev(Event::Ignore)),
            button("Ignore all")
                .style(button::secondary)
                .on_press(ev(Event::IgnoreAll)),
            button("Add to dictionary")
                .style(button::secondary)
                .on_press(ev(Event::Add)),
            space::horizontal(),
            button("Close")
                .style(button::secondary)
                .on_press(ev(Event::Close)),
            button("Change").on_press_maybe(can_change.then_some(ev(Event::Change))),
        ]
        .spacing(6),
    );
    container(body).padding(16).style(container::rounded_box).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mistakes_are_changed_ignored_and_added_in_turn() {
        let sources = vec![
            (Source::Field(0, "Name".into()), "Teh cat sat".to_owned()),
            (
                Source::Field(1, "Note".into()),
                "a recieve and recieve, also qwzx".to_owned(),
            ),
        ];
        let state = Spelling::new(sources, &["qwzx".to_owned()]);
        assert_eq!(state.issues.len(), 3);
        assert_eq!(state.change_to, "The");
        let (state, action) = state.update(Event::Change);
        assert_eq!(
            action,
            Action::Replace(Source::Field(0, "Name".into()), "The cat sat".into())
        );
        let state = state.unwrap();
        assert_eq!(
            state.issues.get(state.current).map(|i| i.word.as_str()),
            Some("recieve")
        );
        // Ignoring everywhere skips the second one too, and a word added is reported.
        let (state, action) = state.update(Event::Add);
        assert_eq!(action, Action::AddWord("recieve".into()));
        let state = state.unwrap();
        assert!(state.issues.get(state.current).is_none());
    }
}
