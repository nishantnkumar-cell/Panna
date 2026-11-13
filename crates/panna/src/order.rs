//! The Reading order panel, as Acrobat's Reading Order tool: a tagged document's paragraphs,
//! headings and figures in the order a screen reader reads them, numbered on the pages too.
//! They can be moved up or down and given other roles, and the result saved as a copy.

use std::path::PathBuf;

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{Column, button, column, container, pick_list, row, scrollable, space, text};
use iced::{Center, Element, Fill, Point, Rectangle, Renderer, Size, Theme, mouse};
use panna_core::{ORDER_ROLES, OrderItem, ReadingOrder};

use crate::Message;
use crate::icons::{self, Icon};
use crate::selection::PageMap;

#[derive(Debug, Clone)]
pub struct Ordering {
    /// The file the order was read from.
    pub source: PathBuf,
    items: Vec<OrderItem>,
    /// Each item's role now, in the items' order.
    roles: Vec<String>,
    editable: bool,
    note: Option<String>,
    loading: bool,
    pub busy: bool,
    selected: Option<usize>,
    error: Option<String>,
    /// Something has been moved.
    moved: bool,
}

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Loaded(Result<ReadingOrder, String>),
    Select(usize),
    Up(usize),
    Down(usize),
    Role(usize, String),
    Save,
    SaveTo(Option<PathBuf>),
    Saved(Result<PathBuf, String>),
    Close,
}

impl Ordering {
    pub fn new(source: PathBuf) -> Self {
        Ordering {
            source,
            items: Vec::new(),
            roles: Vec::new(),
            editable: false,
            note: None,
            loading: true,
            busy: false,
            selected: None,
            error: None,
            moved: false,
        }
    }

    pub fn load(&mut self, order: ReadingOrder) {
        self.roles = order.items.iter().map(|i| i.role.clone()).collect();
        self.items = order.items;
        self.editable = order.editable;
        self.note = order.note;
        self.loading = false;
        self.selected = None;
        self.moved = false;
    }

    pub fn fail(&mut self, error: String) {
        self.loading = false;
        self.busy = false;
        self.error = Some(error);
    }

    /// Whether anything has been moved or given another role.
    fn changed(&self) -> bool {
        self.items.iter().zip(&self.roles).any(|(i, r)| i.role != *r) || self.moved
    }

    /// The order to save: each element and its role.
    pub fn order(&self) -> Vec<(panna_core::ObjRef, String)> {
        self.items
            .iter()
            .zip(&self.roles)
            .map(|(i, r)| (i.id, r.clone()))
            .collect()
    }

    /// Handles an event that only changes the panel; the page to show, when one should be.
    pub fn update(&mut self, event: &Event) -> Option<usize> {
        match event {
            Event::Select(i) => {
                self.selected = Some(*i);
                return self.items.get(*i).and_then(|i| i.page);
            }
            Event::Up(i) if *i > 0 && *i < self.items.len() && self.editable => {
                self.items.swap(*i, i - 1);
                self.roles.swap(*i, i - 1);
                self.selected = Some(i - 1);
                self.moved = true;
            }
            Event::Down(i) if i + 1 < self.items.len() && self.editable => {
                self.items.swap(*i, i + 1);
                self.roles.swap(*i, i + 1);
                self.selected = Some(i + 1);
                self.moved = true;
            }
            Event::Role(i, role) if self.editable => {
                if let Some(r) = self.roles.get_mut(*i) {
                    r.clone_from(role);
                }
            }
            _ => {}
        }
        None
    }

    /// Numbered boxes for the elements on `page`, drawn over it.
    pub fn overlay<'a>(&self, page: usize, map: PageMap, size: Size) -> Option<Element<'a, Message>> {
        let boxes: Vec<(usize, Rectangle, bool)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.page == Some(page))
            .filter_map(|(n, item)| {
                let area = item.area?;
                let a = map.to_view(area.x0, area.y0);
                let b = map.to_view(area.x1, area.y1);
                let rect = Rectangle::new(
                    Point::new(a.x.min(b.x), a.y.min(b.y)),
                    Size::new((a.x - b.x).abs().max(4.0), (a.y - b.y).abs().max(4.0)),
                );
                Some((n, rect, self.selected == Some(n)))
            })
            .collect();
        if boxes.is_empty() {
            return None;
        }
        Some(
            canvas::Canvas::new(Boxes { boxes })
                .width(size.width)
                .height(size.height)
                .into(),
        )
    }
}

/// What a role is called in the list.
fn role_name(role: &str) -> String {
    match role {
        "P" => "Paragraph".into(),
        "H1" | "H2" | "H3" | "H4" | "H5" | "H6" => format!("Heading {}", &role[1..]),
        "Figure" => "Figure".into(),
        "Table" => "Table".into(),
        "L" => "List".into(),
        "Note" => "Note".into(),
        "Caption" => "Caption".into(),
        "Sect" => "Section".into(),
        other => other.to_owned(),
    }
}

/// A role in the drop-down.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Role(String);

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&role_name(&self.0))
    }
}

/// The panel, showing the elements on `page` (all of them when `page` is `None`).
pub fn panel(state: &Ordering, page: usize) -> Element<'_, Message> {
    let ev = Message::Order;
    let mut body = column![
        row![
            icons::glyph::<Message>(Icon::ReadingOrder, true),
            text("Reading order").size(14),
            space::horizontal(),
            icons::button(Icon::Close, "Close the reading order", Some(ev(Event::Close))),
        ]
        .spacing(6)
        .align_y(Center),
    ]
    .spacing(8)
    .width(340);
    if state.loading {
        body = body.push(text("Reading the tags…").size(13));
    }
    if let Some(note) = &state.note {
        body = body.push(text(note.as_str()).size(12).style(text::secondary));
    }
    let on_page: Vec<usize> = (0..state.items.len())
        .filter(|&i| state.items.get(i).and_then(|i| i.page) == Some(page))
        .collect();
    if !state.loading && !state.items.is_empty() {
        body = body.push(
            text(format!(
                "Page {}: {} of {} elements. Screen readers read them in this order.",
                page + 1,
                on_page.len(),
                state.items.len()
            ))
            .size(12)
            .style(text::secondary),
        );
    }
    let roles: Vec<Role> = ORDER_ROLES.iter().map(|r| Role((*r).to_owned())).collect();
    let mut list = Column::new().spacing(4).width(Fill);
    for &i in &on_page {
        let (Some(item), Some(role)) = (state.items.get(i), state.roles.get(i)) else {
            continue;
        };
        let picked = state.selected == Some(i);
        let preview = if item.preview.is_empty() {
            role_name(role)
        } else {
            item.preview.clone()
        };
        let role_list: Element<'_, Message> = if state.editable {
            pick_list(roles.clone(), Some(Role(role.clone())), move |r: Role| {
                ev(Event::Role(i, r.0))
            })
            .text_size(12)
            .padding([2, 6])
            .into()
        } else {
            text(role_name(role)).size(12).into()
        };
        let entry = column![
            row![
                text(format!("{}", i + 1)).size(13).width(28),
                role_list,
                space::horizontal(),
                icons::button(
                    Icon::MoveUp,
                    "Read earlier",
                    (state.editable && i > 0).then_some(ev(Event::Up(i)))
                ),
                icons::button(
                    Icon::MoveDown,
                    "Read later",
                    (state.editable && i + 1 < state.items.len()).then_some(ev(Event::Down(i)))
                ),
            ]
            .spacing(4)
            .align_y(Center),
            button(text(preview).size(12))
                .style(if picked { button::secondary } else { button::text })
                .width(Fill)
                .padding([2, 6])
                .on_press(ev(Event::Select(i))),
        ]
        .spacing(2);
        list = list.push(entry);
    }
    if !state.loading && on_page.is_empty() && !state.items.is_empty() {
        list = list.push(text("Nothing on this page is tagged.").size(12));
    }
    body = body.push(scrollable(list).height(iced::Length::Fixed(380.0)));
    if let Some(error) = &state.error {
        body = body.push(text(error.as_str()).size(12).style(text::danger));
    }
    if state.editable {
        body = body.push(
            row![
                space::horizontal(),
                button(if state.busy { "Saving…" } else { "Save as…" })
                    .on_press_maybe((state.changed() && !state.busy).then_some(ev(Event::Save))),
            ]
            .spacing(8),
        );
    }
    container(body)
        .padding(10)
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

struct Boxes {
    boxes: Vec<(usize, Rectangle, bool)>,
}

impl canvas::Program<Message> for Boxes {
    type State = ();

    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let palette = theme.extended_palette();
        for (n, rect, picked) in &self.boxes {
            let color = if *picked {
                palette.danger.base.color
            } else {
                palette.primary.base.color
            };
            frame.fill_rectangle(rect.position(), rect.size(), color.scale_alpha(0.08));
            frame.stroke(
                &Path::rectangle(rect.position(), rect.size()),
                Stroke::default()
                    .with_color(color)
                    .with_width(if *picked { 2.0 } else { 1.0 }),
            );
            let label = format!("{}", n + 1);
            let tag = Size::new(8.0 + 7.0 * label.len() as f32, 16.0);
            let at = Point::new(rect.x, (rect.y - tag.height).max(0.0));
            frame.fill_rectangle(at, tag, color);
            frame.fill_text(Text {
                content: label,
                position: Point::new(at.x + 4.0, at.y + 2.0),
                color: iced::Color::WHITE,
                size: 12.0.into(),
                ..Text::default()
            });
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(n: u32, role: &str) -> OrderItem {
        OrderItem {
            id: panna_core::ObjRef {
                number: n,
                generation: 0,
            },
            role: role.into(),
            page: Some(0),
            preview: String::new(),
            area: None,
        }
    }

    #[test]
    fn elements_move_and_take_new_roles() {
        let mut state = Ordering::new(PathBuf::from("a.pdf"));
        state.load(ReadingOrder {
            items: vec![item(1, "H1"), item(2, "P"), item(3, "P")],
            editable: true,
            note: None,
        });
        assert!(!state.changed());
        state.update(&Event::Down(0));
        state.update(&Event::Role(0, "H2".into()));
        // Moving past the ends does nothing.
        state.update(&Event::Up(0));
        state.update(&Event::Down(2));
        let order: Vec<(u32, String)> = state.order().into_iter().map(|(id, r)| (id.number, r)).collect();
        assert_eq!(order, [(2, "H2".into()), (1, "H1".into()), (3, "P".into())]);
        assert!(state.changed());
    }
}
