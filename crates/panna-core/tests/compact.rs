#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, Limits};

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

fn stream(dict: &str, data: &str) -> String {
    format!("<< {dict} /Length {} >>\nstream\n{data}\nendstream", data.len())
}

/// Ten pages, each with its own uncompressed copy of the same content and the same picture.
fn bulky() -> Vec<u8> {
    let pages = 10;
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            (0..pages)
                .map(|i| format!("{} 0 R", 3 + i * 3))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    ];
    let mut content = String::new();
    for line in 0..30 {
        content.push_str(&format!(
            "BT /F1 10 Tf 40 {} Td (Line {line} of the page) Tj ET\n",
            760 - line * 20
        ));
    }
    content.push_str("q 100 0 0 100 300 100 cm /Im1 Do Q\n");
    // A red square, 16 by 16, without compression.
    let picture: String = "\u{ff}\u{0}\u{0}".repeat(256);
    for i in 0..pages {
        let page = 3 + i * 3;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R \
             /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> \
             /XObject << /Im1 {} 0 R >> >> >>",
            page + 1,
            page + 2
        ));
        objects.push(stream("", &content));
        objects.push(String::new());
        let last = objects.len() - 1;
        objects[last] = "PICTURE".into();
    }
    let mut bytes = build(&objects);
    // Pictures go in as raw bytes, which aren't text.
    let mut raw = Vec::new();
    for b in picture.chars() {
        raw.push(b as u32 as u8);
    }
    let mut image = format!(
        "<< /Type /XObject /Subtype /Image /Width 16 /Height 16 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
        raw.len()
    )
    .into_bytes();
    image.extend_from_slice(&raw);
    image.extend_from_slice(b"\nendstream");
    // Rebuild with the pictures in place, so the offsets are right.
    let objects: Vec<Vec<u8>> = objects
        .iter()
        .map(|o| {
            if o == "PICTURE" {
                image.clone()
            } else {
                o.clone().into_bytes()
            }
        })
        .collect();
    bytes.clear();
    bytes.extend_from_slice(b"%PDF-1.7\n");
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn looks(doc: &Document) -> Vec<Vec<u8>> {
    let cache = doc.new_cache();
    (0..doc.page_count())
        .map(|i| doc.render_page(&cache, i, 0.5).unwrap().rgba)
        .collect()
}

#[test]
fn a_compacted_file_is_smaller_and_looks_the_same() {
    let bytes = bulky();
    let doc = Document::from_bytes(bytes.clone(), None, Limits::default()).unwrap();
    let small = doc.compacted().unwrap();
    assert!(
        small.len() * 4 < bytes.len(),
        "{} -> {}",
        bytes.len(),
        small.len()
    );
    let new = Document::from_bytes(small, None, Limits::default()).unwrap();
    assert_eq!(new.page_count(), 10);
    assert_eq!(looks(&new), looks(&doc));
    let text = new.page_text(9).unwrap();
    assert!(text.slice(0..text.len()).contains("Line 29 of the page"));
    // Compacting again changes nothing that matters.
    let again = new.compacted().unwrap();
    let newer = Document::from_bytes(again, None, Limits::default()).unwrap();
    assert_eq!(looks(&newer), looks(&doc));
}

#[test]
fn compacted_encrypted_files_keep_their_password() {
    for bytes in [
        &include_bytes!("fixtures/encrypted-rc4.pdf")[..],
        include_bytes!("fixtures/encrypted-aes128.pdf"),
        include_bytes!("fixtures/encrypted-aes256.pdf"),
    ] {
        let doc = Document::from_bytes(bytes.to_vec(), Some("secret"), Limits::default()).unwrap();
        let small = doc.compacted().unwrap();
        assert!(Document::from_bytes(small.clone(), None, Limits::default()).is_err());
        let new = Document::from_bytes(small, Some("secret"), Limits::default()).unwrap();
        assert_eq!(looks(&new), looks(&doc));
        assert_eq!(new.info().unwrap().fields.len(), doc.info().unwrap().fields.len());
    }
}
