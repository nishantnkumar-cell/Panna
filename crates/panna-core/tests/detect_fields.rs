#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, Limits, NewFieldKind};

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

fn stream(dict: &str, data: &str) -> String {
    format!("<< {dict} /Length {} >>\nstream\n{data}\nendstream", data.len())
}

/// A paper form: a blank of underscores, a line to write on, a box to tick, a box for
/// comments, a table heading in a box (not a field), and a line that already has a field.
const CONTENT: &str = "BT /F1 12 Tf 50 740 Td (Full name: ________________) Tj ET \
    BT /F1 12 Tf 50 700 Td (Email:) Tj ET 100 698 m 300 698 l S \
    50 658 12 12 re S BT /F1 12 Tf 70 660 Td (Send me news) Tj ET \
    BT /F1 12 Tf 50 620 Td (Comments) Tj ET 50 520 300 90 re S \
    50 470 200 30 re S BT /F1 12 Tf 60 480 Td (Heading) Tj ET \
    BT /F1 12 Tf 50 440 Td (Phone:) Tj ET 100 438 m 300 438 l S";

fn form(with_field: bool) -> Document {
    let annots = if with_field { "/Annots [6 0 R]" } else { "" };
    let acroform = if with_field {
        "/AcroForm << /Fields [6 0 R] >>"
    } else {
        ""
    };
    let bytes = build(&[
        &format!("<< /Type /Catalog /Pages 2 0 R {acroform} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        &format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> >> {annots} >>"
        ),
        &stream("", CONTENT),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (phone) /Rect [100 438 300 454] /P 3 0 R >>",
    ]);
    Document::from_bytes(bytes, None, Limits::default()).unwrap()
}

#[test]
fn blanks_lines_and_boxes_become_named_fields() {
    let found = form(false).detect_fields().unwrap();
    let summary: Vec<(&str, &NewFieldKind)> = found.iter().map(|f| (f.name.as_str(), &f.kind)).collect();
    assert_eq!(
        summary,
        [
            ("Full name", &NewFieldKind::Text { multiline: false }),
            ("Email", &NewFieldKind::Text { multiline: false }),
            ("Send me news", &NewFieldKind::CheckBox),
            ("Comments", &NewFieldKind::Text { multiline: true }),
            ("Phone", &NewFieldKind::Text { multiline: false }),
        ]
    );
    // The line's field sits on it, as shown (from the top).
    let email = &found[1].rect;
    assert!(
        (email.x0 - 100.0).abs() < 1.0 && (email.x1 - 300.0).abs() < 1.0,
        "{email:?}"
    );
    assert!(
        (email.y1 - 94.0).abs() < 1.0 && email.height() > 10.0,
        "{email:?}"
    );
    let check = &found[2].rect;
    assert!(
        (check.width() - 12.0).abs() < 1.0 && (check.x0 - 50.0).abs() < 1.0,
        "{check:?}"
    );
}

#[test]
fn places_with_a_field_are_left_alone_and_found_fields_can_be_added() {
    let doc = form(true);
    let found = doc.detect_fields().unwrap();
    assert!(!found.iter().any(|f| f.name == "Phone"), "{found:?}");
    let new = doc.reopen(doc.with_form_fields(&found, &[]).unwrap()).unwrap();
    assert_eq!(new.info().unwrap().fields.len(), found.len() + 1);
    // Nothing more is found once they're there.
    assert!(new.detect_fields().unwrap().is_empty());
}
