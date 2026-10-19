#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{
    Changes, Document, FieldKind, FieldValue, Limits, NewField, NewFieldKind, RadioButton, TextBox,
};

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

/// Two pages, the second turned a quarter.
fn blank() -> Document {
    let bytes = build(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Rotate 90 >>",
    ]);
    Document::from_bytes(bytes, None, Limits::default()).unwrap()
}

fn b(x0: f32, y0: f32, x1: f32, y1: f32) -> TextBox {
    TextBox { x0, y0, x1, y1 }
}

fn field(page: usize, rect: TextBox, name: &str, kind: NewFieldKind) -> NewField {
    NewField {
        page,
        rect,
        name: name.into(),
        tooltip: String::new(),
        required: false,
        kind,
    }
}

fn close(a: TextBox, b: TextBox) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)]
        .iter()
        .all(|(x, y)| (x - y).abs() < 0.05)
}

#[test]
fn new_fields_are_found_where_they_were_put_and_can_be_filled() {
    let doc = blank();
    let add = vec![
        field(
            0,
            b(20.0, 30.0, 180.0, 50.0),
            "Name",
            NewFieldKind::Text { multiline: false },
        ),
        field(0, b(20.0, 70.0, 34.0, 84.0), "Agree", NewFieldKind::CheckBox),
        field(
            0,
            b(20.0, 100.0, 120.0, 118.0),
            "Colour",
            NewFieldKind::Dropdown(vec!["Red".into(), "Grün".into()]),
        ),
        field(
            1,
            b(10.0, 200.0, 150.0, 260.0),
            "Name",
            NewFieldKind::Text { multiline: true },
        ),
    ];
    let new = doc.reopen(doc.with_form_fields(&add, &[]).unwrap()).unwrap();
    let fields = new.info().unwrap().fields;
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Name", "Agree", "Colour", "Name 2"]);
    for (f, a) in fields.iter().zip(&add) {
        assert_eq!(f.widgets.len(), 1);
        assert_eq!(f.widgets[0].page, a.page);
        assert!(
            close(f.widgets[0].at, a.rect),
            "{:?} {:?}",
            f.widgets[0].at,
            a.rect
        );
        assert!(f.editable());
    }
    assert!(matches!(fields[0].kind, FieldKind::Text { multiline: false, .. }));
    assert!(matches!(fields[1].kind, FieldKind::CheckBox));
    assert!(matches!(&fields[2].kind, FieldKind::Choice { combo: true, options, .. } if options.len() == 2));
    assert!(matches!(fields[3].kind, FieldKind::Text { multiline: true, .. }));

    // They fill in like any other.
    let on = fields[1].widgets[0].on.clone();
    assert_eq!(on.as_deref(), Some("Yes"));
    let filled = new
        .with_changes(&Changes {
            fields: vec![
                (fields[0].id, FieldValue::Text("Jo".into())),
                (fields[1].id, FieldValue::State(on.clone())),
            ],
            ..Changes::default()
        })
        .unwrap();
    let filled = new.reopen(filled).unwrap();
    let values: Vec<FieldValue> = filled
        .info()
        .unwrap()
        .fields
        .into_iter()
        .map(|f| f.value)
        .collect();
    assert_eq!(values[0], FieldValue::Text("Jo".into()));
    assert_eq!(values[1], FieldValue::State(on));
}

#[test]
fn fields_are_taken_out_of_the_form_and_the_page() {
    let doc = blank();
    let add = vec![
        field(
            0,
            b(20.0, 30.0, 180.0, 50.0),
            "Keep",
            NewFieldKind::Text { multiline: false },
        ),
        field(0, b(20.0, 70.0, 34.0, 84.0), "Drop", NewFieldKind::CheckBox),
    ];
    let new = doc.reopen(doc.with_form_fields(&add, &[]).unwrap()).unwrap();
    let fields = new.info().unwrap().fields;
    let drop = fields.iter().find(|f| f.name == "Drop").unwrap().id;
    // Taking one out and adding another with its name in the same change is fine.
    let again = vec![field(
        1,
        b(10.0, 10.0, 60.0, 30.0),
        "Drop",
        NewFieldKind::CheckBox,
    )];
    let after = new
        .reopen(new.with_form_fields(&again, &[drop]).unwrap())
        .unwrap();
    let fields = after.info().unwrap().fields;
    let found: Vec<(&str, usize)> = fields
        .iter()
        .map(|f| (f.name.as_str(), f.widgets[0].page))
        .collect();
    assert_eq!(found, [("Keep", 0), ("Drop", 1)]);
}

#[test]
fn fields_join_an_existing_encrypted_form() {
    let bytes = include_bytes!("fixtures/encrypted-aes256.pdf");
    let doc = Document::from_bytes(bytes.to_vec(), Some("secret"), Limits::default()).unwrap();
    let before = doc.info().unwrap().fields.len();
    let add = vec![field(
        0,
        b(50.0, 600.0, 200.0, 620.0),
        "Extra",
        NewFieldKind::Text { multiline: false },
    )];
    let saved = doc.with_form_fields(&add, &[]).unwrap();
    assert!(Document::from_bytes(saved.clone(), None, Limits::default()).is_err());
    assert!(
        !saved.windows(7).any(|w| w == b"(Extra)"),
        "the name is in the clear"
    );
    let new = Document::from_bytes(saved, Some("secret"), Limits::default()).unwrap();
    let fields = new.info().unwrap().fields;
    assert_eq!(fields.len(), before + 1);
    assert_eq!(fields.last().unwrap().name, "Extra");
}

#[test]
fn list_boxes_and_radio_groups_are_made_and_filled() {
    let doc = blank();
    let radio = |choice: &str, rect| RadioButton {
        choice: choice.into(),
        rect,
    };
    let add = vec![
        field(
            0,
            b(20.0, 20.0, 120.0, 80.0),
            "Size",
            NewFieldKind::List(vec!["Small".into(), "Large".into()]),
        ),
        field(
            1,
            b(0.0, 0.0, 0.0, 0.0),
            "Pay by",
            NewFieldKind::Radio(vec![
                radio("Card", b(20.0, 30.0, 34.0, 44.0)),
                radio(" Card ", b(20.0, 50.0, 34.0, 64.0)),
                radio("Off", b(20.0, 70.0, 34.0, 84.0)),
                radio("Bank (UK)", b(20.0, 90.0, 34.0, 104.0)),
            ]),
        ),
    ];
    let new = doc.reopen(doc.with_form_fields(&add, &[]).unwrap()).unwrap();
    let fields = new.info().unwrap().fields;
    assert_eq!(fields.len(), 2);
    assert!(matches!(&fields[0].kind, FieldKind::Choice { combo: false, options, .. } if options.len() == 2));
    let group = &fields[1];
    assert_eq!((group.name.as_str(), &group.kind), ("Pay by", &FieldKind::Radio));
    let NewFieldKind::Radio(buttons) = &add[1].kind else {
        unreachable!()
    };
    assert_eq!(group.widgets.len(), 4);
    for (w, button) in group.widgets.iter().zip(buttons) {
        assert_eq!(w.page, 1);
        assert!(close(w.at, button.rect), "{:?} {:?}", w.at, button.rect);
    }
    // Repeated, blank-padded and "Off" choices get names of their own.
    let states: Vec<&str> = group.widgets.iter().map(|w| w.on.as_deref().unwrap()).collect();
    assert_eq!(states, ["Card", "Card 2", "Off 2", "Bank (UK)"]);
    assert_eq!(group.value, FieldValue::State(None));

    // Picking one button turns the group to it.
    let filled = new
        .with_changes(&Changes {
            fields: vec![
                (fields[0].id, FieldValue::Text("Large".into())),
                (group.id, FieldValue::State(Some("Bank (UK)".into()))),
            ],
            ..Changes::default()
        })
        .unwrap();
    let filled = new.reopen(filled).unwrap();
    let values: Vec<FieldValue> = filled
        .info()
        .unwrap()
        .fields
        .into_iter()
        .map(|f| f.value)
        .collect();
    assert_eq!(
        values,
        [
            FieldValue::Text("Large".into()),
            FieldValue::State(Some("Bank (UK)".into()))
        ]
    );

    // Deleting the group takes every button off the page.
    let saved = new.with_form_fields(&[], &[group.id]).unwrap();
    let widgets = saved.windows(16).filter(|w| w == b"/Subtype /Widget").count();
    assert_eq!(widgets, 1, "only the list box is left");
    assert_eq!(new.reopen(saved).unwrap().info().unwrap().fields.len(), 1);
}
