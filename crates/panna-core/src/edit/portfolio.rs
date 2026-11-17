//! Making a PDF Portfolio: files of any kind packaged in one PDF, as its attached files, with
//! a `/Collection` that tells PDF readers to show them as a list, and a cover page naming them
//! for readers that don't.

use std::fmt::Write as _;

use super::annot::{literal, text_string};
use super::pack::deflate;

/// Files in one portfolio, at most.
pub const MAX_PORTFOLIO_FILES: usize = 1_000;
/// Names listed on the cover page, at most; the rest are counted.
const LISTED: usize = 40;

/// The cover page's text in Helvetica's own encoding: Latin-1 letters stay, others become `?`.
fn latin1(s: &str) -> Vec<u8> {
    s.chars()
        .filter(|c| !c.is_control())
        .map(|c| {
            u8::try_from(u32::from(c))
                .ok()
                .filter(|b| *b >= 0x20)
                .unwrap_or(b'?')
        })
        .collect()
}

/// A PDF Portfolio holding `files`, each a name and its contents, in that order.
pub fn make_portfolio(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let files = files
        .get(..files.len().min(MAX_PORTFOLIO_FILES))
        .unwrap_or_default();
    // 1 catalog, 2 pages, 3 page, 4 contents, 5 font, then a spec and a stream per file.
    let mut objects: Vec<Vec<u8>> = Vec::new();
    let first_file = 6;
    let mut names = String::new();
    for i in 0..files.len() {
        let spec = first_file + 2 * i;
        // Name tree keys sort as bytes; numbering them keeps the files in the order given.
        let _ = write!(names, "({:05}) {spec} 0 R ", i + 1);
    }
    let catalog = format!(
        "<< /Type /Catalog /Pages 2 0 R /PageMode /UseAttachments \
         /Names << /EmbeddedFiles << /Names [{}] >> >> \
         /Collection << /Type /Collection /View /D /Sort << /S /N >> >> >>",
        names.trim_end()
    );
    objects.push(catalog.into_bytes());
    objects.push(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec());
    objects.push(
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
          /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_vec(),
    );
    let mut content = String::from("BT /F1 20 Tf 72 720 Td (PDF Portfolio) Tj ET\n");
    let _ = writeln!(
        content,
        "BT /F1 11 Tf 72 696 Td ({}) Tj ET",
        literal(&latin1(&format!(
            "{} {}. Open the attachments to see {}.",
            files.len(),
            if files.len() == 1 { "file" } else { "files" },
            if files.len() == 1 { "it" } else { "them" }
        )))
    );
    for (i, (name, _)) in files.iter().take(LISTED).enumerate() {
        let y = 666 - 15 * i as i32;
        let _ = writeln!(
            content,
            "BT /F1 10 Tf 84 {y} Td ({}) Tj ET",
            literal(&latin1(name))
        );
    }
    if files.len() > LISTED {
        let y = 666 - 15 * LISTED as i32;
        let _ = writeln!(
            content,
            "BT /F1 10 Tf 84 {y} Td ({}) Tj ET",
            literal(&latin1(&format!("and {} more", files.len() - LISTED)))
        );
    }
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend(content.as_bytes());
    stream.extend(b"\nendstream");
    objects.push(stream);
    objects
        .push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec());
    for (i, (name, data)) in files.iter().enumerate() {
        let stream_num = first_file + 2 * i + 1;
        let name = text_string(name);
        objects.push(
            format!(
                "<< /Type /Filespec /F {name} /UF {name} /EF << /F {stream_num} 0 R /UF {stream_num} 0 R >> >>"
            )
            .into_bytes(),
        );
        let packed = deflate(data);
        let mut file = format!(
            "<< /Type /EmbeddedFile /Filter /FlateDecode /Params << /Size {} >> /Length {} >>\nstream\n",
            data.len(),
            packed.len()
        )
        .into_bytes();
        file.extend(packed);
        file.extend(b"\nendstream");
        objects.push(file);
    }

    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
