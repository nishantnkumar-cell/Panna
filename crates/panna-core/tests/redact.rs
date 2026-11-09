#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, FindOptions, Limits, SaveError, TextBox};

fn build(objects: &[&str]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
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

fn stream(dict: &str, data: &str) -> String {
    format!("<< {dict} /Length {} >>\nstream\n{data}\nendstream", data.len())
}

const CONTENT: &str = "BT /F1 20 Tf 20 100 Td (Secret Public) Tj ET \
    q 50 0 0 50 300 20 cm /Im1 Do Q q 1 0 0 1 20 150 cm /Fm1 Do Q";

/// Two pages drawing the same text, picture and form, the second turned a quarter. The first
/// has a note over the picture and one away from everything.
fn sample() -> Document {
    let resources = "/Resources << /Font << /F1 5 0 R >> /XObject << /Im1 6 0 R /Fm1 7 0 R >> >>";
    let bytes = build(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        &format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Contents 8 0 R {resources} \
             /Annots [9 0 R 10 0 R] >>"
        ),
        &format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Rotate 90 /Contents 8 0 R \
             {resources} >>"
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        &stream(
            "/Type /XObject /Subtype /Image /Width 3 /Height 1 /ColorSpace /DeviceRGB \
             /BitsPerComponent 8",
            "SECRETIMG",
        ),
        &stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 40] \
             /Resources << /Font << /F1 5 0 R >> >>",
            "BT /F1 12 Tf 0 10 Td (FormSecret) Tj ET",
        ),
        &stream("", CONTENT),
        "<< /Type /Annot /Subtype /Text /Rect [310 30 330 50] /Contents (Hidden note) >>",
        "<< /Type /Annot /Subtype /Text /Rect [370 170 390 190] /Contents (Kept note) >>",
    ]);
    Document::from_bytes(bytes, None, Limits::default()).unwrap()
}

fn words(doc: &Document, page: usize) -> String {
    let text = doc.page_text(page).unwrap();
    text.slice(0..text.len())
}

/// The boxes of `word` on `page`, in the space areas are marked in.
fn boxes(doc: &Document, page: usize, word: &str) -> Vec<TextBox> {
    let text = doc.page_text(page).unwrap();
    let options = FindOptions {
        match_case: true,
        whole_word: true,
    };
    let found = text.find(word, options);
    assert_eq!(found.len(), 1, "{word} on page {page}");
    text.rects(found[0].clone())
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn marked_text_pictures_forms_and_notes_are_gone() {
    let doc = sample();
    let mut marks = Vec::new();
    let mut public = Vec::new();
    for page in [0, 1] {
        // The picture's area as shown: the image sits at 300..350 by 20..70 on the page.
        let picture = if page == 0 {
            TextBox {
                x0: 300.0,
                y0: 130.0,
                x1: 350.0,
                y1: 180.0,
            }
        } else {
            // Turned a quarter clockwise, the page is 200 wide and 400 tall.
            TextBox {
                x0: 20.0,
                y0: 300.0,
                x1: 70.0,
                y1: 350.0,
            }
        };
        let mut areas = boxes(&doc, page, "Secret");
        areas.extend(boxes(&doc, page, "FormSecret"));
        areas.push(picture);
        marks.push((page, areas));
        public.push(boxes(&doc, page, "Public"));
    }

    // One page alone: the other still shows everything.
    let new = doc.reopen(doc.redacted(&marks[..1], &[]).unwrap()).unwrap();
    assert!(!words(&new, 0).contains("Secret"));
    assert!(words(&new, 1).contains("Secret Public"));

    // Both pages: nothing marked is left anywhere in the file.
    let bytes = doc.redacted(&marks, &[]).unwrap();
    for hidden in [b"Secret".as_slice(), b"SECRETIMG", b"Hidden note"] {
        assert!(!contains(&bytes, hidden), "{}", String::from_utf8_lossy(hidden));
    }
    let new = doc.reopen(bytes).unwrap();
    for page in [0, 1] {
        let text = words(&new, page);
        assert!(!text.contains("Secret"), "{text:?}");
        assert!(text.contains("Public"), "{text:?}");
        // What's left stays where it was.
        let after = boxes(&new, page, "Public");
        assert_eq!(after.len(), public[page].len());
        for (a, b) in public[page].iter().zip(&after) {
            assert!(
                (a.x0 - b.x0).abs() < 0.5 && (a.y0 - b.y0).abs() < 0.5,
                "{a:?} {b:?}"
            );
        }
    }
    let notes: Vec<String> = new.comments().into_iter().map(|c| c.contents).collect();
    assert_eq!(notes, ["Kept note"]);
}

#[test]
fn marked_areas_are_covered_in_black() {
    let doc = sample();
    let area = TextBox {
        x0: 100.0,
        y0: 20.0,
        x1: 200.0,
        y1: 60.0,
    };
    let new = doc
        .reopen(doc.redacted(&[(0, vec![area])], &[]).unwrap())
        .unwrap();
    let cache = new.new_cache();
    let page = new.render_page(&cache, 0, 1.0).unwrap();
    let at = |x: u32, y: u32| {
        let i = ((y * page.width + x) * 4) as usize;
        [page.rgba[i], page.rgba[i + 1], page.rgba[i + 2]]
    };
    assert!(at(150, 40).iter().all(|c| *c < 20));
    assert!(at(250, 40).iter().all(|c| *c > 235));
}

#[test]
fn form_fields_are_not_redacted() {
    let bytes = build(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Annots [4 0 R] >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /Rect [10 10 100 30] /P 3 0 R >>",
    ]);
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    let area = TextBox {
        x0: 0.0,
        y0: 160.0,
        x1: 50.0,
        y1: 200.0,
    };
    assert!(matches!(
        doc.redacted(&[(0, vec![area])], &[]),
        Err(SaveError::RedactsFormField)
    ));
}

#[test]
fn encrypted_files_are_redacted() {
    let fixtures: [&[u8]; 3] = [
        include_bytes!("fixtures/encrypted-rc4.pdf"),
        include_bytes!("fixtures/encrypted-aes128.pdf"),
        include_bytes!("fixtures/encrypted-aes256.pdf"),
    ];
    for bytes in fixtures {
        let doc = Document::from_bytes(bytes.to_vec(), Some("secret"), Limits::default()).unwrap();
        let areas = boxes(&doc, 0, "secret");
        let new = doc.reopen(doc.redacted(&[(0, areas)], &[]).unwrap()).unwrap();
        let text = words(&new, 0);
        assert!(text.contains("Hello") && text.contains("world"), "{text:?}");
        assert!(!text.contains("secret"), "{text:?}");
    }
}

#[test]
fn bookmarks_and_comments_that_mention_redacted_text_can_go_too() {
    let bytes = build(&[
        "<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> /Annots [9 0 R 10 0 R 11 0 R] >>",
        &stream("", "BT /F1 20 Tf 20 100 Td (Acme Ltd pays) Tj ET"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "<< /Type /Outlines /First 7 0 R /Last 8 0 R /Count 2 >>",
        "<< /Title (Payments to ACME  ltd) /Parent 6 0 R /Next 8 0 R /Dest [3 0 R /Fit] >>",
        "<< /Title (Summary) /Parent 6 0 R /Prev 7 0 R /Dest [3 0 R /Fit] >>",
        "<< /Type /Annot /Subtype /Text /Rect [370 170 390 190] /Contents (Ask acme ltd first) >>",
        "<< /Type /Annot /Subtype /Text /Rect [370 10 390 30] /Contents (Kept note) >>",
        "<< /Type /Annot /Subtype /Text /Rect [300 10 320 30] /Contents (Agreed) /IRT 9 0 R >>",
    ]);
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    let text = doc.page_text(0).unwrap();
    let found = text.find("Acme Ltd", FindOptions::default());
    let areas = vec![(0, text.rects(found[0].clone()))];

    let left = doc.leftovers(&areas);
    assert_eq!(left.phrases, vec!["acme ltd".to_owned()]);
    assert_eq!((left.bookmarks, left.comments), (1, 1));

    // Without the phrases, both stay.
    let kept = doc
        .reopen(doc.redacted(&areas, &[]).unwrap())
        .unwrap()
        .info()
        .unwrap();
    assert_eq!(kept.comments.len(), 3);
    assert_eq!(kept.outline[0].title, "Payments to ACME ltd");

    let bytes = doc.redacted(&areas, &left.phrases).unwrap();
    let info = doc.reopen(bytes.clone()).unwrap().info().unwrap();
    let titles: Vec<&str> = info.outline.iter().map(|o| o.title.as_str()).collect();
    assert_eq!(titles, ["Payments to ████ ███", "Summary"]);
    // The comment that mentions it goes, with its reply; the other stays.
    let notes: Vec<&str> = info.comments.iter().map(|c| c.contents.as_str()).collect();
    assert_eq!(notes, ["Kept note"]);
    assert!(!contains(&bytes.to_ascii_lowercase(), b"acme"));
}
