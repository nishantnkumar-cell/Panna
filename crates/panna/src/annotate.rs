//! Annotation tools: the tool rail, drawing in progress, and the list of unsaved annotations
//! with undo and redo.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{Column, button, column, container, row, space, text, text_editor};
use iced::{Center, Element, Point, Rectangle, Renderer, Size, Theme, mouse};
use panna_core::{Annotation, Color, Kind, LINE_HEIGHT, NOTE_SIZE, TextBox, text_width, wrap};

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
}

impl Tool {
    pub const ALL: [Tool; 11] = [
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
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Tool::Select => "Select text (V)",
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
        }
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
/// Drags shorter than this (in points) count as clicks.
const CLICK_SLOP: f32 = 3.0;

#[derive(Debug, Clone)]
pub enum Event {
    Toggle,
    Tool(Tool),
    Color(usize),
    Undo,
    Redo,
    Edit(text_editor::Action),
    EditDone,
    EditCancel,
}

/// A shape being drawn, in page points.
#[derive(Debug, Clone)]
pub struct Draft {
    pub page: usize,
    tool: Tool,
    points: Vec<(f32, f32)>,
}

/// A note or text box waiting for its text.
pub struct Editing {
    page: usize,
    tool: Tool,
    at: TextBox,
    pub content: text_editor::Content,
}

#[derive(Default)]
pub struct Annotate {
    pub open: bool,
    pub tool: Tool,
    colors: [Option<Color>; Tool::ALL.len()],
    /// Unsaved annotations, oldest first.
    pub items: Vec<Annotation>,
    redo: Vec<Annotation>,
    pub draft: Option<Draft>,
    pub editing: Option<Editing>,
    author: String,
}

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

    /// Forgets unsaved annotations once they are saved (or discarded, or a new file opens).
    pub fn saved(&mut self) {
        self.items.clear();
        self.redo.clear();
        self.draft = None;
        self.editing = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.items.is_empty()
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
            }
            Event::Color(i) => {
                if let (Some(slot), Some(color)) = (self.colors.get_mut(self.tool.index()), SWATCHES.get(i)) {
                    *slot = Some(*color);
                }
            }
            Event::Undo => {
                if let Some(a) = self.items.pop() {
                    self.redo.push(a);
                    return Outcome::Changed;
                }
            }
            Event::Redo => {
                if let Some(a) = self.redo.pop() {
                    self.items.push(a);
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
                    if editing.tool == Tool::TextBox && text.trim().is_empty() {
                        return Outcome::Nothing;
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
                    self.push(editing.page, kind, editing.tool, text);
                    return Outcome::Changed;
                }
            }
            Event::EditCancel => self.editing = None,
        }
        Outcome::Nothing
    }

    /// Adds a new annotation made with `tool`.
    fn push(&mut self, page: usize, kind: Kind, tool: Tool, contents: String) {
        let color = self
            .colors
            .get(tool.index())
            .copied()
            .flatten()
            .unwrap_or_else(|| tool.default_color());
        self.items.push(Annotation {
            page,
            kind,
            color,
            opacity: 1.0,
            width: STROKE_WIDTH,
            contents,
            author: self.author.clone(),
        });
        self.redo.clear();
    }

    /// Marks selected text with the current markup tool: one annotation per page.
    pub fn mark_text(&mut self, per_page: Vec<(usize, Vec<TextBox>)>) -> Outcome {
        let tool = self.tool;
        let mut changed = false;
        for (page, boxes) in per_page {
            if boxes.is_empty() {
                continue;
            }
            let kind = match tool {
                Tool::Highlight => Kind::Highlight(boxes),
                Tool::Underline => Kind::Underline(boxes),
                Tool::StrikeOut => Kind::StrikeOut(boxes),
                _ => return Outcome::Nothing,
            };
            self.push(page, kind, tool, String::new());
            changed = true;
        }
        if changed {
            Outcome::Changed
        } else {
            Outcome::Nothing
        }
    }

    /// Starts drawing at a point on `page`, for tools that draw.
    pub fn press(&mut self, page: usize, at: (f32, f32)) {
        if self.tool.marks_text() || self.editing.is_some() {
            return;
        }
        self.draft = Some(Draft {
            page,
            tool: self.tool,
            points: vec![at],
        });
    }

    pub fn drag(&mut self, page: usize, at: (f32, f32)) {
        let Some(draft) = self.draft.as_mut().filter(|d| d.page == page) else {
            return;
        };
        if draft.tool == Tool::Pen {
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
        let Some(draft) = self.draft.take() else {
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
                });
                return Outcome::Nothing;
            }
            Tool::Pen => Kind::Ink(vec![draft.points]),
            _ if !dragged => return Outcome::Nothing,
            Tool::Rect => Kind::Rect(rect),
            Tool::Ellipse => Kind::Ellipse(rect),
            Tool::Arrow | Tool::Line => Kind::Line {
                from: start,
                to: end,
                arrow: draft.tool == Tool::Arrow,
            },
            Tool::Select | Tool::Highlight | Tool::Underline | Tool::StrikeOut => return Outcome::Nothing,
        };
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
        let current = self.color();
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
            column![tools, divider(), swatches, divider(), history]
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
        let editing = self.editing.as_ref()?;
        let ev = Message::Annotate;
        let title = if editing.tool == Tool::Note {
            "Note"
        } else {
            "Text box"
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
                    button("Add").on_press(ev(Event::EditDone)),
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
            Tool::Pen => Path::new(|b| {
                b.move_to(first);
                for p in &view {
                    b.line_to(*p);
                }
            }),
            Tool::Rect | Tool::TextBox => Path::rectangle(top_left, size),
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
}
