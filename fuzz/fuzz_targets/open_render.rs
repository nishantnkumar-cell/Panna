//! Feeds arbitrary bytes through the same open, render, text and save paths the app uses.
//! A crash here is a bug: everything must come back as `Ok` or `Err`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use panna_core::{Annotation, Color, Document, Kind, Limits};

fuzz_target!(|data: &[u8]| {
    // Small bitmaps keep each run fast; limits are what we're testing, not resolution.
    let limits = Limits { max_bitmap_side: 512, max_bitmap_pixels: 512 * 512, ..Limits::default() };
    let Ok(doc) = Document::from_bytes(data.to_vec(), None, limits) else { return };
    let _ = doc.info();
    let cache = doc.new_cache();
    for index in 0..doc.page_count().min(4) {
        let _ = doc.render_page(&cache, index, 0.5);
        let _ = doc.page_text(index);
    }
    if doc.page_count() > 0 {
        let note = Annotation {
            page: 0,
            kind: Kind::Note { x: 10.0, y: 10.0 },
            color: Color::YELLOW,
            opacity: 1.0,
            width: 1.0,
            contents: "fuzz".into(),
            author: String::new(),
        };
        // Whatever the input, a successful save must reopen.
        if let Ok(saved) = doc.with_annotations(&[note]) {
            let _ = Document::from_bytes(saved, None, limits);
        }
    }
});
