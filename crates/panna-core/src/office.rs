//! Saving pages as a Word document (.docx): the text, in paragraphs, with headings where the
//! type is larger, and a page break between pages. Layout, pictures and tables aren't kept.

use std::io::Write;

use crate::text::Line;
use crate::{Document, SaveError};

/// A paragraph: its text, font size in points, and heading level (0 for body text).
#[derive(Debug, Clone, PartialEq)]
struct Paragraph {
    text: String,
    size: f32,
    level: u8,
}

enum Block {
    Paragraph(Paragraph),
    PageBreak,
}

/// Lines joined into paragraphs: a line continues the one before when it's close below it,
/// in about the same size of type.
fn paragraphs(lines: &[Line]) -> Vec<Paragraph> {
    let mut out: Vec<Paragraph> = Vec::new();
    let mut prev: Option<&Line> = None;
    for line in lines {
        let continues = prev.is_some_and(|p| {
            let height = (p.y1 - p.y0).max(1.0);
            let gap = line.y0 - p.y1;
            gap > -height * 0.5 && gap < height * 0.6 && (line.size - p.size).abs() <= p.size * 0.15
        });
        match out.last_mut() {
            Some(para) if continues => {
                // A word broken across lines with a hyphen is joined up again.
                let joined = para.text.ends_with('-')
                    && line.text.chars().next().is_some_and(char::is_lowercase)
                    && para.text.chars().rev().nth(1).is_some_and(char::is_alphabetic);
                if joined {
                    para.text.pop();
                } else {
                    para.text.push(' ');
                }
                para.text.push_str(&line.text);
            }
            _ => out.push(Paragraph {
                text: line.text.clone(),
                size: line.size,
                level: 0,
            }),
        }
        prev = Some(line);
    }
    out
}

/// Text that can go in XML: the five special characters escaped, characters XML can't hold
/// left out.
fn xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push(' '),
            c if c.is_control() || matches!(c, '\u{FFFE}' | '\u{FFFF}') => {}
            c => out.push(c),
        }
    }
    out
}

/// The document's text: the body's size is the size most of the text is in, and larger
/// paragraphs that are short enough become headings.
fn blocks(doc: &Document, pages: &[usize]) -> Result<Vec<Block>, SaveError> {
    let mut out = Vec::new();
    for (n, &index) in pages.iter().enumerate() {
        let text = doc.page_text(index).map_err(|_| SaveError::NoSuchPage(index))?;
        if n > 0 {
            out.push(Block::PageBreak);
        }
        out.extend(paragraphs(&text.lines()).into_iter().map(Block::Paragraph));
    }
    // Characters at each size, in half points.
    let mut sizes: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for b in &out {
        if let Block::Paragraph(p) = b {
            *sizes.entry(half_points(p.size)).or_default() += p.text.chars().count();
        }
    }
    let body = sizes
        .iter()
        .max_by_key(|(size, count)| (**count, std::cmp::Reverse(**size)))
        .map_or(22, |(size, _)| *size) as f32
        / 2.0;
    for b in &mut out {
        if let Block::Paragraph(p) = b
            && p.text.chars().count() <= 200
        {
            p.level = if p.size >= body * 1.6 {
                1
            } else if p.size >= body * 1.2 {
                2
            } else {
                0
            };
        }
    }
    Ok(out)
}

/// A size in points as Word's half points, within what Word allows.
fn half_points(size: f32) -> u32 {
    if size.is_finite() {
        (size * 2.0).round().clamp(2.0, 3276.0) as u32
    } else {
        22
    }
}

fn document_xml(blocks: &[Block]) -> String {
    let mut body = String::new();
    // A new page starts with its first paragraph, or with an empty one when it has none.
    let mut new_page = false;
    for b in blocks {
        match b {
            Block::PageBreak => {
                if new_page {
                    body.push_str("<w:p><w:pPr><w:pageBreakBefore/></w:pPr></w:p>");
                }
                new_page = true;
            }
            Block::Paragraph(p) => {
                let style = match p.level {
                    1 => "<w:pStyle w:val=\"Heading1\"/>",
                    2 => "<w:pStyle w:val=\"Heading2\"/>",
                    _ => "",
                };
                let page = if std::mem::take(&mut new_page) {
                    "<w:pageBreakBefore/>"
                } else {
                    ""
                };
                let sz = half_points(p.size);
                body.push_str(&format!(
                    "<w:p><w:pPr>{style}{page}</w:pPr><w:r><w:rPr><w:sz w:val=\"{sz}\"/><w:szCs w:val=\"{sz}\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                    xml(&p.text)
                ));
            }
        }
    }
    if new_page {
        body.push_str("<w:p><w:pPr><w:pageBreakBefore/></w:pPr></w:p>");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document \
         xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr>\
         <w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" \
         w:left=\"1440\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>"
    )
}

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
</Types>";

const RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>";

const DOCUMENT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
</Relationships>";

const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:docDefaults><w:pPrDefault><w:pPr><w:spacing w:after=\"120\"/></w:pPr></w:pPrDefault></w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"Normal\"/>\
<w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before=\"240\"/><w:outlineLvl w:val=\"0\"/></w:pPr>\
<w:rPr><w:b/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/><w:basedOn w:val=\"Normal\"/>\
<w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before=\"200\"/><w:outlineLvl w:val=\"1\"/></w:pPr>\
<w:rPr><w:b/></w:rPr></w:style>\
</w:styles>";

/// A ZIP archive of `files` (name and contents), each compressed.
fn zip(files: &[(&str, &[u8])]) -> Result<Vec<u8>, SaveError> {
    let io = |e: std::io::Error| SaveError::Io(e.to_string());
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let crc = crc.sum();
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).map_err(io)?;
        let packed = enc.finish().map_err(io)?;
        let too_big = || SaveError::Io("The document is too large for a Word file".into());
        let offset = u32::try_from(out.len()).map_err(|_| too_big())?;
        let (packed_len, len) = (
            u32::try_from(packed.len()).map_err(|_| too_big())?,
            u32::try_from(data.len()).map_err(|_| too_big())?,
        );
        let name_len = u16::try_from(name.len()).map_err(|_| too_big())?;
        // Version 2.0, UTF-8 names, deflated, 1980-01-01 00:00.
        let common = |v: &mut Vec<u8>| {
            v.extend(20u16.to_le_bytes());
            v.extend(0x0800u16.to_le_bytes());
            v.extend(8u16.to_le_bytes());
            v.extend(0u16.to_le_bytes());
            v.extend(0x0021u16.to_le_bytes());
            v.extend(crc.to_le_bytes());
            v.extend(packed_len.to_le_bytes());
            v.extend(len.to_le_bytes());
            v.extend(name_len.to_le_bytes());
            v.extend(0u16.to_le_bytes());
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        common(&mut out);
        out.extend(name.as_bytes());
        out.extend(&packed);

        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        common(&mut central);
        // Comment length, disk, internal and external attributes.
        central.extend([0u8; 10]);
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let count = u16::try_from(files.len()).map_err(|_| SaveError::Io("Too many files".into()))?;
    let at = u32::try_from(out.len()).map_err(|_| SaveError::Io("Too large".into()))?;
    let size = u32::try_from(central.len()).map_err(|_| SaveError::Io("Too large".into()))?;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(count.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend(at.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    Ok(out)
}

/// `pages` of `doc` as a .docx file.
pub(crate) fn docx(doc: &Document, pages: &[usize]) -> Result<Vec<u8>, SaveError> {
    let document = document_xml(&blocks(doc, pages)?);
    zip(&[
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", RELS.as_bytes()),
        ("word/document.xml", document.as_bytes()),
        ("word/_rels/document.xml.rels", DOCUMENT_RELS.as_bytes()),
        ("word/styles.xml", STYLES.as_bytes()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, y0: f32, size: f32) -> Line {
        Line {
            text: text.into(),
            x0: 72.0,
            y0,
            y1: y0 + size,
            size,
        }
    }

    #[test]
    fn lines_close_together_make_a_paragraph() {
        let lines = [
            line("Title", 50.0, 24.0),
            line("The first para-", 100.0, 11.0),
            line("graph goes on", 112.0, 11.0),
            line("here.", 124.0, 11.0),
            line("A second one.", 150.0, 11.0),
        ];
        let texts: Vec<String> = paragraphs(&lines).into_iter().map(|p| p.text).collect();
        assert_eq!(
            texts,
            ["Title", "The first paragraph goes on here.", "A second one."]
        );
    }

    #[test]
    fn xml_is_escaped() {
        assert_eq!(xml("a<b & \"c\"\u{1}"), "a&lt;b &amp; &quot;c&quot;");
    }
}
