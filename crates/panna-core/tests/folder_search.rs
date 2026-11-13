#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::folder::{pdfs_under, search};
use panna_core::{FindOptions, Limits};

/// A one-page PDF saying `words`.
fn saying(words: &str) -> Vec<u8> {
    let content = format!("BT /F1 12 Tf 72 700 Td ({words}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
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
fn every_pdf_in_a_folder_and_its_subfolders_is_found_and_searched() {
    let dir = std::env::temp_dir().join(format!("panna-folder-{}", std::process::id()));
    let inner = dir.join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::write(dir.join("a.pdf"), saying("The quarterly invoice is due")).unwrap();
    std::fs::write(inner.join("b.PDF"), saying("No match here")).unwrap();
    std::fs::write(dir.join("notes.txt"), "invoice").unwrap();

    let top = pdfs_under(&dir, false).unwrap();
    assert_eq!(top, [dir.join("a.pdf")]);
    let all = pdfs_under(&dir, true).unwrap();
    assert_eq!(all.len(), 2);

    let found = search(
        &dir.join("a.pdf"),
        "INVOICE",
        FindOptions::default(),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].page, 0);
    assert_eq!(found[0].matched, "invoice");
    assert!(found[0].before.ends_with("quarterly "), "{:?}", found[0].before);
    assert!(found[0].after.starts_with(" is due"), "{:?}", found[0].after);
    let case = FindOptions {
        match_case: true,
        ..FindOptions::default()
    };
    assert!(
        search(&dir.join("a.pdf"), "INVOICE", case, Limits::default())
            .unwrap()
            .is_empty()
    );
    assert!(
        search(
            &inner.join("b.PDF"),
            "invoice",
            FindOptions::default(),
            Limits::default()
        )
        .unwrap()
        .is_empty()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
