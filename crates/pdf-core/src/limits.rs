/// Resource caps applied to every document. They protect against files crafted to exhaust
/// memory (huge page boxes, absurd page counts) rather than to exploit memory bugs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// Largest file that will be read into memory, in bytes.
    pub max_file_bytes: u64,
    /// Documents with more pages than this are rejected.
    pub max_pages: usize,
    /// Longest side of a rendered bitmap, in pixels. The engine uses `u16` dimensions.
    pub max_bitmap_side: u32,
    /// Total pixels in one rendered bitmap.
    pub max_bitmap_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 1024 * 1024 * 1024,
            max_pages: 100_000,
            max_bitmap_side: 12_000,
            max_bitmap_pixels: 64 * 1024 * 1024,
        }
    }
}

impl Limits {
    /// Reduces `scale` so that a page of `width` x `height` points stays inside the bitmap caps.
    /// Returns `None` when the page size itself is unusable (zero, negative or not finite).
    pub fn clamp_scale(&self, width: f32, height: f32, scale: f32) -> Option<f32> {
        if !(width.is_finite() && height.is_finite() && scale.is_finite()) {
            return None;
        }
        if width <= 0.0 || height <= 0.0 || scale <= 0.0 {
            return None;
        }
        let side = self.max_bitmap_side as f32;
        let mut scale = scale.min(side / width).min(side / height);
        let pixels = f64::from(width * scale) * f64::from(height * scale);
        let max_pixels = self.max_bitmap_pixels as f64;
        if pixels > max_pixels {
            scale *= (max_pixels / pixels).sqrt() as f32;
        }
        // A bitmap must be at least one pixel on each side.
        if width * scale < 1.0 || height * scale < 1.0 {
            return None;
        }
        Some(scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_to_side_limit() {
        let limits = Limits::default();
        let scale = limits.clamp_scale(100_000.0, 10.0, 1.0);
        assert!(scale.is_some_and(|s| 100_000.0 * s <= 12_000.0 + 0.5));
    }

    #[test]
    fn clamps_to_pixel_limit() {
        let limits = Limits::default();
        let scale = limits.clamp_scale(10_000.0, 10_000.0, 1.0).unwrap_or(0.0);
        let pixels = f64::from(10_000.0 * scale).powi(2);
        assert!(pixels <= limits.max_bitmap_pixels as f64 * 1.001);
    }

    #[test]
    fn rejects_degenerate_pages() {
        let limits = Limits::default();
        assert_eq!(limits.clamp_scale(0.0, 10.0, 1.0), None);
        assert_eq!(limits.clamp_scale(f32::NAN, 10.0, 1.0), None);
        assert_eq!(limits.clamp_scale(10.0, f32::INFINITY, 1.0), None);
        assert_eq!(limits.clamp_scale(10.0, 10.0, -1.0), None);
    }

    #[test]
    fn leaves_normal_pages_alone() {
        let limits = Limits::default();
        assert_eq!(limits.clamp_scale(612.0, 792.0, 2.0), Some(2.0));
    }
}
