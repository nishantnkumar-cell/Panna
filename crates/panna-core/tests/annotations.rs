#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Annotation, Changes, Color, Comment, CommentKind, Document, Kind, Limits, TextBox};

/// Assembles numbered objects (1-based) into a PDF with a classic xref table.
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

/// The same objects, indexed by a cross-reference stream instead of a table.
fn build_with_xref_stream(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
    }
    let xref = out.len();
    let own = objects.len() + 1;
    let mut rows = vec![0u8, 0, 0, 0, 0, 255, 255];
    for offset in offsets.iter().chain([&xref]) {
        rows.push(1);
        rows.extend_from_slice(&(*offset as u32).to_be_bytes());
        rows.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(
        format!(
            "{own} 0 obj\n<< /Type /XRef /Size {} /W [1 4 2] /Root 1 0 R /Length {} >>\nstream\n",
            own + 1,
            rows.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&rows);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    out
}

fn objects(existing_annot: bool) -> Vec<String> {
    let annots = if existing_annot { "/Annots [5 0 R]" } else { "" };
    vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R {annots} >>"),
        "<< /Length 0 >>\nstream\n\nendstream".into(),
        "<< /Type /Annot /Subtype /Text /Rect [0 0 1 1] /Contents (old) >>".into(),
    ]
}

fn annotation(kind: Kind) -> Annotation {
    Annotation {
        page: 0,
        kind,
        color: Color::RED,
        opacity: 1.0,
        width: 4.0,
        contents: "Hello (world)".into(),
        author: "Tester".into(),
    }
}

fn pixel(doc: &Document, x: usize, y: usize) -> [u8; 4] {
    let cache = doc.new_cache();
    let page = doc.render_page(&cache, 0, 1.0).unwrap();
    let i = (y * page.width as usize + x) * 4;
    [page.rgba[i], page.rgba[i + 1], page.rgba[i + 2], page.rgba[i + 3]]
}

fn reopen(bytes: Vec<u8>) -> Document {
    Document::from_bytes(bytes, None, Limits::default()).unwrap()
}

#[test]
fn added_annotations_render_after_reopening() {
    for bytes in [build(&objects(false)), build_with_xref_stream(&objects(false))] {
        let doc = Document::from_bytes(bytes.clone(), None, Limits::default()).unwrap();
        assert_eq!(pixel(&doc, 50, 50), [255, 255, 255, 255]);
        let saved = doc
            .with_annotations(&[annotation(Kind::Rect(TextBox {
                x0: 40.0,
                y0: 40.0,
                x1: 160.0,
                y1: 60.0,
            }))])
            .unwrap();
        // The original revision is untouched.
        assert_eq!(&saved[..bytes.len()], &bytes[..]);
        let doc = reopen(saved);
        // The rectangle's left edge (4pt wide, inside x 40–44) is red; its middle is still white.
        let edge = pixel(&doc, 42, 50);
        assert!(edge[0] > 180 && edge[1] < 80, "{edge:?}");
        assert_eq!(pixel(&doc, 100, 50), [255, 255, 255, 255]);
    }
}

#[test]
fn every_kind_saves_and_reopens() {
    let b = TextBox {
        x0: 20.0,
        y0: 20.0,
        x1: 120.0,
        y1: 40.0,
    };
    let kinds = vec![
        Kind::Highlight(vec![b]),
        Kind::Underline(vec![b]),
        Kind::StrikeOut(vec![b]),
        Kind::Note { x: 150.0, y: 10.0 },
        Kind::FreeText { rect: b, size: 12.0 },
        Kind::Ink(vec![
            vec![(10.0, 80.0), (60.0, 90.0), (90.0, 70.0)],
            vec![(5.0, 5.0)],
        ]),
        Kind::Ellipse(b),
        Kind::Line {
            from: (10.0, 10.0),
            to: (190.0, 90.0),
            arrow: true,
        },
    ];
    let doc = reopen(build(&objects(true)));
    let all: Vec<Annotation> = kinds.into_iter().map(annotation).collect();
    let saved = doc.with_annotations(&all).unwrap();
    let text = String::from_utf8_lossy(&saved);
    for subtype in [
        "Highlight",
        "Underline",
        "StrikeOut",
        "/Text",
        "FreeText",
        "Ink",
        "Circle",
        "Line",
    ] {
        assert!(
            text.contains(&format!("/Subtype /{}", subtype.trim_start_matches('/'))),
            "{subtype}"
        );
    }
    // The existing annotation is kept alongside the new ones.
    assert!(text.contains("/Annots [5 0 R 6 0 R"), "{text}");
    let doc = reopen(saved);
    assert_eq!(doc.page_count(), 1);
    // The free text is drawn and extractable.
    let page_text = doc.page_text(0).unwrap();
    assert!(page_text.slice(0..page_text.len()).contains("Hello (world)"));
}

#[test]
fn saving_twice_chains_updates() {
    let doc = reopen(build(&objects(false)));
    let once = doc
        .with_annotations(&[annotation(Kind::Note { x: 10.0, y: 10.0 })])
        .unwrap();
    let doc = reopen(once.clone());
    let twice = doc
        .with_annotations(&[annotation(Kind::Note { x: 50.0, y: 10.0 })])
        .unwrap();
    assert_eq!(&twice[..once.len()], &once[..]);
    let text = String::from_utf8_lossy(&twice);
    assert_eq!(text.matches("/Subtype /Text").count(), 3);
    let doc = reopen(twice);
    let p = pixel(&doc, 52, 20);
    assert!(p[0] > 150 && p[1] < 120, "second note is drawn: {p:?}");
}

#[test]
fn nothing_to_add_returns_the_original() {
    let bytes = build(&objects(false));
    let doc = reopen(bytes.clone());
    assert_eq!(doc.with_annotations(&[]).unwrap(), bytes);
}

#[test]
fn saved_comments_are_listed_after_reopening() {
    let doc = reopen(build(&objects(true)));
    // The existing sticky note sits at PDF y 0–1, the bottom of a 100pt page: view y 99–100.
    let before = doc.info().unwrap().comments;
    assert_eq!(before.len(), 1);
    assert_eq!(
        (&before[0].kind, before[0].contents.as_str()),
        (&CommentKind::Note, "old")
    );
    assert!((before[0].at.y0 - 99.0).abs() < 0.01, "{:?}", before[0].at);

    let rect = TextBox {
        x0: 40.0,
        y0: 10.0,
        x1: 160.0,
        y1: 30.0,
    };
    let added = annotation(Kind::Rect(rect));
    let saved = doc.with_annotations(std::slice::from_ref(&added)).unwrap();
    let after = reopen(saved).info().unwrap().comments;
    assert_eq!(after.len(), 2);
    let listed = &after[1];
    assert_eq!(listed.kind, CommentKind::Rect);
    assert_eq!(
        (listed.contents.as_str(), listed.author.as_str()),
        ("Hello (world)", "Tester")
    );
    // What the app shows before saving matches what the file says afterwards, within the
    // appearance's padding for the stroke.
    let fresh = Comment::from(&added);
    assert!((fresh.at.x0 - listed.at.x0).abs() <= 3.0 && (fresh.at.y1 - listed.at.y1).abs() <= 3.0);
}

#[test]
fn saved_comments_come_back_editable() {
    let doc = reopen(build(&objects(false)));
    let rect = TextBox {
        x0: 40.0,
        y0: 10.0,
        x1: 160.0,
        y1: 30.0,
    };
    let mut line = annotation(Kind::Line {
        from: (10.0, 20.0),
        to: (90.0, 70.0),
        arrow: true,
    });
    line.opacity = 0.5;
    let shapes = [annotation(Kind::Rect(rect)), line];
    let saved = doc.with_annotations(&shapes).unwrap();
    let listed = reopen(saved).info().unwrap().comments;
    for (original, comment) in shapes.iter().zip(&listed) {
        let back = comment
            .editable
            .as_ref()
            .expect("Panna's own comments are editable");
        assert_eq!(
            (back.color, back.width, &back.author),
            (original.color, original.width, &original.author)
        );
        assert!((back.opacity - original.opacity).abs() < 0.01);
        match (&original.kind, &back.kind) {
            (Kind::Rect(a), Kind::Rect(b)) => {
                assert!((a.x0 - b.x0).abs() < 0.01 && (a.y1 - b.y1).abs() < 0.01, "{b:?}");
            }
            (
                Kind::Line { from, to, arrow },
                Kind::Line {
                    from: f,
                    to: t,
                    arrow: r,
                },
            ) => {
                assert!((from.0 - f.0).abs() < 0.01 && (to.1 - t.1).abs() < 0.01 && arrow == r);
            }
            other => panic!("came back as {other:?}"),
        }
    }
}

#[test]
fn highlights_keep_their_marked_areas() {
    let doc = reopen(build(&objects(false)));
    let line = TextBox {
        x0: 20.0,
        y0: 30.0,
        x1: 120.0,
        y1: 42.0,
    };
    let saved = doc
        .with_annotations(&[annotation(Kind::Highlight(vec![line]))])
        .unwrap();
    let listed = reopen(saved).info().unwrap().comments;
    let marked = &listed[0].marked;
    assert_eq!(marked.len(), 1, "{listed:?}");
    let m = marked[0];
    assert!(
        (m.x0 - line.x0).abs() < 0.01 && (m.y0 - line.y0).abs() < 0.01 && (m.y1 - line.y1).abs() < 0.01,
        "{m:?}"
    );
}

#[test]
fn removing_a_saved_comment_takes_its_popup_too() {
    // A note (5) with its pop-up (6), and a square (7) that must survive.
    let mut objs = objects(false);
    objs[2] = objs[2].replace("/Contents 4 0 R", "/Contents 4 0 R /Annots [5 0 R 6 0 R 7 0 R]");
    objs[4] = "<< /Type /Annot /Subtype /Text /Rect [0 0 1 1] /Contents (old) /Popup 6 0 R >>".into();
    objs.push("<< /Type /Annot /Subtype /Popup /Rect [0 0 50 50] /Parent 5 0 R >>".into());
    objs.push("<< /Type /Annot /Subtype /Square /Rect [10 10 50 50] /Contents (keep) >>".into());
    let doc = reopen(build(&objs));
    let listed = doc.info().unwrap().comments;
    assert_eq!(listed.len(), 2, "{listed:?}");
    let note = listed.iter().find(|c| c.kind == CommentKind::Note).unwrap();
    let id = note.id.unwrap();
    assert_eq!((id.annot.number, id.popup.map(|p| p.number)), (5, Some(6)));

    let added = annotation(Kind::Rect(TextBox {
        x0: 100.0,
        y0: 10.0,
        x1: 150.0,
        y1: 30.0,
    }));
    let saved = doc
        .with_changes(&Changes {
            add: vec![added],
            remove: vec![id],
            ..Changes::default()
        })
        .unwrap();
    let text = String::from_utf8_lossy(&saved);
    let tail = &text[text.rfind("/Annots [").unwrap()..];
    let annots = &tail[..tail.find(']').unwrap() + 1];
    assert!(annots.starts_with("/Annots [7 0 R "), "{annots}");
    let after = reopen(saved).info().unwrap().comments;
    let kinds: Vec<_> = after
        .iter()
        .map(|c| (c.kind.clone(), c.contents.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [(CommentKind::Rect, "keep"), (CommentKind::Rect, "Hello (world)")]
    );
}
