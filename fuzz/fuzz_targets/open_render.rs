//! Feeds arbitrary bytes through the same open + render path the app uses.
//! A crash here is a bug: everything must come back as `Ok` or `Err`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use panna_core::{Document, Limits};

fuzz_target!(|data: &[u8]| {
    // Small bitmaps keep each run fast; limits are what we're testing, not resolution.
    let limits = Limits { max_bitmap_side: 512, max_bitmap_pixels: 512 * 512, ..Limits::default() };
    let Ok(doc) = Document::from_bytes(data.to_vec(), None, limits) else { return };
    let _ = doc.info();
    let cache = doc.new_cache();
    for index in 0..doc.page_count().min(4) {
        let _ = doc.render_page(&cache, index, 0.5);
    }
});
