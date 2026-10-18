//! Incremental updates: new and replaced objects appended after the original bytes, with a
//! cross-reference section that points back to the previous one (ISO 32000-2, 7.5.6).
//!
//! The original file is never rewritten, so anything this code doesn't understand survives
//! untouched, and earlier revisions (including signed ones) stay intact.

use std::fmt::Write as _;

use super::SaveError;
use super::crypt::Crypt;
use super::lex::{Lexer, dict_entries};

/// How far from the end of the file `startxref` is searched for.
const TAIL: usize = 4096;

/// What the update needs from the file's last trailer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Trailer {
    /// Offset of the last cross-reference section, for `/Prev`.
    pub prev: u64,
    pub size: u32,
    /// Raw `/Root`, `/Info` and `/ID` values, copied as they are.
    pub root: Vec<u8>,
    pub info: Option<Vec<u8>>,
    pub id: Option<Vec<u8>>,
    /// Raw `/Encrypt` value: the update keeps the file encrypted the same way.
    pub encrypt: Option<Vec<u8>>,
    /// The last section is a cross-reference stream, so the update writes one too.
    pub xref_stream: bool,
}

pub(crate) fn read_trailer(data: &[u8]) -> Result<Trailer, SaveError> {
    let tail_start = data.len().saturating_sub(TAIL);
    let tail = data.get(tail_start..).unwrap_or_default();
    let at = find_last(tail, b"startxref").ok_or(SaveError::Damaged)? + tail_start + 9;
    let prev = Lexer::new(data, at).uint().ok_or(SaveError::Damaged)?;
    let prev_at = usize::try_from(prev).map_err(|_| SaveError::Damaged)?;
    if prev_at >= data.len() {
        return Err(SaveError::Damaged);
    }
    let mut lex = Lexer::new(data, prev_at);
    let (dict_at, xref_stream) = if lex.eat(b"xref") {
        let rest = data.get(lex.pos..).unwrap_or_default();
        let t = find_first(rest, b"trailer").ok_or(SaveError::Damaged)?;
        (lex.pos + t + b"trailer".len(), false)
    } else {
        lex.uint().ok_or(SaveError::Damaged)?;
        lex.uint().ok_or(SaveError::Damaged)?;
        if !lex.eat(b"obj") {
            return Err(SaveError::Damaged);
        }
        (lex.pos, true)
    };
    let dict = data.get(dict_at..).unwrap_or_default();
    let entries = dict_entries(dict).ok_or(SaveError::Damaged)?;
    let raw = |key: &[u8]| {
        entries
            .iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| dict.get(v.clone()))
            .map(<[u8]>::to_vec)
    };
    let size = raw(b"Size")
        .and_then(|v| Lexer::new(&v, 0).uint())
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v > 0 && *v < 8_388_607)
        .ok_or(SaveError::Damaged)?;
    Ok(Trailer {
        prev,
        size,
        root: raw(b"Root").ok_or(SaveError::Damaged)?,
        info: raw(b"Info"),
        id: raw(b"ID"),
        encrypt: raw(b"Encrypt"),
        xref_stream,
    })
}

fn find_last(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

fn find_first(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// An incremental update being assembled.
pub(crate) struct Update {
    trailer: Trailer,
    next: u32,
    objects: Vec<(u32, u16, Vec<u8>)>,
    /// Set for encrypted files: new strings and streams are encrypted with it.
    crypt: Option<Crypt>,
}

impl Update {
    pub(crate) fn new(trailer: Trailer) -> Self {
        Self {
            next: trailer.size,
            trailer,
            objects: Vec::new(),
            crypt: None,
        }
    }

    pub(crate) fn encrypt_with(&mut self, crypt: Crypt) {
        self.crypt = Some(crypt);
    }

    /// Reserves an object number for a new object.
    pub(crate) fn alloc(&mut self) -> u32 {
        let id = self.next;
        self.next += 1;
        id
    }

    /// Adds a new object (or replaces one with all-new content). `body` is what goes between
    /// `obj` and `endobj`, unencrypted.
    pub(crate) fn put(&mut self, id: u32, generation: u16, body: Vec<u8>) -> Result<(), SaveError> {
        let body = match &self.crypt {
            Some(crypt) => crypt.seal(id, generation, &body)?,
            None => body,
        };
        self.put_copied(id, generation, body);
        Ok(())
    }

    /// Replaces an object with a body built from its own (already encrypted) values plus new
    /// values passed through [`Update::seal_value`].
    pub(crate) fn put_copied(&mut self, id: u32, generation: u16, body: Vec<u8>) {
        self.objects.retain(|(i, _, _)| *i != id);
        self.objects.push((id, generation, body));
    }

    /// A new value for an entry of object `id`, with any strings in it encrypted.
    pub(crate) fn seal_value(&self, id: u32, generation: u16, value: String) -> Result<String, SaveError> {
        match &self.crypt {
            Some(crypt) => String::from_utf8(crypt.strings_in(id, generation, value.as_bytes())?)
                .map_err(|_| SaveError::Damaged),
            None => Ok(value),
        }
    }

    /// The original bytes followed by the update.
    pub(crate) fn finish(mut self, original: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(original.len() + 4096);
        out.extend_from_slice(original);
        if !matches!(out.last(), Some(b'\n' | b'\r')) {
            out.push(b'\n');
        }
        let mut offsets: Vec<(u32, u16, usize)> = Vec::new();
        for (id, generation, body) in &self.objects {
            offsets.push((*id, *generation, out.len()));
            out.extend_from_slice(format!("{id} {generation} obj\n").as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref_at = out.len();
        let mut trailer = String::new();
        if self.trailer.xref_stream {
            let own = self.alloc();
            offsets.push((own, 0, xref_at));
            offsets.sort_unstable_by_key(|o| o.0);
            let mut rows = Vec::with_capacity(offsets.len() * 7);
            let mut index = String::new();
            for (id, generation, offset) in &offsets {
                rows.push(1u8);
                rows.extend_from_slice(&(*offset as u32).to_be_bytes());
                rows.extend_from_slice(&generation.to_be_bytes());
                let _ = write!(index, "{id} 1 ");
            }
            let _ = write!(
                trailer,
                "{own} 0 obj\n<< /Type /XRef /Size {} /W [1 4 2] /Index [{}] /Length {} ",
                self.next,
                index.trim_end(),
                rows.len()
            );
            self.trailer_entries(&mut trailer);
            trailer.push_str(">>\nstream\n");
            out.extend_from_slice(trailer.as_bytes());
            out.extend_from_slice(&rows);
            out.extend_from_slice(b"\nendstream\nendobj\n");
        } else {
            offsets.sort_unstable_by_key(|o| o.0);
            trailer.push_str("xref\n");
            // One subsection per run of consecutive object numbers.
            let mut i = 0;
            while let Some(&(first, _, _)) = offsets.get(i) {
                let run = offsets
                    .get(i..)
                    .unwrap_or_default()
                    .iter()
                    .enumerate()
                    .take_while(|(k, o)| o.0 as usize == first as usize + k)
                    .count();
                let _ = writeln!(trailer, "{first} {run}");
                for (_, generation, offset) in offsets.get(i..i + run).unwrap_or_default() {
                    let _ = write!(trailer, "{offset:010} {generation:05} n\r\n");
                }
                i += run;
            }
            let _ = write!(trailer, "trailer\n<< /Size {} ", self.next);
            self.trailer_entries(&mut trailer);
            trailer.push_str(">>\n");
            out.extend_from_slice(trailer.as_bytes());
        }
        out.extend_from_slice(format!("startxref\n{xref_at}\n%%EOF\n").as_bytes());
        out
    }

    fn trailer_entries(&self, out: &mut String) {
        let t = &self.trailer;
        let _ = write!(out, "/Root {} ", String::from_utf8_lossy(&t.root));
        if let Some(info) = &t.info {
            let _ = write!(out, "/Info {} ", String::from_utf8_lossy(info));
        }
        if let Some(id) = &t.id {
            let _ = write!(out, "/ID {} ", String::from_utf8_lossy(id));
        }
        if let Some(encrypt) = &t.encrypt {
            let _ = write!(out, "/Encrypt {} ", String::from_utf8_lossy(encrypt));
        }
        let _ = write!(out, "/Prev {} ", t.prev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLASSIC: &[u8] = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\nxref\n0 2\n\
        0000000000 65535 f \n0000000009 00000 n \ntrailer\n<< /Size 2 /Root 1 0 R /ID [<AB> <CD>] >>\n\
        startxref\n45\n%%EOF\n";

    #[test]
    fn reads_a_classic_trailer() {
        let t = read_trailer(CLASSIC).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(t.prev, 45);
        assert_eq!(t.size, 2);
        assert_eq!(t.root, b"1 0 R");
        assert_eq!(t.id.as_deref(), Some(&b"[<AB> <CD>]"[..]));
        assert!(!t.xref_stream && t.encrypt.is_none() && t.info.is_none());
    }

    #[test]
    fn writes_a_table_with_subsections_and_prev() {
        let mut u = Update::new(read_trailer(CLASSIC).unwrap_or_else(|e| panic!("{e:?}")));
        // Object 2 is reserved but never written, so the table needs two subsections.
        let _unused = u.alloc();
        let b = u.alloc();
        u.put(b, 0, b"(b)".to_vec()).unwrap();
        u.put(1, 0, b"<< /Type /Catalog /New true >>".to_vec()).unwrap();
        let out = u.finish(CLASSIC);
        let text = String::from_utf8_lossy(&out);
        let tail = text.get(CLASSIC.len()..).unwrap_or_default();
        assert!(tail.contains("xref\n1 1\n"), "{tail}");
        assert!(tail.contains("3 1\n"), "{tail}");
        assert!(
            tail.contains("/Size 4 /Root 1 0 R /ID [<AB> <CD>] /Prev 45"),
            "{tail}"
        );
        // Every entry is exactly 20 bytes and points at its object.
        for line in tail.lines().filter(|l| l.ends_with(" n\r")) {
            assert_eq!(line.len() + 1, 20);
            let offset: usize = line.get(..10).unwrap_or_default().parse().unwrap_or(0);
            let id = out.get(offset..offset + 2).unwrap_or_default();
            assert!(id[0].is_ascii_digit() && id[1] == b' ', "{line}");
        }
        assert!(text.ends_with("%%EOF\n"));
    }

    #[test]
    fn damaged_trailers_are_refused() {
        assert_eq!(read_trailer(b"%PDF-1.4\nnothing here"), Err(SaveError::Damaged));
        assert_eq!(read_trailer(b"startxref\n999999\n%%EOF"), Err(SaveError::Damaged));
        assert_eq!(
            read_trailer(b"xref\ntrailer\n<< /Size 3 >>\nstartxref\n0\n%%EOF"),
            Err(SaveError::Damaged)
        );
    }
}
