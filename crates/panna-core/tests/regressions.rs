//! Inputs found by the fuzzer. Each must open or fail cleanly, never crash the process.

use panna_core::{Document, Limits};

fn exercise(bytes: &[u8]) {
    if let Ok(doc) = Document::from_bytes(bytes.to_vec(), None, Limits::default()) {
        let _ = doc.info();
        let cache = doc.new_cache();
        for index in 0..doc.page_count().min(4) {
            let _ = doc.render_page(&cache, index, 0.5);
        }
    }
}

/// hayro-syntax 0.8.0 panics on an `unwrap` in `ArrayIter::next` (object/array.rs:153).
/// Our `catch_unwind` boundary must turn that into an error.
#[test]
fn hayro_array_unwrap() {
    exercise(include_bytes!("regressions/hayro-array-unwrap.pdf"));
}
