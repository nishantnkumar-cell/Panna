//! The Loupe tool and the Pan & Zoom window, as Acrobat's View › Zoom: a floating panel that
//! magnifies the page around the pointer, and one with the whole page and a box for the part
//! in the window, which can be clicked or dragged to move the view.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{Stack, column, container, image, pick_list, row, text};
use iced::{Center, Element, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::Message;
use crate::icons::{self, Icon};

/// The loupe's view, in pixels.
pub const LOUPE: Size = Size::new(280.0, 200.0);
/// The pan and zoom window's width, in pixels.
const PAN_WIDTH: f32 = 180.0;

/// How much the loupe magnifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Magnify(pub u8);

impl Magnify {
    pub const ALL: [Magnify; 4] = [Magnify(2), Magnify(3), Magnify(4), Magnify(8)];
}

impl Default for Magnify {
    fn default() -> Self {
        Magnify(3)
    }
}

impl std::fmt::Display for Magnify {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}×", self.0)
    }
}

/// The part of `handle`, an image of a page shown `shown` pixels big, that the loupe shows
/// around `at` when magnifying by `magnify`: cut out of the page's pixels, with white past
/// its edges.
fn cut(handle: &image::Handle, shown: Size, at: Point, magnify: Magnify) -> Option<image::Handle> {
    let image::Handle::Rgba {
        width,
        height,
        pixels,
        ..
    } = handle
    else {
        return None;
    };
    let (w, h) = (*width as usize, *height as usize);
    if w == 0 || h == 0 || shown.width < 1.0 || shown.height < 1.0 || pixels.len() < w * h * 4 {
        return None;
    }
    // Pixels of the image for each shown pixel.
    let per = w as f32 / shown.width;
    let m = f32::from(magnify.0);
    let cw = ((LOUPE.width / m * per).round() as usize).clamp(1, 4096);
    let ch = ((LOUPE.height / m * per).round() as usize).clamp(1, 4096);
    let x0 = (at.x * per).round() as i64 - (cw / 2) as i64;
    let y0 = (at.y * per).round() as i64 - (ch / 2) as i64;
    let mut out = vec![255u8; cw * ch * 4];
    for row in 0..ch {
        let y = y0 + row as i64;
        if y < 0 || y >= h as i64 {
            continue;
        }
        let from = x0.max(0);
        let to = (x0 + cw as i64).min(w as i64);
        if from >= to {
            continue;
        }
        let src = (y as usize * w + from as usize) * 4..(y as usize * w + to as usize) * 4;
        let dst = (row * cw + (from - x0) as usize) * 4;
        let len = src.len();
        if let (Some(to), Some(from)) = (out.get_mut(dst..dst + len), pixels.get(src)) {
            to.copy_from_slice(from);
        }
    }
    Some(image::Handle::from_rgba(cw as u32, ch as u32, out))
}

/// The loupe: the page image `handle`, shown `shown` pixels big, magnified around `at`.
pub fn loupe<'a>(
    handle: Option<&image::Handle>,
    shown: Size,
    at: Point,
    magnify: Magnify,
) -> Element<'a, Message> {
    let view: Element<'a, Message> = match handle.and_then(|h| cut(h, shown, at, magnify)) {
        Some(part) => image(part)
            .width(LOUPE.width)
            .height(LOUPE.height)
            .content_fit(iced::ContentFit::Fill)
            .filter_method(image::FilterMethod::Linear)
            .into(),
        None => container(text("Point at a page to look closer").size(12))
            .center(LOUPE.width)
            .height(LOUPE.height)
            .into(),
    };
    panel(
        "Loupe",
        column![
            view,
            row![
                text("Magnify").size(12),
                pick_list(Magnify::ALL, Some(magnify), Message::LoupeMagnify).text_size(12),
                iced::widget::space::horizontal(),
                icons::button(Icon::Close, "Close the loupe", Some(Message::ToggleLoupe)),
            ]
            .spacing(6)
            .align_y(Center),
        ]
        .spacing(6)
        .width(LOUPE.width)
        .into(),
    )
}

/// The pan and zoom window: page image `handle` of a page shaped `page` (width, height),
/// with `visible` (x0, y0, x1, y1 as fractions of the page) boxed.
pub fn pan_zoom<'a>(
    handle: Option<image::Handle>,
    page: Size,
    visible: [f32; 4],
    zoom: f32,
) -> Element<'a, Message> {
    let height = PAN_WIDTH * page.height / page.width.max(1.0);
    let height = height.clamp(40.0, 400.0);
    let mut layers: Vec<Element<'a, Message>> = Vec::new();
    if let Some(handle) = handle {
        layers.push(image(handle).width(PAN_WIDTH).height(height).into());
    }
    layers.push(
        canvas::Canvas::new(Window { visible })
            .width(PAN_WIDTH)
            .height(height)
            .into(),
    );
    panel(
        "Pan & zoom",
        column![
            container(Stack::with_children(layers))
                .style(|_| container::Style::default().background(iced::Color::WHITE)),
            row![
                icons::button(Icon::ZoomOut, "Zoom out", Some(Message::ZoomOut)),
                text(format!("{:.0}%", zoom * 100.0)).size(12),
                icons::button(Icon::ZoomIn, "Zoom in", Some(Message::ZoomIn)),
                iced::widget::space::horizontal(),
                icons::button(Icon::Close, "Close pan & zoom", Some(Message::TogglePanZoom)),
            ]
            .spacing(4)
            .align_y(Center),
        ]
        .spacing(6)
        .width(PAN_WIDTH)
        .into(),
    )
}

fn panel<'a>(title: &'a str, body: Element<'a, Message>) -> Element<'a, Message> {
    container(column![text(title).size(13), body].spacing(6))
        .padding(8)
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

/// The box for the part of the page in the window; a press or drag moves the view there.
struct Window {
    visible: [f32; 4],
}

impl canvas::Program<Message> for Window {
    type State = bool;

    fn update(
        &self,
        dragging: &mut bool,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let to = |p: Point| {
            Message::PanTo(
                (p.x / bounds.width.max(1.0)).clamp(0.0, 1.0),
                (p.y / bounds.height.max(1.0)).clamp(0.0, 1.0),
            )
        };
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                *dragging = true;
                Some(canvas::Action::publish(to(p)).and_capture())
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) if *dragging => {
                let p = cursor.position_from(bounds.position())?;
                Some(canvas::Action::publish(to(p)).and_capture())
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if *dragging => {
                *dragging = false;
                None
            }
            _ => None,
        }
    }

    fn mouse_interaction(
        &self,
        dragging: &bool,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if *dragging {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        _dragging: &bool,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let [x0, y0, x1, y1] = self.visible.map(|v| v.clamp(0.0, 1.0));
        let top_left = Point::new(x0 * bounds.width, y0 * bounds.height);
        let size = Size::new(
            ((x1 - x0) * bounds.width).max(2.0),
            ((y1 - y0) * bounds.height).max(2.0),
        );
        let color = theme.extended_palette().primary.base.color;
        frame.fill_rectangle(top_left, size, color.scale_alpha(0.12));
        frame.stroke(
            &Path::rectangle(top_left, size),
            Stroke::default().with_color(color).with_width(2.0),
        );
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loupe_cuts_around_the_pointer_with_white_past_the_edge() {
        // A 4×4 page shown 4 pixels big: red on the left half, blue on the right.
        let mut px = Vec::new();
        for _ in 0..4 {
            for x in 0..4 {
                px.extend(if x < 2 { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
            }
        }
        let handle = image::Handle::from_rgba(4, 4, px);
        let Some(image::Handle::Rgba {
            width,
            height,
            pixels,
            ..
        }) = cut(&handle, Size::new(4.0, 4.0), Point::new(0.0, 0.0), Magnify(80))
        else {
            panic!("no cut");
        };
        // 280/80 = 3.5 → 4 across, 200/80 = 2.5 → 3 down (rounded), centred on the corner.
        assert_eq!((width, height), (4, 3));
        // The top left is past the page: white.
        assert_eq!(pixels.get(0..4), Some(&[255, 255, 255, 255][..]));
        // The bottom right is the page's red top left.
        let last = ((2 * 4) + 2) * 4;
        assert_eq!(pixels.get(last..last + 4), Some(&[255, 0, 0, 255][..]));
    }
}
