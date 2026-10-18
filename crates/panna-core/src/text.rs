//! Text extraction for search, selection and copy.
//!
//! The page is interpreted with a device that records every glyph's Unicode text and position
//! instead of drawing it. Spaces and line breaks are inferred from glyph positions, because
//! most PDFs position words individually rather than storing space characters.

use std::ops::Range;

use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps, InterpreterCache,
    InterpreterSettings, SoftMask, TransformExt, interpret_page,
};
use hayro::hayro_syntax::page::Page;
use hayro::kurbo::{Affine, BezPath, Point, Rect, Vec2};

/// Characters kept per page. Pages beyond this are searchable only up to the cap.
const MAX_CHARS: usize = 200_000;
/// Advance used when a font doesn't report one, in glyph units (1000 per em).
const FALLBACK_ADVANCE: f64 = 500.0;
/// Glyph box above and below the baseline, in glyph units. Fonts rarely report these
/// reliably, so typical Latin values are used for every font.
const ASCENT: f64 = 800.0;
const DESCENT: f64 = -200.0;

/// A character's box in page points: origin at the top left of the page as displayed at
/// rotation 0 (the page's own `/Rotate` already applied), y growing downwards.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextBox {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl TextBox {
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }

    fn center(&self) -> (f32, f32) {
        ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }

    pub(crate) fn union(self, other: TextBox) -> TextBox {
        TextBox {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }
}

/// The text of one page in content order, with one box per `char`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageText {
    chars: Vec<char>,
    boxes: Vec<TextBox>,
}

/// Options for [`PageText::find`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub match_case: bool,
    pub whole_word: bool,
}

impl PageText {
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub(crate) fn chars(&self) -> &[char] {
        &self.chars
    }

    pub(crate) fn boxes(&self) -> &[TextBox] {
        &self.boxes
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// The characters in `range` as a string. Out-of-range bounds are clamped.
    pub fn slice(&self, range: Range<usize>) -> String {
        let end = range.end.min(self.chars.len());
        let start = range.start.min(end);
        self.chars.get(start..end).unwrap_or_default().iter().collect()
    }

    /// The text under `areas` (a highlight's boxes, say), in reading order with whitespace
    /// runs collapsed to single spaces.
    pub fn text_in(&self, areas: &[TextBox]) -> String {
        let mut out = String::new();
        for (c, b) in self.chars.iter().zip(&self.boxes) {
            let (x, y) = b.center();
            if areas.iter().any(|a| a.contains(x, y)) {
                if c.is_whitespace() {
                    if !out.is_empty() && !out.ends_with(' ') {
                        out.push(' ');
                    }
                } else {
                    out.push(*c);
                }
            } else if !out.is_empty() && !out.ends_with(' ') {
                // A gap in the marked text, such as the end of a line.
                out.push(' ');
            }
        }
        out.trim_end().to_owned()
    }

    /// Every match of `query`, as character ranges. Any run of whitespace in the query matches
    /// one space or line break, so a phrase is found across a line wrap.
    pub fn find(&self, query: &str, options: FindOptions) -> Vec<Range<usize>> {
        let needle: Vec<char> = squash_whitespace(query);
        if needle.is_empty() || needle.iter().all(|c| *c == ' ') {
            return Vec::new();
        }
        let same = |a: char, b: char| {
            let (a, b) = (if a == '\n' { ' ' } else { a }, b);
            if options.match_case {
                a == b
            } else {
                a == b || a.to_lowercase().eq(b.to_lowercase())
            }
        };
        let is_word = |i: usize| self.chars.get(i).is_some_and(|c| c.is_alphanumeric());
        let mut out = Vec::new();
        let mut start = 0;
        while let Some(window) = self.chars.get(start..start + needle.len()) {
            let hit = window.iter().zip(&needle).all(|(&c, &n)| same(c, n));
            let end = start + needle.len();
            let bounded = !options.whole_word || ((start == 0 || !is_word(start - 1)) && !is_word(end));
            if hit && bounded {
                out.push(start..end);
                start = end;
            } else {
                start += 1;
            }
        }
        out
    }

    /// Highlight rectangles for `range`, one per line.
    pub fn rects(&self, range: Range<usize>) -> Vec<TextBox> {
        let end = range.end.min(self.boxes.len());
        let start = range.start.min(end);
        let mut out: Vec<TextBox> = Vec::new();
        let boxes = self.boxes.get(start..end).unwrap_or_default();
        let chars = self.chars.get(start..end).unwrap_or_default();
        for (b, c) in boxes.iter().zip(chars) {
            // Inferred spaces and line breaks have no ink; skip their empty boxes.
            if b.width() <= 0.0 || b.height() <= 0.0 || *c == '\n' {
                continue;
            }
            match out.last_mut() {
                Some(last) if same_line(last, b) => *last = last.union(*b),
                _ => out.push(*b),
            }
        }
        out
    }

    /// The character nearest to `(x, y)` in page points, for starting or extending a
    /// selection. Returns the index of the gap before or after it, whichever is closer.
    pub fn caret_at(&self, x: f32, y: f32) -> Option<usize> {
        let i = self.char_at(x, y)?;
        let (cx, _) = self.boxes.get(i)?.center();
        Some(if x > cx { i + 1 } else { i })
    }

    /// The character nearest to `(x, y)` in page points.
    pub fn char_at(&self, x: f32, y: f32) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for (i, b) in self.boxes.iter().enumerate() {
            if b.width() <= 0.0 || b.height() <= 0.0 {
                continue;
            }
            let dx = if x < b.x0 {
                b.x0 - x
            } else if x > b.x1 {
                x - b.x1
            } else {
                0.0
            };
            let dy = if y < b.y0 {
                b.y0 - y
            } else if y > b.y1 {
                y - b.y1
            } else {
                0.0
            };
            // Vertical distance counts more, so a point between lines picks the right line.
            let d = dx + dy * 4.0;
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, i));
            }
        }
        best.map(|(_, i)| i)
    }

    /// Whether `(x, y)` is on a character, so the pointer should show a text cursor.
    pub fn hit(&self, x: f32, y: f32) -> bool {
        self.boxes.iter().any(|b| b.contains(x, y))
    }
}

fn same_line(a: &TextBox, b: &TextBox) -> bool {
    let overlap = a.y1.min(b.y1) - a.y0.max(b.y0);
    overlap > 0.5 * a.height().min(b.height()) && b.x0 >= a.x0 - 1.0
}

fn squash_whitespace(s: &str) -> Vec<char> {
    let mut out: Vec<char> = Vec::new();
    for c in s.trim().chars() {
        if c.is_whitespace() {
            if out.last() != Some(&' ') {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// One glyph as the interpreter drew it, in display space (points, y down).
struct RawGlyph {
    text: Text,
    origin: Point,
    end: Point,
    /// Font height in display space.
    size: f64,
    bounds: Rect,
}

enum Text {
    Char(char),
    String(String),
    Unknown,
}

#[derive(Default)]
struct Collector {
    glyphs: Vec<RawGlyph>,
    chars: usize,
}

impl Collector {
    fn push(&mut self, glyph: &Glyph<'_>, transform: Affine) {
        if self.chars >= MAX_CHARS {
            return;
        }
        let advance = match glyph {
            Glyph::Outline(g) => g
                .advance_width()
                .map(f64::from)
                .filter(|a| a.is_finite() && *a > 0.0)
                .unwrap_or(FALLBACK_ADVANCE),
            Glyph::Type3(_) => FALLBACK_ADVANCE,
        };
        let origin = transform * Point::ZERO;
        let end = transform * Point::new(advance, 0.0);
        let size = (transform * Point::new(0.0, 1000.0) - origin).hypot();
        let corners = [
            Point::new(0.0, DESCENT),
            Point::new(advance, DESCENT),
            Point::new(advance, ASCENT),
            Point::new(0.0, ASCENT),
        ]
        .map(|p| transform * p);
        let finite = corners.iter().all(|p| p.x.is_finite() && p.y.is_finite());
        if !finite || !size.is_finite() || size <= 0.0 {
            return;
        }
        let mut bounds = Rect::from_points(corners[0], corners[1]);
        for p in &corners[2..] {
            bounds = bounds.union_pt(*p);
        }
        let text = match glyph.as_unicode() {
            Some(BfString::Char(c)) => Text::Char(c),
            Some(BfString::String(s)) if !s.is_empty() => Text::String(s),
            _ => Text::Unknown,
        };
        // Fake bold and fill-then-stroke draw the same glyph twice at (almost) the same spot.
        if let Some(last) = self.glyphs.last()
            && (last.origin - origin).hypot() < size * 0.1
            && last.text.same(&text)
        {
            return;
        }
        self.chars += 1;
        self.glyphs.push(RawGlyph {
            text,
            origin,
            end,
            size,
            bounds,
        });
    }

    fn finish(self) -> PageText {
        let mut out = PageText::default();
        let mut prev: Option<&RawGlyph> = None;
        for g in &self.glyphs {
            if out.chars.len() >= MAX_CHARS {
                break;
            }
            if let Some(p) = prev
                && let Some(sep) = separator(p, g)
            {
                let at = p.end;
                out.chars.push(sep);
                out.boxes.push(TextBox {
                    x0: at.x as f32,
                    y0: at.y as f32,
                    x1: at.x as f32,
                    y1: at.y as f32,
                });
            }
            let b = TextBox {
                x0: g.bounds.x0 as f32,
                y0: g.bounds.y0 as f32,
                x1: g.bounds.x1 as f32,
                y1: g.bounds.y1 as f32,
            };
            match &g.text {
                Text::Char(c) => push_char(&mut out, *c, b),
                Text::String(s) => {
                    // A ligature like "ﬁ" maps to several characters; share the box out.
                    let n = s.chars().count().max(1) as f32;
                    for (k, c) in s.chars().enumerate() {
                        let w = b.width() / n;
                        let x0 = b.x0 + w * k as f32;
                        push_char(&mut out, c, TextBox { x0, x1: x0 + w, ..b });
                    }
                }
                Text::Unknown => push_char(&mut out, '\u{FFFD}', b),
            }
            prev = Some(g);
        }
        while out.chars.last().is_some_and(|c| c.is_whitespace()) {
            out.chars.pop();
            out.boxes.pop();
        }
        out
    }
}

fn push_char(out: &mut PageText, c: char, b: TextBox) {
    // Control characters would corrupt copied text; keep tabs and line breaks as spaces.
    let c = match c {
        '\t' | '\r' | '\n' => ' ',
        c if c.is_control() => return,
        c => c,
    };
    // Avoid doubled spaces when the PDF has real spaces as well as gaps.
    if c == ' ' && matches!(out.chars.last(), Some(' ' | '\n') | None) {
        return;
    }
    out.chars.push(c);
    out.boxes.push(b);
}

impl Text {
    fn same(&self, other: &Text) -> bool {
        match (self, other) {
            (Text::Char(a), Text::Char(b)) => a == b,
            (Text::String(a), Text::String(b)) => a == b,
            _ => false,
        }
    }

    fn is_space(&self) -> bool {
        match self {
            Text::Char(c) => c.is_whitespace(),
            Text::String(s) => s.chars().all(char::is_whitespace),
            Text::Unknown => false,
        }
    }
}

/// The break to insert between two consecutive glyphs, if any.
fn separator(prev: &RawGlyph, cur: &RawGlyph) -> Option<char> {
    let mut dir = prev.end - prev.origin;
    if dir.hypot() < 1e-6 {
        dir = Vec2::new(1.0, 0.0);
    }
    let dir = dir.normalize();
    let gap = cur.origin - prev.end;
    let along = gap.dot(dir);
    let across = dir.cross(gap).abs();
    let size = prev.size.max(cur.size);
    if across > size * 0.5 || along < -size * 3.0 {
        return Some('\n');
    }
    if along > size * 0.15 && !prev.text.is_space() && !cur.text.is_space() {
        return Some(' ');
    }
    None
}

/// Extracts the text of `page`. Callers catch panics; see [`crate::Document::page_text`].
pub(crate) fn extract(page: &Page<'_>, settings: &InterpreterSettings) -> PageText {
    let cache = InterpreterCache::new();
    let (w, h) = page.render_dimensions();
    let transform = page.initial_transform(true).to_kurbo();
    let mut context = Context::new(
        transform,
        Rect::new(0.0, 0.0, f64::from(w), f64::from(h)),
        &cache,
        page.xref(),
        settings.clone(),
    );
    let mut device = Collector::default();
    interpret_page(page, &mut context, &mut device);
    device.finish()
}

impl<'a> Device<'a> for Collector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        for glyph in run.glyphs() {
            self.push(glyph, props.transform * glyph.transform());
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(s: &str) -> PageText {
        let mut p = PageText::default();
        for (i, c) in s.chars().enumerate() {
            let x = i as f32 * 10.0;
            p.chars.push(c);
            p.boxes.push(TextBox {
                x0: x,
                y0: 0.0,
                x1: x + 10.0,
                y1: 12.0,
            });
        }
        p
    }

    #[test]
    fn find_respects_case_and_words() {
        let p = page("Cat concat CAT\ncat");
        let any = FindOptions::default();
        assert_eq!(p.find("cat", any).len(), 4);
        let case = FindOptions {
            match_case: true,
            ..any
        };
        assert_eq!(p.find("cat", case), vec![7..10, 15..18]);
        let word = FindOptions {
            whole_word: true,
            ..any
        };
        assert_eq!(p.find("cat", word), vec![0..3, 11..14, 15..18]);
        // Whitespace in the query matches a line break.
        assert_eq!(p.find("CAT  cat", any), vec![7..14]);
        assert_eq!(p.find("CAT  cat", word), vec![11..18]);
        assert!(p.find("   ", any).is_empty());
    }

    #[test]
    fn rects_merge_per_line_and_slice_clamps() {
        let p = page("ab cd");
        assert_eq!(p.rects(0..5).len(), 1);
        assert_eq!(p.slice(3..99), "cd");
        assert_eq!(p.slice(9..12), "");
    }

    #[test]
    fn caret_picks_nearest_gap() {
        let p = page("abc");
        assert_eq!(p.caret_at(1.0, 5.0), Some(0));
        assert_eq!(p.caret_at(9.0, 5.0), Some(1));
        assert_eq!(p.caret_at(500.0, 5.0), Some(3));
        assert_eq!(PageText::default().caret_at(0.0, 0.0), None);
    }
}
