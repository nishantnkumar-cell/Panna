#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, Limits};

/// Assembles numbered objects (1-based) into a PDF with a classic xref table.
fn build(objects: &[&str], trailer: &str) -> Vec<u8> {
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
            "trailer\n<< /Size {} /Root 1 0 R {trailer} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// One file attached to the document, one to its second page, and one listed in both places.
fn attached() -> Vec<u8> {
    build(
        &[
            "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Kids [5 0 R] >> >> >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [10 0 R 11 0 R] >>",
            "<< /Limits [(a) (b)] /Names [(a) 6 0 R (b) 12 0 R] >>",
            "<< /Type /Filespec /F (old.txt) /UF (C:\\\\Users\\\\jo\\\\notes.txt) /Desc (Meeting notes) /EF << /F 7 0 R >> >>",
            "<< /Type /EmbeddedFile /Params << /Size 5 >> /Length 5 >>\nstream\nhello\nendstream",
            "<< /Type /Filespec /F (photo.jpg) /EF << /F 9 0 R >> >>",
            "<< /Type /EmbeddedFile /Length 3 >>\nstream\nabc\nendstream",
            "<< /Type /Annot /Subtype /FileAttachment /Rect [10 10 30 30] /FS 8 0 R >>",
            "<< /Type /Annot /Subtype /FileAttachment /Rect [40 10 60 30] /FS 12 0 R >>",
            "<< /Type /Filespec /F (both.txt) /EF << /F 13 0 R >> >>",
            "<< /Type /EmbeddedFile /Length 4 >>\nstream\nboth\nendstream",
        ],
        "",
    )
}

#[test]
fn attachments_are_listed_and_read() {
    let doc = Document::from_bytes(attached(), None, Limits::default()).unwrap();
    let files = doc.info().unwrap().attachments;
    let names: Vec<_> = files.iter().map(|f| (f.name.as_str(), f.page)).collect();
    assert_eq!(
        names,
        [("notes.txt", None), ("both.txt", None), ("photo.jpg", Some(1))]
    );
    assert_eq!(files[0].description, "Meeting notes");
    assert_eq!(files[0].size, Some(5));
    assert_eq!(files[2].size, None);
    assert_eq!(doc.attachment(0).unwrap(), b"hello");
    assert_eq!(doc.attachment(2).unwrap(), b"abc");
    assert_eq!(doc.attachment(1).unwrap(), b"both");
    assert!(doc.attachment(3).is_none());
}
