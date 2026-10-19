//! Redaction: taking content out of marked areas of a page for good, not just covering it.
//!
//! A page's content is read operator by operator, following the graphics and text state, so
//! each glyph, picture and drawing has a known place on the page:
//!
//! - glyphs that touch a marked area are taken out of the text, with the space they took kept
//!   so that the rest of the line stays where it was;
//! - pictures that touch a marked area are taken out whole, and dropped from the page's
//!   resources so their data leaves the file;
//! - drawings that lie wholly inside a marked area are taken out (ones that only cross one
//!   stay, under the black box);
//! - forms (groups of content) that touch a marked area are copied and redacted the same way;
//! - replacement text for screen readers (`/ActualText`, `/Alt`, `/E`) is dropped from the
//!   page's marked content, since it may repeat what was taken out.
//!
//! When a glyph's width can't be worked out (an unusual font), every glyph of that text
//! operation is taken out if any of it might touch a marked area.

use std::collections::{HashMap, HashSet};

/// Glyph boxes, from the baseline, as a share of the font size (as page text uses).
const ASCENT: f64 = 0.8;
const DESCENT: f64 = -0.2;
/// Assumed advance and box width of a glyph whose width isn't known, in ems.
const GUESS_ADVANCE: f64 = 0.6;
const GUESS_BOX: f64 = 1.0;
/// How deeply forms inside forms are followed.
pub(crate) const MAX_DEPTH: usize = 8;
const MAX_STACK: usize = 64;

pub(crate) type Matrix = [f64; 6];
pub(crate) const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a` then `b`.
pub(crate) fn concat(a: Matrix, b: Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

pub(crate) fn apply(m: Matrix, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// The box around `rect` (x0, y0, x1, y1) once `m` is applied.
fn bounds(m: Matrix, [x0, y0, x1, y1]: [f64; 4]) -> [f64; 4] {
    let corners = [
        apply(m, x0, y0),
        apply(m, x1, y0),
        apply(m, x0, y1),
        apply(m, x1, y1),
    ];
    corners.iter().fold(
        [f64::MAX, f64::MAX, f64::MIN, f64::MIN],
        |[a, b, c, d], &(x, y)| [a.min(x), b.min(y), c.max(x), d.max(y)],
    )
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn inside(a: [f64; 4], area: [f64; 4]) -> bool {
    a[0] >= area[0] && a[1] >= area[1] && a[2] <= area[2] && a[3] <= area[3]
}

/// How a font's text bytes split into character codes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Codes {
    One,
    Two,
    /// A character map Panna doesn't read: codes can't be told apart.
    Unknown,
}

/// What a page needs to know about a font: how its text splits into codes, and how wide each
/// code's glyph is, in thousandths of the font size.
#[derive(Debug, Clone)]
pub(crate) struct Font {
    pub codes: Codes,
    pub widths: HashMap<u32, f64>,
    /// For codes not in `widths`; `None` when unknown.
    pub default: Option<f64>,
}

impl Font {
    fn width(&self, code: u32) -> Option<f64> {
        self.widths.get(&code).copied().or(self.default)
    }

    fn split(&self, bytes: &[u8]) -> Option<Vec<u32>> {
        match self.codes {
            Codes::One => Some(bytes.iter().map(|&b| u32::from(b)).collect()),
            Codes::Two => Some(
                bytes
                    .chunks(2)
                    .map(|c| c.iter().fold(0u32, |v, &b| (v << 8) | u32::from(b)))
                    .collect(),
            ),
            Codes::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum XObject {
    Image,
    Form { bbox: [f64; 4], matrix: Matrix },
    Other,
}

/// Fonts and pictures by their names in the content's resources.
pub(crate) trait Lookup {
    fn font(&mut self, name: &[u8]) -> Option<Font>;
    fn xobject(&mut self, name: &[u8]) -> Option<XObject>;
}

/// A form drawn where it touches a marked area: its new name in the redacted content, its
/// name in the resources, and where it's drawn (form space to page space).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FormUse {
    pub new_name: String,
    pub name: Vec<u8>,
    pub ctm: Matrix,
}

#[derive(Debug, Default)]
pub(crate) struct Redacted {
    pub content: Vec<u8>,
    /// Pictures taken out, which the resources must no longer name.
    pub images: HashSet<Vec<u8>>,
    /// Forms touching a marked area: each use, to be replaced with a redacted copy, and the
    /// original name dropped from the resources.
    pub forms: Vec<FormUse>,
    /// Whether anything was taken out at all.
    pub changed: bool,
}

// ---- Reading content ----

#[derive(Debug, Clone)]
enum Operand {
    Number(f64),
    Name(Vec<u8>),
    String(Vec<u8>),
    Array(Vec<Operand>),
    /// A dictionary, by its bytes.
    Dict(std::ops::Range<usize>),
    Other,
}

impl Operand {
    fn number(&self) -> Option<f64> {
        match self {
            Operand::Number(v) => Some(*v),
            _ => None,
        }
    }
}

struct Lexer<'a> {
    data: &'a [u8],
    pos: usize,
}

fn is_white(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

enum Token {
    Operand(Operand),
    ArrayEnd,
    DictEnd,
    Operator(std::ops::Range<usize>),
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn skip(&mut self) {
        while let Some(b) = self.peek() {
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while self.peek().is_some_and(|b| b != b'\n' && b != b'\r') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn run(&mut self) -> std::ops::Range<usize> {
        let start = self.pos;
        while self.peek().is_some_and(|b| !is_white(b) && !is_delimiter(b)) {
            self.pos += 1;
        }
        start..self.pos
    }

    fn string(&mut self) -> Option<Vec<u8>> {
        let start = self.pos;
        if self.peek()? == b'(' {
            let mut depth = 0usize;
            while let Some(b) = self.peek() {
                self.pos += 1;
                match b {
                    b'\\' => self.pos += 1,
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        } else {
            while self.peek().is_some_and(|b| b != b'>') {
                self.pos += 1;
            }
            self.pos += 1;
        }
        super::crypt::parse_string(self.data.get(start..self.pos.min(self.data.len()))?)
    }

    /// The next token, or `None` at the end (or on bytes that make no sense).
    fn token(&mut self, depth: usize) -> Option<Token> {
        self.skip();
        let b = self.peek()?;
        Some(match b {
            b'(' => Token::Operand(Operand::String(self.string()?)),
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                let start = self.pos;
                self.pos += 2;
                let mut nested = 1;
                // Dictionaries are skipped over as a whole.
                while nested > 0 {
                    match self.token(depth + 1)? {
                        Token::DictEnd => nested -= 1,
                        Token::Operand(_) | Token::ArrayEnd | Token::Operator(_) => {}
                    }
                    if depth > MAX_STACK {
                        return None;
                    }
                }
                Token::Operand(Operand::Dict(start..self.pos))
            }
            b'<' => Token::Operand(Operand::String(self.string()?)),
            b'>' if self.data.get(self.pos + 1) == Some(&b'>') => {
                self.pos += 2;
                Token::DictEnd
            }
            b'[' => {
                self.pos += 1;
                if depth > MAX_STACK {
                    return None;
                }
                let mut items = Vec::new();
                loop {
                    match self.token(depth + 1)? {
                        Token::ArrayEnd => break,
                        Token::Operand(o) => items.push(o),
                        Token::Operator(_) | Token::DictEnd => items.push(Operand::Other),
                    }
                }
                Token::Operand(Operand::Array(items))
            }
            b']' => {
                self.pos += 1;
                Token::ArrayEnd
            }
            b'/' => {
                self.pos += 1;
                let run = self.run();
                Token::Operand(Operand::Name(self.data.get(run)?.to_vec()))
            }
            b')' | b'>' | b'{' | b'}' => {
                self.pos += 1;
                Token::Operand(Operand::Other)
            }
            _ => {
                let run = self.run();
                let text = self.data.get(run.clone())?;
                if text.is_empty() {
                    self.pos += 1;
                    return Some(Token::Operand(Operand::Other));
                }
                if text
                    .iter()
                    .all(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.'))
                {
                    let v = std::str::from_utf8(text).ok()?.parse::<f64>().unwrap_or(0.0);
                    Token::Operand(Operand::Number(v))
                } else if matches!(text, b"true" | b"false" | b"null") {
                    Token::Operand(Operand::Other)
                } else {
                    Token::Operator(run)
                }
            }
        })
    }

    /// Skips an inline image's data after `ID`, up to and including `EI`.
    fn inline_image(&mut self) -> Option<()> {
        // One white-space byte follows ID.
        self.pos += 1;
        let rest = self.data.get(self.pos..)?;
        let at = rest
            .windows(4)
            .position(|w| matches!(w, [a, b'E', b'I', d] if is_white(*a) && is_white(*d)));
        let at = match at {
            Some(at) => at + 3,
            None if rest.ends_with(b"EI") => rest.len(),
            None => return None,
        };
        self.pos += at;
        Some(())
    }
}

// ---- Following the state ----

#[derive(Debug, Clone)]
struct State {
    ctm: Matrix,
    char_space: f64,
    word_space: f64,
    scale: f64,
    leading: f64,
    font: Option<Font>,
    size: f64,
    rise: f64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            ctm: IDENTITY,
            char_space: 0.0,
            word_space: 0.0,
            scale: 1.0,
            leading: 0.0,
            font: None,
            size: 0.0,
            rise: 0.0,
        }
    }
}

/// Takes what lies in `areas` (page space) out of `content`, drawn with `ctm` from content
/// space to page space. `names` makes up names for redacted copies of forms.
pub(crate) fn redact(
    content: &[u8],
    lookup: &mut dyn Lookup,
    areas: &[[f64; 4]],
    ctm: Matrix,
    text_only: bool,
    names: &mut dyn FnMut() -> String,
) -> Option<Redacted> {
    // First, which pictures and forms are hit anywhere, so that every use of them goes.
    let first = run(content, lookup, areas, ctm, None, text_only, names)?;
    let plan = (
        first.images,
        first.forms.into_iter().map(|f| f.name).collect::<HashSet<_>>(),
    );
    run(content, lookup, areas, ctm, Some(&plan), text_only, names)
}

type Plan = (HashSet<Vec<u8>>, HashSet<Vec<u8>>);

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2 + 2);
    out.push('<');
    for b in bytes {
        out.push_str(&format!("{b:02X}"));
    }
    out.push('>');
    out
}

fn num(v: f64) -> String {
    super::annot::n(v)
}

fn run(
    content: &[u8],
    lookup: &mut dyn Lookup,
    areas: &[[f64; 4]],
    ctm: Matrix,
    plan: Option<&Plan>,
    text_only: bool,
    names: &mut dyn FnMut() -> String,
) -> Option<Redacted> {
    let mut out = Redacted::default();
    let mut lex = Lexer {
        data: content,
        pos: 0,
    };
    let mut stack: Vec<State> = Vec::new();
    let mut state = State {
        ctm,
        ..State::default()
    };
    let (mut tm, mut tlm) = (IDENTITY, IDENTITY);
    let mut path: Option<[f64; 4]> = None;
    let mut operands: Vec<Operand> = Vec::new();
    let mut start = 0;
    let hit = |b: [f64; 4]| areas.iter().any(|a| overlaps(b, *a));
    let mut fonts: HashMap<Vec<u8>, Option<Font>> = HashMap::new();
    loop {
        lex.skip();
        if operands.is_empty() {
            start = lex.pos;
        }
        let Some(token) = lex.token(0) else {
            break;
        };
        let range = match token {
            Token::Operand(o) => {
                operands.push(o);
                continue;
            }
            Token::ArrayEnd | Token::DictEnd => {
                operands.push(Operand::Other);
                continue;
            }
            Token::Operator(range) => range,
        };
        let op = content.get(range.clone())?.to_vec();
        let raw = |end: usize| content.get(start..end).unwrap_or_default();
        let nums: Vec<f64> = operands.iter().filter_map(Operand::number).collect();
        let n = |i: usize| nums.get(i).copied().unwrap_or(0.0);
        // What to write instead of the operation as it was, if anything.
        let mut replace: Option<Vec<u8>> = None;
        match op.as_slice() {
            b"q" => {
                if stack.len() < MAX_STACK {
                    stack.push(state.clone());
                }
            }
            b"Q" => {
                if let Some(s) = stack.pop() {
                    state = s;
                }
            }
            b"cm" if nums.len() == 6 => {
                state.ctm = concat([n(0), n(1), n(2), n(3), n(4), n(5)], state.ctm);
            }
            b"BT" => {
                tm = IDENTITY;
                tlm = IDENTITY;
            }
            b"Tc" => state.char_space = n(0),
            b"Tw" => state.word_space = n(0),
            b"Tz" => state.scale = n(0) / 100.0,
            b"TL" => state.leading = n(0),
            b"Ts" => state.rise = n(0),
            b"Tf" => {
                if let Some(Operand::Name(name)) = operands.first() {
                    let font = fonts
                        .entry(name.clone())
                        .or_insert_with(|| lookup.font(name))
                        .clone();
                    state.font = font;
                }
                state.size = n(0);
            }
            b"Td" | b"TD" => {
                if op == b"TD" {
                    state.leading = -n(1);
                }
                tlm = concat([1.0, 0.0, 0.0, 1.0, n(0), n(1)], tlm);
                tm = tlm;
            }
            b"Tm" if nums.len() == 6 => {
                tlm = [n(0), n(1), n(2), n(3), n(4), n(5)];
                tm = tlm;
            }
            b"T*" => {
                tlm = concat([1.0, 0.0, 0.0, 1.0, 0.0, -state.leading], tlm);
                tm = tlm;
            }
            b"Tj" | b"TJ" | b"'" | b"\"" => {
                let mut prefix = String::new();
                if op == b"\"" {
                    state.word_space = n(0);
                    state.char_space = n(1);
                    prefix = format!("{} Tw {} Tc ", num(n(0)), num(n(1)));
                }
                if op == b"'" || op == b"\"" {
                    tlm = concat([1.0, 0.0, 0.0, 1.0, 0.0, -state.leading], tlm);
                    tm = tlm;
                    prefix.push_str("T* ");
                }
                let items: Vec<Operand> = match (op.as_slice(), operands.last()) {
                    (b"TJ", Some(Operand::Array(items))) => items.clone(),
                    (_, Some(s @ Operand::String(_))) => vec![s.clone()],
                    _ => Vec::new(),
                };
                let (shown, removed) = show(&items, &state, &mut tm, &hit);
                if removed {
                    out.changed = true;
                    let mut text = prefix.into_bytes();
                    text.extend_from_slice(&shown);
                    text.extend_from_slice(b" TJ");
                    replace = Some(text);
                }
            }
            b"m" | b"l" => {
                let (x, y) = apply(state.ctm, n(0), n(1));
                path = Some(grow(path, x, y));
            }
            b"c" | b"v" | b"y" => {
                for pair in nums.chunks(2) {
                    if let [x, y] = pair {
                        let (x, y) = apply(state.ctm, *x, *y);
                        path = Some(grow(path, x, y));
                    }
                }
            }
            b"re" => {
                let b = bounds(state.ctm, [n(0), n(1), n(0) + n(2), n(1) + n(3)]);
                path = Some(grow(grow(path, b[0], b[1]).into(), b[2], b[3]));
            }
            b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                let painted = op != b"n";
                if painted
                    && !text_only
                    && let Some(b) = path.take()
                {
                    // Strokes reach a little past their path; enough to count for thin lines.
                    if areas.iter().any(|a| inside(b, *a)) {
                        out.changed = true;
                        replace = Some(b"n".to_vec());
                    }
                }
                path = None;
            }
            b"Do" => {
                if let Some(Operand::Name(name)) = operands.first() {
                    match lookup.xobject(name) {
                        Some(XObject::Image) if !text_only => {
                            let b = bounds(state.ctm, [0.0, 0.0, 1.0, 1.0]);
                            let gone = match plan {
                                Some((images, _)) => images.contains(name),
                                None => hit(b),
                            };
                            if gone {
                                out.images.insert(name.clone());
                                out.changed = true;
                                replace = Some(Vec::new());
                            }
                        }
                        Some(XObject::Form { bbox, matrix }) => {
                            let at = concat(matrix, state.ctm);
                            let touched = match plan {
                                Some((_, forms)) => forms.contains(name),
                                None => hit(bounds(at, bbox)),
                            };
                            if touched {
                                out.changed = true;
                                let new_name = if plan.is_some() { names() } else { String::new() };
                                if plan.is_some() {
                                    replace = Some(format!("/{new_name} Do").into_bytes());
                                }
                                out.forms.push(FormUse {
                                    new_name,
                                    name: name.clone(),
                                    ctm: at,
                                });
                            }
                        }
                        Some(XObject::Image | XObject::Other) | None => {}
                    }
                }
            }
            b"BI" => {
                // Up to ID: the image's keys; then its data.
                loop {
                    match lex.token(0) {
                        Some(Token::Operator(r)) if content.get(r.clone()) == Some(b"ID") => break,
                        Some(_) => {}
                        None => return None,
                    }
                }
                lex.inline_image()?;
                if !text_only && hit(bounds(state.ctm, [0.0, 0.0, 1.0, 1.0])) {
                    out.changed = true;
                    replace = Some(Vec::new());
                }
            }
            b"BDC" | b"DP" => {
                if let (Some(Operand::Name(tag)), Some(Operand::Dict(dict))) =
                    (operands.first(), operands.get(1))
                {
                    let data = content.get(dict.clone())?;
                    let entries = super::lex::dict_entries(data)?;
                    if entries
                        .iter()
                        .any(|(k, _)| matches!(*k, b"ActualText" | b"Alt" | b"E"))
                    {
                        let mut text = format!("/{} <<", String::from_utf8_lossy(tag)).into_bytes();
                        for (key, value) in entries {
                            if matches!(key, b"ActualText" | b"Alt" | b"E") {
                                continue;
                            }
                            text.push(b'/');
                            text.extend_from_slice(key);
                            text.push(b' ');
                            text.extend_from_slice(data.get(value)?);
                            text.push(b' ');
                        }
                        text.extend_from_slice(b">> ");
                        text.extend_from_slice(&op);
                        out.changed = true;
                        replace = Some(text);
                    }
                }
            }
            _ => {}
        }
        match replace {
            Some(text) => {
                out.content.extend_from_slice(&text);
                out.content.push(b'\n');
            }
            None => {
                out.content.extend_from_slice(raw(lex.pos));
                out.content.push(b'\n');
            }
        }
        operands.clear();
    }
    Some(out)
}

fn grow(b: Option<[f64; 4]>, x: f64, y: f64) -> [f64; 4] {
    match b {
        Some([a, c, d, e]) => [a.min(x), c.min(y), d.max(x), e.max(y)],
        None => [x, y, x, y],
    }
}

/// Shows `items` (strings and adjustments) as a TJ array with the glyphs that touch a marked
/// area replaced by the space they took; moves `tm` along. Also says whether any went.
fn show(
    items: &[Operand],
    state: &State,
    tm: &mut Matrix,
    hit: &dyn Fn([f64; 4]) -> bool,
) -> (Vec<u8>, bool) {
    let size = state.size;
    let th = state.scale;
    let font = state.font.as_ref();
    let mut out = b"[".to_vec();
    let mut removed = false;
    let mut pending = 0.0;
    let flush = |out: &mut Vec<u8>, pending: &mut f64| {
        if *pending != 0.0 {
            out.extend_from_slice(num(*pending).as_bytes());
            out.push(b' ');
            *pending = 0.0;
        }
    };
    for item in items {
        match item {
            Operand::Number(adj) => {
                *tm = concat([1.0, 0.0, 0.0, 1.0, -adj / 1000.0 * size * th, 0.0], *tm);
                pending += adj;
            }
            Operand::String(bytes) => {
                let codes = font.and_then(|f| f.split(bytes));
                let widths: Option<Vec<f64>> = codes
                    .as_ref()
                    .and_then(|c| c.iter().map(|&code| font?.width(code)).collect());
                let render = |tm: Matrix| {
                    concat(
                        concat([size * th, 0.0, 0.0, size, 0.0, state.rise], tm),
                        state.ctm,
                    )
                };
                match (codes, widths) {
                    (Some(codes), Some(widths)) => {
                        let mut kept = Vec::new();
                        let one_byte = font.is_some_and(|f| f.codes == Codes::One);
                        for (code, w) in codes.iter().zip(widths) {
                            let w0 = w / 1000.0;
                            let b = bounds(render(*tm), [0.0, DESCENT, w0.max(0.0), ASCENT]);
                            let space = if one_byte && *code == 32 {
                                state.word_space
                            } else {
                                0.0
                            };
                            let advance = (w0 * size + state.char_space + space) * th;
                            let gone = hit(b) || size == 0.0 && areas_point(hit, render(*tm));
                            if gone {
                                removed = true;
                                if !kept.is_empty() {
                                    flush(&mut out, &mut pending);
                                    out.extend_from_slice(hex(&kept).as_bytes());
                                    out.push(b' ');
                                    kept.clear();
                                }
                                if size != 0.0 && th != 0.0 {
                                    pending -= advance / (size * th) * 1000.0;
                                }
                            } else {
                                flush(&mut out, &mut pending);
                                let bytes = if one_byte {
                                    vec![*code as u8]
                                } else {
                                    vec![(*code >> 8) as u8, *code as u8]
                                };
                                kept.extend_from_slice(&bytes);
                            }
                            *tm = concat([1.0, 0.0, 0.0, 1.0, advance, 0.0], *tm);
                        }
                        if !kept.is_empty() {
                            flush(&mut out, &mut pending);
                            out.extend_from_slice(hex(&kept).as_bytes());
                            out.push(b' ');
                        }
                    }
                    _ => {
                        // Widths unknown: all of the string goes if any of it might touch.
                        let count = bytes.len() as f64;
                        let b = bounds(render(*tm), [0.0, DESCENT, count * GUESS_BOX, ASCENT]);
                        let advance = (GUESS_ADVANCE * size + state.char_space) * th * count;
                        if hit(b) {
                            removed = true;
                            if size != 0.0 && th != 0.0 {
                                pending -= advance / (size * th) * 1000.0;
                            }
                        } else {
                            flush(&mut out, &mut pending);
                            out.extend_from_slice(hex(bytes).as_bytes());
                            out.push(b' ');
                        }
                        *tm = concat([1.0, 0.0, 0.0, 1.0, advance, 0.0], *tm);
                    }
                }
            }
            _ => {}
        }
    }
    flush(&mut out, &mut pending);
    out.push(b']');
    (out, removed)
}

/// For zero-size text: whether its point lies in a marked area.
fn areas_point(hit: &dyn Fn([f64; 4]) -> bool, m: Matrix) -> bool {
    let (x, y) = apply(m, 0.0, 0.0);
    hit([x - 0.01, y - 0.01, x + 0.01, y + 0.01])
}

// ---- Pictures drawn on a page ----

/// A picture drawn by the content: its name in the resources, where it's drawn (picture
/// space to page space), and the bytes of the operation that draws it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Placed {
    pub name: Vec<u8>,
    pub ctm: Matrix,
    pub range: std::ops::Range<usize>,
}

impl Placed {
    /// The box it covers, in page space.
    pub(crate) fn bounds(&self) -> [f64; 4] {
        bounds(self.ctm, [0.0, 0.0, 1.0, 1.0])
    }
}

/// Every picture `content` draws itself (not those inside forms), in order.
pub(crate) fn placed_images(content: &[u8], lookup: &mut dyn Lookup) -> Vec<Placed> {
    placed(content, lookup, |x| matches!(x, XObject::Image))
}

/// Every form `content` draws itself, in order.
pub(crate) fn placed_forms(content: &[u8], lookup: &mut dyn Lookup) -> Vec<Placed> {
    placed(content, lookup, |x| matches!(x, XObject::Form { .. }))
}

fn placed(content: &[u8], lookup: &mut dyn Lookup, wanted: fn(XObject) -> bool) -> Vec<Placed> {
    let mut lex = Lexer {
        data: content,
        pos: 0,
    };
    let mut stack: Vec<Matrix> = Vec::new();
    let mut ctm = IDENTITY;
    let mut operands: Vec<Operand> = Vec::new();
    let mut start = 0;
    let mut out = Vec::new();
    loop {
        lex.skip();
        if operands.is_empty() {
            start = lex.pos;
        }
        let Some(token) = lex.token(0) else { break };
        let range = match token {
            Token::Operand(o) => {
                operands.push(o);
                continue;
            }
            Token::ArrayEnd | Token::DictEnd => {
                operands.push(Operand::Other);
                continue;
            }
            Token::Operator(range) => range,
        };
        let nums: Vec<f64> = operands.iter().filter_map(Operand::number).collect();
        match content.get(range.clone()).unwrap_or_default() {
            b"q" if stack.len() < MAX_STACK => stack.push(ctm),
            b"Q" => ctm = stack.pop().unwrap_or(ctm),
            b"cm" => {
                if let [a, b, c, d, e, f] = nums[..] {
                    ctm = concat([a, b, c, d, e, f], ctm);
                }
            }
            b"Do" => {
                if let Some(Operand::Name(name)) = operands.first()
                    && lookup.xobject(name).is_some_and(wanted)
                {
                    out.push(Placed {
                        name: name.clone(),
                        ctm,
                        range: start..range.end,
                    });
                }
            }
            _ => {}
        }
        operands.clear();
    }
    out
}

/// Rectangles and straight lines `content` paints itself (not inside forms), in page space:
/// the boxes of `re` rectangles, and each straight segment as `[x0, y0, x1, y1]`.
pub(crate) fn drawn_shapes(content: &[u8]) -> (Vec<[f64; 4]>, Vec<[f64; 4]>) {
    /// Shapes kept at most, so a page of hatching stays cheap.
    const MAX_SHAPES: usize = 20_000;
    let mut lex = Lexer {
        data: content,
        pos: 0,
    };
    let mut stack: Vec<Matrix> = Vec::new();
    let mut ctm = IDENTITY;
    let mut operands: Vec<Operand> = Vec::new();
    let (mut rects, mut lines) = (Vec::new(), Vec::new());
    let (mut path_rects, mut path_lines) = (Vec::new(), Vec::new());
    let mut at: Option<(f64, f64)> = None;
    let mut start: Option<(f64, f64)> = None;
    loop {
        lex.skip();
        let Some(token) = lex.token(0) else { break };
        let range = match token {
            Token::Operand(o) => {
                operands.push(o);
                continue;
            }
            Token::ArrayEnd | Token::DictEnd => {
                operands.push(Operand::Other);
                continue;
            }
            Token::Operator(range) => range,
        };
        let nums: Vec<f64> = operands.iter().filter_map(Operand::number).collect();
        match content.get(range).unwrap_or_default() {
            b"q" if stack.len() < MAX_STACK => stack.push(ctm),
            b"Q" => ctm = stack.pop().unwrap_or(ctm),
            b"cm" => {
                if let [a, b, c, d, e, f] = nums[..] {
                    ctm = concat([a, b, c, d, e, f], ctm);
                }
            }
            b"m" => {
                if let [x, y] = nums[..] {
                    at = Some(apply(ctm, x, y));
                    start = at;
                }
            }
            b"l" => {
                if let ([x, y], Some(from)) = (&nums[..], at) {
                    let to = apply(ctm, *x, *y);
                    path_lines.push([from.0, from.1, to.0, to.1]);
                    at = Some(to);
                }
            }
            b"h" => {
                if let (Some(from), Some(to)) = (at, start) {
                    path_lines.push([from.0, from.1, to.0, to.1]);
                    at = start;
                }
            }
            b"c" | b"v" | b"y" => {
                // Curves aren't fields' edges; carry on from where they end.
                if let [.., x, y] = nums[..] {
                    at = Some(apply(ctm, x, y));
                }
            }
            b"re" => {
                if let [x, y, w, h] = nums[..] {
                    path_rects.push(bounds(ctm, [x, y, x + w, y + h]));
                }
            }
            b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" => {
                if rects.len() + lines.len() < MAX_SHAPES {
                    rects.append(&mut path_rects);
                    lines.append(&mut path_lines);
                }
                path_rects.clear();
                path_lines.clear();
                at = None;
            }
            b"n" => {
                path_rects.clear();
                path_lines.clear();
                at = None;
            }
            _ => {}
        }
        operands.clear();
    }
    (rects, lines)
}

/// A run of text in content: the bytes from `BT` to the end of `ET`, the largest size its
/// text is shown at (in page space, points), and how many bytes of text it shows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextBlock {
    pub range: std::ops::Range<usize>,
    pub size: f64,
    pub shown: usize,
}

/// The text objects `content` shows text in, in order; or `None` when it's already marked
/// up (it has marked content of its own) or can't be read.
pub(crate) fn text_blocks(content: &[u8]) -> Option<Vec<TextBlock>> {
    let mut lex = Lexer {
        data: content,
        pos: 0,
    };
    let mut stack: Vec<(Matrix, f64)> = Vec::new();
    let (mut ctm, mut font_size) = (IDENTITY, 0.0);
    let (mut tm, mut tlm) = (IDENTITY, IDENTITY);
    let mut operands: Vec<Operand> = Vec::new();
    let mut out = Vec::new();
    // The current text object's start, the largest size shown in it so far, and the bytes
    // of text shown.
    let mut open: Option<(usize, f64, usize)> = None;
    let mut start = 0;
    let scale = |m: Matrix| (m[0] * m[3] - m[1] * m[2]).abs().sqrt();
    loop {
        lex.skip();
        if operands.is_empty() {
            start = lex.pos;
        }
        let Some(token) = lex.token(0) else { break };
        let range = match token {
            Token::Operand(o) => {
                operands.push(o);
                continue;
            }
            Token::ArrayEnd | Token::DictEnd => {
                operands.push(Operand::Other);
                continue;
            }
            Token::Operator(range) => range,
        };
        let nums: Vec<f64> = operands.iter().filter_map(Operand::number).collect();
        match content.get(range)? {
            b"BDC" | b"BMC" => return None,
            b"q" if stack.len() < MAX_STACK => stack.push((ctm, font_size)),
            b"Q" => (ctm, font_size) = stack.pop().unwrap_or((ctm, font_size)),
            b"cm" => {
                if let [a, b, c, d, e, f] = nums[..] {
                    ctm = concat([a, b, c, d, e, f], ctm);
                }
            }
            b"BT" => {
                (tm, tlm) = (IDENTITY, IDENTITY);
                // A text object inside another isn't allowed; the outer one stands.
                if open.is_none() {
                    open = Some((start, 0.0, 0));
                }
            }
            b"ET" => {
                if let Some((from, size, shown)) = open.take()
                    && size > 0.0
                {
                    out.push(TextBlock {
                        range: from..lex.pos,
                        size,
                        shown,
                    });
                }
            }
            b"Tf" => {
                if let Some(v) = nums.first() {
                    font_size = v.abs();
                }
            }
            b"Tm" => {
                if let [a, b, c, d, e, f] = nums[..] {
                    tm = [a, b, c, d, e, f];
                    tlm = tm;
                }
            }
            b"Td" | b"TD" => {
                if let [x, y] = nums[..] {
                    tlm = concat([1.0, 0.0, 0.0, 1.0, x, y], tlm);
                    tm = tlm;
                }
            }
            b"Tj" | b"TJ" | b"'" | b"\"" => {
                if let Some((_, size, shown)) = open.as_mut() {
                    let at = font_size * scale(tm) * scale(ctm);
                    if at.is_finite() {
                        *size = size.max(at.max(0.01));
                    }
                    *shown += operands
                        .iter()
                        .map(|o| match o {
                            Operand::String(s) => s.len(),
                            Operand::Array(items) => items
                                .iter()
                                .map(|i| match i {
                                    Operand::String(s) => s.len(),
                                    _ => 0,
                                })
                                .sum(),
                            _ => 0,
                        })
                        .sum::<usize>();
                }
            }
            b"ID" => lex.inline_image()?,
            _ => {}
        }
        operands.clear();
    }
    Some(out)
}

/// The inverse of `m`, unless it squashes everything flat.
pub(crate) fn invert(m: Matrix) -> Option<Matrix> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
}

/// The matrix that, put in front of a picture drawn with `ctm`, moves it from the box
/// `from` to the box `to` (both in page space).
pub(crate) fn moved(ctm: Matrix, from: [f64; 4], to: [f64; 4]) -> Option<Matrix> {
    let (fw, fh) = (from[2] - from[0], from[3] - from[1]);
    if fw.abs() < 1e-9 || fh.abs() < 1e-9 {
        return None;
    }
    let (sx, sy) = ((to[2] - to[0]) / fw, (to[3] - to[1]) / fh);
    let shift = [sx, 0.0, 0.0, sy, to[0] - from[0] * sx, to[1] - from[1] * sy];
    Some(concat(concat(ctm, shift), invert(ctm)?))
}

// ---- Reading fonts and pictures from a file ----

use hayro::hayro_syntax::object::{Array, Dict, Name, Object, Stream};

/// Fonts and pictures from a resources dictionary in the file.
pub(crate) struct Resources<'a> {
    pub dict: Option<Dict<'a>>,
}

impl<'a> Resources<'a> {
    /// The form or picture named `name`, with its object.
    pub(crate) fn stream(&self, name: &[u8]) -> Option<(Stream<'a>, Option<crate::ObjRef>)> {
        let xobjects = self.dict.as_ref()?.get::<Dict<'a>>(b"XObject")?;
        let stream = xobjects.get::<Stream<'a>>(name)?;
        let r = xobjects
            .get_ref(name)
            .and_then(|r| crate::ObjRef::new(r.obj_number, r.gen_number));
        Some((stream, r))
    }
}

/// The widths in a CID font's /W array: `c [w1 w2 ...]` and `first last w` runs.
fn cid_widths(w: &Array<'_>) -> HashMap<u32, f64> {
    const MAX: usize = 70_000;
    let mut out = HashMap::new();
    let items: Vec<Object<'_>> = w.iter::<Object<'_>>().take(MAX).collect();
    let number = |o: Option<&Object<'_>>| match o {
        Some(Object::Number(n)) => Some(n.as_f64()),
        _ => None,
    };
    let mut i = 0;
    while i < items.len() && out.len() < MAX {
        let Some(first) = number(items.get(i)) else { break };
        let first = first.max(0.0) as u32;
        match items.get(i + 1) {
            Some(Object::Array(list)) => {
                for (k, v) in list.iter::<f64>().take(MAX).enumerate() {
                    out.insert(first.saturating_add(k as u32), v);
                }
                i += 2;
            }
            _ => {
                let (Some(last), Some(v)) = (number(items.get(i + 1)), number(items.get(i + 2))) else {
                    break;
                };
                let last = (last.max(0.0) as u32).min(first.saturating_add(MAX as u32));
                for c in first..=last {
                    out.insert(c, v);
                }
                i += 3;
            }
        }
    }
    out
}

pub(crate) fn font_from(dict: &Dict<'_>) -> Option<Font> {
    let subtype = dict
        .get::<Name<'_>>(b"Subtype")
        .map(|n| n.as_str().to_owned())
        .unwrap_or_default();
    match subtype.as_str() {
        "Type0" => {
            let encoding = dict.get::<Name<'_>>(b"Encoding").map(|n| n.as_str().to_owned());
            let codes = match encoding.as_deref() {
                Some("Identity-H" | "Identity-V") => Codes::Two,
                _ => Codes::Unknown,
            };
            let descendant = dict
                .get::<Array<'_>>(b"DescendantFonts")?
                .iter::<Dict<'_>>()
                .next()?;
            Some(Font {
                codes,
                widths: descendant
                    .get::<Array<'_>>(b"W")
                    .map(|w| cid_widths(&w))
                    .unwrap_or_default(),
                default: Some(descendant.get::<f64>(b"DW").unwrap_or(1000.0)),
            })
        }
        // Type 3 glyphs are measured in their own units; their widths aren't worked out.
        "Type3" => Some(Font {
            codes: Codes::One,
            widths: HashMap::new(),
            default: None,
        }),
        _ => {
            if let Some(widths) = dict.get::<Array<'_>>(b"Widths") {
                let first = dict.get::<f64>(b"FirstChar").unwrap_or(0.0).max(0.0) as u32;
                let missing = dict
                    .get::<Dict<'_>>(b"FontDescriptor")
                    .and_then(|d| d.get::<f64>(b"MissingWidth"))
                    .unwrap_or(0.0);
                return Some(Font {
                    codes: Codes::One,
                    widths: widths
                        .iter::<f64>()
                        .take(256)
                        .enumerate()
                        .map(|(i, w)| (first + i as u32, w))
                        .collect(),
                    default: Some(missing),
                });
            }
            // A standard font without widths: the ones whose widths are known.
            let base = dict
                .get::<Name<'_>>(b"BaseFont")
                .map(|n| n.as_str().to_owned())
                .unwrap_or_default();
            let base = base.split_once('+').map_or(base.as_str(), |(_, b)| b);
            let (widths, default) = if base.starts_with("Courier") {
                (HashMap::new(), Some(600.0))
            } else if matches!(base, "Helvetica" | "Helvetica-Oblique" | "Arial" | "ArialMT") {
                let table = super::annot::HELVETICA
                    .iter()
                    .enumerate()
                    .map(|(i, w)| (32 + i as u32, f64::from(*w)))
                    .collect();
                (table, None)
            } else {
                (HashMap::new(), None)
            };
            Some(Font {
                codes: Codes::One,
                widths,
                default,
            })
        }
    }
}

fn numbers<const N: usize>(array: Option<Array<'_>>) -> Option<[f64; N]> {
    let values: Vec<f64> = array?.iter::<f64>().take(N).collect();
    values.try_into().ok()
}

impl Lookup for Resources<'_> {
    fn font(&mut self, name: &[u8]) -> Option<Font> {
        font_from(
            &self
                .dict
                .as_ref()?
                .get::<Dict<'_>>(b"Font")?
                .get::<Dict<'_>>(name)?,
        )
    }

    fn xobject(&mut self, name: &[u8]) -> Option<XObject> {
        let (stream, _) = self.stream(name)?;
        let dict = stream.dict();
        Some(
            match dict
                .get::<Name<'_>>(b"Subtype")
                .map(|n| n.as_str().to_owned())
                .as_deref()
            {
                Some("Image") => XObject::Image,
                Some("Form") => {
                    let bbox = numbers::<4>(dict.get::<Array<'_>>(b"BBox"))?;
                    let bbox = [
                        bbox[0].min(bbox[2]),
                        bbox[1].min(bbox[3]),
                        bbox[0].max(bbox[2]),
                        bbox[1].max(bbox[3]),
                    ];
                    XObject::Form {
                        bbox,
                        matrix: numbers::<6>(dict.get::<Array<'_>>(b"Matrix")).unwrap_or(IDENTITY),
                    }
                }
                _ => XObject::Other,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;

    impl Lookup for Fake {
        fn font(&mut self, name: &[u8]) -> Option<Font> {
            match name {
                // Every glyph 500 wide.
                b"F1" => Some(Font {
                    codes: Codes::One,
                    widths: HashMap::new(),
                    default: Some(500.0),
                }),
                b"F2" => Some(Font {
                    codes: Codes::Unknown,
                    widths: HashMap::new(),
                    default: None,
                }),
                _ => None,
            }
        }

        fn xobject(&mut self, name: &[u8]) -> Option<XObject> {
            match name {
                b"Im1" => Some(XObject::Image),
                b"Fm1" => Some(XObject::Form {
                    bbox: [0.0, 0.0, 10.0, 10.0],
                    matrix: IDENTITY,
                }),
                _ => None,
            }
        }
    }

    fn go(content: &str, areas: &[[f64; 4]]) -> Redacted {
        let mut count = 0;
        let mut names = || {
            count += 1;
            format!("R{count}")
        };
        redact(content.as_bytes(), &mut Fake, areas, IDENTITY, false, &mut names).unwrap()
    }

    fn text(r: &Redacted) -> String {
        String::from_utf8_lossy(&r.content).into_owned()
    }

    #[test]
    fn glyphs_in_the_area_go_and_the_rest_keep_their_place() {
        // Size 10: each glyph 5 wide, from x = 100. "SECRET" is glyphs 4 to 9: x 120 to 150.
        let r = go(
            "BT /F1 10 Tf 100 700 Td (Our SECRET plan) Tj ET",
            &[[120.0, 695.0, 150.0, 710.0]],
        );
        assert!(r.changed);
        let t = text(&r);
        // "Our " stays, then 6 glyphs' room (6 x 500), then " plan".
        assert!(t.contains("[<4F757220> -3000 <20706C616E> ] TJ"), "{t}");
        assert!(!t.contains("SECRET") && !t.contains("534543524554"));
    }

    #[test]
    fn text_elsewhere_is_left_as_it_was() {
        let content = "BT /F1 10 Tf 100 700 Td (Nothing here) Tj ET";
        let r = go(content, &[[0.0, 0.0, 50.0, 50.0]]);
        assert!(!r.changed);
        assert!(text(&r).contains("(Nothing here) Tj"));
    }

    #[test]
    fn positions_follow_matrices_and_kerning() {
        // Scaled by two: glyphs 10 wide from x = 200; a -1000 adjustment moves 20 along.
        let r = go(
            "q 2 0 0 2 0 0 cm BT /F1 10 Tf 100 350 Td [(AB) -1000 (CD)] TJ ET Q",
            &[[252.0, 695.0, 258.0, 720.0]],
        );
        // A at 200, B at 210, then the gap to 240, C at 240, D at 250: D goes.
        assert!(text(&r).contains("[<4142> -1000 <43> -500 ] TJ"), "{}", text(&r));
    }

    #[test]
    fn unreadable_fonts_lose_the_whole_string() {
        let r = go(
            "BT /F2 10 Tf 100 700 Td (abc) Tj ET",
            &[[101.0, 701.0, 102.0, 702.0]],
        );
        assert!(r.changed && !text(&r).contains("abc") && !text(&r).contains("616263"));
    }

    #[test]
    fn pictures_go_everywhere_they_appear_once_one_is_hit() {
        let r = go(
            "q 50 0 0 50 0 0 cm /Im1 Do Q q 50 0 0 50 300 300 cm /Im1 Do Q",
            &[[10.0, 10.0, 20.0, 20.0]],
        );
        assert!(r.images.iter().any(|i| i == b"Im1"));
        assert!(!text(&r).contains("Im1"));
    }

    #[test]
    fn forms_that_are_touched_are_renamed_for_copies() {
        let r = go("/Fm1 Do q 1 0 0 1 100 100 cm /Fm1 Do Q", &[[5.0, 5.0, 6.0, 6.0]]);
        assert_eq!(r.forms.len(), 2);
        assert_eq!(r.forms[1].ctm, [1.0, 0.0, 0.0, 1.0, 100.0, 100.0]);
        let t = text(&r);
        assert!(
            t.contains("/R1 Do") && t.contains("/R2 Do") && !t.contains("Fm1"),
            "{t}"
        );
    }

    #[test]
    fn drawings_inside_go_and_crossing_ones_stay() {
        let r = go("10 10 5 5 re f 0 0 m 500 500 l S", &[[0.0, 0.0, 100.0, 100.0]]);
        let t = text(&r);
        assert!(t.starts_with("10 10 5 5 re\nn\n"), "{t}");
        assert!(t.contains("500 500 l\nS"));
    }

    #[test]
    fn inline_images_and_replacement_text_go() {
        let r = go(
            "/Span << /ActualText (secret) /MCID 3 >> BDC EMC q 20 0 0 20 0 0 cm BI /W 1 /H 1 /CS /G /BPC 8 ID \x01 EI Q",
            &[[1.0, 1.0, 2.0, 2.0]],
        );
        let t = text(&r);
        assert!(
            !t.contains("secret") && t.contains("/MCID 3") && !t.contains("BI"),
            "{t}"
        );
    }

    #[test]
    fn pictures_are_found_and_moved_where_asked() {
        let content = b"q 50 0 0 40 10 20 cm /Im1 Do Q BT /F1 12 Tf (x) Tj ET q 2 0 0 2 0 0 cm 10 0 0 10 5 5 cm /Im1 Do Q";
        let found = placed_images(content, &mut Fake);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].bounds(), [10.0, 20.0, 60.0, 60.0]);
        assert_eq!(found[1].bounds(), [10.0, 10.0, 30.0, 30.0]);
        assert_eq!(&content[found[0].range.clone()], b"/Im1 Do");
        let m = moved(found[1].ctm, found[1].bounds(), [100.0, 100.0, 140.0, 120.0]).unwrap();
        let b = bounds(concat(m, found[1].ctm), [0.0, 0.0, 1.0, 1.0]);
        for (x, y) in b.iter().zip([100.0, 100.0, 140.0, 120.0]) {
            assert!((x - y).abs() < 1e-9, "{b:?}");
        }
    }
}
