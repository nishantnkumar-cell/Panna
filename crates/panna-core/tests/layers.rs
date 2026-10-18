#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, Limits};

/// Assembles numbered objects (1-based) into a PDF with a classic xref table.
fn build(objects: &[&str], trailer: &str) -> Vec<u8> {
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
            "trailer\n<< /Size {} /Root 1 0 R {trailer} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// A page with a red square in layer "Red" (shown) and a blue one in "Blue" (hidden at first).
fn layered() -> Vec<u8> {
    let content = "/OC /R BDC 1 0 0 rg 0 0 50 50 re f EMC /OC /B BDC 0 0 1 rg 50 0 50 50 re f EMC";
    let page = format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len());
    build(
        &[
            "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R 6 0 R] /D << /OFF [6 0 R] /Order [5 0 R 6 0 R] >> >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 50] /Contents 4 0 R \
             /Resources << /Properties << /R 5 0 R /B 6 0 R >> >> >>",
            &page,
            "<< /Type /OCG /Name (Red) >>",
            "<< /Type /OCG /Name <FEFF0042006C00750065> >>",
        ],
        "",
    )
}

/// The colour in the middle of the left and right halves.
fn colours(doc: &Document) -> ([u8; 3], [u8; 3]) {
    let cache = doc.new_cache();
    let page = doc.render_page(&cache, 0, 1.0).unwrap();
    let at = |x: usize| {
        let i = (25 * page.width as usize + x) * 4;
        [page.rgba[i], page.rgba[i + 1], page.rgba[i + 2]]
    };
    (at(25), at(75))
}

#[test]
fn layers_are_listed_and_can_be_hidden_or_shown() {
    let doc = Document::from_bytes(layered(), None, Limits::default()).unwrap();
    let layers = doc.info().unwrap().layers;
    let names: Vec<_> = layers.iter().map(|l| (l.name.as_str(), l.shown)).collect();
    assert_eq!(names, [("Red", true), ("Blue", false)]);
    const WHITE: [u8; 3] = [255, 255, 255];
    assert_eq!(colours(&doc), ([255, 0, 0], WHITE));

    let swapped = doc.with_layers_hidden(&[layers[0].id]).unwrap();
    assert_eq!(colours(&swapped), (WHITE, [0, 0, 255]));
    // With none hidden, both show.
    let both = doc.with_layers_hidden(&[]).unwrap();
    assert_eq!(colours(&both), ([255, 0, 0], [0, 0, 255]));
}
