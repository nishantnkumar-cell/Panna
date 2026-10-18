//! Form data files: FDF (PDF syntax) and XFDF (XML), as Acrobat reads and writes them, holding
//! just field names and values. Scripts and anything else in them are ignored.

use std::collections::BTreeMap;

use crate::FieldValue;
use crate::document::decode_text_string;
use crate::edit::crypt::parse_string;
use crate::edit::lex::{Lexer, array_items, dict_entries};

const MAX_FIELDS: usize = 5_000;
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    Fdf,
    Xfdf,
}

impl DataFormat {
    pub fn extension(self) -> &'static str {
        match self {
            DataFormat::Fdf => "fdf",
            DataFormat::Xfdf => "xfdf",
        }
    }
}

/// A field's value as a data file holds it: text, or the name of a check box or radio
/// button's state (`Off` for none).
fn value_text(value: &FieldValue) -> &str {
    match value {
        FieldValue::Text(t) => t,
        FieldValue::State(Some(s)) => s,
        FieldValue::State(None) => "Off",
    }
}

/// Fields as a tree by the parts of their full names (`address.city` is `city` in `address`).
#[derive(Default)]
struct Node<'a> {
    value: Option<&'a FieldValue>,
    kids: BTreeMap<&'a str, Node<'a>>,
}

fn tree<'a>(fields: &[(&'a str, &'a FieldValue)]) -> Node<'a> {
    let mut root = Node::default();
    for (name, value) in fields.iter().take(MAX_FIELDS) {
        let mut node = &mut root;
        for part in name.split('.').take(MAX_DEPTH) {
            node = node.kids.entry(part).or_default();
        }
        node.value = Some(value);
    }
    root
}

/// A form data file with `fields` (full names and values), for the PDF named `pdf_name`.
pub fn export(fields: &[(&str, &FieldValue)], format: DataFormat, pdf_name: &str) -> Vec<u8> {
    let root = tree(fields);
    match format {
        DataFormat::Fdf => {
            let mut out = String::from("1 0 obj\n<< /FDF << /Fields [\n");
            fdf_fields(&root, &mut out);
            out.push_str("] /F ");
            out.push_str(&pdf_string(pdf_name));
            out.push_str(" >> >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n");
            // The second line's bytes over 127 mark the file as binary, as in PDF files.
            let mut bytes = b"%FDF-1.2\n%\xe2\xe3\xcf\xd3\n".to_vec();
            bytes.extend_from_slice(out.as_bytes());
            bytes
        }
        DataFormat::Xfdf => {
            let mut out = String::from(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">\n",
            );
            out.push_str(&format!("<f href=\"{}\"/>\n<fields>\n", xml_escape(pdf_name)));
            xfdf_fields(&root, &mut out);
            out.push_str("</fields>\n</xfdf>\n");
            out.into_bytes()
        }
    }
}

fn fdf_fields(node: &Node<'_>, out: &mut String) {
    for (name, kid) in &node.kids {
        out.push_str("<< /T ");
        out.push_str(&pdf_string(name));
        match kid.value {
            Some(FieldValue::Text(t)) => {
                out.push_str(" /V ");
                out.push_str(&pdf_string(t));
            }
            Some(value) => {
                out.push_str(" /V ");
                out.push_str(&pdf_name(value_text(value)));
            }
            None => {}
        }
        if !kid.kids.is_empty() {
            out.push_str(" /Kids [\n");
            fdf_fields(kid, out);
            out.push(']');
        }
        out.push_str(" >>\n");
    }
}

fn xfdf_fields(node: &Node<'_>, out: &mut String) {
    for (name, kid) in &node.kids {
        out.push_str(&format!("<field name=\"{}\">", xml_escape(name)));
        if let Some(value) = kid.value {
            out.push_str(&format!("<value>{}</value>", xml_escape(value_text(value))));
        }
        if !kid.kids.is_empty() {
            out.push('\n');
            xfdf_fields(kid, out);
        }
        out.push_str("</field>\n");
    }
}

/// A PDF string: plain when it's printable ASCII, else UTF-16 with a byte order mark.
pub(crate) fn pdf_string(s: &str) -> String {
    if s.bytes().all(|b| (b' '..=b'~').contains(&b)) {
        let mut out = String::from("(");
        for c in s.chars() {
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

/// A PDF name, with `#xx` for bytes that can't appear in one as they are.
pub(crate) fn pdf_name(s: &str) -> String {
    let mut out = String::from("/");
    for b in s.bytes() {
        if b.is_ascii_graphic() && !b"()<>[]{}/%#".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("#{b:02X}"));
        }
    }
    out
}

pub(crate) fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Characters XML 1.0 can't hold at all.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// The field names and values in a form data file, FDF or XFDF.
pub fn import(bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    let start = bytes.trim_ascii_start();
    if start.starts_with(b"%FDF") {
        fdf_import(bytes).ok_or_else(|| "The FDF file is damaged.".into())
    } else if start.starts_with(b"<") || start.starts_with(b"\xef\xbb\xbf") {
        xfdf_import(bytes)
    } else {
        Err("This isn't an FDF or XFDF file.".into())
    }
}

fn fdf_import(bytes: &[u8]) -> Option<Vec<(String, String)>> {
    let at = bytes.windows(7).position(|w| w == b"/Fields")? + 7;
    let mut lex = Lexer::new(bytes, at);
    lex.skip_trivia();
    let span = lex.value()?;
    let mut out = Vec::new();
    fdf_kids(bytes.get(span)?, "", 0, &mut out);
    Some(out)
}

fn fdf_kids(array: &[u8], prefix: &str, depth: usize, out: &mut Vec<(String, String)>) {
    if depth > MAX_DEPTH {
        return;
    }
    for item in items(array) {
        if out.len() >= MAX_FIELDS {
            return;
        }
        // Fields given by reference aren't followed.
        let Some(dict) = array.trim_ascii().get(1..).and_then(|a| a.get(item)) else {
            continue;
        };
        let Some(entries) = dict_entries(dict) else {
            continue;
        };
        let get = |key: &[u8]| {
            entries
                .iter()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| dict.get(v.clone()))
        };
        let part = get(b"T").and_then(parse_string).map(|t| decode_text_string(&t));
        let name = match (&part, prefix.is_empty()) {
            (Some(part), true) => part.clone(),
            (Some(part), false) => format!("{prefix}.{part}"),
            (None, _) => prefix.to_owned(),
        };
        if let Some(value) = get(b"V").and_then(fdf_value)
            && !name.is_empty()
        {
            out.push((name.clone(), value));
        }
        if let Some(kids) = get(b"Kids") {
            fdf_kids(kids, &name, depth + 1, out);
        }
    }
}

/// The spans of an array's items, within the array without its brackets.
fn items(array: &[u8]) -> Vec<std::ops::Range<usize>> {
    let array = array.trim_ascii();
    array
        .strip_prefix(b"[")
        .and_then(|a| a.strip_suffix(b"]"))
        .and_then(array_items)
        .unwrap_or_default()
}

/// A value: a string, a name, or the first of an array of them.
fn fdf_value(raw: &[u8]) -> Option<String> {
    let raw = raw.trim_ascii();
    match raw.first()? {
        b'(' | b'<' if !raw.starts_with(b"<<") => parse_string(raw).map(|s| decode_text_string(&s)),
        b'/' => Some(name_text(raw.get(1..)?)),
        b'[' => {
            let inner = raw.get(1..raw.len().saturating_sub(1))?;
            items(raw)
                .into_iter()
                .find_map(|item| fdf_value(inner.get(item)?))
        }
        _ => None,
    }
}

/// A name's text, with `#xx` escapes undone.
pub(crate) fn name_text(raw: &[u8]) -> String {
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while let Some(&b) = raw.get(i) {
        let hex = raw
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b, hex) {
            (b'#', Some(byte)) => {
                bytes.push(byte);
                i += 3;
            }
            _ => {
                bytes.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| char::from(b)).collect())
}

fn xfdf_import(bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "The XFDF file isn't UTF-8 text.".to_owned())?;
    let doc = roxmltree::Document::parse_with_options(
        text.trim_start_matches('\u{feff}'),
        roxmltree::ParsingOptions {
            allow_dtd: false,
            ..roxmltree::ParsingOptions::default()
        },
    )
    .map_err(|e| format!("The XFDF file is damaged: {e}."))?;
    let root = doc.root_element();
    if root.tag_name().name() != "xfdf" {
        return Err("This isn't an XFDF file.".into());
    }
    let mut out = Vec::new();
    for fields in root.children().filter(|n| n.tag_name().name() == "fields") {
        xfdf_kids(fields, "", 0, &mut out);
    }
    Ok(out)
}

fn xfdf_kids(node: roxmltree::Node<'_, '_>, prefix: &str, depth: usize, out: &mut Vec<(String, String)>) {
    if depth > MAX_DEPTH {
        return;
    }
    for field in node.children().filter(|n| n.tag_name().name() == "field") {
        if out.len() >= MAX_FIELDS {
            return;
        }
        let part = field.attribute("name").unwrap_or_default();
        let name = if prefix.is_empty() {
            part.to_owned()
        } else {
            format!("{prefix}.{part}")
        };
        let value = field
            .children()
            .find(|n| matches!(n.tag_name().name(), "value" | "value-richtext"))
            .map(|v| v.text().unwrap_or_default().to_owned());
        if let Some(value) = value
            && !name.is_empty()
        {
            out.push((name.clone(), value));
        }
        xfdf_kids(field, &name, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<(&'static str, FieldValue)> {
        vec![
            ("name", FieldValue::Text("Jo (Bloggs)".into())),
            ("address.city", FieldValue::Text("Pune — पुणे".into())),
            ("address.zip", FieldValue::Text("411001".into())),
            ("agree", FieldValue::State(Some("Yes".into()))),
            ("colour", FieldValue::State(None)),
        ]
    }

    fn round_trip(format: DataFormat) {
        let values = sample();
        let fields: Vec<(&str, &FieldValue)> = values.iter().map(|(n, v)| (*n, v)).collect();
        let file = export(&fields, format, "form.pdf");
        let mut read = import(&file).unwrap();
        read.sort();
        let mut expect: Vec<(String, String)> = values
            .iter()
            .map(|(n, v)| ((*n).to_owned(), value_text(v).to_owned()))
            .collect();
        expect.sort();
        assert_eq!(read, expect);
    }

    #[test]
    fn fdf_round_trips() {
        round_trip(DataFormat::Fdf);
    }

    #[test]
    fn xfdf_round_trips() {
        round_trip(DataFormat::Xfdf);
    }

    #[test]
    fn reads_acrobat_style_files() {
        let fdf = b"%FDF-1.2\n1 0 obj<</FDF<</Fields[<</T(a)/V/On>><</T(b)/Kids[<</T(c)/V[(x)(y)]>>]>>\
                    <</T(d)/V<FEFF00E9>>>]>>>>endobj trailer<</Root 1 0 R>>%%EOF";
        assert_eq!(
            import(fdf).unwrap(),
            [
                ("a".into(), "On".into()),
                ("b.c".into(), "x".into()),
                ("d".into(), "é".into())
            ]
        );
        let xfdf = br#"<?xml version="1.0"?><xfdf xmlns="http://ns.adobe.com/xfdf/"><fields>
            <field name="a"><value>1 &amp; 2</value></field></fields></xfdf>"#;
        assert_eq!(import(xfdf).unwrap(), [("a".into(), "1 & 2".into())]);
        assert!(import(b"hello").is_err());
        assert!(import(b"<!DOCTYPE x [<!ENTITY a \"b\">]><xfdf/>").is_err());
    }
}
