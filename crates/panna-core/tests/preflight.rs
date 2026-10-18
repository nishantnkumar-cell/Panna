#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, Limits, Rule};

fn build(objects: &[&str], trailer_extra: &str) -> Vec<u8> {
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
            "trailer\n<< /Size {} /Root 1 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn stream(dict: &str, data: &str) -> String {
    format!("<< {dict} /Length {} >>\nstream\n{data}\nendstream", data.len())
}

fn rules(doc: &Document) -> Vec<Rule> {
    let mut r: Vec<Rule> = doc.preflight().into_iter().map(|f| f.rule).collect();
    r.dedup();
    r
}

#[test]
fn an_ordinary_file_falls_short_in_the_usual_ways() {
    let bytes = build(
        &[
            "<< /Type /Catalog /Pages 2 0 R /Names << /JavaScript << /Names [] >> >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> >> /Annots [6 0 R] >>",
            &stream("", "BT /F1 12 Tf 10 10 Td (Hi) Tj ET"),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (Note) >>",
        ],
        "",
    );
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    assert_eq!(
        rules(&doc),
        [
            Rule::NoFileId,
            Rule::NoMetadata,
            Rule::NoOutputIntent,
            Rule::FontNotEmbedded,
            Rule::Scripts,
            Rule::NoAppearance,
            Rule::NotPrinted,
        ]
    );
    let font = doc
        .preflight()
        .into_iter()
        .find(|f| f.rule == Rule::FontNotEmbedded)
        .unwrap();
    assert_eq!((font.detail.as_str(), font.page), ("Helvetica", Some(0)));
}

#[test]
fn a_file_meeting_the_rules_passes() {
    let xmp = "<x:xmpmeta xmlns:x='adobe:ns:meta/'><rdf:RDF><rdf:Description \
               xmlns:pdfaid='http://www.aiim.org/pdfa/ns/id/' pdfaid:part='2' \
               pdfaid:conformance='B'/></rdf:RDF></x:xmpmeta>";
    let bytes = build(
        &[
            "<< /Type /Catalog /Pages 2 0 R /Metadata 5 0 R /OutputIntents [<< /Type /OutputIntent \
             /S /GTS_PDFA1 /OutputConditionIdentifier (sRGB) /DestOutputProfile 6 0 R >>] >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Annots [7 0 R] >>",
            &stream("", "0 0 1 rg 10 10 50 50 re f"),
            &stream("/Type /Metadata /Subtype /XML", xmp),
            &stream("/N 3", "profile"),
            "<< /Type /Annot /Subtype /Square /Rect [10 10 30 30] /F 4 /AP << /N 8 0 R >> >>",
            &stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 20 20]",
                "0 0 20 20 re S",
            ),
        ],
        "/ID [<0123> <0123>]",
    );
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    assert_eq!(doc.preflight(), []);
}

#[test]
fn password_protection_is_reported() {
    let bytes = include_bytes!("fixtures/encrypted-aes256.pdf");
    let doc = Document::from_bytes(bytes.to_vec(), Some("secret"), Limits::default()).unwrap();
    assert!(rules(&doc).contains(&Rule::Encrypted));
}
