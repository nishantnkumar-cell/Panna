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
