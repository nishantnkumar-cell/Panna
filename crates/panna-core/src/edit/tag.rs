//! Tagging an untagged document for screen readers, as an incremental update: each run of
//! text on each page becomes a paragraph or, where the type is larger, a heading, in the
//! order it's drawn. Pictures become figures, for descriptions to be given to them (see
//! [`describe`]); drawings are left untagged.

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Dict, Name};

use super::{SaveError, lex, rewrite, update};

/// Elements made at most, so a huge document stays a reasonable size.
const MAX_ELEMENTS: usize = 500_000;

pub(crate) fn tag(pdf: &Pdf, original: &[u8]) -> Result<Vec<u8>, SaveError> {
    let trailer = update::read_trailer(original)?;
    if trailer.encrypt.is_some() {
        return Err(SaveError::TaggingProtected);
    }
    let root = lex::reference(trailer.root.trim_ascii()).ok_or(SaveError::Damaged)?;
    let xref = pdf.xref();
    let id = |n: u32, g: u16| {
        Ok::<_, SaveError>(hayro::hayro_syntax::object::ObjectIdentifier::new(
            i32::try_from(n).map_err(|_| SaveError::Damaged)?,
            i32::from(g),
        ))
    };
    let catalog = xref
        .get::<Dict<'_>>(id(root.number, root.generation)?)
        .ok_or(SaveError::Damaged)?;
    if catalog.contains_key(b"StructTreeRoot") {
        return Err(SaveError::AlreadyTagged);
    }

    // Each page's content and the text runs in it; pages already marked up are left alone.
    let pages = pdf.pages();
    let mut found = Vec::new();
    let mut total = 0;
    for page in pages.iter() {
        let raw = page.raw();
        let content = super::rebuild::page_content(raw);
        let res = page.resources();
        // Drawn XObjects are kept only when they're pictures.
        let blocks: Vec<_> = super::redact::text_blocks(&content)
            .unwrap_or_default()
            .into_iter()
            .filter(|b| {
                b.drawn.as_ref().is_none_or(|name| {
                    name.is_empty()
                        || Name::new(name)
                            .and_then(|n| res.get_x_object(&n))
                            .is_some_and(|x| {
                                x.dict()
                                    .get::<Name<'_>>(b"Subtype")
                                    .is_some_and(|s| s.as_ref() == b"Image")
                            })
                })
            })
            .collect();
        total += blocks.len();
        if total > MAX_ELEMENTS {
            break;
        }
        found.push((raw, content, blocks));
    }
    if found.iter().all(|(_, _, b)| b.iter().all(|b| b.drawn.is_some())) {
        return Err(SaveError::NothingToTag);
    }

    // The body's size is the one most text is in; larger runs are headings.
    let mut sizes: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    for (_, _, blocks) in &found {
        for b in blocks.iter().filter(|b| b.drawn.is_none()) {
            *sizes.entry((b.size * 2.0).round() as i64).or_default() += b.shown.max(1);
        }
    }
    let body = sizes
        .iter()
        .max_by_key(|(size, count)| (**count, std::cmp::Reverse(**size)))
        .map_or(10.0, |(size, _)| *size as f64 / 2.0);
    let role = |size: f64| {
        if size >= body * 1.6 {
            "H1"
        } else if size >= body * 1.2 {
            "H2"
        } else {
            "P"
        }
    };

    let mut out = update::Update::new(trailer);
    let tree = out.alloc();
    let document = out.alloc();
    let parent_tree = out.alloc();
    let mut kids = Vec::new();
    let mut nums = String::new();
    let mut key = 0usize;
    for (raw, content, blocks) in &found {
        if blocks.is_empty() {
            continue;
        }
        let page_id = raw.obj_id().ok_or(SaveError::Damaged)?;
        let (page_num, page_gen) = (
            u32::try_from(page_id.obj_number).map_err(|_| SaveError::Damaged)?,
            u16::try_from(page_id.gen_number).map_err(|_| SaveError::Damaged)?,
        );
        let mut marked = Vec::with_capacity(content.len() + blocks.len() * 24);
        let mut at = 0;
        let mut elements = Vec::new();
        for (mcid, block) in blocks.iter().enumerate() {
            let s = if block.drawn.is_some() {
                "Figure"
            } else {
                role(block.size)
            };
            marked.extend_from_slice(content.get(at..block.range.start).ok_or(SaveError::Damaged)?);
            marked.extend_from_slice(format!("\n/{s} <</MCID {mcid}>> BDC\n").as_bytes());
            marked.extend_from_slice(content.get(block.range.clone()).ok_or(SaveError::Damaged)?);
            marked.extend_from_slice(b"\nEMC\n");
            at = block.range.end;
            let element = out.alloc();
            out.put(
                element,
                0,
                format!(
                    "<< /Type /StructElem /S /{s} /P {document} 0 R /Pg {page_num} {page_gen} R /K {mcid} >>"
                )
                .into_bytes(),
            )?;
            elements.push(format!("{element} 0 R"));
        }
        marked.extend_from_slice(content.get(at..).unwrap_or_default());
        let stream = out.alloc();
        let packed = super::pack::deflate(&marked);
        let mut body = format!("<< /Filter /FlateDecode /Length {} >>\nstream\n", packed.len()).into_bytes();
        body.extend_from_slice(&packed);
        body.extend_from_slice(b"\nendstream");
        out.put(stream, 0, body)?;
        out.put_copied(
            page_num,
            page_gen,
            rewrite(
                raw.data(),
                &vec![
                    ("Contents", format!("{stream} 0 R")),
                    ("StructParents", key.to_string()),
                    ("Tabs", "/S".to_owned()),
                ],
            )?,
        );
        nums.push_str(&format!("{key} [{}]\n", elements.join(" ")));
        kids.extend(elements);
        key += 1;
    }
    out.put(
        document,
        0,
        format!(
            "<< /Type /StructElem /S /Document /P {tree} 0 R /K [{}] >>",
            kids.join(" ")
        )
        .into_bytes(),
    )?;
    out.put(parent_tree, 0, format!("<< /Nums [{nums}] >>").into_bytes())?;
    out.put(
        tree,
        0,
        format!(
            "<< /Type /StructTreeRoot /K {document} 0 R /ParentTree {parent_tree} 0 R \
             /ParentTreeNextKey {key} >>"
        )
        .into_bytes(),
    )?;
    out.put_copied(
        root.number,
        root.generation,
        rewrite(
            catalog.data(),
            &vec![
                ("MarkInfo", "<< /Marked true >>".to_owned()),
                ("StructTreeRoot", format!("{tree} 0 R")),
            ],
        )?,
    );
    Ok(out.finish(original))
}

/// The document with descriptions (alternate text) given to figures, by their structure
/// elements' object numbers, as an incremental update. An empty description takes one
/// away.
pub(crate) fn describe(
    pdf: &Pdf,
    original: &[u8],
    alts: &[((u32, u16), String)],
) -> Result<Vec<u8>, SaveError> {
    let trailer = update::read_trailer(original)?;
    if trailer.encrypt.is_some() {
        return Err(SaveError::TaggingProtected);
    }
    let mut out = update::Update::new(trailer);
    for ((number, generation), alt) in alts {
        let id = hayro::hayro_syntax::object::ObjectIdentifier::new(
            i32::try_from(*number).map_err(|_| SaveError::Damaged)?,
            i32::from(*generation),
        );
        let elem = pdf.xref().get::<Dict<'_>>(id).ok_or(SaveError::Damaged)?;
        // Only structure elements: anything else named here is left as it is.
        if !elem.contains_key(b"S") || !elem.contains_key(b"P") {
            continue;
        }
        let alt: String = alt.chars().filter(|c| !c.is_control()).take(2_000).collect();
        let value = if alt.trim().is_empty() {
            "null".to_owned()
        } else {
            super::annot::text_string(alt.trim())
        };
        out.put_copied(*number, *generation, rewrite(elem.data(), &vec![("Alt", value)])?);
    }
    Ok(out.finish(original))
}
