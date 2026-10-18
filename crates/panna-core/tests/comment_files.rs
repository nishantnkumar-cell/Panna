#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Annotation, Changes, Color, DataFormat, Document, Kind, Limits, TextBox};

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

/// Two pages, the second turned a quarter and with its media box away from the origin.
fn blank() -> Document {
    let bytes = build(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [50 50 350 250] /Rotate 90 >>",
    ]);
    Document::from_bytes(bytes, None, Limits::default()).unwrap()
}

fn comments() -> Vec<Annotation> {
    let a = |page, kind| Annotation {
        page,
        kind,
        color: Color::RED,
        opacity: 0.8,
        width: 2.0,
        contents: "Look (here) — पन्ना".into(),
        author: "Jo".into(),
    };
    let b = |x0, y0, x1, y1| TextBox { x0, y0, x1, y1 };
    vec![
        a(0, Kind::Highlight(vec![b(10.0, 10.0, 80.0, 22.0)])),
        a(0, Kind::Note { x: 100.0, y: 40.0 }),
        a(0, Kind::Rect(b(20.0, 50.0, 120.0, 150.0))),
        a(
            1,
            Kind::Line {
                from: (10.0, 10.0),
                to: (150.0, 90.0),
                arrow: true,
            },
        ),
        a(1, Kind::Ink(vec![vec![(5.0, 5.0), (30.0, 40.0), (60.0, 20.0)]])),
        a(
            1,
            Kind::Polygon {
                points: vec![(10.0, 100.0), (90.0, 100.0), (50.0, 180.0)],
                cloud: true,
            },
        ),
    ]
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

/// The shape's numbers, to compare with a little leeway for rounding.
fn numbers(kind: &Kind) -> Vec<f32> {
    let b = |r: &TextBox| vec![r.x0, r.y0, r.x1, r.y1];
    match kind {
        Kind::Highlight(r) => r.iter().flat_map(b).collect(),
        Kind::Note { x, y } => vec![*x, *y],
        Kind::Rect(r) => b(r),
        Kind::Line { from, to, .. } => vec![from.0, from.1, to.0, to.1],
        Kind::Ink(s) => s.iter().flatten().flat_map(|(x, y)| [*x, *y]).collect(),
        Kind::Polygon { points, .. } => points.iter().flat_map(|(x, y)| [*x, *y]).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn comments_travel_through_fdf_and_xfdf() {
    let doc = blank();
    let changes = Changes {
        add: comments(),
        ..Changes::default()
    };
    let marked = doc.reopen(doc.with_changes(&changes).unwrap()).unwrap();
    // What reading the comments back from the PDF gives, which a file should match.
    let saved: Vec<Annotation> = marked
        .info()
        .unwrap()
        .comments
        .into_iter()
        .filter_map(|c| c.editable)
        .collect();
    assert_eq!(saved.len(), comments().len());
    // Saved notes come back where they were put.
    assert!(matches!(saved[1].kind, Kind::Note { x, y } if close(x, 100.0) && close(y, 40.0)));
    for format in [DataFormat::Fdf, DataFormat::Xfdf] {
        let file = marked.comments_file(format, "doc.pdf");
        let (read, skipped) = blank().comments_from_file(&file).unwrap();
        assert_eq!(skipped, 0, "{format:?}");
        assert_eq!(read.len(), saved.len(), "{format:?}");
        for (got, want) in read.iter().zip(&saved) {
            assert_eq!(got.page, want.page);
            assert_eq!(got.contents, want.contents);
            assert_eq!(got.author, want.author);
            assert!(close(got.opacity, want.opacity));
            assert_eq!(
                std::mem::discriminant(&got.kind),
                std::mem::discriminant(&want.kind)
            );
            let (g, w) = (numbers(&got.kind), numbers(&want.kind));
            assert!(
                g.len() == w.len() && g.iter().zip(&w).all(|(a, b)| close(*a, *b)),
                "{format:?} {:?} != {:?}",
                got.kind,
                want.kind
            );
        }
    }
}

#[test]
fn files_that_are_not_comments_are_refused() {
    assert!(blank().comments_from_file(b"not a comment file").is_err());
}
