#![allow(clippy::unwrap_used, clippy::indexing_slicing)] // test helpers

//! Files built to exhaust memory must be refused when opened, before anything is decoded.
//! Small limits keep these fast; `examples/hostile.rs` runs full-size versions.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use panna_core::{Document, Limits, OpenError};

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
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

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    v.extend_from_slice(data);
    v.extend_from_slice(b"\nendstream");
    v
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
    z.write_all(data).unwrap();
    z.finish().unwrap()
}

const CATALOG: &[u8] = b"<< /Type /Catalog /Pages 2 0 R >>";
const PAGES: &[u8] = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>";

fn page(contents: &str, extra: &str) -> Vec<u8> {
    format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {contents} {extra} >>")
        .into_bytes()
}

/// 1 MiB may be decoded per stream or page; images may have a million pixels.
fn small() -> Limits {
    Limits {
        max_decoded_bytes: 1 << 20,
        max_image_pixels: 1_000_000,
        ..Limits::default()
    }
}

fn refused(bytes: Vec<u8>) -> bool {
    matches!(
        Document::from_bytes(bytes, None, small()),
        Err(OpenError::Hostile(_))
    )
}

#[test]
fn refuses_a_stream_that_inflates_past_the_limit() {
    let big = zlib(&vec![b' '; 2 << 20]);
    assert!(refused(pdf(&[
        CATALOG.into(),
        PAGES.into(),
        page("4 0 R", ""),
        stream("/Filter /FlateDecode", &big)
    ])));
    // Twice-compressed data is followed through both stages.
    let twice = zlib(&big);
    assert!(refused(pdf(&[
        CATALOG.into(),
        PAGES.into(),
        page("4 0 R", ""),
        stream("/Filter [/FlateDecode /FlateDecode]", &twice),
    ])));
    // So is compressed data written out as hexadecimal text.
    let hex: String = big.iter().map(|b| format!("{b:02x}")).collect::<String>() + ">";
    assert!(refused(pdf(&[
        CATALOG.into(),
        PAGES.into(),
        page("4 0 R", ""),
        stream("/Filter [/ASCIIHexDecode /FlateDecode]", hex.as_bytes()),
    ])));
}

#[test]
fn refuses_a_page_whose_streams_add_up_past_the_limit() {
    let part = zlib(&vec![b' '; 600 << 10]);
    let objects = vec![
        CATALOG.into(),
        PAGES.into(),
        page("[4 0 R 5 0 R]", ""),
        stream("/Filter /FlateDecode", &part),
        stream("/Filter /FlateDecode", &part),
    ];
    assert!(refused(pdf(&objects)));
}

#[test]
fn refuses_images_that_claim_too_many_pixels() {
    let image = |dict: &str, data: &[u8]| {
        pdf(&[
            CATALOG.into(),
            PAGES.into(),
            page("4 0 R", "/Resources << /XObject << /I 5 0 R >> >>"),
            stream("", b"/I Do"),
            stream(&format!("/Type /XObject /Subtype /Image {dict}"), data),
        ])
    };
    assert!(refused(image(
        "/Width 2000 /Height 2000 /ColorSpace /DeviceGray /BitsPerComponent 8",
        b""
    )));
    // A JPEG whose own header claims 4096 × 4096, whatever the dictionary says.
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
        .encode(&[128u8; 8 * 8 * 3], 8, 8, image::ExtendedColorType::Rgb8)
        .unwrap();
    let sof = jpeg.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    jpeg[sof + 5..sof + 9].copy_from_slice(&[0x10, 0x00, 0x10, 0x00]);
    assert!(refused(image(
        "/Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode",
        &jpeg
    )));
}

#[test]
fn ordinary_files_still_open() {
    let content = zlib(&b"0 0 m 100 100 l S ".repeat(1000));
    let doc = Document::from_bytes(
        pdf(&[
            CATALOG.into(),
            PAGES.into(),
            page("4 0 R", ""),
            stream("/Filter /FlateDecode", &content),
        ]),
        None,
        small(),
    );
    assert!(doc.is_ok());
}

#[test]
fn attachments_too_large_to_unpack_are_not_unpacked() {
    let bytes = pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Names [(a.txt) 5 0 R] >> >> >>"
            .to_vec(),
        PAGES.into(),
        page("4 0 R", ""),
        stream("", b"0 0 m"),
        b"<< /Type /Filespec /F (a.txt) /EF << /F 6 0 R >> >>".to_vec(),
        stream(
            "/Type /EmbeddedFile /Filter /FlateDecode",
            &zlib(&vec![0; 3 << 20]),
        ),
    ]);
    // Attachments aren't checked at open (they may be large), only when saved.
    let limits = Limits {
        max_file_bytes: 2 << 20,
        ..small()
    };
    let doc = Document::from_bytes(bytes.clone(), None, limits).unwrap();
    assert_eq!(doc.attachment(0), None);
    let roomy = Document::from_bytes(bytes, None, Limits::default()).unwrap();
    assert_eq!(roomy.attachment(0).map(|a| a.len()), Some(3 << 20));
}
