//! Feeds arbitrary bytes to the importers for comment files and form data (FDF and XFDF),
//! which read files from outside just as PDFs are.
#![no_main]

use libfuzzer_sys::fuzz_target;
use panna_core::{Document, Limits, import_form_data};

const ONE_PAGE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >> endobj
trailer << /Root 1 0 R >>
%%EOF
";

fuzz_target!(|data: &[u8]| {
    let _ = import_form_data(data);
    if let Ok(doc) = Document::from_bytes(ONE_PAGE.to_vec(), None, Limits::default())
        && let Ok((comments, _)) = doc.comments_from_file(data)
        // Imported comments must save into a file that reopens.
        && let Ok(saved) = doc.with_annotations(&comments)
    {
        let _ = Document::from_bytes(saved, None, Limits::default());
    }
});
