//! Layers (optional content groups): parts of pages that can be shown or hidden, such as a
//! drawing's annotations or a map's labels. Showing and hiding them changes only the view.

use std::collections::HashSet;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Name, ObjectIdentifier};

use crate::ObjRef;
use crate::document::decode_text_string;

const MAX_LAYERS: usize = 2_000;

/// A layer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Layer {
    pub id: ObjRef,
    pub name: String,
    /// Whether the document shows it when opened.
    pub shown: bool,
}

/// The references in `array`, in order.
fn refs<'a>(array: &'a Array<'a>) -> impl Iterator<Item = (ObjectIdentifier, ObjRef)> + 'a {
    array
        .raw_iter()
        .filter_map(|item| item.as_obj_ref())
        .take(MAX_LAYERS)
        .filter_map(|r| {
            let oid: ObjectIdentifier = r.into();
            Some((oid, ObjRef::new(oid.obj_number, oid.gen_number)?))
        })
}

/// The document's layers, in the order the document lists them.
pub(crate) fn read(pdf: &Pdf) -> Vec<Layer> {
    let xref = pdf.xref();
    let Some(props) = xref
        .get::<Dict<'_>>(xref.root_id())
        .and_then(|catalog| catalog.get::<Dict<'_>>(b"OCProperties"))
    else {
        return Vec::new();
    };
    let Some(groups) = props.get::<Array<'_>>(b"OCGs") else {
        return Vec::new();
    };
    let config = props.get::<Dict<'_>>(b"D");
    let list = |key: &[u8]| -> HashSet<ObjRef> {
        config
            .as_ref()
            .and_then(|d| d.get::<Array<'_>>(key))
            .map(|a| refs(&a).map(|(_, id)| id).collect())
            .unwrap_or_default()
    };
    let base_off = config
        .as_ref()
        .and_then(|d| d.get::<Name<'_>>(b"BaseState"))
        .is_some_and(|n| n.as_str() == "OFF");
    let (on, off) = (list(b"ON"), list(b"OFF"));
    let mut seen = HashSet::new();
    refs(&groups)
        .filter(|(_, id)| seen.insert(*id))
        .map(|(oid, id)| {
            let name = xref
                .get::<Dict<'_>>(oid)
                .and_then(|d| d.get::<hayro::hayro_syntax::object::String<'_>>(b"Name"))
                .map(|s| decode_text_string(s.as_bytes()))
                .unwrap_or_default();
            let shown = if base_off {
                on.contains(&id)
            } else {
                !off.contains(&id)
            };
            Layer {
                id,
                name: if name.trim().is_empty() {
                    "Unnamed layer".into()
                } else {
                    name
                },
                shown,
            }
        })
        .collect()
}
