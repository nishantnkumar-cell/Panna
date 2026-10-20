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

/// A form drawing the next one twice, 40 deep: 2^40 runs, which would never finish. Refused at
/// open; a chain drawing each form once opens.
#[test]
fn forms_fanning_out_are_refused() {
    fn chain(fanout: usize) -> Vec<u8> {
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R \
             /Resources << /XObject << /F 5 0 R >> >> >>"
                .to_owned(),
            "<< /Length 5 >>\nstream\n/F Do\nendstream".to_owned(),
        ];
        for level in 0..40 {
            let num = 5 + level;
            let data = if level == 39 {
                "0 0 1 1 re f".to_owned()
            } else {
                "/F Do ".repeat(fanout)
            };
            let res = if level == 39 {
                String::new()
            } else {
                format!("/Resources << /XObject << /F {} 0 R >> >>", num + 1)
            };
            objects.push(format!(
                "<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] {res} /Length {} >>\nstream\n{data}\nendstream",
                data.len()
            ));
        }
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
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
    let refused = Document::from_bytes(chain(2), None, Limits::default());
    assert!(matches!(refused, Err(panna_core::OpenError::Hostile(_))));
    let doc = Document::from_bytes(chain(1), None, Limits::default()).unwrap();
    assert!(doc.render_page(&doc.new_cache(), 0, 1.0).is_ok());
}

/// A page with no resources of its own inherits them from the page tree, where a form draws
/// itself twice. The fan-out check read only the page's own resources, so it counted nothing
/// and finding the page's text never finished.
#[test]
fn forms_through_inherited_resources_are_refused() {
    let bytes = include_bytes!("regressions/forms-through-inherited-resources.pdf");
    assert!(Document::from_bytes(bytes.to_vec(), None, Limits::default()).is_err());
}
