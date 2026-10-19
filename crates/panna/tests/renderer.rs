//! The page renderer starts from the built app, locks itself down and still draws its test
//! page. On Windows it must be doing so inside an AppContainer.

use std::io::Read;

/// The tag of the renderer's "ready" message (`panna_core::isolate`).
const READY: u8 = 6;

#[test]
fn renderer_locks_down_and_draws() {
    let program = std::path::Path::new(env!("CARGO_BIN_EXE_panna"));
    let mut child = panna_sandbox::spawn(program, panna_core::isolate::ARG).expect("renderer starts");
    if cfg!(windows) {
        assert!(
            child.app_container,
            "the renderer should start in an AppContainer"
        );
    }
    let mut head = [0u8; 9];
    child
        .stdout
        .read_exact(&mut head)
        .expect("the renderer says it's ready rather than stopping");
    assert_eq!(head[0], READY);
}

/// Two pages saying "Page 1" and "Page 2".
fn two_pages() -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 /MediaBox [0 0 200 100] \
         /Resources << /Font << /F1 7 0 R >> >> >>"
            .to_owned(),
    ];
    for i in 1..=2 {
        let content = format!("BT /F1 24 Tf 20 40 Td (Page {i}) Tj ET");
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /Contents {} 0 R >>",
            2 + 2 * i
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned());
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
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

/// Opening, reading text, changing pages and saving all work when the document is only
/// ever parsed in the locked-down process.
#[test]
fn documents_are_read_and_changed_in_the_locked_down_process() {
    use panna_core::{Export, ExportFormat, Limits, PageEdit, PageSource, RenderWorker};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    panna_core::isolate::use_renderer(env!("CARGO_BIN_EXE_panna").into());
    let dir = std::env::temp_dir().join(format!("panna-remote-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("two.pdf");
    std::fs::write(&path, two_pages()).unwrap();
    let wait = Duration::from_secs(60);

    let (tx, rx) = channel();
    let worker = RenderWorker::open(path.clone(), None, Limits::default(), move |r| {
        let _ = tx.send(r.map(|info| info.pages.len()));
    })
    .unwrap();
    assert_eq!(rx.recv_timeout(wait).unwrap(), Ok(2));

    let (tx, rx) = channel();
    worker.extract_text(1, move |r| {
        let _ = tx.send(r.map(|t| t.slice(0..t.len())));
    });
    assert!(rx.recv_timeout(wait).unwrap().unwrap().contains("Page 2"));

    let (tx, rx) = channel();
    worker.render(0, 1.0, 0, move |r| {
        let _ = tx.send(r.map(|p| (p.width, p.height)));
    });
    assert_eq!(rx.recv_timeout(wait).unwrap(), Ok((200, 100)));

    // Only the second page is kept, then the change is undone and redone.
    let page = |index| PageSource::Page {
        doc: 0,
        index,
        turns: 0,
    };
    let pages = |edit: PageEdit| {
        let (tx, rx) = channel();
        worker.edit_pages(edit, move |r| {
            let _ = tx.send(r.map(|info| info.pages.len()));
        });
        rx.recv_timeout(wait).unwrap()
    };
    assert_eq!(pages(PageEdit::Plan(vec![page(1)])), Ok(1));
    assert_eq!(pages(PageEdit::Undo), Ok(2));
    assert_eq!(pages(PageEdit::Plan(vec![page(1)])), Ok(1));

    let saved = dir.join("saved.pdf");
    let (tx, rx) = channel();
    worker.save(saved.clone(), move |r| {
        let _ = tx.send(r.is_ok());
    });
    assert!(rx.recv_timeout(wait).unwrap());
    let doc = panna_core::Document::open(&saved, None, Limits::default()).unwrap();
    assert_eq!(doc.page_count(), 1);

    let text = dir.join("saved.txt");
    let (tx, rx) = channel();
    worker.export(
        Export {
            format: ExportFormat::Text,
            dpi: 0.0,
            pages: vec![(0, text.clone())],
        },
        move |r| {
            let _ = tx.send(r.is_ok());
        },
    );
    assert!(rx.recv_timeout(wait).unwrap());
    assert!(std::fs::read_to_string(&text).unwrap().contains("Page 2"));
    let _ = std::fs::remove_dir_all(&dir);
}
