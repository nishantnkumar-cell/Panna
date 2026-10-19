//! Feeds arbitrary bytes through the checks and rewrites that `open_render` doesn't reach:
//! signatures, PDF/A, PDF/X and accessibility checks, comments, attachments, redaction,
//! flattening, compaction and tagging. Every rewritten file must reopen.
#![no_main]

use libfuzzer_sys::fuzz_target;
use panna_core::{Document, Limits, TextBox};

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_bitmap_side: 512,
        max_bitmap_pixels: 512 * 512,
        max_decoded_bytes: 16 << 20,
        max_image_pixels: 4 << 20,
        ..Limits::default()
    };
    let Ok(doc) = Document::from_bytes(data.to_vec(), None, limits) else { return };
    let _ = doc.signatures();
    let _ = doc.preflight();
    let _ = doc.preflight_x();
    let _ = doc.accessibility();
    let _ = doc.comments();
    let _ = doc.attachment(0);
    let whole = TextBox { x0: 0.0, y0: 0.0, x1: 10_000.0, y1: 10_000.0 };
    let saves = [
        doc.redacted(&[(0, vec![whole])]),
        doc.flattened(true, true),
        doc.compacted(Some(72)),
        doc.tagged(),
    ];
    for saved in saves.into_iter().flatten() {
        let _ = Document::from_bytes(saved, None, limits);
    }
});
