#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{
    Annotation, Changes, Color, Comment, CommentKind, Document, Kind, Limits, ReviewState, TextBox,
};

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
fn image_stamps_draw_their_picture_upright() {
    // Left half red and opaque, right half see-through.
    let (w, h) = (4u32, 2u32);
    let mut rgba = Vec::new();
    for _ in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(if x < w / 2 {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 255, 0]
            });
        }
    }
    let mut png = Vec::new();
    image::RgbaImage::from_raw(w, h, rgba)
        .unwrap()
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let picture = panna_core::StampImage::decode(&png).unwrap();
    let rect = TextBox {
        x0: 20.0,
        y0: 20.0,
        x1: 180.0,
        y1: 80.0,
    };
    for turned in [false, true] {
        let page = if turned {
            objects(false)[2].replace("/MediaBox", "/Rotate 90 /MediaBox")
        } else {
            objects(false)[2].clone()
        };
        let mut objs = objects(false);
        objs[2] = page;
        let doc = reopen(build(&objs));
        // On the turned page, the box as shown is 100 wide and 200 tall.
        let rect = if turned {
            TextBox {
                x0: 10.0,
                y0: 20.0,
                x1: 90.0,
                y1: 180.0,
            }
        } else {
            rect
        };
        let saved = doc
            .with_annotations(&[annotation(Kind::Stamp {
                rect,
                name: "Signature".into(),
                image: Some(picture.clone()),
            })])
            .unwrap();
        let doc = reopen(saved);
        let left = (
            (rect.x0 + rect.width() * 0.2) as usize,
            (rect.y0 + rect.height() / 2.0) as usize,
        );
        let right = (
            (rect.x0 + rect.width() * 0.8) as usize,
            (rect.y0 + rect.height() / 2.0) as usize,
        );
        let red = pixel(&doc, left.0, left.1);
        assert!(red[0] > 200 && red[2] < 60, "turned {turned}: {red:?}");
        // The see-through half shows the white page.
        assert_eq!(
            pixel(&doc, right.0, right.1),
            [255, 255, 255, 255],
            "turned {turned}"
        );
        // It lists as a stamp but can't be redrawn as words.
        let comments = doc.info().unwrap().comments;
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].kind, CommentKind::Stamp);
        assert!(comments[0].editable.is_none());
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
        Kind::Stamp {
            rect: TextBox {
                x0: 20.0,
                y0: 50.0,
                x1: 140.0,
                y1: 80.0,
            },
            name: "Approved".into(),
            image: None,
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
        "Stamp",
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
    assert!(page_text.slice(0..page_text.len()).contains("APPROVED"));
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
    let polygon = vec![(20.0, 20.0), (80.0, 30.0), (60.0, 80.0)];
    let shapes = [
        annotation(Kind::Rect(rect)),
        line,
        annotation(Kind::Polygon {
            points: polygon.clone(),
            cloud: false,
        }),
        annotation(Kind::Polygon {
            points: polygon,
            cloud: true,
        }),
        annotation(Kind::Stamp {
            rect,
            name: "NotApproved".into(),
            image: None,
        }),
        annotation(Kind::Stamp {
            rect,
            name: "Seen (Jo) é".into(),
            image: None,
        }),
    ];
    let saved = doc.with_annotations(&shapes).unwrap();
    let listed = reopen(saved).info().unwrap().comments;
    for (original, comment) in shapes.iter().zip(&listed) {
        let back = comment
            .editable
            .as_ref()
            .expect("Panna's own comments are editable");
        assert_eq!((back.color, &back.author), (original.color, &original.author));
        // Stamps have no line width of their own.
        if !matches!(original.kind, Kind::Stamp { .. }) {
            assert_eq!(back.width, original.width);
        }
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
            (Kind::Polygon { points, cloud }, Kind::Polygon { points: p, cloud: c }) => {
                assert_eq!((points.len(), cloud), (p.len(), c));
                for (a, b) in points.iter().zip(p) {
                    assert!((a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01, "{p:?}");
                }
                let label = if *cloud { "Cloud" } else { "Polygon" };
                assert_eq!(comment.kind.label(), label);
            }
            (Kind::Stamp { rect: a, name, .. }, Kind::Stamp { rect: b, name: n, .. }) => {
                assert_eq!(name, n);
                assert!((a.x0 - b.x0).abs() < 0.01 && (a.y1 - b.y1).abs() < 0.01, "{b:?}");
                assert_eq!(comment.kind.label(), "Stamp");
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

#[test]
fn replies_and_statuses_join_the_comment_thread() {
    let doc = reopen(build(&objects(true)));
    let note = doc.info().unwrap().comments.remove(0);
    let parent = note.id.unwrap().annot;
    let reply = |contents: &str, state| {
        let mut a = annotation(Kind::Reply {
            to: parent,
            at: note.at,
            state,
        });
        a.contents = contents.into();
        a
    };
    let saved = doc
        .with_annotations(&[
            reply("Agreed", None),
            reply("Accepted set by Tester", Some(ReviewState::Accepted)),
        ])
        .unwrap();
    let text = String::from_utf8_lossy(&saved).into_owned();
    assert!(text.contains("/IRT 5 0 R"), "{text}");
    assert!(text.contains("/State (Accepted) /StateModel (Review)"), "{text}");
    let doc = reopen(saved);
    let listed = doc.info().unwrap().comments;
    assert_eq!(listed.len(), 3);
    assert_eq!(
        (
            listed[1].kind.clone(),
            listed[1].reply_to,
            listed[1].state,
            listed[1].contents.as_str()
        ),
        (CommentKind::Reply, Some(parent), None, "Agreed")
    );
    assert_eq!(
        (listed[2].reply_to, listed[2].state),
        (Some(parent), Some(ReviewState::Accepted))
    );
    // Replies have no look of their own on the page.
    assert!(!text.contains("/F 32 /AP"));
}
