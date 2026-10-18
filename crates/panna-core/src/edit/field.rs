//! Filled-in form values: the field's new /V, and what its widgets now show.

use std::fmt::Write as _;

use hayro::hayro_syntax::object::{Array, Dict, Rect};

use super::annot::{literal, n, text_string, text_width, win_ansi, wrap};
use crate::forms::{Field, FieldKind, FieldValue};

/// Edits to one existing object: keys to set, each with its new value in PDF syntax.
pub(crate) type Entries = Vec<(&'static str, String)>;

/// The new /V for a field, in PDF syntax.
pub(crate) fn value(field: &Field, value: &FieldValue) -> String {
    match value {
        FieldValue::Text(t) => {
            let t = match &field.kind {
                FieldKind::Text {
                    max_len: Some(max), ..
                } => t.chars().take(*max).collect(),
                _ => t.clone(),
            };
            let t = field.format.as_ref().map_or(t.clone(), |f| f.saved(&t));
            text_string(&t)
        }
        FieldValue::State(Some(state)) => name(state),
        FieldValue::State(None) => "/Off".into(),
    }
}

/// A name object, with characters outside the regular set escaped as `#xx`.
pub(crate) fn name(s: &str) -> String {
    let mut out = String::from("/");
    for b in s.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'-' | b'_'
                    | b'.'
                    | b'+'
                    | b'*'
                    | b','
                    | b';'
                    | b':'
                    | b'!'
                    | b'?'
                    | b'@'
                    | b'='
                    | b'~'
                    | b'\''
                    | b'"'
                    | b'|'
                    | b'^'
                    | b'`'
                    | b'&'
                    | b'$'
            )
        {
            out.push(b as char);
        } else {
            let _ = write!(out, "#{b:02X}");
        }
    }
    out
}

/// What a widget's text shows: the choice's label rather than its saved value, and stars for
/// passwords.
fn shown(field: &Field, value: &str) -> String {
    match &field.kind {
        FieldKind::Text { password: true, .. } => "*".repeat(value.chars().count()),
        FieldKind::Text { .. } if field.format.is_some() => field
            .format
            .as_ref()
            .and_then(|f| f.display(value))
            .unwrap_or_else(|| value.to_owned()),
        FieldKind::Choice { options, .. } => options
            .iter()
            .find(|o| o.export == value)
            .map_or_else(|| value.to_owned(), |o| o.label.clone()),
        _ => value.to_owned(),
    }
}

fn mk_color(widget: &Dict<'_>, key: &[u8]) -> Option<String> {
    let c: Vec<f64> = widget
        .get::<Dict<'_>>(b"MK")?
        .get::<Array<'_>>(key)?
        .iter::<f64>()
        .take(4)
        .map(|v| v.clamp(0.0, 1.0))
        .collect();
    match c.as_slice() {
        [g] => Some(format!("{} g", n(*g))),
        [r, g, b] => Some(format!("{} {} {} rg", n(*r), n(*g), n(*b))),
        [c, m, y, k] => Some(format!("{} {} {} {} k", n(*c), n(*m), n(*y), n(*k))),
        _ => None,
    }
}

/// A normal appearance stream showing `value` in a text or choice widget.
pub(crate) fn appearance(field: &Field, widget: &Dict<'_>, value: &str) -> Option<Vec<u8>> {
    let rect = widget.get::<Rect>(b"Rect")?;
    let (w, h) = ((rect.x1 - rect.x0).abs(), (rect.y1 - rect.y0).abs());
    if !(w.is_finite() && h.is_finite()) || w < 1.0 || h < 1.0 {
        return None;
    }
    let multiline = matches!(field.kind, FieldKind::Text { multiline: true, .. });
    let size = match f64::from(field.font_size) {
        s if s > 0.0 => s,
        _ if multiline => 10.0,
        _ => ((h - 4.0) * 0.75).clamp(4.0, 12.0),
    };
    let [r, g, b] = field.color.0.map(|c| f64::from(c.clamp(0.0, 1.0)));
    let mut body = String::new();
    if let Some(bg) = mk_color(widget, b"BG") {
        let _ = write!(body, "{bg} 0 0 {} {} re f ", n(w), n(h));
    }
    if let Some(border) = mk_color(widget, b"BC") {
        // Stroke colour: the fill operators turned into stroke ones.
        let stroke = border
            .replace(" rg", " RG")
            .replace(" g", " G")
            .replace(" k", " K");
        let _ = write!(body, "{stroke} 1 w 0.5 0.5 {} {} re S ", n(w - 1.0), n(h - 1.0));
    }
    let text = shown(field, value);
    let _ = write!(
        body,
        "/Tx BMC q 1 1 {} {} re W n BT /Helv {} Tf {} {} {} rg ",
        n(w - 2.0),
        n(h - 2.0),
        n(size),
        n(r),
        n(g),
        n(b)
    );
    let lines = if multiline {
        wrap(&text, ((w - 4.0) / size).max(1.0))
    } else {
        vec![text.replace(['\r', '\n'], " ")]
    };
    let leading = size * 1.15;
    // Single lines sit centred vertically; multiline text starts at the top.
    let mut y = if multiline {
        h - 2.0 - size * 0.9
    } else {
        (h - size * 0.7) / 2.0
    };
    for line in lines.iter().take(2_000) {
        let width = text_width(line) * size;
        let x = match field.align {
            1 => (w - width) / 2.0,
            2 => w - 2.0 - width,
            _ => 2.0,
        };
        let _ = write!(
            body,
            "1 0 0 1 {} {} Tm ({}) Tj ",
            n(x.max(2.0)),
            n(y),
            literal(&win_ansi(line))
        );
        y -= leading;
    }
    body.push_str("ET Q EMC");
    Some(
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 {} {}] /Resources << /Font << /Helv << \
             /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> >> >> \
             /Length {} >>\nstream\n{body}\nendstream",
            n(w),
            n(h),
            body.len()
        )
        .into_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_escape_odd_characters() {
        assert_eq!(name("Yes"), "/Yes");
        assert_eq!(name("a b#/"), "/a#20b#23#2F");
    }
}
