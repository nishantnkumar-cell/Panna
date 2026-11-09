#![allow(clippy::unwrap_used)] // test helpers

//! Opening through the worker, as the app does, must say why a file didn't open: a password
//! prompt depends on it, and so does the message for a refused file.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use panna_core::{DocumentInfo, Limits, OpenError, RenderWorker};

fn open(path: PathBuf, password: Option<&str>, limits: Limits) -> Result<DocumentInfo, OpenError> {
    let (tx, rx) = mpsc::channel();
    let _worker = RenderWorker::open(path, password.map(str::to_owned), limits, move |r| {
        let _ = tx.send(r);
    })
    .unwrap();
    rx.recv_timeout(Duration::from_secs(30)).unwrap()
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn the_worker_says_a_password_is_needed() {
    let file = fixture("encrypted-aes256.pdf");
    for wrong in [None, Some("nope")] {
        assert_eq!(
            open(file.clone(), wrong, Limits::default()),
            Err(OpenError::PasswordRequired)
        );
    }
    assert!(open(file, Some("secret"), Limits::default()).is_ok());
}

#[test]
fn the_worker_says_why_a_file_was_refused() {
    let tiny = Limits {
        max_image_pixels: 1,
        ..Limits::default()
    };
    // The fixture has no pictures, so use the page limit instead: one page allowed of one.
    let few = Limits {
        max_pages: 0,
        ..Limits::default()
    };
    let file = fixture("encrypted-aes256.pdf");
    assert!(matches!(
        open(file.clone(), Some("secret"), few),
        Err(OpenError::TooManyPages { .. })
    ));
    assert!(open(file, Some("secret"), tiny).is_ok());
}
