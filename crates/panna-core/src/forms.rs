//! Reads a document's interactive form (AcroForm): its text fields, checkboxes, radio buttons
//! and choice lists, with where each one's widgets sit on the pages.
//!
//! The field tree is a hierarchy a hostile file can make cyclic or huge, so the walk is bounded
//! by depth, field count and a visited set.

use std::collections::{HashMap, HashSet};

use hayro::hayro_interpret::TransformExt;
use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Name, Object, Rect};
use hayro::kurbo::{Affine, Point};

use crate::document::decode_text_string;
use crate::edit::Color;
use crate::{ObjRef, TextBox};

const MAX_FIELDS: usize = 5_000;
const MAX_DEPTH: u8 = 32;
const MAX_WIDGETS: usize = 500;
const MAX_OPTIONS: usize = 2_000;
/// Longest value or label kept.
const MAX_TEXT: usize = 32_000;

/// What sort of field it is, with what it allows.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text {
        multiline: bool,
        password: bool,
        /// Most characters allowed.
        max_len: Option<usize>,
    },
    CheckBox,
    /// One of a group: each widget is one choice, named by its on state.
    Radio,
    Choice {
        /// A drop-down rather than a list box.
        combo: bool,
        /// A drop-down that also takes typed text.
        editable: bool,
        options: Vec<ChoiceOption>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceOption {
    /// The value saved in the file.
    pub export: String,
    /// What the list shows.
    pub label: String,
}

/// A field's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    /// Text fields and choices.
    Text(String),
    /// Checkboxes and radio buttons: the on state that is set, or none when off.
    State(Option<String>),
}

/// One place a field shows on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    pub page: usize,
    pub annot: ObjRef,
    /// Its area, in the same top-left, unrotated-view space as page text.
    pub at: TextBox,
    /// For checkboxes and radio buttons: the state this widget turns on.
    pub on: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The field's own object, which holds its value.
    pub id: ObjRef,
    /// Its full name, parts joined with dots.
    pub name: String,
    /// The description shown as a tooltip (/TU), if any.
    pub tooltip: String,
    pub kind: FieldKind,
    pub value: FieldValue,
    /// What resetting the form puts back (/DV): empty or off when the file names none.
    pub default: FieldValue,
    pub read_only: bool,
    pub required: bool,
    /// Font size from the default appearance; 0 means fit to the field.
    pub font_size: f32,
    pub color: Color,
    /// 0 left, 1 centred, 2 right.
    pub align: u8,
    /// For text fields: a standard number, date or other format the form asks for.
    pub format: Option<crate::Format>,
    /// For fields worked out from others: how.
    pub calculation: Option<crate::Calculation>,
    pub widgets: Vec<Widget>,
}

impl Field {
    /// Whether the user can change it here.
    pub fn editable(&self) -> bool {
        !self.read_only && !self.widgets.is_empty()
    }
}

struct Inherited {
    name: String,
    ft: Option<String>,
    flags: u32,
    da: Option<Vec<u8>>,
    q: Option<u8>,
}

struct Ctx {
    /// Widget annotations by object, with their page and that page's view transform.
    placed: HashMap<ObjRef, (usize, Affine)>,
    seen: HashSet<ObjRef>,
    out: Vec<Field>,
    default_da: Option<Vec<u8>>,
    default_q: u8,
    /// Fields' places in the form's calculation order (/CO).
    order: HashMap<ObjRef, usize>,
}

fn obj_ref(dict: &Dict<'_>) -> Option<ObjRef> {
    let id = dict.obj_id()?;
    Some(ObjRef {
        number: u32::try_from(id.obj_number).ok()?,
        generation: u16::try_from(id.gen_number).ok()?,
    })
}

pub(crate) fn read(pdf: &Pdf) -> Vec<Field> {
    let xref = pdf.xref();
    let Some(form) = xref
        .get::<Dict<'_>>(xref.root_id())
        .and_then(|catalog| catalog.get::<Dict<'_>>(b"AcroForm"))
    else {
        return Vec::new();
    };
    let Some(fields) = form.get::<Array<'_>>(b"Fields") else {
        return Vec::new();
    };
    let mut placed = HashMap::new();
    for (index, page) in pdf.pages().iter().enumerate() {
        let to_view = page.initial_transform(true).to_kurbo();
        if let Some(annots) = page.raw().get::<Array<'_>>(b"Annots") {
            for annot in annots.iter::<Dict<'_>>().take(4_000) {
                if let Some(r) = obj_ref(&annot) {
                    placed.entry(r).or_insert((index, to_view));
                }
            }
        }
    }
    let mut ctx = Ctx {
        placed,
        seen: HashSet::new(),
        out: Vec::new(),
        default_da: form
            .get::<hayro::hayro_syntax::object::String<'_>>(b"DA")
            .map(|s| s.as_bytes().to_vec()),
        default_q: form.get::<i64>(b"Q").map_or(0, |q| q.clamp(0, 2) as u8),
        order: form
            .get::<Array<'_>>(b"CO")
            .map(|co| {
                co.iter::<Dict<'_>>()
                    .take(MAX_FIELDS)
                    .filter_map(|d| obj_ref(&d))
                    .enumerate()
                    .map(|(i, r)| (r, i))
                    .collect()
            })
            .unwrap_or_default(),
    };
    let root = Inherited {
        name: String::new(),
        ft: None,
        flags: 0,
        da: None,
        q: None,
    };
    for field in fields.iter::<Dict<'_>>() {
        walk(&mut ctx, &field, &root, 0);
        if ctx.out.len() >= MAX_FIELDS {
            break;
        }
    }
    ctx.out
}

fn text(dict: &Dict<'_>, key: &[u8]) -> Option<String> {
    dict.get::<hayro::hayro_syntax::object::String<'_>>(key).map(|s| {
        let mut t = decode_text_string(s.as_bytes());
        if let Some((cut, _)) = t.char_indices().nth(MAX_TEXT) {
            t.truncate(cut);
        }
        t
    })
}

fn walk(ctx: &mut Ctx, node: &Dict<'_>, parent: &Inherited, depth: u8) {
    if depth > MAX_DEPTH || ctx.out.len() >= MAX_FIELDS {
        return;
    }
    let Some(id) = obj_ref(node) else { return };
    if !ctx.seen.insert(id) {
        return;
    }
    let partial = text(node, b"T");
    let name = match (&partial, parent.name.is_empty()) {
        (Some(t), true) => t.clone(),
        (Some(t), false) => format!("{}.{t}", parent.name),
        (None, _) => parent.name.clone(),
    };
    let here = Inherited {
        name,
        ft: node
            .get::<Name<'_>>(b"FT")
            .map(|n| n.as_str().to_owned())
            .or_else(|| parent.ft.clone()),
        flags: node
            .get::<i64>(b"Ff")
            .map_or(parent.flags, |f| u32::try_from(f & 0xFFFF_FFFF).unwrap_or(0)),
        da: node
            .get::<hayro::hayro_syntax::object::String<'_>>(b"DA")
            .map(|s| s.as_bytes().to_vec())
            .or_else(|| parent.da.clone()),
        q: node.get::<i64>(b"Q").map(|q| q.clamp(0, 2) as u8).or(parent.q),
    };
    // Kids that are fields (they have names) make this a branch; kids without names are this
    // field's widgets.
    let kids: Vec<Dict<'_>> = node
        .get::<Array<'_>>(b"Kids")
        .map(|k| k.iter::<Dict<'_>>().take(MAX_WIDGETS).collect())
        .unwrap_or_default();
    if kids.iter().any(|k| k.contains_key(b"T")) {
        for kid in &kids {
            walk(ctx, kid, &here, depth + 1);
        }
        return;
    }
    let widgets: Vec<Dict<'_>> = if kids.is_empty() { vec![node.clone()] } else { kids };
    terminal(ctx, node, id, &here, &widgets);
}

fn terminal(ctx: &mut Ctx, node: &Dict<'_>, id: ObjRef, here: &Inherited, widgets: &[Dict<'_>]) {
    let flags = here.flags;
    let kind = match here.ft.as_deref() {
        Some("Tx") => FieldKind::Text {
            multiline: flags & (1 << 12) != 0,
            password: flags & (1 << 13) != 0,
            max_len: node
                .get::<i64>(b"MaxLen")
                .and_then(|m| usize::try_from(m).ok())
                .filter(|m| *m > 0),
        },
        // Push buttons do nothing without scripts, which Panna never runs.
        Some("Btn") if flags & (1 << 16) != 0 => return,
        Some("Btn") if flags & (1 << 15) != 0 => FieldKind::Radio,
        Some("Btn") => FieldKind::CheckBox,
        Some("Ch") => FieldKind::Choice {
            combo: flags & (1 << 17) != 0,
            editable: flags & (1 << 18) != 0,
            options: options(node),
        },
        // Signatures and unknown kinds aren't fillable here.
        _ => return,
    };
    let button = matches!(kind, FieldKind::CheckBox | FieldKind::Radio);
    let read_value = |key: &[u8]| {
        if button {
            FieldValue::State(
                node.get::<Name<'_>>(key)
                    .map(|n| n.as_str().to_owned())
                    .filter(|v| v != "Off"),
            )
        } else {
            // A multi-select list's value is an array; the first item stands for it.
            let v = text(node, key).or_else(|| {
                node.get::<Array<'_>>(key).and_then(|a| {
                    a.iter::<hayro::hayro_syntax::object::String<'_>>()
                        .next()
                        .map(|s| decode_text_string(s.as_bytes()))
                })
            });
            FieldValue::Text(v.unwrap_or_default())
        }
    };
    let value = read_value(b"V");
    let default = read_value(b"DV");
    let da = here.da.as_ref().or(ctx.default_da.as_ref());
    let (font_size, color) = da.map_or((0.0, Color::BLACK), |da| appearance(da));
    let mut placed = Vec::new();
    for w in widgets {
        let Some(annot) = obj_ref(w) else { continue };
        let Some(&(page, to_view)) = ctx.placed.get(&annot) else {
            continue;
        };
        // Hidden widgets aren't shown or filled.
        if w.get::<i64>(b"F").is_some_and(|f| f & 2 != 0) {
            continue;
        }
        let Some(rect) = w.get::<Rect>(b"Rect") else {
            continue;
        };
        let a = to_view * Point::new(rect.x0, rect.y0);
        let b = to_view * Point::new(rect.x1, rect.y1);
        let at = TextBox {
            x0: a.x.min(b.x) as f32,
            y0: a.y.min(b.y) as f32,
            x1: a.x.max(b.x) as f32,
            y1: a.y.max(b.y) as f32,
        };
        let on = button
            .then(|| {
                w.get::<Dict<'_>>(b"AP")
                    .and_then(|ap| ap.get::<Dict<'_>>(b"N"))
                    .and_then(|n| n.keys().map(|k| k.as_str().to_owned()).find(|k| k != "Off"))
            })
            .flatten();
        placed.push(Widget { page, annot, at, on });
    }
    ctx.out.push(Field {
        id,
        name: here.name.clone(),
        tooltip: text(node, b"TU").unwrap_or_default(),
        kind,
        value,
        default,
        read_only: flags & 1 != 0,
        required: flags & 2 != 0,
        font_size,
        color,
        align: here.q.unwrap_or(ctx.default_q),
        format: format(node),
        calculation: script(node, b"C")
            .and_then(|s| crate::Formula::from_script(&s))
            .map(|formula| crate::Calculation {
                // Fields left out of the order are worked out last, in form order.
                order: ctx.order.get(&id).copied().unwrap_or(MAX_FIELDS + ctx.out.len()),
                formula,
            }),
        widgets: placed,
    });
}

/// The format set by the field's format (or, failing that, keystroke) script, when it is a
/// standard one. The script itself is never run.
fn format(node: &Dict<'_>) -> Option<crate::Format> {
    [b"F".as_slice(), b"K"]
        .iter()
        .find_map(|key| crate::Format::from_script(&script(node, key)?))
}

/// The text of the field's script for the event `key` (/AA), to recognise, never to run.
fn script(node: &Dict<'_>, key: &[u8]) -> Option<String> {
    let js = node.get::<Dict<'_>>(b"AA")?.get::<Dict<'_>>(key)?;
    js.get::<hayro::hayro_syntax::object::String<'_>>(b"JS")
        .map(|s| decode_text_string(s.as_bytes()))
        .or_else(|| {
            js.get::<hayro::hayro_syntax::object::Stream<'_>>(b"JS")
                .and_then(|s| s.decoded().ok())
                .map(|b| {
                    String::from_utf8_lossy(b.get(..b.len().min(MAX_TEXT)).unwrap_or_default()).into_owned()
                })
        })
}

/// A choice field's options: plain strings, or [export label] pairs.
fn options(node: &Dict<'_>) -> Vec<ChoiceOption> {
    let Some(opt) = node.get::<Array<'_>>(b"Opt") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in opt.iter::<Object<'_>>().take(MAX_OPTIONS) {
        match item {
            Object::Array(pair) => {
                let mut strings = pair
                    .iter::<hayro::hayro_syntax::object::String<'_>>()
                    .map(|s| decode_text_string(s.as_bytes()));
                let export = strings.next().unwrap_or_default();
                let label = strings.next().unwrap_or_else(|| export.clone());
                out.push(ChoiceOption { export, label });
            }
            Object::String(s) => {
                let s = decode_text_string(s.as_bytes());
                out.push(ChoiceOption {
                    export: s.clone(),
                    label: s,
                });
            }
            _ => {}
        }
    }
    out
}

/// The font size and fill colour in a default appearance string such as
/// `/Helv 12 Tf 0 0 1 rg`.
pub(crate) fn appearance(da: &[u8]) -> (f32, Color) {
    let text = String::from_utf8_lossy(da);
    let words: Vec<&str> = text.split_whitespace().collect();
    let num = |i: usize| -> Option<f32> { words.get(i)?.parse::<f32>().ok().filter(|v| v.is_finite()) };
    let mut size = 0.0;
    let mut color = Color::BLACK;
    for (i, w) in words.iter().enumerate() {
        match *w {
            "Tf" => size = i.checked_sub(1).and_then(num).unwrap_or(0.0).clamp(0.0, 144.0),
            "g" => {
                if let Some(g) = i.checked_sub(1).and_then(num) {
                    let g = g.clamp(0.0, 1.0);
                    color = Color([g, g, g]);
                }
            }
            "rg" => {
                if let (Some(r), Some(g), Some(b)) = (
                    i.checked_sub(3).and_then(num),
                    i.checked_sub(2).and_then(num),
                    i.checked_sub(1).and_then(num),
                ) {
                    color = Color([r, g, b].map(|c| c.clamp(0.0, 1.0)));
                }
            }
            _ => {}
        }
    }
    (size, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_appearances_give_size_and_colour() {
        assert_eq!(
            appearance(b"/Helv 10 Tf 0 0 1 rg"),
            (10.0, Color([0.0, 0.0, 1.0]))
        );
        assert_eq!(appearance(b"0.5 g /F1 0 Tf"), (0.0, Color([0.5, 0.5, 0.5])));
        assert_eq!(appearance(b"Tf rg"), (0.0, Color::BLACK));
    }
}
