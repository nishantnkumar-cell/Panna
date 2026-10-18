//! Comment files: a document's comments as FDF (PDF syntax) or XFDF (XML), as Acrobat reads
//! and writes them, so they can be shared without the document and added to another copy.
//!
//! Both kinds of file are read into the same plain description of each comment, in the page's
//! own coordinates. Reading one into a document turns each comment into one Panna can draw,
//! through the same code that reads comments saved in PDFs.

use std::fmt::Write as _;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Name, Rect};

use crate::document::decode_text_string;
use crate::edit::crypt::parse_string;
use crate::edit::lex::{Lexer, array_items, dict_entries};
use crate::formdata::DataFormat;

const MAX_COMMENTS: usize = 10_000;
const MAX_NUMBERS: usize = 100_000;
const MAX_TEXT: usize = 32_000;

/// The kinds of comment that travel in comment files: PDF subtype and XFDF element.
const KINDS: [(&str, &str); 14] = [
    ("Text", "text"),
    ("FreeText", "freetext"),
    ("Line", "line"),
    ("Square", "square"),
    ("Circle", "circle"),
    ("Polygon", "polygon"),
    ("PolyLine", "polyline"),
    ("Highlight", "highlight"),
    ("Underline", "underline"),
    ("Squiggly", "squiggly"),
    ("StrikeOut", "strikeout"),
    ("Stamp", "stamp"),
    ("Ink", "ink"),
    ("Caret", "caret"),
];

/// One comment, in its page's PDF coordinates.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Markup {
    subtype: String,
    pub(crate) page: usize,
    rect: [f64; 4],
    color: Vec<f64>,
    opacity: Option<f64>,
    contents: String,
    author: String,
    date: String,
    name: String,
    width: Option<f64>,
    quads: Vec<f64>,
    ink: Vec<Vec<f64>>,
    vertices: Vec<f64>,
    line: Vec<f64>,
    line_ends: Vec<String>,
    icon: Option<String>,
    appearance: Option<String>,
    cloud: bool,
    pub(crate) in_reply_to: Option<String>,
}

fn clip(mut s: String) -> String {
    if let Some((cut, _)) = s.char_indices().nth(MAX_TEXT) {
        s.truncate(cut);
    }
    s
}

fn finite(numbers: impl Iterator<Item = f64>) -> Vec<f64> {
    numbers.filter(|n| n.is_finite()).take(MAX_NUMBERS).collect()
}

// Reading a document's comments.

/// The name a comment goes by in a file: its own, or one made from its object number.
fn name_of(annot: &Dict<'_>) -> String {
    annot
        .get::<hayro::hayro_syntax::object::String<'_>>(b"NM")
        .map(|s| decode_text_string(s.as_bytes()))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            annot
                .obj_id()
                .map(|id| format!("panna-{}-{}", id.obj_number, id.gen_number))
        })
        .unwrap_or_default()
}

/// Every comment in `pdf` that comment files carry, page by page.
pub(crate) fn read(pdf: &Pdf) -> Vec<Markup> {
    let mut out = Vec::new();
    for (index, page) in pdf.pages().iter().enumerate() {
        let Some(annots) = page.raw().get::<Array<'_>>(b"Annots") else {
            continue;
        };
        for annot in annots.iter::<Dict<'_>>().take(MAX_COMMENTS) {
            if out.len() >= MAX_COMMENTS {
                return out;
            }
            let Some(subtype) = annot.get::<Name<'_>>(b"Subtype") else {
                continue;
            };
            let subtype = subtype.as_str();
            if !KINDS.iter().any(|(s, _)| *s == subtype) {
                continue;
            }
            let text = |key: &[u8]| {
                annot
                    .get::<hayro::hayro_syntax::object::String<'_>>(key)
                    .map(|s| clip(decode_text_string(s.as_bytes())))
                    .unwrap_or_default()
            };
            let numbers = |key: &[u8]| {
                annot
                    .get::<Array<'_>>(key)
                    .map(|a| finite(a.iter::<f64>()))
                    .unwrap_or_default()
            };
            let rect = annot.get::<Rect>(b"Rect").unwrap_or(Rect::ZERO);
            let in_reply_to = annot.get::<Dict<'_>>(b"IRT").map(|parent| name_of(&parent));
            out.push(Markup {
                subtype: subtype.to_owned(),
                page: index,
                rect: [rect.x0, rect.y0, rect.x1, rect.y1],
                color: numbers(b"C"),
                opacity: annot.get::<f64>(b"CA"),
                contents: text(b"Contents"),
                author: text(b"T"),
                date: text(b"M"),
                name: name_of(&annot),
                width: annot
                    .get::<Dict<'_>>(b"BS")
                    .and_then(|bs| bs.get::<f64>(b"W"))
                    .or_else(|| {
                        annot
                            .get::<Array<'_>>(b"Border")
                            .and_then(|b| b.iter::<f64>().nth(2))
                    }),
                quads: numbers(b"QuadPoints"),
                ink: annot
                    .get::<Array<'_>>(b"InkList")
                    .map(|list| {
                        list.iter::<Array<'_>>()
                            .take(1_000)
                            .map(|s| finite(s.iter::<f64>()))
                            .collect()
                    })
                    .unwrap_or_default(),
                vertices: numbers(b"Vertices"),
                line: numbers(b"L"),
                line_ends: annot
                    .get::<Array<'_>>(b"LE")
                    .map(|le| {
                        le.iter::<Name<'_>>()
                            .take(2)
                            .map(|n| n.as_str().to_owned())
                            .collect()
                    })
                    .unwrap_or_default(),
                icon: annot.get::<Name<'_>>(b"Name").map(|n| n.as_str().to_owned()),
                appearance: Some(text(b"DA")).filter(|da| !da.is_empty()),
                cloud: annot
                    .get::<Dict<'_>>(b"BE")
                    .and_then(|be| be.get::<Name<'_>>(b"S"))
                    .is_some_and(|s| s.as_str() == "C"),
                in_reply_to,
            });
        }
    }
    out
}

// Writing.

fn n(v: f64) -> String {
    crate::edit::annot::n(v)
}

fn joined(numbers: &[f64], sep: &str) -> String {
    numbers.iter().map(|v| n(*v)).collect::<Vec<_>>().join(sep)
}

/// Points as XFDF writes them: `x,y;x,y`.
fn points(numbers: &[f64]) -> String {
    let (pairs, _) = numbers.as_chunks::<2>();
    pairs
        .iter()
        .map(|[x, y]| format!("{},{}", n(*x), n(*y)))
        .collect::<Vec<_>>()
        .join(";")
}

fn hex_color(c: &[f64]) -> Option<String> {
    let rgb = match c {
        [g] => [*g, *g, *g],
        [r, g, b] => [*r, *g, *b],
        [c, m, y, k] => [
            (1.0 - c) * (1.0 - k),
            (1.0 - m) * (1.0 - k),
            (1.0 - y) * (1.0 - k),
        ],
        _ => return None,
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(format!(
        "#{:02X}{:02X}{:02X}",
        byte(rgb[0]),
        byte(rgb[1]),
        byte(rgb[2])
    ))
}

use crate::formdata::{pdf_name, pdf_string, xml_escape};

impl Markup {
    /// The comment as a PDF annotation dictionary; in an FDF file it also says its page.
    pub(crate) fn pdf_dict(&self, with_page: bool) -> String {
        let mut d = format!("<< /Type /Annot /Subtype {}", pdf_name(&self.subtype));
        if with_page {
            let _ = write!(d, " /Page {}", self.page);
        }
        let _ = write!(d, " /Rect [{}]", joined(&self.rect, " "));
        if !self.color.is_empty() {
            let _ = write!(d, " /C [{}]", joined(&self.color, " "));
        }
        if let Some(o) = self.opacity {
            let _ = write!(d, " /CA {}", n(o.clamp(0.0, 1.0)));
        }
        for (key, value) in [
            ("Contents", &self.contents),
            ("T", &self.author),
            ("M", &self.date),
            ("NM", &self.name),
        ] {
            if !value.is_empty() {
                let _ = write!(d, " /{key} {}", pdf_string(value));
            }
        }
        if let Some(w) = self.width {
            let _ = write!(d, " /BS << /W {} >>", n(w.clamp(0.0, 50.0)));
        }
        for (key, value) in [
            ("QuadPoints", &self.quads),
            ("Vertices", &self.vertices),
            ("L", &self.line),
        ] {
            if !value.is_empty() {
                let _ = write!(d, " /{key} [{}]", joined(value, " "));
            }
        }
        if !self.ink.is_empty() {
            d.push_str(" /InkList [");
            for stroke in &self.ink {
                let _ = write!(d, "[{}]", joined(stroke, " "));
            }
            d.push(']');
        }
        if !self.line_ends.is_empty() {
            let ends: Vec<String> = self.line_ends.iter().map(|e| pdf_name(e)).collect();
            let _ = write!(d, " /LE [{}]", ends.join(" "));
        }
        if let Some(icon) = &self.icon {
            let _ = write!(d, " /Name {}", pdf_name(icon));
        }
        if let Some(da) = &self.appearance {
            let _ = write!(d, " /DA {}", pdf_string(da));
        }
        if self.cloud {
            d.push_str(" /BE << /S /C /I 1 >>");
        }
        d.push_str(" >>");
        d
    }

    /// The comment as an annotation to read back in. Stamps get a name that marks them as
    /// word stamps Panna can draw.
    pub(crate) fn pdf_dict_for_import(&self, index: usize) -> String {
        let mut m = self.clone();
        if m.subtype == "Stamp" {
            m.name = format!("panna-import-{index}");
        }
        m.pdf_dict(false)
    }

    fn xfdf(&self, out: &mut String) {
        let Some((_, tag)) = KINDS.iter().find(|(s, _)| *s == self.subtype) else {
            return;
        };
        let _ = write!(
            out,
            "<{tag} page=\"{}\" rect=\"{}\"",
            self.page,
            joined(&self.rect, ",")
        );
        let mut attr = |key: &str, value: &str| {
            if !value.is_empty() {
                let _ = write!(out, " {key}=\"{}\"", xml_escape(value));
            }
        };
        attr("color", &hex_color(&self.color).unwrap_or_default());
        attr("opacity", &self.opacity.map(n).unwrap_or_default());
        attr("title", &self.author);
        attr("name", &self.name);
        attr("date", &self.date);
        attr("width", &self.width.map(n).unwrap_or_default());
        attr("coords", &joined(&self.quads, ","));
        attr("vertices", &points(&self.vertices));
        if let [x0, y0, x1, y1, ..] = self.line.as_slice() {
            attr("start", &format!("{},{}", n(*x0), n(*y0)));
            attr("end", &format!("{},{}", n(*x1), n(*y1)));
        }
        attr("head", self.line_ends.first().map_or("", String::as_str));
        attr("tail", self.line_ends.get(1).map_or("", String::as_str));
        attr("icon", self.icon.as_deref().unwrap_or_default());
        attr("inreplyto", self.in_reply_to.as_deref().unwrap_or_default());
        if self.cloud {
            attr("style", "cloudy");
            attr("intensity", "1");
        }
        out.push('>');
        if !self.contents.is_empty() {
            let _ = write!(out, "<contents>{}</contents>", xml_escape(&self.contents));
        }
        if let Some(da) = &self.appearance {
            let _ = write!(out, "<defaultappearance>{}</defaultappearance>", xml_escape(da));
        }
        if !self.ink.is_empty() {
            out.push_str("<inklist>");
            for stroke in &self.ink {
                let _ = write!(out, "<gesture>{}</gesture>", points(stroke));
            }
            out.push_str("</inklist>");
        }
        let _ = writeln!(out, "</{tag}>");
    }
}

/// A comment file holding `comments`, for the PDF named `pdf_name`.
pub(crate) fn export(comments: &[Markup], format: DataFormat, pdf_name: &str) -> Vec<u8> {
    match format {
        DataFormat::Fdf => {
            let mut out = String::from("1 0 obj\n<< /FDF << /Annots [\n");
            // Replies point at their comment by reference, which an FDF file can't keep.
            for c in comments.iter().filter(|c| c.in_reply_to.is_none()) {
                out.push_str(&c.pdf_dict(true));
                out.push('\n');
            }
            let _ = write!(
                out,
                "] /F {} >> >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n",
                pdf_string(pdf_name)
            );
            let mut bytes = b"%FDF-1.2\n%\xe2\xe3\xcf\xd3\n".to_vec();
            bytes.extend_from_slice(out.as_bytes());
            bytes
        }
        DataFormat::Xfdf => {
            let mut out = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">\n\
                 <f href=\"{}\"/>\n<annots>\n",
                xml_escape(pdf_name)
            );
            for c in comments {
                c.xfdf(&mut out);
            }
            out.push_str("</annots>\n</xfdf>\n");
            out.into_bytes()
        }
    }
}

// Reading comment files.

/// The comments in an FDF or XFDF file.
pub(crate) fn import(bytes: &[u8]) -> Result<Vec<Markup>, String> {
    let start = bytes.trim_ascii_start();
    if start.starts_with(b"%FDF") {
        fdf_import(bytes).ok_or_else(|| "The FDF file is damaged.".into())
    } else if start.starts_with(b"<") || start.starts_with(b"\xef\xbb\xbf") {
        xfdf_import(bytes)
    } else {
        Err("This isn't an FDF or XFDF file.".into())
    }
}

fn numbers_in(raw: &[u8]) -> Vec<f64> {
    let raw = raw.trim_ascii();
    let inner = raw
        .strip_prefix(b"[")
        .and_then(|r| r.strip_suffix(b"]"))
        .unwrap_or(raw);
    finite(
        inner
            .split(|b| b.is_ascii_whitespace() || *b == b'[' || *b == b']')
            .filter_map(|w| std::str::from_utf8(w).ok())
            .filter_map(|w| w.parse::<f64>().ok()),
    )
}

fn fdf_import(bytes: &[u8]) -> Option<Vec<Markup>> {
    let at = bytes.windows(7).position(|w| w == b"/Annots")? + 7;
    let mut lex = Lexer::new(bytes, at);
    lex.skip_trivia();
    let span = lex.value()?;
    let array = bytes.get(span)?.trim_ascii();
    let inner = array.strip_prefix(b"[")?.strip_suffix(b"]")?;
    let mut out = Vec::new();
    for item in array_items(inner)?.into_iter().take(MAX_COMMENTS) {
        // Comments given by reference aren't followed.
        let Some(dict) = inner.get(item) else { continue };
        if let Some(markup) = fdf_markup(dict) {
            out.push(markup);
        }
    }
    Some(out)
}

fn fdf_markup(dict: &[u8]) -> Option<Markup> {
    let entries = dict_entries(dict)?;
    let get = |key: &[u8]| {
        entries
            .iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| dict.get(v.clone()))
            .map(<[u8]>::trim_ascii)
    };
    let name = |raw: &[u8]| raw.strip_prefix(b"/").map(crate::formdata::name_text);
    let text = |key: &[u8]| {
        get(key)
            .and_then(parse_string)
            .map(|s| clip(decode_text_string(&s)))
            .unwrap_or_default()
    };
    let subtype = name(get(b"Subtype")?)?;
    let sub = |key: &[u8], inner: &[u8]| -> Option<Vec<u8>> {
        let d = get(key)?;
        let e = dict_entries(d)?;
        e.iter()
            .find(|(k, _)| *k == inner)
            .and_then(|(_, v)| d.get(v.clone()))
            .map(|v| v.trim_ascii().to_vec())
    };
    let rect = numbers_in(get(b"Rect")?);
    let [x0, y0, x1, y1] = rect.get(..4)? else {
        return None;
    };
    Some(Markup {
        subtype,
        page: std::str::from_utf8(get(b"Page")?).ok()?.parse().ok()?,
        rect: [*x0, *y0, *x1, *y1],
        color: get(b"C").map(numbers_in).unwrap_or_default(),
        opacity: get(b"CA").and_then(|v| std::str::from_utf8(v).ok()?.parse().ok()),
        contents: text(b"Contents"),
        author: text(b"T"),
        date: text(b"M"),
        name: text(b"NM"),
        width: sub(b"BS", b"W")
            .and_then(|w| std::str::from_utf8(&w).ok()?.parse().ok())
            .or_else(|| get(b"Border").and_then(|b| numbers_in(b).get(2).copied())),
        quads: get(b"QuadPoints").map(numbers_in).unwrap_or_default(),
        ink: get(b"InkList")
            .and_then(|list| {
                let inner = list.strip_prefix(b"[")?.strip_suffix(b"]")?;
                Some(
                    array_items(inner)?
                        .into_iter()
                        .take(1_000)
                        .filter_map(|s| inner.get(s).map(numbers_in))
                        .collect(),
                )
            })
            .unwrap_or_default(),
        vertices: get(b"Vertices").map(numbers_in).unwrap_or_default(),
        line: get(b"L").map(numbers_in).unwrap_or_default(),
        line_ends: get(b"LE")
            .and_then(|le| le.strip_prefix(b"[")?.strip_suffix(b"]"))
            .map(|le| {
                le.split(u8::is_ascii_whitespace)
                    .filter_map(name)
                    .take(2)
                    .collect()
            })
            .unwrap_or_default(),
        icon: get(b"Name").and_then(name),
        appearance: Some(text(b"DA")).filter(|da| !da.is_empty()),
        cloud: sub(b"BE", b"S").is_some_and(|s| s == b"/C"),
        in_reply_to: get(b"IRT").map(|_| String::new()),
    })
}

/// Numbers written `a,b,c` or `x,y;x,y`.
fn list(text: &str) -> Vec<f64> {
    finite(
        text.split([',', ';', ' '])
            .filter(|w| !w.is_empty())
            .filter_map(|w| w.trim().parse::<f64>().ok()),
    )
}

/// A colour written `#RRGGBB`.
fn color_from_hex(text: &str) -> Vec<f64> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return Vec::new();
    }
    (0..3)
        .filter_map(|i| hex.get(i * 2..i * 2 + 2))
        .filter_map(|h| u8::from_str_radix(h, 16).ok())
        .map(|b| f64::from(b) / 255.0)
        .collect()
}

fn xfdf_import(bytes: &[u8]) -> Result<Vec<Markup>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "The XFDF file isn't UTF-8 text.".to_owned())?;
    let doc = roxmltree::Document::parse_with_options(
        text.trim_start_matches('\u{feff}'),
        roxmltree::ParsingOptions {
            allow_dtd: false,
            ..roxmltree::ParsingOptions::default()
        },
    )
    .map_err(|e| format!("The XFDF file is damaged: {e}."))?;
    let root = doc.root_element();
    if root.tag_name().name() != "xfdf" {
        return Err("This isn't an XFDF file.".into());
    }
    let mut out = Vec::new();
    for annots in root.children().filter(|n| n.tag_name().name() == "annots") {
        for el in annots.children().filter(roxmltree::Node::is_element) {
            if out.len() >= MAX_COMMENTS {
                return Ok(out);
            }
            let Some((subtype, _)) = KINDS.iter().find(|(_, t)| *t == el.tag_name().name()) else {
                continue;
            };
            let attr = |key: &str| el.attribute(key).unwrap_or_default();
            let child = |name: &str| {
                el.children().find(|c| c.tag_name().name() == name).map(|c| {
                    clip(
                        c.descendants()
                            .filter(roxmltree::Node::is_text)
                            .filter_map(|d| d.text())
                            .collect::<String>(),
                    )
                })
            };
            let rect = list(attr("rect"));
            let [x0, y0, x1, y1] = rect.get(..4).unwrap_or(&[0.0; 4]) else {
                continue;
            };
            let number = |key: &str| attr(key).trim().parse::<f64>().ok().filter(|v| v.is_finite());
            let mut line = list(attr("start"));
            line.extend(list(attr("end")));
            let line_ends: Vec<String> = [attr("head"), attr("tail")]
                .into_iter()
                .filter(|e| !e.is_empty())
                .map(str::to_owned)
                .collect();
            let Ok(page) = attr("page").trim().parse::<usize>() else {
                continue;
            };
            out.push(Markup {
                subtype: (*subtype).to_owned(),
                page,
                rect: [*x0, *y0, *x1, *y1],
                color: color_from_hex(attr("color")),
                opacity: number("opacity"),
                contents: child("contents")
                    .or_else(|| child("contents-richtext"))
                    .unwrap_or_default(),
                author: clip(attr("title").to_owned()),
                date: attr("date").to_owned(),
                name: attr("name").to_owned(),
                width: number("width"),
                quads: list(attr("coords")),
                ink: el
                    .children()
                    .filter(|c| c.tag_name().name() == "inklist")
                    .flat_map(|l| l.children().filter(|g| g.tag_name().name() == "gesture"))
                    .take(1_000)
                    .map(|g| list(g.text().unwrap_or_default()))
                    .collect(),
                vertices: list(attr("vertices")),
                line,
                line_ends,
                icon: Some(attr("icon").to_owned()).filter(|i| !i.is_empty()),
                appearance: child("defaultappearance").filter(|d| !d.is_empty()),
                cloud: attr("style") == "cloudy",
                in_reply_to: Some(attr("inreplyto").to_owned()).filter(|r| !r.is_empty()),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Markup> {
        vec![
            Markup {
                subtype: "Highlight".into(),
                page: 0,
                rect: [10.0, 10.0, 60.0, 20.0],
                color: vec![1.0, 1.0, 0.0],
                opacity: Some(0.5),
                contents: "Check <this> & that".into(),
                author: "Jo".into(),
                name: "a1".into(),
                quads: vec![10.0, 20.0, 60.0, 20.0, 10.0, 10.0, 60.0, 10.0],
                ..Markup::default()
            },
            Markup {
                subtype: "Ink".into(),
                page: 1,
                rect: [0.0, 0.0, 30.0, 30.0],
                color: vec![0.0, 0.0, 1.0],
                width: Some(2.0),
                ink: vec![vec![1.0, 2.0, 3.0, 4.0], vec![5.0, 6.0, 7.0, 8.0]],
                name: "a2".into(),
                ..Markup::default()
            },
            Markup {
                subtype: "Line".into(),
                page: 0,
                rect: [0.0, 0.0, 100.0, 100.0],
                color: vec![1.0, 0.0, 0.0],
                line: vec![1.0, 2.0, 99.0, 98.0],
                line_ends: vec!["None".into(), "OpenArrow".into()],
                contents: "पन्ना".into(),
                name: "a3".into(),
                ..Markup::default()
            },
            Markup {
                subtype: "Polygon".into(),
                page: 0,
                rect: [0.0, 0.0, 50.0, 50.0],
                color: vec![0.0, 0.0, 0.0],
                vertices: vec![0.0, 0.0, 50.0, 0.0, 25.0, 50.0],
                cloud: true,
                name: "a4".into(),
                ..Markup::default()
            },
        ]
    }

    #[test]
    fn comments_round_trip_through_both_formats() {
        for format in [DataFormat::Fdf, DataFormat::Xfdf] {
            let file = export(&sample(), format, "doc.pdf");
            let read = import(&file).unwrap();
            assert_eq!(read, sample(), "{format:?}");
        }
    }

    #[test]
    fn replies_only_travel_in_xfdf() {
        let mut comments = sample();
        comments.push(Markup {
            subtype: "Text".into(),
            in_reply_to: Some("a1".into()),
            name: "r1".into(),
            ..Markup::default()
        });
        assert_eq!(import(&export(&comments, DataFormat::Fdf, "")).unwrap().len(), 4);
        let xfdf = import(&export(&comments, DataFormat::Xfdf, "")).unwrap();
        assert_eq!(
            xfdf.last().and_then(|m| m.in_reply_to.clone()).as_deref(),
            Some("a1")
        );
    }
}
