#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{AccessRule, Document, Limits};

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

fn rules(doc: &Document) -> Vec<AccessRule> {
    let mut r: Vec<AccessRule> = doc.accessibility().into_iter().map(|f| f.rule).collect();
    r.dedup();
    r
}

#[test]
fn an_untagged_file_lacks_the_basics() {
    let bytes = build(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> >> /Annots [6 0 R] >>",
            &stream("", "BT /F1 12 Tf 10 10 Td <0001> Tj ET"),
            "<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+Noto /Encoding /Identity-H \
             /DescendantFonts [] >>",
            "<< /Type /Annot /Subtype /Link /Rect [10 10 30 30] >>",
        ],
        "",
    );
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    assert_eq!(
        rules(&doc),
        [
            AccessRule::NotTagged,
            AccessRule::NoLanguage,
            AccessRule::NoTitle,
            AccessRule::TitleNotShown,
            AccessRule::TabOrder,
            AccessRule::NoUnicode,
        ]
    );
    let font = doc
        .accessibility()
        .into_iter()
        .find(|f| f.rule == AccessRule::NoUnicode)
        .unwrap();
    assert_eq!((font.detail.as_str(), font.page), ("Noto", Some(0)));
}

#[test]
fn a_tagged_file_is_checked_inside() {
    let bytes = build(
        &[
            "<< /Type /Catalog /Pages 2 0 R /Lang (en-GB) /MarkInfo << /Marked true >> \
             /StructTreeRoot 7 0 R /ViewerPreferences << /DisplayDocTitle true >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Tabs /S \
             /Annots [5 0 R 6 0 R] >>",
            &stream("", "BT 10 10 Td ET"),
            "<< /Type /Annot /Subtype /Link /Rect [10 10 30 30] /StructParent 0 >>",
            "<< /Type /Annot /Subtype /Text /Rect [40 10 60 30] >>",
            "<< /Type /StructTreeRoot /K 8 0 R /RoleMap << /Picture /Figure >> >>",
            "<< /S /Document /K [9 0 R 10 0 R 11 0 R 12 0 R] >>",
            "<< /S /Picture /Pg 3 0 R /K 0 >>",
            "<< /S /Figure /Pg 3 0 R /Alt (A chart of sales) /K 1 >>",
            "<< /S /Table /Pg 3 0 R /K << /S /TR /K [<< /S /TD /K 2 >>] >> >>",
            "<< /S /Table /Pg 3 0 R /K << /S /TR /K [<< /S /TH /K 3 >>] >> >>",
        ],
        "/Info << /Title (Sales) >>",
    );
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    let found = doc.accessibility();
    assert_eq!(
        rules(&doc),
        [
            AccessRule::FigureNoAlt,
            AccessRule::TableNoHeaders,
            AccessRule::AnnotationNotTagged,
        ]
    );
    // The custom /Picture type counts as a figure; only the one without words is listed.
    assert_eq!(
        found.iter().filter(|f| f.rule == AccessRule::FigureNoAlt).count(),
        1
    );
    assert!(found.iter().all(|f| f.page == Some(0)));
}
