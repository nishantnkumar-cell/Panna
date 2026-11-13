//! The reading order of a tagged document, as Acrobat's Reading Order tool: the top-level
//! structure elements (paragraphs, headings, figures…) in the order a screen reader reads
//! them, each with its page, roughly where it is, and a preview of its text. They can be put
//! in another order and given other roles, saved as an incremental update.

use std::collections::{HashMap, HashSet};

use hayro::hayro_interpret::TransformExt;
use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Dict, MaybeRef, Name, Object, ObjectIdentifier};

use super::{SaveError, rewrite, update};
use crate::{ObjRef, TextBox};

/// Elements listed at most.
const MAX_ITEMS: usize = 20_000;
/// Levels gone down into an element to find what it marks.
const MAX_DEPTH: usize = 6;
/// Characters of preview kept.
const PREVIEW: usize = 120;

/// Roles that can be given.
pub const ROLES: [&str; 13] = [
    "P", "H1", "H2", "H3", "H4", "H5", "H6", "Figure", "Table", "L", "Note", "Caption", "Sect",
];

/// One element in reading order.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OrderItem {
    pub id: ObjRef,
    /// Its role (`/S`), such as P, H1 or Figure.
    pub role: String,
    pub page: Option<usize>,
    /// Its text, or its description for a figure; empty when there's nothing to show.
    pub preview: String,
    /// About where it is on its page, in the page's shown space.
    pub area: Option<TextBox>,
}

/// The document's reading order.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReadingOrder {
    pub items: Vec<OrderItem>,
    /// Whether it can be changed here: every element is an object of its own.
    pub editable: bool,
    /// Why it can't be read or changed, when that's so.
    pub note: Option<String>,
}

fn id_of(r: &ObjRef) -> ObjectIdentifier {
    ObjectIdentifier::new(r.number, i32::from(r.generation))
}

fn obj_ref(id: ObjectIdentifier) -> Option<ObjRef> {
    ObjRef::new(id.obj_number, id.gen_number)
}

/// The structure tree root, and the element holding the reading order: the root's only
/// child (usually /Document) or the root itself.
fn container<'a>(pdf: &'a Pdf) -> Option<(Dict<'a>, ObjectIdentifier)> {
    let xref = pdf.xref();
    let catalog = xref.get::<Dict<'_>>(xref.root_id())?;
    let root_id = match catalog.get_raw::<Dict<'_>>(b"StructTreeRoot")? {
        MaybeRef::Ref(r) => ObjectIdentifier::from(r),
        MaybeRef::NotRef(_) => return None,
    };
    let root = xref.get::<Dict<'_>>(root_id)?;
    let single = match root.get_raw::<Object<'_>>(b"K") {
        Some(MaybeRef::Ref(r)) => Some(ObjectIdentifier::from(r)),
        Some(MaybeRef::NotRef(Object::Array(a))) if a.raw_iter().count() == 1 => match a.raw_iter().next() {
            Some(MaybeRef::Ref(r)) => Some(ObjectIdentifier::from(r)),
            _ => None,
        },
        _ => None,
    };
    match single.and_then(|id| xref.get::<Dict<'_>>(id).map(|d| (d, id))) {
        Some((dict, id))
            if dict
                .get::<Object<'_>>(b"K")
                .is_some_and(|k| matches!(k, Object::Array(_))) =>
        {
            Some((dict, id))
        }
        _ => Some((root, root_id)),
    }
}

/// The children of `dict`'s /K, as written.
fn kids<'a>(dict: &Dict<'a>) -> Vec<MaybeRef<Object<'a>>> {
    match dict.get_raw::<Object<'_>>(b"K") {
        Some(MaybeRef::NotRef(Object::Array(a))) => a.raw_iter().take(MAX_ITEMS + 1).collect(),
        Some(other) => vec![other],
        None => Vec::new(),
    }
}

/// The marked content an element covers: (page, MCID) pairs, going down into its children.
fn marks(
    pdf: &Pdf,
    page_ids: &HashMap<ObjectIdentifier, usize>,
    elem: &Dict<'_>,
    page: Option<usize>,
    depth: usize,
    seen: &mut HashSet<ObjectIdentifier>,
    out: &mut Vec<(usize, i64)>,
) {
    if depth > MAX_DEPTH || out.len() > 10_000 {
        return;
    }
    let page = page_of(page_ids, elem).or(page);
    for kid in kids(elem) {
        let kid = match kid {
            MaybeRef::Ref(r) => {
                let id = ObjectIdentifier::from(r);
                if !seen.insert(id) {
                    continue;
                }
                match pdf.xref().get::<Object<'_>>(id) {
                    Some(o) => o,
                    None => continue,
                }
            }
            MaybeRef::NotRef(o) => o,
        };
        match kid {
            Object::Number(n) => {
                if let Some(p) = page {
                    out.push((p, n.as_f64() as i64));
                }
            }
            Object::Dict(d) => {
                if let Some(mcid) = d.get::<i64>(b"MCID") {
                    if let Some(p) = page_of(page_ids, &d).or(page) {
                        out.push((p, mcid));
                    }
                } else if d.contains_key(b"S") {
                    marks(pdf, page_ids, &d, page, depth + 1, seen, out);
                }
            }
            _ => {}
        }
    }
}

fn page_of(page_ids: &HashMap<ObjectIdentifier, usize>, dict: &Dict<'_>) -> Option<usize> {
    match dict.get_raw::<Dict<'_>>(b"Pg")? {
        MaybeRef::Ref(r) => page_ids.get(&ObjectIdentifier::from(r)).copied(),
        MaybeRef::NotRef(_) => None,
    }
}

/// Bytes of shown text as something readable: plain when they look like ordinary
/// single-byte text, otherwise nothing (the font's own encoding isn't known here).
fn readable(bytes: &[u8]) -> String {
    let printable = bytes
        .iter()
        .filter(|b| (0x20..0x7f).contains(*b) || **b >= 0xa0)
        .count();
    if bytes.is_empty() || printable * 10 < bytes.len() * 9 {
        return String::new();
    }
    let text: String = bytes
        .iter()
        .map(|b| {
            if (0x20..0x7f).contains(b) || *b >= 0xa0 {
                char::from(*b)
            } else {
                ' '
            }
        })
        .collect();
    let squashed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = squashed.chars().take(PREVIEW).collect();
    if squashed.chars().count() > PREVIEW {
        out.push('…');
    }
    out
}

pub(crate) fn read(pdf: &Pdf) -> ReadingOrder {
    let Some((holder, _)) = container(pdf) else {
        return ReadingOrder {
            note: Some(
                "The document has no tags, so it has no reading order. Tag it first (Accessibility check)."
                    .into(),
            ),
            ..ReadingOrder::default()
        };
    };
    let pages = pdf.pages();
    let page_ids: HashMap<ObjectIdentifier, usize> = pages
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.raw().obj_id().map(|id| (id, i)))
        .collect();
    let mut content: HashMap<usize, HashMap<i64, super::redact::Marked>> = HashMap::new();
    let all = kids(&holder);
    let mut editable = all.len() <= MAX_ITEMS;
    let mut items = Vec::new();
    for kid in all.into_iter().take(MAX_ITEMS) {
        let MaybeRef::Ref(r) = kid else {
            editable = false;
            continue;
        };
        let id = ObjectIdentifier::from(r);
        let Some(elem) = pdf.xref().get::<Dict<'_>>(id) else {
            editable = false;
            continue;
        };
        let Some(id) = obj_ref(id) else { continue };
        let role = elem
            .get::<Name<'_>>(b"S")
            .map(|n| n.as_str().to_owned())
            .unwrap_or_default();
        let mut found = Vec::new();
        marks(pdf, &page_ids, &elem, None, 0, &mut HashSet::new(), &mut found);
        let page = page_of(&page_ids, &elem).or_else(|| found.first().map(|(p, _)| *p));
        let mut text = Vec::new();
        let mut area: Option<[f64; 4]> = None;
        for (p, mcid) in found.iter().filter(|(p, _)| Some(*p) == page) {
            let marked = content.entry(*p).or_insert_with(|| {
                pages
                    .get(*p)
                    .map(|page| super::redact::marked_content(&super::rebuild::page_content(page.raw())))
                    .unwrap_or_default()
            });
            if let Some(m) = marked.get(mcid) {
                if !text.is_empty() {
                    text.push(b' ');
                }
                text.extend_from_slice(&m.text);
                if let Some([x0, y0, x1, y1]) = m.area {
                    let a = area.get_or_insert([x0, y0, x1, y1]);
                    *a = [a[0].min(x0), a[1].min(y0), a[2].max(x1), a[3].max(y1)];
                }
            }
        }
        let said = |key: &[u8]| {
            elem.get::<hayro::hayro_syntax::object::String<'_>>(key)
                .map(|s| crate::document::decode_text_string(s.as_bytes()))
                .filter(|s| !s.trim().is_empty())
        };
        let preview = said(b"ActualText")
            .or_else(|| said(b"Alt"))
            .map(|s| s.chars().take(PREVIEW).collect())
            .unwrap_or_else(|| readable(&text));
        let area = page.zip(area).and_then(|(p, [x0, y0, x1, y1])| {
            let to_view = pages.get(p)?.initial_transform(true).to_kurbo();
            let a = to_view * hayro::kurbo::Point::new(x0, y0);
            let b = to_view * hayro::kurbo::Point::new(x1, y1);
            Some(TextBox {
                x0: a.x.min(b.x) as f32,
                y0: a.y.min(b.y) as f32,
                x1: a.x.max(b.x) as f32,
                y1: a.y.max(b.y) as f32,
            })
        });
        items.push(OrderItem {
            id,
            role,
            page,
            preview,
            area,
        });
    }
    let note = (!editable).then(|| {
        "Some of this document's tags aren't objects of their own, so its reading order can be seen but not changed here."
            .to_owned()
    });
    ReadingOrder {
        items,
        editable,
        note,
    }
}

/// The document with its top-level elements in `order`, each with the role given, as an
/// incremental update. `order` must hold the same elements as the document does.
pub(crate) fn write(pdf: &Pdf, original: &[u8], order: &[(ObjRef, String)]) -> Result<Vec<u8>, SaveError> {
    let trailer = update::read_trailer(original)?;
    if trailer.encrypt.is_some() {
        return Err(SaveError::TaggingProtected);
    }
    let current = read(pdf);
    if !current.editable {
        return Err(SaveError::Damaged);
    }
    let (holder, holder_id) = container(pdf).ok_or(SaveError::Damaged)?;
    let before: HashSet<ObjRef> = current.items.iter().map(|i| i.id).collect();
    let after: HashSet<ObjRef> = order.iter().map(|(id, _)| *id).collect();
    if before != after || order.len() != current.items.len() {
        return Err(SaveError::Damaged);
    }
    let mut out = update::Update::new(trailer);
    let list: Vec<String> = order
        .iter()
        .map(|(id, _)| format!("{} {} R", id.number, id.generation))
        .collect();
    let holder_ref = obj_ref(holder_id).ok_or(SaveError::Damaged)?;
    out.put_copied(
        holder_ref.number,
        holder_ref.generation,
        rewrite(holder.data(), &vec![("K", format!("[{}]", list.join(" ")))])?,
    );
    for (id, role) in order {
        let Some(item) = current.items.iter().find(|i| i.id == *id) else {
            continue;
        };
        let role: String = role
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(32)
            .collect();
        if role.is_empty() || role == item.role {
            continue;
        }
        let elem = pdf.xref().get::<Dict<'_>>(id_of(id)).ok_or(SaveError::Damaged)?;
        out.put_copied(
            id.number,
            id.generation,
            rewrite(elem.data(), &vec![("S", format!("/{role}"))])?,
        );
    }
    Ok(out.finish(original))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_text_is_kept_and_other_bytes_are_not() {
        assert_eq!(readable(b"Hello   world"), "Hello world");
        assert_eq!(readable(&[0, 1, 2, 3, 4, 5]), "");
    }
}
