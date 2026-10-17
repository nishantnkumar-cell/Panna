//! Text selection across pages, and the maths that maps page points to the rotated, zoomed view.

use std::ops::Range;
use std::sync::Arc;

use iced::Point;
use panna_core::{PageSize, PageText, TextBox};

/// Extracted text for one page, fetched on demand from the render worker.
#[derive(Default)]
pub struct TextSlot {
    pub text: Option<Arc<PageText>>,
    pub pending: bool,
}

/// A gap between characters: before character `index` of `page`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Caret {
    pub page: usize,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Caret,
    pub focus: Caret,
}

impl Selection {
    pub fn at(caret: Caret) -> Self {
        Self {
            anchor: caret,
            focus: caret,
        }
    }

    fn ordered(&self) -> (Caret, Caret) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    /// The pages the selection touches.
    pub fn pages(&self) -> std::ops::RangeInclusive<usize> {
        let (start, end) = self.ordered();
        start.page..=end.page
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }

    /// The selected characters on `page`, if any. `len` is that page's character count.
    pub fn range_on(&self, page: usize, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ordered();
        if page < start.page || page > end.page {
            return None;
        }
        let from = if page == start.page { start.index } else { 0 };
        let to = if page == end.page { end.index } else { len };
        (from < to).then_some(from.min(len)..to.min(len))
    }

    /// The selected text, with pages joined by line breaks. Pages without extracted text are
    /// skipped, which can only happen for pages the pointer never touched.
    pub fn text(&self, texts: &[TextSlot]) -> String {
        let (start, end) = self.ordered();
        let mut out = String::new();
        for page in start.page..=end.page {
            let Some(text) = texts.get(page).and_then(|t| t.text.as_ref()) else {
                continue;
            };
            if let Some(range) = self.range_on(page, text.len()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&text.slice(range));
            }
        }
        out
    }
}

/// The word around character `index`: a run of letters and digits, or the single character.
pub fn word_at(text: &PageText, index: usize) -> Range<usize> {
    let chars: Vec<char> = text.slice(0..text.len()).chars().collect();
    let Some(c) = chars.get(index) else {
        return index..index;
    };
    if !c.is_alphanumeric() {
        return index..index + 1;
    }
    let start = chars
        .get(..index)
        .unwrap_or_default()
        .iter()
        .rposition(|c| !c.is_alphanumeric())
        .map_or(0, |i| i + 1);
    let end = chars
        .get(index..)
        .unwrap_or_default()
        .iter()
        .position(|c| !c.is_alphanumeric())
        .map_or(chars.len(), |i| index + i);
    start..end
}

/// Maps between a page's own points (y down, page `/Rotate` applied) and the on-screen page,
/// which is turned by the view rotation and scaled by the zoom.
#[derive(Debug, Clone, Copy)]
pub struct PageMap {
    /// Page size in points before the view rotation.
    pub size: PageSize,
    /// Clockwise quarter turns.
    pub rotation: u8,
    pub zoom: f32,
}

impl PageMap {
    pub fn to_view(self, x: f32, y: f32) -> Point {
        let (w, h) = (self.size.width, self.size.height);
        let (vx, vy) = match self.rotation % 4 {
            0 => (x, y),
            1 => (h - y, x),
            2 => (w - x, h - y),
            _ => (y, w - x),
        };
        Point::new(vx * self.zoom, vy * self.zoom)
    }

    pub fn to_page(self, p: Point) -> (f32, f32) {
        let zoom = if self.zoom > 0.0 { self.zoom } else { 1.0 };
        let (vx, vy) = (p.x / zoom, p.y / zoom);
        let (w, h) = (self.size.width, self.size.height);
        match self.rotation % 4 {
            0 => (vx, vy),
            1 => (vy, h - vx),
            2 => (w - vx, h - vy),
            _ => (w - vy, vx),
        }
    }

    /// A text box as an on-screen rectangle: (x, y, width, height).
    pub fn rect(self, b: &TextBox) -> (f32, f32, f32, f32) {
        let a = self.to_view(b.x0, b.y0);
        let c = self.to_view(b.x1, b.y1);
        let (x0, x1) = (a.x.min(c.x), a.x.max(c.x));
        let (y0, y1) = (a.y.min(c.y), a.y.max(c.y));
        (x0, y0, x1 - x0, y1 - y0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caret(page: usize, index: usize) -> Caret {
        Caret { page, index }
    }

    #[test]
    fn ranges_span_pages_in_either_direction() {
        let backwards = Selection {
            anchor: caret(2, 3),
            focus: caret(0, 5),
        };
        assert_eq!(backwards.range_on(0, 10), Some(5..10));
        assert_eq!(backwards.range_on(1, 7), Some(0..7));
        assert_eq!(backwards.range_on(2, 10), Some(0..3));
        assert_eq!(backwards.range_on(3, 10), None);
        assert!(Selection::at(caret(1, 1)).is_empty());
    }

    #[test]
    fn view_mapping_round_trips_for_every_rotation() {
        for rotation in 0..4 {
            let map = PageMap {
                size: PageSize {
                    width: 600.0,
                    height: 800.0,
                },
                rotation,
                zoom: 1.5,
            };
            let p = map.to_view(100.0, 50.0);
            let (x, y) = map.to_page(p);
            assert!((x - 100.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3, "{rotation}");
            let (rx, ry, rw, rh) = map.rect(&TextBox {
                x0: 0.0,
                y0: 0.0,
                x1: 600.0,
                y1: 800.0,
            });
            assert!(rx.abs() < 1e-3 && ry.abs() < 1e-3);
            let (ew, eh) = if rotation % 2 == 1 {
                (1200.0, 900.0)
            } else {
                (900.0, 1200.0)
            };
            assert!((rw - ew).abs() < 1e-3 && (rh - eh).abs() < 1e-3, "{rotation}");
        }
    }
}
