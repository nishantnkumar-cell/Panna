//! Fill & Sign: a signature and initials, drawn or typed once and placed as often as needed,
//! plus the date and check marks. Signatures are kept only while the app is open.

use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont as _, point};
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{Column, button, column, container, image, row, space, text, text_input};
use iced::{Center, Element, Fill, Point, Rectangle, Renderer, Theme, mouse};
use panna_core::{Color, StampImage};

use crate::Message;
use crate::annotate;
use crate::icons::{self, Icon};

/// Dancing Script SemiBold (SIL Open Font License, see fonts/LICENSE-dancing-script), for typed signatures.
const FONT: &[u8] = include_bytes!("../fonts/DancingScript-SemiBold.ttf");

/// The drawing pad's size, in view pixels.
const PAD_WIDTH: f32 = 360.0;
const PAD_HEIGHT: f32 = 130.0;
/// Pictures are drawn this many times larger than the pad, so they print sharply.
const SCALE: f32 = 4.0;
/// The pen's width on the pad.
const PEN: f32 = 2.5;
/// How tall typed letters are drawn, in picture pixels.
const TYPE_HEIGHT: f32 = 200.0;
const MAX_TYPED: usize = 40;
/// Strokes and points kept, so a runaway drag can't grow without end.
const MAX_STROKES: usize = 200;
const MAX_POINTS: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Signature,
    Initials,
}

impl Which {
    /// The name the placed stamp is given in the file.
    pub fn name(self) -> &'static str {
        match self {
            Which::Signature => "Signature",
            Which::Initials => "Initials",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Draw,
    Type,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// Opens the pad to make a new signature or initials.
    Create(Which),
    Mode(Mode),
    Typed(String),
    /// The pen went down, moved or came up on the pad, at a point in pad pixels.
    PenDown(Point),
    PenMove(Point),
    PenUp,
    Clear,
    /// Keeps what is on the pad and places it.
    Use,
    /// Back from the pad to the choices.
    Back,
    Place(Which),
    Date,
    Check,
    Cross,
}

/// A signature or initials, ready to place.
#[derive(Debug, Clone)]
pub struct Made {
    pub image: StampImage,
    preview: image::Handle,
}

#[derive(Debug)]
pub struct Pad {
    which: Which,
    mode: Mode,
    strokes: Vec<Vec<(f32, f32)>>,
    down: bool,
    typed: String,
    /// The typed words drawn, and its preview.
    typed_picture: Option<(Picture, image::Handle)>,
}

#[derive(Debug, Default)]
pub struct Signer {
    signature: Option<Made>,
    initials: Option<Made>,
    pub pad: Option<Pad>,
}

/// A drawn picture, straight (not premultiplied) RGBA.
#[derive(Debug, Clone)]
struct Picture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Picture {
    fn made(self) -> Option<Made> {
        let image = StampImage::from_rgba(self.width, self.height, &self.rgba).ok()?;
        Some(Made {
            image,
            preview: image::Handle::from_rgba(self.width, self.height, self.rgba),
        })
    }
}

/// What the card asks the comment tools to do.
pub enum Action {
    None,
    /// Place this picture, named so, where the user clicked.
    Picture(StampImage, &'static str),
    Date,
    Check,
    Cross,
}

impl Signer {
    pub fn made(&self, which: Which) -> Option<&Made> {
        match which {
            Which::Signature => self.signature.as_ref(),
            Which::Initials => self.initials.as_ref(),
        }
    }

    pub fn update(&mut self, event: Event, ink: Color) -> Action {
        match event {
            Event::Create(which) => {
                self.pad = Some(Pad {
                    which,
                    mode: Mode::Draw,
                    strokes: Vec::new(),
                    down: false,
                    typed: String::new(),
                    typed_picture: None,
                });
            }
            Event::Mode(mode) => {
                if let Some(pad) = &mut self.pad {
                    pad.mode = mode;
                }
            }
            Event::Typed(value) => {
                if let Some(pad) = &mut self.pad {
                    pad.typed = value
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(MAX_TYPED)
                        .collect();
                    pad.typed_picture = draw_text(pad.typed.trim(), ink).map(|p| {
                        let handle = image::Handle::from_rgba(p.width, p.height, p.rgba.clone());
                        (p, handle)
                    });
                }
            }
            Event::PenDown(at) => {
                if let Some(pad) = &mut self.pad
                    && pad.strokes.len() < MAX_STROKES
                {
                    pad.down = true;
                    pad.strokes.push(vec![(at.x, at.y)]);
                }
            }
            Event::PenMove(at) => {
                if let Some(pad) = self.pad.as_mut().filter(|p| p.down) {
                    let total: usize = pad.strokes.iter().map(Vec::len).sum();
                    if total < MAX_POINTS
                        && let Some(stroke) = pad.strokes.last_mut()
                    {
                        stroke.push((at.x, at.y));
                    }
                }
            }
            Event::PenUp => {
                if let Some(pad) = &mut self.pad {
                    pad.down = false;
                }
            }
            Event::Clear => {
                if let Some(pad) = &mut self.pad {
                    pad.strokes.clear();
                    pad.typed.clear();
                    pad.typed_picture = None;
                }
            }
            Event::Back => self.pad = None,
            Event::Use => {
                let Some(pad) = self.pad.take() else {
                    return Action::None;
                };
                let picture = match pad.mode {
                    Mode::Draw => draw_strokes(&pad.strokes, ink),
                    Mode::Type => pad.typed_picture.map(|(p, _)| p),
                };
                let Some(made) = picture.and_then(Picture::made) else {
                    return Action::None;
                };
                let image = made.image.clone();
                match pad.which {
                    Which::Signature => self.signature = Some(made),
                    Which::Initials => self.initials = Some(made),
                }
                return Action::Picture(image, pad.which.name());
            }
            Event::Place(which) => {
                if let Some(made) = self.made(which) {
                    return Action::Picture(made.image.clone(), which.name());
                }
                return self.update(Event::Create(which), ink);
            }
            Event::Date => return Action::Date,
            Event::Check => return Action::Check,
            Event::Cross => return Action::Cross,
        }
        Action::None
    }

    /// The Fill & Sign card, or the pad while one is being made.
    pub fn card(&self) -> Element<'_, Message> {
        let ev = |e: Event| Message::Annotate(annotate::Event::Sign(e));
        let cancel = button("Cancel")
            .style(button::secondary)
            .on_press(Message::Annotate(annotate::Event::EditCancel));
        let body: Element<'_, Message> = if let Some(pad) = &self.pad {
            let title = match pad.which {
                Which::Signature => "Your signature",
                Which::Initials => "Your initials",
            };
            let modes = row![
                icons::toggle(
                    Icon::Pen,
                    "Draw it",
                    pad.mode == Mode::Draw,
                    ev(Event::Mode(Mode::Draw))
                ),
                icons::toggle(
                    Icon::TextBox,
                    "Type it",
                    pad.mode == Mode::Type,
                    ev(Event::Mode(Mode::Type))
                ),
                space::horizontal(),
                icons::button(Icon::Eraser, "Clear", Some(ev(Event::Clear))),
            ]
            .spacing(4)
            .align_y(Center);
            let area: Element<'_, Message> = match pad.mode {
                Mode::Draw => canvas::Canvas::new(Drawing {
                    strokes: &pad.strokes,
                })
                .width(PAD_WIDTH)
                .height(PAD_HEIGHT)
                .into(),
                Mode::Type => {
                    let preview: Element<'_, Message> = match &pad.typed_picture {
                        Some((_, handle)) => image(handle.clone()).height(70).into(),
                        None => space::vertical().height(70).into(),
                    };
                    column![
                        text_input(
                            match pad.which {
                                Which::Signature => "Type your name",
                                Which::Initials => "Type your initials",
                            },
                            &pad.typed
                        )
                        .id(input_id())
                        .on_input(move |v| ev(Event::Typed(v)))
                        .on_submit(ev(Event::Use)),
                        container(preview)
                            .width(Fill)
                            .height(PAD_HEIGHT - 40.0)
                            .center(Fill),
                    ]
                    .spacing(6)
                    .into()
                }
            };
            let ready = match pad.mode {
                Mode::Draw => pad.strokes.iter().any(|s| !s.is_empty()),
                Mode::Type => pad.typed_picture.is_some(),
            };
            column![
                text(title).size(16),
                modes,
                container(area)
                    .width(Fill)
                    .height(PAD_HEIGHT)
                    .style(|theme: &Theme| {
                        let palette = theme.extended_palette();
                        container::Style::default()
                            .background(iced::Color::WHITE)
                            .border(iced::Border {
                                color: palette.background.strong.color,
                                width: 1.0,
                                radius: 6.0.into(),
                            })
                    }),
                text("Kept only until Panna closes.").size(12),
                row![
                    space::horizontal(),
                    button("Back").style(button::secondary).on_press(ev(Event::Back)),
                    button("Place it").on_press_maybe(ready.then(|| ev(Event::Use))),
                ]
                .spacing(8),
            ]
            .spacing(10)
            .into()
        } else {
            let mut list = Column::new().spacing(6);
            for (which, create) in [
                (Which::Signature, "Make your signature…"),
                (Which::Initials, "Make your initials…"),
            ] {
                let line: Element<'_, Message> = match self.made(which) {
                    Some(made) => row![
                        button(image(made.preview.clone()).height(36))
                            .style(button::secondary)
                            .width(Fill)
                            .on_press(ev(Event::Place(which))),
                        icons::button(Icon::Pen, "Make a new one", Some(ev(Event::Create(which)))),
                    ]
                    .spacing(4)
                    .align_y(Center)
                    .into(),
                    None => button(text(create).size(13))
                        .style(button::secondary)
                        .width(Fill)
                        .on_press(ev(Event::Create(which)))
                        .into(),
                };
                list = list.push(line);
            }
            let marks = row![
                button(
                    row![icons::glyph(Icon::Calendar, true), text("Today's date").size(13)]
                        .spacing(6)
                        .align_y(Center)
                )
                .style(button::secondary)
                .on_press(ev(Event::Date)),
                space::horizontal(),
                icons::button(Icon::Check, "Check mark", Some(ev(Event::Check))),
                icons::button(Icon::Close, "Cross", Some(ev(Event::Cross))),
            ]
            .spacing(4)
            .align_y(Center);
            column![
                text("Fill & Sign").size(16),
                list,
                marks,
                row![space::horizontal(), cancel],
            ]
            .spacing(10)
            .into()
        };
        container(container(body).width(PAD_WIDTH + 2.0))
            .padding(16)
            .style(container::rounded_box)
            .into()
    }
}

pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("sign-typed")
}

fn rgb8(ink: Color) -> [u8; 3] {
    ink.0.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The strokes drawn as a picture, cut to the ink with a small margin.
fn draw_strokes(strokes: &[Vec<(f32, f32)>], ink: Color) -> Option<Picture> {
    let points = || strokes.iter().flatten();
    let (x0, y0, x1, y1) = points().fold(
        (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
        |(a, b, c, d), &(x, y)| (a.min(x), b.min(y), c.max(x), d.max(y)),
    );
    if x0 > x1 {
        return None;
    }
    let margin = PEN * 2.0;
    let width = ((x1 - x0 + margin * 2.0) * SCALE).ceil() as u32;
    let height = ((y1 - y0 + margin * 2.0) * SCALE).ceil() as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width.max(1), height.max(1))?;
    let mut paint = tiny_skia::Paint::default();
    let [r, g, b] = rgb8(ink);
    paint.set_color_rgba8(r, g, b, 255);
    paint.anti_alias = true;
    let pen = tiny_skia::Stroke {
        width: PEN * SCALE,
        line_cap: tiny_skia::LineCap::Round,
        line_join: tiny_skia::LineJoin::Round,
        ..tiny_skia::Stroke::default()
    };
    let at = |&(x, y): &(f32, f32)| ((x - x0 + margin) * SCALE, (y - y0 + margin) * SCALE);
    for stroke in strokes {
        let mut path = tiny_skia::PathBuilder::new();
        let mut points = stroke.iter().map(at);
        let Some((sx, sy)) = points.next() else { continue };
        path.move_to(sx, sy);
        let mut any = false;
        for (x, y) in points {
            path.line_to(x, y);
            any = true;
        }
        if !any {
            // A dot: a round cap needs a line to sit on.
            path.line_to(sx + 0.01, sy);
        }
        if let Some(path) = path.finish() {
            pixmap.stroke_path(&path, &paint, &pen, tiny_skia::Transform::identity(), None);
        }
    }
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Some(Picture {
        width: pixmap.width(),
        height: pixmap.height(),
        rgba,
    })
}

/// Typed words drawn in a handwriting font, cut to the letters with a small margin.
fn draw_text(words: &str, ink: Color) -> Option<Picture> {
    if words.is_empty() {
        return None;
    }
    let font = FontRef::try_from_slice(FONT).ok()?;
    let scale = PxScale::from(TYPE_HEIGHT);
    let scaled = font.as_scaled(scale);
    let mut x = 0.0;
    let mut previous = None;
    let mut glyphs = Vec::new();
    for c in words.chars() {
        let id = scaled.glyph_id(c);
        if let Some(p) = previous {
            x += scaled.kern(p, id);
        }
        glyphs.push(id.with_scale_and_position(scale, point(x, scaled.ascent())));
        x += scaled.h_advance(id);
        previous = Some(id);
    }
    let outlines: Vec<_> = glyphs.into_iter().filter_map(|g| font.outline_glyph(g)).collect();
    let (x0, y0, x1, y1) = outlines
        .iter()
        .map(|o| o.px_bounds())
        .fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |(a, b, c, d), r| {
            (a.min(r.min.x), b.min(r.min.y), c.max(r.max.x), d.max(r.max.y))
        });
    if x0 > x1 {
        return None;
    }
    let margin = TYPE_HEIGHT * 0.05;
    let width = (x1 - x0 + margin * 2.0).ceil() as u32;
    let height = (y1 - y0 + margin * 2.0).ceil() as u32;
    let [r, g, b] = rgb8(ink);
    let mut alpha = vec![0.0f32; width as usize * height as usize];
    for outline in &outlines {
        let bounds = outline.px_bounds();
        let left = (bounds.min.x - x0 + margin) as i64;
        let top = (bounds.min.y - y0 + margin) as i64;
        outline.draw(|gx, gy, coverage| {
            let px = left + i64::from(gx);
            let py = top + i64::from(gy);
            if px < 0 || py < 0 || px >= i64::from(width) || py >= i64::from(height) {
                return;
            }
            let index = py as usize * width as usize + px as usize;
            if let Some(a) = alpha.get_mut(index) {
                // Overlapping joins between letters add up, but never past full ink.
                *a = (*a + coverage).min(1.0);
            }
        });
    }
    let rgba = alpha
        .iter()
        .flat_map(|a| [r, g, b, (a * 255.0).round() as u8])
        .collect();
    Some(Picture { width, height, rgba })
}

/// The pad's strokes, drawn as they are made, and the pen's movements.
struct Drawing<'a> {
    strokes: &'a [Vec<(f32, f32)>],
}

impl canvas::Program<Message> for Drawing<'_> {
    /// Whether the pen went down on the pad and is still down.
    type State = bool;

    fn update(
        &self,
        down: &mut bool,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let ev = |e: Event| canvas::Action::publish(Message::Annotate(annotate::Event::Sign(e)));
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                *down = true;
                Some(ev(Event::PenDown(p)).and_capture())
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) if *down => {
                // Moves just outside the pad are kept, so lines reach its edges.
                let p = cursor.position_from(bounds.position())?;
                let inside = Point::new(p.x.clamp(0.0, bounds.width), p.y.clamp(0.0, bounds.height));
                Some(ev(Event::PenMove(inside)).and_capture())
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if *down => {
                *down = false;
                Some(ev(Event::PenUp).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _down: &bool,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        // The line to sign on.
        let line_y = bounds.height * 0.72;
        frame.stroke(
            &Path::line(Point::new(16.0, line_y), Point::new(bounds.width - 16.0, line_y)),
            Stroke::default()
                .with_color(iced::Color::from_rgb(0.75, 0.75, 0.75))
                .with_width(1.0),
        );
        let ink = Stroke::default()
            .with_color(iced::Color::from_rgb(0.1, 0.1, 0.1))
            .with_width(PEN)
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);
        for stroke in self.strokes {
            let mut points = stroke.iter().map(|&(x, y)| Point::new(x, y));
            let Some(first) = points.next() else { continue };
            let path = Path::new(|b| {
                b.move_to(first);
                for p in points {
                    b.line_to(p);
                }
            });
            frame.stroke(&path, ink);
            if stroke.len() == 1 {
                frame.fill(
                    &Path::circle(first, PEN / 2.0),
                    iced::Color::from_rgb(0.1, 0.1, 0.1),
                );
            }
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strokes_become_a_picture_cut_to_the_ink() {
        let picture = draw_strokes(
            &[vec![(10.0, 10.0), (60.0, 30.0)], vec![(20.0, 40.0)]],
            Color::BLACK,
        )
        .expect("a picture");
        let margin = PEN * 2.0 * 2.0;
        assert_eq!(picture.width, ((50.0 + margin) * SCALE).ceil() as u32);
        assert_eq!(picture.height, ((30.0 + margin) * SCALE).ceil() as u32);
        assert!(picture.rgba.chunks(4).any(|p| p.get(3) == Some(&255)));
        assert!(picture.rgba.chunks(4).any(|p| p.get(3) == Some(&0)));
        assert!(draw_strokes(&[], Color::BLACK).is_none());
    }

    #[test]
    fn typed_words_are_drawn_in_ink() {
        let picture = draw_text("Nishant", Color([0.0, 0.0, 1.0])).expect("a picture");
        assert!(picture.width > picture.height);
        let inked: Vec<_> = picture
            .rgba
            .chunks(4)
            .filter(|p| p.get(3) == Some(&255))
            .collect();
        assert!(!inked.is_empty());
        assert!(inked.iter().all(|p| p.get(..3) == Some(&[0, 0, 255][..])));
        assert!(draw_text("", Color::BLACK).is_none());
    }

    #[test]
    fn a_made_signature_is_kept_and_placed_again() {
        let mut signer = Signer::default();
        assert!(matches!(
            signer.update(Event::Place(Which::Signature), Color::BLACK),
            Action::None
        ));
        assert!(signer.pad.is_some());
        signer.update(Event::PenDown(Point::new(5.0, 5.0)), Color::BLACK);
        signer.update(Event::PenMove(Point::new(50.0, 20.0)), Color::BLACK);
        signer.update(Event::PenUp, Color::BLACK);
        assert!(matches!(
            signer.update(Event::Use, Color::BLACK),
            Action::Picture(_, "Signature")
        ));
        assert!(signer.pad.is_none() && signer.made(Which::Signature).is_some());
        assert!(matches!(
            signer.update(Event::Place(Which::Signature), Color::BLACK),
            Action::Picture(..)
        ));
        assert!(signer.made(Which::Initials).is_none());
    }
}
