#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers
use panna_core::{Document, Limits, SigStatus};

fn open(name: &str) -> Document {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    Document::open(&path, None, Limits::default()).unwrap()
}

#[test]
fn rsa_and_ecdsa_signatures_check_out() {
    for name in ["signed-rsa.pdf", "signed-ecdsa.pdf"] {
        let sigs = open(name).signatures();
        assert_eq!(sigs.len(), 1, "{name}");
        let s = &sigs[0];
        assert_eq!(s.status, SigStatus::Valid, "{name}");
        assert_eq!(s.field, "Signature1");
        assert_eq!(s.signer, "Test Signer");
        // The signer issued the certificate themselves.
        assert_eq!(s.issuer, "");
        assert_eq!((s.reason.as_str(), s.location.as_str()), ("I approve", "Leeds"));
        assert_eq!(s.page, Some(0));
        assert!(!s.changed_after);
        assert!(!s.outside_validity);
        assert!(s.time.is_some());
    }
}

#[test]
fn changes_are_noticed() {
    let s = &open("signed-then-changed.pdf").signatures()[0];
    assert_eq!(s.status, SigStatus::Valid);
    assert!(s.changed_after);

    // One byte of the signed part changed.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signed-rsa.pdf");
    let mut bytes = std::fs::read(path).unwrap();
    let at = bytes.windows(4).position(|w| w == b"Leed").unwrap();
    bytes[at] = b'S';
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    let s = &doc.signatures()[0];
    assert!(matches!(s.status, SigStatus::Invalid(_)), "{:?}", s.status);
}
