#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Cleanup, Document, Limits};

/// Assembles numbered objects (1-based) into a PDF with a classic xref table.
fn build(objects: &[&str], trailer: &str) -> Vec<u8> {
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
            "trailer\n<< /Size {} /Root 1 0 R {trailer} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// A page with everything that hides: metadata, scripts, an attachment, comments, a link and
/// a field with a script.
fn busy() -> Vec<u8> {
    build(
        &[
            "<< /Type /Catalog /Pages 2 0 R /Metadata 5 0 R /OpenAction << /S /JavaScript /JS (app.alert\\(1\\)) >> \
             /AA << /WC << /S /JavaScript /JS (app.alert\\(2\\)) >> >> \
             /Names << /JavaScript << /Names [(a) << /S /JavaScript /JS (app.alert\\(3\\)) >>] >> \
             /EmbeddedFiles << /Names [(secret.txt) 6 0 R] >> /Dests << /Names [(here) [3 0 R /Fit]] >> >> \
             /AcroForm << /Fields [12 0 R] >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R \
             /AA << /O << /S /JavaScript /JS (app.alert\\(4\\)) >> >> /Annots [8 0 R 9 0 R 10 0 R 11 0 R 12 0 R 13 0 R] >>",
            "<< /Length 0 >>\nstream\n\nendstream",
            "<< /Type /Metadata /Subtype /XML /Length 13 >>\nstream\n<x>Author</x>\nendstream",
            "<< /Type /Filespec /F (secret.txt) /EF << /F 7 0 R >> >>",
            "<< /Type /EmbeddedFile /Length 6 >>\nstream\nhidden\nendstream",
            "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (A note) /Popup 13 0 R >>",
            "<< /Type /Annot /Subtype /FileAttachment /Rect [40 10 60 30] /FS 6 0 R >>",
            "<< /Type /Annot /Subtype /Link /Rect [70 10 90 30] /A << /S /JavaScript /JS (app.alert\\(5\\)) >> >>",
            "<< /Type /Annot /Subtype /Link /Rect [100 10 120 30] /A << /S /URI /URI (https://example.org) >> >>",
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Jo) /Rect [10 100 150 120] \
             /AA << /K << /S /JavaScript /JS (app.alert\\(6\\)) >> >> >>",
            "<< /Type /Annot /Subtype /Popup /Rect [10 40 80 80] /Parent 8 0 R >>",
            "<< /Title (Busy) /Author (Jo Bloggs) >>",
        ],
        "/Info 14 0 R",
    )
}

fn open(bytes: Vec<u8>) -> Document {
    Document::from_bytes(bytes, None, Limits::default()).unwrap()
}

/// Whether `needle` is in `bytes`, as it is or as a hex string (copied strings are written
/// as hex).
fn has(bytes: &[u8], needle: &str) -> bool {
    let plain = needle.trim_start_matches('(').trim_end_matches(')');
    let hex: String = plain.bytes().map(|b| format!("{b:02X}")).collect();
    [needle.to_owned(), hex].iter().any(|n| {
        bytes
            .windows(n.len())
            .any(|w| w.eq_ignore_ascii_case(n.as_bytes()))
    })
}

#[test]
fn hidden_information_is_removed() {
    let doc = open(busy());
    // The note and the attached file.
    assert_eq!(doc.comments().len(), 2);
    let all = Cleanup {
        metadata: true,
        scripts: true,
        attachments: true,
        comments: true,
    };
    let out = doc.cleaned(all).unwrap();
    for gone in [
        "JavaScript",
        "app.alert",
        "Jo Bloggs",
        "<x>Author",
        "hidden",
        "secret.txt",
        "A note",
        "/Popup",
    ] {
        assert!(!has(&out, gone), "{gone} is still there");
    }
    // The destination name, the web link and the field (with its value) stay.
    for kept in ["(here)", "https://example.org", "(Jo)"] {
        assert!(has(&out, kept), "{kept} went");
    }
    let out = open(out);
    assert!(out.comments().is_empty());
    let info = out.info().unwrap();
    assert_eq!(info.fields.len(), 1);
    assert_eq!(info.links.len(), 1);
    assert!(!info.properties.iter().any(|(k, _)| *k == "Author"));
}

#[test]
fn only_what_is_chosen_goes() {
    let doc = open(busy());
    let out = doc
        .cleaned(Cleanup {
            attachments: true,
            ..Cleanup::default()
        })
        .unwrap();
    assert!(!has(&out, "secret.txt") && !has(&out, "hidden"));
    assert!(has(&out, "Jo Bloggs") && has(&out, "app.alert(1)") && has(&out, "A note"));
    assert_eq!(open(out).comments().len(), 1);
}
