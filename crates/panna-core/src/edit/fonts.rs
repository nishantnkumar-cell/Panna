//! The standard PDF fonts that added and changed text can be drawn in, with the widths of
//! their printable ASCII letters (from Adobe's font metrics, as the PDF engine has them).

use serde::{Deserialize, Serialize};

/// A family of the standard fonts every PDF reader has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
pub enum FontFamily {
    #[default]
    Helvetica,
    Times,
    Courier,
}

impl FontFamily {
    pub const ALL: [FontFamily; 3] = [FontFamily::Helvetica, FontFamily::Times, FontFamily::Courier];

    pub fn label(self) -> &'static str {
        match self {
            FontFamily::Helvetica => "Helvetica",
            FontFamily::Times => "Times",
            FontFamily::Courier => "Courier",
        }
    }
}

impl std::fmt::Display for FontFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A standard font: a family, bold or not, italic or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
pub struct TextFont {
    pub family: FontFamily,
    pub bold: bool,
    pub italic: bool,
}

impl TextFont {
    /// Every font, in the order of their resource names.
    pub const ALL: [TextFont; 12] = [
        TextFont {
            family: FontFamily::Helvetica,
            bold: false,
            italic: false,
        },
        TextFont {
            family: FontFamily::Helvetica,
            bold: true,
            italic: false,
        },
        TextFont {
            family: FontFamily::Helvetica,
            bold: false,
            italic: true,
        },
        TextFont {
            family: FontFamily::Helvetica,
            bold: true,
            italic: true,
        },
        TextFont {
            family: FontFamily::Times,
            bold: false,
            italic: false,
        },
        TextFont {
            family: FontFamily::Times,
            bold: true,
            italic: false,
        },
        TextFont {
            family: FontFamily::Times,
            bold: false,
            italic: true,
        },
        TextFont {
            family: FontFamily::Times,
            bold: true,
            italic: true,
        },
        TextFont {
            family: FontFamily::Courier,
            bold: false,
            italic: false,
        },
        TextFont {
            family: FontFamily::Courier,
            bold: true,
            italic: false,
        },
        TextFont {
            family: FontFamily::Courier,
            bold: false,
            italic: true,
        },
        TextFont {
            family: FontFamily::Courier,
            bold: true,
            italic: true,
        },
    ];

    /// Its PostScript name, as a PDF names it.
    pub fn base_font(self) -> &'static str {
        match (self.family, self.bold, self.italic) {
            (FontFamily::Helvetica, false, false) => "Helvetica",
            (FontFamily::Helvetica, true, false) => "Helvetica-Bold",
            (FontFamily::Helvetica, false, true) => "Helvetica-Oblique",
            (FontFamily::Helvetica, true, true) => "Helvetica-BoldOblique",
            (FontFamily::Times, false, false) => "Times-Roman",
            (FontFamily::Times, true, false) => "Times-Bold",
            (FontFamily::Times, false, true) => "Times-Italic",
            (FontFamily::Times, true, true) => "Times-BoldItalic",
            (FontFamily::Courier, false, false) => "Courier",
            (FontFamily::Courier, true, false) => "Courier-Bold",
            (FontFamily::Courier, false, true) => "Courier-Oblique",
            (FontFamily::Courier, true, true) => "Courier-BoldOblique",
        }
    }

    /// The name drawings use for it in their resources. Helvetica and Helvetica Bold keep
    /// the names they always had.
    pub(crate) fn resource(self) -> &'static str {
        match (self.family, self.bold, self.italic) {
            (FontFamily::Helvetica, false, false) => "PaHe",
            (FontFamily::Helvetica, true, false) => "PaHB",
            (FontFamily::Helvetica, false, true) => "PaHI",
            (FontFamily::Helvetica, true, true) => "PaHX",
            (FontFamily::Times, false, false) => "PaTR",
            (FontFamily::Times, true, false) => "PaTB",
            (FontFamily::Times, false, true) => "PaTI",
            (FontFamily::Times, true, true) => "PaTX",
            (FontFamily::Courier, false, false) => "PaCR",
            (FontFamily::Courier, true, false) => "PaCB",
            (FontFamily::Courier, false, true) => "PaCI",
            (FontFamily::Courier, true, true) => "PaCX",
        }
    }

    /// The resource entry naming it, in the WinAnsi encoding text is written in.
    pub(crate) fn resource_entry(self) -> String {
        format!(
            "/{} << /Type /Font /Subtype /Type1 /BaseFont /{} /Encoding /WinAnsiEncoding >>",
            self.resource(),
            self.base_font()
        )
    }

    /// Width of `s` in this font, in units of the font size. Letters outside printable
    /// ASCII count as a typical letter.
    pub fn width(self, s: &str) -> f64 {
        let table: Option<&[u16; 95]> = match (self.family, self.bold) {
            (FontFamily::Helvetica, false) => Some(&super::annot::HELVETICA),
            (FontFamily::Helvetica, true) => Some(&HELVETICA_BOLD),
            (FontFamily::Times, false) if self.italic => Some(&TIMES_ITALIC),
            (FontFamily::Times, false) => Some(&TIMES_ROMAN),
            (FontFamily::Times, true) if self.italic => Some(&TIMES_BOLD_ITALIC),
            (FontFamily::Times, true) => Some(&TIMES_BOLD),
            (FontFamily::Courier, _) => None,
        };
        s.chars()
            .map(|c| match table {
                Some(table) => {
                    let i = (c as u32).wrapping_sub(32) as usize;
                    f64::from(table.get(i).copied().unwrap_or(556)) / 1000.0
                }
                None => 0.6,
            })
            .sum()
    }
}

/// How lines of text sit in their box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Left,
    Centre,
    Right,
}

const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556,
    556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278,
    556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584,
    556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556,
    333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

const TIMES_ROMAN: [u16; 95] = [
    250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278, 500, 500, 500, 500, 500,
    500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444, 921, 722, 667, 667, 722, 611, 556, 722, 722, 333,
    389, 722, 611, 889, 722, 722, 556, 722, 667, 556, 611, 722, 722, 944, 722, 722, 611, 333, 278, 333, 469,
    500, 333, 444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500, 278, 778, 500, 500, 500, 500, 333, 389,
    278, 500, 500, 722, 500, 500, 444, 480, 200, 480, 541,
];

const TIMES_BOLD: [u16; 95] = [
    250, 333, 555, 500, 500, 1000, 833, 278, 333, 333, 500, 570, 250, 333, 250, 278, 500, 500, 500, 500, 500,
    500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500, 930, 722, 667, 722, 722, 667, 611, 778, 778, 389,
    500, 778, 667, 944, 722, 778, 611, 778, 722, 556, 667, 722, 722, 1000, 722, 722, 667, 333, 278, 333, 581,
    500, 333, 500, 556, 444, 556, 444, 333, 500, 556, 278, 333, 556, 278, 833, 556, 500, 556, 556, 444, 389,
    333, 556, 500, 722, 500, 500, 444, 394, 220, 394, 520,
];

const TIMES_ITALIC: [u16; 95] = [
    250, 333, 420, 500, 500, 833, 778, 214, 333, 333, 500, 675, 250, 333, 250, 278, 500, 500, 500, 500, 500,
    500, 500, 500, 500, 500, 333, 333, 675, 675, 675, 500, 920, 611, 611, 667, 722, 611, 611, 722, 722, 333,
    444, 667, 556, 833, 667, 722, 611, 722, 611, 500, 556, 722, 611, 833, 611, 556, 556, 389, 278, 389, 422,
    500, 333, 500, 500, 444, 500, 444, 278, 500, 500, 278, 278, 444, 278, 722, 500, 500, 500, 500, 389, 389,
    278, 500, 444, 667, 444, 444, 389, 400, 275, 400, 541,
];

const TIMES_BOLD_ITALIC: [u16; 95] = [
    250, 389, 555, 500, 500, 833, 778, 278, 333, 333, 500, 570, 250, 333, 250, 278, 500, 500, 500, 500, 500,
    500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500, 832, 667, 667, 667, 722, 667, 667, 722, 778, 389,
    500, 667, 611, 889, 722, 722, 611, 722, 667, 556, 611, 722, 667, 889, 667, 611, 611, 333, 278, 333, 570,
    500, 333, 500, 500, 444, 500, 444, 333, 500, 556, 278, 278, 500, 278, 778, 556, 500, 500, 500, 389, 389,
    278, 556, 444, 667, 500, 444, 389, 348, 220, 348, 570,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_have_their_own_widths_and_names() {
        let times = TextFont {
            family: FontFamily::Times,
            ..TextFont::default()
        };
        assert!((times.width("W") - 0.944).abs() < 1e-9);
        assert!((TextFont::default().width("W") - 0.944).abs() < 1e-9);
        assert!(times.width("Hello") < TextFont::default().width("Hello"));
        let courier = TextFont {
            family: FontFamily::Courier,
            bold: true,
            italic: true,
        };
        assert!((courier.width("il") - 1.2).abs() < 1e-9);
        assert_eq!(courier.base_font(), "Courier-BoldOblique");
        let names: std::collections::HashSet<_> = TextFont::ALL.iter().map(|f| f.resource()).collect();
        assert_eq!(names.len(), 12);
        assert_eq!(TextFont::ALL[0], TextFont::default());
    }
}
