#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Changes, Document, FieldKind, FieldValue, Limits};

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

/// A 200pt page with a text field, a checkbox, a two-button radio group and a drop-down.
fn form() -> Vec<u8> {
    build(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R 6 0 R 7 0 R 10 0 R] /DA (/Helv 0 Tf 0 g) >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Annots [5 0 R 6 0 R 8 0 R 9 0 R 10 0 R] >>",
        "<< /Length 0 >>\nstream\n\nendstream",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /TU (Your name) /Rect [10 160 190 180] /DA (/Helv 12 Tf 0 0 1 rg) /V (old) /F 4 >>",
        "<< /Type /Annot /Subtype /Widget /FT /Btn /T (agree) /Rect [10 130 25 145] /V /Off /AS /Off /F 4 /AP << /N << /Yes 11 0 R /Off 12 0 R >> >> >>",
        "<< /FT /Btn /Ff 49152 /T (size) /Kids [8 0 R 9 0 R] /V /S >>",
        "<< /Type /Annot /Subtype /Widget /Parent 7 0 R /Rect [10 100 25 115] /AS /S /F 4 /AP << /N << /S 11 0 R /Off 12 0 R >> >> >>",
        "<< /Type /Annot /Subtype /Widget /Parent 7 0 R /Rect [40 100 55 115] /AS /Off /F 4 /AP << /N << /L 11 0 R /Off 12 0 R >> >> >>",
        "<< /Type /Annot /Subtype /Widget /FT /Ch /Ff 131072 /T (colour) /Opt [[(r) (Red)] [(g) (Green)]] /V (r) /Rect [10 60 120 80] /F 4 >>",
        "<< /Type /XObject /Subtype /Form /BBox [0 0 15 15] /Length 14 >>\nstream\n0 0 15 15 re f\nendstream",
        "<< /Type /XObject /Subtype /Form /BBox [0 0 15 15] /Length 0 >>\nstream\n\nendstream",
    ])
}

fn open(bytes: Vec<u8>) -> Document {
    Document::from_bytes(bytes, None, Limits::default()).unwrap()
}

fn pixel(doc: &Document, x: usize, y: usize) -> [u8; 4] {
    let cache = doc.new_cache();
    let page = doc.render_page(&cache, 0, 1.0).unwrap();
    let i = (y * page.width as usize + x) * 4;
    [page.rgba[i], page.rgba[i + 1], page.rgba[i + 2], page.rgba[i + 3]]
}

#[test]
fn fields_are_read_with_their_values_and_places() {
    let fields = open(form()).info().unwrap().fields;
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["name", "agree", "size", "colour"]);
    let [name, agree, size, colour] = &fields[..] else {
        unreachable!()
    };
    assert!(matches!(name.kind, FieldKind::Text { multiline: false, .. }));
    assert_eq!(
        (name.value.clone(), name.tooltip.as_str(), name.font_size),
        (FieldValue::Text("old".into()), "Your name", 12.0)
    );
    // Top-left view space: PDF y 160–180 on a 200pt page is view y 20–40.
    let at = name.widgets[0].at;
    assert!(
        (at.y0 - 20.0).abs() < 0.01 && (at.x1 - 190.0).abs() < 0.01,
        "{at:?}"
    );
    assert_eq!(
        (agree.kind.clone(), agree.value.clone()),
        (FieldKind::CheckBox, FieldValue::State(None))
    );
    assert_eq!(agree.widgets[0].on.as_deref(), Some("Yes"));
    assert_eq!(size.kind, FieldKind::Radio);
    assert_eq!(size.value, FieldValue::State(Some("S".into())));
    let ons: Vec<_> = size.widgets.iter().map(|w| w.on.clone().unwrap()).collect();
    assert_eq!(ons, ["S", "L"]);
    let FieldKind::Choice { combo, options, .. } = &colour.kind else {
        panic!("{:?}", colour.kind)
    };
    assert!(combo);
    assert_eq!(
        (options[1].export.as_str(), options[1].label.as_str()),
        ("g", "Green")
    );
}

#[test]
fn filled_values_save_and_show() {
    let doc = open(form());
    let fields = doc.info().unwrap().fields;
    // The second radio button and the checkbox start off (white).
    assert_eq!(pixel(&doc, 47, 92), [255, 255, 255, 255]);
    let changes = Changes {
        fields: vec![
            (fields[0].id, FieldValue::Text("Ada (L)".into())),
            (fields[1].id, FieldValue::State(Some("Yes".into()))),
            (fields[2].id, FieldValue::State(Some("L".into()))),
            (fields[3].id, FieldValue::Text("g".into())),
        ],
        ..Changes::default()
    };
    let saved = doc.with_changes(&changes).unwrap();
    let doc = open(saved);
    let after = doc.info().unwrap().fields;
    let values: Vec<FieldValue> = after.iter().map(|f| f.value.clone()).collect();
    assert_eq!(
        values,
        [
            FieldValue::Text("Ada (L)".into()),
            FieldValue::State(Some("Yes".into())),
            FieldValue::State(Some("L".into())),
            FieldValue::Text("g".into()),
        ]
    );
    // Checkbox on, the radio choice moved from the first button to the second.
    assert_eq!(pixel(&doc, 17, 62)[0], 0);
    assert_eq!(pixel(&doc, 47, 92)[0], 0);
    assert_eq!(pixel(&doc, 17, 92), [255, 255, 255, 255]);
    // The name is drawn in the field's blue.
    let blue = (22..38).any(|y| {
        (10..80).any(|x| {
            let [r, _, b, _] = pixel(&doc, x, y);
            b > 150 && r < 100
        })
    });
    assert!(blue, "no text drawn in the name field");
}

#[test]
fn flattened_fields_and_comments_stay_drawn() {
    use panna_core::{Annotation, Color, Kind, TextBox};
    let doc = open(form());
    let fields = doc.info().unwrap().fields;
    let changes = Changes {
        fields: vec![
            (fields[0].id, FieldValue::Text("Ada".into())),
            (fields[1].id, FieldValue::State(Some("Yes".into()))),
        ],
        add: vec![Annotation {
            page: 0,
            kind: Kind::Rect(TextBox {
                x0: 150.0,
                y0: 150.0,
                x1: 190.0,
                y1: 190.0,
            }),
            color: Color([1.0, 0.0, 0.0]),
            opacity: 1.0,
            width: 4.0,
            contents: "Look".into(),
            author: String::new(),
        }],
        ..Changes::default()
    };
    let filled = open(doc.with_changes(&changes).unwrap());
    let before = (pixel(&filled, 17, 62), pixel(&filled, 151, 170));
    assert_eq!(filled.comments().len(), 1);

    // Fields only: the form is gone, the comment stays a comment.
    let fields_only = open(filled.flattened(false, true).unwrap());
    assert!(fields_only.info().unwrap().fields.is_empty());
    assert_eq!(fields_only.comments().len(), 1);
    assert_eq!(
        (pixel(&fields_only, 17, 62), pixel(&fields_only, 151, 170)),
        before
    );

    // Both: nothing left to change, everything still drawn.
    let flat = open(filled.flattened(true, true).unwrap());
    assert!(flat.comments().is_empty());
    assert!(flat.info().unwrap().fields.is_empty());
    assert_eq!((pixel(&flat, 17, 62), pixel(&flat, 151, 170)), before);
    assert_eq!(before.1[..3], [255, 0, 0]);
    assert_eq!(before.0[0], 0);
    let blue = |doc: &Document| {
        (22..38).any(|y| {
            (10..80).any(|x| {
                let [r, _, b, _] = pixel(doc, x, y);
                b > 150 && r < 100
            })
        })
    };
    assert!(blue(&flat), "the filled-in name was lost");
    // A field the viewer was left to draw gets drawn when flattened.
    let untouched = open(open(form()).flattened(false, true).unwrap());
    assert!(blue(&untouched), "the unfilled name field's value was lost");
}
