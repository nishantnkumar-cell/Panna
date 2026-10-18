#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, Limits, PageSource};

/// Three pages saying "Page 1" to "Page 3", in a two-level page tree whose top node holds the
/// page size and fonts the pages inherit, with a bookmark to the second page.
fn three_pages() -> Vec<u8> {
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 12 0 R /PageLabels << /Nums [0 << /S /r >>] >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R 10 0 R] /Count 3 /MediaBox [0 0 300 400] \
         /Resources << /Font << /F1 11 0 R >> >> >>"
            .into(),
        "<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 6 0 R] /Count 2 /Rotate 90 >>".into(),
    ];
    for i in 1..=3 {
        let content = format!("BT /F1 24 Tf 40 300 Td (Page {i}) Tj ET");
        let parent = if i < 3 { 3 } else { 2 };
        let page = format!("<< /Type /Page /Parent {parent} 0 R /Contents {{}} 0 R >>");
        let stream = format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len());
        objects.push(page);
        objects.push(stream);
    }
    // Objects 4..9: page, content, page, content, page, content. Fix up numbers.
    objects[3] = objects[3].replace("{}", "5");
    objects[5] = objects[5].replace("{}", "7");
    objects[7] = objects[7].replace("{}", "9");
    // Page 3 is object 8; object 10 is a one-page node holding it.
    objects[7] = objects[7].replace("/Parent 2 0 R", "/Parent 10 0 R");
    objects.push("<< /Type /Pages /Parent 2 0 R /Kids [8 0 R] /Count 1 >>".into());
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    objects.push("<< /Type /Outlines /First 13 0 R /Last 13 0 R /Count 1 >>".into());
    objects.push("<< /Title (Second) /Parent 12 0 R /Dest [6 0 R /Fit] >>".into());
    objects.push("<< /Title (Three pages) /Author (Jo) >>".into());

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
            "trailer\n<< /Size {} /Root 1 0 R /Info 14 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn open(bytes: Vec<u8>, password: Option<&str>) -> Document {
    Document::from_bytes(bytes, password, Limits::default()).unwrap()
}

fn text(doc: &Document, page: usize) -> String {
    let t = doc.page_text(page).unwrap();
    t.slice(0..t.len())
}

fn page(index: usize, turns: u8) -> PageSource {
    PageSource::Page { doc: 0, index, turns }
}

#[test]
fn the_source_reads_as_built() {
    let doc = open(three_pages(), None);
    assert_eq!(doc.page_count(), 3);
    assert!(text(&doc, 2).contains("Page 3"));
    let info = doc.info().unwrap();
    assert_eq!(info.outline[0].page, Some(1));
    // The first two pages inherit a quarter turn.
    assert_eq!((info.pages[0].width, info.pages[0].height), (400.0, 300.0));
    assert_eq!((info.pages[2].width, info.pages[2].height), (300.0, 400.0));
}

#[test]
fn pages_are_reordered_turned_dropped_and_added() {
    let doc = open(three_pages(), None);
    let plan = [
        page(2, 0),
        page(0, 1),
        PageSource::Blank {
            width: 200.0,
            height: 100.0,
        },
    ];
    let bytes = doc.rearranged(&plan, &[]).unwrap();
    // The dropped page's content is gone, not just unlisted.
    assert!(!bytes.windows(8).any(|w| w == b"(Page 2)"));
    let out = open(bytes, None);
    assert_eq!(out.page_count(), 3);
    assert!(text(&out, 0).contains("Page 3"));
    assert!(text(&out, 1).contains("Page 1"));
    assert!(text(&out, 2).is_empty());
    let info = out.info().unwrap();
    // Inherited size and fonts came along; the quarter turn it had plus one more is a half.
    assert_eq!((info.pages[0].width, info.pages[0].height), (300.0, 400.0));
    assert_eq!((info.pages[1].width, info.pages[1].height), (300.0, 400.0));
    assert_eq!((info.pages[2].width, info.pages[2].height), (200.0, 100.0));
    assert_eq!(info.title.as_deref(), Some("Three pages"));
    // The bookmark's page is gone, so it leads nowhere rather than somewhere wrong.
    assert_eq!(info.outline.len(), 1);
    assert_eq!(info.outline[0].page, None);
}

#[test]
fn bookmarks_follow_their_page() {
    let doc = open(three_pages(), None);
    let bytes = doc
        .rearranged(&[page(1, 0), page(0, 0), page(2, 0)], &[])
        .unwrap();
    let out = open(bytes, None);
    assert_eq!(out.info().unwrap().outline[0].page, Some(0));
    assert!(text(&out, 1).contains("Page 1"));
}

#[test]
fn files_are_combined() {
    let doc = open(three_pages(), None);
    let other = open(
        include_bytes!("fixtures/encrypted-aes128.pdf").to_vec(),
        Some("secret"),
    );
    let plan = [
        page(0, 0),
        PageSource::Page {
            doc: 1,
            index: 0,
            turns: 0,
        },
    ];
    let out = open(doc.rearranged(&plan, &[&other]).unwrap(), None);
    assert_eq!(out.page_count(), 2);
    assert!(text(&out, 1).contains("Hello secret world"));
}

#[test]
fn password_protection_is_kept() {
    for (name, bytes) in [
        ("RC4", &include_bytes!("fixtures/encrypted-rc4.pdf")[..]),
        ("AES-128", include_bytes!("fixtures/encrypted-aes128.pdf")),
        ("AES-256", include_bytes!("fixtures/encrypted-aes256.pdf")),
    ] {
        let doc = open(bytes.to_vec(), Some("secret"));
        let saved = doc.rearranged(&[page(0, 0), page(0, 2)], &[]).unwrap();
        assert!(
            Document::from_bytes(saved.clone(), None, Limits::default()).is_err(),
            "{name} opens without its password"
        );
        let out = open(saved, Some("secret"));
        assert_eq!(out.page_count(), 2, "{name}");
        assert!(text(&out, 1).contains("Hello secret world"), "{name}");
        let fields = out.info().unwrap().fields;
        assert_eq!(fields.len(), 1, "{name}");
        assert_eq!(fields[0].name, "name", "{name}");
    }
}

#[test]
fn the_worker_changes_pages_with_undo_and_saves() {
    use panna_core::{Annotation, Changes, Color, Kind, PageEdit, RenderWorker, TextBox};
    use std::sync::mpsc;
    use std::time::Duration;

    let dir = std::env::temp_dir().join(format!("panna-pages-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("three.pdf");
    std::fs::write(&path, three_pages()).unwrap();
    let other = dir.join("other.pdf");
    std::fs::write(&other, three_pages()).unwrap();
    let wait = Duration::from_secs(20);

    let (tx, rx) = mpsc::channel();
    let worker = RenderWorker::open(path.clone(), None, Limits::default(), move |r| {
        let _ = tx.send(r.is_ok());
    })
    .unwrap();
    assert!(rx.recv_timeout(wait).unwrap());
    let pages = |edit: PageEdit| {
        let (tx, rx) = mpsc::channel();
        worker.edit_pages(edit, move |r| {
            let _ = tx.send(r.map(|i| (i.pages.len(), i.comments.len())));
        });
        rx.recv_timeout(wait).unwrap()
    };

    // An unsaved comment on page 3 goes along when page 2 is removed.
    let (tx, rx) = mpsc::channel();
    let rect = TextBox {
        x0: 10.0,
        y0: 10.0,
        x1: 50.0,
        y1: 50.0,
    };
    worker.set_changes(
        Changes {
            add: vec![Annotation {
                page: 2,
                kind: Kind::Rect(rect),
                color: Color([1.0, 0.0, 0.0]),
                opacity: 1.0,
                width: 2.0,
                contents: String::new(),
                author: String::new(),
            }],
            ..Changes::default()
        },
        move |r| {
            let _ = tx.send(r);
        },
    );
    assert_eq!(rx.recv_timeout(wait).unwrap(), Ok(()));
    // The copy to print has the unsaved comment.
    let (tx, rx) = mpsc::channel();
    worker.print_copy(move |r| {
        let _ = tx.send(r);
    });
    let printed = open(rx.recv_timeout(wait).unwrap().unwrap(), None);
    assert_eq!(printed.info().unwrap().comments.len(), 1);
    assert_eq!(pages(PageEdit::Plan(vec![page(0, 0), page(2, 0)])), Ok((2, 1)));
    assert_eq!(
        pages(PageEdit::InsertFile {
            at: 1,
            path: other.clone()
        }),
        Ok((5, 1))
    );
    assert_eq!(pages(PageEdit::Undo), Ok((2, 1)));
    assert_eq!(
        pages(PageEdit::Undo),
        Ok((3, 1)),
        "back to three pages, comment kept"
    );
    assert_eq!(pages(PageEdit::Plan(vec![page(2, 1)])), Ok((1, 1)));

    let (tx, rx) = mpsc::channel();
    let (one, two) = (dir.join("one.pdf"), dir.join("two.pdf"));
    worker.extract(
        vec![
            (vec![page(0, 0)], one.clone()),
            (vec![page(0, 0), page(0, 0)], two.clone()),
        ],
        move |r| {
            let _ = tx.send(r);
        },
    );
    assert_eq!(rx.recv_timeout(wait).unwrap(), Ok(()));
    assert_eq!(open(std::fs::read(&one).unwrap(), None).page_count(), 1);
    assert_eq!(open(std::fs::read(&two).unwrap(), None).page_count(), 2);

    let (tx, rx) = mpsc::channel();
    worker.save(path.clone(), move |r| {
        let _ = tx.send(r.map(|c| c.len()));
    });
    assert_eq!(rx.recv_timeout(wait).unwrap(), Ok(1));
    let saved = open(std::fs::read(&path).unwrap(), None);
    assert_eq!(saved.page_count(), 1);
    assert!(text(&saved, 0).contains("Page 3"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn print_copies_have_no_password_and_respect_permissions() {
    for bytes in [
        &include_bytes!("fixtures/encrypted-rc4.pdf")[..],
        include_bytes!("fixtures/encrypted-aes256.pdf"),
    ] {
        let copy = open(bytes.to_vec(), Some("secret")).print_copy().unwrap();
        let out = open(copy, None);
        assert!(text(&out, 0).contains("Hello secret world"));
    }
    let plain = open(three_pages(), None).print_copy().unwrap();
    assert_eq!(open(plain, None).page_count(), 3);

    let restricted = include_bytes!("fixtures/encrypted-noprint.pdf").to_vec();
    assert_eq!(
        open(restricted.clone(), Some("secret")).print_copy(),
        Err(panna_core::SaveError::PrintingNotAllowed)
    );
    // The owner password lifts the restriction.
    let copy = open(restricted, Some("owner")).print_copy().unwrap();
    assert!(text(&open(copy, None), 0).contains("Hello secret world"));
}

#[test]
fn pages_are_cropped_by_the_margins_as_shown() {
    let doc = open(three_pages(), None);
    // Top, right, bottom, left.
    let margins = [10.0, 20.0, 30.0, 40.0];
    let out = open(doc.cropped(&[0, 2], margins).unwrap(), None);
    let info = out.info().unwrap();
    // Page 1 is shown turned, 400 × 300; page 2 is left alone; page 3 is 300 × 400.
    assert_eq!((info.pages[0].width, info.pages[0].height), (340.0, 260.0));
    assert_eq!((info.pages[1].width, info.pages[1].height), (400.0, 300.0));
    assert_eq!((info.pages[2].width, info.pages[2].height), (240.0, 360.0));
    assert!(text(&out, 2).contains("Page 3"));
    // Cropping again trims what is left, and too much is refused.
    let again = open(out.cropped(&[2], margins).unwrap(), None);
    assert_eq!(again.info().unwrap().pages[2].width, 180.0);
    assert_eq!(
        doc.cropped(&[2], [0.0, 150.0, 0.0, 150.0]),
        Err(panna_core::SaveError::CropTooLarge)
    );
}

#[test]
fn headers_and_footers_sit_on_the_page_as_shown() {
    use panna_core::{HeaderFooter, TextBox};
    let doc = open(three_pages(), None);
    let mut marks = HeaderFooter {
        first: 7,
        ..HeaderFooter::default()
    };
    marks.text[0] = "Three pages".into();
    marks.text[4] = "Page {page} of {pages}".into();
    marks.text[5] = "X{page:4}".into();
    let out = open(doc.with_header_footer(&[0, 2], &marks).unwrap(), None);
    let band = |doc: &Document, page: usize, y0: f32, y1: f32| {
        let width = doc.info().unwrap().pages[page].width;
        doc.page_text(page).unwrap().text_in(&[TextBox {
            x0: 0.0,
            y0,
            x1: width,
            y1,
        }])
    };
    // Page 1 is shown turned, 400 × 300: its header and footer run along the long edges. Its
    // own text starts near the top too.
    assert!(band(&out, 0, 0.0, 60.0).ends_with("Three pages"));
    assert_eq!(band(&out, 0, 240.0, 300.0), "Page 7 of 2 X0007");
    assert!(text(&out, 0).contains("Page 1"));
    // Page 2 is untouched; page 3 is the second one marked.
    assert!(!text(&out, 1).contains("Three pages"));
    assert_eq!(band(&out, 2, 340.0, 400.0), "Page 8 of 2 X0008");
    // The page's own content is still drawn, and the result still opens with its bookmarks.
    assert!(text(&out, 2).contains("Page 3"));
    assert_eq!(out.page_count(), 3);
}

#[test]
fn headers_and_footers_keep_password_protection() {
    let doc = open(
        include_bytes!("fixtures/encrypted-aes256.pdf").to_vec(),
        Some("secret"),
    );
    let mut marks = panna_core::HeaderFooter::default();
    marks.text[1] = "Confidential".into();
    let saved = doc.with_header_footer(&[0], &marks).unwrap();
    assert!(Document::from_bytes(saved.clone(), None, Limits::default()).is_err());
    let out = open(saved, Some("secret"));
    let t = text(&out, 0);
    assert!(
        t.contains("Confidential") && t.contains("Hello secret world"),
        "{t}"
    );
}
