#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, Limits};

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

/// One file attached to the document, one to its second page, and one listed in both places.
fn attached() -> Vec<u8> {
    build(
        &[
            "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Kids [5 0 R] >> >> >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [10 0 R 11 0 R] >>",
            "<< /Limits [(a) (b)] /Names [(a) 6 0 R (b) 12 0 R] >>",
            "<< /Type /Filespec /F (old.txt) /UF (C:\\\\Users\\\\jo\\\\notes.txt) /Desc (Meeting notes) /EF << /F 7 0 R >> >>",
            "<< /Type /EmbeddedFile /Params << /Size 5 >> /Length 5 >>\nstream\nhello\nendstream",
            "<< /Type /Filespec /F (photo.jpg) /EF << /F 9 0 R >> >>",
            "<< /Type /EmbeddedFile /Length 3 >>\nstream\nabc\nendstream",
            "<< /Type /Annot /Subtype /FileAttachment /Rect [10 10 30 30] /FS 8 0 R >>",
            "<< /Type /Annot /Subtype /FileAttachment /Rect [40 10 60 30] /FS 12 0 R >>",
            "<< /Type /Filespec /F (both.txt) /EF << /F 13 0 R >> >>",
            "<< /Type /EmbeddedFile /Length 4 >>\nstream\nboth\nendstream",
        ],
        "",
    )
}

#[test]
fn attachments_are_listed_and_read() {
    let doc = Document::from_bytes(attached(), None, Limits::default()).unwrap();
    let files = doc.info().unwrap().attachments;
    let names: Vec<_> = files.iter().map(|f| (f.name.as_str(), f.page)).collect();
    assert_eq!(
        names,
        [("notes.txt", None), ("both.txt", None), ("photo.jpg", Some(1))]
    );
    assert_eq!(files[0].description, "Meeting notes");
    assert_eq!(files[0].size, Some(5));
    assert_eq!(files[2].size, None);
    assert_eq!(doc.attachment(0).unwrap(), b"hello");
    assert_eq!(doc.attachment(2).unwrap(), b"abc");
    assert_eq!(doc.attachment(1).unwrap(), b"both");
    assert!(doc.attachment(3).is_none());
}

/// A name can't hide what kind of file it is: direction overrides are dropped, and programs
/// are flagged however the name ends.
#[test]
fn attachment_names_cant_disguise_programs() {
    // "invoice", RIGHT-TO-LEFT OVERRIDE, "fdp.exe": shown by some apps as "invoiceexe.pdf".
    let utf16: String = "\u{feff}invoice\u{202e}fdp.exe"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .map(|b| format!("{b:02X}"))
        .collect();
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Names [(a) 4 0 R (b) 6 0 R] >> >> >>"
            .to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_owned(),
        format!("<< /Type /Filespec /UF <{utf16}> /EF << /F 5 0 R >> >>"),
        "<< /Type /EmbeddedFile /Length 2 >>\nstream\nMZ\nendstream".to_owned(),
        "<< /Type /Filespec /F (report.pdf) /EF << /F 5 0 R >> >>".to_owned(),
    ];
    let refs: Vec<&str> = objects.iter().map(String::as_str).collect();
    let doc = Document::from_bytes(build(&refs, ""), None, Limits::default()).unwrap();
    let files = doc.info().unwrap().attachments;
    assert_eq!(files[0].name, "invoicefdp.exe");
    assert!(files[0].is_program());
    assert!(!files[1].is_program());
    for (name, program) in [
        ("RUN.EXE. ", true),
        ("notes.txt", false),
        ("setup", false),
        ("a.lnk", true),
    ] {
        let file = panna_core::Attachment {
            name: name.into(),
            description: String::new(),
            size: None,
            page: None,
            media: None,
        };
        assert_eq!(file.is_program(), program, "{name}");
    }
}

/// Videos, sounds and 3D models on a page are listed with the attachments, to be saved.
#[test]
fn embedded_media_are_listed() {
    let doc = Document::from_bytes(
        build(
            &[
                "<< /Type /Catalog /Pages 2 0 R >>",
                "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R 7 0 R 8 0 R 11 0 R] >>",
                // A screen annotation playing a clip through a rendition selector.
                "<< /Type /Annot /Subtype /Screen /Rect [0 0 100 100] /A << /S /Rendition /R << /S /SR /R [<< /S /MR /C << /S /MCD /CT (audio/mpeg) /D 5 0 R >> >>] >> >> >>",
                "<< /Type /Filespec /UF (song.mp3) /EF << /F 6 0 R >> >>",
                "<< /Type /EmbeddedFile /Length 3 >>\nstream\nID3\nendstream",
                // A 3D model.
                "<< /Type /Annot /Subtype /3D /Rect [0 0 100 100] /3DD 9 0 R >>",
                // Rich media with a video asset.
                "<< /Type /Annot /Subtype /RichMedia /Rect [0 0 100 100] /RichMediaContent << /Assets << /Names [(clip.mp4) 10 0 R] >> >> >>",
                "<< /Type /3D /Subtype /U3D /Length 3 >>\nstream\nU3D\nendstream",
                "<< /Type /Filespec /UF (clip.mp4) /EF << /F 12 0 R >> >>",
                // A movie kept in a file outside the PDF: nothing to save, so not listed.
                "<< /Type /Annot /Subtype /Movie /Rect [0 0 100 100] /Movie << /F (outside.mov) >> >>",
                "<< /Type /EmbeddedFile /Length 4 >>\nstream\nmoov\nendstream",
            ],
            "",
        ),
        None,
        Limits::default(),
    )
    .unwrap();
    let files = doc.info().unwrap().attachments;
    let found: Vec<_> = files.iter().map(|f| (f.name.as_str(), f.page, f.media)).collect();
    use panna_core::Media;
    assert_eq!(
        found,
        [
            ("song.mp3", Some(0), Some(Media::Sound)),
            ("3D model, page 1.u3d", Some(0), Some(Media::Model3D)),
            ("clip.mp4", Some(0), Some(Media::Video)),
        ]
    );
    assert_eq!(doc.attachment(0).unwrap(), b"ID3");
    assert_eq!(doc.attachment(1).unwrap(), b"U3D");
    assert_eq!(doc.attachment(2).unwrap(), b"moov");
}

/// A portfolio made from files holds them in order, says it's a portfolio, and opens on its
/// attachments.
#[test]
fn a_portfolio_holds_its_files() {
    let files = vec![
        ("zebra notes.txt".to_owned(), b"stripes".to_vec()),
        ("Résumé.pdf".to_owned(), b"%PDF-1.7 not really".to_vec()),
        ("photo.jpg".to_owned(), vec![0xFF, 0xD8, 0xFF, 0x00]),
    ];
    let bytes = panna_core::make_portfolio(&files);
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    let info = doc.info().unwrap();
    assert!(info.portfolio);
    assert_eq!(info.pages.len(), 1);
    let names: Vec<_> = info
        .attachments
        .iter()
        .map(|a| (a.name.as_str(), a.size))
        .collect();
    assert_eq!(
        names,
        [
            ("zebra notes.txt", Some(7)),
            ("Résumé.pdf", Some(19)),
            ("photo.jpg", Some(4))
        ]
    );
    for (i, (_, data)) in files.iter().enumerate() {
        assert_eq!(&doc.attachment(i).unwrap(), data);
    }
    assert_eq!(info.initial_view.panel, panna_core::OpenPanel::Attachments);
    let text = doc.page_text(0).unwrap().text();
    assert!(
        text.contains("PDF Portfolio") && text.contains("zebra notes.txt"),
        "{text}"
    );
    // An ordinary document isn't one.
    assert!(
        !Document::from_bytes(attached(), None, Limits::default())
            .unwrap()
            .info()
            .unwrap()
            .portfolio
    );
}
