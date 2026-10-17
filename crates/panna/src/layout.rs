//! Vertical page layout maths, kept free of UI types so it can be unit-tested.

use std::ops::Range;

use panna_core::PageSize;

/// Space between pages and around the page column, in logical pixels.
pub const PAGE_GAP: f32 = 16.0;

/// Page sizes in points, with prefix sums of heights for fast position lookups.
#[derive(Debug, Default, Clone)]
pub struct PageLayout {
    sizes: Vec<PageSize>,
    /// `tops[i]` is the sum of heights of pages before `i`, unzoomed (gaps excluded).
    tops: Vec<f32>,
    max_width: f32,
}

impl PageLayout {
    pub fn new(sizes: &[PageSize]) -> Self {
        let mut tops = Vec::with_capacity(sizes.len() + 1);
        let mut y = 0.0;
        for size in sizes {
            tops.push(y);
            y += size.height;
        }
        tops.push(y);
        let max_width = sizes.iter().map(|s| s.width).fold(0.0, f32::max);
        Self {
            sizes: sizes.to_vec(),
            tops,
            max_width,
        }
    }

    pub fn sizes(&self) -> &[PageSize] {
        &self.sizes
    }

    pub fn max_width(&self) -> f32 {
        self.max_width
    }

    /// Top edge of page `index` in scroll coordinates at `zoom`.
    pub fn page_top(&self, index: usize, zoom: f32) -> f32 {
        let top = self.tops.get(index).copied().unwrap_or(0.0);
        PAGE_GAP + top * zoom + index as f32 * PAGE_GAP
    }

    /// Index of the page under scroll position `y`.
    pub fn page_at(&self, y: f32, zoom: f32) -> usize {
        if self.sizes.is_empty() {
            return 0;
        }
        // Binary search over page tops; positions in a gap belong to the page above.
        let (mut lo, mut hi) = (0, self.sizes.len() - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.page_top(mid, zoom) <= y {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    }

    /// Pages that intersect the viewport `[offset, offset + height)`.
    pub fn visible_range(&self, offset: f32, height: f32, zoom: f32) -> Range<usize> {
        if self.sizes.is_empty() {
            return 0..0;
        }
        let first = self.page_at(offset, zoom);
        let last = self.page_at(offset + height.max(1.0), zoom);
        first..last + 1
    }

    /// Converts a scroll position to a zoom-independent document position (page index plus
    /// fraction through that page), so zooming can keep the same content in view.
    pub fn unzoomed_y(&self, y: f32, zoom: f32) -> (usize, f32) {
        let index = self.page_at(y, zoom);
        let height = self.sizes.get(index).map_or(1.0, |s| s.height * zoom).max(1.0);
        let fraction = ((y - self.page_top(index, zoom)) / height).clamp(0.0, 1.0);
        (index, fraction)
    }

    pub fn zoomed_y(&self, (index, fraction): (usize, f32), zoom: f32) -> f32 {
        let height = self.sizes.get(index).map_or(0.0, |s| s.height * zoom);
        (self.page_top(index, zoom) + fraction * height - PAGE_GAP / 2.0).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter(n: usize) -> PageLayout {
        PageLayout::new(&vec![
            PageSize {
                width: 612.0,
                height: 792.0
            };
            n
        ])
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
}
