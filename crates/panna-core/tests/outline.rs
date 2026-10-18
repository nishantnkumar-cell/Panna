#![allow(clippy::unwrap_used)] // test helpers
use panna_core::{Document, Limits, OutlineItem};

/// Assembles numbered objects (1-based) into a PDF with a correct xref table.
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

fn outline(objects: &[&str]) -> Vec<OutlineItem> {
    let doc = Document::from_bytes(build(objects), None, Limits::default()).unwrap();
    doc.info().unwrap().outline
}

const PAGES: [&str; 4] = [
    "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
];

#[test]
fn reads_nested_bookmarks_with_explicit_and_named_destinations() {
    let mut objects = vec!["<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R /Names << /Dests 10 0 R >> >>"];
    objects.extend(PAGES);
    objects.extend([
        "<< /Type /Outlines /First 7 0 R /Last 8 0 R >>",
        // 7: explicit destination, with one child using a named destination via GoTo.
        "<< /Title (Intro) /Parent 6 0 R /Next 8 0 R /First 9 0 R /Dest [4 0 R /Fit] >>",
        // 8: UTF-16 title, page 3.
        "<< /Title <FEFF0043006800200032> /Parent 6 0 R /Dest [5 0 R /XYZ 0 792 0] >>",
        "<< /Title (Details) /Parent 7 0 R /A << /S /GoTo /D (sec-1) >> >>",
        "<< /Names [(sec-1) [3 0 R /Fit]] >>",
    ]);
    assert_eq!(
        outline(&objects),
        vec![
            OutlineItem {
                title: "Intro".into(),
                depth: 0,
                page: Some(1)
            },
            OutlineItem {
                title: "Details".into(),
                depth: 1,
                page: Some(0)
            },
            OutlineItem {
                title: "Ch 2".into(),
                depth: 0,
                page: Some(2)
            },
        ]
    );
}

#[test]
fn cyclic_outline_terminates() {
    let mut objects = vec!["<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>"];
    objects.extend(PAGES);
    objects.extend([
        "<< /Type /Outlines /First 7 0 R >>",
        // Each item is its own next sibling and first child.
        "<< /Title (Loop) /Next 7 0 R /First 7 0 R /Dest [3 0 R /Fit] >>",
    ]);
    let items = outline(&objects);
    assert_eq!(items.len(), 1);
}

#[test]
fn no_outline_is_empty() {
    let mut objects = vec!["<< /Type /Catalog /Pages 2 0 R >>"];
    objects.extend(PAGES);
    assert!(outline(&objects).is_empty());
}
