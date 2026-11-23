//! Which mouse pointer the pages show, so it always says what a click will do with the tool
//! in use: a hand to pan, a magnifier to zoom, an I-beam where text gets selected or typed, a
//! crosshair where something gets drawn, and arrows on handles that stretch things.
//!
//! The checks run in the same order as the page's press handler picks a tool, so the pointer
//! never promises one thing while a click does another.

use iced::mouse::Interaction;

use crate::{add, annotate};

/// What's going on over a page, for [`pick`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Context {
    /// The add text, picture or link tool, when on.
    pub adder: Option<add::Tool>,
    /// Preparing a form: dragging out new fields.
    pub preparer: bool,
    /// Moving pictures, and whether the pointer is on a picture's corner handle.
    pub picture_editor: Option<bool>,
    pub redactor: bool,
    pub measure: bool,
    /// The snapshot (copy an area) tool.
    pub snapping: bool,
    /// The zoom tool (drag out an area to zoom to).
    pub zoom_box: bool,
    /// The hand tool, and whether the page is being dragged with it right now.
    pub hand: Option<bool>,
    /// A corner handle of the selected comment is under the pointer: whether it stretches
    /// down-right (`true`) or up-right.
    pub over_handle: Option<bool>,
    /// The comment tool in use, while commenting is on.
    pub comment_tool: Option<annotate::Tool>,
    /// A comment is being dragged.
    pub moving_comment: bool,
    /// A comment that the select tool would pick up is under the pointer.
    pub over_comment: bool,
    pub over_link: bool,
    pub over_text: bool,
    /// Text is being selected.
    pub selecting: bool,
}

/// The pointer for `c`.
pub fn pick(c: &Context) -> Interaction {
    use annotate::Tool as T;
    if let Some(tool) = c.adder {
        return match tool {
            // Click to type new text: an I-beam, as in a word processor.
            add::Tool::Text => Interaction::Text,
            // Click a line of text to change it.
            add::Tool::EditText if c.over_text => Interaction::Text,
            add::Tool::EditText => Interaction::Idle,
            add::Tool::Picture | add::Tool::Link => Interaction::Crosshair,
        };
    }
    if c.preparer {
        return Interaction::Crosshair;
    }
    if let Some(at_handle) = c.picture_editor {
        return if at_handle {
            Interaction::ResizingDiagonallyDown
        } else {
            Interaction::Move
        };
    }
    if c.redactor || c.measure || c.snapping {
        return Interaction::Crosshair;
    }
    if c.zoom_box {
        return Interaction::ZoomIn;
    }
    if let Some(dragging) = c.hand {
        return if dragging {
            Interaction::Grabbing
        } else {
            Interaction::Grab
        };
    }
    if let Some(over) = c.over_handle {
        return if over {
            Interaction::ResizingDiagonallyDown
        } else {
            Interaction::ResizingDiagonallyUp
        };
    }
    if c.moving_comment {
        return Interaction::Grabbing;
    }
    match c.comment_tool {
        // Tools that click or drag on the page itself, not on its text.
        Some(
            T::Note
            | T::TextBox
            | T::Callout
            | T::Attach
            | T::Record
            | T::Pen
            | T::Eraser
            | T::Rect
            | T::Ellipse
            | T::Arrow
            | T::Line
            | T::PolyLine
            | T::Polygon
            | T::Cloud
            | T::Stamp
            | T::Sign,
        ) => return Interaction::Crosshair,
        // Insert text puts a caret between letters.
        Some(T::InsertText) => return Interaction::Text,
        _ => {}
    }
    if c.over_comment {
        return Interaction::Grab;
    }
    if c.over_link && !c.selecting {
        return Interaction::Pointer;
    }
    match c.comment_tool {
        // The text markup tools work on text, so they show an I-beam anywhere on the page.
        Some(T::Highlight | T::Underline | T::StrikeOut | T::Replace) => Interaction::Text,
        _ if c.over_text || c.selecting => Interaction::Text,
        _ => Interaction::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use annotate::Tool as T;

    fn with(f: impl FnOnce(&mut Context)) -> Interaction {
        let mut c = Context::default();
        f(&mut c);
        pick(&c)
    }

    #[test]
    fn each_tool_has_its_pointer() {
        assert_eq!(with(|_| {}), Interaction::Idle);
        assert_eq!(with(|c| c.over_text = true), Interaction::Text);
        assert_eq!(with(|c| c.hand = Some(false)), Interaction::Grab);
        assert_eq!(with(|c| c.hand = Some(true)), Interaction::Grabbing);
        assert_eq!(with(|c| c.zoom_box = true), Interaction::ZoomIn);
        assert_eq!(with(|c| c.snapping = true), Interaction::Crosshair);
        assert_eq!(with(|c| c.measure = true), Interaction::Crosshair);
        assert_eq!(with(|c| c.adder = Some(add::Tool::Text)), Interaction::Text);
        assert_eq!(
            with(|c| c.adder = Some(add::Tool::Picture)),
            Interaction::Crosshair
        );
        assert_eq!(with(|c| c.picture_editor = Some(false)), Interaction::Move);
        assert_eq!(
            with(|c| c.picture_editor = Some(true)),
            Interaction::ResizingDiagonallyDown
        );
    }

    #[test]
    fn comment_tools() {
        for tool in [T::Pen, T::Rect, T::Note, T::TextBox, T::Stamp, T::Cloud] {
            assert_eq!(
                with(|c| c.comment_tool = Some(tool)),
                Interaction::Crosshair,
                "{tool:?}"
            );
        }
        for tool in [
            T::Highlight,
            T::Underline,
            T::StrikeOut,
            T::Replace,
            T::InsertText,
        ] {
            assert_eq!(
                with(|c| c.comment_tool = Some(tool)),
                Interaction::Text,
                "{tool:?}"
            );
        }
        assert_eq!(with(|c| c.comment_tool = Some(T::Select)), Interaction::Idle);
        assert_eq!(
            with(|c| {
                c.comment_tool = Some(T::Select);
                c.over_comment = true;
            }),
            Interaction::Grab
        );
    }

    #[test]
    fn drawing_over_a_link_still_draws() {
        // A press with a drawing tool draws, so the pointer mustn't turn into a link hand.
        assert_eq!(
            with(|c| {
                c.comment_tool = Some(T::Pen);
                c.over_link = true;
            }),
            Interaction::Crosshair
        );
        assert_eq!(with(|c| c.over_link = true), Interaction::Pointer);
    }

    #[test]
    fn handles_win() {
        assert_eq!(
            with(|c| {
                c.comment_tool = Some(T::Select);
                c.over_handle = Some(false);
            }),
            Interaction::ResizingDiagonallyUp
        );
    }
}
