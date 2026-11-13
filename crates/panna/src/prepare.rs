//! Preparing a form: adding text fields, check boxes, drop-downs, list boxes and radio
//! buttons to pages, and deleting fields, each as a page change that can be undone from the page tools.

use iced::widget::{
    button, checkbox, column, container, pick_list, pin, row, space, text, text_editor, text_input,
};
use iced::{Center, Element, Theme};
use panna_core::{Field, FieldKind, NewField, NewFieldKind, ObjRef, RadioButton, TextBox};

use crate::Message;
use crate::icons::{self, Icon};
use crate::selection::PageMap;

/// A drag shorter than this, in points, is a click.
const CLICK_SLOP: f32 = 3.0;
const MAX_NAME: usize = 200;
const MAX_OPTIONS: usize = 500;
/// A radio button's side, and the distance between clicked buttons' tops, in points.
const RADIO_SIZE: f32 = 14.0;
const RADIO_STEP: f32 = 22.0;
/// The largest radio button a drag makes.
const RADIO_MAX: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Text,
    CheckBox,
    Dropdown,
    List,
    Radio,
    Date,
    Button,
    Image,
    Signature,
}

/// What a new push button can do, as the card lists it.
pub const BUTTON_ACTIONS: [&str; 8] = [
    "Do nothing",
    "Clear the form",
    "Go to a page",
    "Print",
    "Go to the next page",
    "Go to the previous page",
    "Go to the first page",
    "Go to the last page",
];

/// The action at `choice` in [`BUTTON_ACTIONS`], going to `page` (from 1) for "Go to a page".
fn button_action(choice: usize, page: &str) -> panna_core::ButtonAction {
    use panna_core::ButtonAction as A;
    match choice {
        1 => A::ResetForm,
        2 => page
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|p| p.checked_sub(1))
            .map_or(A::Nothing, A::GoTo),
        3 => A::Print,
        4 => A::NextPage,
        5 => A::PreviousPage,
        6 => A::FirstPage,
        7 => A::LastPage,
        _ => A::Nothing,
    }
}

/// The date patterns a date field can take, as in Acrobat's date formats.
pub const DATE_PATTERNS: [&str; 6] = [
    "dd/mm/yyyy",
    "mm/dd/yyyy",
    "yyyy-mm-dd",
    "d mmm yyyy",
    "mmm d, yyyy",
    "dd.mm.yyyy",
];

/// How picked fields are lined up with the one picked last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Top,
    Bottom,
    /// Centred across: the same middle from left to right.
    CentreAcross,
    /// Centred down: the same middle from top to bottom.
    CentreDown,
    SameWidth,
    SameHeight,
}

impl Align {
    /// `at` lined up with `anchor`.
    fn apply(self, at: TextBox, anchor: TextBox) -> TextBox {
        let (w, h) = (at.width(), at.height());
        match self {
            Align::Left => TextBox {
                x0: anchor.x0,
                x1: anchor.x0 + w,
                ..at
            },
            Align::Right => TextBox {
                x0: anchor.x1 - w,
                x1: anchor.x1,
                ..at
            },
            Align::Top => TextBox {
                y0: anchor.y0,
                y1: anchor.y0 + h,
                ..at
            },
            Align::Bottom => TextBox {
                y0: anchor.y1 - h,
                y1: anchor.y1,
                ..at
            },
            Align::CentreAcross => {
                let x0 = (anchor.x0 + anchor.x1 - w) / 2.0;
                TextBox { x0, x1: x0 + w, ..at }
            }
            Align::CentreDown => {
                let y0 = (anchor.y0 + anchor.y1 - h) / 2.0;
                TextBox { y0, y1: y0 + h, ..at }
            }
            Align::SameWidth => TextBox {
                x1: at.x0 + anchor.width(),
                ..at
            },
            Align::SameHeight => TextBox {
                y1: at.y0 + anchor.height(),
                ..at
            },
        }
    }
}

/// A form field's widget picked on the page.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Picked {
    page: usize,
    field: ObjRef,
    widget: ObjRef,
    at: TextBox,
}

/// A picked widget being dragged: moved, or with `resize`, stretched by its corner.
#[derive(Debug, Clone, Copy)]
struct Grab {
    picked: Picked,
    resize: bool,
}

/// How near the bottom-right corner, in points, a press grabs it for resizing.
const CORNER: f32 = 6.0;
/// The smallest a widget can be made, in points.
const MIN_SIDE: f32 = 6.0;

/// The topmost widget on `page` under `at`.
fn widget_at(fields: &[Field], page: usize, at: (f32, f32)) -> Option<Picked> {
    fields.iter().rev().find_map(|f| {
        f.widgets
            .iter()
            .find(|w| w.page == page && w.at.contains(at.0, at.1))
            .map(|w| Picked {
                page,
                field: f.id,
                widget: w.annot,
                at: w.at,
            })
    })
}

impl Tool {
    /// The size of a clicked (not dragged) field, in points.
    fn size(self) -> (f32, f32) {
        match self {
            Tool::Text => (150.0, 20.0),
            Tool::CheckBox => (14.0, 14.0),
            Tool::Dropdown => (120.0, 20.0),
            Tool::List => (120.0, 60.0),
            Tool::Radio => (RADIO_SIZE, RADIO_SIZE),
            Tool::Date => (100.0, 20.0),
            Tool::Button => (90.0, 24.0),
            Tool::Image => (100.0, 100.0),
            Tool::Signature => (180.0, 50.0),
        }
    }

    /// Whether the field is made from a list of choices.
    fn has_choices(self) -> bool {
        matches!(self, Tool::Dropdown | Tool::List | Tool::Radio)
    }

    fn base_name(self) -> &'static str {
        match self {
            Tool::Text => "Text",
            Tool::CheckBox => "Check box",
            Tool::Dropdown => "Choice",
            Tool::List => "List",
            Tool::Radio => "Group",
            Tool::Date => "Date",
            Tool::Button => "Button",
            Tool::Image => "Image",
            Tool::Signature => "Signature",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Toggle,
    Tool(Tool),
    Name(String),
    Tooltip(String),
    Required(bool),
    Multiline(bool),
    Options(text_editor::Action),
    /// A date field's pattern, by its place in [`DATE_PATTERNS`].
    Pattern(usize),
    /// A button's caption.
    Caption(String),
    /// What a button does, by its place in [`BUTTON_ACTIONS`].
    Action(usize),
    /// The page a "Go to a page" button goes to, as typed.
    Target(String),
    /// Lines the picked fields up with the one picked last.
    Align(Align),
    Done,
    Delete,
    Cancel,
    /// Find where fields could go on every page.
    Detect,
}

#[derive(Debug)]
enum Pending {
    New {
        page: usize,
        rect: TextBox,
        /// Dragged out, rather than clicked.
        dragged: bool,
        tool: Tool,
        name: String,
        tooltip: String,
        required: bool,
        multiline: bool,
        options: text_editor::Content,
        /// For a date field, its pattern's place in [`DATE_PATTERNS`].
        pattern: usize,
        /// For a button: its caption, its action's place in [`BUTTON_ACTIONS`], and the page
        /// it goes to.
        caption: String,
        action: usize,
        target: String,
    },
    Existing {
        id: ObjRef,
        name: String,
        kind: &'static str,
        /// Fields inside a group (with a dotted name) aren't deleted here.
        removable: bool,
    },
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    page: usize,
    from: (f32, f32),
    to: (f32, f32),
}

impl Drag {
    fn rect(&self) -> TextBox {
        TextBox {
            x0: self.from.0.min(self.to.0),
            y0: self.from.1.min(self.to.1),
            x1: self.from.0.max(self.to.0),
            y1: self.from.1.max(self.to.1),
        }
    }
}

#[derive(Debug, Default)]
pub struct Preparer {
    pub on: bool,
    pub tool: Tool,
    drag: Option<Drag>,
    /// A picked field's widget being moved or resized.
    grab: Option<Grab>,
    pending: Option<Pending>,
    /// The widgets picked, the last one the one others line up with.
    selected: Vec<Picked>,
}

/// What the main view does next.
#[derive(Debug, Clone)]
pub enum Outcome {
    Nothing,
    /// Add these fields and take out those.
    Edit {
        add: Vec<NewField>,
        remove: Vec<ObjRef>,
    },
    /// Find where fields could go and add them.
    Detect,
    /// Put these widgets (page, widget, new place) where they now go.
    Move(Vec<(usize, ObjRef, TextBox)>),
}

fn kind_label(kind: &FieldKind) -> &'static str {
    match kind {
        FieldKind::Text { .. } => "Text field",
        FieldKind::CheckBox => "Check box",
        FieldKind::Radio => "Radio buttons",
        FieldKind::Choice { combo: true, .. } => "Drop-down",
        FieldKind::Choice { .. } => "List",
        FieldKind::Button { .. } => "Button",
        FieldKind::Image => "Image field",
        FieldKind::Signature { .. } => "Signature field",
    }
}

fn options_of(content: &text_editor::Content) -> Vec<String> {
    content
        .text()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(MAX_OPTIONS)
        .map(str::to_owned)
        .collect()
}

/// Where the `count` buttons of a radio group go: down from a clicked spot, or spread along
/// the longer side of a dragged box.
fn radio_layout(rect: TextBox, count: usize, dragged: bool) -> Vec<TextBox> {
    let count = count.max(1);
    let square = |x: f32, y: f32, side: f32| TextBox {
        x0: x,
        y0: y,
        x1: x + side,
        y1: y + side,
    };
    if !dragged {
        return (0..count)
            .map(|i| square(rect.x0, rect.y0 + i as f32 * RADIO_STEP, RADIO_SIZE))
            .collect();
    }
    let across = rect.width() > rect.height();
    let (long, short) = if across {
        (rect.width(), rect.height())
    } else {
        (rect.height(), rect.width())
    };
    let side = short.clamp(8.0, RADIO_MAX);
    let step = if count > 1 {
        ((long - side) / (count - 1) as f32).max(side + 4.0)
    } else {
        0.0
    };
    let inset = (short - side) / 2.0;
    (0..count)
        .map(|i| {
            let along = i as f32 * step;
            if across {
                square(rect.x0 + along, rect.y0 + inset, side)
            } else {
                square(rect.x0 + inset, rect.y0 + along, side)
            }
        })
        .collect()
}

/// The box around `boxes`.
fn around(boxes: &[TextBox]) -> Option<TextBox> {
    let first = *boxes.first()?;
    Some(boxes.iter().fold(first, |a, b| TextBox {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }))
}

impl Preparer {
    /// Whether a drag or a card is under way, for Esc.
    pub fn busy(&self) -> bool {
        self.drag.is_some() || self.pending.is_some() || !self.selected.is_empty()
    }

    pub fn cancel(&mut self) {
        self.drag = None;
        self.grab = None;
        self.pending = None;
        self.selected.clear();
    }

    pub fn stop(&mut self) {
        self.on = false;
        self.cancel();
    }

    pub fn update(&mut self, event: Event) -> Outcome {
        let clean = |s: String| {
            s.chars()
                .filter(|c| !c.is_control())
                .take(MAX_NAME)
                .collect::<String>()
        };
        match event {
            Event::Toggle => {
                self.on = !self.on;
                self.cancel();
            }
            Event::Tool(tool) => {
                self.tool = tool;
                self.cancel();
            }
            Event::Cancel => self.cancel(),
            Event::Detect => {
                self.cancel();
                return Outcome::Detect;
            }
            Event::Name(value) => {
                if let Some(Pending::New { name, .. }) = &mut self.pending {
                    *name = clean(value);
                }
            }
            Event::Tooltip(value) => {
                if let Some(Pending::New { tooltip, .. }) = &mut self.pending {
                    *tooltip = clean(value);
                }
            }
            Event::Required(value) => {
                if let Some(Pending::New { required, .. }) = &mut self.pending {
                    *required = value;
                }
            }
            Event::Multiline(value) => {
                if let Some(Pending::New { multiline, .. }) = &mut self.pending {
                    *multiline = value;
                }
            }
            Event::Options(action) => {
                if let Some(Pending::New { options, .. }) = &mut self.pending {
                    options.perform(action);
                }
            }
            Event::Caption(v) => {
                if let Some(Pending::New { caption, .. }) = &mut self.pending {
                    *caption = v.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
                }
            }
            Event::Action(i) => {
                if let Some(Pending::New { action, .. }) = &mut self.pending {
                    *action = i.min(BUTTON_ACTIONS.len() - 1);
                }
            }
            Event::Target(v) => {
                if let Some(Pending::New { target, .. }) = &mut self.pending {
                    *target = v.chars().filter(char::is_ascii_digit).take(6).collect();
                }
            }
            Event::Pattern(i) => {
                if let Some(Pending::New { pattern, .. }) = &mut self.pending {
                    *pattern = i.min(DATE_PATTERNS.len() - 1);
                }
            }
            Event::Align(how) => {
                let Some(anchor) = self.selected.last().copied() else {
                    return Outcome::Nothing;
                };
                let moved: Vec<(usize, ObjRef, TextBox)> = self
                    .selected
                    .iter()
                    .filter(|p| p.page == anchor.page && p.widget != anchor.widget)
                    .filter_map(|p| {
                        let to = how.apply(p.at, anchor.at);
                        (to != p.at).then_some((p.page, p.widget, to))
                    })
                    .collect();
                if moved.is_empty() {
                    return Outcome::Nothing;
                }
                // The widgets get new numbers in the new file, so the picks go.
                self.cancel();
                return Outcome::Move(moved);
            }
            Event::Delete => {
                if let Some(Pending::Existing {
                    id, removable: true, ..
                }) = self.pending
                {
                    self.cancel();
                    return Outcome::Edit {
                        add: Vec::new(),
                        remove: vec![id],
                    };
                }
            }
            Event::Done => {
                let Some(Pending::New {
                    page,
                    rect,
                    dragged,
                    tool,
                    name,
                    tooltip,
                    required,
                    multiline,
                    options,
                    pattern,
                    caption,
                    action,
                    target,
                }) = self.pending.take()
                else {
                    self.pending = None;
                    return Outcome::Nothing;
                };
                let list = options_of(&options);
                if tool.has_choices() && list.is_empty() {
                    self.pending = Some(Pending::New {
                        page,
                        rect,
                        dragged,
                        tool,
                        name,
                        tooltip,
                        required,
                        multiline,
                        options,
                        pattern,
                        caption,
                        action,
                        target,
                    });
                    return Outcome::Nothing;
                }
                let mut rect = rect;
                let kind = match tool {
                    Tool::Text => NewFieldKind::Text { multiline },
                    Tool::CheckBox => NewFieldKind::CheckBox,
                    Tool::Dropdown => NewFieldKind::Dropdown(list),
                    Tool::List => NewFieldKind::List(list),
                    Tool::Date => NewFieldKind::Date(
                        DATE_PATTERNS
                            .get(pattern)
                            .copied()
                            .unwrap_or(DATE_PATTERNS[0])
                            .to_owned(),
                    ),
                    Tool::Button => NewFieldKind::Button {
                        caption: if caption.trim().is_empty() {
                            "Button".to_owned()
                        } else {
                            caption.trim().to_owned()
                        },
                        action: button_action(action, &target),
                    },
                    Tool::Image => NewFieldKind::Image,
                    Tool::Signature => NewFieldKind::Signature,
                    Tool::Radio => {
                        let boxes = radio_layout(rect, list.len(), dragged);
                        rect = around(&boxes).unwrap_or(rect);
                        NewFieldKind::Radio(
                            list.into_iter()
                                .zip(boxes)
                                .map(|(choice, rect)| RadioButton { choice, rect })
                                .collect(),
                        )
                    }
                };
                let name = if name.trim().is_empty() {
                    tool.base_name().to_owned()
                } else {
                    name.trim().to_owned()
                };
                return Outcome::Edit {
                    add: vec![NewField {
                        page,
                        rect,
                        name,
                        tooltip,
                        required,
                        kind,
                    }],
                    remove: Vec::new(),
                };
            }
        }
        Outcome::Nothing
    }

    /// A press on the page: on a field's widget it picks it, ready to be dragged (by its
    /// bottom-right corner, resized); with `adding` (Shift or Ctrl held) it adds the widget to
    /// the picked ones, or takes it out. Elsewhere it starts a new field.
    pub fn press(&mut self, page: usize, at: (f32, f32), fields: &[Field], adding: bool) {
        if matches!(self.pending, Some(Pending::New { .. })) {
            return;
        }
        if let Some(hit) = widget_at(fields, page, at) {
            if adding {
                if let Some(i) = self.selected.iter().position(|p| p.widget == hit.widget) {
                    self.selected.remove(i);
                } else {
                    self.selected.push(hit);
                }
                self.show_picked(fields);
                return;
            }
            let resize = (at.0 - hit.at.x1).abs() <= CORNER && (at.1 - hit.at.y1).abs() <= CORNER;
            self.grab = Some(Grab { picked: hit, resize });
        } else {
            self.grab = None;
        }
        self.drag = Some(Drag {
            page,
            from: at,
            to: at,
        });
    }

    pub fn hover(&mut self, page: usize, at: (f32, f32)) {
        if let Some(drag) = self.drag.as_mut().filter(|d| d.page == page) {
            drag.to = at;
        }
    }

    /// Where a grabbed widget goes for the drag so far.
    fn grabbed_to(grab: &Grab, drag: &Drag) -> TextBox {
        let (dx, dy) = (drag.to.0 - drag.from.0, drag.to.1 - drag.from.1);
        let at = grab.picked.at;
        if grab.resize {
            TextBox {
                x1: (at.x1 + dx).max(at.x0 + MIN_SIDE),
                y1: (at.y1 + dy).max(at.y0 + MIN_SIDE),
                ..at
            }
        } else {
            TextBox {
                x0: at.x0 + dx,
                y0: at.y0 + dy,
                x1: at.x1 + dx,
                y1: at.y1 + dy,
            }
        }
    }

    /// The card for what's picked: one field's details, or the ways to line several up.
    fn show_picked(&mut self, fields: &[Field]) {
        self.pending = match self.selected.as_slice() {
            [one] => fields
                .iter()
                .find(|f| f.id == one.field)
                .map(|field| Pending::Existing {
                    id: field.id,
                    name: field.name.clone(),
                    kind: kind_label(&field.kind),
                    removable: !field.name.contains('.'),
                }),
            _ => None,
        };
    }

    /// The end of a click or drag: a dragged widget moves; a click on a field picks it;
    /// otherwise asks about the new field there.
    pub fn release(&mut self, fields: &[Field]) -> Outcome {
        let Some(drag) = self.drag.take() else {
            return Outcome::Nothing;
        };
        if let Some(grab) = self.grab.take() {
            let moved = (drag.to.0 - drag.from.0).hypot(drag.to.1 - drag.from.1) >= CLICK_SLOP;
            if moved {
                let to = Self::grabbed_to(&grab, &drag);
                self.cancel();
                return Outcome::Move(vec![(grab.picked.page, grab.picked.widget, to)]);
            }
            self.selected = vec![grab.picked];
            self.show_picked(fields);
            return Outcome::Nothing;
        }
        let dragged = drag.rect();
        let is_drag = dragged.width() >= CLICK_SLOP && dragged.height() >= CLICK_SLOP;
        // A click beside the picked fields lets them go.
        if !is_drag && !self.selected.is_empty() {
            self.cancel();
            return Outcome::Nothing;
        }
        self.cancel();
        let tool = self.tool;
        let rect = if is_drag {
            if tool == Tool::CheckBox {
                // Check boxes are square.
                let side = dragged.width().min(dragged.height());
                TextBox {
                    x1: dragged.x0 + side,
                    y1: dragged.y0 + side,
                    ..dragged
                }
            } else {
                dragged
            }
        } else {
            let (w, h) = tool.size();
            TextBox {
                x0: drag.from.0,
                y0: drag.from.1 - h / 2.0,
                x1: drag.from.0 + w,
                y1: drag.from.1 + h / 2.0,
            }
        };
        let number = fields.len() + 1;
        self.pending = Some(Pending::New {
            page: drag.page,
            rect,
            dragged: is_drag,
            tool,
            name: format!("{} {number}", tool.base_name()),
            tooltip: String::new(),
            required: false,
            multiline: false,
            options: text_editor::Content::new(),
            pattern: 0,
            caption: String::new(),
            action: 0,
            target: String::new(),
        });
        Outcome::Nothing
    }

    /// Outlines of the fields on page `page`, with the new field's box.
    pub fn overlays<'a>(
        &'a self,
        page: usize,
        map: PageMap,
        fields: &'a [Field],
    ) -> Vec<Element<'a, Message>> {
        let boxed = |rect: TextBox, strong: bool, label: Option<&'a str>| -> Element<'a, Message> {
            let (x, y, w, h) = map.rect(&rect);
            let inside: Element<'a, Message> = match label {
                Some(name) if h >= 12.0 && w >= 40.0 => text(name).size(10).into(),
                _ => space().into(),
            };
            pin(container(inside)
                .padding([0, 2])
                .width(w.max(1.0))
                .height(h.max(1.0))
                .clip(true)
                .style(move |theme: &Theme| {
                    let primary = theme.extended_palette().primary.base.color;
                    container::Style::default()
                        .background(primary.scale_alpha(if strong { 0.2 } else { 0.1 }))
                        .color(primary)
                        .border(iced::Border {
                            color: primary,
                            width: if strong { 2.0 } else { 1.0 },
                            radius: 2.0.into(),
                        })
                }))
            .x(x)
            .y(y)
            .into()
        };
        let picked = |widget: ObjRef| self.selected.iter().any(|p| p.widget == widget);
        let mut out: Vec<Element<'a, Message>> = fields
            .iter()
            .flat_map(|f| {
                f.widgets
                    .iter()
                    .filter(move |w| w.page == page)
                    .map(move |w| boxed(w.at, picked(w.annot), Some(f.name.as_str())))
            })
            .collect();
        match (&self.drag, &self.pending) {
            (Some(d), _) if d.page == page => {
                let shown = match &self.grab {
                    Some(g) => Self::grabbed_to(g, d),
                    None => d.rect(),
                };
                out.push(boxed(shown, true, None));
            }
            (
                None,
                Some(Pending::New {
                    page: p,
                    rect,
                    dragged,
                    tool: Tool::Radio,
                    options,
                    ..
                }),
            ) if *p == page => {
                for button in radio_layout(*rect, options_of(options).len(), *dragged) {
                    out.push(boxed(button, true, None));
                }
            }
            (None, Some(Pending::New { page: p, rect, .. })) if *p == page => {
                out.push(boxed(*rect, true, None))
            }
            _ => {}
        }
        out
    }

    /// The floating bar with the field kinds.
    pub fn bar(&self) -> Element<'_, Message> {
        let ev = Message::Prepare;
        let tool = |tool: Tool, icon: Icon, tip: &'static str| {
            icons::toggle(icon, tip, self.tool == tool, ev(Event::Tool(tool)))
        };
        let content = row![
            tool(Tool::Text, Icon::TextField, "Add a text field"),
            tool(Tool::CheckBox, Icon::CheckBoxField, "Add a check box"),
            tool(Tool::Dropdown, Icon::DropdownField, "Add a drop-down list"),
            tool(Tool::List, Icon::ListField, "Add a list box"),
            tool(Tool::Radio, Icon::RadioField, "Add radio buttons"),
            tool(Tool::Date, Icon::Calendar, "Add a date field"),
            tool(Tool::Button, Icon::ButtonField, "Add a button"),
            tool(Tool::Image, Icon::ImageField, "Add an image field"),
            tool(Tool::Signature, Icon::Signature, "Add a digital signature field"),
            icons::button(
                Icon::DetectFields,
                "Find fields automatically, from blanks, lines and boxes",
                Some(ev(Event::Detect))
            ),
            self.picked_controls(),
            icons::button(
                Icon::Close,
                "Stop preparing the form (Esc)",
                Some(ev(Event::Toggle))
            ),
        ]
        .spacing(6)
        .align_y(Center);
        container(content)
            .padding([4, 8])
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

    /// The bar's middle: what to do, or the picked field's name and Delete, or with several
    /// picked, the ways to line them up with the one picked last.
    fn picked_controls(&self) -> Element<'_, Message> {
        let ev = Message::Prepare;
        if self.selected.len() > 1 {
            let align = |icon: Icon, tip: &'static str, how: Align| {
                icons::button(icon, tip, Some(ev(Event::Align(how))))
            };
            return row![
                text(format!("{} picked", self.selected.len())).size(13),
                align(
                    Icon::AlignLeft,
                    "Align left edges with the last picked",
                    Align::Left
                ),
                align(
                    Icon::CentreAcross,
                    "Centre across on the last picked",
                    Align::CentreAcross
                ),
                align(
                    Icon::AlignRight,
                    "Align right edges with the last picked",
                    Align::Right
                ),
                align(Icon::AlignTop, "Align top edges with the last picked", Align::Top),
                align(
                    Icon::CentreDown,
                    "Centre down on the last picked",
                    Align::CentreDown
                ),
                align(
                    Icon::AlignBottom,
                    "Align bottom edges with the last picked",
                    Align::Bottom
                ),
                align(
                    Icon::SameWidth,
                    "Make as wide as the last picked",
                    Align::SameWidth
                ),
                align(
                    Icon::SameHeight,
                    "Make as tall as the last picked",
                    Align::SameHeight
                ),
            ]
            .spacing(2)
            .align_y(Center)
            .into();
        }
        if let Some(Pending::Existing {
            name,
            kind,
            removable,
            ..
        }) = &self.pending
        {
            let tip = if *removable {
                "Delete the field and what's filled in"
            } else {
                "This field is part of a group, so it can't be deleted here yet"
            };
            return row![
                column![
                    text(name.as_str()).size(13),
                    text(*kind).size(11).style(text::secondary),
                ]
                .width(200),
                icons::button(Icon::Delete, tip, removable.then_some(ev(Event::Delete))),
                text("Drag to move, drag the corner to resize, Shift+click to pick more")
                    .size(12)
                    .width(220),
            ]
            .spacing(6)
            .align_y(Center)
            .into();
        }
        text("Click or drag to add a field. Click a field to pick it.")
            .size(13)
            .width(240)
            .into()
    }

    /// The card for a new field's details.
    pub fn card(&self) -> Option<Element<'_, Message>> {
        let ev = Message::Prepare;
        let body: Element<'_, Message> = match self.pending.as_ref()? {
            Pending::New {
                tool,
                name,
                tooltip,
                required,
                multiline,
                options,
                pattern,
                caption,
                action,
                target,
                ..
            } => {
                let title = match tool {
                    Tool::Text => "Add a text field",
                    Tool::CheckBox => "Add a check box",
                    Tool::Dropdown => "Add a drop-down list",
                    Tool::List => "Add a list box",
                    Tool::Radio => "Add radio buttons",
                    Tool::Date => "Add a date field",
                    Tool::Button => "Add a button",
                    Tool::Image => "Add an image field",
                    Tool::Signature => "Add a digital signature field",
                };
                let ready = !tool.has_choices() || !options_of(options).is_empty();
                let mut body = column![
                    text(title).size(16),
                    text("Name").size(12),
                    text_input("Name", name)
                        .id(name_id())
                        .on_input(move |v| ev(Event::Name(v)))
                        .on_submit_maybe(ready.then_some(ev(Event::Done))),
                    text("Tip shown on hover (optional)").size(12),
                    text_input("", tooltip).on_input(move |v| ev(Event::Tooltip(v))),
                ]
                .spacing(6);
                if *tool == Tool::Text {
                    body = body.push(
                        checkbox(*multiline)
                            .label("Several lines")
                            .on_toggle(move |v| ev(Event::Multiline(v))),
                    );
                }
                if *tool == Tool::Date {
                    let chosen = DATE_PATTERNS.get(*pattern).copied();
                    body = body.push(text("Date format").size(12)).push(
                        pick_list(DATE_PATTERNS, chosen, move |p: &'static str| {
                            ev(Event::Pattern(
                                DATE_PATTERNS.iter().position(|d| *d == p).unwrap_or(0),
                            ))
                        })
                        .width(iced::Fill),
                    );
                }
                if *tool == Tool::Button {
                    let chosen = BUTTON_ACTIONS.get(*action).copied();
                    body = body
                        .push(text("Caption").size(12))
                        .push(text_input("Button", caption).on_input(move |v| ev(Event::Caption(v))))
                        .push(text("When pressed").size(12))
                        .push(
                            pick_list(BUTTON_ACTIONS, chosen, move |a: &'static str| {
                                ev(Event::Action(
                                    BUTTON_ACTIONS.iter().position(|b| *b == a).unwrap_or(0),
                                ))
                            })
                            .width(iced::Fill),
                        );
                    if *action == 2 {
                        body = body
                            .push(text_input("Page number", target).on_input(move |v| ev(Event::Target(v))));
                    }
                }
                if *tool == Tool::Image {
                    body = body.push(
                        text("Whoever fills in the form clicks the field to choose its picture.").size(12),
                    );
                }
                if *tool == Tool::Signature {
                    body = body.push(
                        text("Whoever fills in the form clicks the field to sign in it with a certificate.")
                            .size(12),
                    );
                }
                if tool.has_choices() {
                    let label = if *tool == Tool::Radio {
                        "Choices, one per line, a button each"
                    } else {
                        "Choices, one per line"
                    };
                    body = body.push(text(label).size(12)).push(
                        text_editor(options)
                            .on_action(move |a| ev(Event::Options(a)))
                            .height(100),
                    );
                }
                body.push(
                    checkbox(*required)
                        .label("Must be filled in")
                        .on_toggle(move |v| ev(Event::Required(v))),
                )
                .push(
                    row![
                        space::horizontal(),
                        button("Cancel")
                            .style(button::secondary)
                            .on_press(ev(Event::Cancel)),
                        button("Add").on_press_maybe(ready.then_some(ev(Event::Done))),
                    ]
                    .spacing(8),
                )
                .into()
            }
            // A picked field's name and controls are on the bar, so more can be picked.
            Pending::Existing { .. } => return None,
        };
        Some(
            container(container(body).width(340))
                .padding(16)
                .style(container::rounded_box)
                .into(),
        )
    }

    /// Which input to focus once the card opens.
    pub fn focus(&self) -> Option<iced::widget::Id> {
        matches!(self.pending, Some(Pending::New { .. })).then(name_id)
    }
}

fn name_id() -> iced::widget::Id {
    iced::widget::Id::new("field-name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use panna_core::{Color, FieldValue, Widget};

    fn text_field(name: &str, at: TextBox) -> Field {
        Field {
            id: ObjRef {
                number: 7,
                generation: 0,
            },
            name: name.into(),
            tooltip: String::new(),
            kind: FieldKind::CheckBox,
            value: FieldValue::State(None),
            default: FieldValue::State(None),
            read_only: false,
            required: false,
            font_size: 0.0,
            color: Color::BLACK,
            align: 0,
            format: None,
            calculation: None,
            widgets: vec![Widget {
                page: 0,
                annot: ObjRef {
                    number: 7,
                    generation: 0,
                },
                at,
                on: Some("Yes".into()),
            }],
        }
    }

    fn on() -> Preparer {
        Preparer {
            on: true,
            ..Preparer::default()
        }
    }

    #[test]
    fn a_click_makes_a_usual_size_field_there() {
        let mut p = on();
        p.press(0, (40.0, 50.0), &[], false);
        p.release(&[]);
        let Outcome::Edit { add, remove } = p.update(Event::Done) else {
            panic!("nothing added");
        };
        assert!(remove.is_empty());
        let f = &add[0];
        assert_eq!(
            (f.name.as_str(), f.rect.x0, f.rect.y0, f.rect.x1),
            ("Text 1", 40.0, 40.0, 190.0)
        );
        assert_eq!(f.kind, NewFieldKind::Text { multiline: false });
    }

    #[test]
    fn a_drop_down_needs_choices() {
        let mut p = on();
        p.update(Event::Tool(Tool::Dropdown));
        p.press(1, (10.0, 10.0), &[], false);
        p.hover(1, (90.0, 30.0));
        p.release(&[]);
        assert!(matches!(p.update(Event::Done), Outcome::Nothing));
        for c in "A\n\nB ".chars() {
            p.update(Event::Options(text_editor::Action::Edit(
                text_editor::Edit::Insert(c),
            )));
        }
        let Outcome::Edit { add, .. } = p.update(Event::Done) else {
            panic!("nothing added");
        };
        assert_eq!(add[0].kind, NewFieldKind::Dropdown(vec!["A".into(), "B".into()]));
        assert_eq!((add[0].page, add[0].rect.x1), (1, 90.0));
    }

    #[test]
    fn clicking_a_field_offers_to_delete_it_unless_grouped() {
        let at = TextBox {
            x0: 0.0,
            y0: 0.0,
            x1: 20.0,
            y1: 20.0,
        };
        let mut p = on();
        let fields = [text_field("Agree", at)];
        p.press(0, (5.0, 5.0), &fields, false);
        p.release(&fields);
        let Outcome::Edit { add, remove } = p.update(Event::Delete) else {
            panic!("not deleted");
        };
        assert!(add.is_empty() && remove.len() == 1);
        let fields = [text_field("group.agree", at)];
        p.press(0, (5.0, 5.0), &fields, false);
        p.release(&fields);
        assert!(matches!(p.update(Event::Delete), Outcome::Nothing));
    }

    fn type_choices(p: &mut Preparer, choices: &str) {
        for c in choices.chars() {
            p.update(Event::Options(text_editor::Action::Edit(
                text_editor::Edit::Insert(c),
            )));
        }
    }

    #[test]
    fn a_list_box_is_a_list_of_its_choices() {
        let mut p = on();
        p.update(Event::Tool(Tool::List));
        p.press(0, (10.0, 40.0), &[], false);
        p.release(&[]);
        type_choices(&mut p, "Small\nLarge");
        let Outcome::Edit { add, .. } = p.update(Event::Done) else {
            panic!("nothing added");
        };
        assert_eq!(add[0].name, "List 1");
        assert_eq!(
            add[0].kind,
            NewFieldKind::List(vec!["Small".into(), "Large".into()])
        );
        assert_eq!((add[0].rect.y0, add[0].rect.y1), (10.0, 70.0));
    }

    #[test]
    fn clicked_radio_buttons_go_down_from_the_click() {
        let mut p = on();
        p.update(Event::Tool(Tool::Radio));
        p.press(0, (10.0, 50.0), &[], false);
        p.release(&[]);
        assert!(matches!(p.update(Event::Done), Outcome::Nothing), "needs choices");
        type_choices(&mut p, "Yes\nNo\nMaybe");
        let Outcome::Edit { add, .. } = p.update(Event::Done) else {
            panic!("nothing added");
        };
        let NewFieldKind::Radio(buttons) = &add[0].kind else {
            panic!("not radio buttons");
        };
        let found: Vec<(&str, f32, f32)> = buttons
            .iter()
            .map(|b| (b.choice.as_str(), b.rect.x0, b.rect.y0))
            .collect();
        assert_eq!(
            found,
            [("Yes", 10.0, 43.0), ("No", 10.0, 65.0), ("Maybe", 10.0, 87.0)]
        );
        assert!(buttons.iter().all(|b| b.rect.width() == RADIO_SIZE));
        assert_eq!((add[0].rect.y0, add[0].rect.y1), (43.0, 101.0));
    }

    fn widget(number: u32, name: &str, at: TextBox) -> Field {
        let mut f = text_field(name, at);
        f.id = ObjRef {
            number,
            generation: 0,
        };
        if let Some(w) = f.widgets.first_mut() {
            w.annot = f.id;
        }
        f
    }

    fn tb(x0: f32, y0: f32, x1: f32, y1: f32) -> TextBox {
        TextBox { x0, y0, x1, y1 }
    }

    #[test]
    fn dragging_a_field_moves_it_and_its_corner_resizes_it() {
        let fields = [widget(7, "Name", tb(10.0, 10.0, 110.0, 30.0))];
        let mut p = on();
        p.press(0, (50.0, 20.0), &fields, false);
        p.hover(0, (70.0, 60.0));
        let Outcome::Move(moved) = p.release(&fields) else {
            panic!("not moved");
        };
        assert_eq!(moved, [(0, fields[0].id, tb(30.0, 50.0, 130.0, 70.0))]);
        // By its bottom-right corner, it is stretched instead, never below the smallest size.
        p.press(0, (108.0, 28.0), &fields, false);
        p.hover(0, (158.0, 18.0));
        let Outcome::Move(moved) = p.release(&fields) else {
            panic!("not resized");
        };
        assert_eq!(moved, [(0, fields[0].id, tb(10.0, 10.0, 160.0, 20.0))]);
        p.press(0, (108.0, 28.0), &fields, false);
        p.hover(0, (0.0, 0.0));
        let Outcome::Move(moved) = p.release(&fields) else {
            panic!("not resized");
        };
        assert_eq!(moved[0].2, tb(10.0, 10.0, 10.0 + MIN_SIDE, 10.0 + MIN_SIDE));
    }

    #[test]
    fn picked_fields_line_up_with_the_last_one() {
        let fields = [
            widget(7, "A", tb(10.0, 10.0, 60.0, 30.0)),
            widget(8, "B", tb(40.0, 50.0, 140.0, 80.0)),
            widget(9, "C", tb(20.0, 100.0, 50.0, 110.0)),
        ];
        let mut p = on();
        p.press(0, (20.0, 20.0), &fields, false);
        p.release(&fields);
        p.press(0, (30.0, 105.0), &fields, true);
        p.press(0, (50.0, 60.0), &fields, true);
        assert_eq!(p.selected.len(), 3);
        let Outcome::Move(moved) = p.update(Event::Align(Align::Left)) else {
            panic!("not lined up");
        };
        assert_eq!(
            moved,
            [
                (0, fields[0].id, tb(40.0, 10.0, 90.0, 30.0)),
                (0, fields[2].id, tb(40.0, 100.0, 70.0, 110.0)),
            ]
        );
        assert!(!p.busy(), "the picks go once lined up");
        // Shift+click on a picked field lets it go again.
        p.press(0, (20.0, 20.0), &fields, true);
        p.press(0, (20.0, 20.0), &fields, true);
        assert!(p.selected.is_empty());
        assert_eq!(
            Align::SameWidth.apply(tb(0.0, 0.0, 10.0, 10.0), tb(5.0, 5.0, 45.0, 9.0)),
            tb(0.0, 0.0, 40.0, 10.0)
        );
        assert_eq!(
            Align::CentreDown.apply(tb(0.0, 0.0, 10.0, 10.0), tb(5.0, 20.0, 45.0, 40.0)),
            tb(0.0, 25.0, 10.0, 35.0)
        );
    }

    #[test]
    fn a_button_takes_its_caption_and_action() {
        let mut p = on();
        p.update(Event::Tool(Tool::Button));
        p.press(0, (10.0, 40.0), &[], false);
        p.release(&[]);
        p.update(Event::Caption("Next".into()));
        p.update(Event::Action(2));
        p.update(Event::Target("3x".into()));
        let Outcome::Edit { add, .. } = p.update(Event::Done) else {
            panic!("nothing added");
        };
        assert_eq!(
            add[0].kind,
            NewFieldKind::Button {
                caption: "Next".into(),
                action: panna_core::ButtonAction::GoTo(2),
            }
        );
        for (tool, kind) in [
            (Tool::Image, NewFieldKind::Image),
            (Tool::Signature, NewFieldKind::Signature),
        ] {
            p.update(Event::Tool(tool));
            p.press(0, (10.0, 140.0), &[], false);
            p.release(&[]);
            let Outcome::Edit { add, .. } = p.update(Event::Done) else {
                panic!("nothing added");
            };
            assert_eq!(add[0].kind, kind);
        }
    }

    #[test]
    fn a_date_field_takes_the_chosen_pattern() {
        let mut p = on();
        p.update(Event::Tool(Tool::Date));
        p.press(0, (10.0, 40.0), &[], false);
        p.release(&[]);
        p.update(Event::Pattern(2));
        let Outcome::Edit { add, .. } = p.update(Event::Done) else {
            panic!("nothing added");
        };
        assert_eq!(add[0].kind, NewFieldKind::Date("yyyy-mm-dd".into()));
        assert_eq!(add[0].name, "Date 1");
    }

    #[test]
    fn dragged_radio_buttons_spread_across_the_box() {
        let boxes = radio_layout(
            TextBox {
                x0: 0.0,
                y0: 0.0,
                x1: 116.0,
                y1: 16.0,
            },
            3,
            true,
        );
        let found: Vec<(f32, f32, f32)> = boxes.iter().map(|b| (b.x0, b.y0, b.x1)).collect();
        assert_eq!(found, [(0.0, 0.0, 16.0), (50.0, 0.0, 66.0), (100.0, 0.0, 116.0)]);
        // Tall boxes take them downwards, no bigger than the largest size.
        let boxes = radio_layout(
            TextBox {
                x0: 0.0,
                y0: 0.0,
                x1: 40.0,
                y1: 100.0,
            },
            2,
            true,
        );
        assert_eq!((boxes[0].x0, boxes[1].y0, boxes[1].y1), (8.0, 76.0, 100.0));
    }
}
