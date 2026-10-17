//! Reads the comments (markup annotations) already in a document, for the comments list.
//!
//! Links, form fields, popups and printer marks aren't comments, so they're left out. Counts
//! are bounded because a hostile file can list millions of annotations.

use hayro::hayro_interpret::TransformExt;
use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Name, Rect};
use hayro::hayro_syntax::page::Page;

use crate::TextBox;
use crate::document::decode_text_string;
use crate::edit::{Annotation, Kind, NOTE_SIZE};

const MAX_COMMENTS: usize = 20_000;
const MAX_PER_PAGE: usize = 2_000;
/// Longest text kept from a comment; the list only shows a snippet.
const MAX_TEXT: usize = 2_000;
/// Most marked areas kept per highlight (each is one quadrilateral, usually one line).
const MAX_AREAS: usize = 500;

/// What sort of comment it is, for the list's icon and label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentKind {
    Highlight,
    Underline,
    StrikeOut,
    Note,
    TextBox,
    Ink,
    Rect,
    Ellipse,
    Line,
    /// Any other markup, with its PDF subtype name (Stamp, Polygon, Caret, …).
    Other(String),
}

impl CommentKind {
    fn from_subtype(subtype: &str) -> Option<Self> {
        Some(match subtype {
            "Highlight" => Self::Highlight,
            "Underline" | "Squiggly" => Self::Underline,
            "StrikeOut" => Self::StrikeOut,
            "Text" => Self::Note,
            "FreeText" => Self::TextBox,
            "Ink" => Self::Ink,
            "Square" => Self::Rect,
            "Circle" => Self::Ellipse,
            "Line" => Self::Line,
            "Link" | "Widget" | "Popup" | "PrinterMark" | "TrapNet" | "Watermark" | "3D" | "RichMedia"
            | "Screen" | "Movie" | "Sound" => return None,
            other => Self::Other(other.to_owned()),
        })
    }

    /// A short name for the list, like Acrobat's.
    pub fn label(&self) -> &str {
        match self {
            Self::Highlight => "Highlight",
            Self::Underline => "Underline",
            Self::StrikeOut => "Strikethrough",
            Self::Note => "Sticky note",
            Self::TextBox => "Text box",
            Self::Ink => "Drawing",
            Self::Rect => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Line => "Line",
            Self::Other(name) => name,
        }
    }
}

/// One comment in the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub page: usize,
    pub kind: CommentKind,
    pub contents: String,
    pub author: String,
    /// Where it sits on the page, in the same top-left, unrotated-view space as page text.
    pub at: TextBox,
    /// For highlights, underlines and strikethroughs: the areas of text they mark.
    pub marked: Vec<TextBox>,
}

impl From<&Annotation> for Comment {
    /// How a newly added annotation appears in the list.
    fn from(a: &Annotation) -> Self {
        let mut at: Option<TextBox> = None;
        let mut add = |x: f32, y: f32| {
            let r = at.get_or_insert(TextBox {
                x0: x,
                y0: y,
                x1: x,
                y1: y,
            });
            *r = TextBox {
                x0: r.x0.min(x),
                y0: r.y0.min(y),
                x1: r.x1.max(x),
                y1: r.y1.max(y),
            };
        };
        let kind = match &a.kind {
            Kind::Highlight(b) | Kind::Underline(b) | Kind::StrikeOut(b) => {
                for r in b {
                    add(r.x0, r.y0);
                    add(r.x1, r.y1);
                }
                match a.kind {
                    Kind::Highlight(_) => CommentKind::Highlight,
                    Kind::Underline(_) => CommentKind::Underline,
                    _ => CommentKind::StrikeOut,
                }
            }
            Kind::Note { x, y } => {
                add(*x, *y);
                add(x + NOTE_SIZE, y + NOTE_SIZE);
                CommentKind::Note
            }
            Kind::FreeText { rect, .. } => {
                add(rect.x0, rect.y0);
                add(rect.x1, rect.y1);
                CommentKind::TextBox
            }
            Kind::Ink(strokes) => {
                for &(x, y) in strokes.iter().flatten() {
                    add(x, y);
                }
                CommentKind::Ink
            }
            Kind::Rect(r) | Kind::Ellipse(r) => {
                add(r.x0, r.y0);
                add(r.x1, r.y1);
                if matches!(a.kind, Kind::Rect(_)) {
                    CommentKind::Rect
                } else {
                    CommentKind::Ellipse
                }
            }
            Kind::Line { from, to, .. } => {
                add(from.0, from.1);
                add(to.0, to.1);
                CommentKind::Line
            }
        };
        let marked = match &a.kind {
            Kind::Highlight(b) | Kind::Underline(b) | Kind::StrikeOut(b) => b.clone(),
            _ => Vec::new(),
        };
        Comment {
            page: a.page,
            kind,
            contents: a.contents.clone(),
            author: a.author.clone(),
            marked,
            at: at.unwrap_or(TextBox {
                x0: 0.0,
                y0: 0.0,
                x1: 0.0,
                y1: 0.0,
            }),
        }
    }
}

pub(crate) fn read(pdf: &Pdf) -> Vec<Comment> {
    let mut out = Vec::new();
    for (index, page) in pdf.pages().iter().enumerate() {
        if out.len() >= MAX_COMMENTS {
            break;
        }
        read_page(page, index, &mut out);
    }
    out
}

fn read_page(page: &Page<'_>, index: usize, out: &mut Vec<Comment>) {
    let Some(annots) = page.raw().get::<Array<'_>>(b"Annots") else {
        return;
    };
    let to_view = page.initial_transform(true).to_kurbo();
    for annot in annots.iter::<Dict<'_>>().take(MAX_PER_PAGE) {
        if out.len() >= MAX_COMMENTS {
            return;
        }
        let Some(kind) = annot
            .get::<Name<'_>>(b"Subtype")
            .and_then(|s| CommentKind::from_subtype(s.as_str()))
        else {
            continue;
        };
        // Hidden annotations (flag bit 2) aren't shown, so they aren't listed either.
        if annot.get::<i64>(b"F").is_some_and(|f| f & 2 != 0) {
            continue;
        }
        let text = |key: &[u8]| {
            annot
                .get::<hayro::hayro_syntax::object::String<'_>>(key)
                .map(|s| {
                    let mut t = decode_text_string(s.as_bytes());
                    if let Some((cut, _)) = t.char_indices().nth(MAX_TEXT) {
                        t.truncate(cut);
                    }
                    t.trim().to_owned()
                })
                .unwrap_or_default()
        };
        let view_box = |points: &[(f64, f64)]| {
            let mut b: Option<TextBox> = None;
            for &(x, y) in points {
                let p = to_view * hayro::kurbo::Point::new(x, y);
                let (x, y) = (p.x as f32, p.y as f32);
                let r = b.get_or_insert(TextBox {
                    x0: x,
                    y0: y,
                    x1: x,
                    y1: y,
                });
                *r = TextBox {
                    x0: r.x0.min(x),
                    y0: r.y0.min(y),
                    x1: r.x1.max(x),
                    y1: r.y1.max(y),
                };
            }
            b
        };
        let rect = annot.get::<Rect>(b"Rect").unwrap_or(Rect::ZERO);
        let Some(at) = view_box(&[(rect.x0, rect.y0), (rect.x1, rect.y1)]) else {
            continue;
        };
        // QuadPoints: four corners per marked area, eight numbers each.
        let marked = match kind {
            CommentKind::Highlight | CommentKind::Underline | CommentKind::StrikeOut => annot
                .get::<Array<'_>>(b"QuadPoints")
                .map(|quads| {
                    let numbers: Vec<f64> = quads.iter::<f64>().take(MAX_AREAS * 8).collect();
                    let (quads, _) = numbers.as_chunks::<8>();
                    quads
                        .iter()
                        .filter_map(|q| {
                            let (points, _) = q.as_chunks::<2>();
                            let corners: Vec<(f64, f64)> = points.iter().map(|&[x, y]| (x, y)).collect();
                            view_box(&corners)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        out.push(Comment {
            page: index,
            kind,
            contents: text(b"Contents"),
            author: text(b"T"),
            at,
            marked,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_and_fields_are_not_comments() {
        assert_eq!(CommentKind::from_subtype("Link"), None);
        assert_eq!(CommentKind::from_subtype("Widget"), None);
        assert_eq!(CommentKind::from_subtype("Popup"), None);
        assert_eq!(
            CommentKind::from_subtype("Squiggly"),
            Some(CommentKind::Underline)
        );
        assert_eq!(
            CommentKind::from_subtype("Stamp"),
            Some(CommentKind::Other("Stamp".into()))
        );
    }
}
