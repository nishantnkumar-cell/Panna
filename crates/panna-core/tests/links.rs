#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, Limits, LinkTarget};

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
fn page_jumps_and_web_links_are_read_and_launches_dropped() {
    let pdf = build(&[
        "<< /Type /Catalog /Pages 2 0 R /Dests << /intro [4 0 R /Fit] >> >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Annots [5 0 R 6 0 R 7 0 R 8 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] >>",
        "<< /Type /Annot /Subtype /Link /Rect [10 80 60 90] /Dest [4 0 R /XYZ 0 100 0] >>",
        "<< /Type /Annot /Subtype /Link /Rect [10 60 60 70] /A << /S /URI /URI (https://example.org/) >> >>",
        "<< /Type /Annot /Subtype /Link /Rect [10 40 60 50] /A << /S /Launch /F (calc.exe) >> >>",
        "<< /Type /Annot /Subtype /Link /Rect [10 20 60 30] /A << /S /GoTo /D /intro >> >>",
    ]);
    let links = Document::from_bytes(pdf, None, Limits::default())
        .unwrap()
        .info()
        .unwrap()
        .links;
    let targets: Vec<&LinkTarget> = links.iter().map(|l| &l.target).collect();
    assert_eq!(
        targets,
        [
            &LinkTarget::Page(1),
            &LinkTarget::Uri("https://example.org/".into()),
            &LinkTarget::Page(1),
        ]
    );
    // PDF y 80–90 on a 100pt page is view y 10–20.
    assert!((links[0].at.y0 - 10.0).abs() < 0.01 && (links[0].at.x1 - 60.0).abs() < 0.01);
}
