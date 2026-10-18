//! Vertical page layout maths, kept free of UI types so it can be unit-tested.

use std::ops::Range;

use panna_core::PageSize;

/// Space between pages and around the page column, in logical pixels.
pub const PAGE_GAP: f32 = 16.0;

/// How pages are arranged: one per row, or side by side like an open book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Spread {
    #[default]
    Single,
    /// Two pages per row: 1–2, 3–4, …
    Two,
    /// Two per row with the first page alone, as a book's cover: 1, 2–3, 4–5, …
    TwoCover,
}

impl Spread {
    pub fn as_str(self) -> &'static str {
        match self {
            Spread::Single => "single",
            Spread::Two => "two",
            Spread::TwoCover => "two-cover",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "single" => Some(Spread::Single),
            "two" => Some(Spread::Two),
            "two-cover" => Some(Spread::TwoCover),
            _ => None,
        }
    }

    /// The next arrangement, for a button that cycles through them.
    pub fn next(self) -> Self {
        match self {
            Spread::Single => Spread::Two,
            Spread::Two => Spread::TwoCover,
            Spread::TwoCover => Spread::Single,
        }
    }
}

/// Page sizes in points, grouped into rows, with prefix sums of row heights for fast position
/// lookups.
#[derive(Debug, Default, Clone)]
pub struct PageLayout {
    sizes: Vec<PageSize>,
    /// The pages in each row.
    rows: Vec<Range<usize>>,
    /// The row each page is in.
    row_of: Vec<usize>,
    /// `tops[r]` is the sum of heights of rows before `r`, unzoomed (gaps excluded).
    tops: Vec<f32>,
    /// The widest row: its pages' total width in points, and how many pages it has.
    widest: (f32, usize),
}

impl PageLayout {
    pub fn new(sizes: &[PageSize], spread: Spread) -> Self {
        let mut rows = Vec::new();
        let mut i = 0;
        while i < sizes.len() {
            let n = match spread {
                Spread::Single => 1,
                Spread::TwoCover if i == 0 => 1,
                _ => 2,
            };
            let end = (i + n).min(sizes.len());
            rows.push(i..end);
            i = end;
        }
        let mut row_of = vec![0; sizes.len()];
        let mut tops = Vec::with_capacity(rows.len() + 1);
        let mut y = 0.0;
        let mut widest = (0.0_f32, 1);
        for (r, row) in rows.iter().enumerate() {
            tops.push(y);
            let page_sizes = sizes.get(row.clone()).unwrap_or_default();
            y += page_sizes.iter().map(|s| s.height).fold(0.0, f32::max);
            let width: f32 = page_sizes.iter().map(|s| s.width).sum();
            if width > widest.0 {
                widest = (width, row.len());
            }
            for slot in row_of.get_mut(row.clone()).unwrap_or_default() {
                *slot = r;
            }
        }
        tops.push(y);
        Self {
            sizes: sizes.to_vec(),
            rows,
            row_of,
            tops,
            widest,
        }
    }

    pub fn sizes(&self) -> &[PageSize] {
        &self.sizes
    }

    pub fn rows(&self) -> &[Range<usize>] {
        &self.rows
    }

    /// Width of the scrolled content at `zoom`: the widest row with a margin on each side, and
    /// at least `viewport` so pages stay centred.
    pub fn content_width(&self, zoom: f32, viewport: f32) -> f32 {
        let (points, pages) = self.widest;
        (points * zoom + pages.saturating_sub(1) as f32 * PAGE_GAP + 2.0 * PAGE_GAP).max(viewport)
    }

    /// The zoom at which the widest row fills `available` pixels.
    pub fn fit_width(&self, available: f32) -> Option<f32> {
        let (points, pages) = self.widest;
        let room = available - pages.saturating_sub(1) as f32 * PAGE_GAP;
        (points > 0.0).then(|| room.max(50.0) / points)
    }

    /// The size of the row holding `page`, in points: total width (gaps excluded) and height.
    pub fn row_size(&self, page: usize) -> Option<(f32, usize, f32)> {
        let row = self.rows.get(*self.row_of.get(page)?)?;
        let sizes = self.sizes.get(row.clone())?;
        Some((
            sizes.iter().map(|s| s.width).sum(),
            row.len(),
            sizes.iter().map(|s| s.height).fold(0.0, f32::max),
        ))
    }

    /// Left edge of page `index` in scroll coordinates, with rows centred in `content_width`.
    pub fn page_left(&self, index: usize, zoom: f32, content_width: f32) -> f32 {
        let Some((points, pages, _)) = self.row_size(index) else {
            return 0.0;
        };
        let row_width = points * zoom + pages.saturating_sub(1) as f32 * PAGE_GAP;
        let row = self
            .row_of
            .get(index)
            .and_then(|r| self.rows.get(*r))
            .cloned()
            .unwrap_or(0..0);
        let before: f32 = self
            .sizes
            .get(row.start..index)
            .unwrap_or_default()
            .iter()
            .map(|s| s.width * zoom + PAGE_GAP)
            .sum();
        (content_width - row_width) / 2.0 + before
    }

    /// The first page of the row after (or before) the one holding `page`, so page-by-page
    /// steps move a whole spread at a time.
    pub fn step(&self, page: usize, forward: bool) -> usize {
        let row = self.row_of.get(page).copied().unwrap_or(0);
        let row = if forward { row + 1 } else { row.saturating_sub(1) };
        self.rows.get(row).map_or(page, |r| r.start)
    }

    /// Top edge of page `index` (its row's top) in scroll coordinates at `zoom`.
    pub fn page_top(&self, index: usize, zoom: f32) -> f32 {
        let row = self.row_of.get(index).copied().unwrap_or(0);
        let top = self.tops.get(row).copied().unwrap_or(0.0);
        PAGE_GAP + top * zoom + row as f32 * PAGE_GAP
    }

    fn row_top(&self, row: usize, zoom: f32) -> f32 {
        PAGE_GAP + self.tops.get(row).copied().unwrap_or(0.0) * zoom + row as f32 * PAGE_GAP
    }

    /// Index of the row under scroll position `y`.
    fn row_at(&self, y: f32, zoom: f32) -> usize {
        if self.rows.is_empty() {
            return 0;
        }
        // Binary search over row tops; positions in a gap belong to the row above.
        let (mut lo, mut hi) = (0, self.rows.len() - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.row_top(mid, zoom) <= y {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    }

    /// Index of the (first) page under scroll position `y`.
    pub fn page_at(&self, y: f32, zoom: f32) -> usize {
        self.rows.get(self.row_at(y, zoom)).map_or(0, |r| r.start)
    }

    /// Pages that intersect the viewport `[offset, offset + height)`.
    pub fn visible_range(&self, offset: f32, height: f32, zoom: f32) -> Range<usize> {
        if self.rows.is_empty() {
            return 0..0;
        }
        let first = self.row_at(offset, zoom);
        let last = self.row_at(offset + height.max(1.0), zoom);
        let start = self.rows.get(first).map_or(0, |r| r.start);
        let end = self.rows.get(last).map_or(start, |r| r.end);
        start..end
    }

    /// Converts a scroll position to a zoom-independent document position (page index plus
    /// fraction through that page's row), so zooming can keep the same content in view.
    pub fn unzoomed_y(&self, y: f32, zoom: f32) -> (usize, f32) {
        let row = self.row_at(y, zoom);
        let index = self.rows.get(row).map_or(0, |r| r.start);
        let height = self.row_size(index).map_or(1.0, |(_, _, h)| h * zoom).max(1.0);
        let fraction = ((y - self.row_top(row, zoom)) / height).clamp(0.0, 1.0);
        (index, fraction)
    }

    pub fn zoomed_y(&self, (index, fraction): (usize, f32), zoom: f32) -> f32 {
        let height = self.row_size(index).map_or(0.0, |(_, _, h)| h * zoom);
        (self.page_top(index, zoom) + fraction * height - PAGE_GAP / 2.0).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter_spread(n: usize, spread: Spread) -> PageLayout {
        PageLayout::new(
            &vec![
                PageSize {
                    width: 612.0,
                    height: 792.0
                };
                n
            ],
            spread,
        )
    }

    fn letter(n: usize) -> PageLayout {
        letter_spread(n, Spread::Single)
    }

    #[test]
    fn page_tops_include_gaps() {
        let layout = letter(3);
        assert_eq!(layout.page_top(0, 1.0), PAGE_GAP);
        assert_eq!(layout.page_top(1, 1.0), PAGE_GAP + 792.0 + PAGE_GAP);
        assert_eq!(layout.page_top(2, 0.5), PAGE_GAP + 792.0 + 2.0 * PAGE_GAP);
    }

    #[test]
    fn finds_page_at_position() {
        let layout = letter(10);
        assert_eq!(layout.page_at(0.0, 1.0), 0);
        assert_eq!(layout.page_at(layout.page_top(4, 1.0) + 1.0, 1.0), 4);
        assert_eq!(layout.page_at(1.0e9, 1.0), 9);
    }

    #[test]
    fn visible_range_covers_viewport() {
        let layout = letter(10);
        let top = layout.page_top(3, 1.0);
        assert_eq!(layout.visible_range(top + 10.0, 900.0, 1.0), 3..5);
        assert_eq!(PageLayout::default().visible_range(0.0, 100.0, 1.0), 0..0);
    }

    #[test]
    fn zoom_round_trip_keeps_page() {
        let layout = letter(10);
        let y = layout.page_top(6, 1.0) + 300.0;
        let anchor = layout.unzoomed_y(y, 1.0);
        let zoomed = layout.zoomed_y(anchor, 2.0);
        assert_eq!(layout.page_at(zoomed + PAGE_GAP, 2.0), 6);
    }

    #[test]
    fn two_page_rows() {
        let layout = letter_spread(5, Spread::Two);
        assert_eq!(layout.rows(), [0..2, 2..4, 4..5]);
        assert_eq!(layout.page_top(3, 1.0), layout.page_top(2, 1.0));
        assert_eq!(layout.page_at(layout.page_top(1, 1.0) + 5.0, 1.0), 0);
        assert_eq!(
            layout.visible_range(layout.page_top(2, 1.0) + 5.0, 10.0, 1.0),
            2..4
        );
        let content = layout.content_width(1.0, 0.0);
        assert_eq!(content, 2.0 * 612.0 + 3.0 * PAGE_GAP);
        assert_eq!(layout.page_left(0, 1.0, content), PAGE_GAP);
        assert_eq!(layout.page_left(1, 1.0, content), 2.0 * PAGE_GAP + 612.0);
        // The last page, alone in its row, is centred.
        assert_eq!(layout.page_left(4, 1.0, content), (content - 612.0) / 2.0);
    }

    #[test]
    fn steps_move_by_row() {
        let layout = letter_spread(5, Spread::TwoCover);
        assert_eq!(layout.step(0, true), 1);
        assert_eq!(layout.step(2, true), 3);
        assert_eq!(layout.step(4, true), 4);
        assert_eq!(layout.step(4, false), 1);
        assert_eq!(layout.step(1, false), 0);
    }

    #[test]
    fn cover_page_sits_alone() {
        let layout = letter_spread(4, Spread::TwoCover);
        assert_eq!(layout.rows(), [0..1, 1..3, 3..4]);
        assert_eq!(Spread::parse(Spread::TwoCover.as_str()), Some(Spread::TwoCover));
    }
}
