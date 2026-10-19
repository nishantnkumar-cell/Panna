//! Adding a digital signature as an incremental update: a signature field (invisible, on the
//! first page) whose value holds room for the signature, and the byte range it will cover,
//! which is everything in the file except that room.

use std::ops::Range;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Object, ObjectIdentifier};

use super::{SaveError, lex, pdf_date, rewrite, update};
use crate::ObjRef;

/// Bytes of room for the signature (written as twice as many hex digits): enough for a key
/// of 4096 bits and a chain of several certificates.
pub(crate) const ROOM: usize = 24 * 1024;

/// What the signature says besides who signed it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignOptions {
    pub reason: String,
    pub location: String,
    /// Certify the document: later changes other than filling in forms and signing make the
    /// certification fail. Only the first signature can certify.
    pub certify: bool,
    /// Where the signature is shown on the page; it's invisible when `None`.
    pub visible: Option<Visible>,
}

/// A signature shown in a corner of a page: who signed, when, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visible {
    pub page: usize,
    pub corner: Corner,
}

/// A corner of the page as it's shown, whichever way the page is turned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

/// The shown signature's size and its distance from the page's edges, in points.
const BOX: (f64, f64) = (220.0, 56.0);
const MARGIN: f64 = 36.0;

/// A PDF string literal with `(`, `)` and `\` escaped, or a UTF-16 hex string when the text
/// isn't plain ASCII.
fn pdf_text(s: &str) -> String {
    if s.is_ascii() {
        let mut out = String::from("(");
        for c in s.chars().filter(|c| !c.is_control()) {
            if matches!(c, '(' | ')' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push(')');
        out
    } else {
        let mut out = String::from("<FEFF");
        for unit in s.encode_utf16() {
            out.push_str(&format!("{unit:04X}"));
        }
        out.push('>');
        out
    }
}

fn id_of(r: ObjRef) -> Result<ObjectIdentifier, SaveError> {
    Ok(ObjectIdentifier::new(
        i32::try_from(r.number).map_err(|_| SaveError::Damaged)?,
        i32::from(r.generation),
    ))
}

/// The items of an array, copied as they are, followed by `extra`.
fn array_with(existing: Option<Array<'_>>, extra: &str) -> Result<String, SaveError> {
    let mut out = String::from("[");
    if let Some(a) = existing {
        let data = a.data();
        for item in lex::array_items(data).ok_or(SaveError::Damaged)? {
            out.push_str(
                std::str::from_utf8(data.get(item).ok_or(SaveError::Damaged)?)
                    .map_err(|_| SaveError::Damaged)?,
            );
            out.push(' ');
        }
    }
    out.push_str(extra);
    out.push(']');
    Ok(out)
}

/// Where the box goes on a page with crop box `crop` turned by `rotate` degrees, in the
/// page's own space, and the matrix that draws its appearance upright as the page is shown.
fn placed(crop: [f64; 4], rotate: i64, corner: Corner) -> ([f64; 4], (f64, f64), &'static str) {
    let [x0, y0, x1, y1] = crop;
    let turned = rotate.rem_euclid(360);
    let (shown_w, shown_h) = if turned % 180 == 0 {
        (x1 - x0, y1 - y0)
    } else {
        (y1 - y0, x1 - x0)
    };
    // Smaller on small pages.
    let w = BOX.0.min((shown_w - 2.0 * MARGIN).max(40.0));
    let h = BOX.1.min((shown_h - 2.0 * MARGIN).max(14.0));
    let margin = MARGIN.min(shown_w / 10.0).min(shown_h / 10.0);
    let (u0, v0) = match corner {
        Corner::TopLeft => (margin, shown_h - margin - h),
        Corner::TopRight => (shown_w - margin - w, shown_h - margin - h),
        Corner::BottomLeft => (margin, margin),
        Corner::BottomRight => (shown_w - margin - w, margin),
    };
    // From where it's shown (u across, v up) back to the page's own space.
    let to_page = |u: f64, v: f64| match turned {
        90 => (x1 - v, y0 + u),
        180 => (x1 - u, y1 - v),
        270 => (x0 + v, y1 - u),
        _ => (x0 + u, y0 + v),
    };
    let (a, b) = to_page(u0, v0);
    let (c, d) = to_page(u0 + w, v0 + h);
    let matrix = match turned {
        90 => "0 1 -1 0 0 0",
        180 => "-1 0 0 -1 0 0",
        270 => "0 -1 1 0 0 0",
        _ => "1 0 0 1 0 0",
    };
    ([a.min(c), b.min(d), a.max(c), b.max(d)], (w, h), matrix)
}

/// What the shown signature draws in a box `w` by `h`: its lines, each shrunk to fit.
fn appearance(lines: &[String], (w, h): (f64, f64), matrix: &str) -> Vec<u8> {
    let n = super::annot::n;
    let mut data = format!(
        "q 0.93 0.94 0.98 rg 0 0 {w} {h} re f 0.35 0.38 0.6 RG 0.75 w 0.375 0.375 {iw} {ih} re S Q\n",
        w = n(w),
        h = n(h),
        iw = n(w - 0.75),
        ih = n(h - 0.75)
    );
    let room = (w - 12.0).max(1.0);
    let line_h = ((h - 8.0) / lines.len().max(1) as f64).min(14.0);
    let mut y = h - 4.0;
    for (i, line) in lines.iter().enumerate() {
        let wanted: f64 = if i == 0 { 11.0 } else { 9.0 };
        let size = wanted
            .min(line_h * 0.85)
            .min(room / super::annot::text_width(line).max(0.01))
            .max(1.0);
        y -= line_h;
        data.push_str(&format!(
            "BT /Helv {} Tf 0.12 0.15 0.35 rg 6 {} Td ({}) Tj ET\n",
            n(size),
            n(y + (line_h - size) / 2.0),
            super::annot::literal(&super::annot::win_ansi(line))
        ));
    }
    format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 {} {}] /Matrix [{matrix}] /Resources << /Font << \
         /Helv << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> >> >> \
         /Length {} >>\nstream\n{data}endstream",
        n(w),
        n(h),
        data.len()
    )
    .into_bytes()
}

/// `original` with an update adding an unsigned signature, and where in the result the room
/// for it is (the hex string, brackets included). Every byte outside that room is final.
pub(crate) fn prepare(
    pdf: &Pdf,
    original: &[u8],
    signer: &str,
    options: &SignOptions,
    signed_before: bool,
) -> Result<(Vec<u8>, Range<usize>), SaveError> {
    let trailer = update::read_trailer(original)?;
    if trailer.encrypt.is_some() {
        return Err(SaveError::SigningProtected);
    }
    if options.certify && signed_before {
        return Err(SaveError::CertifySigned);
    }
    let root = lex::reference(trailer.root.trim_ascii()).ok_or(SaveError::Damaged)?;
    let xref = pdf.xref();
    let catalog = xref.get::<Dict<'_>>(id_of(root)?).ok_or(SaveError::Damaged)?;
    let pages = pdf.pages();
    let on = options.visible.map_or(0, |v| v.page);
    let page = pages.get(on).ok_or(SaveError::NoSuchPage(on))?;
    let page_raw = page.raw();
    let page_id = page_raw.obj_id().ok_or(SaveError::Damaged)?;
    let (page_num, page_gen) = (
        u32::try_from(page_id.obj_number).map_err(|_| SaveError::Damaged)?,
        u16::try_from(page_id.gen_number).map_err(|_| SaveError::Damaged)?,
    );

    let mut out = update::Update::new(trailer);
    let sig_id = out.alloc();
    let field_id = out.alloc();

    // A name no other field has.
    let acro = catalog.get::<Dict<'_>>(b"AcroForm");
    let taken = |name: &str| {
        acro.as_ref()
            .and_then(|a| a.get::<Array<'_>>(b"Fields"))
            .is_some_and(|fields| {
                fields.iter::<Dict<'_>>().any(|f| {
                    matches!(f.get::<Object<'_>>(b"T"), Some(Object::String(s)) if s.as_bytes() == name.as_bytes())
                })
            })
    };
    let name = (1..1000)
        .map(|n| format!("Signature{n}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| "Signature".into());

    let reference = if options.certify {
        "/Reference [<< /Type /SigRef /TransformMethod /DocMDP /TransformParams << /Type \
         /TransformParams /P 2 /V /1.2 >> >>]\n"
    } else {
        ""
    };
    let date = pdf_date(std::time::SystemTime::now());
    let mut sig = format!(
        "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached\n/ByteRange [0 \
         0000000000 0000000000 0000000000]\n/M ({date})\n/Name {}\n{reference}",
        pdf_text(signer),
    );
    if !options.reason.trim().is_empty() {
        sig.push_str(&format!("/Reason {}\n", pdf_text(options.reason.trim())));
    }
    if !options.location.trim().is_empty() {
        sig.push_str(&format!("/Location {}\n", pdf_text(options.location.trim())));
    }
    sig.push_str("/Contents <");
    sig.push_str(&"0".repeat(ROOM * 2));
    sig.push_str(">\n>>");
    out.put(sig_id, 0, sig.into_bytes())?;
    // Shown: a box in a corner with who signed, when and why. Otherwise a widget with no
    // size. Either way it prints and is locked (flags 132).
    let (rect, ap) = match options.visible {
        Some(visible) => {
            let b = page.crop_box();
            let rotate = match page.rotation() {
                hayro::hayro_syntax::page::Rotation::None => 0,
                hayro::hayro_syntax::page::Rotation::Horizontal => 90,
                hayro::hayro_syntax::page::Rotation::Flipped => 180,
                hayro::hayro_syntax::page::Rotation::FlippedHorizontal => 270,
            };
            let (rect, size, matrix) = placed([b.x0, b.y0, b.x1, b.y1], rotate, visible.corner);
            // "D:20261019120000Z" as "2026-10-19 12:00 UTC".
            let d = date.get(2..).unwrap_or_default();
            let part = |r: std::ops::Range<usize>| d.get(r).unwrap_or_default();
            let mut lines = vec![
                format!("Digitally signed by {signer}"),
                format!(
                    "Date: {}-{}-{} {}:{} UTC",
                    part(0..4),
                    part(4..6),
                    part(6..8),
                    part(8..10),
                    part(10..12)
                ),
            ];
            if !options.reason.trim().is_empty() {
                lines.push(format!("Reason: {}", options.reason.trim()));
            }
            if !options.location.trim().is_empty() {
                lines.push(format!("Place: {}", options.location.trim()));
            }
            let ap_id = out.alloc();
            out.put(ap_id, 0, appearance(&lines, size, matrix))?;
            let n = super::annot::n;
            (
                format!("[{} {} {} {}]", n(rect[0]), n(rect[1]), n(rect[2]), n(rect[3])),
                format!("/AP << /N {ap_id} 0 R >> "),
            )
        }
        None => ("[0 0 0 0]".to_owned(), String::new()),
    };
    out.put(
        field_id,
        0,
        format!(
            "<< /Type /Annot /Subtype /Widget /FT /Sig /T {} /F 132 /Rect {rect} {ap}/P \
             {page_num} {page_gen} R /V {sig_id} 0 R >>",
            pdf_text(&name)
        )
        .into_bytes(),
    )?;

    // The page lists the new field among its annotations.
    let annots = array_with(page_raw.get::<Array<'_>>(b"Annots"), &format!("{field_id} 0 R"))?;
    out.put_copied(
        page_num,
        page_gen,
        rewrite(page_raw.data(), &vec![("Annots", annots)])?,
    );

    // The form lists it among its fields, and says it's signed.
    let mut catalog_set: Vec<(&'static str, String)> = Vec::new();
    let fields = array_with(
        acro.as_ref().and_then(|a| a.get::<Array<'_>>(b"Fields")),
        &format!("{field_id} 0 R"),
    )?;
    let form_set = vec![("Fields", fields), ("SigFlags", "3".to_owned())];
    match (catalog.get_ref(b"AcroForm"), &acro) {
        (Some(r), Some(form)) => {
            let num = u32::try_from(r.obj_number).map_err(|_| SaveError::Damaged)?;
            let generation = u16::try_from(r.gen_number).map_err(|_| SaveError::Damaged)?;
            out.put_copied(num, generation, rewrite(form.data(), &form_set)?);
        }
        (_, Some(form)) => {
            let body = rewrite(form.data(), &form_set)?;
            catalog_set.push((
                "AcroForm",
                String::from_utf8(body).map_err(|_| SaveError::Damaged)?,
            ));
        }
        (_, None) => {
            catalog_set.push(("AcroForm", format!("<< /Fields [{field_id} 0 R] /SigFlags 3 >>")));
        }
    }
    if options.certify {
        catalog_set.push(("Perms", format!("<< /DocMDP {sig_id} 0 R >>")));
    }
    if !catalog_set.is_empty() {
        out.put_copied(
            root.number,
            root.generation,
            rewrite(catalog.data(), &catalog_set)?,
        );
    }
    let mut bytes = out.finish(original);

    // The room and the byte range, found in what was written.
    let marker = format!("{sig_id} 0 obj\n");
    let at = bytes
        .windows(marker.len())
        .rposition(|w| w == marker.as_bytes())
        .ok_or(SaveError::Damaged)?;
    let find = |needle: &[u8], from: usize| {
        bytes
            .get(from..)
            .and_then(|rest| rest.windows(needle.len()).position(|w| w == needle))
            .map(|p| p + from)
    };
    let start = find(b"/Contents <", at).ok_or(SaveError::Damaged)? + b"/Contents ".len();
    let end = start + ROOM * 2 + 2;
    let range_at = find(b"/ByteRange [0 ", at).ok_or(SaveError::Damaged)? + b"/ByteRange [0 ".len();
    let range = format!("{start:010} {end:010} {:010}", bytes.len() - end);
    bytes
        .get_mut(range_at..range_at + range.len())
        .ok_or(SaveError::Damaged)?
        .copy_from_slice(range.as_bytes());
    Ok((bytes, start..end))
}
