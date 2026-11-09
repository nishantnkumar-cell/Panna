#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, Limits};

/// Assembles numbered objects (1-based) into a PDF with a classic xref table.
fn build(objects: &[&str]) -> Vec<u8> {
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

/// Three pages; the second has a note and a reply to it.
fn commented() -> Document {
    let bytes = build(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R 7 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "<< /Type /Annot /Subtype /Text /Rect [10 700 30 720] /T (Asha) /Contents (Check this figure) >>",
        "<< /Type /Annot /Subtype /Text /Rect [10 700 30 720] /T (Ravi) /IRT 6 0 R /Contents (Fixed it) >>",
    ]);
    Document::from_bytes(bytes, None, Limits::default()).unwrap()
}

fn all_text(doc: &Document, page: usize) -> String {
    let text = doc.page_text(page).unwrap();
    text.slice(0..text.len())
}

#[test]
fn printing_with_comments_puts_a_list_after_each_commented_page() {
    let doc = commented();
    let out = Document::from_bytes(doc.print_with_comments().unwrap(), None, Limits::default()).unwrap();
    assert_eq!(out.page_count(), 4);
    let list = all_text(&out, 2);
    assert!(list.contains("Comments on page 2"), "{list}");
    assert!(list.contains("Check this figure"), "{list}");
    assert!(list.contains("Reply by Ravi"), "{list}");
    assert!(all_text(&out, 3).is_empty());
}

#[test]
fn the_summary_lists_every_comment_under_the_document_name() {
    let doc = commented();
    let summary = Document::from_bytes(
        doc.comment_summary("report.pdf").unwrap(),
        None,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(summary.page_count(), 1);
    let text = all_text(&summary, 0);
    assert!(text.contains("Comments on report.pdf"), "{text}");
    assert!(
        text.contains("Note by Asha") || text.contains("Sticky note by Asha"),
        "{text}"
    );
    assert!(text.contains("Fixed it"), "{text}");
}

#[test]
fn printing_with_comments_and_none_prints_just_the_pages() {
    let bytes = build(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
    ]);
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    let out = Document::from_bytes(doc.print_with_comments().unwrap(), None, Limits::default()).unwrap();
    assert_eq!(out.page_count(), 1);
}
