use panna_core::{Document, Limits, OpenError, RenderWorker};
use std::sync::mpsc;
use std::time::Duration;

/// Builds a small but well-formed PDF with `pages` US Letter pages, each with a filled square.
fn sample_pdf(pages: usize) -> Vec<u8> {
    let mut objects: Vec<String> = Vec::new();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i * 2)).collect();
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        pages
    ));
    for i in 0..pages {
        let content = format!("1 0 0 rg {} 100 200 200 re f", 100 + i * 10);
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R >>",
            4 + i * 2
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        ));
    }
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

#[test]
fn opens_and_reports_page_sizes() {
    let doc = Document::from_bytes(sample_pdf(3), None, Limits::default()).unwrap();
    let info = doc.info().unwrap();
    assert_eq!(info.pages.len(), 3);
    assert_eq!((info.pages[0].width, info.pages[0].height), (612.0, 792.0));
}

#[test]
fn renders_red_square_on_white() {
    let doc = Document::from_bytes(sample_pdf(1), None, Limits::default()).unwrap();
    let cache = doc.new_cache();
    let page = doc.render_page(&cache, 0, 1.0).unwrap();
    assert_eq!((page.width, page.height), (612, 792));
    assert_eq!(page.rgba.len(), 612 * 792 * 4);
    let px = |x: usize, y: usize| &page.rgba[(y * 612 + x) * 4..][..4];
    assert_eq!(px(5, 5), [255, 255, 255, 255], "background is white");
    // PDF y=100..300 from the bottom is pixel y=492..692 from the top.
    assert_eq!(px(200, 600), [255, 0, 0, 255], "square is red");
}

#[test]
fn missing_page_is_an_error() {
    let doc = Document::from_bytes(sample_pdf(1), None, Limits::default()).unwrap();
    let cache = doc.new_cache();
    assert!(doc.render_page(&cache, 5, 1.0).is_err());
}

#[test]
fn huge_zoom_is_capped() {
    let doc = Document::from_bytes(sample_pdf(1), None, Limits::default()).unwrap();
    let cache = doc.new_cache();
    let page = doc.render_page(&cache, 0, 1000.0).unwrap();
    assert!(page.width <= Limits::default().max_bitmap_side);
    assert!(u64::from(page.width) * u64::from(page.height) <= Limits::default().max_bitmap_pixels);
}

#[test]
fn rejects_garbage_without_panicking() {
    for input in [
        &b""[..],
        b"hello",
        b"%PDF-1.7\n",
        b"%PDF-1.7\n1 0 obj << /Type /Catalog >> endobj",
    ] {
        assert!(Document::from_bytes(input.to_vec(), None, Limits::default()).is_err());
    }
}

#[test]
fn survives_truncated_and_corrupted_files() {
    let good = sample_pdf(2);
    for cut in (0..good.len()).step_by(7) {
        if let Ok(doc) = Document::from_bytes(good[..cut].to_vec(), None, Limits::default()) {
            let cache = doc.new_cache();
            for i in 0..doc.page_count() {
                let _ = doc.render_page(&cache, i, 0.5);
            }
        }
    }
    for pos in (0..good.len()).step_by(3) {
        let mut bad = good.clone();
        bad[pos] ^= 0xA5;
        if let Ok(doc) = Document::from_bytes(bad, None, Limits::default()) {
            let cache = doc.new_cache();
            for i in 0..doc.page_count() {
                let _ = doc.render_page(&cache, i, 0.5);
            }
        }
    }
}

#[test]
fn enforces_size_and_page_limits() {
    let tiny = Limits {
        max_file_bytes: 10,
        ..Limits::default()
    };
    assert!(matches!(
        Document::from_bytes(sample_pdf(1), None, tiny),
        Err(OpenError::TooLarge { .. })
    ));
    let few = Limits {
        max_pages: 2,
        ..Limits::default()
    };
    assert!(matches!(
        Document::from_bytes(sample_pdf(3), None, few),
        Err(OpenError::TooManyPages { .. })
    ));
}

#[test]
fn worker_opens_and_renders_from_disk() {
    let dir = std::env::temp_dir().join(format!("panna-core-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sample.pdf");
    std::fs::write(&path, sample_pdf(2)).unwrap();

    let (open_tx, open_rx) = mpsc::channel();
    let worker = RenderWorker::open(path, None, Limits::default(), move |r| {
        let _ = open_tx.send(r);
    })
    .unwrap();
    let info = open_rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    assert_eq!(info.pages.len(), 2);

    let (tx, rx) = mpsc::channel();
    worker.render(1, 0.5, move |r| {
        let _ = tx.send(r);
    });
    let page = rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    assert_eq!((page.width, page.height), (306, 396));
    let _ = std::fs::remove_dir_all(dir);
}
