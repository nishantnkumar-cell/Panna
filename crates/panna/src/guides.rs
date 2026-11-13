//! Rulers, a grid and guides, as Acrobat's View › Show/Hide › Rulers & Grids: rulers along
//! the top and left of the pages measured from the current page's top-left corner, a grid
//! over every page, and guides (lines across every page) added by clicking a ruler. Placing
//! comments, text, fields and measurements can snap to the grid and guides. Nothing here
//! changes the document.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::Message;
use crate::measure::Unit;

/// How wide the rulers are, in view pixels.
pub const RULER: f32 = 20.0;
/// How near a guide, in view pixels, a point snaps to it.
const SNAP: f32 = 6.0;
/// The most guides kept.
const MAX_GUIDES: usize = 64;

/// A line across every page, at a distance in points from the page's top (across) or left
/// (down) edge as shown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    /// Runs top to bottom, at `at` from the left; else left to right, at `at` from the top.
    pub down: bool,
    pub at: f32,
}

#[derive(Debug, Default)]
pub struct Guides {
    pub rulers: bool,
    pub grid: bool,
    pub snap: bool,
    pub guides: Vec<Guide>,
}

/// The grid's spacing in points for `unit`: (major lines, minor lines between).
pub fn steps(unit: Unit) -> (f32, u32) {
    match unit {
        Unit::Inches => (72.0, 4),
        Unit::Points => (72.0, 6),
        // A centimetre, in halves.
        Unit::Millimetres | Unit::Centimetres => (72.0 / 2.54, 2),
    }
}

impl Guides {
    /// Adds a guide, or takes away one within a few pixels of it (`zoom` is pixels a point).
    pub fn toggle(&mut self, down: bool, at: f32, zoom: f32) {
        let near = SNAP / zoom.max(0.01);
        if let Some(i) = self
            .guides
            .iter()
            .position(|g| g.down == down && (g.at - at).abs() <= near)
        {
            self.guides.remove(i);
        } else if self.guides.len() < MAX_GUIDES && at.is_finite() && at >= 0.0 {
            self.guides.push(Guide { down, at });
        }
    }

    /// `at` (in points from the page's top-left as shown) moved onto the nearest grid line
    /// and guides, when snapping is on.
    pub fn snapped(&self, at: (f32, f32), unit: Unit, zoom: f32) -> (f32, f32) {
        if !self.snap {
            return at;
        }
        let near = SNAP / zoom.max(0.01);
        let (major, minor) = steps(unit);
        let step = major / minor.max(1) as f32;
        let axis = |v: f32, down: bool| {
            let guide = self
                .guides
                .iter()
                .filter(|g| g.down == down && (g.at - v).abs() <= near)
                .min_by(|a, b| (a.at - v).abs().total_cmp(&(b.at - v).abs()))
                .map(|g| g.at);
            match guide {
                Some(g) => g,
                None if self.grid => (v / step).round() * step,
                None => v,
            }
        };
        (axis(at.0, true), axis(at.1, false))
    }

    /// The grid and guides over a page `size` view pixels, `zoom` pixels a point.
    pub fn overlay<'a>(&self, size: Size, zoom: f32, unit: Unit) -> Option<Element<'a, Message>> {
        if !self.grid && self.guides.is_empty() {
            return None;
        }
        Some(
            canvas(PageLines {
                grid: self.grid.then(|| steps(unit)),
                guides: self.guides.clone(),
                zoom,
            })
            .width(size.width)
            .height(size.height)
            .into(),
        )
    }

    /// The ruler along the top (`down` false) or left. `origin` is where the current page's
    /// edge is in the ruler, in pixels, and `pointer` where the pointer is, if over a page.
    pub fn ruler(&self, across: bool, origin: f32, zoom: f32, unit: Unit, pointer: Option<f32>) -> RulerView {
        RulerView(Ruler {
            across,
            origin,
            zoom,
            unit,
            pointer,
            guides: self
                .guides
                .iter()
                .filter(|g| g.down == across)
                .map(|g| g.at)
                .collect(),
            readout: None,
        })
    }
}

/// A ruler, ready to show.
pub struct RulerView(Ruler);

impl RulerView {
    /// Text shown at the ruler's far end: the pointer's place on the page.
    pub fn readout(mut self, text: Option<String>) -> Self {
        self.0.readout = text;
        self
    }
}

impl<'a> From<RulerView> for Element<'a, Message> {
    fn from(view: RulerView) -> Self {
        let (w, h) = if view.0.across {
            (Length::Fill, Length::Fixed(RULER))
        } else {
            (Length::Fixed(RULER), Length::Fill)
        };
        canvas(view.0).width(w).height(h).into()
    }
}

fn canvas<P: canvas::Program<Message>>(program: P) -> canvas::Canvas<P, Message> {
    iced::widget::canvas(program)
}

struct PageLines {
    grid: Option<(f32, u32)>,
    guides: Vec<Guide>,
    zoom: f32,
}

impl canvas::Program<Message> for PageLines {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        if let Some((major, minor)) = self.grid {
            let step = major / minor.max(1) as f32 * self.zoom;
            // Too dense to see: only the major lines.
            let (step, every) = if step < 6.0 {
                (major * self.zoom, 1)
            } else {
                (step, minor.max(1))
            };
            if step >= 2.0 {
                let line = |frame: &mut Frame, a: Point, b: Point, strong: bool| {
                    let color = Color::from_rgba(0.2, 0.45, 0.85, if strong { 0.35 } else { 0.15 });
                    frame.stroke(
                        &Path::line(a, b),
                        Stroke::default().with_color(color).with_width(1.0),
                    );
                };
                let mut i = 1u32;
                while i as f32 * step < w && i < 10_000 {
                    let x = i as f32 * step;
                    line(
                        &mut frame,
                        Point::new(x, 0.0),
                        Point::new(x, h),
                        i.is_multiple_of(every),
                    );
                    i += 1;
                }
                let mut i = 1u32;
                while i as f32 * step < h && i < 10_000 {
                    let y = i as f32 * step;
                    line(
                        &mut frame,
                        Point::new(0.0, y),
                        Point::new(w, y),
                        i.is_multiple_of(every),
                    );
                    i += 1;
                }
            }
        }
        let guide = Color::from_rgb(0.1, 0.75, 0.85);
        for g in &self.guides {
            let v = g.at * self.zoom;
            let (a, b) = if g.down {
                (Point::new(v, 0.0), Point::new(v, h))
            } else {
                (Point::new(0.0, v), Point::new(w, v))
            };
            frame.stroke(
                &Path::line(a, b),
                Stroke::default().with_color(guide).with_width(1.0),
            );
        }
        vec![frame.into_geometry()]
    }
}

struct Ruler {
    across: bool,
    origin: f32,
    zoom: f32,
    unit: Unit,
    pointer: Option<f32>,
    guides: Vec<f32>,
    readout: Option<String>,
}

impl canvas::Program<Message> for Ruler {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event else {
            return None;
        };
        let p = cursor.position_in(bounds)?;
        let along = if self.across { p.x } else { p.y };
        let at = (along - self.origin) / self.zoom.max(0.01);
        Some(canvas::Action::publish(Message::RulerClick(self.across, at)).and_capture())
    }

    fn mouse_interaction(&self, _state: &(), bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let palette = theme.extended_palette();
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), palette.background.weak.color);
        let ink = palette.background.weak.text;
        let length = if self.across { bounds.width } else { bounds.height };
        let (major, minor) = steps(self.unit);
        let major_px = major * self.zoom;
        // Fewer labels when they would crowd.
        let label_every = (40.0 / major_px.max(0.01)).ceil().max(1.0) as i64;
        let minor_px = major_px / minor.max(1) as f32;
        let tick = |frame: &mut Frame, v: f32, size: f32| {
            let (a, b) = if self.across {
                (Point::new(v, RULER - size), Point::new(v, RULER))
            } else {
                (Point::new(RULER - size, v), Point::new(RULER, v))
            };
            frame.stroke(
                &Path::line(a, b),
                Stroke::default().with_color(ink).with_width(1.0),
            );
        };
        if minor_px >= 3.0 {
            let first = (-self.origin / minor_px).floor() as i64;
            let last = ((length - self.origin) / minor_px).ceil() as i64;
            for i in first.max(-100_000)..=last.min(100_000) {
                let v = self.origin + i as f32 * minor_px;
                let is_major = i.rem_euclid(i64::from(minor.max(1))) == 0;
                tick(&mut frame, v, if is_major { RULER * 0.6 } else { RULER * 0.25 });
                let n = i.div_euclid(i64::from(minor.max(1)));
                if is_major && n.rem_euclid(label_every) == 0 {
                    // Labelled in the unit: 1, 2, 3 in or cm, or 72, 144 pt.
                    let value = match self.unit {
                        Unit::Points => n * 72,
                        Unit::Millimetres => n * 10,
                        _ => n,
                    };
                    let position = if self.across {
                        Point::new(v + 2.0, 1.0)
                    } else {
                        Point::new(1.0, v + 2.0)
                    };
                    frame.fill_text(Text {
                        content: value.to_string(),
                        position,
                        color: ink,
                        size: 9.0.into(),
                        ..Text::default()
                    });
                }
            }
        }
        let guide = Color::from_rgb(0.1, 0.75, 0.85);
        for g in &self.guides {
            let v = self.origin + g * self.zoom;
            let (a, b) = if self.across {
                (Point::new(v, 0.0), Point::new(v, RULER))
            } else {
                (Point::new(0.0, v), Point::new(RULER, v))
            };
            frame.stroke(
                &Path::line(a, b),
                Stroke::default().with_color(guide).with_width(2.0),
            );
        }
        if let Some(p) = self.pointer {
            let v = self.origin + p * self.zoom;
            let (a, b) = if self.across {
                (Point::new(v, 0.0), Point::new(v, RULER))
            } else {
                (Point::new(0.0, v), Point::new(RULER, v))
            };
            frame.stroke(
                &Path::line(a, b),
                Stroke::default()
                    .with_color(palette.primary.base.color)
                    .with_width(1.0),
            );
        }
        if let Some(readout) = &self.readout {
            let width = 7.0 * readout.chars().count() as f32 + 12.0;
            let x = (bounds.width - width).max(0.0);
            frame.fill_rectangle(
                Point::new(x, 0.0),
                Size::new(width, RULER),
                palette.background.base.color,
            );
            frame.fill_text(Text {
                content: readout.clone(),
                position: Point::new(x + 6.0, 4.0),
                color: palette.background.base.text,
                size: 11.0.into(),
                ..Text::default()
            });
        }
        // The edge against the pages.
        let (a, b) = if self.across {
            (
                Point::new(0.0, RULER - 0.5),
                Point::new(bounds.width, RULER - 0.5),
            )
        } else {
            (
                Point::new(RULER - 0.5, 0.0),
                Point::new(RULER - 0.5, bounds.height),
            )
        };
        frame.stroke(
            &Path::line(a, b),
            Stroke::default()
                .with_color(palette.background.strong.color)
                .with_width(1.0),
        );
        vec![frame.into_geometry()]
    }
}

/// The pointer's place on the page in `unit`, for the corner where the rulers meet.
pub fn coordinates(at: (f32, f32), unit: Unit) -> String {
    let per = unit.per_point();
    let digits = match unit {
        Unit::Points | Unit::Millimetres => 0,
        Unit::Centimetres => 1,
        Unit::Inches => 2,
    };
    format!(
        "{:.*}, {:.*} {}",
        digits,
        f64::from(at.0) * per,
        digits,
        f64::from(at.1) * per,
        unit.symbol()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guides_are_added_and_taken_away_by_clicking_near_them() {
        let mut g = Guides::default();
        g.toggle(true, 100.0, 1.0);
        g.toggle(false, 50.0, 1.0);
        assert_eq!(g.guides.len(), 2);
        // Within a few pixels takes the guide away again.
        g.toggle(true, 103.0, 1.0);
        assert_eq!(
            g.guides,
            [Guide {
                down: false,
                at: 50.0
            }]
        );
    }

    #[test]
    fn points_snap_to_guides_first_then_the_grid() {
        let mut g = Guides {
            snap: true,
            grid: true,
            ..Guides::default()
        };
        g.toggle(true, 100.0, 1.0);
        // Inches: lines every 18 points.
        assert_eq!(g.snapped((97.0, 40.0), Unit::Inches, 1.0), (100.0, 36.0));
        g.snap = false;
        assert_eq!(g.snapped((97.0, 40.0), Unit::Inches, 1.0), (97.0, 40.0));
    }
}
