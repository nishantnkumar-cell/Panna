#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::batch::{Step, pdfs_in, run};
use panna_core::{Document, Limits};

/// One page with a note on it.
fn page() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Annots [4 0 R] >>",
        "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (Hi) >>",
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

fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("panna-batch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn every_pdf_in_a_folder_gets_the_steps_and_keeps_its_name() {
    let (from, to) = (folder("from"), folder("to"));
    std::fs::write(from.join("b.pdf"), page()).unwrap();
    std::fs::write(from.join("a.PDF"), page()).unwrap();
    std::fs::write(from.join("notes.txt"), "not a pdf").unwrap();
    let files = pdfs_in(&from).unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0].ends_with("a.PDF"));
    let steps = [Step::PageNumbers, Step::Clean, Step::Compact];
    for file in &files {
        run(file, &steps, &to, Limits::default()).unwrap();
    }
    let out = Document::open(&to.join("b.pdf"), None, Limits::default()).unwrap();
    let text = out.page_text(0).unwrap();
    assert!(text.slice(0..text.len()).contains("Page 1 of 1"));
    assert!(
        out.comments().is_empty(),
        "removing hidden information takes the note"
    );
    // A second run leaves the first results alone.
    let again = run(&files[1], &steps, &to, Limits::default());
    assert!(again.unwrap_err().contains("already"));
    // And results can't overwrite the files they come from.
    assert!(run(&files[1], &steps, &from, Limits::default()).is_err());
    let _ = std::fs::remove_dir_all(from);
    let _ = std::fs::remove_dir_all(to);
}

#[test]
fn a_damaged_file_is_reported_not_saved() {
    let (from, to) = (folder("bad"), folder("bad-to"));
    std::fs::write(from.join("broken.pdf"), b"%PDF-1.7\nnot really").unwrap();
    assert!(run(&from.join("broken.pdf"), &[Step::Compact], &to, Limits::default()).is_err());
    assert!(!to.join("broken.pdf").exists());
    let _ = std::fs::remove_dir_all(from);
    let _ = std::fs::remove_dir_all(to);
}
