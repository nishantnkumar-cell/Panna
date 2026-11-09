//! Redacting: marking areas of pages, then taking everything in them out of the file for good
//! and covering them in black, as a page change that can be undone until the file is closed.

use iced::widget::{button, checkbox, column, container, pin, row, space, text};
use iced::{Center, Element, Theme};
use panna_core::{Leftovers, TextBox};

use crate::Message;
use crate::icons::{self, Icon};
use crate::selection::PageMap;

/// A drag shorter than this, in points, is a click.
const CLICK_SLOP: f32 = 3.0;
/// Marks kept at most.
const MAX_MARKS: usize = 10_000;

#[derive(Debug, Clone)]
pub enum Event {
    Toggle,
    /// Marks the text that Find matched.
    MarkResults,
    UndoMark,
    Clear,
    /// Asks before redacting.
    Apply,
    Confirm,
    Cancel,
    /// What else in the file mentions the marked text, for the question.
    Found(Leftovers),
    /// Whether to take out the bookmarks and comments that mention it too.
    Scrub(bool),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Mark {
    page: usize,
    rect: TextBox,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    page: usize,
    from: (f32, f32),
    to: (f32, f32),
}

impl Drag {
    fn rect(&self) -> TextBox {
        TextBox {
            x0: self.from.0.min(self.to.0),
            y0: self.from.1.min(self.to.1),
            x1: self.from.0.max(self.to.0),
            y1: self.from.1.max(self.to.1),
        }
    }
}

#[derive(Debug, Default)]
pub struct Redactor {
    pub on: bool,
    marks: Vec<Mark>,
    drag: Option<Drag>,
    confirming: bool,
    /// Bookmarks and comments that mention the marked text, once looked for.
    leftovers: Option<Leftovers>,
    /// Take those out too (the default).
    keep_mentions: bool,
}

/// What the main view does next.
pub enum Outcome {
    Nothing,
    /// Find's matches are wanted as marks.
    MarkResults,
    /// Look for the text in these areas elsewhere in the file, for the question.
    LookFor(Vec<(usize, Vec<TextBox>)>),
    /// Redact these areas, page by page, and the comments and bookmark titles that mention
    /// these phrases.
    Apply(Vec<(usize, Vec<TextBox>)>, Vec<String>),
}

impl Redactor {
    /// Whether a drag or the question is under way, for Esc.
    pub fn busy(&self) -> bool {
        self.drag.is_some() || self.confirming
    }

    /// Stops the drag or the question, keeping the marks.
    pub fn cancel(&mut self) {
        self.drag = None;
        self.confirming = false;
    }

    /// Leaves redacting, forgetting the marks.
    pub fn stop(&mut self) {
        *self = Self::default();
    }

    pub fn update(&mut self, event: Event) -> Outcome {
        match event {
            Event::Toggle => {
                let on = !self.on;
                self.stop();
                self.on = on;
            }
            Event::MarkResults => return Outcome::MarkResults,
            Event::UndoMark => {
                self.marks.pop();
            }
            Event::Clear => self.marks.clear(),
            Event::Apply => {
                self.confirming = !self.marks.is_empty();
                self.leftovers = None;
                self.keep_mentions = false;
                if self.confirming {
                    return Outcome::LookFor(self.areas());
                }
            }
            Event::Cancel => self.cancel(),
            Event::Found(leftovers) => {
                if self.confirming {
                    self.leftovers = Some(leftovers);
                }
            }
            Event::Scrub(on) => self.keep_mentions = !on,
            Event::Confirm => {
                if !self.confirming {
                    return Outcome::Nothing;
                }
                // The marks stay until the pages change, in case redacting fails.
                self.confirming = false;
                let scrub = match self.leftovers.take() {
                    Some(l) if !self.keep_mentions && l.bookmarks + l.comments > 0 => l.phrases,
                    _ => Vec::new(),
                };
                return Outcome::Apply(self.areas(), scrub);
            }
        }
        Outcome::Nothing
    }

    /// The marks, page by page, in page order.
    fn areas(&self) -> Vec<(usize, Vec<TextBox>)> {
        let mut areas: Vec<(usize, Vec<TextBox>)> = Vec::new();
        let mut marks = self.marks.clone();
        marks.sort_by_key(|m| m.page);
        for mark in marks {
            match areas.last_mut() {
                Some((page, rects)) if *page == mark.page => rects.push(mark.rect),
                _ => areas.push((mark.page, vec![mark.rect])),
            }
        }
        areas
    }

    /// Marks these areas of page `page`, leaving out ones already marked.
    pub fn mark(&mut self, page: usize, rects: impl IntoIterator<Item = TextBox>) {
        for rect in rects {
            let mark = Mark { page, rect };
            if self.marks.len() < MAX_MARKS && !self.marks.contains(&mark) {
                self.marks.push(mark);
            }
        }
    }

    pub fn press(&mut self, page: usize, at: (f32, f32)) {
        if !self.confirming {
            self.drag = Some(Drag {
                page,
                from: at,
                to: at,
            });
        }
    }

    pub fn hover(&mut self, page: usize, at: (f32, f32)) {
        if let Some(drag) = self.drag.as_mut().filter(|d| d.page == page) {
            drag.to = at;
        }
    }

    /// The end of a drag marks its area; a click on a mark takes it away.
    pub fn release(&mut self) {
        let Some(drag) = self.drag.take() else { return };
        let rect = drag.rect();
        if rect.width() >= CLICK_SLOP && rect.height() >= CLICK_SLOP {
            self.mark(drag.page, [rect]);
        } else if let Some(i) = self
            .marks
            .iter()
            .rposition(|m| m.page == drag.page && m.rect.contains(drag.from.0, drag.from.1))
        {
            self.marks.remove(i);
        }
    }

    /// The marks on page `page`, and the box being dragged, drawn over it.
    pub fn overlays(&self, page: usize, map: PageMap) -> Vec<Element<'_, Message>> {
        let dragged = self.drag.filter(|d| d.page == page).map(|d| d.rect());
        self.marks
            .iter()
            .filter(|m| m.page == page)
            .map(|m| m.rect)
            .chain(dragged)
            .map(|rect| {
                let (x, y, w, h) = map.rect(&rect);
                pin(container(space())
                    .width(w.max(1.0))
                    .height(h.max(1.0))
                    .style(|theme: &Theme| {
                        let danger = theme.extended_palette().danger.base.color;
                        container::Style::default()
                            .background(danger.scale_alpha(0.18))
                            .border(iced::Border {
                                color: danger,
                                width: 1.5,
                                radius: 0.0.into(),
                            })
                    }))
                .x(x)
                .y(y)
                .into()
            })
            .collect()
    }

    /// The floating bar: marking Find's matches, taking marks back, and redacting.
    pub fn bar(&self, has_results: bool) -> Element<'_, Message> {
        let ev = Message::Redact;
        let marked = !self.marks.is_empty();
        let hint = match self.marks.len() {
            0 => "Drag over what to remove. Click a mark to take it back.".to_owned(),
            1 => "1 area marked.".to_owned(),
            n => format!("{n} areas marked."),
        };
        let content = row![
            icons::button(
                Icon::MarkResults,
                "Mark the text Find matched",
                has_results.then_some(ev(Event::MarkResults))
            ),
            icons::button(
                Icon::Undo,
                "Take back the last mark",
                marked.then_some(ev(Event::UndoMark))
            ),
            icons::button(
                Icon::Delete,
                "Clear every mark",
                marked.then_some(ev(Event::Clear))
            ),
            text(hint).size(13).width(330),
            icons::button(
                Icon::Check,
                "Remove what's marked, for good…",
                marked.then_some(ev(Event::Apply))
            ),
            icons::button(Icon::Close, "Stop redacting (Esc)", Some(ev(Event::Toggle))),
        ]
        .spacing(6)
        .align_y(Center);
        container(content)
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

    /// The question before redacting.
    pub fn card(&self) -> Option<Element<'_, Message>> {
        if !self.confirming {
            return None;
        }
        let ev = Message::Redact;
        let areas = self.areas();
        let marks = self.marks.len();
        let what = match (marks, areas.len()) {
            (1, _) => "the marked area".to_owned(),
            (m, 1) => format!("the {m} marked areas"),
            (m, p) => format!("the {m} marked areas on {p} pages"),
        };
        let elsewhere = self.leftovers.as_ref().and_then(|l| {
            let count = |n: usize, one: &str, many: &str| match n {
                0 => None,
                1 => Some(format!("1 {one}")),
                n => Some(format!("{n} {many}")),
            };
            let found: Vec<String> = [
                count(l.bookmarks, "bookmark", "bookmarks"),
                count(l.comments, "comment", "comments"),
            ]
            .into_iter()
            .flatten()
            .collect();
            (!found.is_empty()).then(|| {
                checkbox(!self.keep_mentions)
                    .label(format!(
                        "Also remove the marked text from {} elsewhere in the file \
                         (comments that mention it are deleted)",
                        found.join(" and ")
                    ))
                    .on_toggle(|on| Message::Redact(Event::Scrub(on)))
                    .size(16)
                    .text_size(13)
            })
        });
        let body = column![
            text("Remove marked content for good?").size(16),
            text(format!(
                "Text, pictures and drawings in {what} are taken out of the file and covered \
                 in black. Comments over them go too."
            ))
            .size(13),
        ]
        .push(elsewhere)
        .push(
            text("You can undo this until you close the file. Once it's saved, what was removed can't be recovered from the saved file.")
                .size(12)
                .style(text::secondary),
        )
        .push(
            row![
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(ev(Event::Cancel)),
                button("Remove").style(button::danger).on_press(ev(Event::Confirm)),
            ]
            .spacing(8),
        )
        .spacing(10);
        Some(
            container(container(body).width(380))
                .padding(16)
                .style(container::rounded_box)
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Redactor {
        Redactor {
            on: true,
            ..Redactor::default()
        }
    }

    #[test]
    fn drags_mark_and_clicks_take_marks_back() {
        let mut r = on();
        r.press(0, (10.0, 10.0));
        r.hover(0, (50.0, 30.0));
        r.release();
        r.press(2, (5.0, 5.0));
        r.hover(2, (20.0, 20.0));
        r.release();
        assert_eq!(r.marks.len(), 2);
        // A click away from any mark does nothing; on one, takes it away.
        r.press(0, (80.0, 80.0));
        r.release();
        assert_eq!(r.marks.len(), 2);
        r.press(0, (20.0, 20.0));
        r.release();
        assert_eq!(r.marks.len(), 1);
        assert_eq!(r.marks[0].page, 2);
    }

    #[test]
    fn applying_asks_first_then_hands_over_marks_by_page() {
        let mut r = on();
        let b = |x: f32| TextBox {
            x0: x,
            y0: 0.0,
            x1: x + 10.0,
            y1: 10.0,
        };
        r.mark(3, [b(0.0), b(20.0)]);
        r.mark(1, [b(5.0), b(5.0)]);
        assert_eq!(r.marks.len(), 3);
        assert!(matches!(r.update(Event::Confirm), Outcome::Nothing));
        assert!(matches!(r.update(Event::Apply), Outcome::LookFor(_)));
        assert!(r.card().is_some());
        r.update(Event::Found(Leftovers {
            phrases: vec!["acme".into()],
            bookmarks: 1,
            comments: 0,
        }));
        let Outcome::Apply(areas, scrub) = r.update(Event::Confirm) else {
            panic!("not applied");
        };
        assert_eq!(
            areas.iter().map(|(p, a)| (*p, a.len())).collect::<Vec<_>>(),
            [(1, 1), (3, 2)]
        );
        assert_eq!(scrub, ["acme"]);
        assert!(r.on && !r.busy());
        // Unticked, the mentions stay.
        r.update(Event::Apply);
        r.update(Event::Found(Leftovers {
            phrases: vec!["acme".into()],
            bookmarks: 1,
            comments: 0,
        }));
        r.update(Event::Scrub(false));
        assert!(matches!(r.update(Event::Confirm), Outcome::Apply(_, s) if s.is_empty()));
    }

    #[test]
    fn nothing_marked_asks_nothing() {
        let mut r = on();
        r.update(Event::Apply);
        assert!(!r.busy() && r.card().is_none());
    }
}
