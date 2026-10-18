#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, Limits, PictureChange, StampImage, TextBox};

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

/// A red picture at 300..350 by 20..70 and a green one at 20..60 by 120..160, drawn by two
/// pages, the second turned a quarter.
fn sample() -> Document {
    let resources = "/Resources << /XObject << /Red 5 0 R /Green 6 0 R >> >>";
    let image = |hex: &str| {
        stream(
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB \
             /BitsPerComponent 8 /Filter /ASCIIHexDecode",
            hex,
        )
    };
    let bytes = build(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        &format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Contents 7 0 R {resources} >>"),
        &format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Rotate 90 /Contents 7 0 R \
             {resources} >>"
        ),
        &image("FF0000>"),
        &image("00FF00>"),
        &stream(
            "",
            "q 50 0 0 50 300 20 cm /Red Do Q q 2 0 0 2 10 60 cm q 20 0 0 20 0 30 cm /Green Do Q Q",
        ),
    ]);
    Document::from_bytes(bytes, None, Limits::default()).unwrap()
}

fn b(x0: f32, y0: f32, x1: f32, y1: f32) -> TextBox {
    TextBox { x0, y0, x1, y1 }
}

fn close(a: TextBox, b: TextBox) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)]
        .iter()
        .all(|(x, y)| (x - y).abs() < 0.05)
}

fn assert_boxes(found: &[TextBox], expected: &[TextBox]) {
    assert_eq!(found.len(), expected.len(), "{found:?}");
    for (f, e) in found.iter().zip(expected) {
        assert!(close(*f, *e), "{f:?} {e:?}");
    }
}

/// The colour drawn at (x, y) as shown.
fn color(doc: &Document, page: usize, x: u32, y: u32) -> [u8; 3] {
    let cache = doc.new_cache();
    let page = doc.render_page(&cache, page, 1.0).unwrap();
    let i = ((y * page.width + x) * 4) as usize;
    [page.rgba[i], page.rgba[i + 1], page.rgba[i + 2]]
}

const RED: [u8; 3] = [255, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

#[test]
fn pictures_are_found_as_shown() {
    let doc = sample();
    assert_boxes(
        &doc.pictures(0),
        &[b(300.0, 130.0, 350.0, 180.0), b(10.0, 40.0, 50.0, 80.0)],
    );
    // Turned a quarter clockwise, the page is 200 wide and 400 tall.
    assert_boxes(
        &doc.pictures(1),
        &[b(20.0, 300.0, 70.0, 350.0), b(120.0, 10.0, 160.0, 50.0)],
    );
    assert!(doc.pictures(2).is_empty());
}

#[test]
fn pictures_move_and_resize_where_asked() {
    let doc = sample();
    for (page, to) in [(0, b(100.0, 20.0, 200.0, 60.0)), (1, b(40.0, 100.0, 90.0, 220.0))] {
        let before = doc.pictures(page);
        let new = doc
            .reopen(
                doc.with_pictures_changed(page, &[(0, PictureChange::Move(to))])
                    .unwrap(),
            )
            .unwrap();
        assert_boxes(&new.pictures(page), &[to, before[1]]);
        let mid = |b: TextBox| (((b.x0 + b.x1) / 2.0) as u32, ((b.y0 + b.y1) / 2.0) as u32);
        let (x, y) = mid(to);
        assert_eq!(color(&new, page, x, y), RED);
        let (x, y) = mid(before[0]);
        assert_eq!(color(&new, page, x, y), WHITE);
        // The other page is as it was.
        let other = 1 - page;
        assert_boxes(&new.pictures(other), &doc.pictures(other));
    }
}

#[test]
fn pictures_are_taken_out() {
    let doc = sample();
    let new = doc
        .reopen(
            doc.with_pictures_changed(0, &[(0, PictureChange::Delete)])
                .unwrap(),
        )
        .unwrap();
    assert_boxes(&new.pictures(0), &[b(10.0, 40.0, 50.0, 80.0)]);
    assert_eq!(color(&new, 0, 325, 155), WHITE);
    assert_eq!(new.pictures(1).len(), 2);
}

#[test]
fn replaced_pictures_keep_their_shape_in_the_old_place() {
    let doc = sample();
    // Twice as wide as tall, in a square: it fills the width and sits in the middle.
    let blue = StampImage::from_rgba(2, 1, &[0, 0, 255, 255, 0, 0, 255, 255]).unwrap();
    for page in [0, 1] {
        let old = doc.pictures(page)[0];
        let new = doc
            .reopen(
                doc.with_pictures_changed(page, &[(0, PictureChange::Replace(blue.clone()))])
                    .unwrap(),
            )
            .unwrap();
        let found = new.pictures(page);
        let mid = (old.y0 + old.y1) / 2.0;
        assert_boxes(&found[..1], &[b(old.x0, mid - 12.5, old.x1, mid + 12.5)]);
        let x = ((old.x0 + old.x1) / 2.0) as u32;
        assert_eq!(color(&new, page, x, mid as u32), [0, 0, 255]);
        assert_eq!(color(&new, page, x, old.y0 as u32 + 3), WHITE);
    }
}

#[test]
fn several_changes_at_once() {
    let doc = sample();
    let to = b(100.0, 100.0, 140.0, 140.0);
    let new = doc
        .reopen(
            doc.with_pictures_changed(0, &[(0, PictureChange::Delete), (1, PictureChange::Move(to))])
                .unwrap(),
        )
        .unwrap();
    assert_boxes(&new.pictures(0), &[to]);
}
