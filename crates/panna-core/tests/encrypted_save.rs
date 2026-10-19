#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Annotation, Changes, Color, Document, FieldValue, Kind, Limits, TextBox};

const FIXTURES: [(&str, &[u8]); 3] = [
    ("RC4", include_bytes!("fixtures/encrypted-rc4.pdf")),
    ("AES-128", include_bytes!("fixtures/encrypted-aes128.pdf")),
    ("AES-256", include_bytes!("fixtures/encrypted-aes256.pdf")),
];

fn open(bytes: &[u8], password: &str) -> Document {
    Document::from_bytes(bytes.to_vec(), Some(password), Limits::default()).unwrap()
}

fn red_pixels(doc: &Document) -> usize {
    let cache = doc.new_cache();
    let page = doc.render_page(&cache, 0, 1.0).unwrap();
    page.rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[0] > 180 && px[1] < 80 && px[2] < 80)
        .count()
}

#[test]
fn changes_save_into_encrypted_files() {
    for (method, bytes) in FIXTURES {
        {
            let password = "secret";
            let doc = open(bytes, password);
            let field = doc.info().unwrap().fields[0].id;
            let changes = Changes {
                add: vec![Annotation {
                    page: 0,
                    kind: Kind::Rect(TextBox {
                        x0: 100.0,
                        y0: 300.0,
                        x1: 300.0,
                        y1: 400.0,
                    }),
                    color: Color::RED,
                    opacity: 1.0,
                    width: 4.0,
                    contents: "Hello (world)".into(),
                    author: "Tester".into(),
                }],
                fields: vec![(field, FieldValue::Text("Jane Doe".into()))],
                ..Changes::default()
            };
            let saved = doc.with_changes(&changes).unwrap();
            assert_eq!(&saved[..bytes.len()], bytes, "{method}");
            let text = String::from_utf8_lossy(&saved[bytes.len()..]).into_owned();
            assert!(text.contains("/Encrypt 6 0 R"), "{method}: {text}");
            // Encrypted strings are random-looking hex, which can contain any four digits, but
            // a plain text string would start with the byte order mark.
            assert!(
                !text.contains("Tester") && !text.contains("<FEFF"),
                "{method}: strings left in the clear"
            );

            assert!(Document::from_bytes(saved.clone(), None, Limits::default()).is_err());
            let doc = open(&saved, "secret");
            let info = doc.info().unwrap();
            assert_eq!(info.comments.len(), 1, "{method}");
            assert_eq!(info.comments[0].contents, "Hello (world)", "{method}");
            assert_eq!(
                info.fields[0].value,
                FieldValue::Text("Jane Doe".into()),
                "{method}"
            );
            assert!(red_pixels(&doc) > 100, "{method}: the rectangle isn't drawn");
            // The original text is still readable.
            assert!(info.pages.len() == 1);
        }
    }
}

/// The app path: the worker shows unsaved changes, then saves them twice in a row.
#[test]
fn the_worker_keeps_the_password_for_changes_and_saves() {
    use std::sync::mpsc;
    use std::time::Duration;

    let dir = std::env::temp_dir().join(format!("panna-worker-crypt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("locked.pdf");
    std::fs::write(&path, FIXTURES[1].1).unwrap();
    let wait = Duration::from_secs(20);

    let (tx, rx) = mpsc::channel();
    let worker =
        panna_core::RenderWorker::open(path.clone(), Some("secret".into()), Limits::default(), move |r| {
            let _ = tx.send(r.is_ok());
        })
        .unwrap();
    assert!(rx.recv_timeout(wait).unwrap());

    let rect = |y0: f32| Annotation {
        page: 0,
        kind: Kind::Rect(TextBox {
            x0: 100.0,
            y0,
            x1: 300.0,
            y1: y0 + 50.0,
        }),
        color: Color([1.0, 0.0, 0.0]),
        opacity: 1.0,
        width: 4.0,
        contents: String::new(),
        author: String::new(),
    };
    for (round, y0) in [300.0, 400.0].into_iter().enumerate() {
        let (tx, rx) = mpsc::channel();
        worker.set_changes(
            Changes {
                add: vec![rect(y0)],
                ..Changes::default()
            },
            move |r| {
                let _ = tx.send(r);
            },
        );
        assert_eq!(rx.recv_timeout(wait).unwrap(), Ok(()), "round {round}");
        let (tx, rx) = mpsc::channel();
        worker.save(path.clone(), move |r| {
            let _ = tx.send(r.map(|c| c.len()));
        });
        assert_eq!(rx.recv_timeout(wait).unwrap(), Ok(round + 1));
        let (tx, rx) = mpsc::channel();
        worker.set_changes(Changes::default(), move |r| {
            let _ = tx.send(r);
        });
        assert_eq!(rx.recv_timeout(wait).unwrap(), Ok(()));
    }
    let saved = open(&std::fs::read(&path).unwrap(), "secret");
    assert_eq!(saved.comments().len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn old_protection_is_redone_with_aes_256_keeping_both_passwords() {
    for (name, bytes) in FIXTURES {
        let user = open(bytes, "secret");
        assert!(
            !user.info().unwrap().weak_protection,
            "{name}: opened without owner rights"
        );
        let owner = open(bytes, "owner");
        let info = owner.info().unwrap();
        let shown = info
            .properties
            .iter()
            .find(|(k, _)| *k == "Password protection")
            .unwrap();
        assert_eq!(shown.1, format!("Yes ({name})"));
        assert_eq!(info.weak_protection, name != "AES-256", "{name}");
        if name == "AES-256" {
            assert!(owner.strengthened().is_err());
            continue;
        }
        let (bytes, password) = owner.strengthened().unwrap();
        assert_eq!(*password, "owner");
        // The same passwords open it, now with AES-256, and the text is still there.
        let again = open(&bytes, "secret");
        let shown = again.info().unwrap().properties;
        assert!(
            shown.contains(&("Password protection".into(), "Yes (AES-256)".to_owned())),
            "{name}"
        );
        assert!(open(&bytes, "owner").info().is_ok());
        assert!(Document::from_bytes(bytes.clone(), Some("wrong"), Limits::default()).is_err());
        assert_eq!(red_pixels(&again), red_pixels(&owner), "{name}");
    }
}
