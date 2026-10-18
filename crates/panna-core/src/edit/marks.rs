//! Headers and footers (lines of text at the top and bottom of pages, such as page numbers, a
//! title, a date or Bates numbers), watermarks and page backgrounds.
//!
//! They become part of the page itself, each in a form of its own that the page draws before
//! or after its content, marked as artifacts so that screen readers skip them.

use std::fmt::Write as _;

use super::annot::{Color, literal, n, text_width, win_ansi};

/// Text for the top and bottom of pages. In the text, `{page}` is the page's number,
/// `{page:6}` the same padded with zeros to six digits (for Bates numbers), `{pages}` the
/// number of pages marked and `{date}` today's date.
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderFooter {
    /// The header's left, centre and right text, then the footer's.
    pub text: [String; 6],
    /// Text size, in points.
    pub size: f32,
    /// Distance from the edges of the page as shown to the text, in points.
    pub margin: f32,
    /// The number the first marked page gets.
    pub first: u32,
}

impl Default for HeaderFooter {
    fn default() -> Self {
        Self {
            text: Default::default(),
            size: 10.0,
            margin: 36.0,
            first: 1,
        }
    }
}

impl HeaderFooter {
    /// Whether there is nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.text.iter().all(|t| t.trim().is_empty())
    }

    /// The text for one page, numbered `number` of `total`, on `date`.
    pub(crate) fn for_page(&self, number: u64, total: usize, date: &str) -> Lines {
        Lines {
            text: self.text.clone().map(|t| fill(&t, number, total, date)),
            size: f64::from(self.size.clamp(4.0, 72.0)),
            margin: f64::from(self.margin.max(0.0)),
        }
    }
}

/// Replaces the fields in `template`. Unknown fields stay as they are.
pub fn fill(template: &str, number: u64, total: usize, date: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(rest.get(..open).unwrap_or_default());
        let after = rest.get(open..).unwrap_or_default();
        let Some(close) = after.find('}') else {
            out.push_str(after);
            return out;
        };
        let field = after.get(1..close).unwrap_or_default();
        match field.split_once(':') {
            _ if field == "page" => {
                let _ = write!(out, "{number}");
            }
            _ if field == "pages" => {
                let _ = write!(out, "{total}");
            }
            _ if field == "date" => out.push_str(date),
            Some(("page", digits)) if digits.parse::<usize>().is_ok_and(|d| d <= 20) => {
                let width = digits.parse::<usize>().unwrap_or(0);
                let _ = write!(out, "{number:0width$}");
            }
            _ => out.push_str(after.get(..=close).unwrap_or_default()),
        }
        rest = after.get(close + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// Header and footer text ready to draw on one page.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Lines {
    text: [String; 6],
    size: f64,
    margin: f64,
}

/// Text across the middle of pages, such as DRAFT or CONFIDENTIAL, and a colour for the whole
/// page behind its content. The text may use the fields headers and footers do.
#[derive(Debug, Clone, PartialEq)]
pub struct Watermark {
    pub text: String,
    /// Text size in points; 0 or less fits the text to the page.
    pub size: f32,
    /// Degrees anticlockwise from level.
    pub angle: f32,
    /// From 0 (invisible) to 1 (solid).
    pub opacity: f32,
    pub color: Color,
    /// Drawn behind the page's content instead of over it.
    pub behind: bool,
    /// A colour filling the page behind its content.
    pub background: Option<Color>,
}

impl Default for Watermark {
    fn default() -> Self {
        Self {
            text: String::new(),
            size: 0.0,
            angle: 45.0,
            opacity: 0.3,
            color: Color([0.5, 0.5, 0.5]),
            behind: false,
            background: None,
        }
    }
}

impl Watermark {
    /// Whether there is nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.background.is_none()
    }

    /// What to draw on one page, numbered `number` of `total`, on `date`: behind the page's
    /// content, and over it.
    pub(crate) fn for_page(
        &self,
        number: u64,
        total: usize,
        date: &str,
    ) -> (Option<Drawing>, Option<Drawing>) {
        let text = Drawing::Watermark {
            text: fill(&self.text, number, total, date),
            size: f64::from(self.size.min(500.0)),
            angle: f64::from(self.angle),
            opacity: f64::from(self.opacity.clamp(0.0, 1.0)),
            color: self.color,
        };
        let text = (!self.text.trim().is_empty()).then_some(text);
        let background = self.background.map(Drawing::Background);
        if self.behind {
            // The colour first, so the text shows on it.
            let both = match (background, text) {
                (Some(b), Some(t)) => Some(Drawing::Both(Box::new(b), Box::new(t))),
                (b, t) => b.or(t),
            };
            (both, None)
        } else {
            (background, text)
        }
    }
}

/// Something drawn on a page in a space with its origin at the bottom-left corner of the page
/// as shown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Drawing {
    Lines(Lines),
    Watermark {
        text: String,
        size: f64,
        angle: f64,
        opacity: f64,
        color: Color,
    },
    Background(Color),
    /// Lines of text added to the page, the first line's top-left corner at `x`, `top`
    /// (from the top of the page as shown).
    Text {
        x: f64,
        top: f64,
        lines: Vec<String>,
        size: f64,
        color: Color,
    },
    /// A picture added to the page, filling `rect` (left, top, right, bottom from the
    /// top-left corner of the page as shown).
    Picture {
        rect: [f64; 4],
        image: super::picture::StampImage,
    },
    /// One, then the other.
    Both(Box<Drawing>, Box<Drawing>),
}

/// The fonts and transparency the drawings use, as they're named in the forms' resources.
pub(crate) const RESOURCES: &str = "<< /Font << \
    /PaHe << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> \
    /PaHB << /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >> >> >>";

fn rgb(Color([r, g, b]): Color) -> String {
    format!("{} {} {}", n(f64::from(r)), n(f64::from(g)), n(f64::from(b)))
}

impl Drawing {
    /// The form's resources: the fonts, and the transparency a watermark needs.
    pub(crate) fn resources(&self) -> String {
        match self.opacity() {
            Some(a) => format!(
                "{} /ExtGState << /PaGS << /Type /ExtGState /CA {a} /ca {a} >> >> >>",
                RESOURCES.trim_end_matches(">>").trim_end(),
                a = n(a)
            ),
            None => RESOURCES.to_owned(),
        }
    }

    /// The picture it draws, which needs objects of its own.
    pub(crate) fn picture(&self) -> Option<&super::picture::StampImage> {
        match self {
            Drawing::Picture { image, .. } => Some(image),
            _ => None,
        }
    }

    fn opacity(&self) -> Option<f64> {
        match self {
            Drawing::Watermark { opacity, .. } => Some(*opacity),
            Drawing::Both(a, b) => a.opacity().or(b.opacity()),
            _ => None,
        }
    }

    /// Drawing instructions for a page `width` by `height` points as shown.
    pub(crate) fn content(&self, width: f64, height: f64) -> String {
        match self {
            Drawing::Lines(lines) => lines.content(width, height),
            Drawing::Background(color) => format!(
                "/Artifact << /Type /Pagination /Subtype /Background >> BDC {} rg 0 0 {} {} re f EMC\n",
                rgb(*color),
                n(width),
                n(height)
            ),
            Drawing::Watermark {
                text,
                size,
                angle,
                opacity: _,
                color,
            } => {
                // Helvetica-Bold runs about 6% wider than the Helvetica widths we have.
                let em = text_width(text).max(0.5) * 1.06;
                let (sin, cos) = angle.to_radians().sin_cos();
                let size = if *size > 0.0 {
                    *size
                } else {
                    // As large as fits within 90% of the page, turned.
                    let along = 0.9 * width / (em * cos.abs() + 0.72 * sin.abs());
                    let across = 0.9 * height / (em * sin.abs() + 0.72 * cos.abs());
                    along.min(across).max(1.0)
                };
                let mut out = format!(
                    "/Artifact << /Type /Pagination /Subtype /Watermark >> BDC q /PaGS gs {} {} {} {} {} {} cm BT /PaHB {} Tf {} rg {} {} Td (",
                    n(cos),
                    n(sin),
                    n(-sin),
                    n(cos),
                    n(width / 2.0),
                    n(height / 2.0),
                    n(size),
                    rgb(*color),
                    n(-em * size / 2.0),
                    // Centred on capitals, about 0.72 of the size tall.
                    n(-0.36 * size),
                );
                out.push_str(&literal(&win_ansi(text)));
                out.push_str(") Tj ET Q EMC\n");
                out
            }
            Drawing::Text {
                x,
                top,
                lines,
                size,
                color,
            } => {
                let mut out = format!(
                    "BT /PaHe {} Tf {} rg {} TL ",
                    n(*size),
                    rgb(*color),
                    n(size * f64::from(super::annot::LINE_HEIGHT))
                );
                // The first baseline sits an ascent below the top.
                let _ = write!(out, "{} {} Td", n(*x), n(height - top - size * 0.77));
                for (i, line) in lines.iter().enumerate() {
                    out.push_str(if i == 0 { " (" } else { " T* (" });
                    out.push_str(&literal(&win_ansi(line)));
                    out.push_str(") Tj");
                }
                out.push_str(" ET\n");
                out
            }
            Drawing::Picture {
                rect: [x0, top, x1, bottom],
                ..
            } => format!(
                "q {} 0 0 {} {} {} cm /PaIm Do Q\n",
                n(x1 - x0),
                n(bottom - top),
                n(*x0),
                n(height - bottom)
            ),
            Drawing::Both(a, b) => a.content(width, height) + &b.content(width, height),
        }
    }
}

impl Lines {
    /// Drawing instructions for a page `width` by `height` points as shown, with the origin
    /// at its bottom-left corner as shown.
    pub(crate) fn content(&self, width: f64, height: f64) -> String {
        let (size, margin) = (self.size, self.margin);
        // Capitals reach about 0.72 of the size above the baseline; descenders about 0.21
        // below it.
        let top = height - margin - size * 0.72;
        let bottom = margin + size * 0.21;
        let mut out = String::new();
        for (i, text) in self.text.iter().enumerate() {
            if text.trim().is_empty() {
                continue;
            }
            let (subtype, y) = if i < 3 {
                ("Header", top)
            } else {
                ("Footer", bottom)
            };
            let w = text_width(text) * size;
            let x = match i % 3 {
                0 => margin,
                1 => (width - w) / 2.0,
                _ => width - margin - w,
            };
            let _ = write!(
                out,
                "/Artifact << /Type /Pagination /Subtype /{subtype} >> BDC BT /PaHe {} Tf 0 g {} {} Td (",
                n(size),
                n(x),
                n(y)
            );
            out.push_str(&literal(&win_ansi(text)));
            out.push_str(") Tj ET EMC\n");
        }
        out
    }
}

/// Today's date as year-month-day, in UTC.
pub fn today() -> String {
    let d = super::pdf_date(std::time::SystemTime::now());
    match (d.get(2..6), d.get(6..8), d.get(8..10)) {
        (Some(y), Some(m), Some(day)) => format!("{y}-{m}-{day}"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_filled() {
        assert_eq!(fill("Page {page} of {pages}", 3, 12, ""), "Page 3 of 12");
        assert_eq!(fill("ABC{page:6}", 42, 1, ""), "ABC000042");
        assert_eq!(fill("{date}", 1, 1, "2026-10-18"), "2026-10-18");
        assert_eq!(fill("{other} {page", 1, 1, ""), "{other} {page");
        assert_eq!(fill("{page:x}", 1, 1, ""), "{page:x}");
    }

    #[test]
    fn watermarks_fit_and_sit_behind_or_over() {
        let mark = Watermark {
            text: "DRAFT".into(),
            background: Some(Color([1.0, 1.0, 0.9])),
            ..Watermark::default()
        };
        let (under, over) = mark.for_page(1, 1, "");
        assert!(matches!(under, Some(Drawing::Background(_))));
        let over = over.unwrap();
        assert!(over.resources().contains("/ca 0.3"));
        let content = over.content(600.0, 800.0);
        assert!(
            content.contains("0.707 0.707 -0.707 0.707 300 400 cm"),
            "{content}"
        );
        assert!(content.contains("(DRAFT) Tj"));
        let (under, over) = Watermark { behind: true, ..mark }.for_page(1, 1, "");
        assert!(matches!(under, Some(Drawing::Both(..))) && over.is_none());
    }

    #[test]
    fn text_sits_inside_the_margins() {
        let mut text: [String; 6] = Default::default();
        text[0] = "Left".into();
        text[5] = "Right".into();
        let lines = HeaderFooter {
            text,
            ..HeaderFooter::default()
        }
        .for_page(1, 1, "");
        let content = lines.content(600.0, 800.0);
        assert!(content.contains("36 756.8 Td (Left)"), "{content}");
        let right_x = 600.0 - 36.0 - text_width("Right") * 10.0;
        assert!(
            content.contains(&format!("{} 38.1 Td (Right)", n(right_x))),
            "{content}"
        );
        assert_eq!(content.matches("BDC").count(), 2);
    }
}
