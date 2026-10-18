//! Measuring tools: distance between two points, the perimeter of a shape and the area inside
//! it, on a page. Measurements are shown, not saved.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{container, pick_list, row, text};
use iced::{Center, Element, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::Message;
use crate::icons::{self, Icon};
use crate::selection::PageMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Distance,
    Perimeter,
    Area,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Unit {
    Points,
    Inches,
    #[default]
    Millimetres,
    Centimetres,
}

impl Unit {
    pub const ALL: [Unit; 4] = [Unit::Millimetres, Unit::Centimetres, Unit::Inches, Unit::Points];

    /// How many of this unit make a point (1/72 inch).
    fn per_point(self) -> f64 {
        match self {
            Unit::Points => 1.0,
            Unit::Inches => 1.0 / 72.0,
            Unit::Millimetres => 25.4 / 72.0,
            Unit::Centimetres => 2.54 / 72.0,
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Unit::Points => "pt",
            Unit::Inches => "in",
            Unit::Millimetres => "mm",
            Unit::Centimetres => "cm",
        }
    }
}

impl std::fmt::Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Unit::Points => "Points",
            Unit::Inches => "Inches",
            Unit::Millimetres => "Millimetres",
            Unit::Centimetres => "Centimetres",
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Event {
    Toggle,
    Kind(Kind),
    Unit(Unit),
}

/// The measuring tool and the measurement in progress, in page points from the page's
/// top-left corner (as [`PageMap`] gives them).
#[derive(Debug, Default)]
pub struct Measure {
    pub on: bool,
    pub kind: Kind,
    pub unit: Unit,
    page: usize,
    points: Vec<(f32, f32)>,
    /// Where the pointer is, while a measurement is unfinished.
    cursor: Option<(f32, f32)>,
    done: bool,
}

/// The total length of the lines through `points`, back to the first when `closed`.
fn length(points: &[(f32, f32)], closed: bool) -> f64 {
    let segment = |a: &(f32, f32), b: &(f32, f32)| f64::from(a.0 - b.0).hypot(f64::from(a.1 - b.1));
    let open: f64 = points
        .iter()
        .zip(points.iter().skip(1))
        .map(|(a, b)| segment(a, b))
        .sum();
    match (closed && points.len() > 2, points.first(), points.last()) {
        (true, Some(first), Some(last)) => open + segment(last, first),
        _ => open,
    }
}

/// The area inside the shape with corners `points` (the shoelace formula).
fn area(points: &[(f32, f32)]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let next = points.iter().cycle().skip(1);
    let twice: f64 = points
        .iter()
        .zip(next)
        .map(|(a, b)| f64::from(a.0) * f64::from(b.1) - f64::from(b.0) * f64::from(a.1))
        .sum();
    twice.abs() / 2.0
}

/// A number with as many decimals as suit its size.
fn number(value: f64) -> String {
    if value >= 100.0 {
        format!("{value:.0}")
    } else if value >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

impl Measure {
    pub fn update(&mut self, event: Event) {
        match event {
            Event::Toggle => {
                self.on = !self.on;
                self.clear();
            }
            Event::Kind(kind) => {
                self.kind = kind;
                self.clear();
            }
            Event::Unit(unit) => self.unit = unit,
        }
    }

    pub fn clear(&mut self) {
        self.points.clear();
        self.cursor = None;
        self.done = false;
    }

    /// Whether a measurement is being made or shown.
    pub fn active(&self) -> bool {
        !self.points.is_empty()
    }

    /// A click on `page` at `at`: a new corner, or the start of a new measurement.
    pub fn press(&mut self, page: usize, at: (f32, f32)) {
        if self.done || page != self.page {
            self.clear();
            self.page = page;
        }
        // A double-click's second press lands on the same spot.
        if self
            .points
            .last()
            .is_some_and(|p| (p.0 - at.0).hypot(p.1 - at.1) < 0.5)
        {
            return;
        }
        self.points.push(at);
        if self.kind == Kind::Distance && self.points.len() == 2 {
            self.done = true;
            self.cursor = None;
        }
    }

    pub fn hover(&mut self, page: usize, at: (f32, f32)) {
        if !self.done && page == self.page && !self.points.is_empty() {
            self.cursor = Some(at);
        }
    }

    /// Ends a perimeter or area (on a double-click).
    pub fn finish(&mut self) {
        if self.kind != Kind::Distance && self.points.len() >= 2 {
            self.done = true;
            self.cursor = None;
        }
    }

    /// The corners, with the pointer as the next one while unfinished.
    fn shape(&self) -> Vec<(f32, f32)> {
        let mut points = self.points.clone();
        points.extend(self.cursor);
        points
    }

    /// The measurement, such as "Distance: 25.4 mm".
    pub fn result(&self) -> String {
        let points = self.shape();
        let unit = self.unit;
        let k = unit.per_point();
        match self.kind {
            _ if points.len() < 2 => match self.kind {
                Kind::Distance => "Click two points to measure between them.".into(),
                _ => "Click each corner, and double-click the last.".into(),
            },
            Kind::Distance => format!(
                "Distance: {} {}",
                number(length(&points, false) * k),
                unit.symbol()
            ),
            Kind::Perimeter => format!(
                "Perimeter: {} {}",
                number(length(&points, true) * k),
                unit.symbol()
            ),
            Kind::Area => format!("Area: {} sq {}", number(area(&points) * k * k), unit.symbol()),
        }
    }

    /// The measurement drawn over page `page`, `size` view pixels in size.
    pub fn overlay(&self, page: usize, map: PageMap, size: Size) -> Option<Element<'_, Message>> {
        if !self.on || page != self.page || self.points.is_empty() {
            return None;
        }
        Some(
            canvas::Canvas::new(Drawing {
                points: self.shape().iter().map(|(x, y)| map.to_view(*x, *y)).collect(),
                closed: self.kind != Kind::Distance,
            })
            .width(size.width)
            .height(size.height)
            .into(),
        )
    }

    /// The floating bar with the kinds of measurement, the unit and the result.
    pub fn bar(&self) -> Element<'_, Message> {
        let ev = Message::Measure;
        let kind = |kind: Kind, icon: Icon, tip: &'static str| {
            icons::toggle(icon, tip, self.kind == kind, ev(Event::Kind(kind)))
        };
        let content = row![
            kind(Kind::Distance, Icon::Ruler, "Distance"),
            kind(Kind::Perimeter, Icon::Pentagon, "Perimeter"),
            kind(Kind::Area, Icon::Area, "Area"),
            pick_list(Unit::ALL, Some(self.unit), |u| Message::Measure(Event::Unit(u))).text_size(13),
            // A steady width, so the buttons stay put as the number changes.
            text(self.result()).size(13).width(220),
            icons::button(Icon::Close, "Stop measuring (Esc)", Some(ev(Event::Toggle))),
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

struct Drawing {
    points: Vec<Point>,
    closed: bool,
}

impl canvas::Program<Message> for Drawing {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let color = theme.extended_palette().primary.strong.color;
        if let Some(first) = self.points.first().copied() {
            let path = Path::new(|b| {
                b.move_to(first);
                for p in self.points.iter().skip(1) {
                    b.line_to(*p);
                }
                if self.closed && self.points.len() > 2 {
                    b.close();
                }
            });
            if self.closed && self.points.len() > 2 {
                frame.fill(&path, color.scale_alpha(0.15));
            }
            frame.stroke(&path, Stroke::default().with_color(color).with_width(2.0));
            for p in &self.points {
                frame.fill(&Path::circle(*p, 3.5), color);
            }
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_and_areas() {
        let square = [(0.0, 0.0), (72.0, 0.0), (72.0, 72.0), (0.0, 72.0)];
        assert!((length(&square[..2], false) - 72.0).abs() < 1e-9);
        assert!((length(&square, true) - 288.0).abs() < 1e-9);
        assert!((area(&square) - 5184.0).abs() < 1e-9);
        assert!(area(&square[..2]).abs() < 1e-9);
    }

    #[test]
    fn a_distance_takes_two_clicks() {
        let mut m = Measure {
            on: true,
            unit: Unit::Inches,
            ..Measure::default()
        };
        m.press(0, (0.0, 0.0));
        m.hover(0, (36.0, 0.0));
        assert_eq!(m.result(), "Distance: 0.50 in");
        m.press(0, (72.0, 0.0));
        m.hover(0, (500.0, 0.0));
        assert_eq!(m.result(), "Distance: 1.00 in");
        // The next click starts again.
        m.press(0, (10.0, 10.0));
        assert_eq!(m.points.len(), 1);
    }

    #[test]
    fn an_area_ends_on_a_double_click() {
        let mut m = Measure {
            on: true,
            kind: Kind::Area,
            unit: Unit::Points,
            ..Measure::default()
        };
        for p in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (10.0, 10.0)] {
            m.press(0, p);
        }
        m.finish();
        assert_eq!(m.points.len(), 3);
        assert_eq!(m.result(), "Area: 50.0 sq pt");
    }
}
