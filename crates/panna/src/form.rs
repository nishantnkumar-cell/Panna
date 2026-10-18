//! Filling in forms: the document's fields drawn over the page as inputs, and what has been
//! filled in since the last save.

use std::collections::HashMap;

use iced::widget::{button, container, pick_list, pin, svg, text_editor, text_input};
use iced::{Border, Center, Color, Element, Fill, Length, Theme};
use panna_core::{Field, FieldKind, FieldValue, ObjRef};

use crate::Message;
use crate::icons::Icon;
use crate::selection::PageMap;

/// The light blue Acrobat uses to show where fields are.
const FIELD_FILL: Color = Color::from_rgb(0.86, 0.9, 1.0);
const FIELD_EDGE: Color = Color::from_rgb(0.55, 0.65, 0.9);
/// The outline of a value that doesn't fit the field's format.
const INVALID: Color = Color::from_rgb(0.85, 0.15, 0.15);

#[derive(Debug, Clone)]
pub enum Event {
    Text(usize, String),
    /// A multiline field's editor changed.
    Edit(usize, text_editor::Action),
    /// A checkbox or radio button was clicked: the state it turns on.
    Press(usize, Option<String>),
    Choose(usize, String),
}

#[derive(Default)]
pub struct Form {
    fields: Vec<Field>,
    /// Values now, one per field.
    values: Vec<FieldValue>,
    /// Editors for multiline fields, by field index.
    editors: HashMap<usize, text_editor::Content>,
    /// The field being typed in, shown as typed; the others show their formatted values.
    active: Option<usize>,
}

/// A choice in a drop-down: what it shows, and the value it saves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    label: String,
    export: String,
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

impl Form {
    /// Takes a newly opened (or just saved) document's fields.
    pub fn load(&mut self, fields: &[Field]) {
        self.fields = fields.to_vec();
        self.values = fields.iter().map(|f| f.value.clone()).collect();
        self.editors = fields
            .iter()
            .enumerate()
            .filter(|(_, f)| matches!(f.kind, FieldKind::Text { multiline: true, .. }))
            .map(|(i, f)| {
                let text = match &f.value {
                    FieldValue::Text(t) => t.as_str(),
                    FieldValue::State(_) => "",
                };
                (i, text_editor::Content::with_text(text))
            })
            .collect();
    }

    /// The current values become the saved ones.
    pub fn saved(&mut self) {
        for (field, value) in self.fields.iter_mut().zip(&self.values) {
            field.value = value.clone();
        }
    }

    /// Puts every field back as it was when last saved.
    pub fn discard(&mut self) {
        let fields = std::mem::take(&mut self.fields);
        self.load(&fields);
    }

    /// Typing moved on (Tab, or a click elsewhere), so the last field shows its formatted value.
    pub fn leave(&mut self) {
        self.active = None;
    }

    /// Whether resetting would change anything.
    pub fn can_reset(&self) -> bool {
        self.fields
            .iter()
            .zip(&self.values)
            .any(|(f, v)| f.editable() && f.default != *v)
    }

    /// Puts every field back to its default, as Acrobat's Reset Form does.
    pub fn reset(&mut self) {
        for (i, (field, value)) in self.fields.iter().zip(&mut self.values).enumerate() {
            if !field.editable() {
                continue;
            }
            *value = field.default.clone();
            if let (Some(editor), FieldValue::Text(t)) = (self.editors.get_mut(&i), &field.default) {
                *editor = text_editor::Content::with_text(t);
            }
        }
    }

    /// Fields whose values differ from the file's.
    pub fn changes(&self) -> Vec<(ObjRef, FieldValue)> {
        self.fields
            .iter()
            .zip(&self.values)
            .filter(|(f, v)| f.value != **v)
            .map(|(f, v)| (f.id, v.clone()))
            .collect()
    }

    pub fn dirty(&self) -> bool {
        self.fields.iter().zip(&self.values).any(|(f, v)| f.value != *v)
    }

    /// Whether the document has a form.
    pub fn exists(&self) -> bool {
        !self.fields.is_empty()
    }

    /// Every named field's full name and current value, for a form data file.
    pub fn data(&self) -> Vec<(&str, &FieldValue)> {
        self.fields
            .iter()
            .zip(&self.values)
            .filter(|(f, _)| !f.name.is_empty())
            .map(|(f, v)| (f.name.as_str(), v))
            .collect()
    }

    /// Fills in fields from a form data file's names and values. Fields that can't be changed,
    /// and values a field can't take, are left out. Returns how many fields were filled in.
    pub fn import(&mut self, data: &[(String, String)]) -> usize {
        let mut filled = 0;
        for (i, field) in self.fields.iter().enumerate() {
            if !field.editable() {
                continue;
            }
            let Some((_, text)) = data.iter().rev().find(|(name, _)| *name == field.name) else {
                continue;
            };
            let value = match &field.kind {
                FieldKind::CheckBox | FieldKind::Radio if text == "Off" => FieldValue::State(None),
                FieldKind::CheckBox | FieldKind::Radio => {
                    if !field
                        .widgets
                        .iter()
                        .any(|w| w.on.as_deref() == Some(text.as_str()))
                    {
                        continue;
                    }
                    FieldValue::State(Some(text.clone()))
                }
                FieldKind::Choice {
                    editable: false,
                    options,
                    ..
                } if !text.is_empty() && !options.iter().any(|o| o.export == *text) => continue,
                FieldKind::Text {
                    max_len: Some(max), ..
                } => FieldValue::Text(text.chars().take(*max).collect()),
                _ => FieldValue::Text(text.clone()),
            };
            if let (Some(editor), FieldValue::Text(t)) = (self.editors.get_mut(&i), &value) {
                *editor = text_editor::Content::with_text(t);
            }
            if let Some(slot) = self.values.get_mut(i) {
                *slot = value;
                filled += 1;
            }
        }
        filled
    }

    /// Applies an edit. Returns whether a value changed.
    pub fn update(&mut self, event: Event) -> bool {
        let index = match &event {
            Event::Text(i, _) | Event::Edit(i, _) | Event::Press(i, _) | Event::Choose(i, _) => *i,
        };
        let Some(field) = self.fields.get(index).filter(|f| f.editable()) else {
            return false;
        };
        let new = match event {
            Event::Text(_, mut text) => {
                self.active = Some(index);
                if let Some(format) = &field.format {
                    text.retain(|c| format.allows(c));
                }
                if let FieldKind::Text {
                    max_len: Some(max), ..
                } = field.kind
                    && let Some((cut, _)) = text.char_indices().nth(max)
                {
                    text.truncate(cut);
                }
                FieldValue::Text(text)
            }
            Event::Edit(_, action) => {
                let Some(content) = self.editors.get_mut(&index) else {
                    return false;
                };
                let edits = action.is_edit();
                content.perform(action);
                if !edits {
                    return false;
                }
                FieldValue::Text(content.text().trim_end_matches('\n').to_owned())
            }
            Event::Press(_, on) => {
                let current = self.values.get(index);
                // A checkbox toggles; a radio button only turns on.
                match (&field.kind, current) {
                    (FieldKind::CheckBox, Some(FieldValue::State(Some(_)))) => FieldValue::State(None),
                    _ => FieldValue::State(on),
                }
            }
            Event::Choose(_, export) => FieldValue::Text(export),
        };
        match self.values.get_mut(index) {
            Some(value) if *value != new => {
                *value = new;
                true
            }
            _ => false,
        }
    }

    /// The inputs for `page`'s fields, positioned over the page at its current zoom.
    pub fn overlays<'a>(&'a self, page: usize, map: PageMap) -> Vec<Element<'a, Message>> {
        let mut out = Vec::new();
        for (index, field) in self.fields.iter().enumerate() {
            let Some(value) = self.values.get(index) else {
                continue;
            };
            for widget in field.widgets.iter().filter(|w| w.page == page) {
                let (x, y, w, h) = map.rect(&widget.at);
                if w < 2.0 || h < 2.0 {
                    continue;
                }
                let input = self.input(index, field, value, widget.on.as_deref(), map.zoom, h);
                out.push(
                    pin(container(input).width(w).height(h).clip(true))
                        .x(x)
                        .y(y)
                        .into(),
                );
            }
        }
        out
    }

    fn input<'a>(
        &'a self,
        index: usize,
        field: &'a Field,
        value: &'a FieldValue,
        on: Option<&'a str>,
        zoom: f32,
        height: f32,
    ) -> Element<'a, Message> {
        let ev = Message::Form;
        let editable = field.editable();
        let auto = (height * 0.62).clamp(6.0, 14.0 * zoom);
        let size = if field.font_size > 0.0 {
            field.font_size * zoom
        } else {
            auto
        };
        let ink = Color::from_rgb(field.color.0[0], field.color.0[1], field.color.0[2]);
        let text = match value {
            FieldValue::Text(t) => t.as_str(),
            FieldValue::State(_) => "",
        };
        let valid = field.format.as_ref().is_none_or(|f| f.accepts(text));
        // Formatted once typing has moved on: "1234.5" becomes "$1,234.50".
        let shown = match &field.format {
            Some(f) if self.active != Some(index) => f.display(text).unwrap_or_else(|| text.to_owned()),
            _ => text.to_owned(),
        };
        match &field.kind {
            FieldKind::Text { multiline: true, .. } => {
                let Some(content) = self.editors.get(&index) else {
                    return box_only();
                };
                let mut editor = text_editor(content).size(size).height(Fill).padding(2).style(
                    move |theme: &Theme, status| {
                        let mut s = text_editor::default(theme, status);
                        s.background = FIELD_FILL.into();
                        s.border = Border {
                            width: if matches!(status, text_editor::Status::Focused { .. }) {
                                1.5
                            } else {
                                1.0
                            },
                            ..edge(true)
                        };
                        s.value = ink;
                        s
                    },
                );
                if editable {
                    editor = editor.on_action(move |a| ev(Event::Edit(index, a)));
                }
                editor.into()
            }
            FieldKind::Text { password, .. } => {
                let mut input = text_input("", &shown)
                    .size(size)
                    .padding([0.0, 2.0])
                    .width(Fill)
                    .secure(*password)
                    .align_x(match field.align {
                        1 => iced::alignment::Horizontal::Center,
                        2 => iced::alignment::Horizontal::Right,
                        _ => iced::alignment::Horizontal::Left,
                    })
                    .style(move |theme: &Theme, status| {
                        let mut s = text_input::default(theme, status);
                        s.background = FIELD_FILL.into();
                        s.border = edge(matches!(status, text_input::Status::Focused { .. }));
                        s.value = ink;
                        s
                    });
                if editable {
                    input = input.on_input(move |t| ev(Event::Text(index, t)));
                }
                let boxed = container(input).height(Fill).align_y(Center);
                if valid {
                    boxed.style(fill).into()
                } else {
                    boxed.style(invalid).into()
                }
            }
            FieldKind::CheckBox | FieldKind::Radio => {
                let set = matches!(value, FieldValue::State(Some(s)) if Some(s.as_str()) == on);
                let radio = field.kind == FieldKind::Radio;
                let mark: Element<'a, Message> = match (set, radio) {
                    (false, _) => iced::widget::space().into(),
                    (true, false) => svg(svg::Handle::from_memory(Icon::Check.bytes()))
                        .width(Fill)
                        .height(Fill)
                        .style(move |_: &Theme, _| svg::Style { color: Some(ink) })
                        .into(),
                    (true, true) => {
                        let dot = (height * 0.5).max(3.0);
                        container(iced::widget::space())
                            .width(dot)
                            .height(dot)
                            .style(move |_| {
                                container::Style::default().background(ink).border(Border {
                                    radius: (dot / 2.0).into(),
                                    ..Border::default()
                                })
                            })
                            .into()
                    }
                };
                let press = editable.then(|| ev(Event::Press(index, on.map(str::to_owned))));
                button(container(mark).center(Fill))
                    .padding(1)
                    .width(Fill)
                    .height(Fill)
                    .style(move |_: &Theme, status| button::Style {
                        background: Some(FIELD_FILL.into()),
                        border: Border {
                            radius: if radio { 999.0.into() } else { 0.0.into() },
                            color: FIELD_EDGE,
                            width: if matches!(status, button::Status::Hovered) {
                                1.5
                            } else {
                                1.0
                            },
                        },
                        ..button::Style::default()
                    })
                    .on_press_maybe(press)
                    .into()
            }
            FieldKind::Choice { options, .. } => {
                let choices: Vec<Choice> = options
                    .iter()
                    .map(|o| Choice {
                        label: o.label.clone(),
                        export: o.export.clone(),
                    })
                    .collect();
                let selected = choices.iter().find(|c| c.export == text).cloned();
                let on_select = move |c: Choice| ev(Event::Choose(index, c.export));
                let list = pick_list(choices, selected, on_select)
                    .text_size(size)
                    .padding([0.0, 2.0])
                    .width(Fill)
                    .style(move |theme: &Theme, status| {
                        let mut s = pick_list::default(theme, status);
                        s.background = FIELD_FILL.into();
                        s.border = edge(matches!(status, pick_list::Status::Opened { .. }));
                        s.text_color = ink;
                        s
                    });
                if editable {
                    container(list).height(Fill).align_y(Center).style(fill).into()
                } else {
                    // A read-only list can't be opened.
                    container(iced::widget::text(text.to_owned()).size(size).color(ink))
                        .height(Fill)
                        .align_y(Center)
                        .style(fill)
                        .into()
                }
            }
        }
    }
}

/// An input's own border: only drawn while it has focus, since the box around it has one.
fn edge(active: bool) -> Border {
    Border {
        color: FIELD_EDGE,
        width: if active { 1.5 } else { 0.0 },
        radius: 0.0.into(),
    }
}

/// The field's box, which also covers the file's own drawing of the field.
fn fill(_: &Theme) -> container::Style {
    container::Style::default().background(FIELD_FILL).border(Border {
        color: FIELD_EDGE,
        width: 1.0,
        radius: 0.0.into(),
    })
}

fn invalid(theme: &Theme) -> container::Style {
    fill(theme).border(Border {
        color: INVALID,
        width: 2.0,
        radius: 0.0.into(),
    })
}

fn box_only<'a>() -> Element<'a, Message> {
    container(iced::widget::space())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use panna_core::{Color as Ink, TextBox, Widget};

    fn field(number: u32, kind: FieldKind, value: FieldValue, on: &[&str]) -> Field {
        Field {
            id: ObjRef {
                number,
                generation: 0,
            },
            name: format!("f{number}"),
            tooltip: String::new(),
            kind,
            default: match &value {
                FieldValue::Text(_) => FieldValue::Text(String::new()),
                FieldValue::State(_) => FieldValue::State(None),
            },
            value,
            read_only: false,
            required: false,
            font_size: 0.0,
            color: Ink::BLACK,
            align: 0,
            format: None,
            widgets: on
                .iter()
                .map(|s| Widget {
                    page: 0,
                    annot: ObjRef {
                        number,
                        generation: 0,
                    },
                    at: TextBox {
                        x0: 0.0,
                        y0: 0.0,
                        x1: 10.0,
                        y1: 10.0,
                    },
                    on: (!s.is_empty()).then(|| (*s).to_owned()),
                })
                .collect(),
        }
    }

    fn form() -> Form {
        let mut f = Form::default();
        f.load(&[
            field(
                1,
                FieldKind::Text {
                    multiline: false,
                    password: false,
                    max_len: Some(3),
                },
                FieldValue::Text(String::new()),
                &[""],
            ),
            field(2, FieldKind::CheckBox, FieldValue::State(None), &["Yes"]),
            field(
                3,
                FieldKind::Radio,
                FieldValue::State(Some("A".into())),
                &["A", "B"],
            ),
        ]);
        f
    }

    #[test]
    fn text_is_cut_to_its_maximum_length() {
        let mut f = form();
        assert!(f.update(Event::Text(0, "abcdef".into())));
        assert_eq!(
            f.changes(),
            [(
                ObjRef {
                    number: 1,
                    generation: 0
                },
                FieldValue::Text("abc".into())
            )]
        );
    }

    #[test]
    fn form_data_fills_in_what_fits() {
        let mut f = form();
        let data = [
            ("f1".to_owned(), "abcdef".to_owned()),
            ("f2".to_owned(), "Yes".to_owned()),
            ("f3".to_owned(), "Z".to_owned()),
            ("other".to_owned(), "x".to_owned()),
        ];
        // The radio button has no state Z, and there's no field named other.
        assert_eq!(f.import(&data), 2);
        assert_eq!(f.values[0], FieldValue::Text("abc".into()));
        assert_eq!(f.values[1], FieldValue::State(Some("Yes".into())));
        assert_eq!(f.values[2], FieldValue::State(Some("A".into())));
        let names: Vec<&str> = f.data().iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["f1", "f2", "f3"]);
    }

    #[test]
    fn checkboxes_toggle_and_radios_only_turn_on() {
        let mut f = form();
        f.update(Event::Press(1, Some("Yes".into())));
        assert_eq!(f.values[1], FieldValue::State(Some("Yes".into())));
        f.update(Event::Press(1, Some("Yes".into())));
        assert_eq!(f.values[1], FieldValue::State(None));
        assert!(!f.update(Event::Press(2, Some("A".into()))));
        assert!(f.update(Event::Press(2, Some("B".into()))));
        assert!(f.dirty());
    }

    #[test]
    fn saving_and_discarding() {
        let mut f = form();
        f.update(Event::Text(0, "x".into()));
        f.saved();
        assert!(!f.dirty() && f.changes().is_empty());
        f.update(Event::Text(0, "y".into()));
        f.discard();
        assert_eq!(f.values[0], FieldValue::Text("x".into()));
    }

    #[test]
    fn resetting_restores_defaults() {
        let mut f = form();
        assert!(f.can_reset());
        f.reset();
        assert_eq!(f.values[2], FieldValue::State(None));
        assert!(!f.can_reset() && f.dirty());
    }

    #[test]
    fn formatted_fields_filter_typing() {
        let mut f = Form::default();
        let mut amount = field(
            5,
            FieldKind::Text {
                multiline: false,
                password: false,
                max_len: None,
            },
            FieldValue::Text(String::new()),
            &[""],
        );
        amount.format = Some(panna_core::Format::Special(2));
        f.load(&[amount]);
        f.update(Event::Text(0, "555-12ab34".into()));
        assert_eq!(f.values[0], FieldValue::Text("555-1234".into()));
        assert_eq!(f.active, Some(0));
        f.leave();
        assert_eq!(f.active, None);
    }

    #[test]
    fn read_only_fields_ignore_edits() {
        let mut f = Form::default();
        let mut locked = field(4, FieldKind::CheckBox, FieldValue::State(None), &["Yes"]);
        locked.read_only = true;
        f.load(&[locked]);
        assert!(!f.update(Event::Press(0, Some("Yes".into()))));
    }
}
