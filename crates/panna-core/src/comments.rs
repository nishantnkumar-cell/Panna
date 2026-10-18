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
use crate::edit::{Annotation, Color, Kind, NOTE_SIZE};

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
    Polygon,
    Cloud,
    Stamp,
    /// A reply in another comment's thread, or a review status set on it.
    Reply,
    /// Any other markup, with its PDF subtype name (Caret, FileAttachment, …).
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
            "Polygon" => Self::Polygon,
            "Stamp" => Self::Stamp,
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
            Self::Polygon => "Polygon",
            Self::Cloud => "Cloud",
            Self::Stamp => "Stamp",
            Self::Reply => "Reply",
            Self::Other(name) => name,
        }
    }
}

/// A review status set on a comment, as in Acrobat's review workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    None,
    Accepted,
    Rejected,
    Cancelled,
    Completed,
}

impl ReviewState {
    pub const ALL: [ReviewState; 5] = [
        ReviewState::None,
        ReviewState::Accepted,
        ReviewState::Rejected,
        ReviewState::Cancelled,
        ReviewState::Completed,
    ];

    /// The value of /State in the file.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Accepted => "Accepted",
            Self::Rejected => "Rejected",
            Self::Cancelled => "Cancelled",
            Self::Completed => "Completed",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "No status",
            other => other.name(),
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.name() == s)
    }
}

impl std::fmt::Display for ReviewState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A PDF object reference, `number generation R`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjRef {
    pub number: u32,
    pub generation: u16,
}

impl ObjRef {
    pub(crate) fn new(number: i32, generation: i32) -> Option<Self> {
        Some(Self {
            number: u32::try_from(number).ok()?,
            generation: u16::try_from(generation).ok()?,
        })
    }
}

/// Which saved annotation a comment is, so it can be removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommentId {
    pub page: usize,
    pub annot: ObjRef,
    /// Its pop-up window annotation, removed along with it.
    pub popup: Option<ObjRef>,
}

/// One comment in the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    /// Set for comments saved in the file as their own objects (nearly all of them).
    pub id: Option<CommentId>,
    pub page: usize,
    pub kind: CommentKind,
    pub contents: String,
    pub author: String,
    /// Where it sits on the page, in the same top-left, unrotated-view space as page text.
    pub at: TextBox,
    /// For highlights, underlines and strikethroughs: the areas of text they mark.
    pub marked: Vec<TextBox>,
    /// For replies: the comment replied to.
    pub reply_to: Option<ObjRef>,
    /// For review statuses (replies that set one): the status.
    pub state: Option<ReviewState>,
    /// The comment as an annotation Panna can redraw, for the kinds it can make itself. Moving
    /// or restyling a saved comment replaces it with this, changed.
    pub editable: Option<Annotation>,
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
            Kind::Polygon { points, cloud } => {
                for &(x, y) in points {
                    add(x, y);
                }
                if *cloud {
                    CommentKind::Cloud
                } else {
                    CommentKind::Polygon
                }
            }
            Kind::Stamp { rect, .. } => {
                add(rect.x0, rect.y0);
                add(rect.x1, rect.y1);
                CommentKind::Stamp
            }
            Kind::Reply { at, .. } => {
                add(at.x0, at.y0);
                add(at.x1, at.y1);
                CommentKind::Reply
            }
        };
        let (reply_to, state) = match &a.kind {
            Kind::Reply { to, state, .. } => (Some(*to), *state),
            _ => (None, None),
        };
        let marked = match &a.kind {
            Kind::Highlight(b) | Kind::Underline(b) | Kind::StrikeOut(b) => b.clone(),
            _ => Vec::new(),
        };
        Comment {
            id: None,
            page: a.page,
            kind,
            contents: a.contents.clone(),
            author: a.author.clone(),
            marked,
            reply_to,
            state,
            editable: None,
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
        let Some(subtype) = annot.get::<Name<'_>>(b"Subtype") else {
            continue;
        };
        let subtype = subtype.as_str();
        let Some(mut kind) = CommentKind::from_subtype(subtype) else {
            continue;
        };
        if kind == CommentKind::Polygon && is_cloudy(&annot) {
            kind = CommentKind::Cloud;
        }
        let reply_to = annot
            .get_ref(b"IRT")
            .and_then(|r| ObjRef::new(r.obj_number, r.gen_number));
        if reply_to.is_some() {
            // Grouped annotations act as one with their parent; they aren't replies.
            if annot
                .get::<Name<'_>>(b"RT")
                .is_some_and(|rt| rt.as_str() == "Group")
            {
                continue;
            }
            kind = CommentKind::Reply;
        } else if annot.get::<i64>(b"F").is_some_and(|f| f & 2 != 0) {
            // Hidden annotations (flag bit 2) aren't shown, so they aren't listed either.
            // Replies are often hidden on the page but still belong in the thread.
            continue;
        }
        let state = reply_to
            .and_then(|_| annot.get::<hayro::hayro_syntax::object::String<'_>>(b"State"))
            .filter(|_| {
                annot
                    .get::<hayro::hayro_syntax::object::String<'_>>(b"StateModel")
                    .is_none_or(|m| decode_text_string(m.as_bytes()) == "Review")
            })
            .and_then(|s| ReviewState::parse(&decode_text_string(s.as_bytes())));
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
        let id = annot
            .obj_id()
            .and_then(|id| ObjRef::new(id.obj_number, id.gen_number))
            .map(|annot_ref| CommentId {
                page: index,
                annot: annot_ref,
                popup: annot
                    .get_ref(b"Popup")
                    .and_then(|r| ObjRef::new(r.obj_number, r.gen_number)),
            });
        let contents = text(b"Contents");
        let author = text(b"T");
        let redrawn = match reply_to {
            Some(to) => Some(Kind::Reply { to, at, state }),
            None => editable(&annot, subtype, at, &marked, &view_box),
        };
        let editable = redrawn.map(|kind| Annotation {
            page: index,
            kind,
            color: color(&annot),
            opacity: annot.get::<f64>(b"CA").map_or(1.0, |o| o.clamp(0.0, 1.0) as f32),
            width: width(&annot),
            contents: contents.clone(),
            author: author.clone(),
        });
        out.push(Comment {
            id,
            page: index,
            kind,
            contents,
            author,
            at,
            marked,
            reply_to,
            state,
            editable,
        });
    }
}

/// Maps PDF points to the bounding box of their view-space positions.
type ViewBox<'a> = dyn Fn(&[(f64, f64)]) -> Option<TextBox> + 'a;

/// The shape of a saved comment, for the kinds Panna draws itself.
fn editable(
    annot: &Dict<'_>,
    subtype: &str,
    at: TextBox,
    marked: &[TextBox],
    view_box: &ViewBox<'_>,
) -> Option<Kind> {
    // Panna pads a shape's /Rect by a point on each side; others pad by /RD or not at all.
    let inner = |pad: f32| {
        let pad = pad.min(at.width() / 2.0).min(at.height() / 2.0).max(0.0);
        TextBox {
            x0: at.x0 + pad,
            y0: at.y0 + pad,
            x1: at.x1 - pad,
            y1: at.y1 - pad,
        }
    };
    Some(match subtype {
        "Highlight" if !marked.is_empty() => Kind::Highlight(marked.to_vec()),
        "Underline" if !marked.is_empty() => Kind::Underline(marked.to_vec()),
        "StrikeOut" if !marked.is_empty() => Kind::StrikeOut(marked.to_vec()),
        "Text" => Kind::Note { x: at.x0, y: at.y0 },
        "FreeText" => Kind::FreeText {
            rect: at,
            size: annot
                .get::<hayro::hayro_syntax::object::String<'_>>(b"DA")
                .and_then(|da| font_size(da.as_bytes()))
                .unwrap_or(12.0),
        },
        "Square" | "Circle" => {
            let pad = annot
                .get::<Array<'_>>(b"RD")
                .and_then(|rd| rd.iter::<f64>().next())
                .map_or(1.0, |p| p as f32);
            if subtype == "Square" {
                Kind::Rect(inner(pad))
            } else {
                Kind::Ellipse(inner(pad))
            }
        }
        "Line" => {
            let l: Vec<f64> = annot.get::<Array<'_>>(b"L")?.iter::<f64>().take(4).collect();
            let [x0, y0, x1, y1] = l.as_slice() else {
                return None;
            };
            let a = view_box(&[(*x0, *y0)])?;
            let b = view_box(&[(*x1, *y1)])?;
            let arrow = annot
                .get::<Array<'_>>(b"LE")
                .is_some_and(|le| le.iter::<Name<'_>>().any(|n| n.as_str().contains("Arrow")));
            Kind::Line {
                from: (a.x0, a.y0),
                to: (b.x0, b.y0),
                arrow,
            }
        }
        "Polygon" => {
            let numbers: Vec<f64> = annot
                .get::<Array<'_>>(b"Vertices")?
                .iter::<f64>()
                .take(20_000)
                .collect();
            let (pairs, _) = numbers.as_chunks::<2>();
            let points: Vec<(f32, f32)> = pairs
                .iter()
                .filter_map(|&[x, y]| view_box(&[(x, y)]).map(|p| (p.x0, p.y0)))
                .collect();
            if points.len() < 3 {
                return None;
            }
            Kind::Polygon {
                points,
                cloud: is_cloudy(annot),
            }
        }
        // Only Panna's own word stamps: redrawing another program's, or a picture, would lose
        // its artwork.
        "Stamp"
            if annot
                .get::<hayro::hayro_syntax::object::String<'_>>(b"NM")
                .map(|nm| decode_text_string(nm.as_bytes()))
                .is_some_and(|nm| nm.starts_with("panna-") && !nm.starts_with("panna-image-")) =>
        {
            Kind::Stamp {
                rect: inner(1.0),
                name: annot
                    .get::<Name<'_>>(b"Name")
                    .map_or_else(|| "Draft".to_owned(), |n| n.as_str().to_owned()),
                image: None,
            }
        }
        "Ink" => {
            let mut strokes = Vec::new();
            let mut points = 0;
            for stroke in annot.get::<Array<'_>>(b"InkList")?.iter::<Array<'_>>() {
                let numbers: Vec<f64> = stroke.iter::<f64>().take(40_000).collect();
                let (pairs, _) = numbers.as_chunks::<2>();
                let stroke: Vec<(f32, f32)> = pairs
                    .iter()
                    .filter_map(|&[x, y]| view_box(&[(x, y)]).map(|p| (p.x0, p.y0)))
                    .collect();
                points += stroke.len();
                if points > 100_000 {
                    return None;
                }
                if !stroke.is_empty() {
                    strokes.push(stroke);
                }
            }
            if strokes.is_empty() {
                return None;
            }
            Kind::Ink(strokes)
        }
        _ => return None,
    })
}

/// A border effect of /S /C: a cloudy outline.
fn is_cloudy(annot: &Dict<'_>) -> bool {
    annot
        .get::<Dict<'_>>(b"BE")
        .and_then(|be| be.get::<Name<'_>>(b"S"))
        .is_some_and(|s| s.as_str() == "C")
}

/// The colour in /C: grey, RGB or CMYK.
fn color(annot: &Dict<'_>) -> Color {
    let c: Vec<f32> = annot
        .get::<Array<'_>>(b"C")
        .map(|c| {
            c.iter::<f64>()
                .take(4)
                .map(|v| v.clamp(0.0, 1.0) as f32)
                .collect()
        })
        .unwrap_or_default();
    match c.as_slice() {
        [g] => Color([*g, *g, *g]),
        [r, g, b] => Color([*r, *g, *b]),
        [c, m, y, k] => Color([
            (1.0 - c) * (1.0 - k),
            (1.0 - m) * (1.0 - k),
            (1.0 - y) * (1.0 - k),
        ]),
        _ => Color::BLACK,
    }
}

/// The border width, from /BS or the older /Border array.
fn width(annot: &Dict<'_>) -> f32 {
    annot
        .get::<Dict<'_>>(b"BS")
        .and_then(|bs| bs.get::<f64>(b"W"))
        .or_else(|| {
            annot
                .get::<Array<'_>>(b"Border")
                .and_then(|b| b.iter::<f64>().nth(2))
        })
        .map_or(1.0, |w| w.clamp(0.0, 50.0) as f32)
}

/// The font size in a default appearance string such as `/Helv 12 Tf 0 g`.
fn font_size(da: &[u8]) -> Option<f32> {
    let da = std::str::from_utf8(da).ok()?;
    let words: Vec<&str> = da.split_whitespace().collect();
    let at = words.iter().position(|w| *w == "Tf")?;
    let size: f32 = words.get(at.checked_sub(1)?)?.parse().ok()?;
    (size.is_finite() && size > 0.0).then_some(size.min(144.0))
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
            CommentKind::from_subtype("Caret"),
            Some(CommentKind::Other("Caret".into()))
        );
    }

    #[test]
    fn font_sizes_come_from_the_appearance_string() {
        assert_eq!(font_size(b"/Helv 14 Tf 0 0 1 rg"), Some(14.0));
        assert_eq!(font_size(b"0 g /F1 0 Tf"), None);
        assert_eq!(font_size(b"Tf"), None);
    }
}
