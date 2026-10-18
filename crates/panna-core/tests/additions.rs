#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Addition, Color, Document, Limits, LinkTarget, StampImage, TextBox};

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

/// Two pages, the second turned a quarter and with its media box away from the origin.
fn blank() -> Document {
    let bytes = build(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [50 50 350 250] /Rotate 90 >>",
    ]);
    Document::from_bytes(bytes, None, Limits::default()).unwrap()
}

fn b(x0: f32, y0: f32, x1: f32, y1: f32) -> TextBox {
    TextBox { x0, y0, x1, y1 }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

#[test]
fn text_pictures_and_links_become_part_of_the_page() {
    let doc = blank();
    let red = [255, 0, 0, 255].repeat(4);
    let picture = StampImage::from_rgba(2, 2, &red).unwrap();
    for page in [0, 1] {
        let additions = vec![
            Addition::Text {
                at: (20.0, 30.0),
                text: "Hello there\nSecond line".into(),
                size: 12.0,
                color: Color::BLACK,
            },
            Addition::Picture {
                rect: b(100.0, 100.0, 140.0, 140.0),
                image: picture.clone(),
            },
            Addition::Link {
                rect: b(20.0, 150.0, 120.0, 170.0),
                target: LinkTarget::Uri("https://example.com/a(b)".into()),
            },
            Addition::Link {
                rect: b(150.0, 10.0, 180.0, 40.0),
                target: LinkTarget::Page(1 - page),
            },
        ];
        let bytes = doc.with_additions(page, &additions).unwrap();
        let new = doc.reopen(bytes).unwrap();
        let text = new.page_text(page).unwrap();
        let all = text.slice(0..text.len());
        assert!(
            all.contains("Hello there") && all.contains("Second line"),
            "{all:?}"
        );
        // The first line's top is where it was put, the second a line below.
        let first = text.rects(0..1)[0];
        assert!(
            (first.x0 - 20.0).abs() < 1.0 && (first.y0 - 30.0).abs() < 2.0,
            "{first:?}"
        );
        let lower = text.find("Second", Default::default())[0].clone();
        assert!((text.rects(lower)[0].y0 - 44.4).abs() < 2.0);

        let links = new.info().unwrap().links;
        let on_page: Vec<_> = links.iter().filter(|l| l.page == page).collect();
        assert_eq!(on_page.len(), 2, "{links:?}");
        let web = on_page
            .iter()
            .find(|l| matches!(l.target, LinkTarget::Uri(_)))
            .unwrap();
        assert_eq!(web.target, LinkTarget::Uri("https://example.com/a(b)".into()));
        let r = web.at;
        assert!(
            close(r.x0, 20.0) && close(r.y0, 150.0) && close(r.x1, 120.0) && close(r.y1, 170.0),
            "{r:?}"
        );
        assert!(on_page.iter().any(|l| l.target == LinkTarget::Page(1 - page)));

        // The picture is drawn where it was put.
        let rendered = new.render_page(&new.new_cache(), page, 1.0).unwrap();
        let px = |x: u32, y: u32| {
            let i = ((y * rendered.width + x) * 4) as usize;
            [rendered.rgba[i], rendered.rgba[i + 1], rendered.rgba[i + 2]]
        };
        assert_eq!(px(120, 120), [255, 0, 0]);
        assert_eq!(px(160, 120), [255, 255, 255]);
    }
}

#[test]
fn unsafe_addresses_and_missing_pages_are_left_out() {
    let doc = blank();
    let additions = vec![
        Addition::Link {
            rect: b(0.0, 0.0, 10.0, 10.0),
            target: LinkTarget::Uri("javascript:alert(1)".into()),
        },
        Addition::Link {
            rect: b(0.0, 0.0, 10.0, 10.0),
            target: LinkTarget::Page(9),
        },
    ];
    let new = doc.reopen(doc.with_additions(0, &additions).unwrap()).unwrap();
    assert!(new.info().unwrap().links.is_empty());
    assert!(doc.with_additions(5, &additions).is_err());
}
