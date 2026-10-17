//! Toolbars that dock to any window edge or float over the document.
//!
//! A floating toolbar is moved by dragging its grip. Dropping it close to a window edge docks it
//! there; the dock menu on every toolbar switches position directly.

use std::fmt;

use iced::widget::{Column, Row, container, mouse_area};
use iced::{Center, Element, Point, Size, Vector, mouse};

use crate::icons::{self, Icon};

/// Where a toolbar sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dock {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
    Floating,
}

impl Dock {
    pub const ALL: [Dock; 5] = [Dock::Top, Dock::Bottom, Dock::Left, Dock::Right, Dock::Floating];

    pub fn as_str(self) -> &'static str {
        match self {
            Dock::Top => "top",
            Dock::Bottom => "bottom",
            Dock::Left => "left",
            Dock::Right => "right",
            Dock::Floating => "floating",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Dock::Top => Icon::DockTop,
            Dock::Bottom => Icon::DockBottom,
            Dock::Left => Icon::DockLeft,
            Dock::Right => Icon::DockRight,
            Dock::Floating => Icon::Float,
        }
    }

    pub fn parse(s: &str) -> Option<Dock> {
        Dock::ALL.into_iter().find(|d| d.as_str() == s)
    }

    fn vertical(self) -> bool {
        matches!(self, Dock::Left | Dock::Right)
    }
}

impl fmt::Display for Dock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Dock::Top => "Dock top",
            Dock::Bottom => "Dock bottom",
            Dock::Left => "Dock left",
            Dock::Right => "Dock right",
            Dock::Floating => "Floating",
        })
    }
}

/// Distance from a window edge, in logical pixels, at which a dropped toolbar docks.
const SNAP: f32 = 24.0;

#[derive(Debug, Clone, Copy)]
pub struct Toolbar {
    pub dock: Dock,
    /// Top-left corner while floating, in window coordinates.
    pub pos: Point,
    /// Cursor position within the grip, updated while hovering it.
    grip_hover: Vector,
    /// Offset from the cursor to the toolbar origin while being dragged.
    drag: Option<Vector>,
    /// Whether the dock choices are showing.
    menu_open: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Event {
    SetDock(Dock),
    ToggleMenu,
    GripMoved(Point),
    GripPressed,
    CursorMoved(Point),
    Released,
}

impl Toolbar {
    pub fn new(dock: Dock, pos: Point) -> Self {
        Self {
            dock,
            pos,
            grip_hover: Vector::ZERO,
            drag: None,
            menu_open: false,
        }
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Applies an event. Returns true when the dock or position changed for good (worth saving).
    pub fn update(&mut self, event: Event, window: Size) -> bool {
        match event {
            Event::SetDock(dock) => {
                if dock == Dock::Floating && self.dock != Dock::Floating {
                    // Pop out near the top-left corner of the document area.
                    self.pos = Point::new(40.0, 60.0);
                }
                self.dock = dock;
                self.drag = None;
                self.menu_open = false;
                true
            }
            Event::ToggleMenu => {
                self.menu_open = !self.menu_open;
                false
            }
            Event::GripMoved(p) => {
                self.grip_hover = Vector::new(p.x, p.y);
                false
            }
            Event::GripPressed => {
                if self.dock != Dock::Floating {
                    // Tear a docked toolbar off where it stands; the cursor keeps its grip spot.
                    self.dock = Dock::Floating;
                    self.pos = Point::ORIGIN;
                }
                self.drag = Some(self.grip_hover + Vector::new(GRIP_PAD, GRIP_PAD));
                false
            }
            Event::CursorMoved(cursor) => {
                if let Some(offset) = self.drag {
                    self.pos = clamp(cursor - offset, window);
                }
                false
            }
            Event::Released => {
                if self.drag.take().is_none() {
                    return false;
                }
                self.dock = snap(self.pos, window);
                true
            }
        }
    }

    /// Lays out `items` for the current dock, with a drag grip and the dock menu.
    pub fn view<'a, M: Clone + 'a>(
        &self,
        items: Vec<Element<'a, M>>,
        on_event: impl Fn(Event) -> M + Clone + 'a,
    ) -> Element<'a, M> {
        let vertical = self.dock.vertical();
        let grip_icon = icons::glyph(
            if vertical {
                Icon::GripHorizontal
            } else {
                Icon::GripVertical
            },
            true,
        );
        let grip = mouse_area(container(grip_icon).padding(GRIP_PAD))
            .on_move({
                let on_event = on_event.clone();
                move |p| on_event(Event::GripMoved(p))
            })
            .on_press(on_event(Event::GripPressed))
            .interaction(if self.drag.is_some() {
                mouse::Interaction::Grabbing
            } else {
                mouse::Interaction::Grab
            });
        let grip = icons::with_tip(grip, "Drag to move · drop near an edge to dock");

        let mut all = Vec::with_capacity(items.len() + 8);
        all.push(grip);
        all.extend(items);
        all.push(icons::button(
            Icon::DockMenu,
            "Toolbar position",
            Some(on_event(Event::ToggleMenu)),
        ));
        if self.menu_open {
            for dock in Dock::ALL {
                // The current position shows disabled, so the choices read as "move to …".
                let press = (dock != self.dock).then(|| on_event(Event::SetDock(dock)));
                all.push(icons::button(dock.icon(), dock.to_string(), press));
            }
        }

        let bar: Element<'a, M> = if vertical {
            Column::with_children(all)
                .spacing(6)
                .padding(6)
                .align_x(Center)
                .into()
        } else {
            Row::with_children(all)
                .spacing(6)
                .padding(6)
                .align_y(Center)
                .into()
        };
        let floating = self.dock == Dock::Floating;
        container(bar)
            .style(move |theme: &iced::Theme| {
                let palette = theme.extended_palette();
                let style = container::Style::default().background(palette.background.base.color);
                // No drop shadow; see the note on page borders in main.rs.
                if floating {
                    style.border(iced::Border {
                        color: palette.background.strong.color,
                        width: 1.0,
                        radius: 8.0.into(),
                    })
                } else {
                    style
                }
            })
            .into()
    }
}

const GRIP_PAD: f32 = 4.0;

/// Keeps at least part of a floating toolbar reachable inside the window.
fn clamp(p: Point, window: Size) -> Point {
    Point::new(
        p.x.clamp(0.0, (window.width - 60.0).max(0.0)),
        p.y.clamp(0.0, (window.height - 40.0).max(0.0)),
    )
}

/// Which dock a toolbar dropped at `p` belongs to.
fn snap(p: Point, window: Size) -> Dock {
    if p.y <= SNAP {
        Dock::Top
    } else if p.x <= SNAP {
        Dock::Left
    } else if p.y >= window.height - 40.0 - SNAP {
        Dock::Bottom
    } else if p.x >= window.width - 60.0 - SNAP {
        Dock::Right
    } else {
        Dock::Floating
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN: Size = Size::new(1000.0, 800.0);

    #[test]
    fn dropping_near_edges_docks() {
        assert_eq!(snap(Point::new(300.0, 5.0), WIN), Dock::Top);
        assert_eq!(snap(Point::new(5.0, 300.0), WIN), Dock::Left);
        assert_eq!(snap(Point::new(300.0, 760.0), WIN), Dock::Bottom);
        assert_eq!(snap(Point::new(940.0, 300.0), WIN), Dock::Right);
        assert_eq!(snap(Point::new(300.0, 300.0), WIN), Dock::Floating);
    }

    #[test]
    fn drag_moves_then_docks_on_release() {
        let mut bar = Toolbar::new(Dock::Top, Point::ORIGIN);
        bar.update(Event::GripMoved(Point::new(2.0, 3.0)), WIN);
        bar.update(Event::GripPressed, WIN);
        assert_eq!(bar.dock, Dock::Floating);
        bar.update(Event::CursorMoved(Point::new(400.0, 300.0)), WIN);
        assert_eq!(bar.pos, Point::new(394.0, 293.0));
        assert!(bar.update(Event::Released, WIN));
        assert_eq!(bar.dock, Dock::Floating);
        assert!(!bar.dragging());

        bar.update(Event::GripPressed, WIN);
        bar.update(Event::CursorMoved(Point::new(10.0, 300.0)), WIN);
        bar.update(Event::Released, WIN);
        assert_eq!(bar.dock, Dock::Left);
    }

    #[test]
    fn floating_stays_inside_window() {
        assert_eq!(clamp(Point::new(-50.0, 5000.0), WIN), Point::new(0.0, 760.0));
    }

    #[test]
    fn dock_names_round_trip() {
        for d in Dock::ALL {
            assert_eq!(Dock::parse(d.as_str()), Some(d));
        }
        assert_eq!(Dock::parse("nonsense"), None);
    }
}
