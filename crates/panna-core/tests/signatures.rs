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

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn plain() -> Document {
    let bytes = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages \
/Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>\nendobj\n";
    let mut out = bytes.to_vec();
    let offsets: Vec<usize> = [b"1 0 obj", b"2 0 obj", b"3 0 obj"]
        .iter()
        .map(|n| out.windows(7).position(|w| w == *n).unwrap())
        .collect();
    let xref = out.len();
    out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    Document::from_bytes(out, None, Limits::default()).unwrap()
}

#[test]
fn signing_with_a_certificate_file_makes_a_valid_signature() {
    let options = panna_core::SignOptions {
        reason: "Approved".into(),
        location: "Café".into(),
        certify: true,
    };
    for p12 in ["signer-rsa.p12", "signer-ec.p12"] {
        let doc = plain();
        let signed = doc.signed(&fixture(p12), "secret", &options).unwrap();
        let doc = doc.reopen(signed).unwrap();
        let sigs = doc.signatures();
        assert_eq!(sigs.len(), 1);
        let s = &sigs[0];
        assert_eq!(s.status, SigStatus::Valid, "{p12}");
        assert_eq!(s.signer, "Pat Example");
        assert_eq!((s.reason.as_str(), s.location.as_str()), ("Approved", "Café"));
        assert_eq!(s.certifies, Some(2));
        assert!(!s.changed_after);
        assert_eq!(s.page, Some(0));

        // A second signature leaves the first valid, for the version it signed.
        let again = doc
            .signed(&fixture(p12), "secret", &panna_core::SignOptions::default())
            .unwrap();
        let doc = doc.reopen(again).unwrap();
        let sigs = doc.signatures();
        assert_eq!(sigs.len(), 2);
        assert!(sigs.iter().all(|s| s.status == SigStatus::Valid));
        assert!(sigs[0].changed_after && !sigs[1].changed_after);
        assert_ne!(sigs[0].field, sigs[1].field);
        // Only the first signature can certify.
        assert_eq!(
            doc.signed(&fixture(p12), "secret", &options),
            Err(panna_core::SaveError::CertifySigned)
        );
    }
}

#[test]
fn a_wrong_password_is_refused() {
    let err = plain()
        .signed(
            &fixture("signer-rsa.p12"),
            "nope",
            &panna_core::SignOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(err, panna_core::SaveError::Certificate(_)), "{err:?}");
}
