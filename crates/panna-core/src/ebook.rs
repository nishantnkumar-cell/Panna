//! Saving the text of pages as a web page (HTML), a Rich Text file (RTF) or an e-book (EPUB):
//! the same paragraphs and headings as the Word export, which reflow to fit any screen.
//! Layout and pictures aren't kept.

use crate::office::{Block, Paragraph, blocks, xml, zip_stored_first};
use crate::{Document, SaveError};

/// The paragraphs of each page, in order.
fn pages_of(blocks: Vec<Block>) -> Vec<Vec<Paragraph>> {
    let mut pages = vec![Vec::new()];
    for block in blocks {
        match block {
            Block::PageBreak => pages.push(Vec::new()),
            Block::Paragraph(p) => {
                if let Some(page) = pages.last_mut() {
                    page.push(p);
                }
            }
        }
    }
    pages
}

/// A paragraph as an HTML or XHTML element, with `id` on headings.
fn element(p: &Paragraph, id: &str) -> String {
    match p.level {
        1 => format!("<h1 id=\"{id}\">{}</h1>\n", xml(&p.text)),
        2 => format!("<h2 id=\"{id}\">{}</h2>\n", xml(&p.text)),
        _ => format!("<p>{}</p>\n", xml(&p.text)),
    }
}

const STYLE: &str = "body{max-width:40em;margin:2em auto;padding:0 1em;font-family:Georgia,serif;\
line-height:1.5}h1,h2{font-family:system-ui,sans-serif;line-height:1.2}\
section+section{border-top:1px solid #ccc;margin-top:2em}";

/// `pages` of `doc` as one web page, a section for each page.
pub(crate) fn html(doc: &Document, pages: &[usize], title: &str) -> Result<Vec<u8>, SaveError> {
    let mut body = String::new();
    for (n, page) in pages_of(blocks(doc, pages)?).iter().enumerate() {
        let number = pages.get(n).map_or(n + 1, |i| i + 1);
        body.push_str(&format!("<section id=\"page-{number}\">\n"));
        for (k, p) in page.iter().enumerate() {
            body.push_str(&element(p, &format!("p{number}-{k}")));
        }
        body.push_str("</section>\n");
    }
    Ok(format!(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" \
         content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>{STYLE}</style>\n\
         </head>\n<body>\n{body}</body>\n</html>\n",
        xml(title)
    )
    .into_bytes())
}

/// Text that can go in RTF: its special characters escaped, and everything past ASCII as
/// Unicode escapes (with `?` for readers that don't know them).
fn rtf_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\t' => out.push_str("\\tab "),
            c if c.is_control() => {}
            c if c.is_ascii() => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    // RTF's \u takes a signed 16-bit number.
                    out.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
    out
}

/// `pages` of `doc` as a Rich Text file, a page break between pages.
pub(crate) fn rtf(doc: &Document, pages: &[usize]) -> Result<Vec<u8>, SaveError> {
    let mut out = String::from(
        "{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1{\\fonttbl{\\f0\\froman Times New Roman;}{\\f1\\fswiss Arial;}}\n",
    );
    for (n, page) in pages_of(blocks(doc, pages)?).iter().enumerate() {
        if n > 0 {
            out.push_str("\\page\n");
        }
        for p in page {
            let size = if p.size.is_finite() {
                (p.size * 2.0).round().clamp(8.0, 400.0) as u32
            } else {
                24
            };
            let (font, bold, outline) = match p.level {
                1 => (1, "\\b", "\\outlinelevel0"),
                2 => (1, "\\b", "\\outlinelevel1"),
                _ => (0, "", ""),
            };
            out.push_str(&format!(
                "\\pard\\sa120{outline}\\f{font}\\fs{size}{bold} {}\\b0\\par\n",
                rtf_text(&p.text)
            ));
        }
    }
    out.push('}');
    Ok(out.into_bytes())
}

/// A name for the book that's the same for the same text, as EPUB wants one.
fn identifier(text: &[u8]) -> String {
    // FNV-1a, twice with different starting points, shaped as a UUID.
    let hash = |seed: u64| {
        text.iter()
            .fold(seed, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
    };
    let (a, b) = (hash(0xcbf2_9ce4_8422_2325), hash(0x84222325_cbf29ce4));
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0x0fff,
        0x8000 | (b >> 48) & 0x3fff,
        b & 0xffff_ffff_ffff
    )
}

/// `pages` of `doc` as an EPUB 3 e-book, a chapter for each page, with a table of contents
/// of the headings (or of the pages, when there are none).
pub(crate) fn epub(doc: &Document, pages: &[usize], title: &str) -> Result<Vec<u8>, SaveError> {
    let chapters = pages_of(blocks(doc, pages)?);
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut toc = String::new();
    let mut manifest = String::new();
    let mut spine = String::new();
    let mut all_text = Vec::new();
    for (n, page) in chapters.iter().enumerate() {
        let number = pages.get(n).map_or(n + 1, |i| i + 1);
        let name = format!("page-{number}.xhtml");
        let mut body = String::new();
        for (k, p) in page.iter().enumerate() {
            let id = format!("p{k}");
            body.push_str(&element(p, &id));
            all_text.extend(p.text.as_bytes());
            if p.level == 1 {
                toc.push_str(&format!(
                    "<li><a href=\"{name}#{id}\">{}</a></li>\n",
                    xml(&p.text)
                ));
            }
        }
        let xhtml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html \
             xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\">\n<head>\
             <meta charset=\"utf-8\"/><title>{}</title><link rel=\"stylesheet\" href=\"style.css\"/></head>\n\
             <body>\n<section id=\"page-{number}\">\n{body}</section>\n</body>\n</html>\n",
            xml(title)
        );
        manifest.push_str(&format!(
            "<item id=\"page{number}\" href=\"{name}\" media-type=\"application/xhtml+xml\"/>\n"
        ));
        spine.push_str(&format!("<itemref idref=\"page{number}\"/>\n"));
        files.push((format!("OEBPS/{name}"), xhtml.into_bytes()));
    }
    if toc.is_empty() {
        for (n, _) in chapters.iter().enumerate() {
            let number = pages.get(n).map_or(n + 1, |i| i + 1);
            toc.push_str(&format!(
                "<li><a href=\"page-{number}.xhtml\">Page {number}</a></li>\n"
            ));
        }
    }
    let nav = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html \
         xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\">\n<head>\
         <meta charset=\"utf-8\"/><title>{}</title></head>\n<body>\n<nav epub:type=\"toc\" id=\"toc\">\
         <h1>Contents</h1>\n<ol>\n{toc}</ol>\n</nav>\n</body>\n</html>\n",
        xml(title)
    );
    let modified = {
        let d = crate::edit::pdf_date(std::time::SystemTime::now());
        match (
            d.get(2..6),
            d.get(6..8),
            d.get(8..10),
            d.get(10..12),
            d.get(12..14),
            d.get(14..16),
        ) {
            (Some(y), Some(mo), Some(da), Some(h), Some(mi), Some(se)) => {
                format!("{y}-{mo}-{da}T{h}:{mi}:{se}Z")
            }
            _ => "2000-01-01T00:00:00Z".to_owned(),
        }
    };
    let opf = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" \
         version=\"3.0\" unique-identifier=\"book-id\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n\
         <dc:identifier id=\"book-id\">urn:uuid:{}</dc:identifier>\n<dc:title>{}</dc:title>\n\
         <dc:language>en</dc:language>\n<meta property=\"dcterms:modified\">{modified}</meta>\n</metadata>\n\
         <manifest>\n<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n\
         <item id=\"style\" href=\"style.css\" media-type=\"text/css\"/>\n{manifest}</manifest>\n\
         <spine>\n{spine}</spine>\n</package>\n",
        identifier(&all_text),
        xml(title)
    );
    let container = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container version=\"1.0\" \
                     xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n<rootfiles>\n<rootfile \
                     full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n\
                     </rootfiles>\n</container>\n";
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/style.css", STYLE.as_bytes()),
    ];
    entries.extend(files.iter().map(|(name, data)| (name.as_str(), data.as_slice())));
    zip_stored_first(&entries, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtf_escapes_braces_and_writes_unicode() {
        assert_eq!(rtf_text("a{b}\\c"), "a\\{b\\}\\\\c");
        assert_eq!(rtf_text("é"), "\\u233?");
        // Past the 16-bit range: a surrogate pair, each signed.
        assert_eq!(rtf_text("😀"), "\\u-10179?\\u-8704?");
    }

    #[test]
    fn identifiers_look_like_uuids_and_follow_the_text() {
        let a = identifier(b"one");
        assert_eq!(a.len(), 36);
        assert_eq!(a.chars().nth(14), Some('4'));
        assert_eq!(a, identifier(b"one"));
        assert_ne!(a, identifier(b"two"));
    }
}
