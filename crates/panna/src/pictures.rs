//! Editing the pictures a page draws: picking one, then moving it, resizing it from a
//! corner, replacing it with another picture or deleting it, each as a page change that can
//! be undone from the page tools.

use std::collections::{HashMap, HashSet};

use iced::widget::{container, pin, row, space, text};
use iced::{Center, Element, Theme};
use panna_core::{PictureChange, TextBox};

use crate::Message;
use crate::icons::{self, Icon};
use crate::selection::PageMap;

/// A drag shorter than this, in points, is a click.
const CLICK_SLOP: f32 = 2.0;
/// The resize handle's side, in view pixels.
pub const HANDLE: f32 = 10.0;
/// The smallest a picture is made, in points.
const MIN_SIDE: f32 = 4.0;

#[derive(Debug, Clone)]
pub enum Event {
    Toggle,
    Replace,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Grip {
    Move,
    /// From the corner at the box's end (`x1`, `y1`), keeping its shape.
    Resize,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    grip: Grip,
    from: (f32, f32),
    to: (f32, f32),
}

#[derive(Debug, Default)]
pub struct PictureEditor {
    pub on: bool,
    /// Where each page's pictures are, as shown, once the worker has said.
    found: HashMap<usize, Vec<TextBox>>,
    asked: HashSet<usize>,
    /// The picked picture: its page and place in that page's list.
    selected: Option<(usize, usize)>,
    drag: Option<Drag>,
    /// A replacement picture is being chosen for the picked one.
    replacing: bool,
}

/// What the main view does next.
pub enum Outcome {
    Nothing,
    /// Ask for a picture to put in the picked one's place.
    PickReplacement,
    Change {
        page: usize,
        changes: Vec<(usize, PictureChange)>,
    },
}

impl PictureEditor {
    pub fn busy(&self) -> bool {
        self.drag.is_some() || self.selected.is_some()
    }

    pub fn cancel(&mut self) {
        self.drag = None;
        self.selected = None;
        self.replacing = false;
    }

    pub fn stop(&mut self) {
        self.on = false;
        self.cancel();
    }

    /// Forgets where pictures were, as the pages have changed.
    pub fn reload(&mut self) {
        self.found.clear();
        self.asked.clear();
        self.drag = None;
        self.replacing = false;
    }

    /// Whether page `page`'s pictures should be asked for, marking them asked for.
    pub fn should_ask(&mut self, page: usize) -> bool {
        self.on && self.asked.insert(page)
    }

    pub fn found(&mut self, page: usize, boxes: Vec<TextBox>) {
        if let Some((p, i)) = self.selected
            && p == page
            && i >= boxes.len()
        {
            self.selected = None;
        }
        self.found.insert(page, boxes);
    }

    /// Whether a replacement picture is awaited.
    pub fn replacing(&self) -> bool {
        self.replacing
    }

    pub fn update(&mut self, event: Event) -> Outcome {
        match event {
            Event::Toggle => {
                self.on = !self.on;
                self.cancel();
            }
            Event::Replace => {
                if self.selected.is_some() {
                    self.replacing = true;
                    return Outcome::PickReplacement;
                }
            }
            Event::Delete => {
                if let Some((page, index)) = self.selected.take() {
                    return Outcome::Change {
                        page,
                        changes: vec![(index, PictureChange::Delete)],
                    };
                }
            }
        }
        Outcome::Nothing
    }

    /// The chosen replacement, or `None` when none was.
    pub fn picture(&mut self, image: Option<panna_core::StampImage>) -> Outcome {
        let waiting = std::mem::take(&mut self.replacing);
        match (waiting, image, self.selected) {
            (true, Some(image), Some((page, index))) => Outcome::Change {
                page,
                changes: vec![(index, PictureChange::Replace(image))],
            },
            _ => Outcome::Nothing,
        }
    }

    fn selected_box(&self) -> Option<(usize, TextBox)> {
        let (page, index) = self.selected?;
        let b = self.found.get(&page)?.get(index)?;
        Some((page, *b))
    }

    /// A press at `at` on page `page`; `slop` is how near the handle counts, in points.
    pub fn press(&mut self, page: usize, at: (f32, f32), slop: f32) {
        if self.replacing {
            return;
        }
        if let Some((p, b)) = self.selected_box()
            && p == page
            && (at.0 - b.x1).abs() <= slop
            && (at.1 - b.y1).abs() <= slop
        {
            self.drag = Some(Drag {
                grip: Grip::Resize,
                from: at,
                to: at,
            });
            return;
        }
        // The picture drawn last is on top.
        let hit = self
            .found
            .get(&page)
            .and_then(|boxes| boxes.iter().rposition(|b| b.contains(at.0, at.1)));
        match hit {
            Some(index) => {
                self.selected = Some((page, index));
                self.drag = Some(Drag {
                    grip: Grip::Move,
                    from: at,
                    to: at,
                });
            }
            None => self.cancel(),
        }
    }

    pub fn hover(&mut self, page: usize, at: (f32, f32)) {
        if self.selected.is_some_and(|(p, _)| p == page)
            && let Some(drag) = &mut self.drag
        {
            drag.to = at;
        }
    }

    /// Where the picked picture would go with the drag as it is.
    fn dragged(&self) -> Option<(usize, TextBox)> {
        let drag = self.drag?;
        let (page, b) = self.selected_box()?;
        let (dx, dy) = (drag.to.0 - drag.from.0, drag.to.1 - drag.from.1);
        let to = match drag.grip {
            Grip::Move => TextBox {
                x0: b.x0 + dx,
                y0: b.y0 + dy,
                x1: b.x1 + dx,
                y1: b.y1 + dy,
            },
            Grip::Resize => {
                let (w, h) = (b.width().max(f32::EPSILON), b.height().max(f32::EPSILON));
                let scale = ((w + dx) / w).max((h + dy) / h).max(MIN_SIDE / w.min(h));
                TextBox {
                    x1: b.x0 + w * scale,
                    y1: b.y0 + h * scale,
                    ..b
                }
            }
        };
        Some((page, to))
    }

    pub fn release(&mut self) -> Outcome {
        let moved = self
            .drag
            .is_some_and(|d| (d.to.0 - d.from.0).abs().max((d.to.1 - d.from.1).abs()) >= CLICK_SLOP);
        let to = self.dragged();
        self.drag = None;
        match (moved, to, self.selected) {
            (true, Some((page, rect)), Some((_, index))) => {
                // Shown where it went until the page comes back.
                if let Some(b) = self.found.get_mut(&page).and_then(|v| v.get_mut(index)) {
                    *b = rect;
                }
                Outcome::Change {
                    page,
                    changes: vec![(index, PictureChange::Move(rect))],
                }
            }
            _ => Outcome::Nothing,
        }
    }

    /// Whether `at` on page `page` is over the resize handle, for the pointer.
    pub fn over_handle(&self, page: usize, at: (f32, f32), slop: f32) -> bool {
        self.selected_box()
            .is_some_and(|(p, b)| p == page && (at.0 - b.x1).abs() <= slop && (at.1 - b.y1).abs() <= slop)
    }

    /// Outlines of page `page`'s pictures, with the picked one's handle.
    pub fn overlays(&self, page: usize, map: PageMap) -> Vec<Element<'_, Message>> {
        let boxed = |rect: TextBox, strong: bool| -> Element<'_, Message> {
            let (x, y, w, h) = map.rect(&rect);
            pin(container(space())
                .width(w.max(1.0))
                .height(h.max(1.0))
                .style(move |theme: &Theme| {
                    let primary = theme.extended_palette().primary.base.color;
                    container::Style::default()
                        .background(primary.scale_alpha(if strong { 0.15 } else { 0.05 }))
                        .border(iced::Border {
                            color: primary,
                            width: if strong { 2.0 } else { 1.0 },
                            radius: 0.0.into(),
                        })
                }))
            .x(x)
            .y(y)
            .into()
        };
        let picked = self.selected.filter(|(p, _)| *p == page).map(|(_, i)| i);
        let mut out: Vec<Element<'_, Message>> = self
            .found
            .get(&page)
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|(i, _)| picked != Some(*i))
            .map(|(_, b)| boxed(*b, false))
            .collect();
        let shown = self
            .dragged()
            .filter(|(p, _)| *p == page)
            .or_else(|| self.selected_box().filter(|(p, _)| *p == page));
        if let Some((_, b)) = shown {
            out.push(boxed(b, true));
            let corner = map.to_view(b.x1, b.y1);
            out.push(
                pin(container(space())
                    .width(HANDLE)
                    .height(HANDLE)
                    .style(|theme: &Theme| {
                        container::Style::default()
                            .background(iced::Color::WHITE)
                            .border(iced::Border {
                                color: theme.extended_palette().primary.base.color,
                                width: 1.5,
                                radius: 1.0.into(),
                            })
                    }))
                .x(corner.x - HANDLE / 2.0)
                .y(corner.y - HANDLE / 2.0)
                .into(),
            );
        }
        out
    }

    /// The floating bar.
    pub fn bar(&self) -> Element<'_, Message> {
        let ev = Message::EditPictures;
        let picked = self.selected.is_some() && !self.replacing;
        let hint = if self.selected.is_some() {
            "Drag to move it, or its corner to resize it."
        } else {
            "Click a picture to pick it."
        };
        let content = row![
            text(hint).size(13).width(290),
            icons::button(
                Icon::ReplacePicture,
                "Replace with another picture…",
                picked.then_some(ev(Event::Replace))
            ),
            icons::button(
                Icon::Delete,
                "Delete the picture",
                picked.then_some(ev(Event::Delete))
            ),
            icons::button(
                Icon::Close,
                "Stop editing pictures (Esc)",
                Some(ev(Event::Toggle))
            ),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x0: f32, y0: f32, x1: f32, y1: f32) -> TextBox {
        TextBox { x0, y0, x1, y1 }
    }

    fn editor() -> PictureEditor {
        let mut e = PictureEditor {
            on: true,
            ..PictureEditor::default()
        };
        assert!(e.should_ask(0));
        assert!(!e.should_ask(0));
        e.found(0, vec![b(10.0, 10.0, 60.0, 40.0), b(30.0, 20.0, 50.0, 30.0)]);
        e
    }

    #[test]
    fn a_drag_moves_the_picture_on_top() {
        let mut e = editor();
        e.press(0, (40.0, 25.0), 3.0);
        e.hover(0, (50.0, 35.0));
        let Outcome::Change { page, changes } = e.release() else {
            panic!("not moved");
        };
        assert_eq!(page, 0);
        assert_eq!(changes, [(1, PictureChange::Move(b(40.0, 30.0, 60.0, 40.0)))]);
        // A click only picks.
        e.press(0, (12.0, 12.0), 3.0);
        assert!(matches!(e.release(), Outcome::Nothing));
        assert_eq!(e.selected, Some((0, 0)));
    }

    #[test]
    fn the_corner_resizes_keeping_the_shape() {
        let mut e = editor();
        e.press(0, (12.0, 12.0), 3.0);
        e.release();
        e.press(0, (60.0, 40.0), 3.0);
        e.hover(0, (110.0, 45.0));
        let Outcome::Change { changes, .. } = e.release() else {
            panic!("not resized");
        };
        assert_eq!(changes, [(0, PictureChange::Move(b(10.0, 10.0, 110.0, 70.0)))]);
    }

    #[test]
    fn delete_and_replace_need_a_picked_picture() {
        let mut e = editor();
        assert!(matches!(e.update(Event::Delete), Outcome::Nothing));
        assert!(matches!(e.update(Event::Replace), Outcome::Nothing));
        e.press(0, (12.0, 12.0), 3.0);
        e.release();
        assert!(matches!(e.update(Event::Replace), Outcome::PickReplacement));
        assert!(matches!(e.picture(None), Outcome::Nothing));
        let Outcome::Change { changes, .. } = e.update(Event::Delete) else {
            panic!("not deleted");
        };
        assert_eq!(changes, [(0, PictureChange::Delete)]);
        assert_eq!(e.selected, None);
        // Clicking away from every picture picks none.
        e.press(0, (12.0, 12.0), 3.0);
        e.press(0, (200.0, 200.0), 3.0);
        assert_eq!(e.selected, None);
    }
}
