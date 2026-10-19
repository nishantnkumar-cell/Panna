//! Reads the document outline (bookmarks) into a flat, depth-annotated list.
//!
//! Outlines are linked lists of dictionaries, so a hostile file can build cycles or absurdly deep
//! trees. Every walk here is bounded by item count, depth and a visited set.

use std::collections::{HashMap, HashSet};

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, MaybeRef, Object, ObjectIdentifier};

use crate::document::decode_text_string;

/// One bookmark. `depth` 0 is a top-level entry.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutlineItem {
    pub title: String,
    pub depth: u16,
    /// Zero-based page the bookmark jumps to, when it points inside this document.
    pub page: Option<usize>,
}

const MAX_ITEMS: usize = 20_000;
const MAX_DEPTH: u16 = 48;
/// Bound on name-tree nodes visited per lookup.
const MAX_NAME_NODES: usize = 4_096;

pub(crate) struct Ctx<'a> {
    catalog: Dict<'a>,
    page_ids: HashMap<ObjectIdentifier, usize>,
    page_count: usize,
    seen: HashSet<ObjectIdentifier>,
    out: Vec<OutlineItem>,
}

impl<'a> Ctx<'a> {
    /// What resolving destinations needs: the catalog and where each page object sits.
    pub(crate) fn new(pdf: &'a Pdf) -> Option<Self> {
        let xref = pdf.xref();
        let catalog = xref.get::<Dict<'_>>(xref.root_id())?;
        let pages = pdf.pages();
        let page_ids = pages
            .iter()
            .enumerate()
            .filter_map(|(i, page)| page.raw().obj_id().map(|id| (id, i)))
            .collect();
        Some(Ctx {
            catalog,
            page_ids,
            page_count: pages.len(),
            seen: HashSet::new(),
            out: Vec::new(),
        })
    }
}

pub(crate) fn read(pdf: &Pdf) -> Vec<OutlineItem> {
    let Some(mut ctx) = Ctx::new(pdf) else {
        return Vec::new();
    };
    let Some(root) = ctx.catalog.get::<Dict<'_>>(b"Outlines") else {
        return Vec::new();
    };
    walk(&mut ctx, root.get::<Dict<'_>>(b"First"), 0);
    ctx.out
}

/// Every bookmark that is an object of its own, with its title as written, for redacting.
pub(crate) fn titled(pdf: &Pdf) -> Vec<(ObjectIdentifier, String)> {
    let mut out = Vec::new();
    let xref = pdf.xref();
    let Some(root) = xref
        .get::<Dict<'_>>(xref.root_id())
        .and_then(|c| c.get::<Dict<'_>>(b"Outlines"))
    else {
        return out;
    };
    let mut seen = HashSet::new();
    let mut lists = vec![(root.get::<Dict<'_>>(b"First"), 0u16)];
    while let Some((mut node, depth)) = lists.pop() {
        while let Some(item) = node {
            if out.len() >= MAX_ITEMS || depth > MAX_DEPTH {
                break;
            }
            let Some(id) = item.obj_id().filter(|id| seen.insert(*id)) else {
                break;
            };
            let title = item
                .get::<hayro::hayro_syntax::object::String<'_>>(b"Title")
                .map(|s| decode_text_string(s.as_bytes()))
                .unwrap_or_default();
            out.push((id, title));
            lists.push((item.get::<Dict<'_>>(b"First"), depth + 1));
            node = item.get::<Dict<'_>>(b"Next");
        }
    }
    out
}

fn walk<'a>(ctx: &mut Ctx<'a>, first: Option<Dict<'a>>, depth: u16) {
    if depth > MAX_DEPTH {
        return;
    }
    let mut node = first;
    while let Some(item) = node {
        if ctx.out.len() >= MAX_ITEMS {
            return;
        }
        // A sibling or child pointing back at an earlier item would loop forever.
        if let Some(id) = item.obj_id()
            && !ctx.seen.insert(id)
        {
            return;
        }
        let title = item
            .get::<hayro::hayro_syntax::object::String<'_>>(b"Title")
            .map(|s| clean_title(&decode_text_string(s.as_bytes())))
            .unwrap_or_default();
        let page = destination(ctx, &item);
        ctx.out.push(OutlineItem { title, depth, page });
        walk(ctx, item.get::<Dict<'_>>(b"First"), depth + 1);
        node = item.get::<Dict<'_>>(b"Next");
    }
}

/// Collapses whitespace and control characters so a title fits on one line.
fn clean_title(s: &str) -> String {
    s.split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Page index for an outline item's or link's `/Dest` or `/A << /S /GoTo /D … >>`.
pub(crate) fn destination(ctx: &Ctx<'_>, item: &Dict<'_>) -> Option<usize> {
    let dest = match item.get::<Object<'_>>(b"Dest") {
        Some(dest) => dest,
        None => {
            let action = item.get::<Dict<'_>>(b"A")?;
            let kind = action.get::<hayro::hayro_syntax::object::Name<'_>>(b"S")?;
            if kind.as_str() != "GoTo" {
                return None;
            }
            action.get::<Object<'_>>(b"D")?
        }
    };
    resolve_dest(ctx, dest, 0)
}

fn resolve_dest(ctx: &Ctx<'_>, dest: Object<'_>, hops: u8) -> Option<usize> {
    if hops > 2 {
        return None;
    }
    match dest {
        Object::Array(array) => page_of(ctx, &array),
        // A named destination's value can be an array or a dictionary with /D.
        Object::Dict(dict) => resolve_dest(ctx, dict.get::<Object<'_>>(b"D")?, hops + 1),
        Object::Name(name) => resolve_dest(ctx, named(ctx, name.as_str().as_bytes())?, hops + 1),
        Object::String(s) => resolve_dest(ctx, named(ctx, s.as_bytes())?, hops + 1),
        _ => None,
    }
}

/// The first element of an explicit destination is a page reference (or, in some files, a
/// page number).
fn page_of(ctx: &Ctx<'_>, array: &Array<'_>) -> Option<usize> {
    match array.raw_iter().next()? {
        MaybeRef::Ref(r) => ctx.page_ids.get(&r.into()).copied(),
        MaybeRef::NotRef(Object::Number(n)) => {
            let n = n.as_f64();
            (n >= 0.0 && n < ctx.page_count as f64).then_some(n as usize)
        }
        MaybeRef::NotRef(_) => None,
    }
}

/// Looks up a named destination in the PDF 1.1 `/Dests` dictionary or the `/Names /Dests` tree.
fn named<'a>(ctx: &Ctx<'a>, key: &[u8]) -> Option<Object<'a>> {
    if let Some(dests) = ctx.catalog.get::<Dict<'a>>(b"Dests") {
        let found = dests.entries().find(|(name, _)| name.as_str().as_bytes() == key);
        if found.is_some() {
            return std::str::from_utf8(key)
                .ok()
                .and_then(|k| dests.get::<Object<'a>>(k));
        }
    }
    let tree = ctx.catalog.get::<Dict<'a>>(b"Names")?.get::<Dict<'a>>(b"Dests")?;
    let mut budget = MAX_NAME_NODES;
    let mut visited = HashSet::new();
    search_name_tree(tree, key, 0, &mut budget, &mut visited)
}

fn search_name_tree<'a>(
    node: Dict<'a>,
    key: &[u8],
    depth: u16,
    budget: &mut usize,
    visited: &mut HashSet<ObjectIdentifier>,
) -> Option<Object<'a>> {
    if depth > MAX_DEPTH || *budget == 0 {
        return None;
    }
    *budget -= 1;
    if let Some(id) = node.obj_id()
        && !visited.insert(id)
    {
        return None;
    }
    if let Some(names) = node.get::<Array<'a>>(b"Names") {
        let mut entries = names.iter::<Object<'a>>();
        while let (Some(name), Some(value)) = (entries.next(), entries.next()) {
            if let Object::String(name) = name
                && name.as_bytes() == key
            {
                return Some(value);
            }
        }
    }
    let kids = node.get::<Array<'a>>(b"Kids")?;
    for kid in kids.iter::<Dict<'a>>() {
        if let Some(found) = search_name_tree(kid, key, depth + 1, budget, visited) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_single_line() {
        assert_eq!(clean_title("  Chapter\r\n 1\t\u{7}Intro "), "Chapter 1 Intro");
    }
}
