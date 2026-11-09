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
    let form = "/X Do /X Do";
    let pdf = build(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /XObject << /X 5 0 R >> >> >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>".to_owned(),
        "<< /Length 5 >>\nstream\n/X Do\nendstream".to_owned(),
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] /Length {} >>\nstream\n{form}\nendstream",
            form.len()
        ),
    ]);
    let refused = Document::from_bytes(pdf, None, Limits::default());
    assert!(matches!(refused, Err(panna_core::OpenError::Hostile(_))));
}

/// A PDF of `objects`, numbered from 1, the first being the catalog.
fn build(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    let size = objects.len() + 1;
    out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

/// The fuzzed file that first showed the bug above. hayro 0.9 reads its damaged page tree
/// differently and finds no page contents to draw, so it may now open; either way, it must
/// not hang.
#[test]
fn the_fuzzed_inherited_resources_file_finishes() {
    let bytes = include_bytes!("regressions/forms-through-inherited-resources.pdf");
    let started = std::time::Instant::now();
    if let Ok(doc) = Document::from_bytes(bytes.to_vec(), None, Limits::default()) {
        let _ = doc.accessibility();
        for page in 0..doc.page_count() {
            let _ = doc.page_text(page);
            let _ = doc.render_page(&doc.new_cache(), page, 1.0);
        }
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
}

/// A checkbox's appearance draws a form that draws itself twice (through the page's
/// resources, as a form without its own falls back on them). The engine draws
/// annotations' appearances with the page, but the fan-out check counted only the page's
/// content, so finding the page's text never finished.
#[test]
fn forms_in_annotation_appearances_are_refused() {
    let form = "/X Do /X Do";
    let pdf = build(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Annots [4 0 R] \
         /Resources << /XObject << /X 5 0 R >> >> >>"
            .to_owned(),
        "<< /Type /Annot /Subtype /Widget /FT /Btn /Rect [0 0 10 10] /AS /On \
         /AP << /N << /On 5 0 R /Off 5 0 R >> >> >>"
            .to_owned(),
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] /Length {} >>\nstream\n{form}\nendstream",
            form.len()
        ),
    ]);
    let refused = Document::from_bytes(pdf, None, Limits::default());
    assert!(matches!(refused, Err(panna_core::OpenError::Hostile(_))));
}

/// The fuzzed file that first showed the bug above: hayro 0.9 reads it differently, so it
/// may now open, but the checks run on it must still finish.
#[test]
fn the_fuzzed_annotation_appearance_file_finishes() {
    let bytes = include_bytes!("regressions/forms-in-annotation-appearance.pdf");
    let started = std::time::Instant::now();
    if let Ok(doc) = Document::from_bytes(bytes.to_vec(), None, Limits::default()) {
        let _ = doc.accessibility();
        for page in 0..doc.page_count() {
            let _ = doc.page_text(page);
            let _ = doc.render_page(&doc.new_cache(), page, 1.0);
        }
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
}

/// A page whose `res` resources hold, among others, a form `X` (object 5) that draws itself
/// twice, and whose content is `content`; `extra` objects follow from number 6.
fn fanning_page(res: &str, content: &str, extra: &[String]) -> Vec<u8> {
    let form = "/X Do /X Do";
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R \
             /Resources << {res} /XObject << /X 5 0 R >> >> >>"
        ),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] /Length {} >>\nstream\n{form}\nendstream",
            form.len()
        ),
    ];
    objects.extend_from_slice(extra);
    build(&objects)
}

fn assert_refused(pdf: Vec<u8>) {
    let refused = Document::from_bytes(pdf, None, Limits::default());
    assert!(matches!(refused, Err(panna_core::OpenError::Hostile(_))));
}

/// Forms fanning out reached through a Type 3 font's glyph procedure, a tiling pattern's
/// cell and a soft mask's group: each made drawing the page never finish, because the
/// fan-out check followed only `Do`.
#[test]
fn forms_through_glyphs_patterns_and_masks_are_refused() {
    let glyph = "1000 0 d0 /X Do";
    assert_refused(fanning_page(
        "/Font << /T 6 0 R >>",
        "BT /T 10 Tf (aaaa) Tj ET",
        &[
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [0.001 0 0 0.001 0 0] \
             /CharProcs << /a 7 0 R >> /Encoding << /Differences [97 /a] >> /FirstChar 97 \
             /LastChar 97 /Widths [1000] >>"
                .to_owned(),
            format!("<< /Length {} >>\nstream\n{glyph}\nendstream", glyph.len()),
        ],
    ));
    let cell = "/X Do";
    assert_refused(fanning_page(
        "/Pattern << /P 6 0 R >>",
        "/Pattern cs /P scn 0 0 100 100 re f",
        &[format!(
            "<< /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 \
             /Resources << /XObject << /X 5 0 R >> >> /Length {} >>\nstream\n{cell}\nendstream",
            cell.len()
        )],
    ));
    assert_refused(fanning_page(
        "/ExtGState << /G 6 0 R >>",
        "/G gs 0 0 100 100 re f",
        &[
            "<< /Type /ExtGState /SMask << /S /Luminosity /G 7 0 R >> >>".to_owned(),
            format!(
                "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency >> \
                 /Resources << /XObject << /X 5 0 R >> >> /Length {} >>\nstream\n{cell}\nendstream",
                cell.len()
            ),
        ],
    ));
}
