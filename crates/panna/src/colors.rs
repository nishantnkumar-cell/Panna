//! Page colour themes for comfortable reading: pages as printed, dark (light text on a dark
//! page, colours kept), or sepia. Applied to rendered pixels on the render thread.

/// How pages are coloured on screen. Saving and printing always use the file's own colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageColors {
    #[default]
    Normal,
    Dark,
    Sepia,
}

/// The dark theme's page and text greys, softer than pure black and white.
const DARK_PAGE: f32 = 30.0;
const DARK_TEXT: f32 = 225.0;
/// The sepia theme's paper colour.
const SEPIA: [f32; 3] = [244.0, 236.0, 216.0];

impl PageColors {
    pub fn as_str(self) -> &'static str {
        match self {
            PageColors::Normal => "normal",
            PageColors::Dark => "dark",
            PageColors::Sepia => "sepia",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "normal" => Some(PageColors::Normal),
            "dark" => Some(PageColors::Dark),
            "sepia" => Some(PageColors::Sepia),
            _ => None,
        }
    }

    /// The next theme, for a button that cycles through them.
    pub fn next(self) -> Self {
        match self {
            PageColors::Normal => PageColors::Dark,
            PageColors::Dark => PageColors::Sepia,
            PageColors::Sepia => PageColors::Normal,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PageColors::Normal => "original",
            PageColors::Dark => "dark",
            PageColors::Sepia => "sepia",
        }
    }

    /// Recolours RGBA pixels in place.
    pub fn apply(self, rgba: &mut [u8]) {
        match self {
            PageColors::Normal => {}
            PageColors::Dark => {
                let span = (DARK_TEXT - DARK_PAGE) / 255.0;
                for px in rgba.as_chunks_mut::<4>().0 {
                    // Inverts lightness but keeps hue: equivalent to inverting each channel
                    // and turning the hue half way round. White becomes black, red stays red.
                    let [r, g, b, _] = *px;
                    let lo = r.min(g).min(b);
                    let hi = r.max(g).max(b);
                    let shift = 255.0 - f32::from(lo) - f32::from(hi);
                    for c in px.iter_mut().take(3) {
                        let v = (f32::from(*c) + shift).clamp(0.0, 255.0);
                        *c = (DARK_PAGE + v * span).round() as u8;
                    }
                }
            }
            PageColors::Sepia => {
                for px in rgba.as_chunks_mut::<4>().0 {
                    for (c, paper) in px.iter_mut().zip(SEPIA) {
                        *c = (f32::from(*c) * paper / 255.0).round() as u8;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(colors: PageColors, px: [u8; 4]) -> [u8; 4] {
        let mut v = px;
        colors.apply(&mut v);
        v
    }

    #[test]
    fn dark_swaps_light_and_dark_but_keeps_colour() {
        assert_eq!(one(PageColors::Dark, [255, 255, 255, 255]), [30, 30, 30, 255]);
        assert_eq!(one(PageColors::Dark, [0, 0, 0, 200]), [225, 225, 225, 200]);
        let [r, g, b, _] = one(PageColors::Dark, [220, 30, 30, 255]);
        assert!(r > 150 && g < 80 && b < 80, "{r} {g} {b}");
    }

    #[test]
    fn sepia_tints_paper_and_keeps_black() {
        assert_eq!(one(PageColors::Sepia, [255, 255, 255, 255]), [244, 236, 216, 255]);
        assert_eq!(one(PageColors::Sepia, [0, 0, 0, 255]), [0, 0, 0, 255]);
        assert_eq!(one(PageColors::Normal, [1, 2, 3, 4]), [1, 2, 3, 4]);
    }

    #[test]
    fn names_round_trip() {
        let mut c = PageColors::Normal;
        for _ in 0..3 {
            assert_eq!(PageColors::parse(c.as_str()), Some(c));
            c = c.next();
        }
        assert_eq!(c, PageColors::Normal);
    }
}
