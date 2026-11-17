//! Touch on the pages: one finger scrolls, two fingers pinch to zoom and scroll, a quick
//! sideways swipe turns the page, and a tap clicks. With a drawing or marking tool on, one
//! finger or a pen draws as the mouse does; putting a second finger down drops that stroke
//! and scrolls or zooms instead.
//!
//! iced reports pens as touches, so a pen and a finger can't be told apart: which one draws
//! depends on the tool, not the device.

use std::time::{Duration, Instant};

use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, overlay, renderer};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, mouse, touch};

/// How far a finger may move, in pixels, and still tap.
const TAP_SLOP: f32 = 10.0;
/// A tap or swipe is over within this time.
const QUICK: Duration = Duration::from_millis(500);
/// A swipe goes at least this far sideways, in pixels.
const SWIPE: f32 = 80.0;

/// What the fingers did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    /// One finger moved by this much: the pages follow it.
    Pan(Vector),
    /// Two fingers moved: zoom by `factor` around `center` (relative to the area's top left),
    /// and follow their midpoint by `shift`.
    Pinch {
        factor: f32,
        center: Point,
        shift: Vector,
    },
    /// A quick sideways swipe: true to the next page.
    Swipe(bool),
    /// A second finger came down while one was drawing: drop what it started.
    Cancel,
}

/// What to do with a touch event besides any gesture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pass {
    /// Hand it to the pages, which treat a finger as the mouse.
    Through,
    /// Keep it from the pages.
    Keep,
    /// The finger tapped here: give the pages a press and a release.
    Tap(Point),
}

/// The fingers down and what they're doing.
#[derive(Debug, Default)]
pub struct Tracker {
    fingers: Vec<(touch::Finger, Point)>,
    /// Where and when the first finger came down.
    start: Option<(Point, Instant)>,
    moved: bool,
    /// The first finger is drawing, its events handed to the pages.
    drawing: bool,
    /// Two fingers have been down since the first came down.
    pinched: bool,
    /// The two fingers' midpoint and distance apart, last time.
    pair: Option<(Point, f32)>,
}

fn pair(fingers: &[(touch::Finger, Point)]) -> Option<(Point, f32)> {
    let [(_, a), (_, b), ..] = fingers else {
        return None;
    };
    let mid = Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    Some((mid, a.distance(*b).max(1.0)))
}

impl Tracker {
    /// Takes in `event`, with positions relative to the area. `draws` is whether one finger
    /// should draw with the current tool.
    pub fn event(&mut self, event: &touch::Event, draws: bool, now: Instant) -> (Pass, Option<Gesture>) {
        match *event {
            touch::Event::FingerPressed { id, position } => {
                self.fingers.retain(|(f, _)| *f != id);
                self.fingers.push((id, position));
                match self.fingers.len() {
                    1 => {
                        *self = Tracker {
                            fingers: std::mem::take(&mut self.fingers),
                            start: Some((position, now)),
                            drawing: draws,
                            ..Tracker::default()
                        };
                        (if draws { Pass::Through } else { Pass::Keep }, None)
                    }
                    2 => {
                        self.pinched = true;
                        self.pair = pair(&self.fingers);
                        let cancel = std::mem::take(&mut self.drawing).then_some(Gesture::Cancel);
                        (Pass::Keep, cancel)
                    }
                    _ => (Pass::Keep, None),
                }
            }
            touch::Event::FingerMoved { id, position } => {
                let Some(prev) = self.fingers.iter_mut().find(|(f, _)| *f == id) else {
                    return (Pass::Keep, None);
                };
                let delta = position - prev.1;
                prev.1 = position;
                if self.drawing {
                    return (Pass::Through, None);
                }
                if self.pinched {
                    let (Some((mid, dist)), Some((last_mid, last_dist))) = (pair(&self.fingers), self.pair)
                    else {
                        return (Pass::Keep, None);
                    };
                    self.pair = Some((mid, dist));
                    let gesture = Gesture::Pinch {
                        factor: dist / last_dist,
                        center: mid,
                        shift: mid - last_mid,
                    };
                    return (Pass::Keep, Some(gesture));
                }
                if self.start.is_some_and(|(s, _)| s.distance(position) > TAP_SLOP) {
                    self.moved = true;
                }
                (Pass::Keep, self.moved.then_some(Gesture::Pan(delta)))
            }
            touch::Event::FingerLifted { id, position } | touch::Event::FingerLost { id, position } => {
                let lifted = matches!(event, touch::Event::FingerLifted { .. });
                let known = self.fingers.iter().any(|(f, _)| *f == id);
                self.fingers.retain(|(f, _)| *f != id);
                if self.drawing {
                    self.drawing = false;
                    return (Pass::Through, None);
                }
                if !known || !self.fingers.is_empty() {
                    // A finger of a pinch: the other one goes on scrolling, not panning anew.
                    self.pair = pair(&self.fingers);
                    return (Pass::Keep, None);
                }
                let (pinched, moved, start) = (self.pinched, self.moved, self.start.take());
                *self = Tracker::default();
                let Some((start, at)) = start else {
                    return (Pass::Keep, None);
                };
                let quick = now.duration_since(at) < QUICK;
                if pinched || !lifted || !quick {
                    return (Pass::Keep, None);
                }
                if !moved {
                    return (Pass::Tap(start), None);
                }
                let d = position - start;
                if d.x.abs() >= SWIPE && d.x.abs() > d.y.abs() * 2.0 {
                    return (Pass::Keep, Some(Gesture::Swipe(d.x < 0.0)));
                }
                (Pass::Keep, None)
            }
        }
    }
}

/// `content` with touch handled as above; `on` turns a gesture into a message.
pub struct Touch<'a, Message> {
    content: Element<'a, Message>,
    draws: bool,
    on: fn(Gesture) -> Message,
}

pub fn touch<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    draws: bool,
    on: fn(Gesture) -> Message,
) -> Touch<'a, Message> {
    Touch {
        content: content.into(),
        draws,
        on,
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Touch<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Tracker>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Tracker::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        match tree.children.first_mut() {
            Some(child) => self.content.as_widget_mut().layout(child, renderer, limits),
            None => layout::Node::new(Size::ZERO),
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        if let Some(child) = tree.children.first_mut() {
            self.content
                .as_widget_mut()
                .operate(child, layout, renderer, operation);
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let Event::Touch(touch_event) = event else {
            if let Some(child) = tree.children.first_mut() {
                self.content
                    .as_widget_mut()
                    .update(child, event, layout, cursor, renderer, clipboard, shell, viewport);
            }
            return;
        };
        let bounds = layout.bounds();
        let tracker = tree.state.downcast_mut::<Tracker>();
        let fresh = matches!(touch_event, touch::Event::FingerPressed { .. });
        let position = match *touch_event {
            touch::Event::FingerPressed { position, .. }
            | touch::Event::FingerMoved { position, .. }
            | touch::Event::FingerLifted { position, .. }
            | touch::Event::FingerLost { position, .. } => position,
        };
        // Fingers that come down outside belong to something else.
        if fresh && tracker.fingers.is_empty() && !bounds.contains(position) {
            return;
        }
        let relative = |p: Point| Point::new(p.x - bounds.x, p.y - bounds.y);
        let local = match *touch_event {
            touch::Event::FingerPressed { id, position } => touch::Event::FingerPressed {
                id,
                position: relative(position),
            },
            touch::Event::FingerMoved { id, position } => touch::Event::FingerMoved {
                id,
                position: relative(position),
            },
            touch::Event::FingerLifted { id, position } => touch::Event::FingerLifted {
                id,
                position: relative(position),
            },
            touch::Event::FingerLost { id, position } => touch::Event::FingerLost {
                id,
                position: relative(position),
            },
        };
        let (pass, gesture) = tracker.event(&local, self.draws, Instant::now());
        if let Some(gesture) = gesture {
            shell.publish((self.on)(gesture));
        }
        let Some(child) = tree.children.first_mut() else {
            return;
        };
        let mut send = |event: &Event, cursor: mouse::Cursor, shell: &mut Shell<'_, Message>| {
            self.content
                .as_widget_mut()
                .update(child, event, layout, cursor, renderer, clipboard, shell, viewport);
        };
        match pass {
            Pass::Through => send(event, cursor, shell),
            Pass::Keep => {}
            Pass::Tap(at) => {
                let at = Point::new(at.x + bounds.x, at.y + bounds.y);
                let id = touch::Finger(u64::MAX);
                let cursor = mouse::Cursor::Available(at);
                send(
                    &Event::Touch(touch::Event::FingerPressed { id, position: at }),
                    cursor,
                    shell,
                );
                send(
                    &Event::Touch(touch::Event::FingerLifted { id, position: at }),
                    cursor,
                    shell,
                );
            }
        }
        shell.capture_event();
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        tree.children.first().map_or(mouse::Interaction::None, |child| {
            self.content
                .as_widget()
                .mouse_interaction(child, layout, cursor, viewport, renderer)
        })
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if let Some(child) = tree.children.first() {
            self.content
                .as_widget()
                .draw(child, renderer, theme, style, layout, cursor, viewport);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        let child = tree.children.first_mut()?;
        self.content
            .as_widget_mut()
            .overlay(child, layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<Touch<'a, Message>> for Element<'a, Message> {
    fn from(touch: Touch<'a, Message>) -> Self {
        Element::new(touch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(id: u64, x: f32, y: f32) -> touch::Event {
        touch::Event::FingerPressed {
            id: touch::Finger(id),
            position: Point::new(x, y),
        }
    }
    fn moved(id: u64, x: f32, y: f32) -> touch::Event {
        touch::Event::FingerMoved {
            id: touch::Finger(id),
            position: Point::new(x, y),
        }
    }
    fn lift(id: u64, x: f32, y: f32) -> touch::Event {
        touch::Event::FingerLifted {
            id: touch::Finger(id),
            position: Point::new(x, y),
        }
    }

    #[test]
    fn a_finger_taps_pans_and_swipes() {
        let t0 = Instant::now();
        let later = t0 + Duration::from_millis(100);
        let mut t = Tracker::default();
        assert_eq!(t.event(&press(1, 100.0, 100.0), false, t0), (Pass::Keep, None));
        assert_eq!(
            t.event(&lift(1, 102.0, 101.0), false, later),
            (Pass::Tap(Point::new(100.0, 100.0)), None)
        );

        t.event(&press(1, 100.0, 100.0), false, t0);
        assert_eq!(t.event(&moved(1, 100.0, 105.0), false, later).1, None);
        assert_eq!(
            t.event(&moved(1, 100.0, 140.0), false, later).1,
            Some(Gesture::Pan(Vector::new(0.0, 35.0)))
        );
        assert_eq!(t.event(&lift(1, 100.0, 140.0), false, later), (Pass::Keep, None));

        t.event(&press(1, 300.0, 100.0), false, t0);
        t.event(&moved(1, 150.0, 110.0), false, later);
        assert_eq!(
            t.event(&lift(1, 150.0, 110.0), false, later).1,
            Some(Gesture::Swipe(true))
        );
        // A slow drag isn't a swipe.
        t.event(&press(1, 300.0, 100.0), false, t0);
        t.event(&moved(1, 150.0, 110.0), false, later);
        assert_eq!(t.event(&lift(1, 150.0, 110.0), false, t0 + QUICK).1, None);
    }

    #[test]
    fn two_fingers_pinch_and_stop_a_stroke() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        assert_eq!(t.event(&press(1, 100.0, 100.0), true, t0), (Pass::Through, None));
        assert_eq!(t.event(&moved(1, 110.0, 100.0), true, t0), (Pass::Through, None));
        assert_eq!(
            t.event(&press(2, 210.0, 100.0), true, t0),
            (Pass::Keep, Some(Gesture::Cancel))
        );
        let (pass, gesture) = t.event(&moved(2, 310.0, 100.0), true, t0);
        assert_eq!(pass, Pass::Keep);
        assert_eq!(
            gesture,
            Some(Gesture::Pinch {
                factor: 2.0,
                center: Point::new(210.0, 100.0),
                shift: Vector::new(50.0, 0.0),
            })
        );
        // Lifting one finger, moving the other and lifting it do nothing more.
        assert_eq!(t.event(&lift(2, 310.0, 100.0), true, t0), (Pass::Keep, None));
        assert_eq!(t.event(&moved(1, 120.0, 100.0), true, t0).1, None);
        assert_eq!(t.event(&lift(1, 120.0, 100.0), true, t0), (Pass::Keep, None));
    }
}
