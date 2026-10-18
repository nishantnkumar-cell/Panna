//! Annotation tools: the tool rail, drawing in progress, and the list of unsaved annotations
//! with undo and redo.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{
    Column, button, column, container, row, space, text, text_editor, text_input, vertical_slider,
};
use iced::{Center, Element, Fill, Point, Rectangle, Renderer, Size, Theme, mouse};
use panna_core::{
    Annotation, Changes, Color, Comment, CommentId, Kind, LINE_HEIGHT, NOTE_SIZE, ObjRef, ReviewState,
    STAMPS, TextBox, stamp_label, text_width, wrap,
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
    Note,
    TextBox,
    Pen,
    Rect,
    Ellipse,
    Arrow,
    Line,
    Polygon,
    Cloud,
    Stamp,
}

impl Tool {
    pub const ALL: [Tool; 14] = [
        Tool::Select,
        Tool::Highlight,
        Tool::Underline,
        Tool::StrikeOut,
        Tool::Note,
        Tool::TextBox,
        Tool::Pen,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Arrow,
        Tool::Line,
        Tool::Polygon,
        Tool::Cloud,
        Tool::Stamp,
    ];

    fn icon(self) -> Icon {
        match self {
            Tool::Select => Icon::Pointer,
            Tool::Highlight => Icon::Highlighter,
            Tool::Underline => Icon::Underline,
            Tool::StrikeOut => Icon::Strikethrough,
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
        }
    }

    fn tip(self) -> String {
        let what = match self {
            Tool::Select => "Select text",
            Tool::Highlight => "Highlight text",
            Tool::Underline => "Underline text",
            Tool::StrikeOut => "Strike through text",
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
        };
        format!("{what} ({})", self.key().to_ascii_uppercase())
    }

    /// The single key that picks this tool while the comment tools are open.
    pub fn key(self) -> char {
        match self {
            Tool::Select => 'v',
            Tool::Highlight => 'h',
            Tool::Underline => 'u',
            Tool::StrikeOut => 'k',
            Tool::Note => 'n',
            Tool::TextBox => 't',
            Tool::Pen => 'p',
            Tool::Rect => 'r',
            Tool::Ellipse => 'e',
            Tool::Arrow => 'a',
            Tool::Line => 'l',
            Tool::Polygon => 'g',
            Tool::Cloud => 'c',
            Tool::Stamp => 's',
        }
    }

    pub fn from_key(key: &str) -> Option<Tool> {
        Tool::ALL
            .into_iter()
            .find(|t| key.len() == 1 && key.starts_with(t.key()))
    }

    /// Tools that place one corner per click instead of drawing with a drag.
    pub fn by_corners(self) -> bool {
        matches!(self, Tool::Polygon | Tool::Cloud)
    }

    /// Tools that work on selected text, so dragging selects text as the Select tool does.
    pub fn marks_text(self) -> bool {
        matches!(
            self,
            Tool::Select | Tool::Highlight | Tool::Underline | Tool::StrikeOut
        )
    }

    fn index(self) -> usize {
        Tool::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn default_color(self) -> Color {
        match self {
            Tool::Select | Tool::Highlight | Tool::Note => SWATCHES[0],
            Tool::TextBox => SWATCHES[5],
            Tool::Pen => SWATCHES[2],
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
    /// Places the standard stamp at this index of [`STAMPS`].
    Stamp(usize),
    /// Typing a custom stamp's text.
    StampText(String),
    /// Places a stamp with the typed text.
    StampCustom,
}

/// Where a stamp goes, while its kind is being chosen.
#[derive(Debug, Clone)]
pub struct Stamping {
    page: usize,
    /// The dragged box, or a point (an empty box) for a click.
    at: TextBox,
    custom: String,
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
            }
            Event::Stamp(i) => {
                if let Some(name) = STAMPS.get(i) {
                    return self.place_stamp((*name).to_owned());
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
                    return self.place_stamp(text);
                }
            }
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
            Event::CancelShape => {
                if self.draft.as_ref().is_some_and(|d| d.tool.by_corners()) {
                    self.draft = None;
                }
            }
        }
        Outcome::Nothing
    }

    /// Adds the stamp being placed. A click centres a stamp sized to its words on the point.
    fn place_stamp(&mut self, name: String) -> Outcome {
        let Some(stamping) = self.stamping.take() else {
            return Outcome::Nothing;
        };
        let at = stamping.at;
        let rect = if at.width() >= CLICK_SLOP && at.height() >= CLICK_SLOP {
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
            },
            color: SWATCHES[0],
            opacity: 1.0,
            width: STROKE_WIDTH,
            contents,
            author: self.author.clone(),
        });
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
        if !matches!(tool, Tool::Highlight | Tool::Underline | Tool::StrikeOut)
            || per_page.iter().all(|(_, b)| b.is_empty())
        {
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
                let closes = fixed >= 3
                    && (near(draft.points.first()) || near(draft.points.get(fixed.saturating_sub(1))));
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
        if points.len() > 3
            && let (Some(first), Some(last)) = (points.first(), points.last())
            && (first.0 - last.0).hypot(first.1 - last.1) < CLICK_SLOP * 2.0
        {
            points.pop();
        }
        if points.len() < 3 {
            return Outcome::Nothing;
        }
        self.record();
        let kind = Kind::Polygon {
            points,
            cloud: draft.tool == Tool::Cloud,
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
        } else if draft.tool == Tool::Pen {
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
                });
                return Outcome::Nothing;
            }
            Tool::Pen => Kind::Ink(vec![draft.points]),
            Tool::Stamp => {
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
            | Tool::Polygon
            | Tool::Cloud => {
                return Outcome::Nothing;
            }
        };
        self.record();
        self.push(draft.page, kind, draft.tool, String::new());
        Outcome::Changed
    }

    /// The vertical tool rail shown at the edge of the page view.
    pub fn rail(&self) -> Element<'_, Message> {
        let ev = Message::Annotate;
        let mut tools = Column::new().spacing(2).align_x(Center);
        for tool in Tool::ALL {
            tools = tools.push(icons::toggle(
                tool.icon(),
                tool.tip(),
                self.tool == tool,
                ev(Event::Tool(tool)),
            ));
        }
        let current = self.picked_annotation().map_or_else(|| self.color(), |a| a.color);
        let mut swatches = Column::new().spacing(4).align_x(Center);
        for (i, color) in SWATCHES.iter().enumerate() {
            let c = to_iced(*color, 1.0);
            let selected = *color == current;
            let swatch = button(space().width(16).height(16))
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
                .on_press(ev(Event::Color(i)));
            swatches = swatches.push(icons::with_tip(swatch, COLOR_NAMES.get(i).copied().unwrap_or("")));
        }
        let opacity = icons::with_tip(
            vertical_slider(0.1..=1.0, self.opacity(), move |v| ev(Event::Opacity(v)))
                .step(0.05)
                .on_release(ev(Event::OpacityDone))
                .height(72),
            format!("Opacity {}%", (self.opacity() * 100.0).round()),
        );
        let m = icons::MOD;
        let history = column![
            icons::button(
                Icon::Undo,
                format!("Undo ({m}Z)"),
                self.can_undo().then(|| ev(Event::Undo))
            ),
            icons::button(
                Icon::Redo,
                format!("Redo ({m}Y)"),
                self.can_redo().then(|| ev(Event::Redo))
            ),
        ]
        .spacing(2);
        container(
            column![tools, divider(), swatches, opacity, divider(), history]
                .spacing(6)
                .padding(4)
                .align_x(Center),
        )
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style::default()
                .background(palette.background.base.color)
                .border(iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 8.0.into(),
                })
        })
        .into()
    }

    /// The card for typing a note's or text box's text.
    pub fn editor(&self) -> Option<Element<'_, Message>> {
        if let Some(stamping) = &self.stamping {
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
    container(
        column![
            text("Stamp").size(16),
            grid,
            custom,
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

fn divider<'a>() -> Element<'a, Message> {
    container(space().width(20).height(1))
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
            Tool::Pen | Tool::Polygon | Tool::Cloud => Path::new(|b| {
                b.move_to(first);
                for p in &view {
                    b.line_to(*p);
                }
                if self.draft.tool.by_corners() && view.len() > 2 {
                    b.close();
                }
            }),
            Tool::Rect | Tool::TextBox | Tool::Stamp => Path::rectangle(top_left, size),
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
    fn every_tool_has_its_own_key() {
        let mut keys: Vec<char> = Tool::ALL.iter().map(|t| t.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), Tool::ALL.len());
        assert_eq!(Tool::from_key("r"), Some(Tool::Rect));
        assert_eq!(Tool::from_key("rr"), None);
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
    fn a_stamp_is_chosen_then_placed_on_the_click() {
        let mut a = Annotate::new();
        a.update(Event::Tool(Tool::Stamp));
        a.press(1, (100.0, 50.0));
        a.release();
        assert!(a.stamping.is_some() && a.items.is_empty());
        assert!(matches!(a.update(Event::Stamp(0)), Outcome::Changed));
        let Some(Annotation {
            page: 1,
            kind: Kind::Stamp { rect, name },
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
            Some(Annotation { kind: Kind::Stamp { rect, name }, color, .. })
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
