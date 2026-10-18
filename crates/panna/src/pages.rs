//! Page tools in the Pages tab: turning, moving, adding, removing and extracting pages.
//!
//! Each tool works on the current page and becomes a new page list for the worker, which
//! rewrites the document in memory until it is saved.

use iced::Element;
use iced::widget::row;
use panna_core::{PageSize, PageSource};

use crate::Message;
use crate::icons::{self, Icon};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    RotateCcw,
    RotateCw,
    MoveUp,
    MoveDown,
    /// A blank page after the current one, the same size.
    InsertBlank,
    /// Asks for a PDF and puts its pages after the current one.
    InsertFile,
    /// Asks where to save the current page as a new file.
    Extract,
    Delete,
    Undo,
}

/// The page list after `op` on page `current` of `count`, and the page to show afterwards.
/// `None` when it doesn't apply (moving the first page up, removing the only page) or the
/// tool needs more than a page list.
pub fn plan(op: Op, count: usize, current: usize, size: PageSize) -> Option<(Vec<PageSource>, usize)> {
    if current >= count {
        return None;
    }
    let mut pages: Vec<PageSource> = (0..count)
        .map(|index| PageSource::Page {
            doc: 0,
            index,
            turns: 0,
        })
        .collect();
    let show = match op {
        Op::RotateCw | Op::RotateCcw => {
            if let Some(PageSource::Page { turns, .. }) = pages.get_mut(current) {
                *turns = if op == Op::RotateCw { 1 } else { 3 };
            }
            current
        }
        Op::MoveUp if current > 0 => {
            pages.swap(current, current - 1);
            current - 1
        }
        Op::MoveDown if current + 1 < count => {
            pages.swap(current, current + 1);
            current + 1
        }
        Op::InsertBlank => {
            pages.insert(
                current + 1,
                PageSource::Blank {
                    width: size.width,
                    height: size.height,
                },
            );
            current + 1
        }
        Op::Delete if count > 1 => {
            pages.remove(current);
            current.min(count - 2)
        }
        _ => return None,
    };
    Some((pages, show))
}

/// The row of page tools above the thumbnails.
pub fn tools<'a>(count: usize, current: usize, can_undo: bool, busy: bool) -> Element<'a, Message> {
    let ev = |op: Op, ok: bool| (ok && !busy && count > 0).then_some(Message::Pages(op));
    let m = icons::MOD;
    row![
        icons::button(Icon::RotateCcw, "Turn this page left", ev(Op::RotateCcw, true)),
        icons::button(Icon::RotateCw, "Turn this page right", ev(Op::RotateCw, true)),
        icons::button(Icon::MoveUp, "Move this page up", ev(Op::MoveUp, current > 0)),
        icons::button(
            Icon::MoveDown,
            "Move this page down",
            ev(Op::MoveDown, current + 1 < count)
        ),
        icons::button(
            Icon::PageAdd,
            "Add a blank page after this one",
            ev(Op::InsertBlank, true)
        ),
        icons::button(
            Icon::PageInsertFile,
            "Insert pages from another PDF after this one",
            ev(Op::InsertFile, true)
        ),
        icons::button(
            Icon::PageExtract,
            "Save this page as a new PDF",
            ev(Op::Extract, true)
        ),
        icons::button(Icon::Delete, "Delete this page", ev(Op::Delete, count > 1)),
        icons::button(
            Icon::Undo,
            format!("Undo page change ({m}Z)"),
            ev(Op::Undo, can_undo)
        ),
    ]
    .spacing(0)
    .padding([2, 4])
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: PageSize = PageSize {
        width: 100.0,
        height: 200.0,
    };

    fn indices(plan: &[PageSource]) -> Vec<Option<usize>> {
        plan.iter()
            .map(|p| match p {
                PageSource::Page { index, .. } => Some(*index),
                PageSource::Blank { .. } => None,
            })
            .collect()
    }

    #[test]
    fn plans_for_each_tool() {
        let (p, show) = plan(Op::MoveUp, 3, 2, SIZE).unwrap();
        assert_eq!((indices(&p), show), (vec![Some(0), Some(2), Some(1)], 1));
        let (p, show) = plan(Op::MoveDown, 3, 0, SIZE).unwrap();
        assert_eq!((indices(&p), show), (vec![Some(1), Some(0), Some(2)], 1));
        let (p, show) = plan(Op::Delete, 3, 2, SIZE).unwrap();
        assert_eq!((indices(&p), show), (vec![Some(0), Some(1)], 1));
        let (p, show) = plan(Op::InsertBlank, 2, 0, SIZE).unwrap();
        assert_eq!((indices(&p), show), (vec![Some(0), None, Some(1)], 1));
        let (p, _) = plan(Op::RotateCcw, 2, 1, SIZE).unwrap();
        assert!(matches!(p[1], PageSource::Page { turns: 3, .. }));
    }

    #[test]
    fn tools_that_dont_apply_give_no_plan() {
        assert!(plan(Op::MoveUp, 3, 0, SIZE).is_none());
        assert!(plan(Op::MoveDown, 3, 2, SIZE).is_none());
        assert!(plan(Op::Delete, 1, 0, SIZE).is_none());
        assert!(plan(Op::Extract, 3, 0, SIZE).is_none());
        assert!(plan(Op::RotateCw, 3, 5, SIZE).is_none());
    }
}
