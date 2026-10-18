//! New annotations and their appearance streams.
//!
//! Every annotation gets an appearance stream (`/AP`), so it looks the same in every viewer,
//! including ours (the renderer draws annotations only from their appearance streams).

use std::fmt::Write as _;

use hayro::kurbo::{Affine, Point};

use crate::TextBox;

/// An RGB colour, each channel 0–1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color(pub [f32; 3]);

impl Color {
    pub const YELLOW: Color = Color([1.0, 0.85, 0.1]);
    pub const RED: Color = Color([0.86, 0.15, 0.15]);
    pub const BLACK: Color = Color([0.0, 0.0, 0.0]);
}

/// What an annotation looks like. Coordinates are in page points as displayed: origin at the
/// top left, y down, the page's own `/Rotate` applied (the same space as [`TextBox`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Highlight(Vec<TextBox>),
    Underline(Vec<TextBox>),
    StrikeOut(Vec<TextBox>),
    /// A sticky note, with its icon's top-left corner here.
    Note {
        x: f32,
        y: f32,
    },
    /// Text drawn directly on the page.
    FreeText {
        rect: TextBox,
        size: f32,
    },
    /// Freehand strokes.
    Ink(Vec<Vec<(f32, f32)>>),
    Rect(TextBox),
    Ellipse(TextBox),
    Line {
        from: (f32, f32),
        to: (f32, f32),
        arrow: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub page: usize,
    pub kind: Kind,
    pub color: Color,
    /// 0–1.
    pub opacity: f32,
    /// Stroke width in points, for ink, shapes and lines.
    pub width: f32,
    /// The comment text (for notes and text boxes, the text itself).
    pub contents: String,
    pub author: String,
}

/// Size of a sticky note's icon, in points.
pub const NOTE_SIZE: f32 = 20.0;

/// Line height of free text, as a multiple of the font size.
pub const LINE_HEIGHT: f32 = 1.2;

/// The two objects that make up one annotation.
pub(crate) struct Built {
    pub dict: Vec<u8>,
    pub appearance: Vec<u8>,
}

/// Everything about the target page that an annotation needs.
pub(crate) struct PageInfo {
    /// Maps displayed page points to PDF user space.
    pub to_user: Affine,
    /// The page object's reference, e.g. `12 0 R`.
    pub page_ref: String,
}

impl Annotation {
    /// Shifts it by `dx`, `dy` page points.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        let shift = |b: &mut TextBox| {
            b.x0 += dx;
            b.x1 += dx;
            b.y0 += dy;
            b.y1 += dy;
        };
        match &mut self.kind {
            Kind::Highlight(boxes) | Kind::Underline(boxes) | Kind::StrikeOut(boxes) => {
                boxes.iter_mut().for_each(shift);
            }
            Kind::Note { x, y } => {
                *x += dx;
                *y += dy;
            }
            Kind::FreeText { rect, .. } | Kind::Rect(rect) | Kind::Ellipse(rect) => shift(rect),
            Kind::Ink(strokes) => {
                for (x, y) in strokes.iter_mut().flatten() {
                    *x += dx;
                    *y += dy;
                }
            }
            Kind::Line { from, to, .. } => {
                for p in [from, to] {
                    p.0 += dx;
                    p.1 += dy;
                }
            }
        }
    }

    pub(crate) fn build(&self, page: &PageInfo, ap_ref: &str, name: &str, date: &str) -> Built {
        let map = |x: f32, y: f32| page.to_user * Point::new(f64::from(x), f64::from(y));
        let user_box = |b: &TextBox| {
            let a = map(b.x0, b.y0);
            let c = map(b.x1, b.y1);
            [a.x.min(c.x), a.y.min(c.y), a.x.max(c.x), a.y.max(c.y)]
        };
        let [r, g, b] = self.color.0.map(|c| c.clamp(0.0, 1.0));
        let opacity = self.opacity.clamp(0.05, 1.0);
        let width = f64::from(self.width.clamp(0.25, 50.0));
        let color = format!("{} {} {}", num(r), num(g), num(b));

        let mut extra = String::new();
        let mut content = String::new();
        let mut resources = String::new();
        let mut blend = "/Normal";
        let mut bounds = Bounds::default();

        let subtype = match &self.kind {
            Kind::Highlight(boxes) | Kind::Underline(boxes) | Kind::StrikeOut(boxes) => {
                let quads: Vec<[f64; 4]> = boxes.iter().map(user_box).collect();
                let _ = write!(extra, "/QuadPoints [");
                for [x0, y0, x1, y1] in &quads {
                    // Upper left, upper right, lower left, lower right.
                    let _ = write!(
                        extra,
                        "{} {} {} {} {} {} {} {} ",
                        n(*x0),
                        n(*y1),
                        n(*x1),
                        n(*y1),
                        n(*x0),
                        n(*y0),
                        n(*x1),
                        n(*y0)
                    );
                    bounds.add(*x0, *y0);
                    bounds.add(*x1, *y1);
                }
                extra.push_str("] ");
                match &self.kind {
                    Kind::Highlight(_) => {
                        blend = "/Multiply";
                        let _ = write!(content, "{color} rg ");
                        for [x0, y0, x1, y1] in &quads {
                            let _ = write!(
                                content,
                                "{} {} {} {} re f ",
                                n(*x0),
                                n(*y0),
                                n(x1 - x0),
                                n(y1 - y0)
                            );
                        }
                        "Highlight"
                    }
                    kind => {
                        let _ = write!(content, "{color} RG ");
                        for [x0, y0, x1, y1] in &quads {
                            let h = y1 - y0;
                            let (y, w) = if matches!(kind, Kind::Underline(_)) {
                                (y0 + h * 0.08, (h * 0.07).max(0.5))
                            } else {
                                (y0 + h * 0.42, (h * 0.07).max(0.5))
                            };
                            let _ = write!(
                                content,
                                "{} w {} {} m {} {} l S ",
                                n(w),
                                n(*x0),
                                n(y),
                                n(*x1),
                                n(y)
                            );
                        }
                        if matches!(kind, Kind::Underline(_)) {
                            "Underline"
                        } else {
                            "StrikeOut"
                        }
                    }
                }
            }
            Kind::Note { x, y } => {
                let [x0, y0, x1, y1] = user_box(&TextBox {
                    x0: *x,
                    y0: *y,
                    x1: x + NOTE_SIZE,
                    y1: y + NOTE_SIZE,
                });
                bounds.add(x0, y0);
                bounds.add(x1, y1);
                // A note card with three lines of "text".
                let (w, h) = (x1 - x0, y1 - y0);
                let _ = write!(
                    content,
                    "{color} rg 0.25 0.25 0.25 RG 0.8 w {} {} {} {} re B 0.3 0.3 0.3 RG 1.2 w ",
                    n(x0 + 1.0),
                    n(y0 + 1.0),
                    n(w - 2.0),
                    n(h - 2.0)
                );
                for k in 1..=3 {
                    let ly = y1 - h * f64::from(k) / 4.2 - 1.0;
                    let _ = write!(
                        content,
                        "{} {} m {} {} l S ",
                        n(x0 + 4.5),
                        n(ly),
                        n(x1 - 4.5),
                        n(ly)
                    );
                }
                extra.push_str("/Name /Comment /Open false ");
                "Text"
            }
            Kind::FreeText { rect, size } => {
                let [x0, y0, x1, y1] = user_box(rect);
                bounds.add(x0, y0);
                bounds.add(x1, y1);
                let size = f64::from(size.clamp(4.0, 144.0));
                let lines = wrap(&self.contents, ((x1 - x0) - 4.0).max(10.0) / size);
                resources.push_str(
                    "/Font << /Helv << /Type /Font /Subtype /Type1 /BaseFont /Helvetica \
                     /Encoding /WinAnsiEncoding >> >> ",
                );
                let _ = write!(content, "/Tx BMC q BT /Helv {} Tf {color} rg ", n(size));
                let y = y1 - size;
                let leading = size * f64::from(LINE_HEIGHT);
                let _ = write!(content, "{} {} Td ", n(x0 + 2.0), n(y));
                for (i, line) in lines.iter().enumerate() {
                    if i > 0 {
                        let _ = write!(content, "0 {} Td ", n(-leading));
                    }
                    content.push('(');
                    content.push_str(&literal(&win_ansi(line)));
                    content.push_str(") Tj ");
                }
                content.push_str("ET Q EMC ");
                let _ = write!(
                    extra,
                    "/DA ({}) ",
                    format_args!("/Helv {} Tf {color} rg", n(size))
                );
                "FreeText"
            }
            Kind::Ink(strokes) => {
                let _ = write!(content, "{color} RG {} w 1 J 1 j ", n(width));
                extra.push_str("/InkList [");
                for stroke in strokes {
                    extra.push('[');
                    for (i, (x, y)) in stroke.iter().enumerate() {
                        let p = map(*x, *y);
                        bounds.add(p.x, p.y);
                        let _ = write!(extra, "{} {} ", n(p.x), n(p.y));
                        let op = if i == 0 { "m" } else { "l" };
                        let _ = write!(content, "{} {} {op} ", n(p.x), n(p.y));
                    }
                    if stroke.len() == 1 {
                        // A dot: a zero-length line with round caps.
                        if let Some((x, y)) = stroke.first() {
                            let p = map(*x, *y);
                            let _ = write!(content, "{} {} l ", n(p.x), n(p.y));
                        }
                    }
                    content.push_str("S ");
                    extra.push_str("] ");
                }
                extra.push_str("] ");
                bounds.grow(width);
                "Ink"
            }
            Kind::Rect(rect) | Kind::Ellipse(rect) => {
                let [x0, y0, x1, y1] = user_box(rect);
                bounds.add(x0, y0);
                bounds.add(x1, y1);
                let inset = width / 2.0;
                let (ix0, iy0, ix1, iy1) = (x0 + inset, y0 + inset, x1 - inset, y1 - inset);
                let _ = write!(content, "{color} RG {} w ", n(width));
                if matches!(self.kind, Kind::Rect(_)) {
                    let _ = write!(
                        content,
                        "{} {} {} {} re S ",
                        n(ix0),
                        n(iy0),
                        n(ix1 - ix0),
                        n(iy1 - iy0)
                    );
                    "Square"
                } else {
                    ellipse(&mut content, ix0, iy0, ix1, iy1);
                    content.push_str("S ");
                    "Circle"
                }
            }
            Kind::Line { from, to, arrow } => {
                let a = map(from.0, from.1);
                let c = map(to.0, to.1);
                let _ = write!(extra, "/L [{} {} {} {}] ", n(a.x), n(a.y), n(c.x), n(c.y));
                let _ = write!(
                    content,
                    "{color} RG {} w 1 J 1 j {} {} m {} {} l S ",
                    n(width),
                    n(a.x),
                    n(a.y),
                    n(c.x),
                    n(c.y)
                );
                bounds.add(a.x, a.y);
                bounds.add(c.x, c.y);
                if *arrow {
                    extra.push_str("/LE [/None /OpenArrow] ");
                    let d = c - a;
                    let len = d.hypot();
                    if len > 1e-6 {
                        let u = d / len;
                        let head = width * 3.0 + 6.0;
                        for angle in [0.5_f64, -0.5] {
                            let (s, co) = angle.sin_cos();
                            let back = Point::new(
                                c.x - head * (u.x * co - u.y * s),
                                c.y - head * (u.x * s + u.y * co),
                            );
                            bounds.add(back.x, back.y);
                            let _ = write!(
                                content,
                                "{} {} m {} {} l S ",
                                n(c.x),
                                n(c.y),
                                n(back.x),
                                n(back.y)
                            );
                        }
                    }
                }
                bounds.grow(width);
                "Line"
            }
        };
        if !matches!(self.kind, Kind::Ink(_) | Kind::Line { .. }) {
            bounds.grow(1.0);
        }
        let [x0, y0, x1, y1] = bounds.rect();
        let rect = format!("[{} {} {} {}]", n(x0), n(y0), n(x1), n(y1));

        let mut dict = format!(
            "<< /Type /Annot /Subtype /{subtype} /Rect {rect} /P {} /NM {} /M ({date}) /F 4 \
             /C [{color}] /CA {} /AP << /N {ap_ref} >> ",
            page.page_ref,
            text_string(name),
            num(opacity),
        );
        if matches!(
            self.kind,
            Kind::Ink(_) | Kind::Rect(_) | Kind::Ellipse(_) | Kind::Line { .. }
        ) {
            let _ = write!(dict, "/BS << /W {} >> ", n(width));
        }
        if !self.contents.is_empty() {
            let _ = write!(dict, "/Contents {} ", text_string(&self.contents));
        }
        if !self.author.is_empty() {
            let _ = write!(dict, "/T {} ", text_string(&self.author));
        }
        dict.push_str(&extra);
        dict.push_str(">>");

        let stream_body = content.trim_end().to_owned();
        let appearance = format!(
            "<< /Type /XObject /Subtype /Form /BBox {rect} /Resources << {resources}/ExtGState << /GS0 << \
             /CA {o} /ca {o} /BM {blend} >> >> >> /Length {} >>\nstream\n/GS0 gs {stream_body}\nendstream",
            stream_body.len() + "/GS0 gs ".len(),
            o = num(opacity),
        );
        Built {
            dict: dict.into_bytes(),
            appearance: appearance.into_bytes(),
        }
    }
}

#[derive(Default)]
struct Bounds(Option<[f64; 4]>);

impl Bounds {
    fn add(&mut self, x: f64, y: f64) {
        let r = self.0.get_or_insert([x, y, x, y]);
        *r = [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)];
    }

    fn grow(&mut self, by: f64) {
        if let Some(r) = &mut self.0 {
            *r = [r[0] - by, r[1] - by, r[2] + by, r[3] + by];
        }
    }

    fn rect(&self) -> [f64; 4] {
        self.0.unwrap_or([0.0, 0.0, 1.0, 1.0])
    }
}

fn ellipse(out: &mut String, x0: f64, y0: f64, x1: f64, y1: f64) {
    const K: f64 = 0.552_284_75;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (rx, ry) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let (kx, ky) = (rx * K, ry * K);
    let _ = write!(out, "{} {} m ", n(cx + rx), n(cy));
    let curves = [
        [cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry],
        [cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy],
        [cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry],
        [cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy],
    ];
    for c in curves {
        let _ = write!(
            out,
            "{} {} {} {} {} {} c ",
            n(c[0]),
            n(c[1]),
            n(c[2]),
            n(c[3]),
            n(c[4]),
            n(c[5])
        );
    }
}

/// A number as PDF syntax, with at most three decimals.
pub(crate) fn n(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn num(v: f32) -> String {
    n(f64::from(v))
}

/// A text string in UTF-16BE with a byte order mark, as hex, which needs no escaping.
pub(crate) fn text_string(s: &str) -> String {
    let mut out = String::from("<FEFF");
    for unit in s.encode_utf16() {
        let _ = write!(out, "{unit:04X}");
    }
    out.push('>');
    out
}

/// Escapes bytes for a literal string.
pub(crate) fn literal(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        match b {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(b as char);
            }
            0x20..=0x7e => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out
}

/// Encodes text in WinAnsiEncoding, the encoding of the standard Helvetica font. Characters it
/// can't represent become `?`.
pub(crate) fn win_ansi(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c {
            ' '..='~' => c as u8,
            '\u{a0}'..='\u{ff}' => c as u32 as u8,
            '€' => 0x80,
            '…' => 0x85,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            _ => b'?',
        })
        .collect()
}

/// Helvetica advance widths for printable ASCII, in thousandths of the font size.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556,
    556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278,
    500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469,
    556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500,
    278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Width of `s` in Helvetica, in units of the font size.
pub fn text_width(s: &str) -> f64 {
    s.chars()
        .map(|c| {
            let i = (c as u32).wrapping_sub(32) as usize;
            f64::from(HELVETICA.get(i).copied().unwrap_or(556)) / 1000.0
        })
        .sum()
}

/// Wraps text into lines no wider than `max` (in units of the font size), keeping the
/// author's own line breaks. Words longer than a line are split.
pub fn wrap(text: &str, max: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if text_width(&candidate) <= max || line.is_empty() && text_width(word) <= max {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // Split an over-long word across lines.
            for c in word.chars() {
                let mut next = line.clone();
                next.push(c);
                if text_width(&next) > max && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                    line.push(c);
                } else {
                    line = next;
                }
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translating_moves_every_point() {
        let mut a = Annotation {
            page: 0,
            kind: Kind::Line {
                from: (1.0, 2.0),
                to: (3.0, 4.0),
                arrow: false,
            },
            color: Color::RED,
            opacity: 1.0,
            width: 1.0,
            contents: String::new(),
            author: String::new(),
        };
        a.translate(10.0, -1.0);
        assert!(matches!(
            a.kind,
            Kind::Line {
                from: (11.0, 1.0),
                to: (13.0, 3.0),
                ..
            }
        ));
    }

    #[test]
    fn numbers_are_compact() {
        assert_eq!(n(1.0), "1");
        assert_eq!(n(-0.0001), "0");
        assert_eq!(n(2.5), "2.5");
        assert_eq!(n(f64::NAN), "0");
        assert_eq!(n(1.23456), "1.235");
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(text_string("Hi€"), "<FEFF0048006920AC>");
        assert_eq!(literal(b"a(b)\\\n"), "a\\(b\\)\\\\\\012");
        assert_eq!(win_ansi("é€漢"), vec![0xe9, 0x80, b'?']);
    }

    #[test]
    fn wrapping_respects_width_and_breaks() {
        let lines = wrap("one two three\nfour", text_width("one two") + 0.01);
        assert_eq!(lines, vec!["one two", "three", "four"]);
        let long = wrap("abcdefghij", text_width("abcd") + 0.01);
        assert!(long.len() >= 2 && long.iter().all(|l| text_width(l) <= text_width("abcd") + 0.01));
        assert_eq!(wrap("", 10.0), vec![String::new()]);
    }
}
