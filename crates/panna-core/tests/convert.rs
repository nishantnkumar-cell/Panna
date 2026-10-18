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
