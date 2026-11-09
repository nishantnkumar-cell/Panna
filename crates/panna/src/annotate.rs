//! Annotation tools: the Comment toolbar, drawing in progress, and the list of unsaved annotations
//! with undo and redo.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{
    Column, Row, button, column, container, row, slider, space, text, text_editor, text_input,
    vertical_slider,
};
use iced::{Center, Element, Fill, Point, Rectangle, Renderer, Size, Theme, mouse};
use panna_core::{
    Annotation, Changes, Color, Comment, CommentId, Kind, LINE_HEIGHT, NOTE_SIZE, ObjRef, ReviewState,
    STAMPS, StampImage, TextBox, stamp_label, text_width, wrap,
};

use crate::Message;
use crate::icons::{self, Icon};
use crate::selection::PageMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Highlight,
    Underline,
    StrikeOut,
    /// Acrobat's Replace text: crosses out the selected words and adds the new ones.
    Replace,
    /// Acrobat's Insert text: a caret where the words go, with them as its comment.
    InsertText,
    Note,
    TextBox,
    Pen,
    /// Rubs out parts of freehand drawings.
    Eraser,
    Rect,
    Ellipse,
    Arrow,
    Line,
    PolyLine,
    Polygon,
    Cloud,
    Stamp,
    Sign,
}

impl Tool {
    pub const ALL: [Tool; 19] = [
        Tool::Select,
        Tool::Highlight,
        Tool::Underline,
        Tool::StrikeOut,
        Tool::Replace,
        Tool::InsertText,
        Tool::Note,
        Tool::TextBox,
        Tool::Pen,
        Tool::Eraser,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Arrow,
        Tool::Line,
        Tool::PolyLine,
        Tool::Polygon,
        Tool::Cloud,
        Tool::Stamp,
        Tool::Sign,
    ];

    pub fn icon(self) -> Icon {
        match self {
            Tool::Select => Icon::Pointer,
            Tool::Highlight => Icon::Highlighter,
            Tool::Underline => Icon::Underline,
            Tool::StrikeOut => Icon::Strikethrough,
            Tool::Replace => Icon::ReplaceText,
            Tool::InsertText => Icon::InsertText,
            Tool::Eraser => Icon::Eraser,
            Tool::PolyLine => Icon::PolyLine,
            Tool::Note => Icon::StickyNote,
            Tool::TextBox => Icon::TextBox,
            Tool::Pen => Icon::Pen,
            Tool::Rect => Icon::Square,
            Tool::Ellipse => Icon::Circle,
            Tool::Arrow => Icon::Arrow,
            Tool::Line => Icon::Line,
            Tool::Polygon => Icon::Polygon,
            Tool::Cloud => Icon::Cloud,
            Tool::Stamp => Icon::Stamp,
            Tool::Sign => Icon::Signature,
        }
    }

    pub fn tip(self) -> String {
        let what = match self {
            Tool::Select => "Select text",
            Tool::Highlight => "Highlight text",
            Tool::Underline => "Underline text",
            Tool::StrikeOut => "Strike through text",
            Tool::Replace => "Replace text: select the words, then type the new ones",
            Tool::InsertText => "Insert text: click where the words go, then type them",
            Tool::Eraser => "Erase parts of freehand drawings",
            Tool::PolyLine => "Draw a polygon line: click each point, double-click or Enter to finish",
            Tool::Note => "Add a sticky note",
            Tool::TextBox => "Add a text box",
            Tool::Pen => "Draw freehand",
            Tool::Rect => "Draw a rectangle",
            Tool::Ellipse => "Draw an ellipse",
            Tool::Arrow => "Draw an arrow",
            Tool::Line => "Draw a line",
            Tool::Polygon => "Draw a polygon: click each corner, double-click or Enter to finish",
            Tool::Cloud => "Draw a cloud: click each corner, double-click or Enter to finish",
            Tool::Stamp => "Add a stamp such as Approved or Draft: click, or drag to size it",
            Tool::Sign => {
                "Fill & Sign: add your signature, initials, the date or a check mark where you click"
            }
        };
        match self.key() {
            Some(key) => format!("{what} ({key})"),
            None => what.to_owned(),
        }
    }

    /// The tool's name in the Comment menu.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Highlight => "Highlight",
            Tool::Underline => "Underline",
            Tool::StrikeOut => "Strike out",
            Tool::Replace => "Replace text",
            Tool::InsertText => "Insert text",
            Tool::Eraser => "Eraser",
            Tool::PolyLine => "Polygon line",
            Tool::Note => "Sticky note",
            Tool::TextBox => "Text box",
            Tool::Pen => "Draw",
            Tool::Rect => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Arrow => "Arrow",
            Tool::Line => "Line",
            Tool::Polygon => "Polygon",
            Tool::Cloud => "Cloud",
            Tool::Stamp => "Stamp",
            Tool::Sign => "Fill & Sign",
        }
    }

    /// The key that picks this tool, as in Acrobat's single-key accelerators: U, S, X, Q and K
    /// pick a tool, and Shift+U and Shift+D step through the text markups and the drawing
    /// tools. Fill & Sign has none in Acrobat either.
    pub fn key(self) -> Option<&'static str> {
        match self {
            Tool::Select => Some("V"),
            Tool::Highlight => Some("U"),
            Tool::Underline | Tool::StrikeOut => Some("Shift+U"),
            Tool::Note => Some("S"),
            Tool::TextBox => Some("X"),
            Tool::Pen
            | Tool::Eraser
            | Tool::Rect
            | Tool::Ellipse
            | Tool::Arrow
            | Tool::Line
            | Tool::PolyLine
            | Tool::Polygon => Some("Shift+D"),
            Tool::Replace | Tool::InsertText => None,
            Tool::Cloud => Some("Q"),
            Tool::Stamp => Some("K"),
            Tool::Sign => None,
        }
    }

    /// The text markups Shift+U steps through, in Acrobat's order.
    pub const MARKUPS: [Tool; 3] = [Tool::Highlight, Tool::Underline, Tool::StrikeOut];
    /// The drawing tools Shift+D steps through, in Acrobat's order.
    pub const DRAWING: [Tool; 9] = [
        Tool::Cloud,
        Tool::Arrow,
        Tool::Line,
        Tool::Rect,
        Tool::Ellipse,
        Tool::PolyLine,
        Tool::Polygon,
        Tool::Pen,
        Tool::Eraser,
    ];

    /// The tool after `self` in `group`, or the group's first.
    pub fn next_in(self, group: &[Tool]) -> Tool {
        let next = group.iter().position(|t| *t == self).map_or(0, |i| i + 1);
        group.get(next).or_else(|| group.first()).copied().unwrap_or(self)
    }

    /// Tools that place one corner per click instead of drawing with a drag.
    pub fn by_corners(self) -> bool {
        matches!(self, Tool::Polygon | Tool::Cloud | Tool::PolyLine)
    }

    /// Corners a finished shape needs at least.
    fn min_corners(self) -> usize {
        if self == Tool::PolyLine { 2 } else { 3 }
    }

    /// Tools that work on selected text, so dragging selects text as the Select tool does.
    pub fn marks_text(self) -> bool {
        matches!(
            self,
            Tool::Select | Tool::Highlight | Tool::Underline | Tool::StrikeOut | Tool::Replace
        )
    }

    fn index(self) -> usize {
        Tool::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn default_color(self) -> Color {
        match self {
            Tool::Select | Tool::Highlight | Tool::Note => SWATCHES[0],
            Tool::TextBox | Tool::Sign => SWATCHES[5],
            Tool::Pen | Tool::Replace | Tool::InsertText => SWATCHES[2],
            _ => SWATCHES[3],
        }
    }
}

/// Yellow, green, blue, red, purple, black.
pub const SWATCHES: [Color; 6] = [
    Color([1.0, 0.85, 0.1]),
    Color([0.18, 0.63, 0.26]),
    Color([0.12, 0.44, 0.92]),
    Color([0.86, 0.15, 0.15]),
    Color([0.55, 0.36, 0.96]),
    Color([0.0, 0.0, 0.0]),
];

const STROKE_WIDTH: f32 = 2.0;
const TEXT_SIZE: f32 = 12.0;
const TEXT_BOX_WIDTH: f32 = 200.0;
/// Height of a stamp placed with a click, in points.
const STAMP_HEIGHT: f32 = 34.0;
/// A picture stamp's height for a click, and its widest, in points.
const PICTURE_HEIGHT: f32 = 60.0;
const PICTURE_MAX_WIDTH: f32 = 220.0;
/// The usual height of a placed signature and of initials, in points.
const SIGNATURE_HEIGHT: f32 = 36.0;
const INITIALS_HEIGHT: f32 = 24.0;
/// The size of a placed check mark or cross, in points.
const MARK_SIZE: f32 = 12.0;
/// Drags shorter than this (in points) count as clicks.
const CLICK_SLOP: f32 = 3.0;
/// Most corners one polygon or cloud can have.
const MAX_CORNERS: usize = 1_000;

#[derive(Debug, Clone)]
pub enum Event {
    Toggle,
    Tool(Tool),
    Color(usize),
    /// The opacity slider moved (0–1).
    Opacity(f32),
    /// The opacity slider was let go.
    OpacityDone,
    /// Deletes the selected comment.
    DeletePicked,
    Undo,
    Redo,
    /// Removes the unsaved annotation at this index.
    Delete(usize),
    /// Removes a comment saved in the file.
    Remove(CommentId),
    Edit(text_editor::Action),
    EditDone,
    EditCancel,
    /// Finishes the polygon or cloud being drawn.
    FinishShape,
    /// Drops the polygon or cloud being drawn.
    CancelShape,
    /// Opens the editor for a reply to a saved comment.
    Reply(Thread),
    /// Sets a saved comment's review status.
    SetStatus(Thread, ReviewState),
    /// Puts a checkmark on a saved comment, or takes it off.
    ToggleCheck(Thread),
    /// Toggles the checkmark on the selected saved comment, as Acrobat's Shift+K.
    CheckPicked,
    /// Places the standard stamp at this index of [`STAMPS`].
    Stamp(usize),
    /// Typing a custom stamp's text.
    StampText(String),
    /// Places a stamp with the typed text.
    StampCustom,
    /// Places a picture, such as a signature or logo, named for its file.
    StampPicture(StampImage, String),
    Sign(crate::sign::Event),
}

/// Where a stamp goes, while its kind is being chosen.
#[derive(Debug, Clone)]
pub struct Stamping {
    page: usize,
    /// The dragged box, or a point (an empty box) for a click.
    at: TextBox,
    custom: String,
    /// Placing with Fill & Sign rather than choosing a stamp.
    signing: bool,
}

/// A saved comment that replies and statuses attach to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thread {
    pub page: usize,
    pub comment: ObjRef,
    /// Where the comment is, which its replies share.
    pub at: TextBox,
}

/// A shape being drawn, in page points.
#[derive(Debug, Clone)]
pub struct Draft {
    pub page: usize,
    tool: Tool,
    points: Vec<(f32, f32)>,
}

/// The comment selected for moving or restyling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picked {
    /// An unsaved annotation, by index.
    New(usize),
    /// A comment saved in the file.
    Saved(CommentId),
}

/// The selected comment being dragged, in page points.
struct Moving {
    from: (f32, f32),
    to: (f32, f32),
    /// The page's size, which the comment stays within.
    page: (f32, f32),
    /// Set when a corner handle is dragged: the opposite corner, which stays put.
    anchor: Option<(f32, f32)>,
}

/// A note or text box waiting for its text, or a comment whose text is being changed.
pub struct Editing {
    page: usize,
    tool: Tool,
    at: TextBox,
    pub content: text_editor::Content,
    /// The comment being changed; `None` for a new one.
    target: Option<Picked>,
    /// Set when writing a reply: the comment replied to.
    reply: Option<Thread>,
    /// For Replace text: the words crossed out, by page.
    replacing: Vec<(usize, Vec<TextBox>)>,
}

#[derive(Default)]
pub struct Annotate {
    pub open: bool,
    pub tool: Tool,
    colors: [Option<Color>; Tool::ALL.len()],
    opacities: [Option<f32>; Tool::ALL.len()],
    /// Unsaved annotations, oldest first.
    pub items: Vec<Annotation>,
    /// Saved comments taken out, not yet saved.
    pub removed: Vec<CommentId>,
    /// Earlier versions of `items` and `removed`, for undo and redo.
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    pub draft: Option<Draft>,
    pub editing: Option<Editing>,
    pub stamping: Option<Stamping>,
    /// The signature and initials made for Fill & Sign.
    pub sign: crate::sign::Signer,
    author: String,
    /// Saved comments Panna can redraw, so they can be moved and restyled.
    saved: Vec<(CommentId, Annotation)>,
    pub picked: Option<Picked>,
    moving: Option<Moving>,
    /// The opacity slider is being dragged; its undo step is already recorded.
    sliding: bool,
}

type Snapshot = (Vec<Annotation>, Vec<CommentId>);

/// What an event changed.
pub enum Outcome {
    Nothing,
    /// The annotation list changed and must be re-applied to the document.
    Changed,
}

impl Annotate {
    pub fn new() -> Self {
        let author = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
        Self {
            author,
            ..Self::default()
        }
    }

    /// The name put on new comments and replies.
    pub fn set_author(&mut self, name: String) {
        self.author = name;
    }

    pub fn author(&self) -> &str {
        &self.author
    }

    pub fn color(&self) -> Color {
        self.colors
            .get(self.tool.index())
            .copied()
            .flatten()
            .unwrap_or_else(|| self.tool.default_color())
    }

    /// The current opacity: the selected comment's, or the tool's.
    pub fn opacity(&self) -> f32 {
        if let Some(a) = self.picked_annotation() {
            return a.opacity;
        }
        self.opacities
            .get(self.tool.index())
            .copied()
            .flatten()
            .unwrap_or(1.0)
    }

    /// Forgets unsaved annotations once they are saved (or discarded, or a new file opens),
    /// and takes the file's comments.
    pub fn saved(&mut self, comments: &[Comment]) {
        self.items.clear();
        self.removed.clear();
        self.undo.clear();
        self.redo.clear();
        self.draft = None;
        self.editing = None;
        self.unpick();
        self.saved = comments
            .iter()
            .filter_map(|c| Some((c.id?, c.editable.clone()?)))
            .collect();
    }

    pub fn unpick(&mut self) {
        self.picked = None;
        self.moving = None;
        self.sliding = false;
    }

    /// Whether a drag (drawing or moving) is in progress.
    pub fn busy(&self) -> bool {
        self.draft.is_some() || self.moving.is_some()
    }

    pub fn is_moving(&self) -> bool {
        self.moving.as_ref().is_some_and(|m| m.anchor.is_none())
    }

    pub fn is_resizing(&self) -> bool {
        self.moving.as_ref().is_some_and(|m| m.anchor.is_some())
    }

    /// The selected comment's box on `page`, if it can be resized by its corners.
    pub fn resizable(&self, page: usize) -> Option<TextBox> {
        let a = self
            .picked_annotation()
            .filter(|a| a.page == page && can_resize(a))?;
        Some(Comment::from(a).at)
    }

    /// The corner handle of the selected comment under a point, if any: the opposite corner.
    /// `slop` is how near counts, in points.
    pub fn handle_at(&self, page: usize, (x, y): (f32, f32), slop: f32) -> Option<(f32, f32)> {
        let b = self.resizable(page)?;
        corners(b)
            .into_iter()
            .zip(corners(b).into_iter().rev())
            .find(|((cx, cy), _)| (cx - x).abs() <= slop && (cy - y).abs() <= slop)
            .map(|(_, opposite)| opposite)
    }

    fn picked_annotation(&self) -> Option<&Annotation> {
        match self.picked? {
            Picked::New(i) => self.items.get(i),
            Picked::Saved(id) => self.saved.iter().find(|(s, _)| *s == id).map(|(_, a)| a),
        }
    }

    /// The selected comment's outline on `page`, moved along while it is being dragged.
    pub fn picked_outline(&self, page: usize) -> Option<TextBox> {
        let a = self.picked_annotation().filter(|a| a.page == page)?;
        let mut b = Comment::from(a).at;
        if let Some(m) = self.moving.as_ref().filter(|m| m.anchor.is_some()) {
            b = resized(m);
        } else if let Some(m) = &self.moving {
            let (dx, dy) = clamp_shift(b, m);
            b = TextBox {
                x0: b.x0 + dx,
                y0: b.y0 + dy,
                x1: b.x1 + dx,
                y1: b.y1 + dy,
            };
        }
        Some(grown(b, 2.0))
    }

    /// The comment Panna can move under a point, newest first.
    pub fn hit(&self, page: usize, at: (f32, f32)) -> Option<Picked> {
        let new = self
            .items
            .iter()
            .enumerate()
            .rev()
            .find(|(_, a)| a.page == page && hits(a, at))
            .map(|(i, _)| Picked::New(i));
        new.or_else(|| {
            self.saved
                .iter()
                .rev()
                .find(|(id, a)| a.page == page && !self.removed.contains(id) && hits(a, at))
                .map(|(id, _)| Picked::Saved(*id))
        })
    }

    /// Selects `picked`, if it is one Panna can change.
    pub fn pick(&mut self, picked: Picked) {
        self.moving = None;
        self.picked = match picked {
            Picked::New(i) => (i < self.items.len()).then_some(picked),
            Picked::Saved(id) => self.saved.iter().any(|(s, _)| *s == id).then_some(picked),
        };
    }

    /// Selects the comment under the pointer and starts moving it. Returns false when there
    /// is none, so the press selects text instead.
    pub fn grab(&mut self, page: usize, at: (f32, f32), size: (f32, f32), slop: f32) -> bool {
        // A corner handle of the selected comment resizes it.
        if let Some(anchor) = self.handle_at(page, at, slop) {
            self.moving = Some(Moving {
                from: at,
                to: at,
                page: size,
                anchor: Some(anchor),
            });
            return true;
        }
        let Some(picked) = self.hit(page, at) else {
            self.unpick();
            return false;
        };
        self.pick(picked);
        self.moving = Some(Moving {
            from: at,
            to: at,
            page: size,
            anchor: None,
        });
        true
    }

    /// Opens the editor on the text of the comment under a point (a double-click with the
    /// select tool). Returns false when there is none.
    pub fn edit_text(&mut self, page: usize, at: (f32, f32)) -> bool {
        let Some(picked) = self.hit(page, at) else {
            return false;
        };
        self.pick(picked);
        let Some(a) = self.picked_annotation() else {
            return false;
        };
        let tool = match a.kind {
            Kind::Note { .. } => Tool::Note,
            Kind::FreeText { .. } => Tool::TextBox,
            _ => Tool::Select,
        };
        self.editing = Some(Editing {
            page,
            tool,
            at: Comment::from(a).at,
            content: text_editor::Content::with_text(&a.contents),
            target: Some(picked),
            reply: None,
            replacing: Vec::new(),
        });
        true
    }

    /// Changes the selected comment as one undo step. A saved comment is taken out and redrawn
    /// as a new one, as Acrobat does.
    fn edit(&mut self, record: bool, change: impl FnOnce(&mut Annotation)) -> bool {
        let Some(picked) = self.picked else {
            return false;
        };
        let index = match picked {
            Picked::New(i) if i < self.items.len() => {
                if record {
                    self.record();
                }
                i
            }
            Picked::New(_) => return false,
            Picked::Saved(id) => {
                let Some(a) = self.saved.iter().find(|(s, _)| *s == id).map(|(_, a)| a.clone()) else {
                    return false;
                };
                // Taking a saved comment out is always its own step, even mid-slide.
                self.record();
                self.removed.push(id);
                self.items.push(a);
                self.items.len() - 1
            }
        };
        self.picked = Some(Picked::New(index));
        if let Some(a) = self.items.get_mut(index) {
            change(a);
        }
        true
    }

    /// Everything not yet saved, for the renderer and for saving.
    pub fn changes(&self) -> Changes {
        Changes {
            add: self.items.clone(),
            remove: self.removed.clone(),
            ..Changes::default()
        }
    }

    /// Whether anything needs saving.
    pub fn dirty(&self) -> bool {
        !self.items.is_empty() || !self.removed.is_empty()
    }

    /// Swaps in a snapshot, returning the state it replaced.
    fn restore(&mut self, (items, removed): Snapshot) -> Snapshot {
        (
            std::mem::replace(&mut self.items, items),
            std::mem::replace(&mut self.removed, removed),
        )
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Keeps the current list for undo, before changing it.
    fn record(&mut self) {
        self.undo.push((self.items.clone(), self.removed.clone()));
        self.redo.clear();
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn update(&mut self, event: Event) -> Outcome {
        match event {
            Event::Toggle => {
                self.open = !self.open;
                if !self.open {
                    self.tool = Tool::Select;
                    self.draft = None;
                }
            }
            Event::Tool(tool) => {
                self.tool = tool;
                self.draft = None;
                if tool != Tool::Select {
                    self.unpick();
                }
            }
            Event::Color(i) => {
                let Some(color) = SWATCHES.get(i).copied() else {
                    return Outcome::Nothing;
                };
                if self.picked.is_some() {
                    if self.picked_annotation().is_some_and(|a| a.color != color)
                        && self.edit(true, |a| a.color = color)
                    {
                        return Outcome::Changed;
                    }
                } else if let Some(slot) = self.colors.get_mut(self.tool.index()) {
                    *slot = Some(color);
                }
            }
            Event::Opacity(value) => {
                let value = value.clamp(0.1, 1.0);
                if self.picked.is_some() {
                    let first = !self.sliding;
                    self.sliding = true;
                    self.edit(first, |a| a.opacity = value);
                } else if let Some(slot) = self.opacities.get_mut(self.tool.index()) {
                    *slot = Some(value);
                }
            }
            Event::OpacityDone => {
                if std::mem::take(&mut self.sliding) {
                    return Outcome::Changed;
                }
            }
            Event::DeletePicked => match self.picked.take() {
                Some(Picked::New(i)) => return self.update(Event::Delete(i)),
                Some(Picked::Saved(id)) => return self.update(Event::Remove(id)),
                None => {}
            },
            Event::Undo => {
                self.unpick();
                if let Some(before) = self.undo.pop() {
                    let now = self.restore(before);
                    self.redo.push(now);
                    return Outcome::Changed;
                }
            }
            Event::Redo => {
                self.unpick();
                if let Some(after) = self.redo.pop() {
                    let now = self.restore(after);
                    self.undo.push(now);
                    return Outcome::Changed;
                }
            }
            Event::Remove(id) => {
                self.unpick();
                if !self.removed.contains(&id) {
                    self.record();
                    self.removed.push(id);
                    // Its replies and statuses go with it.
                    let replies: Vec<CommentId> = self
                        .saved
                        .iter()
                        .filter(|(r, a)| {
                            matches!(a.kind, Kind::Reply { to, .. } if to == id.annot)
                                && !self.removed.contains(r)
                        })
                        .map(|(r, _)| *r)
                        .collect();
                    self.removed.extend(replies);
                    self.items
                        .retain(|a| !matches!(a.kind, Kind::Reply { to, .. } if to == id.annot));
                    return Outcome::Changed;
                }
            }
            Event::Delete(index) => {
                self.unpick();
                if index < self.items.len() {
                    self.record();
                    self.items.remove(index);
                    return Outcome::Changed;
                }
            }
            Event::Edit(action) => {
                if let Some(editing) = &mut self.editing {
                    editing.content.perform(action);
                }
            }
            Event::EditDone => {
                if let Some(editing) = self.editing.take() {
                    let text = editing.content.text().trim_end().to_owned();
                    if (editing.tool == Tool::TextBox || editing.reply.is_some()) && text.trim().is_empty() {
                        return Outcome::Nothing;
                    }
                    if let Some(thread) = editing.reply {
                        self.record();
                        self.push_reply(thread, None, text);
                        return Outcome::Changed;
                    }
                    if let Some(target) = editing.target {
                        self.picked = Some(target);
                        let unchanged = self.picked_annotation().is_none_or(|a| a.contents == text);
                        if unchanged || !self.edit(true, |a| set_text(a, text)) {
                            return Outcome::Nothing;
                        }
                        return Outcome::Changed;
                    }
                    if text.trim().is_empty() && matches!(editing.tool, Tool::Replace | Tool::InsertText) {
                        return Outcome::Nothing;
                    }
                    if editing.tool == Tool::Replace {
                        // One step: the old words crossed out, and a caret after them carrying
                        // the new ones, as Acrobat's Replace text does.
                        self.record();
                        for (page, boxes) in editing.replacing {
                            self.push(page, Kind::StrikeOut(boxes), Tool::Replace, text.clone());
                        }
                        self.push(
                            editing.page,
                            Kind::Caret { rect: editing.at },
                            Tool::Replace,
                            text,
                        );
                        return Outcome::Changed;
                    }
                    if editing.tool == Tool::InsertText {
                        self.record();
                        self.push(
                            editing.page,
                            Kind::Caret { rect: editing.at },
                            Tool::InsertText,
                            text,
                        );
                        return Outcome::Changed;
                    }
                    let kind = if editing.tool == Tool::Note {
                        Kind::Note {
                            x: editing.at.x0,
                            y: editing.at.y0,
                        }
                    } else {
                        Kind::FreeText {
                            rect: fit_text_box(editing.at, &text),
                            size: TEXT_SIZE,
                        }
                    };
                    self.record();
                    self.push(editing.page, kind, editing.tool, text);
                    return Outcome::Changed;
                }
            }
            Event::EditCancel => {
                self.editing = None;
                self.stamping = None;
                self.sign.pad = None;
            }
            Event::Stamp(i) => {
                if let Some(name) = STAMPS.get(i) {
                    return self.place_stamp((*name).to_owned(), None, PICTURE_HEIGHT);
                }
            }
            Event::StampText(value) => {
                if let Some(s) = &mut self.stamping {
                    s.custom = value.chars().filter(|c| !c.is_control()).take(60).collect();
                }
            }
            Event::StampCustom => {
                let text = self.stamping.as_ref().map(|s| s.custom.trim().to_owned());
                if let Some(text) = text.filter(|t| !t.is_empty()) {
                    return self.place_stamp(text, None, PICTURE_HEIGHT);
                }
            }
            Event::StampPicture(image, name) => return self.place_stamp(name, Some(image), PICTURE_HEIGHT),
            Event::Sign(event) => return self.sign(event),
            Event::FinishShape => return self.finish_shape(),
            Event::Reply(thread) => {
                self.unpick();
                self.editing = Some(Editing {
                    page: thread.page,
                    tool: Tool::Select,
                    at: thread.at,
                    content: text_editor::Content::new(),
                    target: None,
                    reply: Some(thread),
                    replacing: Vec::new(),
                });
            }
            Event::SetStatus(thread, state) => {
                if self.status(thread.comment) == state {
                    return Outcome::Nothing;
                }
                // Acrobat words a status change like this in the thread.
                let text = if self.author.is_empty() {
                    format!("Status set to {}", state.label())
                } else {
                    format!("{} set by {}", state.label(), self.author)
                };
                self.record();
                self.push_reply(thread, Some(state), text);
                return Outcome::Changed;
            }
            Event::ToggleCheck(thread) => {
                let checked = !self.checked(thread.comment);
                self.record();
                self.push_check(thread, checked);
                return Outcome::Changed;
            }
            Event::CheckPicked => {
                let Some(Picked::Saved(id)) = self.picked else {
                    return Outcome::Nothing;
                };
                let Some(at) = self.picked_annotation().map(|a| Comment::from(a).at) else {
                    return Outcome::Nothing;
                };
                return self.update(Event::ToggleCheck(Thread {
                    page: id.page,
                    comment: id.annot,
                    at,
                }));
            }
            Event::CancelShape => {
                if self.draft.as_ref().is_some_and(|d| d.tool.by_corners()) {
                    self.draft = None;
                }
            }
        }
        Outcome::Nothing
    }

    /// Adds the stamp being placed. A click centres a stamp sized to its words on the point; a
    /// picture keeps its shape, fitted inside a dragged box or at a usual size for a click.
    fn place_stamp(&mut self, name: String, image: Option<StampImage>, height: f32) -> Outcome {
        let Some(stamping) = self.stamping.take() else {
            return Outcome::Nothing;
        };
        let at = stamping.at;
        let dragged = at.width() >= CLICK_SLOP && at.height() >= CLICK_SLOP;
        let rect = if let Some(image) = &image {
            let aspect = image.width as f32 / image.height.max(1) as f32;
            let (w, h) = if dragged {
                let w = at.width().min(at.height() * aspect);
                (w, w / aspect)
            } else {
                let w = (height * aspect).min(PICTURE_MAX_WIDTH);
                (w, w / aspect)
            };
            let (cx, cy) = if dragged {
                ((at.x0 + at.x1) / 2.0, (at.y0 + at.y1) / 2.0)
            } else if stamping.signing {
                // Signatures start where the click was, as when signing on a line.
                (at.x0 + w / 2.0, at.y0)
            } else {
                (at.x0, at.y0)
            };
            TextBox {
                x0: cx - w / 2.0,
                y0: cy - h / 2.0,
                x1: cx + w / 2.0,
                y1: cy + h / 2.0,
            }
        } else if dragged {
            at
        } else {
            let h = STAMP_HEIGHT;
            let w = (text_width(&stamp_label(&name)) as f32 * 1.06 * h * 0.55 + h * 0.8).max(h * 2.0);
            TextBox {
                x0: at.x0 - w / 2.0,
                y0: at.y0 - h / 2.0,
                x1: at.x0 + w / 2.0,
                y1: at.y0 + h / 2.0,
            }
        };
        self.record();
        let chosen = self.colors.get(Tool::Stamp.index()).copied().flatten();
        self.push(
            stamping.page,
            Kind::Stamp {
                rect,
                name: name.clone(),
                image,
            },
            Tool::Stamp,
            String::new(),
        );
        if chosen.is_none()
            && let Some(a) = self.items.last_mut()
        {
            a.color = stamp_color(&name);
        }
        Outcome::Changed
    }

    /// A Fill & Sign choice: makes a signature, or places one or a mark where the user clicked.
    fn sign(&mut self, event: crate::sign::Event) -> Outcome {
        use crate::sign::Action;
        let ink = self
            .colors
            .get(Tool::Sign.index())
            .copied()
            .flatten()
            .unwrap_or_else(|| Tool::Sign.default_color());
        let action = self.sign.update(event, ink);
        let Some(stamping) = &self.stamping else {
            return Outcome::Nothing;
        };
        let (page, at) = (stamping.page, stamping.at);
        // A click's point, or the middle of a dragged box.
        let (cx, cy) = ((at.x0 + at.x1) / 2.0, (at.y0 + at.y1) / 2.0);
        let kind = match action {
            Action::None => return Outcome::Nothing,
            Action::Picture(image, name) => {
                let height = if name == crate::sign::Which::Initials.name() {
                    INITIALS_HEIGHT
                } else {
                    SIGNATURE_HEIGHT
                };
                return self.place_stamp(name.to_owned(), Some(image), height);
            }
            Action::Date => {
                let date = panna_core::today();
                self.stamping = None;
                self.record();
                let rect = fit_text_box(
                    TextBox {
                        x0: at.x0,
                        y0: at.y0,
                        x1: at.x0 + TEXT_BOX_WIDTH,
                        y1: at.y0 + NOTE_SIZE,
                    },
                    &date,
                );
                self.push(
                    page,
                    Kind::FreeText {
                        rect,
                        size: TEXT_SIZE,
                    },
                    Tool::Sign,
                    date,
                );
                return Outcome::Changed;
            }
            Action::Check => {
                let s = MARK_SIZE;
                Kind::Ink(vec![vec![
                    (cx - s * 0.5, cy),
                    (cx - s * 0.15, cy + s * 0.4),
                    (cx + s * 0.55, cy - s * 0.45),
                ]])
            }
            Action::Cross => {
                let h = MARK_SIZE * 0.45;
                Kind::Ink(vec![
                    vec![(cx - h, cy - h), (cx + h, cy + h)],
                    vec![(cx - h, cy + h), (cx + h, cy - h)],
                ])
            }
        };
        self.stamping = None;
        self.record();
        self.push(page, kind, Tool::Sign, String::new());
        Outcome::Changed
    }

    /// Adds a new annotation made with `tool`.
    fn push(&mut self, page: usize, kind: Kind, tool: Tool, contents: String) {
        let color = self
            .colors
            .get(tool.index())
            .copied()
            .flatten()
            .unwrap_or_else(|| tool.default_color());
        let opacity = self.opacities.get(tool.index()).copied().flatten().unwrap_or(1.0);
        self.items.push(Annotation {
            page,
            kind,
            color,
            opacity,
            width: STROKE_WIDTH,
            contents,
            author: self.author.clone(),
        });
    }

    fn push_reply(&mut self, thread: Thread, state: Option<ReviewState>, contents: String) {
        self.items.push(Annotation {
            page: thread.page,
            kind: Kind::Reply {
                to: thread.comment,
                at: thread.at,
                state,
                checked: None,
            },
            color: SWATCHES[0],
            opacity: 1.0,
            width: STROKE_WIDTH,
            contents,
            author: self.author.clone(),
        });
    }

    fn push_check(&mut self, thread: Thread, checked: bool) {
        self.items.push(Annotation {
            page: thread.page,
            kind: Kind::Reply {
                to: thread.comment,
                at: thread.at,
                state: None,
                checked: Some(checked),
            },
            color: SWATCHES[0],
            opacity: 1.0,
            width: STROKE_WIDTH,
            contents: String::new(),
            author: self.author.clone(),
        });
    }

    /// Whether a saved comment has a checkmark: the latest one put on or taken off, saved or not.
    pub fn checked(&self, comment: ObjRef) -> bool {
        let check_of = |a: &Annotation| match a.kind {
            Kind::Reply { to, checked, .. } if to == comment => checked,
            _ => None,
        };
        self.items
            .iter()
            .rev()
            .find_map(check_of)
            .or_else(|| {
                self.saved
                    .iter()
                    .rev()
                    .filter(|(id, _)| !self.removed.contains(id))
                    .find_map(|(_, a)| check_of(a))
            })
            .unwrap_or(false)
    }

    /// A saved comment's review status: the latest one set, saved or not.
    pub fn status(&self, comment: ObjRef) -> ReviewState {
        let state_of = |a: &Annotation| match a.kind {
            Kind::Reply { to, state, .. } if to == comment => state,
            _ => None,
        };
        self.items
            .iter()
            .rev()
            .find_map(state_of)
            .or_else(|| {
                self.saved
                    .iter()
                    .rev()
                    .filter(|(id, _)| !self.removed.contains(id))
                    .find_map(|(_, a)| state_of(a))
            })
            .unwrap_or(ReviewState::None)
    }

    /// Marks selected text with the current markup tool: one annotation per page.
    pub fn mark_text(&mut self, per_page: Vec<(usize, Vec<TextBox>)>) -> Outcome {
        let tool = self.tool;
        if !matches!(
            tool,
            Tool::Highlight | Tool::Underline | Tool::StrikeOut | Tool::Replace
        ) || per_page.iter().all(|(_, b)| b.is_empty())
        {
            return Outcome::Nothing;
        }
        if tool == Tool::Replace {
            // Ask for the new words; the crossing out and the caret are added with them.
            let replacing: Vec<(usize, Vec<TextBox>)> =
                per_page.into_iter().filter(|(_, b)| !b.is_empty()).collect();
            let Some((page, last)) = replacing
                .last()
                .and_then(|(page, boxes)| Some((*page, *boxes.last()?)))
            else {
                return Outcome::Nothing;
            };
            self.editing = Some(Editing {
                page,
                tool,
                at: caret_box(last.x1, last.y1, last.height()),
                content: text_editor::Content::new(),
                target: None,
                reply: None,
                replacing,
            });
            return Outcome::Nothing;
        }
        // One undo step, even when the selection spans pages.
        self.record();
        for (page, boxes) in per_page {
            let kind = match tool {
                _ if boxes.is_empty() => continue,
                Tool::Highlight => Kind::Highlight(boxes),
                Tool::Underline => Kind::Underline(boxes),
                _ => Kind::StrikeOut(boxes),
            };
            self.push(page, kind, tool, String::new());
        }
        Outcome::Changed
    }

    /// Rubs out the parts of freehand drawings on `page` that the eraser passed over, saved
    /// ones included, as one undo step. A drawing rubbed out entirely is removed.
    fn erase(&mut self, page: usize, path: &[(f32, f32)]) -> Outcome {
        let mut changed_new: Vec<(usize, Strokes)> = Vec::new();
        for (i, a) in self.items.iter().enumerate() {
            if let (true, Kind::Ink(strokes)) = (a.page == page, &a.kind)
                && let Some(left) = erase_strokes(strokes, path)
            {
                changed_new.push((i, left));
            }
        }
        let mut changed_saved: Vec<(CommentId, Annotation)> = Vec::new();
        for (id, a) in &self.saved {
            if self.removed.contains(id) || a.page != page {
                continue;
            }
            if let Kind::Ink(strokes) = &a.kind
                && let Some(left) = erase_strokes(strokes, path)
            {
                let mut copy = a.clone();
                copy.kind = Kind::Ink(left);
                changed_saved.push((*id, copy));
            }
        }
        if changed_new.is_empty() && changed_saved.is_empty() {
            return Outcome::Nothing;
        }
        self.record();
        self.unpick();
        for (i, left) in &changed_new {
            if let Some(a) = self.items.get_mut(*i) {
                a.kind = Kind::Ink(left.clone());
            }
        }
        // Drawings with nothing left go, highest index first so the others stay put.
        for (i, left) in changed_new.iter().rev() {
            if left.is_empty() && *i < self.items.len() {
                self.items.remove(*i);
            }
        }
        for (id, copy) in changed_saved {
            self.removed.push(id);
            if matches!(&copy.kind, Kind::Ink(s) if !s.is_empty()) {
                self.items.push(copy);
            }
        }
        Outcome::Changed
    }

    /// Adds comments read from a comments file, as one undo step.
    pub fn add_imported(&mut self, list: Vec<Annotation>) -> Outcome {
        if list.is_empty() {
            return Outcome::Nothing;
        }
        self.record();
        self.items.extend(list);
        Outcome::Changed
    }

    /// Starts drawing at a point on `page`, for tools that draw.
    pub fn press(&mut self, page: usize, at: (f32, f32)) -> Outcome {
        if self.tool.marks_text() || self.editing.is_some() {
            return Outcome::Nothing;
        }
        if self.tool.by_corners() {
            // The last point follows the pointer; a click fixes it and starts the next one.
            if let Some(draft) = self
                .draft
                .as_mut()
                .filter(|d| d.page == page && d.tool.by_corners())
            {
                // Clicking the first corner, or the last one again (as a double-click does),
                // finishes the shape.
                let near = |p: Option<&(f32, f32)>| {
                    p.is_some_and(|p| (p.0 - at.0).hypot(p.1 - at.1) < CLICK_SLOP * 2.0)
                };
                let fixed = draft.points.len() - 1;
                let min = draft.tool.min_corners();
                // A polygon line ends on a double-click; it doesn't close on its first point.
                let at_first = draft.tool != Tool::PolyLine && near(draft.points.first());
                let closes = fixed >= min && (at_first || near(draft.points.get(fixed.saturating_sub(1))));
                if closes {
                    return self.finish_shape();
                }
                if let Some(last) = draft.points.last_mut() {
                    *last = at;
                }
                if draft.points.len() < MAX_CORNERS {
                    draft.points.push(at);
                }
                return Outcome::Nothing;
            }
        }
        let points = if self.tool.by_corners() {
            vec![at, at]
        } else {
            vec![at]
        };
        self.draft = Some(Draft {
            page,
            tool: self.tool,
            points,
        });
        Outcome::Nothing
    }

    /// Ends a polygon or cloud: the corners clicked so far, if there are at least three.
    fn finish_shape(&mut self) -> Outcome {
        let Some(draft) = self.draft.take_if(|d| d.tool.by_corners()) else {
            return Outcome::Nothing;
        };
        // Drop the point following the pointer, and repeats from a double-click.
        let mut points: Vec<(f32, f32)> = Vec::new();
        for p in draft.points.iter().take(draft.points.len().saturating_sub(1)) {
            if points
                .last()
                .is_none_or(|q| (q.0 - p.0).hypot(q.1 - p.1) >= CLICK_SLOP)
            {
                points.push(*p);
            }
        }
        if draft.tool != Tool::PolyLine
            && points.len() > 3
            && let (Some(first), Some(last)) = (points.first(), points.last())
            && (first.0 - last.0).hypot(first.1 - last.1) < CLICK_SLOP * 2.0
        {
            points.pop();
        }
        if points.len() < draft.tool.min_corners() {
            return Outcome::Nothing;
        }
        self.record();
        let kind = if draft.tool == Tool::PolyLine {
            Kind::PolyLine(points)
        } else {
            Kind::Polygon {
                points,
                cloud: draft.tool == Tool::Cloud,
            }
        };
        self.push(draft.page, kind, draft.tool, String::new());
        Outcome::Changed
    }

    pub fn drag(&mut self, page: usize, at: (f32, f32)) {
        if self.moving.is_some() {
            let on_page = self.picked_annotation().is_some_and(|a| a.page == page);
            if let Some(m) = self.moving.as_mut().filter(|_| on_page) {
                m.to = at;
            }
            return;
        }
        let Some(draft) = self.draft.as_mut().filter(|d| d.page == page) else {
            return;
        };
        if draft.tool.by_corners() {
            if let Some(last) = draft.points.last_mut() {
                *last = at;
            }
        } else if matches!(draft.tool, Tool::Pen | Tool::Eraser) {
            let far = draft
                .points
                .last()
                .is_none_or(|(x, y)| (x - at.0).hypot(y - at.1) > 0.75);
            if far && draft.points.len() < 20_000 {
                draft.points.push(at);
            }
        } else {
            draft.points.truncate(1);
            draft.points.push(at);
        }
    }

    pub fn release(&mut self) -> Outcome {
        if let Some(m) = self.moving.take() {
            let Some(b) = self.picked_annotation().map(|a| Comment::from(a).at) else {
                return Outcome::Nothing;
            };
            if m.anchor.is_some() {
                let to = resized(&m);
                let same =
                    (to.x0 - b.x0).abs() + (to.y0 - b.y0).abs() + (to.x1 - b.x1).abs() + (to.y1 - b.y1).abs();
                if same < 1.0 || !self.edit(true, |a| a.resize(b, to)) {
                    return Outcome::Nothing;
                }
                return Outcome::Changed;
            }
            let (dx, dy) = clamp_shift(b, &m);
            if dx.hypot(dy) < 1.0 || !self.edit(true, |a| a.translate(dx, dy)) {
                return Outcome::Nothing;
            }
            return Outcome::Changed;
        }
        // Polygons and clouds go on until they are finished.
        let Some(draft) = self.draft.take_if(|d| !d.tool.by_corners()) else {
            return Outcome::Nothing;
        };
        let start = draft.points.first().copied().unwrap_or_default();
        let end = draft.points.last().copied().unwrap_or(start);
        let dragged = (end.0 - start.0).hypot(end.1 - start.1) >= CLICK_SLOP;
        let rect = TextBox {
            x0: start.0.min(end.0),
            y0: start.1.min(end.1),
            x1: start.0.max(end.0),
            y1: start.1.max(end.1),
        };
        let kind = match draft.tool {
            Tool::Note | Tool::TextBox => {
                let at = if draft.tool == Tool::Note || !dragged {
                    TextBox {
                        x0: start.0,
                        y0: start.1,
                        x1: start.0 + TEXT_BOX_WIDTH,
                        y1: start.1 + NOTE_SIZE,
                    }
                } else {
                    rect
                };
                self.editing = Some(Editing {
                    page: draft.page,
                    tool: draft.tool,
                    at,
                    content: text_editor::Content::new(),
                    target: None,
                    reply: None,
                    replacing: Vec::new(),
                });
                return Outcome::Nothing;
            }
            Tool::Pen => Kind::Ink(vec![draft.points]),
            Tool::Eraser => return self.erase(draft.page, &draft.points),
            Tool::InsertText => {
                self.editing = Some(Editing {
                    page: draft.page,
                    tool: Tool::InsertText,
                    at: caret_box(end.0, end.1, CARET_SIZE),
                    content: text_editor::Content::new(),
                    target: None,
                    reply: None,
                    replacing: Vec::new(),
                });
                return Outcome::Nothing;
            }
            Tool::Stamp | Tool::Sign => {
                self.sign.pad = None;
                self.stamping = Some(Stamping {
                    page: draft.page,
                    at: if dragged {
                        rect
                    } else {
                        TextBox {
                            x0: start.0,
                            y0: start.1,
                            x1: start.0,
                            y1: start.1,
                        }
                    },
                    custom: String::new(),
                    signing: draft.tool == Tool::Sign,
                });
                return Outcome::Nothing;
            }
            _ if !dragged => return Outcome::Nothing,
            Tool::Rect => Kind::Rect(rect),
            Tool::Ellipse => Kind::Ellipse(rect),
            Tool::Arrow | Tool::Line => Kind::Line {
                from: start,
                to: end,
                arrow: draft.tool == Tool::Arrow,
            },
            Tool::Select
            | Tool::Highlight
            | Tool::Underline
            | Tool::StrikeOut
            | Tool::Replace
            | Tool::Polygon
            | Tool::PolyLine
            | Tool::Cloud => {
                return Outcome::Nothing;
            }
        };
        self.record();
        self.push(draft.page, kind, draft.tool, String::new());
        Outcome::Changed
    }

    /// The Comment toolbar's buttons: the tools, colours, opacity, and undo and redo, laid
    /// out for a column when `vertical`.
    pub fn bar_items(&self, vertical: bool) -> Vec<Element<'_, Message>> {
        let ev = Message::Annotate;
        let mut items: Vec<Element<'_, Message>> = Tool::ALL
            .into_iter()
            .map(|tool| icons::toggle(tool.icon(), tool.tip(), self.tool == tool, ev(Event::Tool(tool))))
            .collect();
        items.push(divider(vertical));
        let current = self.picked_annotation().map_or_else(|| self.color(), |a| a.color);
        let swatches = SWATCHES
            .iter()
            .enumerate()
            .map(|(i, color)| swatch(*color, *color == current, ev(Event::Color(i))));
        items.push(if vertical {
            Column::with_children(swatches).spacing(4).align_x(Center).into()
        } else {
            Row::with_children(swatches).spacing(4).align_y(Center).into()
        });
        let tip = format!("Opacity {}%", (self.opacity() * 100.0).round());
        items.push(if vertical {
            icons::with_tip(
                vertical_slider(0.1..=1.0, self.opacity(), move |v| ev(Event::Opacity(v)))
                    .step(0.05_f32)
                    .on_release(ev(Event::OpacityDone))
                    .height(72),
                tip,
            )
        } else {
            icons::with_tip(
                slider(0.1..=1.0, self.opacity(), move |v| ev(Event::Opacity(v)))
                    .step(0.05_f32)
                    .on_release(ev(Event::OpacityDone))
                    .width(72),
                tip,
            )
        });
        items.push(divider(vertical));
        let m = icons::MOD;
        items.push(icons::button(
            Icon::Undo,
            format!("Undo ({m}Z)"),
            self.can_undo().then(|| ev(Event::Undo)),
        ));
        items.push(icons::button(
            Icon::Redo,
            format!("Redo ({m}Y)"),
            self.can_redo().then(|| ev(Event::Redo)),
        ));
        items
    }

    /// The card for typing a note's or text box's text.
    pub fn editor(&self) -> Option<Element<'_, Message>> {
        if let Some(stamping) = &self.stamping {
            if stamping.signing {
                return Some(self.sign.card());
            }
            return Some(stamp_card(stamping));
        }
        let editing = self.editing.as_ref()?;
        let ev = Message::Annotate;
        let title = match (editing.tool, editing.target.is_some()) {
            _ if editing.reply.is_some() => "Reply",
            (Tool::Note, false) => "Note",
            (Tool::Note, true) => "Edit note",
            (Tool::TextBox, false) => "Text box",
            (Tool::TextBox, true) => "Edit text box",
            (Tool::Replace, _) => "Replace with",
            (Tool::InsertText, _) => "Insert text",
            _ => "Comment",
        };
        let done = match (editing.target.is_some(), editing.reply.is_some()) {
            (true, _) => "Save",
            (_, true) => "Reply",
            _ => "Add",
        };
        let card = container(
            column![
                text(title).size(16),
                text_editor(&editing.content)
                    .id(editor_id())
                    .placeholder("Type here")
                    .on_action(move |a| ev(Event::Edit(a)))
                    .height(120),
                row![
                    space::horizontal(),
                    button("Cancel")
                        .style(button::secondary)
                        .on_press(ev(Event::EditCancel)),
                    button(done).on_press(ev(Event::EditDone)),
                ]
                .spacing(8),
            ]
            .spacing(10)
            .width(340),
        )
        .padding(16)
        .style(container::rounded_box);
        Some(card.into())
    }
}

/// A round colour button, named in its tip, ringed when `selected`.
pub fn swatch<'a>(color: Color, selected: bool, on_press: Message) -> Element<'a, Message> {
    let c = to_iced(color, 1.0);
    let name = SWATCHES
        .iter()
        .position(|s| *s == color)
        .and_then(|i| COLOR_NAMES.get(i))
        .copied()
        .unwrap_or("");
    let button = button(space().width(16).height(16))
        .padding(0)
        .style(move |theme: &Theme, _| button::Style {
            background: Some(c.into()),
            border: iced::Border {
                color: if selected {
                    theme.extended_palette().primary.strong.color
                } else {
                    theme.extended_palette().background.strong.color
                },
                width: if selected { 3.0 } else { 1.0 },
                radius: 9.0.into(),
            },
            ..button::Style::default()
        })
        .on_press(on_press);
    icons::with_tip(button, name)
}

/// The card for choosing which stamp to place.
fn stamp_card(stamping: &Stamping) -> Element<'_, Message> {
    let ev = Message::Annotate;
    let mut grid = Column::new().spacing(6);
    let (pairs, odd) = STAMPS.as_chunks::<2>();
    let stamp = |i: usize, name: &str| {
        let label = stamp_label(name);
        button(text(label).size(12).width(Fill).align_x(Center))
            .style(button::secondary)
            .width(Fill)
            .on_press(ev(Event::Stamp(i)))
    };
    for (row_index, pair) in pairs.iter().enumerate() {
        let i = row_index * 2;
        grid = grid.push(row![stamp(i, pair[0]), stamp(i + 1, pair[1])].spacing(6));
    }
    for name in odd {
        grid = grid.push(stamp(STAMPS.len() - 1, name));
    }
    let custom = row![
        text_input("Your own words", &stamping.custom)
            .id(stamp_input_id())
            .on_input(move |v| ev(Event::StampText(v)))
            .on_submit(ev(Event::StampCustom)),
        button("Add").on_press_maybe((!stamping.custom.trim().is_empty()).then(|| ev(Event::StampCustom))),
    ]
    .spacing(6);
    let picture = button(text("A picture, such as a signature or logo…").size(13))
        .style(button::secondary)
        .width(Fill)
        .on_press(Message::StampPicture);
    container(
        column![
            text("Stamp").size(16),
            grid,
            custom,
            picture,
            row![
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(ev(Event::EditCancel)),
            ],
        ]
        .spacing(10)
        .width(340),
    )
    .padding(16)
    .style(container::rounded_box)
    .into()
}

pub fn stamp_input_id() -> iced::widget::Id {
    iced::widget::Id::new("stamp-text")
}

/// The usual colour for a standard stamp: green for approval, red for warnings, blue otherwise.
fn stamp_color(name: &str) -> Color {
    match name {
        "Approved" | "Final" | "ForPublicRelease" => SWATCHES[1],
        "NotApproved" | "Confidential" | "TopSecret" | "Expired" | "NotForPublicRelease" => SWATCHES[3],
        _ => SWATCHES[2],
    }
}

const COLOR_NAMES: [&str; 6] = ["Yellow", "Green", "Blue", "Red", "Purple", "Black"];

pub fn editor_id() -> iced::widget::Id {
    iced::widget::Id::new("annotation-text")
}

fn divider<'a>(vertical: bool) -> Element<'a, Message> {
    let (width, height) = if vertical { (20, 1) } else { (1, 20) };
    container(space().width(width).height(height))
        .style(|theme: &Theme| {
            container::Style::default().background(theme.extended_palette().background.strong.color)
        })
        .into()
}

/// A text box just big enough for `text`, keeping the chosen width when the box was dragged.
fn fit_text_box(at: TextBox, text: &str) -> TextBox {
    let lines = wrap(text, f64::from((at.width() - 4.0).max(10.0) / TEXT_SIZE));
    let widest = lines.iter().map(|l| text_width(l)).fold(0.0, f64::max) as f32 * TEXT_SIZE + 6.0;
    let width = if at.width() >= TEXT_BOX_WIDTH {
        widest.min(at.width())
    } else {
        at.width()
    };
    let height = (lines.len() as f32 * TEXT_SIZE * LINE_HEIGHT + 6.0).max(at.height());
    TextBox {
        x0: at.x0,
        y0: at.y0,
        x1: at.x0 + width.max(12.0),
        y1: at.y0 + height,
    }
}

/// Size of a caret placed with a click, in points.
const CARET_SIZE: f32 = 10.0;
/// How far the eraser reaches either side of the pointer, in points.
const ERASER_RADIUS: f32 = 5.0;

/// The box of a caret whose tip is at (x, y), sized for text `height` points tall.
fn caret_box(x: f32, y: f32, height: f32) -> TextBox {
    let size = (height * 0.6).clamp(5.0, 24.0);
    TextBox {
        x0: x - size / 2.0,
        y0: y,
        x1: x + size / 2.0,
        y1: y + size,
    }
}

/// Ink strokes, each a list of points.
type Strokes = Vec<Vec<(f32, f32)>>;

/// Splits strokes where they pass within the eraser's reach of `path`; `None` when nothing
/// was touched.
fn erase_strokes(strokes: &[Vec<(f32, f32)>], path: &[(f32, f32)]) -> Option<Strokes> {
    // Distance from p to the segment a–b.
    let gap = |p: &(f32, f32), a: &(f32, f32), b: &(f32, f32)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = dx * dx + dy * dy;
        let t = if len > 0.0 {
            (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
    };
    let near = |p: &(f32, f32)| match path {
        [only] => gap(p, only, only) <= ERASER_RADIUS,
        _ => path.windows(2).any(|w| match w {
            [a, b] => gap(p, a, b) <= ERASER_RADIUS,
            _ => false,
        }),
    };
    let mut touched = false;
    let mut out = Vec::new();
    for stroke in strokes {
        let mut piece: Vec<(f32, f32)> = Vec::new();
        for p in stroke {
            if near(p) {
                touched = true;
                if !piece.is_empty() {
                    out.push(std::mem::take(&mut piece));
                }
            } else {
                piece.push(*p);
            }
        }
        if !piece.is_empty() {
            out.push(piece);
        }
    }
    touched.then_some(out)
}

/// Kinds that can be stretched by their corners: not notes (icon-sized) or text markup.
fn can_resize(a: &Annotation) -> bool {
    matches!(
        a.kind,
        Kind::Rect(_)
            | Kind::Ellipse(_)
            | Kind::FreeText { .. }
            | Kind::Ink(_)
            | Kind::Line { .. }
            | Kind::Polygon { .. }
            | Kind::PolyLine(_)
            | Kind::Stamp { .. }
    )
}

/// Top-left, top-right, bottom-left, bottom-right; reversed, each corner's opposite.
fn corners(b: TextBox) -> [(f32, f32); 4] {
    [(b.x0, b.y0), (b.x1, b.y0), (b.x0, b.y1), (b.x1, b.y1)]
}

/// The box while a corner is dragged: from the fixed corner to the pointer, kept on the page
/// and at least a few points across.
fn resized(m: &Moving) -> TextBox {
    let (ax, ay) = m.anchor.unwrap_or(m.from);
    let x = m.to.0.clamp(0.0, m.page.0.max(0.0));
    let y = m.to.1.clamp(0.0, m.page.1.max(0.0));
    const MIN: f32 = 4.0;
    let x = if (x - ax).abs() < MIN {
        ax + MIN.copysign(x - ax)
    } else {
        x
    };
    let y = if (y - ay).abs() < MIN {
        ay + MIN.copysign(y - ay)
    } else {
        y
    };
    TextBox {
        x0: ax.min(x),
        y0: ay.min(y),
        x1: ax.max(x),
        y1: ay.max(y),
    }
}

/// New text for a comment; a text box grows or shrinks in height to fit it.
fn set_text(a: &mut Annotation, text: String) {
    if let Kind::FreeText { rect, size } = &mut a.kind {
        let size_f = size.max(1.0);
        let lines = wrap(&text, f64::from((rect.width() - 4.0).max(10.0) / size_f));
        let height = lines.len().max(1) as f32 * size_f * LINE_HEIGHT + 6.0;
        rect.y1 = rect.y0 + height;
    }
    a.contents = text;
}

/// Whether a point (in page points) is on an annotation, with a little slack for thin lines.
fn hits(a: &Annotation, (x, y): (f32, f32)) -> bool {
    match &a.kind {
        Kind::Reply { .. } => false,
        Kind::Highlight(boxes) | Kind::Underline(boxes) | Kind::StrikeOut(boxes) => {
            boxes.iter().any(|b| grown(*b, 1.0).contains(x, y))
        }
        _ => grown(Comment::from(a).at, a.width.max(4.0)).contains(x, y),
    }
}

fn grown(b: TextBox, by: f32) -> TextBox {
    TextBox {
        x0: b.x0 - by,
        y0: b.y0 - by,
        x1: b.x1 + by,
        y1: b.y1 + by,
    }
}

/// How far a drag moves a comment with outline `b`, keeping it on the page.
fn clamp_shift(b: TextBox, m: &Moving) -> (f32, f32) {
    // A comment already partly off the page can still move, just not further off.
    let clamp = |d: f32, lo: f32, hi: f32, size: f32| d.clamp((-lo).min(0.0), (size - hi).max(0.0));
    (
        clamp(m.to.0 - m.from.0, b.x0, b.x1, m.page.0),
        clamp(m.to.1 - m.from.1, b.y0, b.y1, m.page.1),
    )
}

pub fn to_iced(c: Color, alpha: f32) -> iced::Color {
    let [r, g, b] = c.0;
    iced::Color::from_rgba(r, g, b, alpha)
}

/// Draws a shape being dragged out, over the page it is on.
pub struct Preview<'a> {
    pub draft: &'a Draft,
    pub map: PageMap,
    pub color: Color,
}

impl canvas::Program<Message> for Preview<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let view: Vec<Point> = self
            .draft
            .points
            .iter()
            .map(|(x, y)| self.map.to_view(*x, *y))
            .collect();
        let (Some(first), Some(last)) = (view.first().copied(), view.last().copied()) else {
            return vec![frame.into_geometry()];
        };
        let stroke = Stroke::default()
            .with_color(to_iced(self.color, 1.0))
            .with_width(STROKE_WIDTH * self.map.zoom);
        let top_left = Point::new(first.x.min(last.x), first.y.min(last.y));
        let size = Size::new((first.x - last.x).abs(), (first.y - last.y).abs());
        let path = match self.draft.tool {
            Tool::Pen | Tool::Eraser | Tool::Polygon | Tool::PolyLine | Tool::Cloud => Path::new(|b| {
                b.move_to(first);
                for p in &view {
                    b.line_to(*p);
                }
                if matches!(self.draft.tool, Tool::Polygon | Tool::Cloud) && view.len() > 2 {
                    b.close();
                }
            }),
            Tool::Rect | Tool::TextBox | Tool::Stamp | Tool::Sign => Path::rectangle(top_left, size),
            Tool::Ellipse => Path::new(|b| {
                b.ellipse(canvas::path::arc::Elliptical {
                    center: Point::new(top_left.x + size.width / 2.0, top_left.y + size.height / 2.0),
                    radii: iced::Vector::new(size.width / 2.0, size.height / 2.0),
                    rotation: iced::Radians(0.0),
                    start_angle: iced::Radians(0.0),
                    end_angle: iced::Radians(std::f32::consts::TAU),
                });
            }),
            _ => Path::line(first, last),
        };
        frame.stroke(&path, stroke);
        vec![frame.into_geometry()]
    }
}

/// The preview layer for a page, if something is being drawn on it.
pub fn preview<'a>(
    annotate: &'a Annotate,
    page: usize,
    map: PageMap,
    size: Size,
) -> Option<Element<'a, Message>> {
    let draft = annotate.draft.as_ref().filter(|d| d.page == page)?;
    Some(
        canvas::Canvas::new(Preview {
            draft,
            map,
            color: annotate.color(),
        })
        .width(size.width)
        .height(size.height)
        .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_a_rectangle_and_undoing_it() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::Rect));
        a.press(0, (10.0, 10.0));
        a.drag(0, (60.0, 40.0));
        assert!(matches!(a.release(), Outcome::Changed));
        assert!(matches!(a.items.first().map(|i| &i.kind), Some(Kind::Rect(r)) if r.x1 == 60.0));
        assert!(matches!(a.update(Event::Undo), Outcome::Changed));
        assert!(a.items.is_empty() && a.can_redo());
        a.update(Event::Redo);
        assert_eq!(a.items.len(), 1);
    }

    #[test]
    fn shift_keys_step_through_groups() {
        assert_eq!(Tool::Highlight.next_in(&Tool::MARKUPS), Tool::Underline);
        assert_eq!(Tool::StrikeOut.next_in(&Tool::MARKUPS), Tool::Highlight);
        assert_eq!(Tool::Note.next_in(&Tool::DRAWING), Tool::Cloud);
        assert_eq!(Tool::Pen.next_in(&Tool::DRAWING), Tool::Eraser);
        assert_eq!(Tool::Eraser.next_in(&Tool::DRAWING), Tool::Cloud);
        for tool in Tool::DRAWING {
            assert!(tool.key().is_some());
        }
    }

    #[test]
    fn a_click_with_a_shape_tool_adds_nothing() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::Ellipse));
        a.press(0, (10.0, 10.0));
        assert!(matches!(a.release(), Outcome::Nothing));
        assert!(a.items.is_empty());
    }

    #[test]
    fn a_note_waits_for_its_text() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::Note));
        a.press(2, (100.0, 50.0));
        a.release();
        assert!(a.editing.is_some() && a.items.is_empty());
        a.update(Event::Edit(text_editor::Action::Edit(text_editor::Edit::Insert(
            'x',
        ))));
        assert!(matches!(a.update(Event::EditDone), Outcome::Changed));
        assert_eq!(
            a.items.first().map(|i| (i.page, i.contents.as_str())),
            Some((2, "x"))
        );
    }

    #[test]
    fn fill_and_sign_places_marks_the_date_and_a_signature_at_the_click() {
        use crate::sign::Event as Sign;
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::Sign));
        a.press(0, (100.0, 50.0));
        a.release();
        assert!(a.stamping.as_ref().is_some_and(|s| s.signing));
        assert!(matches!(a.update(Event::Sign(Sign::Check)), Outcome::Changed));
        assert!(
            matches!(a.items.first(), Some(Annotation { kind: Kind::Ink(lines), .. }) if lines.len() == 1)
        );
        assert!(a.stamping.is_none());

        a.press(0, (20.0, 30.0));
        a.release();
        assert!(matches!(a.update(Event::Sign(Sign::Date)), Outcome::Changed));
        assert!(matches!(
            a.items.get(1),
            Some(Annotation { kind: Kind::FreeText { rect, .. }, contents, color, .. })
                if rect.x0 == 20.0 && contents.len() == 10 && *color == SWATCHES[5]
        ));

        a.press(0, (40.0, 200.0));
        a.release();
        a.update(Event::Sign(Sign::Create(crate::sign::Which::Initials)));
        a.update(Event::Sign(Sign::PenDown(Point::new(0.0, 0.0))));
        a.update(Event::Sign(Sign::PenMove(Point::new(30.0, 20.0))));
        a.update(Event::Sign(Sign::PenUp));
        assert!(matches!(a.update(Event::Sign(Sign::Use)), Outcome::Changed));
        let Some(Annotation {
            kind:
                Kind::Stamp {
                    rect,
                    name,
                    image: Some(_),
                },
            ..
        }) = a.items.get(2)
        else {
            panic!("no initials: {:?}", a.items.get(2));
        };
        assert_eq!(name, "Initials");
        assert!((rect.x0 - 40.0).abs() < 0.01 && (rect.height() - INITIALS_HEIGHT).abs() < 0.01);
        assert!(a.sign.made(crate::sign::Which::Initials).is_some());
    }

    #[test]
    fn a_stamp_is_chosen_then_placed_on_the_click() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::Stamp));
        a.press(1, (100.0, 50.0));
        a.release();
        assert!(a.stamping.is_some() && a.items.is_empty());
        assert!(matches!(a.update(Event::Stamp(0)), Outcome::Changed));
        let Some(Annotation {
            page: 1,
            kind: Kind::Stamp { rect, name, .. },
            color,
            ..
        }) = a.items.first()
        else {
            panic!("no stamp: {:?}", a.items);
        };
        assert_eq!(name, "Approved");
        assert_eq!(*color, SWATCHES[1]);
        assert!((rect.x0 + rect.x1 - 200.0).abs() < 0.01 && (rect.height() - STAMP_HEIGHT).abs() < 0.01);
        assert!(a.stamping.is_none());

        // A custom stamp, dragged to size, in the colour picked for the tool.
        a.update(Event::Color(4));
        a.press(0, (10.0, 10.0));
        a.drag(0, (110.0, 40.0));
        a.release();
        a.update(Event::StampText("Checked by Jo".into()));
        assert!(matches!(a.update(Event::StampCustom), Outcome::Changed));
        assert!(matches!(
            a.items.get(1),
            Some(Annotation { kind: Kind::Stamp { rect, name, .. }, color, .. })
                if name == "Checked by Jo" && rect.x1 == 110.0 && *color == SWATCHES[4]
        ));
    }

    #[test]
    fn colours_are_remembered_per_tool() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::Pen));
        a.update(Event::Color(3));
        a.update(Event::Tool(Tool::Highlight));
        assert_eq!(a.color(), SWATCHES[0]);
        a.update(Event::Tool(Tool::Pen));
        assert_eq!(a.color(), SWATCHES[3]);
    }

    #[test]
    fn empty_text_boxes_are_dropped() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::TextBox));
        a.press(0, (0.0, 0.0));
        a.release();
        assert!(matches!(a.update(Event::EditDone), Outcome::Nothing));
        assert!(a.items.is_empty());
    }

    #[test]
    fn deleting_can_be_undone() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::Rect));
        for x in [10.0, 100.0] {
            a.press(0, (x, 10.0));
            a.drag(0, (x + 50.0, 40.0));
            a.release();
        }
        assert!(matches!(a.update(Event::Delete(0)), Outcome::Changed));
        assert!(matches!(a.items.as_slice(), [i] if matches!(i.kind, Kind::Rect(r) if r.x0 == 100.0)));
        a.update(Event::Undo);
        assert_eq!(a.items.len(), 2);
        assert!(matches!(a.update(Event::Delete(9)), Outcome::Nothing));
    }

    #[test]
    fn marking_across_pages_is_one_undo_step() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::Highlight));
        let line = TextBox {
            x0: 0.0,
            y0: 0.0,
            x1: 10.0,
            y1: 10.0,
        };
        a.mark_text(vec![(0, vec![line]), (1, vec![line])]);
        assert_eq!(a.items.len(), 2);
        a.update(Event::Undo);
        assert!(a.items.is_empty() && !a.can_undo());
    }

    #[test]
    fn removing_a_saved_comment_is_undoable_and_dirty() {
        let mut a = Annotate::new();
        let id = CommentId {
            page: 0,
            annot: panna_core::ObjRef {
                number: 9,
                generation: 0,
            },
            popup: None,
        };
        assert!(!a.dirty());
        assert!(matches!(a.update(Event::Remove(id)), Outcome::Changed));
        assert!(a.dirty() && a.changes().remove == [id]);
        assert!(matches!(a.update(Event::Remove(id)), Outcome::Nothing));
        a.update(Event::Undo);
        assert!(!a.dirty());
        a.update(Event::Redo);
        assert_eq!(a.removed, [id]);
    }

    fn rect_at(a: &mut Annotate, x: f32) {
        a.update(Event::Tool(Tool::Rect));
        a.press(0, (x, 10.0));
        a.drag(0, (x + 50.0, 40.0));
        a.release();
        a.update(Event::Tool(Tool::Select));
    }

    #[test]
    fn dragging_a_comment_moves_it_within_the_page() {
        let mut a = Annotate::new();
        rect_at(&mut a, 10.0);
        assert!(a.grab(0, (30.0, 20.0), (200.0, 100.0), 3.0));
        a.drag(0, (60.0, 500.0));
        assert!(matches!(a.release(), Outcome::Changed));
        // Down as far as the page allows, across by the full drag.
        assert!(
            matches!(a.items.first().map(|i| &i.kind), Some(Kind::Rect(r)) if r.x0 == 40.0 && r.y1 == 100.0),
            "{:?}",
            a.items
        );
        a.update(Event::Undo);
        assert!(matches!(a.items.first().map(|i| &i.kind), Some(Kind::Rect(r)) if r.x0 == 10.0));
        assert!(!a.grab(0, (190.0, 90.0), (200.0, 100.0), 3.0) && a.picked.is_none());
    }

    #[test]
    fn dragging_a_corner_resizes_the_selected_comment() {
        let mut a = Annotate::new();
        rect_at(&mut a, 10.0);
        // Not selected yet: the corner just selects it (and would move it).
        assert!(a.grab(0, (60.0, 40.0), (200.0, 100.0), 3.0) && !a.is_resizing());
        a.release();
        // Now the bottom-right corner stretches it, with the top-left fixed.
        assert_eq!(a.handle_at(0, (59.0, 41.0), 3.0), Some((10.0, 10.0)));
        assert!(a.grab(0, (60.0, 40.0), (200.0, 100.0), 3.0) && a.is_resizing());
        a.drag(0, (110.0, 300.0));
        assert!(matches!(a.release(), Outcome::Changed));
        assert_eq!(
            a.items.first().map(|i| &i.kind),
            Some(&Kind::Rect(TextBox {
                x0: 10.0,
                y0: 10.0,
                x1: 110.0,
                y1: 100.0
            }))
        );
        a.update(Event::Undo);
        assert!(matches!(a.items.first().map(|i| &i.kind), Some(Kind::Rect(r)) if r.x1 == 60.0));
    }

    #[test]
    fn editing_a_text_box_changes_its_text_and_height() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::TextBox));
        a.press(0, (10.0, 10.0));
        a.release();
        a.editing.as_mut().unwrap().content = text_editor::Content::with_text("Hi");
        a.update(Event::EditDone);
        let before = a.items[0].clone();
        a.update(Event::Tool(Tool::Select));
        assert!(a.edit_text(0, (12.0, 12.0)));
        assert_eq!(a.editing.as_ref().unwrap().content.text().trim_end(), "Hi");
        a.editing.as_mut().unwrap().content = text_editor::Content::with_text("one\ntwo\nthree");
        assert!(matches!(a.update(Event::EditDone), Outcome::Changed));
        let after = &a.items[0];
        assert_eq!(after.contents, "one\ntwo\nthree");
        let height = |k: &Kind| match k {
            Kind::FreeText { rect, .. } => rect.height(),
            _ => 0.0,
        };
        assert!(height(&after.kind) > height(&before.kind) * 2.0);
        // Nothing to save when the text is left as it was.
        assert!(a.edit_text(0, (12.0, 12.0)));
        assert!(matches!(a.update(Event::EditDone), Outcome::Nothing));
        a.update(Event::Undo);
        assert_eq!(a.items[0], before);
    }

    #[test]
    fn a_polygon_line_stays_open() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::PolyLine));
        a.press(0, (10.0, 10.0));
        a.press(0, (60.0, 10.0));
        // Clicking near the first point adds a point rather than closing the line.
        a.press(0, (11.0, 11.0));
        assert!(matches!(a.update(Event::FinishShape), Outcome::Changed));
        assert!(matches!(a.items.first().map(|i| &i.kind), Some(Kind::PolyLine(p)) if p.len() == 3));
        // Two points are enough for a line.
        a.press(0, (1.0, 1.0));
        a.press(0, (40.0, 1.0));
        a.press(0, (40.0, 1.0));
        assert!(matches!(a.items.get(1).map(|i| &i.kind), Some(Kind::PolyLine(p)) if p.len() == 2));
    }

    #[test]
    fn the_eraser_splits_and_removes_drawings() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::Pen));
        a.press(0, (0.0, 0.0));
        for x in 1..=40 {
            a.drag(0, (x as f32, 0.0));
        }
        a.release();
        a.update(Event::Tool(Tool::Eraser));
        a.press(0, (20.0, -10.0));
        a.drag(0, (20.0, 10.0));
        assert!(matches!(a.release(), Outcome::Changed));
        assert!(matches!(a.items.first().map(|i| &i.kind), Some(Kind::Ink(s)) if s.len() == 2));
        // Rubbing over everything takes the drawing away; undo brings it back.
        a.press(0, (0.0, 0.0));
        for x in 1..=40 {
            a.drag(0, (x as f32, 0.0));
        }
        assert!(matches!(a.release(), Outcome::Changed));
        assert!(a.items.is_empty());
        a.update(Event::Undo);
        assert_eq!(a.items.len(), 1);
        // Missing every drawing changes nothing.
        a.press(0, (200.0, 200.0));
        assert!(matches!(a.release(), Outcome::Nothing));
    }

    #[test]
    fn replacing_text_crosses_out_and_adds_a_caret() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::Replace));
        let word = TextBox {
            x0: 10.0,
            y0: 10.0,
            x1: 50.0,
            y1: 22.0,
        };
        assert!(matches!(a.mark_text(vec![(0, vec![word])]), Outcome::Nothing));
        assert!(a.editing.is_some());
        a.update(Event::Edit(text_editor::Action::Edit(text_editor::Edit::Paste(
            std::sync::Arc::new("new words".into()),
        ))));
        assert!(matches!(a.update(Event::EditDone), Outcome::Changed));
        assert!(matches!(
            a.items.first().map(|i| &i.kind),
            Some(Kind::StrikeOut(_))
        ));
        assert!(matches!(
            a.items.get(1).map(|i| &i.kind),
            Some(Kind::Caret { .. })
        ));
        assert!(a.items.iter().all(|i| i.contents == "new words"));
        // One undo takes both away.
        a.update(Event::Undo);
        assert!(a.items.is_empty());
    }

    #[test]
    fn inserting_text_needs_words() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::InsertText));
        a.press(0, (30.0, 40.0));
        assert!(matches!(a.release(), Outcome::Nothing));
        assert!(matches!(a.update(Event::EditDone), Outcome::Nothing));
        assert!(a.items.is_empty());
    }

    #[test]
    fn a_cloud_takes_a_corner_per_click() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::Cloud));
        for at in [(10.0, 10.0), (100.0, 10.0), (100.0, 80.0)] {
            a.press(0, at);
            assert!(matches!(a.release(), Outcome::Nothing));
            a.drag(0, (at.0 + 5.0, at.1 + 5.0));
        }
        // A double-click's second press lands on the last corner again, which finishes it.
        assert!(matches!(a.press(0, (100.0, 80.0)), Outcome::Changed));
        assert!(matches!(a.update(Event::FinishShape), Outcome::Nothing));
        assert_eq!(
            a.items.first().map(|i| &i.kind),
            Some(&Kind::Polygon {
                points: vec![(10.0, 10.0), (100.0, 10.0), (100.0, 80.0)],
                cloud: true
            })
        );
        // Clicking the first corner again closes the shape too.
        a.update(Event::Tool(Tool::Polygon));
        for at in [(10.0, 10.0), (50.0, 10.0), (50.0, 50.0)] {
            a.press(0, at);
        }
        assert!(matches!(a.press(0, (11.0, 11.0)), Outcome::Changed));
        assert!(
            matches!(a.items.get(1).map(|i| &i.kind), Some(Kind::Polygon { points, cloud: false }) if points.len() == 3)
        );
        // Two corners aren't a shape; Escape drops what's drawn.
        a.press(0, (1.0, 1.0));
        a.press(0, (9.0, 1.0));
        assert!(matches!(a.update(Event::FinishShape), Outcome::Nothing));
        a.press(0, (1.0, 1.0));
        a.update(Event::CancelShape);
        assert!(a.draft.is_none() && a.items.len() == 2);
    }

    #[test]
    fn replies_and_statuses_join_a_saved_comment() {
        let mut a = Annotate::new();
        a.author = "Ann".into();
        let parent = CommentId {
            page: 0,
            annot: ObjRef {
                number: 5,
                generation: 0,
            },
            popup: None,
        };
        let at = TextBox {
            x0: 1.0,
            y0: 2.0,
            x1: 21.0,
            y1: 22.0,
        };
        let thread = Thread {
            page: 0,
            comment: parent.annot,
            at,
        };
        // An earlier status saved in the file.
        let accepted = Annotation {
            page: 0,
            kind: Kind::Reply {
                to: parent.annot,
                at,
                state: Some(ReviewState::Accepted),
                checked: None,
            },
            color: SWATCHES[0],
            opacity: 1.0,
            width: 1.0,
            contents: String::new(),
            author: String::new(),
        };
        let mut saved = Comment::from(&accepted);
        let saved_id = CommentId {
            annot: ObjRef {
                number: 9,
                generation: 0,
            },
            ..parent
        };
        saved.id = Some(saved_id);
        saved.editable = Some(accepted);
        a.saved(&[saved]);
        assert_eq!(a.status(parent.annot), ReviewState::Accepted);
        // Replies aren't hit on the page.
        assert_eq!(a.hit(0, (10.0, 10.0)), None);

        a.update(Event::Reply(thread));
        a.editing.as_mut().unwrap().content = text_editor::Content::with_text("Looks good");
        assert!(matches!(a.update(Event::EditDone), Outcome::Changed));
        assert!(
            matches!(&a.items[0].kind, Kind::Reply { state: None, .. })
                && a.items[0].contents == "Looks good"
        );

        assert!(matches!(
            a.update(Event::SetStatus(thread, ReviewState::Rejected)),
            Outcome::Changed
        ));
        assert_eq!(a.items[1].contents, "Rejected set by Ann");
        assert_eq!(a.status(parent.annot), ReviewState::Rejected);
        assert!(matches!(
            a.update(Event::SetStatus(thread, ReviewState::Rejected)),
            Outcome::Nothing
        ));

        // Checkmarks toggle, and don't change the review status.
        assert!(!a.checked(parent.annot));
        a.update(Event::ToggleCheck(thread));
        assert!(a.checked(parent.annot));
        a.update(Event::ToggleCheck(thread));
        assert!(!a.checked(parent.annot));
        assert_eq!(a.status(parent.annot), ReviewState::Rejected);
        a.items.truncate(2);

        // Deleting the comment takes its whole thread with it, as one undo step.
        a.update(Event::Remove(parent));
        assert!(a.items.is_empty() && a.removed == [parent, saved_id]);
        a.update(Event::Undo);
        assert_eq!(a.items.len(), 2);
        assert_eq!(a.status(parent.annot), ReviewState::Rejected);
    }

    #[test]
    fn recolouring_the_selected_comment() {
        let mut a = Annotate::new();
        rect_at(&mut a, 10.0);
        a.pick(Picked::New(0));
        assert!(matches!(a.update(Event::Color(1)), Outcome::Changed));
        assert_eq!(a.items.first().map(|i| i.color), Some(SWATCHES[1]));
        // The tool's own colour is untouched.
        a.unpick();
        assert_eq!(a.color(), Tool::Select.default_color());
        // A whole slide is one undo step.
        a.pick(Picked::New(0));
        for v in [0.9, 0.7, 0.5] {
            a.update(Event::Opacity(v));
        }
        assert!(matches!(a.update(Event::OpacityDone), Outcome::Changed));
        assert_eq!(a.items.first().map(|i| i.opacity), Some(0.5));
        a.update(Event::Undo);
        assert_eq!(a.items.first().map(|i| i.opacity), Some(1.0));
    }

    #[test]
    fn changing_a_saved_comment_replaces_it() {
        let mut a = Annotate::new();
        let id = CommentId {
            page: 0,
            annot: panna_core::ObjRef {
                number: 5,
                generation: 0,
            },
            popup: None,
        };
        let mut shape = Annotation {
            page: 0,
            kind: Kind::Ellipse(TextBox {
                x0: 0.0,
                y0: 0.0,
                x1: 20.0,
                y1: 20.0,
            }),
            color: SWATCHES[0],
            opacity: 1.0,
            width: 1.0,
            contents: String::new(),
            author: String::new(),
        };
        let mut comment = Comment::from(&shape);
        comment.id = Some(id);
        comment.editable = Some(shape.clone());
        a.saved(&[comment]);
        assert_eq!(a.hit(0, (10.0, 10.0)), Some(Picked::Saved(id)));
        a.pick(Picked::Saved(id));
        assert!(!a.dirty());
        assert!(matches!(a.update(Event::Color(2)), Outcome::Changed));
        shape.color = SWATCHES[2];
        assert_eq!(
            (a.removed.as_slice(), a.items.as_slice()),
            (&[id][..], &[shape][..])
        );
        assert_eq!(a.picked, Some(Picked::New(0)));
        a.update(Event::Undo);
        assert!(!a.dirty());
    }
}
