//! Saving pages as Office files: a Word document (.docx) of the text, in paragraphs, with
//! headings where the type is larger and a page break between pages (layout, pictures and
//! tables aren't kept); a PowerPoint presentation (.pptx) with a picture of each page as a
//! slide; or an Excel workbook (.xlsx) with a sheet for each page, a row for each line of
//! text, and columns where the words line up.

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

const NS: &str = "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"";

const XML_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// An empty shape tree, which every slide, layout and master starts with.
const SP_TREE_START: &str = "<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/>\
</p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" \
y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";

fn relationships(rels: &[(&str, &str, String)]) -> String {
    let mut out = format!(
        "{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">"
    );
    for (id, kind, target) in rels {
        out.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{target}\"/>"
        ));
    }
    out.push_str("</Relationships>");
    out
}

fn theme() -> String {
    let colors = [
        ("dk2", "1F2937"),
        ("lt2", "E5E7EB"),
        ("accent1", "4F46E5"),
        ("accent2", "DB2777"),
        ("accent3", "059669"),
        ("accent4", "D97706"),
        ("accent5", "0284C7"),
        ("accent6", "7C3AED"),
        ("hlink", "2563EB"),
        ("folHlink", "7C3AED"),
    ];
    let mut scheme = String::from(
        "<a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1><a:lt1><a:sysClr \
         val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>",
    );
    for (name, rgb) in colors {
        scheme.push_str(&format!("<a:{name}><a:srgbClr val=\"{rgb}\"/></a:{name}>"));
    }
    let font = "<a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/>";
    let fill = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>";
    let line = format!("<a:ln w=\"9525\">{fill}</a:ln>");
    let effect = "<a:effectStyle><a:effectLst/></a:effectStyle>";
    format!(
        "{XML_HEAD}<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
         name=\"Panna\"><a:themeElements><a:clrScheme name=\"Panna\">{scheme}</a:clrScheme><a:fontScheme \
         name=\"Panna\"><a:majorFont>{font}</a:majorFont><a:minorFont>{font}</a:minorFont></a:fontScheme>\
         <a:fmtScheme name=\"Panna\"><a:fillStyleLst>{fill}{fill}{fill}</a:fillStyleLst><a:lnStyleLst>{line}\
         {line}{line}</a:lnStyleLst><a:effectStyleLst>{effect}{effect}{effect}</a:effectStyleLst>\
         <a:bgFillStyleLst>{fill}{fill}{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements>\
         <a:objectDefaults/><a:extraClrSchemeLst/></a:theme>"
    )
}

/// Points to English Metric Units, PowerPoint's unit.
fn emu(points: f64) -> i64 {
    (points * 12_700.0).round() as i64
}

/// A slide of each page: its picture as a PNG, and its size in points. The slides are the
/// size of the first page; other pages are fitted inside, centred.
pub(crate) fn pptx(slides: &[(Vec<u8>, f64, f64)]) -> Result<Vec<u8>, SaveError> {
    // PowerPoint allows slides from 1 to 56 inches a side.
    let (w, h) = slides.first().map_or((720.0, 540.0), |(_, w, h)| (*w, *h));
    let fit = (4032.0 / w.max(h)).min(1.0).max(72.0 / w.min(h).max(1.0));
    let (cx, cy) = (emu(w * fit), emu(h * fit));
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut types = format!(
        "{XML_HEAD}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Default Extension=\"png\" ContentType=\"image/png\"/>\
         <Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
         <Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
         <Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
         <Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>"
    );
    let mut ids = String::new();
    let mut rels = vec![
        ("rId1", "slideMaster", "slideMasters/slideMaster1.xml".to_owned()),
        ("rId2", "theme", "theme/theme1.xml".to_owned()),
    ];
    let rel_ids: Vec<String> = (0..slides.len()).map(|i| format!("rId{}", i + 3)).collect();
    for (i, (png, pw, ph)) in slides.iter().enumerate() {
        let n = i + 1;
        let scale = (cx as f64 / emu(*pw).max(1) as f64).min(cy as f64 / emu(*ph).max(1) as f64);
        let (pcx, pcy) = (
            (emu(*pw) as f64 * scale).round() as i64,
            (emu(*ph) as f64 * scale).round() as i64,
        );
        let (x, y) = ((cx - pcx) / 2, (cy - pcy) / 2);
        let slide = format!(
            "{XML_HEAD}<p:sld {NS}><p:cSld>{SP_TREE_START}<p:pic><p:nvPicPr><p:cNvPr id=\"2\" \
             name=\"Page {n}\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/>\
             </p:nvPicPr><p:blipFill><a:blip r:embed=\"rId2\"/><a:stretch><a:fillRect/></a:stretch>\
             </p:blipFill><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{pcx}\" cy=\"{pcy}\"/>\
             </a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic></p:spTree>\
             </p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
        );
        files.push((format!("ppt/slides/slide{n}.xml"), slide.into_bytes()));
        files.push((
            format!("ppt/slides/_rels/slide{n}.xml.rels"),
            relationships(&[
                ("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml".into()),
                ("rId2", "image", format!("../media/page{n}.png")),
            ])
            .into_bytes(),
        ));
        files.push((format!("ppt/media/page{n}.png"), png.clone()));
        types.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"
        ));
        ids.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"{}\"/>",
            256 + i,
            rel_ids.get(i).map_or("", String::as_str)
        ));
    }
    for (i, id) in rel_ids.iter().enumerate() {
        rels.push((id.as_str(), "slide", format!("slides/slide{}.xml", i + 1)));
    }
    types.push_str("</Types>");
    let presentation = format!(
        "{XML_HEAD}<p:presentation {NS}><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" \
         r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx=\"{cx}\" \
         cy=\"{cy}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
    );
    let color_map = "<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" \
                     accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" \
                     accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>";
    let master = format!(
        "{XML_HEAD}<p:sldMaster {NS}><p:cSld>{SP_TREE_START}</p:spTree></p:cSld>{color_map}\
         <p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst></p:sldMaster>"
    );
    let layout = format!(
        "{XML_HEAD}<p:sldLayout {NS} type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\">\
         {SP_TREE_START}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
    );
    let mut all: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), types.into_bytes()),
        (
            "_rels/.rels".into(),
            relationships(&[("rId1", "officeDocument", "ppt/presentation.xml".into())]).into_bytes(),
        ),
        ("ppt/presentation.xml".into(), presentation.into_bytes()),
        (
            "ppt/_rels/presentation.xml.rels".into(),
            relationships(&rels).into_bytes(),
        ),
        ("ppt/slideMasters/slideMaster1.xml".into(), master.into_bytes()),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels".into(),
            relationships(&[
                ("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml".into()),
                ("rId2", "theme", "../theme/theme1.xml".into()),
            ])
            .into_bytes(),
        ),
        ("ppt/slideLayouts/slideLayout1.xml".into(), layout.into_bytes()),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(),
            relationships(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml".into())]).into_bytes(),
        ),
        ("ppt/theme/theme1.xml".into(), theme().into_bytes()),
    ];
    all.extend(files);
    let refs: Vec<(&str, &[u8])> = all.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    zip(&refs)
}

/// Rows of cells from lines: a column starts at each left edge that cells share, give or
/// take, so text that lines up lands in the same column.
fn table(lines: &[Line]) -> Vec<Vec<(usize, String)>> {
    let size = {
        let mut sizes: Vec<f32> = lines.iter().map(|l| l.size).filter(|s| *s > 0.0).collect();
        sizes.sort_by(f32::total_cmp);
        sizes.get(sizes.len() / 2).copied().unwrap_or(10.0)
    };
    let mut edges: Vec<f32> = lines.iter().flat_map(|l| l.cells.iter().map(|c| c.0)).collect();
    edges.sort_by(f32::total_cmp);
    let mut columns: Vec<f32> = Vec::new();
    for x in edges {
        if columns.last().is_none_or(|c| x - c > size) {
            columns.push(x);
        }
    }
    lines
        .iter()
        .map(|line| {
            let mut row: Vec<(usize, String)> = Vec::new();
            for (x, text) in &line.cells {
                let col = columns.iter().rposition(|c| *c <= x + 0.01).unwrap_or(0);
                // Two cells in one column share it.
                match row.last_mut() {
                    Some((c, cell)) if *c >= col => {
                        cell.push(' ');
                        cell.push_str(text);
                    }
                    _ => row.push((col, text.clone())),
                }
            }
            row
        })
        .collect()
}

/// A spreadsheet column's letters: 0 is A, 26 is AA.
fn column_name(mut col: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// The number a cell shows, if it's only a number: digits with an optional sign, decimal
/// point and thousands separators.
fn number(text: &str) -> Option<f64> {
    let t = text.trim();
    let plain: String = t.chars().filter(|c| *c != ',').collect();
    let ok = !plain.is_empty()
        && plain.chars().any(|c| c.is_ascii_digit())
        && plain
            .char_indices()
            .all(|(i, c)| c.is_ascii_digit() || c == '.' || (i == 0 && matches!(c, '-' | '+')))
        // Thousands separators only between groups of three digits.
        && t.split(',').skip(1).all(|g| g.len() >= 3 && g.chars().take(3).all(|c| c.is_ascii_digit()));
    ok.then(|| plain.parse::<f64>().ok())
        .flatten()
        .filter(|v| v.is_finite())
}

fn sheet_xml(rows: &[Vec<(usize, String)>]) -> String {
    let mut data = String::new();
    for (r, row) in rows.iter().enumerate() {
        if row.is_empty() {
            continue;
        }
        let r = r + 1;
        data.push_str(&format!("<row r=\"{r}\">"));
        for (col, text) in row {
            let at = format!("{}{r}", column_name(*col));
            match number(text) {
                Some(v) => data.push_str(&format!("<c r=\"{at}\"><v>{v}</v></c>")),
                None => data.push_str(&format!(
                    "<c r=\"{at}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
                    xml(text)
                )),
            }
        }
        data.push_str("</row>");
    }
    format!(
        "{XML_HEAD}<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
         <sheetData>{data}</sheetData></worksheet>"
    )
}

/// `pages` of `doc` as a .xlsx file, a sheet for each page.
pub(crate) fn xlsx(doc: &Document, pages: &[usize]) -> Result<Vec<u8>, SaveError> {
    let ns = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
              xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
    let mut types = format!(
        "{XML_HEAD}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
         <Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>"
    );
    let mut sheets = String::new();
    let mut rels = vec![("rId1".to_owned(), "styles", "styles.xml".to_owned())];
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, &index) in pages.iter().enumerate() {
        let text = doc.page_text(index).map_err(|_| SaveError::NoSuchPage(index))?;
        let n = i + 1;
        files.push((
            format!("xl/worksheets/sheet{n}.xml"),
            sheet_xml(&table(&text.lines())).into_bytes(),
        ));
        types.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        ));
        sheets.push_str(&format!(
            "<sheet name=\"Page {}\" sheetId=\"{n}\" r:id=\"rId{}\"/>",
            index + 1,
            n + 1
        ));
        rels.push((
            format!("rId{}", n + 1),
            "worksheet",
            format!("worksheets/sheet{n}.xml"),
        ));
    }
    types.push_str("</Types>");
    let workbook = format!("{XML_HEAD}<workbook {ns}><sheets>{sheets}</sheets></workbook>");
    let styles = format!(
        "{XML_HEAD}<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
         <fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
         <fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill \
         patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/>\
         <bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" \
         fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"1\"><xf \
         numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs><cellStyles \
         count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>"
    );
    let rel_refs: Vec<(&str, &str, String)> =
        rels.iter().map(|(a, b, c)| (a.as_str(), *b, c.clone())).collect();
    let mut all: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), types.into_bytes()),
        (
            "_rels/.rels".into(),
            relationships(&[("rId1", "officeDocument", "xl/workbook.xml".into())]).into_bytes(),
        ),
        ("xl/workbook.xml".into(), workbook.into_bytes()),
        (
            "xl/_rels/workbook.xml.rels".into(),
            relationships(&rel_refs).into_bytes(),
        ),
        ("xl/styles.xml".into(), styles.into_bytes()),
    ];
    all.extend(files);
    let refs: Vec<(&str, &[u8])> = all.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    zip(&refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_that_line_up_share_a_column() {
        let row = |cells: &[(f32, &str)]| Line {
            text: String::new(),
            x0: 0.0,
            y0: 0.0,
            y1: 10.0,
            size: 10.0,
            cells: cells.iter().map(|(x, t)| (*x, (*t).to_owned())).collect(),
            x1: 0.0,
        };
        let lines = [
            row(&[(72.0, "Item"), (200.0, "Price")]),
            row(&[(72.0, "Tea"), (201.0, "1,250.50")]),
            row(&[(72.0, "A long title")]),
            row(&[(200.5, "3")]),
        ];
        let t = table(&lines);
        assert_eq!(t[1], [(0, "Tea".to_owned()), (1, "1,250.50".to_owned())]);
        assert_eq!(t[3], [(1, "3".to_owned())]);
        assert_eq!(number("1,250.50"), Some(1250.5));
        assert_eq!(number("12,34"), None);
        assert_eq!(number("2026-10-19"), None);
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(27), "AB");
    }

    fn line(text: &str, y0: f32, size: f32) -> Line {
        Line {
            cells: Vec::new(),
            x1: 300.0,
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
