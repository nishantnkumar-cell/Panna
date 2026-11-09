//! Page labels: the numbers a document gives its pages, such as i, ii, iii for a preface
//! then 1, 2, 3 (ISO 32000-2, 12.4.2).

use std::collections::HashSet;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier};
use serde::{Deserialize, Serialize};

use crate::document::decode_text_string;

/// How the numbers in a range are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LabelStyle {
    /// 1, 2, 3
    #[default]
    Decimal,
    /// I, II, III
    UpperRoman,
    /// i, ii, iii
    LowerRoman,
    /// A to Z, then AA to ZZ, …
    UpperLetters,
    /// a to z, then aa to zz, …
    LowerLetters,
    /// The prefix alone.
    None,
}

impl LabelStyle {
    fn code(self) -> Option<&'static str> {
        match self {
            LabelStyle::Decimal => Some("D"),
            LabelStyle::UpperRoman => Some("R"),
            LabelStyle::LowerRoman => Some("r"),
            LabelStyle::UpperLetters => Some("A"),
            LabelStyle::LowerLetters => Some("a"),
            LabelStyle::None => None,
        }
    }
}

/// Pages from `start` (zero-based) up to the next range's start, labelled `prefix` then a
/// number counting from `first`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelRange {
    pub start: usize,
    pub style: LabelStyle,
    pub prefix: String,
    pub first: u32,
}

/// Ranges read at most, and number-tree nodes followed at most.
const MAX_RANGES: usize = 10_000;
const MAX_NODES: usize = 10_000;

/// The document's label ranges, by start page; empty when its pages are just numbered.
pub(crate) fn read(pdf: &Pdf) -> Vec<LabelRange> {
    let xref = pdf.xref();
    let Some(root) = xref
        .get::<Dict<'_>>(xref.root_id())
        .and_then(|c| c.get::<Dict<'_>>(b"PageLabels"))
    else {
        return Vec::new();
    };
    let pages = pdf.pages().len();
    let mut out: Vec<LabelRange> = Vec::new();
    let mut seen: HashSet<ObjectIdentifier> = HashSet::new();
    let mut nodes = vec![root];
    let mut visited = 0;
    while let Some(node) = nodes.pop() {
        visited += 1;
        if visited > MAX_NODES || out.len() >= MAX_RANGES {
            break;
        }
        if let Some(kids) = node.get::<Array<'_>>(b"Kids") {
            for kid in kids.iter::<Dict<'_>>() {
                if kid.obj_id().is_none_or(|id| seen.insert(id)) {
                    nodes.push(kid);
                }
            }
        }
        let Some(nums) = node.get::<Array<'_>>(b"Nums") else {
            continue;
        };
        let mut items = nums.iter::<Object<'_>>();
        while let (Some(key), Some(value)) = (items.next(), items.next()) {
            let (Object::Number(key), Object::Dict(value)) = (key, value) else {
                continue;
            };
            let Ok(start) = usize::try_from(key.as_i64()) else {
                continue;
            };
            if start >= pages {
                continue;
            }
            let style = match value.get::<Name<'_>>(b"S").as_ref().map(|n| n.as_str()) {
                Some("D") => LabelStyle::Decimal,
                Some("R") => LabelStyle::UpperRoman,
                Some("r") => LabelStyle::LowerRoman,
                Some("A") => LabelStyle::UpperLetters,
                Some("a") => LabelStyle::LowerLetters,
                _ => LabelStyle::None,
            };
            let prefix = value
                .get::<hayro::hayro_syntax::object::String<'_>>(b"P")
                .map(|s| decode_text_string(s.as_bytes()))
                .unwrap_or_default();
            let first = value
                .get::<i64>(b"St")
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n >= 1)
                .unwrap_or(1);
            out.push(LabelRange {
                start,
                style,
                prefix,
                first,
            });
        }
    }
    out.sort_by_key(|r| r.start);
    out.dedup_by_key(|r| r.start);
    out
}

/// The label of page `index` (zero-based) under `ranges`; its number from 1 when no range
/// covers it.
pub fn page_label(ranges: &[LabelRange], index: usize) -> String {
    let Some(range) = ranges.iter().rev().find(|r| r.start <= index) else {
        return (index + 1).to_string();
    };
    let n = u64::from(range.first) + (index - range.start) as u64;
    let number = match range.style {
        LabelStyle::Decimal => n.to_string(),
        LabelStyle::UpperRoman => roman(n),
        LabelStyle::LowerRoman => roman(n).to_lowercase(),
        LabelStyle::UpperLetters => letters(n),
        LabelStyle::LowerLetters => letters(n).to_lowercase(),
        LabelStyle::None => String::new(),
    };
    format!("{}{number}", range.prefix)
}

/// Roman numerals; beyond 3999, as many M as it takes.
fn roman(mut n: u64) -> String {
    const PARTS: [(u64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, part) in PARTS {
        while n >= value && out.len() < 64 {
            out.push_str(part);
            n -= value;
        }
    }
    out
}

/// A to Z, then AA to ZZ, AAA…, as PDF counts with letters.
fn letters(n: u64) -> String {
    let Some(n) = n.checked_sub(1) else {
        return String::new();
    };
    let letter = char::from(b'A' + (n % 26) as u8);
    let times = usize::try_from(n / 26 + 1).unwrap_or(1).min(64);
    std::iter::repeat_n(letter, times).collect()
}

/// The `/PageLabels` value for `ranges`, with `string` writing each prefix as a string as it
/// goes in the file.
pub(crate) fn entry(
    ranges: &[LabelRange],
    mut string: impl FnMut(&str) -> Result<String, crate::SaveError>,
) -> Result<String, crate::SaveError> {
    let mut out = String::from("<< /Nums [");
    for range in ranges {
        out.push_str(&format!(" {} <<", range.start));
        if let Some(code) = range.style.code() {
            out.push_str(&format!(" /S /{code}"));
        }
        if !range.prefix.is_empty() {
            out.push_str(&format!(" /P {}", string(&range.prefix)?));
        }
        if range.first != 1 {
            out.push_str(&format!(" /St {}", range.first));
        }
        out.push_str(" >>");
    }
    out.push_str(" ] >>");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_count_in_each_style() {
        let ranges = vec![
            LabelRange {
                start: 0,
                style: LabelStyle::LowerRoman,
                prefix: String::new(),
                first: 1,
            },
            LabelRange {
                start: 4,
                style: LabelStyle::Decimal,
                prefix: "A-".into(),
                first: 8,
            },
            LabelRange {
                start: 6,
                style: LabelStyle::UpperLetters,
                prefix: String::new(),
                first: 26,
            },
        ];
        let labels: Vec<String> = (0..8).map(|i| page_label(&ranges, i)).collect();
        assert_eq!(labels, ["i", "ii", "iii", "iv", "A-8", "A-9", "Z", "AA"]);
        assert_eq!(page_label(&[], 2), "3");
        assert_eq!(roman(1994), "MCMXCIV");
    }
}
