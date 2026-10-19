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
    plain_turned(0)
}

fn plain_turned(rotate: u32) -> Document {
    let bytes = format!(
        "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages \
/Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Rotate {rotate} >>\nendobj\n"
    );
    let mut out = bytes.into_bytes();
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
        visible: None,
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

#[test]
fn a_shown_signature_is_drawn_in_the_corner_as_the_page_is_shown() {
    for rotate in [0, 90, 180, 270] {
        let options = panna_core::SignOptions {
            reason: "Approved".into(),
            visible: Some(panna_core::Visible {
                page: 0,
                corner: panna_core::Corner::BottomRight,
            }),
            ..panna_core::SignOptions::default()
        };
        let doc = plain_turned(rotate);
        let signed = doc
            .signed(&fixture("signer-rsa.p12"), "secret", &options)
            .unwrap();
        let doc = doc.reopen(signed).unwrap();
        assert_eq!(doc.signatures()[0].status, SigStatus::Valid);
        let cache = doc.new_cache();
        let page = doc.render_page(&cache, 0, 1.0).unwrap();
        let (w, h) = (page.width as usize, page.height as usize);
        let white = |x: usize, y: usize| page.rgba[(y * w + x) * 4..][..3] == [255, 255, 255];
        // On a 200-point page the box is 128 by 56, 20 from the edges.
        let inside = (54..178)
            .flat_map(|x| (126..178).map(move |y| (x, y)))
            .filter(|&(x, y)| !white(x, y))
            .count();
        assert!(inside > 124 * 52 * 9 / 10, "rotate {rotate}: {inside}");
        assert!(
            white(30, 30) && white(30, h - 30) && white(w - 30, 30),
            "rotate {rotate}"
        );
    }
}

#[test]
fn rsa_pss_signatures_check_out() {
    let sigs = open("signed-rsa-pss.pdf").signatures();
    assert_eq!(sigs[0].status, SigStatus::Valid);
    assert_eq!(sigs[0].signer, "Test Signer");
    // One changed byte in the signed part (the page's size) makes it fail.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signed-rsa-pss.pdf");
    let mut bytes = std::fs::read(path).unwrap();
    let at = bytes.windows(15).position(|w| w == b"MediaBox [0 0 2").unwrap() + 14;
    bytes[at] = b'3';
    let doc = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    assert!(matches!(doc.signatures()[0].status, SigStatus::Invalid(_)));
}

#[test]
fn certificates_lead_to_the_ones_trusted() {
    let fixture_path = |n: &str| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(n)
    };
    let ca = panna_core::Cert::from_file(&std::fs::read(fixture_path("test-ca.pem")).unwrap()).unwrap();
    assert_eq!((ca.name.as_str(), ca.issuer.as_str()), ("Panna Test CA", ""));
    assert_eq!(ca.fingerprint.len(), 64);

    let with_ca = open("signed-by-ca.pdf").signatures().remove(0);
    assert_eq!(with_ca.status, SigStatus::Valid);
    let names: Vec<&str> = with_ca.chain.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["CA Signer", "Panna Test CA"]);
    assert_eq!(with_ca.chain[1].fingerprint, ca.fingerprint);

    let alone = open("signed-by-ca-alone.pdf").signatures().remove(0);
    assert_eq!(alone.chain.len(), 1);
    assert_eq!(alone.issuer, "Panna Test CA");

    let trusted = [ca];
    for sig in [&with_ca, &alone] {
        assert_eq!(
            panna_core::trusted_by(sig, &trusted).map(|c| c.name.as_str()),
            Some("Panna Test CA")
        );
    }
    // A self-signed certificate is trusted only when it's on the list itself.
    let own = open("signed-rsa.pdf").signatures().remove(0);
    assert!(panna_core::trusted_by(&own, &trusted).is_none());
    assert!(panna_core::trusted_by(&own, std::slice::from_ref(&own.chain[0])).is_some());
}

/// A key in the system's certificate store, as the app reaches it: only its certificates
/// and a way to sign a digest, giving raw signatures as Windows does.
fn held(p12: &str) -> panna_core::DigitalId {
    use p256::pkcs8::DecodePrivateKey;
    let store = p12_keystore::KeyStore::from_pkcs12(&fixture(p12), "secret").unwrap();
    let (_, chain) = store.private_key_chain().unwrap();
    let certs = chain.chain().iter().map(|c| c.as_der().to_vec()).collect();
    let key = chain.key().to_vec();
    let sign: panna_core::SignDigest = if let Ok(k) = rsa::RsaPrivateKey::from_pkcs8_der(&key) {
        std::sync::Arc::new(move |digest: &[u8]| {
            k.sign(rsa::Pkcs1v15Sign::new::<sha2::Sha256>(), digest)
                .map_err(|e| e.to_string())
        })
    } else {
        let k = p256::ecdsa::SigningKey::from_pkcs8_der(&key).unwrap();
        std::sync::Arc::new(move |digest: &[u8]| {
            use p256::ecdsa::signature::hazmat::PrehashSigner;
            let sig: p256::ecdsa::Signature = k.sign_prehash(digest).map_err(|e| e.to_string())?;
            Ok(sig.to_bytes().to_vec())
        })
    };
    panna_core::DigitalId::Held(panna_core::HeldKey { chain: certs, sign })
}

#[test]
fn signing_with_a_key_held_elsewhere_makes_a_valid_signature() {
    for p12 in ["signer-rsa.p12", "signer-ec.p12"] {
        let doc = plain();
        let signed = doc
            .signed_by(&held(p12), &panna_core::SignOptions::default())
            .unwrap();
        let sigs = doc.reopen(signed).unwrap().signatures();
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].status, SigStatus::Valid, "{p12}");
        assert_eq!(sigs[0].signer, "Pat Example");
    }
}
