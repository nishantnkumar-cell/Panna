//! The toolbars: small bars of related buttons, each of which floats over the document or
//! snaps to a window edge.
//!
//! A bar is moved by dragging its grip. Letting go near a window edge snaps it there, next to
//! any bars already on that edge, in the order they were dropped along it. Bars snapped to the
//! bottom sit in the middle unless the user picks left or right.

use iced::widget::{Column, Row, container, mouse_area, opaque, pin, rule, space, tooltip};
use iced::{Center, Element, Fill, Point, Size, Vector, mouse};

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

    pub fn parse(s: &str) -> Option<Dock> {
        Dock::ALL.into_iter().find(|d| d.as_str() == s)
    }

    pub fn vertical(self) -> bool {
        matches!(self, Dock::Left | Dock::Right)
    }
}

/// One of the toolbars, by what its buttons are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    File,
    Page,
    Zoom,
    View,
    Tools,
    Review,
    /// The comment and drawing tools, shown while commenting is on.
    Comment,
}

impl Bar {
    pub const ALL: [Bar; 7] = [
        Bar::File,
        Bar::Page,
        Bar::Zoom,
        Bar::View,
        Bar::Tools,
        Bar::Review,
        Bar::Comment,
    ];

    /// The name shown in View ▸ Toolbars.
    pub fn name(self) -> &'static str {
        match self {
            Bar::File => "File",
            Bar::Page => "Page",
            Bar::Zoom => "Zoom",
            Bar::View => "View",
            Bar::Tools => "Tools",
            Bar::Review => "Edit and review",
            Bar::Comment => "Comment",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Bar::File => "file",
            Bar::Page => "page",
            Bar::Zoom => "zoom",
            Bar::View => "view",
            Bar::Tools => "tools",
            Bar::Review => "review",
            Bar::Comment => "comment",
        }
    }

    /// Where the bar starts, and where Reset toolbar layout puts it back.
    fn default_place(self) -> Place {
        let (dock, along) = match self {
            Bar::File => (Dock::Top, 0.0),
            Bar::View => (Dock::Top, 100.0),
            Bar::Tools => (Dock::Top, 200.0),
            Bar::Review => (Dock::Top, 300.0),
            Bar::Page => (Dock::Bottom, 0.0),
            Bar::Zoom => (Dock::Bottom, 100.0),
            Bar::Comment => (Dock::Floating, 0.0),
        };
        Place {
            dock,
            pos: Point::new(300.0, 130.0),
            along,
            shown: true,
        }
    }
}

/// Where the bars snapped to the bottom edge line up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Center,
    Left,
    Right,
}

impl Align {
    pub const ALL: [Align; 3] = [Align::Center, Align::Left, Align::Right];

    pub fn as_str(self) -> &'static str {
        match self {
            Align::Center => "center",
            Align::Left => "left",
            Align::Right => "right",
        }
    }

    pub fn parse(s: &str) -> Option<Align> {
        Align::ALL.into_iter().find(|a| a.as_str() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Align::Center => "Centre",
            Align::Left => "Left",
            Align::Right => "Right",
        }
    }
}

/// Where one bar is, kept between runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub dock: Dock,
    /// Top-left corner while floating, in window coordinates.
    pub pos: Point,
    /// Where along its edge it was dropped, which orders the bars on that edge.
    pub along: f32,
    pub shown: bool,
}

/// Where every bar is, and how they behave, kept between runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    file: Place,
    page: Place,
    zoom: Place,
    view: Place,
    tools: Place,
    review: Place,
    comment: Place,
    pub bottom: Align,
    /// Locked bars have no grips, so they can't be dragged by accident.
    pub locked: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            file: Bar::File.default_place(),
            page: Bar::Page.default_place(),
            zoom: Bar::Zoom.default_place(),
            view: Bar::View.default_place(),
            tools: Bar::Tools.default_place(),
            review: Bar::Review.default_place(),
            comment: Bar::Comment.default_place(),
            bottom: Align::Center,
            locked: false,
        }
    }
}

impl Layout {
    pub fn place(&self, bar: Bar) -> &Place {
        match bar {
            Bar::File => &self.file,
            Bar::Page => &self.page,
            Bar::Zoom => &self.zoom,
            Bar::View => &self.view,
            Bar::Tools => &self.tools,
            Bar::Review => &self.review,
            Bar::Comment => &self.comment,
        }
    }

    fn place_mut(&mut self, bar: Bar) -> &mut Place {
        match bar {
            Bar::File => &mut self.file,
            Bar::Page => &mut self.page,
            Bar::Zoom => &mut self.zoom,
            Bar::View => &mut self.view,
            Bar::Tools => &mut self.tools,
            Bar::Review => &mut self.review,
            Bar::Comment => &mut self.comment,
        }
    }

    /// Reads one settings line, returning false when the key isn't a toolbar one.
    pub fn parse_line(&mut self, key: &str, value: &str, finite: impl Fn(&str) -> Option<f32>) -> bool {
        let value = value.trim();
        match key {
            "bars.bottom" => self.bottom = Align::parse(value).unwrap_or(self.bottom),
            "bars.locked" => self.locked = value == "true",
            _ => {
                let Some((name, field)) = key.strip_prefix("bar.").and_then(|k| k.split_once('.')) else {
                    return false;
                };
                let Some(bar) = Bar::ALL.into_iter().find(|b| b.key() == name) else {
                    return false;
                };
                let place = self.place_mut(bar);
                match field {
                    "dock" => place.dock = Dock::parse(value).unwrap_or(place.dock),
                    "x" => place.pos.x = finite(value).unwrap_or(place.pos.x),
                    "y" => place.pos.y = finite(value).unwrap_or(place.pos.y),
                    "along" => place.along = finite(value).unwrap_or(place.along),
                    "shown" => place.shown = value != "false",
                    _ => return false,
                }
            }
        }
        true
    }

    pub fn serialize(&self, out: &mut String) {
        for bar in Bar::ALL {
            let p = self.place(bar);
            let k = bar.key();
            out.push_str(&format!(
                "bar.{k}.dock={}\nbar.{k}.x={}\nbar.{k}.y={}\nbar.{k}.along={}\nbar.{k}.shown={}\n",
                p.dock.as_str(),
                p.pos.x,
                p.pos.y,
                p.along,
                p.shown
            ));
        }
        out.push_str(&format!(
            "bars.bottom={}\nbars.locked={}\n",
            self.bottom.as_str(),
            self.locked
        ));
    }
}

/// Distance from a window edge, in logical pixels, within which a dropped bar snaps to it.
const SNAP: f32 = 40.0;
/// How thick the outline showing where a bar will snap is.
const GUIDE: f32 = 46.0;
const GRIP_PAD: f32 = 4.0;

#[derive(Debug, Clone, Copy)]
pub enum Event {
    /// The cursor moved over a grip, to this spot within it.
    GripMoved(Point),
    GripPressed(Bar),
    CursorMoved(Point),
    Released,
    Show(Bar, bool),
    BottomAlign(Align),
    ToggleLock,
    Reset,
}

/// The toolbars and the one being dragged, if any.
#[derive(Debug, Clone)]
pub struct Bars {
    pub layout: Layout,
    /// How far below the window's top edge the document area (and so the top edge's bars)
    /// starts, below the menu bar.
    top: f32,
    grip_hover: Vector,
    drag: Option<Drag>,
    cursor: Point,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    bar: Bar,
    /// From the cursor to the bar's top-left corner.
    offset: Vector,
    /// Where the bar was, to go back to when let go without moving.
    from: Place,
    moved: bool,
}

/// The bars laid out for the window: a slot per edge, and the floating ones to pin on top.
pub struct Arranged<'a, M> {
    pub top: Element<'a, M>,
    pub bottom: Element<'a, M>,
    pub left: Element<'a, M>,
    pub right: Element<'a, M>,
    /// Floating bars and, while dragging, the outline of where the bar will snap.
    pub floating: Vec<Element<'a, M>>,
}

impl Bars {
    pub fn new(layout: Layout, top: f32) -> Self {
        Self {
            layout,
            top,
            grip_hover: Vector::ZERO,
            drag: None,
            cursor: Point::ORIGIN,
        }
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Applies an event. Returns true when the layout changed for good (worth saving).
    pub fn update(&mut self, event: Event, window: Size) -> bool {
        match event {
            Event::GripMoved(p) => {
                self.grip_hover = Vector::new(p.x, p.y);
                false
            }
            Event::GripPressed(bar) => {
                if self.layout.locked {
                    return false;
                }
                let place = self.layout.place_mut(bar);
                let from = *place;
                if place.dock != Dock::Floating {
                    // Tear a snapped bar off where it stands; the cursor keeps its grip spot,
                    // and the bar jumps to it on the first move.
                    place.dock = Dock::Floating;
                    place.pos = Point::new(-1000.0, -1000.0);
                }
                self.drag = Some(Drag {
                    bar,
                    offset: self.grip_hover + Vector::new(GRIP_PAD + PAD, GRIP_PAD + PAD),
                    from,
                    moved: false,
                });
                false
            }
            Event::CursorMoved(cursor) => {
                self.cursor = cursor;
                if let Some(drag) = &mut self.drag {
                    drag.moved = true;
                    self.layout.place_mut(drag.bar).pos = clamp(cursor - drag.offset, window, self.top);
                }
                false
            }
            Event::Released => {
                let Some(Drag { bar, from, moved, .. }) = self.drag.take() else {
                    return false;
                };
                if !moved {
                    // A click on the grip, not a drag.
                    *self.layout.place_mut(bar) = from;
                    return false;
                }
                let dock = snap(self.cursor, window, self.top);
                let place = self.layout.place_mut(bar);
                place.dock = dock;
                place.along = if dock.vertical() {
                    self.cursor.y
                } else {
                    self.cursor.x
                };
                true
            }
            Event::Show(bar, shown) => {
                self.layout.place_mut(bar).shown = shown;
                true
            }
            Event::BottomAlign(align) => {
                self.layout.bottom = align;
                true
            }
            Event::ToggleLock => {
                self.layout.locked = !self.layout.locked;
                true
            }
            Event::Reset => {
                self.layout = Layout::default();
                self.drag = None;
                true
            }
        }
    }

    /// Lays out the bars. `items` gives a bar's buttons, for a column when `vertical`, or
    /// `None` when the bar has nothing to show right now.
    pub fn arrange<'a, M: Clone + 'a>(
        &self,
        mut items: impl FnMut(Bar, bool) -> Option<Vec<Element<'a, M>>>,
        on_event: impl Fn(Event) -> M + Clone + 'a,
        window: Size,
    ) -> Arranged<'a, M> {
        let (mut top, mut bottom, mut left, mut right) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut floating = Vec::new();
        for bar in Bar::ALL {
            let place = *self.layout.place(bar);
            if !place.shown {
                continue;
            }
            let vertical = place.dock.vertical();
            let side = match place.dock {
                Dock::Bottom => tooltip::Position::Top,
                Dock::Left => tooltip::Position::Right,
                Dock::Right => tooltip::Position::Left,
                Dock::Top | Dock::Floating => tooltip::Position::Bottom,
            };
            let Some(view) = icons::tips_on(side, || {
                let content = items(bar, vertical)?;
                Some(self.bar_view(bar, place.dock, content, on_event.clone()))
            }) else {
                continue;
            };
            let edge = match place.dock {
                Dock::Top => &mut top,
                Dock::Bottom => &mut bottom,
                Dock::Left => &mut left,
                Dock::Right => &mut right,
                Dock::Floating => {
                    floating.push(pin(opaque(view)).x(place.pos.x).y(place.pos.y).into());
                    continue;
                }
            };
            edge.push((place.along, view));
        }
        if let Some(dock) = self.target(window) {
            floating.push(guide(dock, window, self.top));
        }
        let [top, bottom, left, right] =
            [top, bottom, left, right].map(|mut bars: Vec<(f32, Element<'a, M>)>| {
                bars.sort_by(|a, b| a.0.total_cmp(&b.0));
                bars.into_iter().map(|(_, e)| e).collect::<Vec<_>>()
            });
        Arranged {
            top: edge(top, false, Align::Left),
            bottom: edge(bottom, false, self.layout.bottom),
            left: edge(left, true, Align::Left),
            right: edge(right, true, Align::Left),
            floating,
        }
    }

    /// Where the bar being dragged would snap if let go now.
    fn target(&self, window: Size) -> Option<Dock> {
        self.drag.filter(|d| d.moved)?;
        Some(snap(self.cursor, window, self.top)).filter(|d| *d != Dock::Floating)
    }

    fn bar_view<'a, M: Clone + 'a>(
        &self,
        bar: Bar,
        dock: Dock,
        items: Vec<Element<'a, M>>,
        on_event: impl Fn(Event) -> M + Clone + 'a,
    ) -> Element<'a, M> {
        let vertical = dock.vertical();
        let mut all = Vec::with_capacity(items.len() + 1);
        if !self.layout.locked {
            let grip_icon = icons::glyph(
                if vertical {
                    Icon::GripHorizontal
                } else {
                    Icon::GripVertical
                },
                true,
            );
            let dragging = self.drag.is_some_and(|d| d.bar == bar);
            let grip = mouse_area(container(grip_icon).padding(GRIP_PAD))
                .on_move({
                    let on_event = on_event.clone();
                    move |p| on_event(Event::GripMoved(p))
                })
                .on_press(on_event(Event::GripPressed(bar)))
                .interaction(if dragging {
                    mouse::Interaction::Grabbing
                } else {
                    mouse::Interaction::Grab
                });
            all.push(icons::with_tip(
                grip,
                format!("{}: drag to move, drop near an edge to snap", bar.name()),
            ));
        }
        all.extend(items);
        let content: Element<'a, M> = if vertical {
            Column::with_children(all)
                .spacing(2)
                .padding(PAD)
                .align_x(Center)
                .into()
        } else {
            Row::with_children(all)
                .spacing(2)
                .padding(PAD)
                .align_y(Center)
                .into()
        };
        let floating = dock == Dock::Floating;
        container(content)
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

/// Padding inside a bar, around its buttons.
const PAD: f32 = 4.0;

/// One window edge's bars side by side, with a thin line between bars, on the toolbar colour.
fn edge<'a, M: 'a>(bars: Vec<Element<'a, M>>, vertical: bool, align: Align) -> Element<'a, M> {
    if bars.is_empty() {
        return space().into();
    }
    let count = bars.len();
    let mut all = Vec::with_capacity(count * 2);
    for (i, bar) in bars.into_iter().enumerate() {
        if i > 0 {
            all.push(if vertical {
                container(rule::horizontal(1)).width(32).into()
            } else {
                container(rule::vertical(1)).height(28).into()
            });
        }
        all.push(bar);
    }
    let lined: Element<'a, M> = if vertical {
        Column::with_children(all).spacing(2).align_x(Center).into()
    } else {
        // Onto a second line when the window is too narrow for every bar.
        Row::with_children(all).spacing(4).align_y(Center).wrap().into()
    };
    let slot = container(lined).padding(2).style(|theme: &iced::Theme| {
        container::Style::default().background(theme.extended_palette().background.base.color)
    });
    if vertical {
        slot.height(Fill).into()
    } else {
        slot.width(Fill)
            .align_x(match align {
                Align::Center => iced::alignment::Horizontal::Center,
                Align::Left => iced::alignment::Horizontal::Left,
                Align::Right => iced::alignment::Horizontal::Right,
            })
            .into()
    }
}

/// A blue outline along the edge a dragged bar will snap to.
fn guide<'a, M: 'a>(dock: Dock, window: Size, top: f32) -> Element<'a, M> {
    let (pos, size) = match dock {
        Dock::Top => (Point::new(0.0, top), Size::new(window.width, GUIDE)),
        Dock::Bottom => (
            Point::new(0.0, window.height - GUIDE),
            Size::new(window.width, GUIDE),
        ),
        Dock::Left => (Point::new(0.0, top), Size::new(GUIDE, window.height - top)),
        Dock::Right | Dock::Floating => (
            Point::new(window.width - GUIDE, top),
            Size::new(GUIDE, window.height - top),
        ),
    };
    let outline = container(space().width(size.width.max(0.0)).height(size.height.max(0.0))).style(
        |theme: &iced::Theme| {
            let primary = theme.extended_palette().primary.base.color;
            container::Style::default()
                .background(iced::Color { a: 0.15, ..primary })
                .border(iced::Border {
                    color: primary,
                    width: 2.0,
                    radius: 6.0.into(),
                })
        },
    );
    pin(outline).position(pos).into()
}

/// Keeps a floating bar below the menu bar and at least partly inside the window.
fn clamp(p: Point, window: Size, top: f32) -> Point {
    Point::new(
        p.x.clamp(0.0, (window.width - 60.0).max(0.0)),
        p.y.clamp(top, (window.height - 40.0).max(top)),
    )
}

/// Which edge a bar let go of with the cursor at `cursor` snaps to, if any.
fn snap(cursor: Point, window: Size, top: f32) -> Dock {
    if cursor.y <= top + SNAP {
        Dock::Top
    } else if cursor.y >= window.height - SNAP {
        Dock::Bottom
    } else if cursor.x <= SNAP {
        Dock::Left
    } else if cursor.x >= window.width - SNAP {
        Dock::Right
    } else {
        Dock::Floating
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN: Size = Size::new(1000.0, 800.0);
    const TOP: f32 = 28.0;

    #[test]
    fn letting_go_near_edges_snaps() {
        assert_eq!(snap(Point::new(300.0, 40.0), WIN, TOP), Dock::Top);
        assert_eq!(snap(Point::new(5.0, 300.0), WIN, TOP), Dock::Left);
        assert_eq!(snap(Point::new(300.0, 790.0), WIN, TOP), Dock::Bottom);
        assert_eq!(snap(Point::new(990.0, 300.0), WIN, TOP), Dock::Right);
        assert_eq!(snap(Point::new(300.0, 300.0), WIN, TOP), Dock::Floating);
    }

    #[test]
    fn default_layout_matches_the_design() {
        let layout = Layout::default();
        for bar in [Bar::File, Bar::View, Bar::Tools, Bar::Review] {
            assert_eq!(layout.place(bar).dock, Dock::Top, "{bar:?}");
        }
        assert_eq!(layout.place(Bar::Page).dock, Dock::Bottom);
        assert_eq!(layout.place(Bar::Zoom).dock, Dock::Bottom);
        assert_eq!(layout.place(Bar::Comment).dock, Dock::Floating);
        assert_eq!(layout.bottom, Align::Center);
        assert!(!layout.locked);
    }

    #[test]
    fn drag_moves_one_bar_then_snaps_where_let_go() {
        let mut bars = Bars::new(Layout::default(), TOP);
        bars.update(Event::GripMoved(Point::new(2.0, 3.0)), WIN);
        bars.update(Event::GripPressed(Bar::Zoom), WIN);
        assert!(bars.dragging());
        assert_eq!(bars.layout.place(Bar::Zoom).dock, Dock::Floating);
        bars.update(Event::CursorMoved(Point::new(400.0, 300.0)), WIN);
        assert_eq!(bars.layout.place(Bar::Zoom).pos, Point::new(390.0, 289.0));
        // Nothing else moved.
        assert_eq!(bars.layout.place(Bar::Page).dock, Dock::Bottom);
        assert!(bars.update(Event::Released, WIN));
        assert_eq!(bars.layout.place(Bar::Zoom).dock, Dock::Floating);
        assert!(!bars.dragging());

        bars.update(Event::GripPressed(Bar::Zoom), WIN);
        bars.update(Event::CursorMoved(Point::new(10.0, 300.0)), WIN);
        assert_eq!(bars.target(WIN), Some(Dock::Left));
        bars.update(Event::Released, WIN);
        assert_eq!(bars.layout.place(Bar::Zoom).dock, Dock::Left);
        assert_eq!(bars.layout.place(Bar::Zoom).along, 300.0);
    }

    #[test]
    fn a_click_on_a_grip_moves_nothing() {
        let mut bars = Bars::new(Layout::default(), TOP);
        bars.update(Event::GripPressed(Bar::File), WIN);
        assert!(!bars.update(Event::Released, WIN));
        assert_eq!(*bars.layout.place(Bar::File), Bar::File.default_place());
    }

    #[test]
    fn locked_bars_dont_move() {
        let mut bars = Bars::new(Layout::default(), TOP);
        bars.update(Event::ToggleLock, WIN);
        bars.update(Event::GripPressed(Bar::File), WIN);
        assert!(!bars.dragging());
        assert_eq!(bars.layout.place(Bar::File).dock, Dock::Top);
    }

    #[test]
    fn floating_stays_inside_window_and_below_the_menus() {
        assert_eq!(clamp(Point::new(-50.0, 5000.0), WIN, TOP), Point::new(0.0, 760.0));
        assert_eq!(clamp(Point::new(50.0, 2.0), WIN, TOP), Point::new(50.0, TOP));
    }

    #[test]
    fn layout_round_trips_and_ignores_garbage() {
        let mut layout = Layout::default();
        *layout.place_mut(Bar::Tools) = Place {
            dock: Dock::Right,
            pos: Point::new(12.5, 40.0),
            along: 220.0,
            shown: false,
        };
        layout.bottom = Align::Right;
        layout.locked = true;
        let mut text = String::new();
        layout.serialize(&mut text);
        let finite = |s: &str| s.trim().parse::<f32>().ok().filter(|v| v.is_finite());
        let mut read = Layout::default();
        for line in text.lines() {
            let (k, v) = line.split_once('=').unwrap();
            assert!(read.parse_line(k, v, finite), "{line}");
        }
        assert_eq!(read, layout);

        let mut read = Layout::default();
        assert!(!read.parse_line("bar.nope.dock", "left", finite));
        assert!(read.parse_line("bar.tools.dock", "sideways", finite));
        assert!(read.parse_line("bars.bottom", "middle", finite));
        assert_eq!(read, Layout::default());
    }

    #[test]
    fn dock_names_round_trip() {
        for d in Dock::ALL {
            assert_eq!(Dock::parse(d.as_str()), Some(d));
        }
        assert_eq!(Dock::parse("nonsense"), None);
    }
}
