#![allow(clippy::unwrap_used)] // test helpers
use panna_core::{Document, Limits, ObjRef, OutlineItem};

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
                page: Some(1),
                id: Some(ObjRef {
                    number: 7,
                    generation: 0
                }),
            },
            OutlineItem {
                title: "Details".into(),
                depth: 1,
                page: Some(0),
                id: Some(ObjRef {
                    number: 9,
                    generation: 0
                }),
            },
            OutlineItem {
                title: "Ch 2".into(),
                depth: 0,
                page: Some(2),
                id: Some(ObjRef {
                    number: 8,
                    generation: 0
                }),
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

fn reopen(doc: &Document, bytes: Vec<u8>) -> Vec<OutlineItem> {
    doc.reopen(bytes).unwrap().info().unwrap().outline
}

#[test]
fn bookmarks_are_renamed_moved_added_and_removed() {
    let mut objects = vec!["<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>"];
    objects.extend(PAGES);
    objects.extend([
        "<< /Type /Outlines /First 7 0 R /Last 8 0 R /Count 2 >>",
        "<< /Title (One) /Parent 6 0 R /Next 8 0 R /Dest [3 0 R /XYZ 0 500 2] /C [1 0 0] >>",
        "<< /Title (Two) /Parent 6 0 R /Prev 7 0 R /Dest [4 0 R /Fit] >>",
    ]);
    let doc = Document::from_bytes(build(&objects), None, Limits::default()).unwrap();
    let mut items = doc.info().unwrap().outline;
    assert_eq!(items.len(), 2);

    // "Two" is renamed, put first and pointed at page 3; "One" goes under it; a new one is
    // added at the end.
    let mut two = items.remove(1);
    two.title = "Second → first".into();
    two.page = Some(2);
    let mut one = items.remove(0);
    one.depth = 1;
    let new = OutlineItem {
        title: "Added".into(),
        depth: 0,
        page: Some(0),
        id: None,
    };
    let bytes = doc.with_bookmarks(&[two, one, new]).unwrap();
    let edited = reopen(&doc, bytes.clone());
    let shown: Vec<(&str, u16, Option<usize>)> = edited
        .iter()
        .map(|i| (i.title.as_str(), i.depth, i.page))
        .collect();
    assert_eq!(
        shown,
        [
            ("Second → first", 0, Some(2)),
            ("One", 1, Some(0)),
            ("Added", 0, Some(0))
        ]
    );
    // "One" kept its exact destination and colour: its page didn't change.
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Dest [3 0 R /XYZ 0 500 2]"), "{text}");
    assert!(text.contains("/C [1 0 0]"));

    // Removing every bookmark leaves an empty outline.
    let edited = doc.reopen(bytes).unwrap();
    let bytes = edited.with_bookmarks(&[]).unwrap();
    assert!(reopen(&edited, bytes).is_empty());
}

#[test]
fn bookmarks_can_be_added_to_a_file_without_any() {
    let mut objects = vec!["<< /Type /Catalog /Pages 2 0 R /PageMode /UseNone >>"];
    objects.extend(PAGES);
    let doc = Document::from_bytes(build(&objects), None, Limits::default()).unwrap();
    let items = [
        OutlineItem {
            title: "Start".into(),
            depth: 0,
            page: Some(0),
            id: None,
        },
        // Too deep for where it is: becomes a child of "Start".
        OutlineItem {
            title: "End".into(),
            depth: 5,
            page: Some(2),
            id: None,
        },
    ];
    let bytes = doc.with_bookmarks(&items).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("/PageMode /UseNone"));
    let shown: Vec<(String, u16, Option<usize>)> = reopen(&doc, bytes)
        .into_iter()
        .map(|i| (i.title, i.depth, i.page))
        .collect();
    assert_eq!(shown, [("Start".into(), 0, Some(0)), ("End".into(), 1, Some(2))]);
}
