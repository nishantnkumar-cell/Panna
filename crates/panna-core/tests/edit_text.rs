#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Addition, Document, Limits};

/// A page with a pale blue box and dark red text over it.
fn page() -> Document {
    let content = "0.8 0.9 1 rg 10 100 180 40 re f\n\
                   BT /F1 12 Tf 0.6 0 0 rg 20 115 Td (Old words here) Tj ET\n\
                   BT /F1 12 Tf 0 0 0 rg 20 60 Td (Stays the same) Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    Document::from_bytes(out, None, Limits::default()).unwrap()
}

#[test]
fn replacing_text_keeps_what_is_behind_it() {
    let doc = page();
    let text = doc.page_text(0).unwrap();
    let all = text.slice(0..text.len());
    let at = all.find("Old words here").unwrap();
    let boxes = text.rects(at..at + "Old words here".len());
    let area = boxes[0];
    let edited = doc
        .with_additions(
            0,
            &[Addition::ReplaceText {
                area,
                text: "New words".into(),
                size: 12.0,
                color: None,
            }],
        )
        .unwrap();
    let doc = doc.reopen(edited).unwrap();
    let text = doc.page_text(0).unwrap();
    let all = text.slice(0..text.len());
    assert!(all.contains("New words") && !all.contains("Old"), "{all}");
    assert!(all.contains("Stays the same"), "{all}");
    let cache = doc.new_cache();
    let shown = doc.render_page(&cache, 0, 1.0).unwrap();
    let px = |x: usize, y: usize| <[u8; 3]>::try_from(&shown.rgba[(y * 200 + x) * 4..][..3]).unwrap();
    // The box behind is still there, past the end of the new text.
    assert_eq!(px(180, 75), [204, 230, 255]);
    // The new text is in the old text's dark red.
    let row: Vec<[u8; 3]> = (20..80)
        .flat_map(|x| (78..90).map(move |y| (x, y)))
        .map(|(x, y)| px(x, y))
        .collect();
    assert!(
        row.iter().any(|p| p[0] > 120 && p[1] < 40 && p[2] < 40),
        "no red ink"
    );
}
