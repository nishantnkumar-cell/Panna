//! Editing bookmarks in the sidebar's Bookmarks tab: add one for the page on screen, rename,
//! move up and down, indent and outdent, delete. A bookmark moves with the ones under it.
//! Each change is a page change, so it can be undone and is kept when saved.

use iced::widget::{Column, button, column, container, row, scrollable, space, text, text_input};
use iced::{Center, Element, Fill, Length, Theme};
use panna_core::OutlineItem;

use crate::Message;
use crate::find::Find;
use crate::icons::{self, Icon};

const PAD: f32 = 12.0;

#[derive(Debug, Clone)]
pub enum Event {
    /// Picks a bookmark, and goes to its page.
    Select(usize),
    Add,
    Rename,
    Typed(String),
    /// Keeps the name typed.
    Commit,
    Up,
    Down,
    Indent,
    Outdent,
    Delete,
}

/// What the app does after an event.
pub enum Outcome {
    None,
    GoTo(usize),
    /// Replace the bookmarks with these, then select the one at the index.
    Save(Vec<OutlineItem>, Option<usize>),
    /// Put the cursor in the name box.
    Focus,
}

#[derive(Debug, Default)]
pub struct Editor {
    selected: Option<usize>,
    /// The name being typed for the selected bookmark.
    renaming: Option<String>,
}

pub fn name_id() -> iced::widget::Id {
    iced::widget::Id::new("bookmark-name")
}

fn depth(outline: &[OutlineItem], i: usize) -> Option<u16> {
    outline.get(i).map(|b| b.depth)
}

/// The bookmark at `i` and the ones under it, as a range of the list.
fn subtree(outline: &[OutlineItem], i: usize) -> std::ops::Range<usize> {
    let Some(d) = depth(outline, i) else { return i..i };
    let len = outline.iter().skip(i + 1).take_while(|b| b.depth > d).count();
    i..i + 1 + len
}

/// The bookmark before `i` at the same level, under the same parent.
fn previous_sibling(outline: &[OutlineItem], i: usize) -> Option<usize> {
    let d = depth(outline, i)?;
    let j = outline.iter().take(i).rposition(|b| b.depth <= d)?;
    (depth(outline, j) == Some(d)).then_some(j)
}

/// The bookmark after `i`'s subtree at the same level, under the same parent.
fn next_sibling(outline: &[OutlineItem], i: usize) -> Option<usize> {
    let end = subtree(outline, i).end;
    (depth(outline, end)? == depth(outline, i)?).then_some(end)
}

/// `outline` with the bookmarks in `range` moved `by` levels (in when positive).
fn shifted(outline: &[OutlineItem], range: std::ops::Range<usize>, by: i32) -> Vec<OutlineItem> {
    let mut list = outline.to_vec();
    for b in list.iter_mut().take(range.end).skip(range.start) {
        b.depth = u16::try_from(i32::from(b.depth) + by).unwrap_or(0);
    }
    list
}

/// `outline` with the bookmarks in `moved` put at `to` (an index of what remains).
fn moved(outline: &[OutlineItem], moved: std::ops::Range<usize>, to: usize) -> Vec<OutlineItem> {
    let mut list = outline.to_vec();
    let taken: Vec<OutlineItem> = list.drain(moved).collect();
    let to = to.min(list.len());
    list.splice(to..to, taken);
    list
}

impl Editor {
    /// Forgets the selection, as when another document is shown.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// A name is being typed.
    pub fn renaming(&self) -> bool {
        self.renaming.is_some()
    }

    /// The bookmarks changed: keeps the selection only if it's still there.
    pub fn reloaded(&mut self, outline: &[OutlineItem], select: Option<usize>) {
        self.selected = select.or(self.selected).filter(|&i| i < outline.len());
        if self.selected.is_none() {
            self.renaming = None;
        }
    }

    pub fn update(&mut self, event: Event, outline: &[OutlineItem], page: usize) -> Outcome {
        let selected = self.selected.filter(|&i| i < outline.len());
        match event {
            Event::Select(i) => {
                if self.selected != Some(i) {
                    self.renaming = None;
                }
                self.selected = Some(i);
                outline
                    .get(i)
                    .and_then(|b| b.page)
                    .map_or(Outcome::None, Outcome::GoTo)
            }
            Event::Add => {
                // After the selected bookmark and the ones under it, at its level.
                let (at, depth) = match selected.and_then(|i| Some((i, depth(outline, i)?))) {
                    Some((i, d)) => (subtree(outline, i).end, d),
                    None => (outline.len(), 0),
                };
                let mut list = outline.to_vec();
                let title = format!("Page {}", page + 1);
                list.insert(
                    at,
                    OutlineItem {
                        title: title.clone(),
                        depth,
                        page: Some(page),
                        id: None,
                    },
                );
                self.renaming = Some(title);
                Outcome::Save(list, Some(at))
            }
            Event::Rename => match selected.and_then(|i| outline.get(i)) {
                Some(b) => {
                    self.renaming = Some(b.title.clone());
                    Outcome::Focus
                }
                None => Outcome::None,
            },
            Event::Typed(name) => {
                if self.renaming.is_some() {
                    self.renaming = Some(name);
                }
                Outcome::None
            }
            Event::Commit => {
                let (Some(i), Some(name)) = (selected, self.renaming.take()) else {
                    return Outcome::None;
                };
                let name = name.trim();
                let mut list = outline.to_vec();
                match list.get_mut(i) {
                    Some(b) if !name.is_empty() && name != b.title => b.title = name.to_owned(),
                    _ => return Outcome::None,
                }
                Outcome::Save(list, Some(i))
            }
            Event::Up => match selected.and_then(|i| Some((i, previous_sibling(outline, i)?))) {
                Some((i, before)) => Outcome::Save(moved(outline, subtree(outline, i), before), Some(before)),
                None => Outcome::None,
            },
            Event::Down => match selected.and_then(|i| Some((i, next_sibling(outline, i)?))) {
                // Moving it down is moving the next one (with its own) up above it.
                Some((i, after)) => {
                    let next = subtree(outline, after);
                    let len = next.len();
                    Outcome::Save(moved(outline, next, i), Some(i + len))
                }
                None => Outcome::None,
            },
            Event::Indent => match selected.filter(|&i| previous_sibling(outline, i).is_some()) {
                Some(i) => Outcome::Save(shifted(outline, subtree(outline, i), 1), Some(i)),
                None => Outcome::None,
            },
            Event::Outdent => match selected.filter(|&i| depth(outline, i).is_some_and(|d| d > 0)) {
                Some(i) => Outcome::Save(shifted(outline, subtree(outline, i), -1), Some(i)),
                None => Outcome::None,
            },
            Event::Delete => {
                let Some(i) = selected else { return Outcome::None };
                let mut list = outline.to_vec();
                list.drain(subtree(outline, i));
                self.selected = None;
                self.renaming = None;
                let select = list.len().checked_sub(1).map(|last| i.min(last));
                Outcome::Save(list, select)
            }
        }
    }

    /// The Bookmarks tab: the tools, then the bookmarks. `busy` while a change is saved.
    pub fn view<'a>(
        &'a self,
        outline: &'a [OutlineItem],
        find: &Find,
        found_in: impl Fn(usize) -> bool,
        busy: bool,
    ) -> Element<'a, Message> {
        let ev = |e: Event| (!busy).then_some(Message::Bookmarks(e));
        let selected = self.selected.filter(|&i| i < outline.len());
        let on = |e: Event, ok: bool| if ok { ev(e) } else { None };
        let tools = row![
            icons::button(Icon::BookmarkAdd, "Add a bookmark for this page", ev(Event::Add)),
            icons::button(
                Icon::Rename,
                "Rename bookmark",
                on(Event::Rename, selected.is_some())
            ),
            icons::button(
                Icon::MoveUp,
                "Move bookmark up",
                on(
                    Event::Up,
                    selected.is_some_and(|i| previous_sibling(outline, i).is_some())
                )
            ),
            icons::button(
                Icon::MoveDown,
                "Move bookmark down",
                on(
                    Event::Down,
                    selected.is_some_and(|i| next_sibling(outline, i).is_some())
                )
            ),
            icons::button(
                Icon::Outdent,
                "Move bookmark out a level",
                on(
                    Event::Outdent,
                    selected.is_some_and(|i| depth(outline, i).is_some_and(|d| d > 0))
                )
            ),
            icons::button(
                Icon::Indent,
                "Move bookmark in, under the one above",
                on(
                    Event::Indent,
                    selected.is_some_and(|i| previous_sibling(outline, i).is_some())
                )
            ),
            space::horizontal(),
            icons::button(
                Icon::Delete,
                "Delete bookmark and the ones under it",
                on(Event::Delete, selected.is_some())
            ),
        ]
        .spacing(2)
        .padding([4, 6])
        .align_y(Center);
        if outline.is_empty() {
            let note = text("This document has no bookmarks.")
                .size(13)
                .style(text::secondary);
            return column![tools, container(note).padding(PAD)].into();
        }
        let searching = find.active();
        let items = outline.iter().enumerate().map(|(i, item)| {
            let indent = f32::from(item.depth.min(8)) * 12.0;
            let picked = selected == Some(i);
            if let (true, Some(name)) = (picked, &self.renaming) {
                let input = text_input("Bookmark name", name)
                    .id(name_id())
                    .size(13)
                    .padding([3, 6])
                    .on_input(|s| Message::Bookmarks(Event::Typed(s)))
                    .on_submit(Message::Bookmarks(Event::Commit));
                return row![space().width(indent), input].into();
            }
            let title = if item.title.is_empty() {
                "(untitled)"
            } else {
                item.title.as_str()
            };
            // Like Word's Headings tab, sections that contain search results are highlighted.
            let found = searching && found_in(i);
            let label = text(title).size(13).wrapping(text::Wrapping::WordOrGlyph);
            row![
                space().width(indent),
                button(label)
                    .padding([3, 6])
                    .width(Length::Fill)
                    .style(move |theme: &Theme, status| {
                        let palette = theme.extended_palette();
                        let mut style = button::text(theme, status);
                        if picked {
                            style.background = Some(palette.background.strong.color.into());
                        } else if found {
                            style.background = Some(palette.primary.weak.color.into());
                            style.text_color = palette.primary.weak.text;
                        }
                        style
                    })
                    .on_press(Message::Bookmarks(Event::Select(i))),
            ]
            .into()
        });
        column![
            tools,
            scrollable(
                Column::with_children(items)
                    .spacing(1)
                    .padding([4, 6])
                    .width(Fill),
            )
            .height(Fill),
        ]
        .into()
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn list(depths: &[u16]) -> Vec<OutlineItem> {
        depths
            .iter()
            .enumerate()
            .map(|(i, &depth)| OutlineItem {
                title: i.to_string(),
                depth,
                page: Some(i),
                id: None,
            })
            .collect()
    }

    fn shape(outcome: Outcome) -> (Vec<(String, u16)>, Option<usize>) {
        match outcome {
            Outcome::Save(list, select) => (list.into_iter().map(|b| (b.title, b.depth)).collect(), select),
            _ => panic!("nothing saved"),
        }
    }

    fn owned(pairs: &[(&str, u16)]) -> Vec<(String, u16)> {
        pairs.iter().map(|(t, d)| ((*t).to_owned(), *d)).collect()
    }

    #[test]
    fn bookmarks_move_with_the_ones_under_them() {
        // 0, 1 (under it 2), 3
        let outline = list(&[0, 0, 1, 0]);
        let mut editor = Editor::default();
        editor.update(Event::Select(1), &outline, 0);
        let (moved, select) = shape(editor.update(Event::Up, &outline, 0));
        assert_eq!(moved, owned(&[("1", 0), ("2", 1), ("0", 0), ("3", 0)]));
        assert_eq!(select, Some(0));
        let (moved, select) = shape(editor.update(Event::Down, &outline, 0));
        assert_eq!(moved, owned(&[("0", 0), ("3", 0), ("1", 0), ("2", 1)]));
        assert_eq!(select, Some(2));
        let (moved, _) = shape(editor.update(Event::Indent, &outline, 0));
        assert_eq!(moved, owned(&[("0", 0), ("1", 1), ("2", 2), ("3", 0)]));
        let (moved, select) = shape(editor.update(Event::Delete, &outline, 0));
        assert_eq!(moved, owned(&[("0", 0), ("3", 0)]));
        assert_eq!(select, Some(1));
    }

    #[test]
    fn the_first_child_cant_move_up_or_in() {
        let outline = list(&[0, 1, 1]);
        let mut editor = Editor::default();
        editor.update(Event::Select(1), &outline, 0);
        assert!(matches!(editor.update(Event::Up, &outline, 0), Outcome::None));
        assert!(matches!(editor.update(Event::Indent, &outline, 0), Outcome::None));
        let (moved, _) = shape(editor.update(Event::Outdent, &outline, 0));
        assert_eq!(moved, owned(&[("0", 0), ("1", 0), ("2", 1)]));
    }

    #[test]
    fn added_bookmarks_go_after_the_selected_one_and_are_named() {
        let outline = list(&[0, 1, 0]);
        let mut editor = Editor::default();
        editor.update(Event::Select(0), &outline, 0);
        let (added, select) = shape(editor.update(Event::Add, &outline, 6));
        assert_eq!(added[2], ("Page 7".to_owned(), 0));
        assert_eq!(select, Some(2));
        // The new one is named at once.
        let mut with_new = outline.clone();
        with_new.insert(
            2,
            OutlineItem {
                title: "Page 7".into(),
                depth: 0,
                page: Some(6),
                id: None,
            },
        );
        editor.reloaded(&with_new, Some(2));
        editor.update(Event::Typed("Appendix".into()), &with_new, 6);
        let (renamed, _) = shape(editor.update(Event::Commit, &with_new, 6));
        assert_eq!(renamed[2].0, "Appendix");
    }
}
