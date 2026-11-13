//! Thin lines, as Acrobat's Print Production › Fix Hairlines: lines drawn thinner than a
//! printing press can reproduce, found on each page and in the drawings it uses, and made
//! wider. Also the inks a document asks for beyond the four process inks (spot colours), for
//! the output preview.

use std::collections::HashSet;
use std::ops::Range;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier, Stream};

use super::SaveError;
use super::update;

/// Lines thinner than this, in points, are hairlines; 0 is the thinnest the device can draw.
pub const HAIRLINE: f32 = 0.25;
/// Drawings inside drawings followed at most.
const MAX_DEPTH: u8 = 12;
/// Drawings looked at at most, so a hostile file stays quick.
const MAX_VISITS: usize = 20_000;
/// Spot inks listed at most.
const MAX_INKS: usize = 64;

fn is_white(b: u8) -> bool {
    matches!(b, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// Every `w` (line width) operator's operand in `content`: where the number is, and its value.
pub(crate) fn widths(content: &[u8]) -> Vec<(Range<usize>, f64)> {
    let mut out = Vec::new();
    let mut pos = 0;
    // The number just before, if the last token was one.
    let mut last: Option<(Range<usize>, f64)> = None;
    let at = |pos: usize| content.get(pos).copied();
    while let Some(b) = at(pos) {
        if is_white(b) {
            pos += 1;
            continue;
        }
        match b {
            b'%' => {
                while at(pos).is_some_and(|b| b != b'\n' && b != b'\r') {
                    pos += 1;
                }
                last = None;
            }
            b'(' => {
                let mut depth = 0usize;
                while let Some(b) = at(pos) {
                    pos += 1;
                    match b {
                        b'\\' => pos += 1,
                        b'(' => depth += 1,
                        b')' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                last = None;
            }
            b'<' if at(pos + 1) == Some(b'<') => {
                pos += 2;
                last = None;
            }
            b'>' if at(pos + 1) == Some(b'>') => {
                pos += 2;
                last = None;
            }
            b'<' => {
                while at(pos).is_some_and(|b| b != b'>') {
                    pos += 1;
                }
                pos += 1;
                last = None;
            }
            b'/' => {
                pos += 1;
                while at(pos).is_some_and(|b| !is_white(b) && !is_delimiter(b)) {
                    pos += 1;
                }
                last = None;
            }
            _ if is_delimiter(b) => {
                pos += 1;
                last = None;
            }
            _ => {
                let start = pos;
                while at(pos).is_some_and(|b| !is_white(b) && !is_delimiter(b)) {
                    pos += 1;
                }
                let run = content.get(start..pos).unwrap_or_default();
                if run
                    .iter()
                    .all(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.'))
                {
                    last = std::str::from_utf8(run)
                        .ok()
                        .and_then(|r| r.parse::<f64>().ok())
                        .map(|v| (start..pos, v));
                    continue;
                }
                match run {
                    b"w" => {
                        if let Some(number) = last.take() {
                            out.push(number);
                        }
                    }
                    // An inline picture's data, which isn't PDF syntax, runs from after ID to EI.
                    b"ID" => {
                        pos += 1;
                        let rest = content.get(pos..).unwrap_or_default();
                        let end = rest
                            .windows(4)
                            .position(|w| matches!(w, [a, b'E', b'I', d] if is_white(*a) && is_white(*d)))
                            .map_or(rest.len(), |i| i + 3);
                        pos += end;
                    }
                    _ => {}
                }
                last = None;
                if run.is_empty() {
                    pos += 1;
                }
            }
        }
    }
    out
}

/// How many lines in `content` are thinner than `min`.
fn thin_in(content: &[u8], min: f64) -> usize {
    widths(content).iter().filter(|(_, w)| *w < min).count()
}

/// `content` with every line thinner than `min` made `min` wide, and how many were.
fn thickened(content: &[u8], min: f64) -> (Vec<u8>, usize) {
    let mut out = Vec::with_capacity(content.len() + 16);
    let mut from = 0;
    let mut count = 0;
    for (range, width) in widths(content) {
        if width >= min {
            continue;
        }
        out.extend_from_slice(content.get(from..range.start).unwrap_or_default());
        out.extend_from_slice(super::annot::n(min).as_bytes());
        from = range.end;
        count += 1;
    }
    out.extend_from_slice(content.get(from..).unwrap_or_default());
    (out, count)
}

/// The drawings (form XObjects) `res` names, by object.
fn forms<'a>(pdf: &'a Pdf, res: &Dict<'a>) -> Vec<(ObjectIdentifier, Stream<'a>)> {
    let Some(xobjects) = res.get::<Dict<'_>>(b"XObject") else {
        return Vec::new();
    };
    let xref = pdf.xref();
    xobjects
        .keys()
        .collect::<Vec<_>>()
        .into_iter()
        .filter_map(|key| {
            let id = ObjectIdentifier::from(xobjects.get_ref(key.as_ref())?);
            let stream = xref.get::<Stream<'_>>(id)?;
            stream
                .dict()
                .get::<Name<'_>>(b"Subtype")
                .is_some_and(|s| s.as_str() == "Form")
                .then_some((id, stream))
        })
        .collect()
}

/// Walks the drawings a page uses, each once, calling `visit` with each one's object and
/// stream.
struct Walk<'a> {
    pdf: &'a Pdf,
    seen: HashSet<ObjectIdentifier>,
    visits: usize,
}

impl<'a> Walk<'a> {
    fn new(pdf: &'a Pdf) -> Self {
        Self {
            pdf,
            seen: HashSet::new(),
            visits: 0,
        }
    }

    fn forms(
        &mut self,
        res: &Dict<'a>,
        depth: u8,
        visit: &mut dyn FnMut(ObjectIdentifier, &Stream<'a>) -> Result<(), SaveError>,
    ) -> Result<(), SaveError> {
        if depth > MAX_DEPTH {
            return Ok(());
        }
        for (id, stream) in forms(self.pdf, res) {
            self.visits += 1;
            if self.visits > MAX_VISITS || !self.seen.insert(id) {
                continue;
            }
            visit(id, &stream)?;
            if let Some(inner) = stream.dict().get::<Dict<'_>>(b"Resources") {
                self.forms(&inner, depth + 1, visit)?;
            }
        }
        Ok(())
    }
}

/// For each page with lines thinner than `min` points, in it or the drawings it uses (each
/// drawing counted on the first page using it), the page and how many.
pub(crate) fn thin_lines(pdf: &Pdf, min: f32) -> Vec<(usize, usize)> {
    let min = f64::from(min);
    let mut walk = Walk::new(pdf);
    let mut out = Vec::new();
    for (index, page) in pdf.pages().iter().enumerate() {
        let mut count = thin_in(&super::rebuild::page_content(page.raw()), min);
        let res = page.resources().x_objects.clone();
        let _ = walk.forms_of(&res, &mut |_, stream| {
            count += thin_in(&stream.decoded().unwrap_or_default(), min);
            Ok(())
        });
        if count > 0 {
            out.push((index, count));
        }
    }
    out
}

impl<'a> Walk<'a> {
    /// Like [`Walk::forms`], for a page's XObject dictionary itself (hayro gives a page's
    /// resources already resolved, inherited or not).
    fn forms_of(
        &mut self,
        xobjects: &Dict<'a>,
        visit: &mut dyn FnMut(ObjectIdentifier, &Stream<'a>) -> Result<(), SaveError>,
    ) -> Result<(), SaveError> {
        let xref = self.pdf.xref();
        for key in xobjects.keys().collect::<Vec<_>>() {
            let Some(id) = xobjects.get_ref(key.as_ref()).map(ObjectIdentifier::from) else {
                continue;
            };
            let Some(stream) = xref.get::<Stream<'_>>(id) else {
                continue;
            };
            if stream
                .dict()
                .get::<Name<'_>>(b"Subtype")
                .is_none_or(|s| s.as_str() != "Form")
            {
                continue;
            }
            self.visits += 1;
            if self.visits > MAX_VISITS || !self.seen.insert(id) {
                continue;
            }
            visit(id, &stream)?;
            if let Some(inner) = stream.dict().get::<Dict<'_>>(b"Resources") {
                self.forms(&inner, 1, visit)?;
            }
        }
        Ok(())
    }
}

/// A stream object's body with new (unencoded) `data`, compressed, keeping the rest of its
/// dictionary.
fn restream(dict: &[u8], data: &[u8]) -> Result<Vec<u8>, SaveError> {
    let entries = super::lex::dict_entries(dict).ok_or(SaveError::Damaged)?;
    let packed = super::pack::deflate(data);
    let mut body = b"<<".to_vec();
    for (key, value) in &entries {
        if matches!(*key, b"Filter" | b"DecodeParms" | b"Length" | b"DL") {
            continue;
        }
        body.push(b'/');
        body.extend_from_slice(key);
        body.push(b' ');
        body.extend_from_slice(dict.get(value.clone()).ok_or(SaveError::Damaged)?);
        body.push(b'\n');
    }
    body.extend_from_slice(format!("/Filter /FlateDecode /Length {} >>\nstream\n", packed.len()).as_bytes());
    body.extend_from_slice(&packed);
    body.extend_from_slice(b"\nendstream");
    Ok(body)
}

/// The document with every line thinner than `min` points made `min` wide, on its pages and
/// in the drawings they use, as an incremental update; and how many lines were changed.
pub(crate) fn fix(pdf: &Pdf, original: &[u8], min: f32) -> Result<(Vec<u8>, usize), SaveError> {
    let trailer = update::read_trailer(original)?;
    if trailer.encrypt.is_some() {
        return Err(SaveError::LinesProtected);
    }
    let wide = f64::from(min);
    let mut out = update::Update::new(trailer);
    let mut total = 0;
    let mut walk = Walk::new(pdf);
    for page in pdf.pages().iter() {
        let raw = page.raw();
        let content = super::rebuild::page_content(raw);
        let (content, count) = thickened(&content, wide);
        if count > 0 {
            let id = raw.obj_id().ok_or(SaveError::Damaged)?;
            let generation = u16::try_from(id.gen_number).map_err(|_| SaveError::Damaged)?;
            let num = out.alloc();
            out.put(num, 0, restream(b"<< >>", &content)?)?;
            let number = id.obj_number;
            out.put_copied(
                number,
                generation,
                super::rewrite(raw.data(), &vec![("Contents", format!("{num} 0 R"))])?,
            );
            total += count;
        }
        let xobjects = page.resources().x_objects.clone();
        walk.forms_of(&xobjects, &mut |id, stream| {
            let data = stream.decoded().map_err(|_| SaveError::Damaged)?;
            let (data, count) = thickened(&data, wide);
            if count == 0 {
                return Ok(());
            }
            let number = id.obj_number;
            let generation = u16::try_from(id.gen_number).map_err(|_| SaveError::Damaged)?;
            out.put(number, generation, restream(stream.dict().data(), &data)?)?;
            total += count;
            Ok(())
        })?;
    }
    if total == 0 {
        return Ok((original.to_vec(), 0));
    }
    Ok((out.finish(original), total))
}

/// The spot inks (Separation and DeviceN colour spaces) the document's pages and drawings
/// use, by name, in the order first found. The four process inks aren't listed.
pub(crate) fn spot_inks(pdf: &Pdf) -> Vec<String> {
    let mut inks: Vec<String> = Vec::new();
    let mut add = |name: &str| {
        let process = matches!(name, "Cyan" | "Magenta" | "Yellow" | "Black" | "All" | "None");
        if !process && !name.is_empty() && inks.len() < MAX_INKS && !inks.iter().any(|i| i == name) {
            inks.push(name.to_owned());
        }
    };
    fn names_in(space: &Array<'_>, add: &mut dyn FnMut(&str)) {
        let mut items = space.iter::<Object<'_>>();
        match items.next() {
            Some(Object::Name(n)) if n.as_str() == "Separation" => {
                if let Some(Object::Name(ink)) = items.next() {
                    add(ink.as_str());
                }
            }
            Some(Object::Name(n)) if n.as_str() == "DeviceN" => {
                if let Some(Object::Array(list)) = items.next() {
                    for ink in list.iter::<Name<'_>>().take(32) {
                        add(ink.as_str());
                    }
                }
            }
            // An indexed space's base may be a spot ink.
            Some(Object::Name(n)) if n.as_str() == "Indexed" => {
                if let Some(Object::Array(base)) = items.next() {
                    names_in(&base, add);
                }
            }
            _ => {}
        }
    }
    let spaces = |res: &Dict<'_>, add: &mut dyn FnMut(&str)| {
        if let Some(spaces) = res.get::<Dict<'_>>(b"ColorSpace") {
            for key in spaces.keys().collect::<Vec<_>>().into_iter().take(256) {
                if let Some(space) = spaces.get::<Array<'_>>(key.as_ref()) {
                    names_in(&space, add);
                }
            }
        }
        if let Some(xobjects) = res.get::<Dict<'_>>(b"XObject") {
            for key in xobjects.keys().collect::<Vec<_>>().into_iter().take(256) {
                if let Some(space) = xobjects
                    .get::<Stream<'_>>(key.as_ref())
                    .and_then(|s| s.dict().get::<Array<'_>>(b"ColorSpace"))
                {
                    names_in(&space, add);
                }
            }
        }
    };
    let mut walk = Walk::new(pdf);
    for page in pdf.pages().iter().take(10_000) {
        // hayro has the page's resources resolved, inherited or not.
        let res = page.resources();
        for key in res.color_spaces.keys().collect::<Vec<_>>().into_iter().take(256) {
            if let Some(space) = res.color_spaces.get::<Array<'_>>(key.as_ref()) {
                names_in(&space, &mut add);
            }
        }
        for key in res.x_objects.keys().collect::<Vec<_>>().into_iter().take(256) {
            if let Some(space) = res
                .x_objects
                .get::<Stream<'_>>(key.as_ref())
                .and_then(|s| s.dict().get::<Array<'_>>(b"ColorSpace"))
            {
                names_in(&space, &mut add);
            }
        }
        let xobjects = res.x_objects.clone();
        let _ = walk.forms_of(&xobjects, &mut |_, stream| {
            if let Some(inner) = stream.dict().get::<Dict<'_>>(b"Resources") {
                spaces(&inner, &mut add);
            }
            Ok(())
        });
    }
    inks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_widths_are_found_and_only_thin_ones_widened() {
        let content = b"q 0 w 0 0 m 10 10 l S 2 w (1 w) Tj /F 0.1 w BI /W 1 ID xx 0 w EI\n 0.1 w Q";
        let found: Vec<f64> = widths(content).into_iter().map(|(_, w)| w).collect();
        assert_eq!(found, vec![0.0, 2.0, 0.1, 0.1]);
        let (fixed, count) = thickened(content, 0.25);
        assert_eq!(count, 3);
        assert_eq!(
            fixed,
            b"q 0.25 w 0 0 m 10 10 l S 2 w (1 w) Tj /F 0.25 w BI /W 1 ID xx 0 w EI\n 0.25 w Q".to_vec()
        );
    }
}
