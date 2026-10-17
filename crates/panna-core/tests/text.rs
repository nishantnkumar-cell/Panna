#![allow(clippy::unwrap_used)] // test helpers
use panna_core::{Document, FindOptions, Limits, PageText};

/// Assembles numbered objects (1-based) into a PDF with a correct xref table.
fn build(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn page_text(content: &str, rotate: u16) -> PageText {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Rotate {rotate} \
             /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
        ),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let doc = Document::from_bytes(build(&objects), None, Limits::default()).unwrap();
    doc.page_text(0).unwrap()
}

const TWO_LINES: &str = "BT /F1 20 Tf 100 700 Td (Hello) Tj 80 0 Td (World) Tj 0 -40 Td (Second line) Tj ET";

#[test]
fn infers_spaces_and_line_breaks() {
    let text = page_text(TWO_LINES, 0);
    assert_eq!(text.slice(0..text.len()), "Hello World\nSecond line");
}

#[test]
fn boxes_are_in_display_space() {
    let text = page_text(TWO_LINES, 0);
    let hits = text.find("world", FindOptions::default());
    assert_eq!(hits.len(), 1);
    let rects = text.rects(hits[0].clone());
    assert_eq!(rects.len(), 1);
    let r = rects[0];
    // "World" starts at x=180; its baseline is 100pt from the top, so the box straddles y=100.
    assert!((r.x0 - 180.0).abs() < 0.5, "{r:?}");
    assert!(r.y0 < 100.0 && r.y1 > 100.0 && r.y1 < 110.0, "{r:?}");
    // Selecting from the top-left of "Hello" lands on its first gap.
    assert_eq!(text.caret_at(101.0, 95.0), Some(0));
}

#[test]
fn page_rotation_moves_boxes() {
    let text = page_text(TWO_LINES, 90);
    let r = text.rects(text.find("Hello", FindOptions::default())[0].clone())[0];
    // Turned clockwise, the page is 800 wide and the text runs down the page from x≈700.
    assert!(r.x0 > 650.0 && r.x1 < 720.0 && r.y0 > 90.0, "{r:?}");
}

#[test]
fn hidden_text_layers_are_searchable() {
    // Render mode 3 is invisible text, as used by OCR layers.
    let text = page_text("BT 3 Tr /F1 12 Tf 50 50 Td (scanned words) Tj ET", 0);
    assert_eq!(text.find("scanned", FindOptions::default()).len(), 1);
}
