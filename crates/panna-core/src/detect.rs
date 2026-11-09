//! Finding where a form's fields should go on pages that have none, the way a person would:
//! blanks of underscores, empty boxes, lines to write on, and box characters to tick. Each
//! is named after the words beside it.

use crate::{NewField, NewFieldKind, PageText, TextBox};

/// Fields found on one page at most.
const MAX_PER_PAGE: usize = 300;
/// Underscores in a row that make a blank.
const MIN_UNDERSCORES: usize = 3;
/// The narrowest blank, line or box a text field goes on, in points.
const MIN_WIDTH: f32 = 36.0;
/// How tall a field on a line is, in points.
const LINE_FIELD_HEIGHT: f32 = 16.0;
/// A line no thicker than this is one to write on.
const MAX_LINE_THICKNESS: f32 = 1.5;
/// Square boxes this size are check boxes.
const CHECK_SIDE: (f32, f32) = (6.0, 24.0);
/// Boxes taller than this aren't fields: they frame the page or a section.
const MAX_BOX_HEIGHT: f32 = 160.0;
/// Boxes this tall or more take several lines.
const MULTILINE_HEIGHT: f32 = 36.0;
/// How far a label goes looking for words, in points.
const LABEL_REACH: f32 = 200.0;
const MAX_LABEL: usize = 60;

const BOX_CHARS: [char; 6] = ['☐', '□', '▢', '❏', '❑', '◻'];

/// What a page has: its text, the rectangles and lines it draws (as shown), and the fields
/// already on it.
pub(crate) struct Page<'a> {
    pub index: usize,
    pub text: &'a PageText,
    pub rects: &'a [TextBox],
    pub lines: &'a [TextBox],
    pub existing: &'a [TextBox],
}

fn overlap(a: &TextBox, b: &TextBox) -> f32 {
    let w = a.x1.min(b.x1) - a.x0.max(b.x0);
    let h = a.y1.min(b.y1) - a.y0.max(b.y0);
    if w <= 0.0 || h <= 0.0 { 0.0 } else { w * h }
}

fn area(b: &TextBox) -> f32 {
    b.width().max(0.0) * b.height().max(0.0)
}

/// Whether `a` and `b` cover much of the same ground.
fn clashes(a: &TextBox, b: &TextBox) -> bool {
    overlap(a, b) > 0.3 * area(a).min(area(b)).max(f32::EPSILON)
}

pub(crate) fn detect(page: &Page<'_>) -> Vec<NewField> {
    let chars = page.text.chars();
    let boxes = page.text.boxes();
    let inked = |i: usize| {
        let c = chars.get(i).copied().unwrap_or(' ');
        let b = boxes.get(i).copied().unwrap_or_default();
        (!c.is_whitespace() && b.width() > 0.0 && b.height() > 0.0).then_some((c, b))
    };
    // Whether words sit inside `rect` (underscores and box characters don't count).
    let has_words = |rect: &TextBox| {
        (0..chars.len()).filter_map(inked).any(|(c, b)| {
            c != '_' && !BOX_CHARS.contains(&c) && rect.contains((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0)
        })
    };

    let mut found: Vec<(TextBox, NewFieldKind)> = Vec::new();
    let push = |rect: TextBox, kind: NewFieldKind, found: &mut Vec<(TextBox, NewFieldKind)>| {
        if found.len() < MAX_PER_PAGE
            && !page.existing.iter().any(|e| clashes(e, &rect))
            && !found.iter().any(|(f, _)| clashes(f, &rect))
        {
            found.push((rect, kind));
        }
    };

    // Blanks of underscores, and box characters.
    let mut i = 0;
    while i < chars.len() {
        match inked(i) {
            Some(('_', first)) => {
                let mut run = first;
                let mut n = 1;
                while let Some(('_', b)) = inked(i + n) {
                    run = run.union(b);
                    n += 1;
                }
                if n >= MIN_UNDERSCORES && run.width() >= MIN_WIDTH / 2.0 {
                    // Room to write, standing on the blank.
                    let rect = TextBox {
                        y0: run.y0.min(run.y1 - LINE_FIELD_HEIGHT),
                        ..run
                    };
                    push(rect, NewFieldKind::Text { multiline: false }, &mut found);
                }
                i += n;
                continue;
            }
            Some((c, b)) if BOX_CHARS.contains(&c) => {
                let side = b.width().min(b.height());
                let (cx, cy) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
                let square = TextBox {
                    x0: cx - side / 2.0,
                    y0: cy - side / 2.0,
                    x1: cx + side / 2.0,
                    y1: cy + side / 2.0,
                };
                push(square, NewFieldKind::CheckBox, &mut found);
            }
            _ => {}
        }
        i += 1;
    }

    // Empty boxes: small squares to tick, wider ones to write in.
    for r in page.rects {
        let (w, h) = (r.width(), r.height());
        let square = (w - h).abs() <= 0.2 * w.max(h);
        if square && (CHECK_SIDE.0..=CHECK_SIDE.1).contains(&w) {
            if !has_words(r) {
                push(*r, NewFieldKind::CheckBox, &mut found);
            }
        } else if w >= MIN_WIDTH && (CHECK_SIDE.0 * 1.5..=MAX_BOX_HEIGHT).contains(&h) && !has_words(r) {
            push(
                *r,
                NewFieldKind::Text {
                    multiline: h >= MULTILINE_HEIGHT,
                },
                &mut found,
            );
        }
    }

    // Lines to write on: the field sits just above.
    for l in page.lines {
        if l.height() > MAX_LINE_THICKNESS || l.width() < MIN_WIDTH {
            continue;
        }
        let rect = TextBox {
            x0: l.x0,
            y0: l.y0 - LINE_FIELD_HEIGHT,
            x1: l.x1,
            y1: l.y0,
        };
        if !has_words(&rect) {
            push(rect, NewFieldKind::Text { multiline: false }, &mut found);
        }
    }

    // Top to bottom, then left to right, as they'd be filled in.
    found.sort_by(|(a, _), (b, _)| {
        let same_row = (a.y0 + a.y1) / 2.0 - (b.y0 + b.y1) / 2.0;
        if same_row.abs() < a.height().min(b.height()) / 2.0 {
            a.x0.total_cmp(&b.x0)
        } else {
            a.y0.total_cmp(&b.y0)
        }
    });
    let mut text_count = 0;
    let mut check_count = 0;
    found
        .into_iter()
        .map(|(rect, kind)| {
            let check = kind == NewFieldKind::CheckBox;
            let label = label(page.text, &rect, check);
            let name = if label.is_empty() {
                if check {
                    check_count += 1;
                    format!("Check box {check_count}")
                } else {
                    text_count += 1;
                    format!("Text {text_count}")
                }
            } else {
                label
            };
            NewField {
                page: page.index,
                rect,
                name,
                tooltip: String::new(),
                required: false,
                kind,
            }
        })
        .collect()
}

/// The words beside a field on its line: before it for a text field, after it for a check
/// box (or before, when nothing follows).
fn label(text: &PageText, rect: &TextBox, check: bool) -> String {
    let chars = text.chars();
    let boxes = text.boxes();
    let on_line = |b: &TextBox| {
        let h = b.y1.min(rect.y1) - b.y0.max(rect.y0);
        b.width() > 0.0 && h > 0.4 * b.height().min(rect.height()).max(1.0)
    };
    let word = |c: char| !c.is_whitespace() && c != '_' && !BOX_CHARS.contains(&c);
    // The nearest word character on the line, left or right of the field.
    let nearest = |left: bool| {
        (0..chars.len())
            .filter(|&i| chars.get(i).copied().is_some_and(word))
            .filter_map(|i| Some((i, *boxes.get(i)?)))
            .filter(|(_, b)| on_line(b))
            .filter_map(|(i, b)| {
                let gap = if left { rect.x0 - b.x1 } else { b.x0 - rect.x1 };
                (-1.0..=LABEL_REACH).contains(&gap).then_some((i, gap))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    let gather = |at: usize, back: bool| {
        let mut picked = vec![at];
        let mut last = boxes.get(at).copied().unwrap_or_default();
        let mut i = at;
        while picked.len() < MAX_LABEL {
            let next = if back { i.checked_sub(1) } else { Some(i + 1) };
            let Some(n) = next else { break };
            let (Some(&c), Some(&b)) = (chars.get(n), boxes.get(n)) else {
                break;
            };
            if c == '\n' || c == '_' || BOX_CHARS.contains(&c) {
                break;
            }
            if b.width() > 0.0 {
                let gap = if back { last.x0 - b.x1 } else { b.x0 - last.x1 };
                // A wide gap, or a jump to another line, ends the label.
                if !on_line(&b) || gap > 2.0 * last.height().max(4.0) || gap < -2.0 * last.width().max(2.0) {
                    break;
                }
                // Words before a colon earlier on the line belong to another field.
                if back && c == ':' {
                    break;
                }
                last = b;
            }
            picked.push(n);
            i = n;
        }
        if back {
            picked.reverse();
        }
        let s: String = picked.iter().filter_map(|&i| chars.get(i)).collect();
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let found = if check {
        nearest(false)
            .map(|i| gather(i, false))
            .or_else(|| nearest(true).map(|i| gather(i, true)))
    } else {
        nearest(true).map(|i| gather(i, true))
    };
    let found = found.filter(|s| !s.is_empty()).or_else(|| above(text, rect));
    found
        .unwrap_or_default()
        .trim_end_matches([':', ' ', '.', '*'])
        .trim()
        .to_owned()
}

/// The line of words just above a field, over its left part: a box's heading.
fn above(text: &PageText, rect: &TextBox) -> Option<String> {
    let chars = text.chars();
    let boxes = text.boxes();
    let candidates: Vec<usize> = (0..chars.len())
        .filter(|&i| chars.get(i).is_some_and(|c| !c.is_whitespace() && *c != '_'))
        .filter(|&i| {
            boxes.get(i).is_some_and(|b| {
                let gap = rect.y0 - b.y1;
                b.width() > 0.0
                    && (-2.0..=1.5 * b.height().max(8.0)).contains(&gap)
                    && b.x0 >= rect.x0 - 10.0
                    && b.x1 <= rect.x1
            })
        })
        .collect();
    let lowest = candidates
        .iter()
        .filter_map(|&i| boxes.get(i))
        .min_by(|a, b| (rect.y0 - a.y1).total_cmp(&(rect.y0 - b.y1)))?;
    let line: Vec<usize> = candidates
        .into_iter()
        .filter(|&i| {
            boxes.get(i).is_some_and(|b| {
                b.y1.min(lowest.y1) - b.y0.max(lowest.y0) > 0.5 * b.height().min(lowest.height())
            })
        })
        .collect();
    let (first, last) = (*line.first()?, *line.last()?);
    let s: String = chars.get(first..=last)?.iter().take(MAX_LABEL).collect();
    Some(s.split_whitespace().collect::<Vec<_>>().join(" "))
}
