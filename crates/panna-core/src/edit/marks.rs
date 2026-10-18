//! Headers and footers: lines of text drawn at the top and bottom of pages, such as page
//! numbers, a title, a date or Bates numbers.
//!
//! They become part of the page itself, in a form of their own that the page draws after its
//! content, marked as pagination artifacts so that screen readers skip them.

use std::fmt::Write as _;

use super::annot::{literal, n, text_width, win_ansi};

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

/// The font the text is drawn in, as it's named in the form's resources.
pub(crate) const RESOURCES: &str = "<< /Font << /PaHe << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> >> >>";

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
pub(crate) fn today() -> String {
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
