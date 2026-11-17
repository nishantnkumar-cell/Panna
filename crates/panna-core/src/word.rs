//! Saving pages as a Word document (.docx) that keeps their layout. Each page is a section
//! of its own, with the page's size and margins. Paragraphs keep their indent, alignment,
//! type size, line spacing and the space above them. Lines set in columns become a table.
//! Pictures are cut out of the page, drawn at 150 dpi, and placed where they come in the
//! text. Fonts, colours and text wrapped around pictures aren't kept.

use std::fmt::Write as _;

use crate::office::{self, Paragraph, half_points, xml};
use crate::text::{Line, TextBox};
use crate::{Document, SaveError};

/// Resolution pictures are cut out at.
const DPI: f32 = 150.0;
/// Pictures in one document, at most; later ones are left out.
const MAX_PICTURES: usize = 2_000;
/// Pictures smaller than this across or down, in points, are left out: rules, bullets and
/// specks rather than pictures.
const MIN_PICTURE: f32 = 12.0;
/// Word's largest page side, in points (22 inches).
const MAX_PAGE: f32 = 1584.0;
/// Word's single line spacing, as a share of the type size, for the usual fonts.
const SINGLE_LINE: f32 = 1.17;

/// Points as twentieths of a point, Word's unit for lengths.
fn twips(points: f32) -> i64 {
    if points.is_finite() {
        (points * 20.0).round().clamp(-31_680.0, 31_680.0) as i64
    } else {
        0
    }
}

/// Points as English Metric Units, DrawingML's unit for lengths.
fn emu(points: f32) -> i64 {
    if points.is_finite() {
        (f64::from(points) * 12_700.0).round().clamp(0.0, 1e12) as i64
    } else {
        0
    }
}

/// A table: its column edges, right edge and rows of cells (see [`office::table`]).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Table {
    pub(crate) columns: Vec<f32>,
    pub(crate) right: f32,
    pub(crate) y0: f32,
    pub(crate) y1: f32,
    pub(crate) size: f32,
    pub(crate) rows: Vec<Vec<(usize, String)>>,
}

/// Something on a page, in the order it is read.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Item {
    Text(Paragraph),
    Table(Table),
    /// A picture's place and its number among the document's pictures.
    Picture(TextBox, usize),
}

impl Item {
    fn top(&self) -> f32 {
        match self {
            Item::Text(p) => p.y0,
            Item::Table(t) => t.y0,
            Item::Picture(b, _) => b.y0,
        }
    }

    fn bottom(&self) -> f32 {
        match self {
            Item::Text(p) => p.y1,
            Item::Table(t) => t.y1,
            Item::Picture(b, _) => b.y1,
        }
    }

    fn left(&self) -> f32 {
        match self {
            Item::Text(p) => p.x0,
            Item::Table(t) => t.columns.first().copied().unwrap_or(t.right),
            Item::Picture(b, _) => b.x0,
        }
    }

    fn right(&self) -> f32 {
        match self {
            Item::Text(p) => p.x1,
            Item::Table(t) => t.right,
            Item::Picture(b, _) => b.x1,
        }
    }
}

/// Whether `run`, lines that follow each other, reads as a table rather than text set in
/// columns: at least two of its lines are cut into cells, they make two columns or more,
/// and with only two columns the cells are short.
fn is_table(run: &[Line]) -> bool {
    if run.iter().filter(|l| l.cells.len() >= 2).count() < 2 {
        return false;
    }
    let columns = office::columns(run).len();
    let cells: usize = run.iter().map(|l| l.cells.len()).sum();
    let chars: usize = run
        .iter()
        .flat_map(|l| &l.cells)
        .map(|c| c.1.chars().count())
        .sum();
    columns >= 3 || (columns == 2 && chars <= cells * 30)
}

/// A page's lines as paragraphs and tables, in the order read.
pub(crate) fn items(lines: &[Line]) -> Vec<Item> {
    let mut out = Vec::new();
    let flush = |out: &mut Vec<Item>, from: usize, to: usize| {
        let text = lines.get(from..to).unwrap_or_default();
        out.extend(office::paragraphs(text).into_iter().map(Item::Text));
    };
    let mut text_from = 0;
    let mut i = 0;
    while let Some(first) = lines.get(i) {
        if first.cells.len() < 2 {
            i += 1;
            continue;
        }
        // A table takes the lines close below that are cut into cells too, and a line that
        // isn't but sits between two that are.
        let mut end = i + 1;
        while let (Some(prev), Some(line)) = (lines.get(end - 1), lines.get(end)) {
            let close = line.y0 >= prev.y0 && line.y0 - prev.y1 < prev.size.max(line.size) * 2.0;
            let cut = line.cells.len() >= 2 || lines.get(end + 1).is_some_and(|n| n.cells.len() >= 2);
            if !(close && cut) {
                break;
            }
            end += 1;
        }
        let run = lines.get(i..end).unwrap_or_default();
        if !is_table(run) {
            i += 1;
            continue;
        }
        flush(&mut out, text_from, i);
        let columns = office::columns(run);
        let fold =
            |f: fn(&Line) -> f32, pick: fn(f32, f32) -> f32| run.iter().map(f).reduce(pick).unwrap_or(0.0);
        let mut sizes: Vec<f32> = run.iter().map(|l| l.size).collect();
        sizes.sort_by(f32::total_cmp);
        out.push(Item::Table(Table {
            columns,
            right: fold(|l| l.x1, f32::max),
            y0: fold(|l| l.y0, f32::min),
            y1: fold(|l| l.y1, f32::max),
            size: sizes.get(sizes.len() / 2).copied().unwrap_or(10.0),
            rows: office::table(run),
        }));
        i = end;
        text_from = end;
    }
    flush(&mut out, text_from, lines.len());
    out
}

/// The pictures worth keeping from where a page's pictures are drawn: on the page, not too
/// small, each once, and not a background or scan behind the page's text.
pub(crate) fn pictures(drawn: &[TextBox], width: f32, height: f32, has_text: bool) -> Vec<TextBox> {
    let mut out: Vec<TextBox> = Vec::new();
    for b in drawn {
        let b = TextBox {
            x0: b.x0.max(0.0),
            y0: b.y0.max(0.0),
            x1: b.x1.min(width),
            y1: b.y1.min(height),
        };
        let fits = b.width() >= MIN_PICTURE && b.height() >= MIN_PICTURE;
        let backdrop = has_text && b.width() * b.height() > width * height * 0.6;
        let seen = out.iter().any(|o| {
            (o.x0 - b.x0).abs() < 1.0
                && (o.y0 - b.y0).abs() < 1.0
                && (o.x1 - b.x1).abs() < 1.0
                && (o.y1 - b.y1).abs() < 1.0
        });
        if fits && !backdrop && !seen {
            out.push(b);
        }
    }
    out
}

/// One page, laid out.
struct Page {
    width: f32,
    height: f32,
    /// Top, right, bottom and left.
    margins: [f32; 4],
    items: Vec<Item>,
}

impl Page {
    fn new(width: f32, height: f32, items: Vec<Item>) -> Self {
        let width = width.clamp(72.0, MAX_PAGE);
        let height = height.clamp(72.0, MAX_PAGE);
        let fold = |f: fn(&Item) -> f32, pick: fn(f32, f32) -> f32| items.iter().map(f).reduce(pick);
        let side =
            |gap: Option<f32>, page: f32| gap.map_or(72.0_f32.min(page / 4.0), |g| g.clamp(18.0, page / 4.0));
        let left = side(fold(Item::left, f32::min), width);
        // Most pages have the same margin on both sides; the right one is never wider than
        // the left, so a page whose lines stop short isn't made narrower.
        let right = side(fold(Item::right, f32::max).map(|r| width - r), width).min(left);
        let margins = [
            side(fold(Item::top, f32::min), height),
            right,
            side(fold(Item::bottom, f32::max).map(|b| height - b), height),
            left,
        ];
        Page {
            width,
            height,
            margins,
            items,
        }
    }

    fn section(&self) -> String {
        let [top, right, bottom, left] = self.margins;
        let orient = if self.width > self.height {
            " w:orient=\"landscape\""
        } else {
            ""
        };
        format!(
            "<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"{orient}/><w:pgMar w:top=\"{}\" w:right=\"{}\" \
             w:bottom=\"{}\" w:left=\"{}\" w:header=\"0\" w:footer=\"0\" w:gutter=\"0\"/></w:sectPr>",
            twips(self.width),
            twips(self.height),
            twips(top),
            twips(right),
            twips(bottom),
            twips(left)
        )
    }

    /// The text column's left and right edges.
    fn column(&self) -> (f32, f32) {
        (self.margins[3], self.width - self.margins[1])
    }
}

/// A paragraph's properties: its style, the space above it, its indent and alignment, and
/// the section it ends, if any.
fn paragraph_props(p: &Paragraph, page: &Page, before: f32, section: &str) -> String {
    let (left, right) = page.column();
    let style = match p.level {
        1 => "<w:pStyle w:val=\"Heading1\"/>",
        2 => "<w:pStyle w:val=\"Heading2\"/>",
        _ => "",
    };
    let mut line = String::new();
    if p.lines >= 2 {
        let pitch = (p.y1 - p.y0 - p.size) / (p.lines - 1) as f32;
        if pitch >= p.size {
            line = format!(" w:line=\"{}\" w:lineRule=\"exact\"", twips(pitch));
        }
    }
    let gap_left = p.x0 - left;
    let gap_right = right - p.x1;
    let size = p.size.max(6.0);
    let middle = (p.x0 + p.x1) / 2.0 - (left + right) / 2.0;
    let (jc, indent) = if gap_left > size * 2.0 && middle.abs() <= size {
        ("<w:jc w:val=\"center\"/>", String::new())
    } else if gap_right <= size * 0.5 && gap_left > (right - left) * 0.3 {
        ("<w:jc w:val=\"right\"/>", String::new())
    } else {
        let first = p.first_x - p.x0;
        let first = if first > size * 0.3 {
            format!(" w:firstLine=\"{}\"", twips(first))
        } else {
            String::new()
        };
        let indent = if gap_left > 1.0 || !first.is_empty() {
            format!("<w:ind w:left=\"{}\"{first}/>", twips(gap_left.max(0.0)))
        } else {
            String::new()
        };
        ("", indent)
    };
    format!(
        "<w:pPr>{style}<w:spacing w:before=\"{}\" w:after=\"0\"{line}/>{indent}{jc}{section}</w:pPr>",
        twips(before)
    )
}

/// How much taller Word makes `item` than it is on the page, in points.
fn taller(item: &Item) -> f32 {
    match item {
        Item::Text(p) => {
            let pitch = if p.lines >= 2 {
                ((p.y1 - p.y0 - p.size) / (p.lines - 1) as f32).max(p.size * SINGLE_LINE)
            } else {
                p.size * SINGLE_LINE
            };
            (pitch * p.lines as f32 - (p.y1 - p.y0)).max(0.0)
        }
        Item::Table(t) => (t.size * SINGLE_LINE * t.rows.len() as f32 - (t.y1 - t.y0)).max(0.0),
        Item::Picture(..) => 0.0,
    }
}

fn run(text: &str, size: f32) -> String {
    let sz = half_points(size);
    format!(
        "<w:r><w:rPr><w:sz w:val=\"{sz}\"/><w:szCs w:val=\"{sz}\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r>",
        xml(text)
    )
}

fn table_xml(t: &Table, page: &Page, out: &mut String) {
    let (left, right) = page.column();
    let Some(&first) = t.columns.first() else { return };
    let mut widths: Vec<f32> = t
        .columns
        .iter()
        // The last column runs to the table's right edge, with room for the text to come
        // out a little wider in Word's font.
        .zip(t.columns.iter().skip(1).copied().chain([t.right + t.size * 2.0]))
        .map(|(a, b)| (b - a).max(18.0))
        .collect();
    let indent = (first - left).max(0.0);
    let total: f32 = widths.iter().sum();
    let room = (right - left - indent).max(72.0);
    if total > room {
        widths.iter_mut().for_each(|w| *w *= room / total);
    }
    let border =
        |side: &str| format!("<w:{side} w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>");
    let _ = write!(
        out,
        "<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblInd w:w=\"{}\" w:type=\"dxa\"/><w:tblBorders>{}{}{}{}{}{}</w:tblBorders>\
         <w:tblLayout w:type=\"fixed\"/><w:tblCellMar><w:left w:w=\"57\" w:type=\"dxa\"/><w:right w:w=\"57\" w:type=\"dxa\"/></w:tblCellMar>\
         </w:tblPr><w:tblGrid>",
        twips(indent),
        border("top"),
        border("left"),
        border("bottom"),
        border("right"),
        border("insideH"),
        border("insideV")
    );
    for w in &widths {
        let _ = write!(out, "<w:gridCol w:w=\"{}\"/>", twips(*w));
    }
    out.push_str("</w:tblGrid>");
    for row in &t.rows {
        if row.is_empty() {
            continue;
        }
        out.push_str("<w:tr>");
        for (col, w) in widths.iter().enumerate() {
            let text = row
                .iter()
                .find(|(c, _)| *c == col)
                .map_or("", |(_, text)| text.as_str());
            let _ = write!(
                out,
                "<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/></w:tcPr><w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\"/></w:pPr>{}</w:p></w:tc>",
                twips(*w),
                if text.is_empty() {
                    String::new()
                } else {
                    run(text, t.size)
                }
            );
        }
        out.push_str("</w:tr>");
    }
    out.push_str("</w:tbl>");
}

fn picture_xml(b: &TextBox, n: usize, page: &Page, before: f32, section: &str, out: &mut String) {
    let (left, right) = page.column();
    let room_x = (right - left).max(36.0);
    let room_y = (page.height - page.margins[0] - page.margins[2]).max(36.0);
    let fit = (room_x / b.width()).min(room_y / b.height()).min(1.0);
    let (cx, cy) = (emu(b.width() * fit), emu(b.height() * fit));
    let indent = if fit < 1.0 {
        0.0
    } else {
        (b.x0 - left).clamp(0.0, room_x - b.width())
    };
    let id = n + 1;
    let _ = write!(
        out,
        "<w:p><w:pPr><w:spacing w:before=\"{}\" w:after=\"0\"/><w:ind w:left=\"{}\"/>{section}</w:pPr><w:r><w:drawing>\
         <wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
         <wp:docPr id=\"{id}\" name=\"Picture {id}\"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr>\
         <a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic>\
         <pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"image{id}.png\"/><pic:cNvPicPr/></pic:nvPicPr>\
         <pic:blipFill><a:blip r:embed=\"rIdPic{id}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
         <pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr>\
         </pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>",
        twips(before),
        twips(indent)
    );
}

fn document_xml(pages: &[Page]) -> String {
    let mut body = String::new();
    for (n, page) in pages.iter().enumerate() {
        let last_page = n + 1 == pages.len();
        // A page ends its section in its last paragraph; the last page's section is the
        // body's own.
        let section = if last_page { String::new() } else { page.section() };
        let mut bottom = page.margins[0];
        let mut extra = 0.0;
        let count = page.items.len();
        for (i, item) in page.items.iter().enumerate() {
            let before = (item.top() - bottom - extra).clamp(0.0, 144.0);
            let ends = if i + 1 == count { section.as_str() } else { "" };
            match item {
                Item::Text(p) => {
                    let _ = write!(
                        body,
                        "<w:p>{}{}</w:p>",
                        paragraph_props(p, page, before, ends),
                        run(&p.text, p.size)
                    );
                }
                Item::Picture(b, pic) => picture_xml(b, *pic, page, before, ends, &mut body),
                Item::Table(t) => {
                    if before >= 1.0 {
                        let _ = write!(
                            body,
                            "<w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"{}\" w:lineRule=\"exact\"/></w:pPr></w:p>",
                            twips(before)
                        );
                    }
                    table_xml(t, page, &mut body);
                }
            }
            bottom = item.bottom();
            extra = taller(item);
        }
        // A section, and the document, end with a paragraph.
        if page.items.last().is_none_or(|i| matches!(i, Item::Table(_))) {
            let _ = write!(
                body,
                "<w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"20\" w:lineRule=\"exact\"/>{section}</w:pPr></w:p>"
            );
        }
        if last_page {
            body.push_str(&page.section());
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document \
         xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
         xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
         xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
         xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:body>{body}</w:body></w:document>"
    )
}

/// The part of `page` inside `b` (in points, at `scale` pixels a point) as a PNG.
fn cut(page: &crate::RenderedPage, b: &TextBox, scale: f32) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    let px = |v: f32, max: u32| ((v * scale).round().max(0.0) as u32).min(max);
    let (x0, x1) = (px(b.x0, page.width), px(b.x1, page.width));
    let (y0, y1) = (px(b.y0, page.height), px(b.y1, page.height));
    if x1 <= x0 + 1 || y1 <= y0 + 1 {
        return None;
    }
    let stride = page.width as usize * 4;
    let mut rgba = Vec::with_capacity((x1 - x0) as usize * (y1 - y0) as usize * 4);
    for y in y0..y1 {
        let start = y as usize * stride + x0 as usize * 4;
        rgba.extend_from_slice(page.rgba.get(start..start + (x1 - x0) as usize * 4)?);
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&rgba, x1 - x0, y1 - y0, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(out)
}

/// `pages` of `doc` as a Word document that keeps their layout.
pub(crate) fn docx<'a>(
    doc: &'a Document,
    cache: &hayro::RenderCache<'a>,
    pages: &[usize],
) -> Result<Vec<u8>, SaveError> {
    let mut laid: Vec<Page> = Vec::with_capacity(pages.len());
    let mut media: Vec<Vec<u8>> = Vec::new();
    for &index in pages {
        let text = doc.page_text(index).map_err(|_| SaveError::NoSuchPage(index))?;
        let size = doc.page_size(index).ok_or(SaveError::NoSuchPage(index))?;
        let lines = text.lines();
        let mut items = items(&lines);
        let found = pictures(text.pictures(), size.width, size.height, !lines.is_empty());
        let room = MAX_PICTURES.saturating_sub(media.len());
        if !found.is_empty() && room > 0 {
            // A picture that can't be drawn is left out, not the whole document.
            if let Ok(drawn) = doc.render_page(cache, index, DPI / 72.0) {
                let scale = drawn.width as f32 / size.width.max(1.0);
                for b in found.iter().take(room) {
                    let Some(png) = cut(&drawn, b, scale) else {
                        continue;
                    };
                    let at = items
                        .iter()
                        .position(|i| i.top() > b.y0 + 1.0)
                        .unwrap_or(items.len());
                    items.insert(at, Item::Picture(*b, media.len()));
                    media.push(png);
                }
            }
        }
        laid.push(Page::new(size.width, size.height, items));
    }
    let body = office::body_size(laid.iter().flat_map(|p| &p.items).filter_map(|i| match i {
        Item::Text(p) => Some(p),
        _ => None,
    }));
    for item in laid.iter_mut().flat_map(|p| &mut p.items) {
        if let Item::Text(p) = item {
            p.level = office::level(p, body);
        }
    }
    let document = document_xml(&laid);
    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>",
    );
    let names: Vec<String> = (1..=media.len())
        .map(|n| format!("word/media/image{n}.png"))
        .collect();
    for n in 1..=media.len() {
        let _ = write!(
            rels,
            "<Relationship Id=\"rIdPic{n}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/image{n}.png\"/>"
        );
    }
    rels.push_str("</Relationships>");
    let mut files: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", office::RELS.as_bytes()),
        ("word/document.xml", document.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/styles.xml", office::STYLES.as_bytes()),
    ];
    files.extend(
        names
            .iter()
            .map(String::as_str)
            .zip(media.iter().map(Vec::as_slice)),
    );
    office::zip(&files)
}

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Default Extension=\"png\" ContentType=\"image/png\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
</Types>";

#[cfg(test)]
mod tests {
    use super::*;

    fn line(cells: &[(f32, &str)], y0: f32) -> Line {
        let text = cells.iter().map(|c| c.1).collect::<Vec<_>>().join("   ");
        let x0 = cells.first().map_or(72.0, |c| c.0);
        Line {
            cells: cells.iter().map(|(x, t)| (*x, (*t).to_owned())).collect(),
            text,
            x0,
            x1: cells.last().map_or(300.0, |c| c.0 + 40.0),
            y0,
            y1: y0 + 10.0,
            size: 10.0,
        }
    }

    #[test]
    fn lines_in_columns_make_a_table() {
        let lines = [
            line(&[(72.0, "A heading for the table")], 50.0),
            line(&[(72.0, "Item"), (200.0, "Qty"), (300.0, "Price")], 80.0),
            line(&[(72.0, "Apples"), (200.0, "3"), (300.0, "1.20")], 93.0),
            line(&[(72.0, "Pears"), (300.0, "0.80")], 106.0),
            line(&[(72.0, "After the table.")], 140.0),
        ];
        let items = items(&lines);
        assert_eq!(items.len(), 3, "{items:?}");
        let Some(Item::Table(t)) = items.get(1) else {
            panic!("{items:?}")
        };
        assert_eq!(t.columns, [72.0, 200.0, 300.0]);
        assert_eq!(t.rows.len(), 3);
        assert_eq!(
            t.rows.get(2),
            Some(&vec![(0, "Pears".to_owned()), (2, "0.80".to_owned())])
        );
        assert!(matches!(items.first(), Some(Item::Text(p)) if p.text == "A heading for the table"));
    }

    #[test]
    fn two_columns_of_prose_stay_text() {
        let prose = "a long line of running text that goes on and on";
        let lines = [
            line(&[(72.0, prose), (320.0, prose)], 80.0),
            line(&[(72.0, prose), (320.0, prose)], 93.0),
        ];
        assert!(items(&lines).iter().all(|i| matches!(i, Item::Text(_))));
    }

    #[test]
    fn backgrounds_specks_and_repeats_are_not_pictures() {
        let b = |x0, y0, x1, y1| TextBox { x0, y0, x1, y1 };
        let drawn = [
            b(0.0, 0.0, 612.0, 792.0),
            b(100.0, 100.0, 300.0, 250.0),
            b(100.2, 100.0, 300.0, 250.0),
            b(50.0, 50.0, 55.0, 55.0),
            b(500.0, 700.0, 700.0, 900.0),
        ];
        assert_eq!(
            pictures(&drawn, 612.0, 792.0, true),
            [b(100.0, 100.0, 300.0, 250.0), b(500.0, 700.0, 612.0, 792.0)]
        );
        // A scanned page without text keeps its picture.
        assert_eq!(pictures(&drawn[..1], 612.0, 792.0, false).len(), 1);
    }
}
