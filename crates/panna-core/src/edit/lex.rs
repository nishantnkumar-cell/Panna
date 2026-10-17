//! A minimal PDF tokenizer for incremental saves.
//!
//! Saving copies parts of existing objects byte for byte (a page dictionary minus its `/Annots`,
//! the trailer's `/Root` and `/ID`), so their strings keep the exact encoding (and encryption) of
//! the original file. This module only finds where top-level values start and end; it never
//! interprets them. Input is untrusted: every loop advances, and nesting depth is bounded.

use std::ops::Range;

const MAX_DEPTH: usize = 64;

fn is_white(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_regular(b: u8) -> bool {
    !is_white(b) && !is_delim(b)
}

pub(crate) struct Lexer<'a> {
    data: &'a [u8],
    pub(crate) pos: usize,
}

impl<'a> Lexer<'a> {
    pub(crate) fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn starts_with(&self, s: &[u8]) -> bool {
        self.data.get(self.pos..).is_some_and(|rest| rest.starts_with(s))
    }

    pub(crate) fn skip_trivia(&mut self) {
        while let Some(b) = self.peek() {
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(b) = self.peek() {
                    if b == b'\r' || b == b'\n' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// Consumes `s` (after trivia) if it is next.
    pub(crate) fn eat(&mut self, s: &[u8]) -> bool {
        self.skip_trivia();
        if self.starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn regular_run(&mut self) -> &'a [u8] {
        let start = self.pos;
        while self.peek().is_some_and(is_regular) {
            self.pos += 1;
        }
        self.data.get(start..self.pos).unwrap_or_default()
    }

    /// Reads a name, returning it without the leading slash and still escaped.
    pub(crate) fn name(&mut self) -> Option<&'a [u8]> {
        self.skip_trivia();
        if self.peek() != Some(b'/') {
            return None;
        }
        self.pos += 1;
        Some(self.regular_run())
    }

    /// Reads a non-negative integer.
    pub(crate) fn uint(&mut self) -> Option<u64> {
        self.skip_trivia();
        let run = self.regular_run();
        if run.is_empty() || run.len() > 18 || !run.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(run).ok()?.parse().ok()
    }

    /// Skips one value and returns its byte span.
    pub(crate) fn value(&mut self) -> Option<Range<usize>> {
        self.skip_trivia();
        let start = self.pos;
        self.value_at(0)?;
        Some(start..self.pos)
    }

    fn skip_value(&mut self, depth: usize) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.skip_trivia();
        match self.peek()? {
            b'/' => {
                self.pos += 1;
                self.regular_run();
            }
            b'(' => self.skip_literal_string()?,
            b'<' if self.starts_with(b"<<") => {
                self.pos += 2;
                loop {
                    if self.eat(b">>") {
                        break;
                    }
                    self.name()?;
                    self.value_at(depth + 1)?;
                }
            }
            b'<' => {
                self.pos += 1;
                while self.peek()? != b'>' {
                    self.pos += 1;
                }
                self.pos += 1;
            }
            b'[' => {
                self.pos += 1;
                loop {
                    if self.eat(b"]") {
                        break;
                    }
                    self.value_at(depth + 1)?;
                }
            }
            b if is_regular(b) => {
                self.regular_run();
            }
            _ => return None,
        }
        Some(())
    }

    fn value_at(&mut self, depth: usize) -> Option<()> {
        self.skip_trivia();
        let start = self.pos;
        self.skip_value(depth)?;
        // `12 0 R` is one value: an indirect reference.
        if self.data.get(start..self.pos)?.iter().all(u8::is_ascii_digit) {
            let save = self.pos;
            if !(self.uint().is_some() && self.eat(b"R") && !self.peek().is_some_and(is_regular)) {
                self.pos = save;
            }
        }
        Some(())
    }

    fn skip_literal_string(&mut self) -> Option<()> {
        self.pos += 1;
        let mut open = 1usize;
        loop {
            match self.peek()? {
                b'\\' => self.pos += 2,
                b'(' => {
                    open += 1;
                    self.pos += 1;
                }
                b')' => {
                    open -= 1;
                    self.pos += 1;
                    if open == 0 {
                        return Some(());
                    }
                }
                _ => self.pos += 1,
            }
        }
    }
}

/// A dictionary's top-level entries: each key (without the slash, still escaped) and the byte
/// span of its value within `data`. `data` must start with `<<`, optionally after whitespace.
pub(crate) fn dict_entries(data: &[u8]) -> Option<Vec<(&[u8], Range<usize>)>> {
    let mut lex = Lexer::new(data, 0);
    if !lex.eat(b"<<") {
        return None;
    }
    let mut out = Vec::new();
    loop {
        if lex.eat(b">>") {
            return Some(out);
        }
        let key = lex.name()?;
        let value = lex.value()?;
        out.push((key, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(data: &[u8]) -> Vec<(String, String)> {
        dict_entries(data)
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| {
                (
                    String::from_utf8_lossy(k).into_owned(),
                    String::from_utf8_lossy(data.get(v).unwrap_or_default()).into_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn splits_values_of_every_kind() {
        let e = entries(
            b"<< /Type/Page /Parent 2 0 R /Annots [3 0 R 4 0 R] /Res << /F << /F1 9 0 R >> >> \
              /S (a \\) (nested) b) /H <0A0b> /N -1.5 /B true % comment\n /X /Y >>",
        );
        let expect = [
            ("Type", "/Page"),
            ("Parent", "2 0 R"),
            ("Annots", "[3 0 R 4 0 R]"),
            ("Res", "<< /F << /F1 9 0 R >> >>"),
            ("S", "(a \\) (nested) b)"),
            ("H", "<0A0b>"),
            ("N", "-1.5"),
            ("B", "true"),
            ("X", "/Y"),
        ];
        assert_eq!(e.len(), expect.len());
        for ((k, v), (ek, ev)) in e.iter().zip(expect) {
            assert_eq!((k.as_str(), v.as_str()), (ek, ev));
        }
    }

    #[test]
    fn numbers_that_are_not_references_stay_separate() {
        let e = entries(b"<< /A 1 /B 2 /Kids [1 2 3] /C 4 0 R>>");
        assert_eq!(e[0].1, "1");
        assert_eq!(e[1].1, "2");
        assert_eq!(e[2].1, "[1 2 3]");
        assert_eq!(e[3].1, "4 0 R");
    }

    #[test]
    fn rejects_malformed_and_deep_input() {
        assert!(dict_entries(b"<< /A (unterminated >>").is_none());
        assert!(dict_entries(b"<< /A [1 2 >>").is_none());
        assert!(dict_entries(b"no dict").is_none());
        let deep = "[".repeat(10_000);
        assert!(dict_entries(format!("<< /A {deep} >>").as_bytes()).is_none());
    }
}
