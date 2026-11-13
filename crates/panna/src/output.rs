//! The Output preview panel, as Acrobat's Print Production › Output Preview: the current page
//! as it would print with only some of the four process inks (cyan, magenta, yellow, black),
//! how much of each ink the page uses, the inks under the pointer, and where the inks
//! together pass a limit. The spot inks the document asks for are listed.
//!
//! The inks are worked out from the page's colours as shown, with a simple conversion that
//! needs no printer profile, so they're an estimate of what a press would use.

use std::cell::RefCell;

use iced::widget::{Column, checkbox, column, image, pick_list, row, space, text};
use iced::{Center, Element, Point, Size};

use crate::Message;
use crate::icons::{self, Icon};

/// The preview's width, in pixels.
const WIDTH: u32 = 240;
/// The four process inks, in plate order.
const INKS: [&str; 4] = ["Cyan", "Magenta", "Yellow", "Black"];

/// The highest total of the inks allowed in one place, in percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limit(pub u16);

impl Limit {
    pub const ALL: [Limit; 6] = [
        Limit(180),
        Limit(200),
        Limit(220),
        Limit(240),
        Limit(260),
        Limit(280),
    ];
}

impl Default for Limit {
    fn default() -> Self {
        Limit(240)
    }
}

impl std::fmt::Display for Limit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}%", self.0)
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Plate(usize, bool),
    Warn(bool),
    Limit(Limit),
    Close,
}

/// What the preview shows for one page image.
#[derive(Debug, Clone)]
struct Preview {
    image: image::Handle,
    /// Each ink's share of the page, 0 to 1.
    coverage: [f32; 4],
    /// The page's share where the inks pass the limit.
    over: f32,
}

/// A page image, by its handle's identity, and the settings.
type Key = (u64, [bool; 4], bool, Limit);

fn id_of(handle: &image::Handle) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    handle.id().hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Default)]
pub struct OutputPreview {
    /// The plates shown.
    pub plates: [bool; 4],
    /// Mark where the inks together pass `limit`.
    pub warn: bool,
    pub limit: Limit,
    cache: RefCell<Option<(Key, Preview)>>,
}

impl OutputPreview {
    pub fn new() -> Self {
        Self {
            plates: [true; 4],
            ..Self::default()
        }
    }

    pub fn update(&mut self, event: &Event) {
        match *event {
            Event::Plate(i, on) => {
                if let Some(p) = self.plates.get_mut(i) {
                    *p = on;
                }
            }
            Event::Warn(on) => self.warn = on,
            Event::Limit(limit) => self.limit = limit,
            Event::Open | Event::Close => {}
        }
    }

    /// The preview of `page`, made once for each page image and setting.
    fn preview(&self, page: &image::Handle) -> Option<Preview> {
        let key = (id_of(page), self.plates, self.warn, self.limit);
        if let Some((k, p)) = self.cache.borrow().as_ref()
            && *k == key
        {
            return Some(p.clone());
        }
        let made = make(page, self.plates, self.warn.then_some(self.limit))?;
        *self.cache.borrow_mut() = Some((key, made.clone()));
        Some(made)
    }
}

/// The process inks for a colour, each 0 to 1, by the usual simple conversion: black takes the
/// part all three share, and cyan, magenta and yellow the rest.
pub fn cmyk(r: u8, g: u8, b: u8) -> [f32; 4] {
    let [r, g, b] = [r, g, b].map(|v| f32::from(v) / 255.0);
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let of = |v: f32| ((1.0 - v - k) / (1.0 - k)).clamp(0.0, 1.0);
    [of(r), of(g), of(b), k]
}

/// A pixel of `rgba` over white paper.
fn over_white(px: &[u8]) -> (u8, u8, u8) {
    let (r, g, b, a) = match *px {
        [r, g, b, a, ..] => (r, g, b, a),
        _ => return (255, 255, 255),
    };
    let mix = |v: u8| {
        let a = u16::from(a);
        ((u16::from(v) * a + 255 * (255 - a)) / 255) as u8
    };
    (mix(r), mix(g), mix(b))
}

/// The inks at a pixel of `handle`, an image of a page shown `shown` pixels big, at `at`.
fn inks_at(handle: &image::Handle, shown: Size, at: Point) -> Option<[f32; 4]> {
    let image::Handle::Rgba {
        width,
        height,
        pixels,
        ..
    } = handle
    else {
        return None;
    };
    if shown.width < 1.0 || shown.height < 1.0 || at.x < 0.0 || at.y < 0.0 {
        return None;
    }
    let x = (at.x / shown.width * *width as f32) as usize;
    let y = (at.y / shown.height * *height as f32) as usize;
    if x >= *width as usize || y >= *height as usize {
        return None;
    }
    let i = (y * *width as usize + x) * 4;
    let (r, g, b) = over_white(pixels.get(i..i + 4)?);
    Some(cmyk(r, g, b))
}

/// The page as printed with only `plates`, at the preview's width, with where the inks pass
/// `limit` marked, and the inks' coverage.
fn make(handle: &image::Handle, plates: [bool; 4], limit: Option<Limit>) -> Option<Preview> {
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
    if w == 0 || h == 0 || pixels.len() < w * h * 4 {
        return None;
    }
    let ow = (WIDTH as usize).min(w);
    let oh = (h * ow / w).max(1);
    let mut out = vec![255u8; ow * oh * 4];
    let mut sums = [0f64; 4];
    let mut over = 0usize;
    let shown = |on: bool, v: f32| if on { v } else { 0.0 };
    for y in 0..oh {
        let sy = y * h / oh;
        for x in 0..ow {
            let sx = x * w / ow;
            let i = (sy * w + sx) * 4;
            let (r, g, b) = over_white(pixels.get(i..i + 4).unwrap_or_default());
            let inks = cmyk(r, g, b);
            for (sum, v) in sums.iter_mut().zip(inks) {
                *sum += f64::from(v);
            }
            let total: f32 = inks.iter().sum::<f32>() * 100.0;
            let o = (y * ow + x) * 4;
            let Some(px) = out.get_mut(o..o + 4) else {
                continue;
            };
            if let Some(limit) = limit
                && total > f32::from(limit.0)
            {
                over += 1;
                px.copy_from_slice(&[0, 200, 80, 255]);
                continue;
            }
            let [c, m, yl, k] = {
                let [c, m, yl, k] = inks;
                let [pc, pm, py, pk] = plates;
                [shown(pc, c), shown(pm, m), shown(py, yl), shown(pk, k)]
            };
            let paper = |ink: f32| ((1.0 - ink) * (1.0 - k) * 255.0).round() as u8;
            px.copy_from_slice(&[paper(c), paper(m), paper(yl), 255]);
        }
    }
    let n = (ow * oh) as f64;
    Some(Preview {
        image: image::Handle::from_rgba(ow as u32, oh as u32, out),
        coverage: sums.map(|s| (s / n) as f32),
        over: over as f32 / n as f32,
    })
}

/// The panel, for `page` (its image, if drawn) and the pointer over it: `hover` is where,
/// with the page's size as shown.
pub fn panel<'a>(
    state: &'a OutputPreview,
    page: usize,
    handle: Option<&image::Handle>,
    hover: Option<(Point, Size)>,
    spots: &'a [String],
) -> Element<'a, Message> {
    let ev = Message::Output;
    let preview = handle.and_then(|h| state.preview(h));
    let pointed = match (handle, hover) {
        (Some(h), Some((at, shown))) => inks_at(h, shown, at),
        _ => None,
    };
    let mut body = column![
        row![
            icons::glyph::<Message>(Icon::OutputPreview, true),
            text(format!("Output preview, page {}", page + 1)).size(14),
            space::horizontal(),
            icons::button(Icon::Close, "Close the output preview", Some(ev(Event::Close))),
        ]
        .spacing(6)
        .align_y(Center),
    ]
    .spacing(8)
    .width(WIDTH as f32 + 24.0);
    body = body.push(match &preview {
        Some(p) => Element::from(image(p.image.clone()).width(WIDTH as f32)),
        None => text("Drawing the page…").size(12).into(),
    });
    let mut plates = Column::new().spacing(2).push(
        row![
            text("Ink").size(12).style(text::secondary).width(96),
            text("Page").size(12).style(text::secondary).width(70),
            text(if pointed.is_some() { "Pointer" } else { "" })
                .size(12)
                .style(text::secondary),
        ]
        .spacing(4),
    );
    for (i, name) in INKS.iter().enumerate() {
        let on = state.plates.get(i).copied().unwrap_or(true);
        let coverage = preview
            .as_ref()
            .and_then(|p| p.coverage.get(i).copied())
            .map_or(String::new(), |c| format!("{:.1}%", c * 100.0));
        let here = pointed
            .and_then(|inks| inks.get(i).copied())
            .map_or(String::new(), |v| format!("{:.0}%", v * 100.0));
        plates = plates.push(
            row![
                checkbox(on)
                    .label(*name)
                    .text_size(13)
                    .on_toggle(move |v| ev(Event::Plate(i, v)))
                    .width(96),
                text(coverage).size(12).width(70),
                text(here).size(12),
            ]
            .spacing(4)
            .align_y(Center),
        );
    }
    body = body.push(plates);
    if let Some(inks) = pointed {
        body = body.push(
            text(format!(
                "Under the pointer: {:.0}% of the inks together",
                inks.iter().sum::<f32>() * 100.0
            ))
            .size(12),
        );
    }
    body = body.push(
        row![
            checkbox(state.warn)
                .label("Mark where the inks pass")
                .text_size(13)
                .on_toggle(move |v| ev(Event::Warn(v))),
            pick_list(Limit::ALL, Some(state.limit), move |l| ev(Event::Limit(l)))
                .text_size(13)
                .padding([2, 6]),
        ]
        .spacing(6)
        .align_y(Center),
    );
    if state.warn
        && let Some(p) = &preview
    {
        body = body.push(
            text(if p.over > 0.0 {
                format!(
                    "{:.1}% of the page passes {} (marked in green).",
                    p.over * 100.0,
                    state.limit
                )
            } else {
                format!("Nowhere on the page passes {}.", state.limit)
            })
            .size(12),
        );
    }
    if !spots.is_empty() {
        body = body.push(
            text(format!(
                "Spot inks in the document: {}. They're shown mixed from the four process inks.",
                spots.join(", ")
            ))
            .size(12),
        );
    }
    body = body.push(
        text(
            "Estimated from the colours on screen, without the printer's colour profile; \
             the press's own figures may differ.",
        )
        .size(11)
        .style(text::secondary),
    );
    body.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_split_into_process_inks() {
        assert_eq!(cmyk(255, 255, 255), [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(cmyk(0, 0, 0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(cmyk(255, 0, 0), [0.0, 1.0, 1.0, 0.0]);
        let [c, m, y, k] = cmyk(0, 128, 255);
        assert!((c - 1.0).abs() < 1e-6 && (m - 0.498).abs() < 0.01 && y == 0.0 && k == 0.0);
    }

    #[test]
    fn only_the_plates_shown_are_printed_and_coverage_counted() {
        // A red page: magenta and yellow.
        let page = image::Handle::from_rgba(4, 4, [255u8, 0, 0, 255].repeat(16));
        let all = make(&page, [true; 4], None).unwrap();
        assert_eq!(all.coverage, [0.0, 1.0, 1.0, 0.0]);
        let no_yellow = make(&page, [true, true, false, true], None).unwrap();
        let image::Handle::Rgba { pixels, .. } = no_yellow.image else {
            panic!()
        };
        // Magenta alone: no green, full red and blue.
        assert_eq!(pixels.get(..4).unwrap(), [255, 0, 255, 255]);
        let marked = make(&page, [true; 4], Some(Limit(180))).unwrap();
        assert!((marked.over - 1.0).abs() < 1e-6);
    }
}
