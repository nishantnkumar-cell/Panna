#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{CalcOp, Document, Formula, Limits};

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

#[test]
fn standard_calculations_are_read_in_the_forms_order() {
    let widget = |name: &str, y: u32, extra: &str| {
        format!(
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T ({name}) /Rect [10 {y} 100 {}] /P 3 0 R {extra} >>",
            y + 20
        )
    };
    let bytes = build(&[
        // Total is worked out after Line, as /CO says, though it comes first in /Fields.
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [7 0 R 4 0 R 5 0 R 6 0 R] /CO [6 0 R 7 0 R] >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Annots [4 0 R 5 0 R 6 0 R 7 0 R] >>",
        &widget("Price", 10, "/V (12.50)"),
        &widget("Qty", 40, "/V (3)"),
        &widget(
            "Line",
            70,
            "/AA << /C << /S /JavaScript /JS (/** BVCALC Price * Qty EVCALC **/ event.value = 0;) >> >>",
        ),
        &widget(
            "Total",
            100,
            "/AA << /C << /S /JavaScript /JS (AFSimple_Calculate\\(\"SUM\", new Array \\(\"Line\", \"Qty\"\\)\\);) >> >>",
        ),
    ]);
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    let fields = doc.info().unwrap().fields;
    let calc = |name: &str| {
        fields
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .calculation
            .clone()
    };
    assert_eq!(calc("Price"), None);
    let line = calc("Line").unwrap();
    let total = calc("Total").unwrap();
    assert!(line.order < total.order);
    assert_eq!(line.formula.inputs(), ["Price", "Qty"]);
    assert_eq!(
        total.formula,
        Formula::Simple(CalcOp::Sum, vec!["Line".into(), "Qty".into()])
    );
    let values = |n: &str| match n {
        "Price" => vec![12.5],
        "Qty" => vec![3.0],
        _ => Vec::new(),
    };
    assert_eq!(line.formula.evaluate(&values), Some(37.5));
}
