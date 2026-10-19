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

#[test]
fn tags_are_added_to_an_untagged_file() {
    let content = "BT /F1 24 Tf 20 160 Td (Report) Tj ET\n\
                   q 10 0 0 10 20 120 cm BI /W 1 /H 1 /CS /G /BPC 8 ID x EI Q\n\
                   BT /F1 11 Tf 20 100 Td (The first paragraph.) Tj ET\n\
                   BT /F1 11 Tf 20 80 Td (The second one.) Tj ET";
    let bytes = build(
        &[
            "<< /Type /Catalog /Pages 2 0 R /Lang (en-GB) >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> >> >>",
            &stream("", content),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        ],
        "",
    );
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    assert!(rules(&doc).contains(&AccessRule::NotTagged));
    let before = doc.page_text(0).unwrap();
    let tagged = doc.reopen(doc.tagged().unwrap()).unwrap();
    assert!(
        !rules(&tagged).contains(&AccessRule::NotTagged),
        "{:?}",
        rules(&tagged)
    );
    assert!(!rules(&tagged).contains(&AccessRule::TabOrder));
    let after = tagged.page_text(0).unwrap();
    assert_eq!(before.slice(0..before.len()), after.slice(0..after.len()));
    assert_eq!(tagged.tagged(), Err(panna_core::SaveError::AlreadyTagged));
    // The heading and paragraphs are marked in the page's content.
    let text = String::from_utf8_lossy(tagged.bytes()).into_owned();
    assert!(text.contains("/StructTreeRoot"));
    let roles: Vec<&str> = ["/S /H1", "/S /P"]
        .into_iter()
        .filter(|r| text.contains(r))
        .collect();
    assert_eq!(roles, ["/S /H1", "/S /P"]);
}
