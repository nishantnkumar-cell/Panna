//! Editing bookmarks in the sidebar's Bookmarks tab: add one for the page on screen, rename,
//! move up and down, indent and outdent, delete, and, as in Acrobat's bookmark properties,
//! colour a title and make it bold or italic. A bookmark moves with the ones under it.
//! Each change is a page change, so it can be undone and is kept when saved. Bookmarks with
//! others under them open and close, and can all be expanded or collapsed at once; the open
//! state is saved with the next change.

use std::collections::HashSet;

use iced::widget::{Column, button, column, container, pick_list, row, scrollable, space, text, text_input};
use iced::{Center, Element, Fill, Font, Length, Theme, font};
use panna_core::{ObjRef, OutlineItem};

use crate::Message;
use crate::find::Find;
use crate::icons::{self, Icon};

const PAD: f32 = 12.0;

#[derive(Debug, Clone)]
pub enum Event {
    /// Picks a bookmark, and goes to its page.
    Select(usize),
    Add,
    /// Adds one named after the selected text, as Acrobat does.
    AddTitled(String),
    Rename,
    Typed(String),
    /// Keeps the name typed.
    Commit,
    Up,
    Down,
    Indent,
    Outdent,
    Delete,
    /// Opens or closes the bookmark at the index.
    Toggle(usize),
    ExpandAll,
    CollapseAll,
    /// Shows or hides the selected bookmark's appearance.
    Properties,
    Style(Style),
    Color(Option<[u8; 3]>),
}

/// A title's text style, as in Acrobat's bookmark properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Bold,
    Italic,
    BoldItalic,
}

impl Style {
    pub const ALL: [Style; 4] = [Style::Plain, Style::Bold, Style::Italic, Style::BoldItalic];

    fn of(item: &OutlineItem) -> Self {
        match (item.bold, item.italic) {
            (false, false) => Style::Plain,
            (true, false) => Style::Bold,
            (false, true) => Style::Italic,
            (true, true) => Style::BoldItalic,
        }
    }
}

impl std::fmt::Display for Style {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Style::Plain => "Plain",
            Style::Bold => "Bold",
            Style::Italic => "Italic",
            Style::BoldItalic => "Bold & italic",
        })
    }
}

/// The colours offered for titles; `None` is the reader's own.
pub const COLORS: [(Option<[u8; 3]>, &str); 7] = [
    (None, "Default"),
    (Some([204, 0, 0]), "Red"),
    (Some([230, 115, 0]), "Orange"),
    (Some([0, 140, 50]), "Green"),
    (Some([0, 80, 200]), "Blue"),
    (Some([130, 40, 170]), "Purple"),
    (Some([120, 120, 120]), "Grey"),
];

/// Names a bookmark while bookmarks are edited: by its object, or by its place for one not
/// saved yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Obj(ObjRef),
    At(usize),
}

fn key(outline: &[OutlineItem], i: usize) -> Key {
    outline.get(i).and_then(|b| b.id).map_or(Key::At(i), Key::Obj)
}

/// Whether bookmarks sit under the one at `i`.
fn has_children(outline: &[OutlineItem], i: usize) -> bool {
    subtree(outline, i).len() > 1
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
    /// The closed bookmarks, once one is opened or closed here; until then, as the file says.
    collapsed: Option<HashSet<Key>>,
    /// The appearance row is shown.
    styling: bool,
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

    /// Whether the bookmark at `i` is shown closed.
    fn closed(&self, outline: &[OutlineItem], i: usize) -> bool {
        match &self.collapsed {
            Some(set) => set.contains(&key(outline, i)),
            None => outline.get(i).is_some_and(|b| b.closed),
        }
    }

    /// The closed bookmarks, taken from the file the first time they're changed here.
    fn collapsed_mut(&mut self, outline: &[OutlineItem]) -> &mut HashSet<Key> {
        self.collapsed.get_or_insert_with(|| {
            (0..outline.len())
                .filter(|&i| outline.get(i).is_some_and(|b| b.closed))
                .map(|i| key(outline, i))
                .collect()
        })
    }

    /// Saves `list`, with each bookmark's open state as shown.
    fn save(&self, mut list: Vec<OutlineItem>, select: Option<usize>) -> Outcome {
        if let Some(set) = &self.collapsed {
            let keys: Vec<Key> = (0..list.len()).map(|i| key(&list, i)).collect();
            for (item, k) in list.iter_mut().zip(keys) {
                item.closed = set.contains(&k);
            }
        }
        Outcome::Save(list, select)
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
            Event::Add | Event::AddTitled(_) => {
                // After the selected bookmark and the ones under it, at its level.
                let (at, depth) = match selected.and_then(|i| Some((i, depth(outline, i)?))) {
                    Some((i, d)) => (subtree(outline, i).end, d),
                    None => (outline.len(), 0),
                };
                let mut list = outline.to_vec();
                let title = match event {
                    Event::AddTitled(name) if !name.trim().is_empty() => name
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(200)
                        .collect(),
                    _ => format!("Page {}", page + 1),
                };
                list.insert(
                    at,
                    OutlineItem {
                        title: title.clone(),
                        depth,
                        page: Some(page),
                        id: None,
                        ..Default::default()
                    },
                );
                self.renaming = Some(title);
                self.save(list, Some(at))
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
                self.save(list, Some(i))
            }
            Event::Up => match selected.and_then(|i| Some((i, previous_sibling(outline, i)?))) {
                Some((i, before)) => self.save(moved(outline, subtree(outline, i), before), Some(before)),
                None => Outcome::None,
            },
            Event::Down => match selected.and_then(|i| Some((i, next_sibling(outline, i)?))) {
                // Moving it down is moving the next one (with its own) up above it.
                Some((i, after)) => {
                    let next = subtree(outline, after);
                    let len = next.len();
                    self.save(moved(outline, next, i), Some(i + len))
                }
                None => Outcome::None,
            },
            Event::Indent => match selected.filter(|&i| previous_sibling(outline, i).is_some()) {
                Some(i) => self.save(shifted(outline, subtree(outline, i), 1), Some(i)),
                None => Outcome::None,
            },
            Event::Outdent => match selected.filter(|&i| depth(outline, i).is_some_and(|d| d > 0)) {
                Some(i) => self.save(shifted(outline, subtree(outline, i), -1), Some(i)),
                None => Outcome::None,
            },
            Event::Delete => {
                let Some(i) = selected else { return Outcome::None };
                let mut list = outline.to_vec();
                list.drain(subtree(outline, i));
                self.selected = None;
                self.renaming = None;
                let select = list.len().checked_sub(1).map(|last| i.min(last));
                self.save(list, select)
            }
            Event::Toggle(i) => {
                if has_children(outline, i) {
                    let k = key(outline, i);
                    let set = self.collapsed_mut(outline);
                    if !set.remove(&k) {
                        set.insert(k);
                    }
                }
                Outcome::None
            }
            Event::ExpandAll => {
                self.collapsed = Some(HashSet::new());
                Outcome::None
            }
            Event::CollapseAll => {
                self.collapsed = Some(
                    (0..outline.len())
                        .filter(|&i| has_children(outline, i))
                        .map(|i| key(outline, i))
                        .collect(),
                );
                Outcome::None
            }
            Event::Properties => {
                self.styling = !self.styling;
                Outcome::None
            }
            Event::Style(style) => {
                let Some(i) = selected else { return Outcome::None };
                let mut list = outline.to_vec();
                match list.get_mut(i) {
                    Some(b) if Style::of(b) != style => {
                        b.bold = matches!(style, Style::Bold | Style::BoldItalic);
                        b.italic = matches!(style, Style::Italic | Style::BoldItalic);
                    }
                    _ => return Outcome::None,
                }
                self.save(list, Some(i))
            }
            Event::Color(color) => {
                let Some(i) = selected else { return Outcome::None };
                let mut list = outline.to_vec();
                match list.get_mut(i) {
                    Some(b) if b.color != color => b.color = color,
                    _ => return Outcome::None,
                }
                self.save(list, Some(i))
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
            icons::toggle(
                Icon::Properties,
                "Bookmark colour and style",
                self.styling,
                Message::Bookmarks(Event::Properties)
            ),
            icons::button(
                Icon::ExpandAll,
                "Expand all bookmarks",
                on(Event::ExpandAll, outline.iter().enumerate().any(|(i, _)| self.closed(outline, i)))
            ),
            icons::button(
                Icon::CollapseAll,
                "Collapse all bookmarks",
                on(
                    Event::CollapseAll,
                    (0..outline.len()).any(|i| has_children(outline, i) && !self.closed(outline, i))
                )
            ),
            icons::button(
                Icon::Delete,
                "Delete bookmark and the ones under it",
                on(Event::Delete, selected.is_some())
            ),
        ]
        .spacing(2)
        .padding([4, 6])
        .align_y(Center)
        // A narrow sidebar puts the rest on a second line.
        .wrap();
        if outline.is_empty() {
            let note = text("This document has no bookmarks.")
                .size(13)
                .style(text::secondary);
            return column![tools, container(note).padding(PAD)].into();
        }
        let styling = self.styling.then(|| self.appearance(selected.and_then(|i| outline.get(i)), busy));
        let searching = find.active();
        // Bookmarks under a closed one are hidden.
        let mut hidden_until = 0;
        let shown = outline.iter().enumerate().filter(|&(i, _)| {
            if i < hidden_until {
                return false;
            }
            if self.closed(outline, i) {
                hidden_until = subtree(outline, i).end;
            }
            true
        });
        let items = shown.map(|(i, item)| {
            let indent = f32::from(item.depth.min(8)) * 12.0;
            let closed = self.closed(outline, i);
            let opener: Element<'a, Message> = if has_children(outline, i) {
                icons::with_tip(
                    button(icons::glyph(
                        if closed { Icon::NextPage } else { Icon::ChevronDown },
                        true,
                    ))
                    .padding(1)
                    .style(button::text)
                    .on_press(Message::Bookmarks(Event::Toggle(i))),
                    if closed { "Show the bookmarks under it" } else { "Hide the bookmarks under it" },
                )
            } else {
                space().width(20).into()
            };
            let picked = selected == Some(i);
            if let (true, Some(name)) = (picked, &self.renaming) {
                let input = text_input("Bookmark name", name)
                    .id(name_id())
                    .size(13)
                    .padding([3, 6])
                    .on_input(|s| Message::Bookmarks(Event::Typed(s)))
                    .on_submit(Message::Bookmarks(Event::Commit));
                return row![space().width(indent), opener, input].align_y(Center).into();
            }
            let title = if item.title.is_empty() {
                "(untitled)"
            } else {
                item.title.as_str()
            };
            // Like Word's Headings tab, sections that contain search results are highlighted.
            let found = searching && found_in(i);
            let font = Font {
                weight: if item.bold { font::Weight::Bold } else { font::Weight::Normal },
                style: if item.italic { font::Style::Italic } else { font::Style::Normal },
                ..Font::default()
            };
            let mut label = text(title).size(13).font(font).wrapping(text::Wrapping::WordOrGlyph);
            if let Some([r, g, b]) = item.color {
                label = label.color(iced::Color::from_rgb8(r, g, b));
            }
            row![
                space().width(indent),
                opener,
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
            .align_y(Center)
            .into()
        });
        column![
            tools,
            styling,
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

impl Editor {
    /// The selected bookmark's text style and colour.
    fn appearance<'a>(&self, item: Option<&OutlineItem>, busy: bool) -> Element<'a, Message> {
        let Some(item) = item else {
            let note = text("Select a bookmark to change its colour and style.")
                .size(12)
                .style(text::secondary);
            return container(note).padding([2, 10]).into();
        };
        let ev = move |e: Event| (!busy).then_some(Message::Bookmarks(e));
        let style = pick_list(Style::ALL, Some(Style::of(item)), move |s| {
            ev(Event::Style(s)).unwrap_or(Message::Ignore)
        })
        .text_size(12)
        .padding([3, 6])
        .width(110);
        let current = item.color;
        let swatches = COLORS.iter().map(move |&(color, name)| {
            let chosen = current == color;
            let fill = color.map(|[r, g, b]| iced::Color::from_rgb8(r, g, b));
            icons::with_tip(
                button(space().width(16).height(16))
                    .padding(0)
                    .style(move |theme: &Theme, _| {
                        let palette = theme.extended_palette();
                        button::Style {
                            background: Some(fill.unwrap_or(palette.background.base.text).into()),
                            border: iced::Border {
                                color: if chosen {
                                    palette.primary.base.color
                                } else {
                                    palette.background.strong.color
                                },
                                width: if chosen { 3.0 } else { 1.0 },
                                radius: 3.0.into(),
                            },
                            ..button::Style::default()
                        }
                    })
                    .on_press_maybe(ev(Event::Color(color))),
                name,
            )
        });
        row![style]
            .extend(swatches)
            .spacing(4)
            .padding([2, 8])
            .align_y(Center)
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
                ..Default::default()
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
                ..Default::default()
            },
        );
        editor.reloaded(&with_new, Some(2));
        editor.update(Event::Typed("Appendix".into()), &with_new, 6);
        let (renamed, _) = shape(editor.update(Event::Commit, &with_new, 6));
        assert_eq!(renamed[2].0, "Appendix");
    }

    #[test]
    fn bookmarks_open_close_and_keep_that_when_saved() {
        // 0 (under it 1, under that 2), 3
        let mut outline = list(&[0, 1, 2, 0]);
        outline[1].closed = true;
        let mut editor = Editor::default();
        assert!(editor.closed(&outline, 1));
        editor.update(Event::Toggle(0), &outline, 0);
        assert!(editor.closed(&outline, 0) && editor.closed(&outline, 1));
        // One without any under it doesn't close.
        editor.update(Event::Toggle(3), &outline, 0);
        assert!(!editor.closed(&outline, 3));
        editor.update(Event::ExpandAll, &outline, 0);
        assert!((0..4).all(|i| !editor.closed(&outline, i)));
        editor.update(Event::CollapseAll, &outline, 0);
        assert_eq!((0..4).map(|i| editor.closed(&outline, i)).collect::<Vec<_>>(), [true, true, false, false]);
        // The next change saves the open state as shown.
        editor.update(Event::Select(3), &outline, 0);
        match editor.update(Event::Style(Style::Bold), &outline, 0) {
            Outcome::Save(list, _) => {
                assert_eq!(list.iter().map(|b| b.closed).collect::<Vec<_>>(), [true, true, false, false]);
                assert!(list[3].bold && !list[3].italic);
            }
            _ => panic!("nothing saved"),
        }
    }

    #[test]
    fn colour_and_style_change_the_selected_bookmark() {
        let outline = list(&[0, 0]);
        let mut editor = Editor::default();
        assert!(matches!(editor.update(Event::Color(Some([204, 0, 0])), &outline, 0), Outcome::None));
        editor.update(Event::Select(1), &outline, 0);
        match editor.update(Event::Color(Some([204, 0, 0])), &outline, 0) {
            Outcome::Save(list, select) => {
                assert_eq!((list[0].color, list[1].color), (None, Some([204, 0, 0])));
                assert_eq!(select, Some(1));
            }
            _ => panic!("nothing saved"),
        }
        // Choosing what it already has changes nothing.
        assert!(matches!(editor.update(Event::Style(Style::Plain), &outline, 0), Outcome::None));
    }
}
