#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, FindOptions, Limits, SaveError, pictures_to_pdf};

/// A page of large type, then a picture of it in a PDF of its own, as a scan would be.
fn scanned() -> (Document, Document) {
    let content =
        "BT /F1 28 Tf 72 700 Td (Panna reads scanned pages) Tj 0 -60 Td (Invoice 2026 total 431) Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> >>"
            .to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for o in offsets {
        bytes.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let typed = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    let page = typed.render_page(&typed.new_cache(), 0, 150.0 / 72.0).unwrap();
    let picture = image::RgbaImage::from_raw(page.width, page.height, page.rgba).unwrap();
    let mut png = Vec::new();
    picture
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let scan = pictures_to_pdf(vec![png]).unwrap();
    (
        typed,
        Document::from_bytes(scan, None, Limits::default()).unwrap(),
    )
}

#[test]
fn scanned_text_is_recognised_and_found_where_it_shows() {
    let (typed, scan) = scanned();
    assert!(scan.page_text(0).unwrap().is_empty());
    let (bytes, pages, words) = scan.recognised(&[0]).unwrap();
    assert_eq!(pages, 1);
    assert!(words >= 8, "{words}");
    let done = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    let text = done.page_text(0).unwrap();
    let all = text.slice(0..text.len());
    assert!(all.contains("scanned") && all.contains("431"), "{all}");
    // It can't be seen: the page looks the same.
    let look = |d: &Document| d.render_page(&d.new_cache(), 0, 0.5).unwrap().rgba;
    assert_eq!(look(&done), look(&scan));
    // The word's box is where the word shows (the scan fits the page to A4, so compare
    // relative to the page).
    let size = |d: &Document| d.page_size(0).unwrap();
    let place = |d: &Document, word: &str| {
        let t = d.page_text(0).unwrap();
        let range = t.find(word, FindOptions::default()).into_iter().next().unwrap();
        let r = t.rects(range)[0];
        let s = size(d);
        (r.x0 / s.width, r.y0 / s.height, r.x1 / s.width, r.y1 / s.height)
    };
    let (a, b) = (place(&typed, "Invoice"), place(&done, "Invoice"));
    for (x, y) in [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)] {
        assert!((x - y).abs() < 0.02, "{a:?} {b:?}");
    }
    // Once it has text, there's nothing more to recognise.
    assert!(matches!(
        done.recognised(&[0]),
        Err(SaveError::NothingRecognised(true))
    ));
}
