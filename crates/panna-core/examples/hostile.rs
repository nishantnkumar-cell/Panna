//! Builds one hostile PDF by name and runs it through every read and save path, so a harness
//! can watch the process's time and memory. `cargo run --release --example hostile -- <case>`.
//! Prints the steps it got through; a crash or abort shows which step took it down.

use std::io::Write;
use std::time::Instant;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use panna_core::{Changes, Document, Limits};

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

fn s(x: &str) -> Vec<u8> {
    x.as_bytes().to_vec()
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    v.extend_from_slice(data);
    v.extend_from_slice(b"\nendstream");
    v
}

/// `n` bytes of `byte`, zlib-compressed without ever holding them in memory.
fn bomb(n: usize, byte: u8) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
    let chunk = vec![byte; 1 << 20];
    let mut left = n;
    while left > 0 {
        let k = left.min(chunk.len());
        z.write_all(chunk.get(..k).unwrap_or_default()).ok();
        left -= k;
    }
    z.finish().unwrap_or_default()
}

const CATALOG: &str = "<< /Type /Catalog /Pages 2 0 R >>";
const PAGES: &str = "<< /Type /Pages /Kids [3 0 R] /Count 1 >>";

fn page(extra: &str) -> Vec<u8> {
    s(&format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R {extra} >>"
    ))
}

fn build(case: &str) -> Vec<u8> {
    const GB: usize = 1 << 30;
    match case {
        // 2 GiB of spaces behind a 2 MB content stream.
        "flate-content" => pdf(&[
            s(CATALOG),
            s(PAGES),
            page(""),
            stream("/Filter /FlateDecode", &bomb(2 * GB, b' ')),
        ]),
        // An attached file that inflates to 2 GiB.
        "flate-attachment" => pdf(&[
            s("<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Names [(a.txt) 5 0 R] >> >> >>"),
            s(PAGES),
            page(""),
            stream("", b"0 0 m"),
            s("<< /Type /Filespec /F (a.txt) /EF << /F 6 0 R >> >>"),
            stream("/Type /EmbeddedFile /Filter /FlateDecode", &bomb(2 * GB, 0)),
        ]),
        // XMP metadata that inflates to 2 GiB (read by preflight and properties).
        "flate-xmp" => pdf(&[
            s("<< /Type /Catalog /Pages 2 0 R /Metadata 5 0 R >>"),
            s(PAGES),
            page(""),
            stream("", b"0 0 m"),
            stream(
                "/Type /Metadata /Subtype /XML /Filter /FlateDecode",
                &bomb(2 * GB, b' '),
            ),
        ]),
        // A form XObject that draws itself.
        "xobject-loop" => pdf(&[
            s(CATALOG),
            s(PAGES),
            page("/Resources << /XObject << /X 5 0 R >> >>"),
            stream("", b"/X Do"),
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /X 5 0 R >> >>",
                b"/X Do",
            ),
        ]),
        // Two form XObjects that each draw the other twice: 2^depth work if unchecked.
        "xobject-fanout" => pdf(&[
            s(CATALOG),
            s(PAGES),
            page("/Resources << /XObject << /A 5 0 R >> >>"),
            stream("", b"/A Do"),
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /B 6 0 R >> >>",
                b"/B Do /B Do",
            ),
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /A 5 0 R >> >>",
                b"/A Do /A Do",
            ),
        ]),
        // A million nested q operators and arrays in the content stream.
        "deep-content" => {
            let mut c = "q ".repeat(1_000_000);
            c.push_str(&"[".repeat(200_000));
            pdf(&[s(CATALOG), s(PAGES), page(""), stream("", c.as_bytes())])
        }
        // A page tree whose Kids point back at itself.
        "page-tree-loop" => pdf(&[
            s(CATALOG),
            s("<< /Type /Pages /Kids [2 0 R 3 0 R 2 0 R] /Count 3 >>"),
            page(""),
            stream("", b"0 0 m"),
        ]),
        // A page tree that claims a billion pages.
        "page-count-lie" => pdf(&[
            s(CATALOG),
            s("<< /Type /Pages /Kids [3 0 R] /Count 1000000000 >>"),
            page(""),
            stream("", b"0 0 m"),
        ]),
        // An image that says it is 60000 × 60000 pixels, with almost no data.
        "huge-image" => pdf(&[
            s(CATALOG),
            s(PAGES),
            page("/Resources << /XObject << /I 5 0 R >> >>"),
            stream("", b"q 612 0 0 792 0 0 cm /I Do Q"),
            stream(
                "/Type /XObject /Subtype /Image /Width 60000 /Height 60000 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode",
                &bomb(1 << 20, 0x80),
            ),
        ]),
        // A tiling pattern whose cell paints with the same pattern.
        "pattern-loop" => pdf(&[
            s(CATALOG),
            s(PAGES),
            page("/Resources << /Pattern << /P 5 0 R >> >>"),
            stream("", b"/Pattern cs /P scn 0 0 612 792 re f"),
            stream(
                "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 1 1] /XStep 1 /YStep 1 /Resources << /Pattern << /P 5 0 R >> >>",
                b"/Pattern cs /P scn 0 0 1 1 re f",
            ),
        ]),
        // An annotation appearance that draws itself, and an outline that loops.
        "annot-outline-loop" => pdf(&[
            s("<< /Type /Catalog /Pages 2 0 R /Outlines 7 0 R >>"),
            s(PAGES),
            page("/Annots [5 0 R]"),
            stream("", b"0 0 m"),
            s("<< /Type /Annot /Subtype /Square /Rect [0 0 100 100] /AP << /N 6 0 R >> >>"),
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /S 6 0 R >> >>",
                b"/S Do",
            ),
            s("<< /Type /Outlines /First 8 0 R /Last 8 0 R >>"),
            s("<< /Title (loop) /Parent 7 0 R /First 8 0 R /Next 8 0 R /Dest [3 0 R /Fit] >>"),
        ]),
        // A form field tree that is its own kid, a thousand levels of AcroForm nesting.
        "field-loop" => pdf(&[
            s("<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] >> >>"),
            s(PAGES),
            page("/Annots [5 0 R]"),
            stream("", b"0 0 m"),
            s("<< /FT /Tx /T (a) /Kids [5 0 R] /Parent 5 0 R /Rect [0 0 10 10] /Subtype /Widget >>"),
        ]),
        // A cross-reference section whose /Prev points at itself.
        "xref-loop" => {
            let mut v = pdf(&[s(CATALOG), s(PAGES), page(""), stream("", b"0 0 m")]);
            let at = String::from_utf8_lossy(&v).rfind("trailer").unwrap_or(0);
            let xref = String::from_utf8_lossy(&v).rfind("xref\n").unwrap_or(0);
            v.truncate(at);
            v.extend_from_slice(
                format!("trailer\n<< /Size 5 /Root 1 0 R /Prev {xref} >>\nstartxref\n{xref}\n%%EOF\n")
                    .as_bytes(),
            );
            v
        }
        // A real 8 × 8 JPEG whose header is patched to claim 65535 × 65535 pixels.
        "jpeg-huge" => {
            let mut jpeg = Vec::new();
            let pixels = vec![128u8; 8 * 8 * 3];
            image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
                .encode(&pixels, 8, 8, image::ExtendedColorType::Rgb8)
                .ok();
            // SOF0 marker: FF C0, length (2), precision (1), height (2), width (2).
            if let Some(at) = jpeg.windows(2).position(|w| w == [0xFF, 0xC0]) {
                for i in [at + 5, at + 6, at + 7, at + 8] {
                    if let Some(b) = jpeg.get_mut(i) {
                        *b = 0xFF;
                    }
                }
            }
            pdf(&[
                s(CATALOG),
                s(PAGES),
                page("/Resources << /XObject << /I 5 0 R >> >>"),
                stream("", b"q 612 0 0 792 0 0 cm /I Do Q"),
                stream(
                    "/Type /XObject /Subtype /Image /Width 65535 /Height 65535 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode",
                    &jpeg,
                ),
            ])
        }
        // An object stream (parsed while opening) that inflates to 2 GiB.
        "flate-objstm" => pdf(&[
            s(CATALOG),
            s(PAGES),
            page(""),
            stream("", b"0 0 m"),
            stream(
                "/Type /ObjStm /N 1 /First 4 /Filter /FlateDecode",
                &bomb(2 * GB, b' '),
            ),
        ]),
        // Twenty content streams of 200 MiB each, joined into one page.
        "content-array" => {
            let one = bomb(200 << 20, b' ');
            let mut objects = vec![s(CATALOG), s(PAGES)];
            let refs: Vec<String> = (0..20).map(|i| format!("{} 0 R", 4 + i)).collect();
            objects.push(s(&format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents [{}] >>",
                refs.join(" ")
            )));
            for _ in 0..20 {
                objects.push(stream("/Filter /FlateDecode", &one));
            }
            pdf(&objects)
        }
        // Two million tiny path segments.
        "many-ops" => {
            let c = "1 1 m 2 2 l ".repeat(1_000_000) + "S";
            pdf(&[s(CATALOG), s(PAGES), page(""), stream("", c.as_bytes())])
        }
        // A real file, to time the open-time checks on ordinary documents.
        path if path.starts_with("file:") => std::fs::read(&path[5..]).unwrap_or_default(),
        _ => {
            eprintln!("unknown case");
            std::process::exit(2);
        }
    }
}

fn step(name: &str, t: Instant, f: impl FnOnce() -> String) {
    print!("{name}…");
    std::io::stdout().flush().ok();
    let r = f();
    println!(" {r} ({} ms)", t.elapsed().as_millis());
}

fn main() {
    let case = std::env::args().nth(1).unwrap_or_default();
    let bytes = build(&case);
    println!("{case}: {} bytes", bytes.len());
    let t = Instant::now();
    let opened = Instant::now();
    let doc = match Document::from_bytes(bytes, None, Limits::default()) {
        Ok(d) => d,
        Err(e) => {
            println!("open: rejected: {e}");
            return;
        }
    };
    println!("open: {} ms", opened.elapsed().as_millis());
    step("info", t, || format!("{:?}", doc.info().map(|i| i.pages.len())));
    let cache = doc.new_cache();
    for i in 0..doc.page_count().min(3) {
        step(&format!("render {i}"), t, || {
            format!("{:?}", doc.render_page(&cache, i, 1.0).map(|p| p.width))
        });
        step(&format!("text {i}"), t, || {
            format!("{:?}", doc.page_text(i).map(|p| p.len()))
        });
    }
    step("attachment", t, || {
        format!("{:?}", doc.attachment(0).map(|a| a.len()))
    });
    step("signatures", t, || doc.signatures().len().to_string());
    step("preflight", t, || doc.preflight().len().to_string());
    step("accessibility", t, || doc.accessibility().len().to_string());
    step("comments", t, || doc.comments().len().to_string());
    step("save", t, || {
        format!("{:?}", doc.with_changes(&Changes::default()).map(|b| b.len()))
    });
    step("compact", t, || {
        format!("{:?}", doc.compacted(None).map(|b| b.len()))
    });
    step("flatten", t, || {
        format!("{:?}", doc.flattened(true, true).map(|b| b.len()))
    });
    println!("done");
}
