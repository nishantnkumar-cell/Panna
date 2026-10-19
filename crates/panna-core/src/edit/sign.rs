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
}

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
    let page = pages.first().ok_or(SaveError::NoSuchPage(0))?;
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
    let mut sig = format!(
        "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached\n/ByteRange [0 \
         0000000000 0000000000 0000000000]\n/M ({})\n/Name {}\n{reference}",
        pdf_date(std::time::SystemTime::now()),
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
    out.put(
        field_id,
        0,
        format!(
            "<< /Type /Annot /Subtype /Widget /FT /Sig /T {} /F 132 /Rect [0 0 0 0] /P {page_num} \
             {page_gen} R /V {sig_id} 0 R >>",
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
