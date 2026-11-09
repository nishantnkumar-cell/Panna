//! Rotating or deleting a range of pages, as Acrobat's Rotate Pages and Delete Pages dialogs
//! do: pages such as 2-5, 9, then for rotating, even or odd pages only and landscape or
//! portrait pages only.

use iced::widget::{button, column, container, pick_list, radio, row, space, text, text_input};
use iced::{Element, Theme};
use panna_core::{PageSize, PageSource};

use crate::Message;
use crate::pages::parse_range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Rotate,
    Delete,
}

/// Which page numbers count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Parity {
    #[default]
    Both,
    Even,
    Odd,
}

impl Parity {
    const ALL: [Parity; 3] = [Parity::Both, Parity::Even, Parity::Odd];

    fn keeps(self, index: usize) -> bool {
        // Page numbers count from 1: index 0 is page 1, an odd page.
        match self {
            Parity::Both => true,
            Parity::Even => !index.is_multiple_of(2),
            Parity::Odd => index.is_multiple_of(2),
        }
    }
}

impl std::fmt::Display for Parity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Parity::Both => "Even and odd pages",
            Parity::Even => "Even pages only",
            Parity::Odd => "Odd pages only",
        })
    }
}

/// Which way the pages face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Any,
    Landscape,
    Portrait,
}

impl Orientation {
    const ALL: [Orientation; 3] = [Orientation::Any, Orientation::Landscape, Orientation::Portrait];

    fn keeps(self, size: PageSize) -> bool {
        match self {
            Orientation::Any => true,
            Orientation::Landscape => size.width > size.height,
            Orientation::Portrait => size.width <= size.height,
        }
    }
}

impl std::fmt::Display for Orientation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Orientation::Any => "Pages of either orientation",
            Orientation::Landscape => "Landscape pages",
            Orientation::Portrait => "Portrait pages",
        })
    }
}

/// The card, as filled in so far.
#[derive(Debug, Clone)]
pub struct PageRange {
    pub action: Action,
    pub pages: String,
    /// Quarter turns clockwise: 1, 2 or 3.
    pub turns: u8,
    pub parity: Parity,
    pub orientation: Orientation,
}

#[derive(Debug, Clone)]
pub enum Event {
    Pages(String),
    Turns(u8),
    Parity(Parity),
    Orientation(Orientation),
    Go,
    Cancel,
}

impl PageRange {
    /// Starts on the page shown, as Acrobat's dialogs do.
    pub fn new(action: Action, current: usize) -> Self {
        Self {
            action,
            pages: (current + 1).to_string(),
            turns: 1,
            parity: Parity::Both,
            orientation: Orientation::Any,
        }
    }

    /// The pages picked, from 0, each once and in order.
    pub fn picked(&self, sizes: &[PageSize]) -> Result<Vec<usize>, String> {
        let mut pages = parse_range(&self.pages, sizes.len())?;
        pages.sort_unstable();
        pages.dedup();
        if self.action == Action::Rotate {
            pages.retain(|&i| {
                self.parity.keeps(i) && sizes.get(i).is_some_and(|s| self.orientation.keeps(*s))
            });
            if pages.is_empty() {
                return Err("None of those pages match.".into());
            }
        }
        Ok(pages)
    }

    /// The new page list, and the page to show after.
    pub fn plan(&self, sizes: &[PageSize]) -> Result<(Vec<PageSource>, usize), String> {
        let picked = self.picked(sizes)?;
        let count = sizes.len();
        match self.action {
            Action::Rotate => {
                let turns = self.turns.clamp(1, 3);
                let plan = (0..count)
                    .map(|index| PageSource::Page {
                        doc: 0,
                        index,
                        turns: if picked.binary_search(&index).is_ok() {
                            turns
                        } else {
                            0
                        },
                    })
                    .collect();
                Ok((plan, picked.first().copied().unwrap_or(0)))
            }
            Action::Delete => {
                if picked.len() >= count {
                    return Err("A document needs at least one page, so not every page can go.".into());
                }
                let plan: Vec<PageSource> = (0..count)
                    .filter(|i| picked.binary_search(i).is_err())
                    .map(|index| PageSource::Page {
                        doc: 0,
                        index,
                        turns: 0,
                    })
                    .collect();
                let first = picked.first().copied().unwrap_or(0);
                Ok((plan, first.min(count - picked.len() - 1)))
            }
        }
    }
}

pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("page-range")
}

/// The Rotate pages or Delete pages card.
pub fn card<'a>(state: &PageRange, sizes: &[PageSize]) -> Element<'a, Message> {
    let ev = Message::PageRange;
    let checked = state.plan(sizes);
    let (title, verb) = match state.action {
        Action::Rotate => ("Rotate pages", "Rotate"),
        Action::Delete => ("Delete pages", "Delete"),
    };
    let summary: Element<'a, Message> = match &checked {
        Ok(_) if state.action == Action::Delete => {
            let n = state.picked(sizes).map_or(0, |p| p.len());
            text(if n == 1 {
                "1 page will be deleted. Undo brings it back until the file is closed.".to_owned()
            } else {
                format!("{n} pages will be deleted. Undo brings them back until the file is closed.")
            })
            .size(13)
            .into()
        }
        Ok(_) => {
            let n = state.picked(sizes).map_or(0, |p| p.len());
            text(if n == 1 {
                "1 page will turn.".to_owned()
            } else {
                format!("{n} pages will turn.")
            })
            .size(13)
            .into()
        }
        Err(e) => text(e.clone())
            .size(13)
            .style(|theme: &Theme| text::Style {
                color: Some(theme.extended_palette().danger.base.color),
            })
            .into(),
    };
    let mut body = column![
        text(title).size(16),
        row![
            text("Pages").width(70),
            text_input("1-3, 5, 8-", &state.pages)
                .id(input_id())
                .on_input(move |v| ev(Event::Pages(v)))
                .on_submit(ev(Event::Go))
                .width(200),
            button("All")
                .style(button::secondary)
                .on_press(ev(Event::Pages(format!("1-{}", sizes.len())))),
        ]
        .spacing(8)
        .align_y(iced::Center),
    ]
    .spacing(10)
    .width(400);
    if state.action == Action::Rotate {
        let turns = Some(state.turns);
        body = body
            .push(
                row![
                    radio("Right", 1, turns, move |t| ev(Event::Turns(t))),
                    radio("Upside down", 2, turns, move |t| ev(Event::Turns(t))),
                    radio("Left", 3, turns, move |t| ev(Event::Turns(t))),
                ]
                .spacing(16),
            )
            .push(pick_list(Parity::ALL, Some(state.parity), move |p| {
                ev(Event::Parity(p))
            }))
            .push(pick_list(Orientation::ALL, Some(state.orientation), move |o| {
                ev(Event::Orientation(o))
            }));
    }
    body = body.push(summary).push(
        row![
            space::horizontal(),
            button("Cancel")
                .style(button::secondary)
                .on_press(ev(Event::Cancel)),
            button(verb).on_press_maybe(checked.is_ok().then_some(ev(Event::Go))),
        ]
        .spacing(8),
    );
    container(body).padding(16).style(container::rounded_box).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes() -> Vec<PageSize> {
        // Pages 1 and 3 portrait, 2 and 4 landscape.
        (0..4usize)
            .map(|i| {
                if i.is_multiple_of(2) {
                    PageSize {
                        width: 612.0,
                        height: 792.0,
                    }
                } else {
                    PageSize {
                        width: 792.0,
                        height: 612.0,
                    }
                }
            })
            .collect()
    }

    #[test]
    fn rotating_keeps_only_matching_pages() {
        let mut state = PageRange::new(Action::Rotate, 0);
        state.pages = "1-4".into();
        state.parity = Parity::Even;
        assert_eq!(state.picked(&sizes()).unwrap(), vec![1, 3]);
        state.parity = Parity::Both;
        state.orientation = Orientation::Portrait;
        assert_eq!(state.picked(&sizes()).unwrap(), vec![0, 2]);
        state.parity = Parity::Even;
        assert!(state.picked(&sizes()).is_err());
        state.orientation = Orientation::Landscape;
        state.turns = 2;
        let (plan, show) = state.plan(&sizes()).unwrap();
        assert_eq!(show, 1);
        assert!(matches!(plan[1], PageSource::Page { turns: 2, .. }));
        assert!(matches!(plan[0], PageSource::Page { turns: 0, .. }));
    }

    #[test]
    fn deleting_leaves_at_least_one_page() {
        let mut state = PageRange::new(Action::Delete, 1);
        state.pages = "2-3".into();
        let (plan, show) = state.plan(&sizes()).unwrap();
        assert_eq!(plan.len(), 2);
        assert!(matches!(plan[1], PageSource::Page { index: 3, .. }));
        assert_eq!(show, 1);
        state.pages = "1-4".into();
        assert!(state.plan(&sizes()).is_err());
        state.pages = "4".into();
        assert_eq!(state.plan(&sizes()).unwrap().1, 2);
    }
}
