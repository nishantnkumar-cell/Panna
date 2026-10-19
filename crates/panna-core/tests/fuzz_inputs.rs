#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
//! The fuzzers' seeds and every input that has crashed a fuzzer, run through what the fuzz
//! targets do. Nothing may panic or hang; well-formed seeds must open and draw.

use std::path::{Path, PathBuf};

use panna_core::{Document, Limits, TextBox, import_form_data};

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    out.retain(|p| p.is_file() && p.extension().is_none_or(|e| e != "md"));
    out.sort();
    out
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn limits() -> Limits {
    Limits {
        max_bitmap_side: 512,
        max_bitmap_pixels: 512 * 512,
        max_decoded_bytes: 16 << 20,
        max_image_pixels: 4 << 20,
        ..Limits::default()
    }
}

/// What `open_render` and `analyse` do with a file.
fn exercise(data: &[u8]) -> Option<Document> {
    let doc = Document::from_bytes(data.to_vec(), None, limits()).ok()?;
    let _ = doc.info();
    let cache = doc.new_cache();
    for index in 0..doc.page_count().min(4) {
        let _ = doc.render_page(&cache, index, 0.5);
        let _ = doc.page_text(index);
    }
    let _ = doc.signatures();
    let _ = doc.preflight();
    let _ = doc.accessibility();
    let whole = TextBox {
        x0: 0.0,
        y0: 0.0,
        x1: 10_000.0,
        y1: 10_000.0,
    };
    let saves = [
        doc.redacted(&[(0, vec![whole])], &["seed".to_owned()]),
        doc.flattened(true, true),
        doc.compacted(Some(72)),
        doc.tagged(),
    ];
    for saved in saves.into_iter().flatten() {
        let _ = Document::from_bytes(saved, None, limits());
    }
    Some(doc)
}

#[test]
fn seeds_and_past_crashes_neither_panic_nor_hang() {
    let mut seen = 0;
    for path in [
        root().join("fuzz/seeds/pdf"),
        root().join("crates/panna-core/tests/fuzz-regressions"),
    ]
    .iter()
    .flat_map(|d| files(d))
    {
        let data = std::fs::read(&path).unwrap();
        let _ = exercise(&data);
        seen += 1;
    }
    for path in files(&root().join("fuzz/seeds/data")) {
        let _ = import_form_data(&std::fs::read(path).unwrap());
    }
    assert!(seen >= 10, "the seeds are missing");
}

#[test]
fn well_formed_seeds_open_and_draw() {
    let seeds = root().join("fuzz/seeds/pdf");
    for (name, text) in [
        ("text.pdf", "Hello, seed"),
        ("shapes-and-pictures.pdf", ""),
        ("form-and-comments.pdf", "Name and choice"),
        ("outline-links-attachment.pdf", "Links"),
        ("object-streams.pdf", "Packed"),
        ("incremental-update.pdf", "Changed"),
    ] {
        let data = std::fs::read(seeds.join(name)).unwrap();
        let doc = exercise(&data).unwrap_or_else(|| panic!("{name} didn't open"));
        let cache = doc.new_cache();
        assert!(doc.render_page(&cache, 0, 1.0).is_ok(), "{name}");
        let page = doc.page_text(0).unwrap();
        assert!(page.slice(0..page.len()).contains(text), "{name}");
    }
}
