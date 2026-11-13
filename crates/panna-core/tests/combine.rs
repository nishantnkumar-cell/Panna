#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use image::ImageEncoder;
use panna_core::combine::combine;
use panna_core::{Document, Limits, PageEdit, blank_pdf};

fn png(width: u32, height: u32) -> Vec<u8> {
    let pixels: Vec<u8> = [255, 0, 0, 255].repeat((width * height) as usize);
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}

#[test]
fn files_and_pictures_combine_in_order_with_a_bookmark_each() {
    let dir = std::env::temp_dir().join(format!("panna-combine-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // The first file has a bookmark of its own, on its second page.
    let (mut first, _) =
        panna_core::Isolated::open(blank_pdf(300.0, 400.0, 2).unwrap(), None, Limits::default()).unwrap();
    first
        .edit(PageEdit::Bookmarks(vec![panna_core::OutlineItem {
            title: "Inside".into(),
            page: Some(1),
            ..Default::default()
        }]))
        .unwrap();
    let a = dir.join("Report.pdf");
    std::fs::write(&a, first.bytes()).unwrap();
    let b = dir.join("photo.PNG");
    std::fs::write(&b, png(40, 20)).unwrap();
    let c = dir.join("Appendix.pdf");
    std::fs::write(&c, blank_pdf(500.0, 500.0, 3).unwrap()).unwrap();

    let mut seen = Vec::new();
    let bytes = combine(&[a.clone(), b, c], Limits::default(), |i| seen.push(i)).unwrap();
    assert_eq!(seen, [0, 1, 2]);
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    let info = doc.info().unwrap();
    assert_eq!(info.pages.len(), 6);
    assert_eq!(info.pages[0].width.round(), 300.0);
    assert_eq!(info.pages[3].width.round(), 500.0);
    let marks: Vec<(String, u16, Option<usize>)> = info
        .outline
        .iter()
        .map(|o| (o.title.clone(), o.depth, o.page))
        .collect();
    assert_eq!(
        marks,
        [
            ("Report".to_owned(), 0, Some(0)),
            ("Inside".to_owned(), 1, Some(1)),
            ("photo".to_owned(), 0, Some(2)),
            ("Appendix".to_owned(), 0, Some(3)),
        ]
    );
    // One file alone isn't combining.
    assert!(combine(&[a], Limits::default(), |_| ()).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
