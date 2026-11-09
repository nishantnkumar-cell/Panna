//! Replacing the document's bookmarks, as an incremental update.
//!
//! Bookmarks that were already in the file keep their object numbers, so what's copied from
//! them (where they go, their colour and style) stays readable in an encrypted file; only
//! their title and their place in the tree are written anew.

use std::collections::{HashMap, HashSet};

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Dict, ObjectIdentifier};

use super::{SaveError, annot, lex, update};
use crate::OutlineItem;

/// Entries that place an item in the tree; everything else on an existing item is kept.
const TREE_KEYS: &[&[u8]] = &[b"Title", b"Parent", b"Prev", b"Next", b"First", b"Last", b"Count"];
/// Entries that say where an item goes, dropped when it's pointed at another page.
const TARGET_KEYS: &[&[u8]] = &[b"Dest", b"A", b"SE"];

/// Where an item sits in the tree, by index into the list.
#[derive(Clone, Default)]
struct Node {
    parent: Option<usize>,
    prev: Option<usize>,
    next: Option<usize>,
    first: Option<usize>,
    last: Option<usize>,
    /// How many items are under it, at every level.
    descendants: usize,
}

fn ident(num: u32, generation: u16) -> ObjectIdentifier {
    ObjectIdentifier::new(i32::try_from(num).unwrap_or(-1), i32::from(generation))
}

fn id_of(id: ObjectIdentifier) -> Result<(u32, u16), SaveError> {
    Ok((
        u32::try_from(id.obj_number).map_err(|_| SaveError::Damaged)?,
        u16::try_from(id.gen_number).map_err(|_| SaveError::Damaged)?,
    ))
}

/// `original` with its bookmarks replaced by `items`, in reading order (each item's depth at
/// most one more than the one before it).
pub(crate) fn with_outline(
    pdf: &Pdf,
    original: &[u8],
    password: &str,
    items: &[OutlineItem],
) -> Result<Vec<u8>, SaveError> {
    let trailer = update::read_trailer(original)?;
    let crypt = match &trailer.encrypt {
        Some(raw) => Some(super::encryption(pdf, raw, trailer.id.as_deref(), password)?),
        None => None,
    };
    let mut out = update::Update::new(trailer);
    if let Some(crypt) = crypt {
        out.encrypt_with(crypt);
    }
    let xref = pdf.xref();
    let catalog = xref.get::<Dict<'_>>(xref.root_id()).ok_or(SaveError::Damaged)?;
    let pages: Vec<String> = pdf
        .pages()
        .iter()
        .map(|page| {
            let (num, generation) = id_of(page.raw().obj_id().ok_or(SaveError::Damaged)?)?;
            Ok(format!("{num} {generation} R"))
        })
        .collect::<Result<_, SaveError>>()?;

    // Where each item was going before, so an item whose page is unchanged keeps its exact
    // destination (a zoom, a named destination).
    let before = crate::outline::read(pdf);

    let before: HashMap<crate::ObjRef, Option<usize>> =
        before.into_iter().filter_map(|b| Some((b.id?, b.page))).collect();

    // The tree, with depths made consistent: each item one level at most below the one
    // before it.
    let mut nodes = vec![Node::default(); items.len()];
    let mut top = Node::default();
    let mut ancestors: Vec<usize> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        ancestors.truncate(usize::from(item.depth));
        let parent = ancestors.last().copied();
        for &a in &ancestors {
            if let Some(node) = nodes.get_mut(a) {
                node.descendants += 1;
            }
        }
        let holder = match parent {
            Some(p) => nodes.get_mut(p).ok_or(SaveError::Damaged)?,
            None => &mut top,
        };
        let prev = holder.last.replace(i);
        holder.first.get_or_insert(i);
        if let Some(prev) = prev
            && let Some(node) = nodes.get_mut(prev)
        {
            node.next = Some(i);
        }
        if let Some(node) = nodes.get_mut(i) {
            node.parent = parent;
            node.prev = prev;
        }
        ancestors.push(i);
    }

    // Object numbers: existing items keep theirs (each once), new ones get new numbers.
    let mut seen = HashSet::new();
    let mut ids: Vec<(u32, u16, bool)> = Vec::with_capacity(items.len());
    for item in items {
        match item.id.filter(|id| seen.insert(*id)) {
            Some(id) if xref.get::<Dict<'_>>(ident(id.number, id.generation)).is_some() => {
                ids.push((id.number, id.generation, true));
            }
            _ => ids.push((out.alloc(), 0, false)),
        }
    }
    let existing_root = catalog.get_ref(b"Outlines").map(ObjectIdentifier::from);
    let root = match existing_root {
        Some(id) => id_of(id)?,
        None if items.is_empty() => return Ok(original.to_vec()),
        None => (out.alloc(), 0),
    };
    let reference = |(num, generation): (u32, u16)| format!("{num} {generation} R");
    let item_ref = |i: usize| {
        ids.get(i)
            .map(|&(num, generation, _)| reference((num, generation)))
            .ok_or(SaveError::Damaged)
    };
    let dest = |page: usize| -> Result<String, SaveError> {
        let page = pages.get(page).ok_or(SaveError::NoSuchPage(page))?;
        Ok(format!(" /Dest [{page} /XYZ null null null]"))
    };

    for ((item, node), &(num, generation, kept)) in items.iter().zip(&nodes).zip(&ids) {
        let mut body = String::from("<<");
        let title = annot::text_string(&item.title);
        let title = if kept {
            out.seal_value(num, generation, title)?
        } else {
            title
        };
        body.push_str(&format!("/Title {title}"));
        let parent = match node.parent {
            Some(p) => item_ref(p)?,
            None => reference(root),
        };
        body.push_str(&format!(" /Parent {parent}"));
        if let Some(prev) = node.prev {
            body.push_str(&format!(" /Prev {}", item_ref(prev)?));
        }
        if let Some(next) = node.next {
            body.push_str(&format!(" /Next {}", item_ref(next)?));
        }
        let mut closed = false;
        if kept {
            let dict = xref
                .get::<Dict<'_>>(ident(num, generation))
                .ok_or(SaveError::Damaged)?;
            closed = dict.get::<i32>(b"Count").is_some_and(|c| c < 0);
            let same_page = item.id.and_then(|id| before.get(&id)) == Some(&item.page);
            let data = dict.data();
            for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
                if TREE_KEYS.contains(&key) || (!same_page && TARGET_KEYS.contains(&key)) {
                    continue;
                }
                body.push_str(&format!(" /{} ", String::from_utf8_lossy(key)));
                body.push_str(&String::from_utf8_lossy(
                    data.get(value).ok_or(SaveError::Damaged)?,
                ));
            }
            if !same_page && let Some(page) = item.page {
                body.push_str(&dest(page)?);
            }
        } else if let Some(page) = item.page {
            body.push_str(&dest(page)?);
        }
        if let (Some(first), Some(last)) = (node.first, node.last) {
            let count = i64::try_from(node.descendants).unwrap_or(i64::MAX);
            body.push_str(&format!(
                " /First {} /Last {} /Count {}",
                item_ref(first)?,
                item_ref(last)?,
                if closed { -count } else { count }
            ));
        }
        body.push_str(" >>");
        if kept {
            out.put_copied(num, generation, body.into_bytes());
        } else {
            out.put(num, generation, body.into_bytes())?;
        }
    }

    let mut root_body = String::from("<< /Type /Outlines");
    if let (Some(first), Some(last)) = (top.first, top.last) {
        root_body.push_str(&format!(
            " /First {} /Last {} /Count {}",
            item_ref(first)?,
            item_ref(last)?,
            items.len()
        ));
    } else {
        root_body.push_str(" /Count 0");
    }
    root_body.push_str(" >>");
    out.put_copied(root.0, root.1, root_body.into_bytes());

    if existing_root.is_none() {
        // A new outline is pointed to from the catalog, every other entry copied as it is.
        let (num, generation) = id_of(xref.root_id())?;
        let data = catalog.data();
        let mut dict = b"<<".to_vec();
        for (key, value) in lex::dict_entries(data).ok_or(SaveError::Damaged)? {
            if key == b"Outlines" {
                continue;
            }
            dict.push(b'/');
            dict.extend_from_slice(key);
            dict.push(b' ');
            dict.extend_from_slice(data.get(value).ok_or(SaveError::Damaged)?);
            dict.push(b'\n');
        }
        dict.extend_from_slice(format!("/Outlines {}\n>>", reference(root)).as_bytes());
        out.put_copied(num, generation, dict);
    }
    Ok(out.finish(original))
}
