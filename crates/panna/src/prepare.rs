//! Preparing a form: adding text fields, check boxes, drop-downs, list boxes and radio
//! buttons to pages, and deleting fields, each as a page change that can be undone from the page tools.

use iced::widget::{button, checkbox, column, container, pin, row, space, text, text_editor, text_input};
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
    pending: Option<Pending>,
}

/// What the main view does next.
pub enum Outcome {
    Nothing,
    /// Add these fields and take out those.
    Edit {
        add: Vec<NewField>,
        remove: Vec<ObjRef>,
    },
    /// Find where fields could go and add them.
    Detect,
}

fn kind_label(kind: &FieldKind) -> &'static str {
    match kind {
        FieldKind::Text { .. } => "Text field",
        FieldKind::CheckBox => "Check box",
        FieldKind::Radio => "Radio buttons",
        FieldKind::Choice { combo: true, .. } => "Drop-down",
        FieldKind::Choice { .. } => "List",
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
        self.drag.is_some() || self.pending.is_some()
    }

    pub fn cancel(&mut self) {
        self.drag = None;
        self.pending = None;
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
            Event::Delete => {
                if let Some(Pending::Existing {
                    id, removable: true, ..
                }) = self.pending
                {
                    self.pending = None;
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
                    });
                    return Outcome::Nothing;
                }
                let mut rect = rect;
                let kind = match tool {
                    Tool::Text => NewFieldKind::Text { multiline },
                    Tool::CheckBox => NewFieldKind::CheckBox,
                    Tool::Dropdown => NewFieldKind::Dropdown(list),
                    Tool::List => NewFieldKind::List(list),
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

    pub fn press(&mut self, page: usize, at: (f32, f32)) {
        if self.pending.is_none() {
            self.drag = Some(Drag {
                page,
                from: at,
                to: at,
            });
        }
    }

    pub fn hover(&mut self, page: usize, at: (f32, f32)) {
        if let Some(drag) = self.drag.as_mut().filter(|d| d.page == page) {
            drag.to = at;
        }
    }

    /// The end of a click or drag: a click on a field picks it; otherwise asks about the new
    /// field there.
    pub fn release(&mut self, fields: &[Field]) {
        let Some(drag) = self.drag.take() else { return };
        let dragged = drag.rect();
        let is_drag = dragged.width() >= CLICK_SLOP && dragged.height() >= CLICK_SLOP;
        if !is_drag {
            let picked = fields.iter().rev().find(|f| {
                f.widgets
                    .iter()
                    .any(|w| w.page == drag.page && w.at.contains(drag.from.0, drag.from.1))
            });
            if let Some(field) = picked {
                self.pending = Some(Pending::Existing {
                    id: field.id,
                    name: field.name.clone(),
                    kind: kind_label(&field.kind),
                    removable: !field.name.contains('.'),
                });
                return;
            }
        }
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
        });
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
        let picked = match &self.pending {
            Some(Pending::Existing { id, .. }) => Some(*id),
            _ => None,
        };
        let mut out: Vec<Element<'a, Message>> = fields
            .iter()
            .flat_map(|f| {
                f.widgets
                    .iter()
                    .filter(move |w| w.page == page)
                    .map(move |w| boxed(w.at, picked == Some(f.id), Some(f.name.as_str())))
            })
            .collect();
        match (&self.drag, &self.pending) {
            (Some(d), _) if d.page == page => out.push(boxed(d.rect(), true, None)),
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
            icons::button(
                Icon::DetectFields,
                "Find fields automatically, from blanks, lines and boxes",
                Some(ev(Event::Detect))
            ),
            text("Click or drag to add a field. Click a field to delete it.")
                .size(13)
                .width(370),
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

    /// The card for a new field's details, or a picked field.
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
                ..
            } => {
                let title = match tool {
                    Tool::Text => "Add a text field",
                    Tool::CheckBox => "Add a check box",
                    Tool::Dropdown => "Add a drop-down list",
                    Tool::List => "Add a list box",
                    Tool::Radio => "Add radio buttons",
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
            Pending::Existing {
                name,
                kind,
                removable,
                ..
            } => {
                let note = if *removable {
                    "Deleting the field takes it and what's filled in out of the form."
                } else {
                    "This field is part of a group, so it can't be deleted here yet."
                };
                column![
                    text(name.as_str()).size(16),
                    text(*kind).size(12).style(text::secondary),
                    text(note).size(13),
                    row![
                        space::horizontal(),
                        button("Close")
                            .style(button::secondary)
                            .on_press(ev(Event::Cancel)),
                        button("Delete field")
                            .style(button::danger)
                            .on_press_maybe(removable.then_some(ev(Event::Delete))),
                    ]
                    .spacing(8),
                ]
                .spacing(8)
                .into()
            }
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
        p.press(0, (40.0, 50.0));
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
        p.press(1, (10.0, 10.0));
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
        p.press(0, (5.0, 5.0));
        p.release(&[text_field("Agree", at)]);
        let Outcome::Edit { add, remove } = p.update(Event::Delete) else {
            panic!("not deleted");
        };
        assert!(add.is_empty() && remove.len() == 1);
        p.press(0, (5.0, 5.0));
        p.release(&[text_field("group.agree", at)]);
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
        p.press(0, (10.0, 40.0));
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
        p.press(0, (10.0, 50.0));
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
