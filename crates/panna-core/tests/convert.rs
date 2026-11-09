#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use image::ImageEncoder;
use panna_core::{Document, Limits, pictures_to_pdf};

fn png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let pixels: Vec<u8> = rgba.repeat((width * height) as usize);
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}

fn jpeg(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let pixels: Vec<u8> = rgb.repeat((width * height) as usize);
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95)
        .write_image(&pixels, width, height, image::ExtendedColorType::Rgb8)
        .unwrap();
    out
}

#[test]
fn pictures_become_pages() {
    let jpg = jpeg(2480, 3508, [0, 0, 255]);
    let pdf = pictures_to_pdf(vec![png(200, 100, [255, 0, 0, 128]), jpg.clone()]).unwrap();
    // The JPEG goes in as it is.
    assert!(pdf.windows(jpg.len()).any(|w| w == jpg));
    let doc = Document::from_bytes(pdf, None, Limits::default()).unwrap();
    let info = doc.info().unwrap();
    assert_eq!(info.pages.len(), 2);
    // Small pictures keep 96 pixels an inch; a page-sized scan fills an A4 page.
    assert_eq!((info.pages[0].width, info.pages[0].height), (150.0, 75.0));
    assert!((info.pages[1].height - 842.0).abs() < 0.5 && (info.pages[1].width - 595.0).abs() < 1.0);
    let cache = doc.new_cache();
    let page = doc.render_page(&cache, 0, 1.0).unwrap();
    let mid = ((page.height / 2 * page.width + page.width / 2) * 4) as usize;
    // Half-transparent red over white.
    let [r, g, b] = [page.rgba[mid], page.rgba[mid + 1], page.rgba[mid + 2]];
    assert!(
        r > 240 && (110..150).contains(&g) && (110..150).contains(&b),
        "{r} {g} {b}"
    );
    let page = doc.render_page(&cache, 1, 0.1).unwrap();
    let mid = ((page.height / 2 * page.width + page.width / 2) * 4) as usize;
    assert!(page.rgba[mid] < 30 && page.rgba[mid + 2] > 220);
}

#[test]
fn unreadable_pictures_are_named() {
    let err = pictures_to_pdf(vec![png(1, 1, [0; 4]), b"not a picture".to_vec()]).unwrap_err();
    assert!(err.starts_with("Picture 2:"), "{err}");
    assert!(pictures_to_pdf(Vec::new()).is_err());
}

#[test]
fn tiff_pictures_become_pages_too() {
    let pixels: Vec<u8> = [0u8, 160, 0].repeat(40 * 20);
    let mut tiff = std::io::Cursor::new(Vec::new());
    image::codecs::tiff::TiffEncoder::new(&mut tiff)
        .write_image(&pixels, 40, 20, image::ExtendedColorType::Rgb8)
        .unwrap();
    let pdf = pictures_to_pdf(vec![tiff.into_inner()]).unwrap();
    let doc = Document::from_bytes(pdf, None, Limits::default()).unwrap();
    let info = doc.info().unwrap();
    assert_eq!((info.pages[0].width, info.pages[0].height), (30.0, 15.0));
    let page = doc.render_page(&doc.new_cache(), 0, 1.0).unwrap();
    let mid = ((page.height / 2 * page.width + page.width / 2) * 4) as usize;
    assert!(
        page.rgba[mid] < 30 && page.rgba[mid + 1] > 130,
        "{:?}",
        &page.rgba[mid..mid + 3]
    );
}

#[test]
fn blank_pdfs_have_the_pages_asked_for() {
    let doc = Document::from_bytes(
        panna_core::blank_pdf(842.0, 595.0, 3).unwrap(),
        None,
        Limits::default(),
    )
    .unwrap();
    let info = doc.info().unwrap();
    assert_eq!(info.pages.len(), 3);
    assert!(info.pages.iter().all(|p| (p.width, p.height) == (842.0, 595.0)));
    assert!(panna_core::blank_pdf(612.0, 792.0, 0).is_err());
    assert!(panna_core::blank_pdf(1.0, 792.0, 1).is_err());
}

#[test]
fn a_clipboard_picture_becomes_a_page() {
    let rgba = [10u8, 20, 30, 255].repeat(40 * 30);
    let pdf = panna_core::rgba_to_pdf(40, 30, &rgba).unwrap();
    let doc = Document::from_bytes(pdf, None, Limits::default()).unwrap();
    let info = doc.info().unwrap();
    assert_eq!(info.pages.len(), 1);
    // Wider than tall, at 96 pixels an inch.
    assert_eq!((info.pages[0].width, info.pages[0].height), (30.0, 22.5));
    assert!(panna_core::rgba_to_pdf(0, 0, &[]).is_err());
}
