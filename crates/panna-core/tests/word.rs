#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
//! Word export keeps pictures, tables and the page's layout.

use panna_core::{Export, ExportFormat, Limits, RenderWorker};

/// A landscape page with a centred title, a paragraph, a table and a picture, then a portrait
/// page of plain text.
fn report() -> Vec<u8> {
    let mut page1 = String::from("BT /F1 24 Tf 300 540 Td (Quarterly report) Tj ET\n");
    for (i, line) in [
        "Sales went up in every region this quarter, led by the north, where two new",
        "shops opened. The table below gives the figures, and the chart after it shows",
        "how they compare with last year.",
    ]
    .iter()
    .enumerate()
    {
        page1.push_str(&format!("BT /F1 11 Tf 72 {} Td ({line}) Tj ET\n", 500 - 14 * i));
    }
    for (i, row) in [
        ["Region", "Shops", "Sales"],
        ["North", "12", "4,200"],
        ["South", "9", "3,100"],
        ["East", "7", "2,950"],
    ]
    .iter()
    .enumerate()
    {
        let y = 420 - 15 * i;
        for (x, cell) in [72, 220, 360].iter().zip(row) {
            page1.push_str(&format!("BT /F1 11 Tf {x} {y} Td ({cell}) Tj ET\n"));
        }
    }
    page1.push_str("q 240 0 0 120 72 220 cm /Im1 Do Q\n");
    page1.push_str("BT /F1 11 Tf 72 180 Td (After the chart.) Tj ET\n");
    let page2 = "BT /F1 11 Tf 72 720 Td (The second page.) Tj ET";
    // A 4 x 2 picture: red, green, blue and white squares over black.
    let pixels: Vec<u8> = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]]
        .iter()
        .chain([[0, 0, 0]; 4].iter())
        .flatten()
        .copied()
        .collect();
    let mut image = format!(
        "<< /Type /XObject /Subtype /Image /Width 4 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
        pixels.len()
    )
    .into_bytes();
    image.extend(&pixels);
    image.extend(b"\nendstream");
    let stream = |s: &str| format!("<< /Length {} >>\nstream\n{s}\nendstream", s.len()).into_bytes();
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 792 612] /Contents 4 0 R \
           /Resources << /Font << /F1 7 0 R >> /XObject << /Im1 8 0 R >> >> >>"
            .to_vec(),
        stream(&page1),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R /Resources << /Font << /F1 7 0 R >> >> >>"
            .to_vec(),
        stream(page2),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        image,
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[test]
fn word_export_keeps_pictures() {
    let dir = std::env::temp_dir().join(format!("panna-word-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("report.pdf");
    std::fs::write(&pdf, report()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = RenderWorker::open(pdf, None, Limits::default(), move |r| {
        tx.send(r.is_ok()).unwrap();
    })
    .unwrap();
    assert!(rx.recv().unwrap());
    let docx = dir.join("report.docx");
    let (tx, rx) = std::sync::mpsc::channel();
    worker.export(
        Export {
            format: ExportFormat::Word,
            dpi: 0.0,
            pages: vec![(0, docx.clone()), (1, docx.clone())],
        },
        move |r| tx.send(r).unwrap(),
    );
    rx.recv().unwrap().unwrap();
    let bytes = std::fs::read(&docx).unwrap();
    assert!(bytes.starts_with(b"PK\x03\x04"));
    // Names in a ZIP aren't compressed: one picture, cut from the page.
    let has = |name: &[u8]| bytes.windows(name.len()).any(|w| w == name);
    assert!(has(b"word/media/image1.png"));
    assert!(!has(b"word/media/image2.png"));
    if std::env::var_os("PANNA_KEEP_WORD").is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}
