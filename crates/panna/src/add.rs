//! Adding text, pictures and links to pages, and changing a line of text already there.
//! Unlike comments, these become part of the page itself, as a page change that can be
//! undone from the page tools.

use iced::widget::{column, container, pick_list, pin, row, space, text, text_editor, text_input};
use iced::{Center, Element, Theme};
use panna_core::{Addition, Color, LinkTarget, StampImage, TextBox};

use crate::Message;
use crate::annotate::SWATCHES;
use crate::icons::{self, Icon};
use crate::selection::PageMap;

/// Text sizes on offer, in points.
const SIZES: [u16; 9] = [8, 10, 11, 12, 14, 18, 24, 36, 48];
/// A drag shorter than this, in points, is a click.
const CLICK_SLOP: f32 = 3.0;
/// A clicked link's area, and the most a clicked picture takes up, in points.
const LINK_SIZE: (f32, f32) = (120.0, 18.0);
const PICTURE_SIZE: f32 = 150.0;
const MAX_TEXT: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Text,
    Picture,
    Link,
    /// Change a line of text already on the page.
    EditText,
}

#[derive(Debug, Clone)]
pub enum Event {
    Toggle,
    Tool(Tool),
    Edit(text_editor::Action),
    Size(u16),
    Color(usize),
    Address(String),
    Done,
    Cancel,
}

/// What's being typed for the area the user picked.
#[derive(Debug)]
enum Details {
    Text(text_editor::Content),
    Link(String),
    /// Waiting for a picture file to be chosen.
    Picture,
    /// A line already on the page, its text as it'll be and its size.
    Replace {
        content: text_editor::Content,
        was: String,
        size: f32,
    },
}

#[derive(Debug)]
pub struct Pending {
    page: usize,
    rect: TextBox,
    details: Details,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    page: usize,
    from: (f32, f32),
    to: (f32, f32),
}

#[derive(Debug)]
pub struct Adder {
    pub on: bool,
    pub tool: Tool,
    size: u16,
    color: usize,
    drag: Option<Drag>,
    pub pending: Option<Pending>,
}

impl Default for Adder {
    fn default() -> Self {
        Self {
            on: false,
            tool: Tool::Text,
            size: 12,
            // Black.
            color: SWATCHES.len() - 1,
            drag: None,
            pending: None,
        }
    }
}

/// What the main view does next.
pub enum Outcome {
    Nothing,
    /// Add this to page `page`.
    Add(usize, Addition),
    /// Ask for a picture file to place.
    PickPicture,
    /// Find the line of text at this point of page `page`, for [`Adder::edit_line`].
    FindLine(usize, (f32, f32)),
}

/// Reads what the user typed for a link: a page number, or a web or email address.
fn link_target(typed: &str) -> Option<LinkTarget> {
    let typed = typed.trim();
    if typed.is_empty() || typed.chars().any(char::is_whitespace) {
        return None;
    }
    if let Ok(number) = typed.parse::<usize>() {
        return number.checked_sub(1).map(LinkTarget::Page);
    }
    let lower = typed.to_ascii_lowercase();
    let uri = if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        typed.to_owned()
    } else if typed.contains('@') {
        format!("mailto:{typed}")
    } else if typed.contains('.') && !typed.contains(':') {
        format!("https://{typed}")
    } else {
        return None;
    };
    panna_core::safe_uri(&uri).then_some(LinkTarget::Uri(uri))
}

impl Adder {
    fn color_value(&self) -> Color {
        SWATCHES.get(self.color).copied().unwrap_or(Color::BLACK)
    }

    /// Whether a drag or a card is under way, for Esc.
    pub fn busy(&self) -> bool {
        self.drag.is_some() || self.pending.is_some()
    }

    pub fn cancel(&mut self) {
        self.drag = None;
        self.pending = None;
    }

    pub fn update(&mut self, event: Event, pages: usize) -> Outcome {
        match event {
            Event::Toggle => {
                self.on = !self.on;
                self.cancel();
            }
            Event::Tool(tool) => {
                self.tool = tool;
                self.cancel();
            }
            Event::Edit(action) => {
                if let Some(Pending {
                    details: Details::Text(content) | Details::Replace { content, .. },
                    ..
                }) = &mut self.pending
                {
                    let grows = action.is_edit();
                    content.perform(action);
                    if grows && content.text().chars().count() > MAX_TEXT {
                        content.perform(text_editor::Action::Edit(text_editor::Edit::Backspace));
                    }
                }
            }
            Event::Size(size) => self.size = size,
            Event::Color(index) => self.color = index.min(SWATCHES.len() - 1),
            Event::Address(value) => {
                if let Some(Pending {
                    details: Details::Link(typed),
                    ..
                }) = &mut self.pending
                {
                    *typed = value.chars().filter(|c| !c.is_control()).take(2_000).collect();
                }
            }
            Event::Cancel => self.cancel(),
            Event::Done => {
                let Some(pending) = self.pending.take() else {
                    return Outcome::Nothing;
                };
                let addition = match pending.details {
                    Details::Text(content) => {
                        let text = content.text();
                        let text = text.trim_end();
                        if text.trim().is_empty() {
                            return Outcome::Nothing;
                        }
                        Addition::Text {
                            at: (pending.rect.x0, pending.rect.y0),
                            text: text.to_owned(),
                            size: f32::from(self.size),
                            color: self.color_value(),
                        }
                    }
                    Details::Link(typed) => match link_target(&typed) {
                        Some(LinkTarget::Page(p)) if p >= pages => {
                            self.pending = Some(Pending {
                                details: Details::Link(typed),
                                ..pending
                            });
                            return Outcome::Nothing;
                        }
                        Some(target) => Addition::Link {
                            rect: pending.rect,
                            target,
                        },
                        None => {
                            self.pending = Some(Pending {
                                details: Details::Link(typed),
                                ..pending
                            });
                            return Outcome::Nothing;
                        }
                    },
                    Details::Replace { content, was, size } => {
                        let text = content.text();
                        let text = text.trim_end();
                        if text == was {
                            return Outcome::Nothing;
                        }
                        Addition::ReplaceText {
                            area: pending.rect,
                            text: text.to_owned(),
                            size,
                            color: None,
                        }
                    }
                    Details::Picture => return Outcome::Nothing,
                };
                return Outcome::Add(pending.page, addition);
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

    /// The end of a click or drag: asks for what goes there.
    pub fn release(&mut self) -> Outcome {
        let Some(Drag { page, from, to }) = self.drag.take() else {
            return Outcome::Nothing;
        };
        let rect = TextBox {
            x0: from.0.min(to.0),
            y0: from.1.min(to.1),
            x1: from.0.max(to.0),
            y1: from.1.max(to.1),
        };
        let dragged = rect.width() >= CLICK_SLOP && rect.height() >= CLICK_SLOP;
        let (rect, details) = match self.tool {
            Tool::EditText => return Outcome::FindLine(page, to),
            Tool::Text => (rect, Details::Text(text_editor::Content::new())),
            Tool::Link if dragged => (rect, Details::Link(String::new())),
            Tool::Link => (
                TextBox {
                    x0: from.0,
                    y0: from.1 - LINK_SIZE.1 / 2.0,
                    x1: from.0 + LINK_SIZE.0,
                    y1: from.1 + LINK_SIZE.1 / 2.0,
                },
                Details::Link(String::new()),
            ),
            // An empty box for a click: the picture's own shape decides its size.
            Tool::Picture if dragged => (rect, Details::Picture),
            Tool::Picture => (
                TextBox {
                    x0: from.0,
                    y0: from.1,
                    x1: from.0,
                    y1: from.1,
                },
                Details::Picture,
            ),
        };
        let picking = matches!(details, Details::Picture);
        self.pending = Some(Pending { page, rect, details });
        if picking {
            Outcome::PickPicture
        } else {
            Outcome::Nothing
        }
    }

    /// Opens the card for changing `line` (its text, box and size) on page `page`.
    pub fn edit_line(&mut self, page: usize, line: Option<(String, TextBox, f32)>) {
        if let Some((was, rect, size)) = line {
            self.pending = Some(Pending {
                page,
                rect,
                details: Details::Replace {
                    content: text_editor::Content::with_text(&was),
                    was,
                    size: size.clamp(1.0, 500.0),
                },
            });
        }
    }

    /// Places the chosen picture, keeping its shape, inside the dragged box or at a usual
    /// size from the clicked point.
    pub fn picture(&mut self, image: Option<StampImage>) -> Outcome {
        let Some(pending) = self.pending.take_if(|p| matches!(p.details, Details::Picture)) else {
            return Outcome::Nothing;
        };
        let Some(image) = image else {
            return Outcome::Nothing;
        };
        let aspect = image.width as f32 / image.height.max(1) as f32;
        let at = pending.rect;
        let (w, h) = if at.width() >= CLICK_SLOP && at.height() >= CLICK_SLOP {
            let w = at.width().min(at.height() * aspect);
            (w, w / aspect)
        } else if aspect >= 1.0 {
            (PICTURE_SIZE, PICTURE_SIZE / aspect)
        } else {
            (PICTURE_SIZE * aspect, PICTURE_SIZE)
        };
        let rect = TextBox {
            x0: at.x0,
            y0: at.y0,
            x1: at.x0 + w,
            y1: at.y0 + h,
        };
        Outcome::Add(pending.page, Addition::Picture { rect, image })
    }

    /// The box being dragged on page `page`, drawn over it.
    pub fn overlay(&self, page: usize, map: PageMap) -> Option<Element<'_, Message>> {
        let rect = match (&self.drag, &self.pending) {
            (Some(d), _) if d.page == page => TextBox {
                x0: d.from.0.min(d.to.0),
                y0: d.from.1.min(d.to.1),
                x1: d.from.0.max(d.to.0),
                y1: d.from.1.max(d.to.1),
            },
            (None, Some(p)) if p.page == page && !matches!(p.details, Details::Text(_)) => p.rect,
            _ => return None,
        };
        let a = map.to_view(rect.x0, rect.y0);
        let b = map.to_view(rect.x1, rect.y1);
        let (x, y) = (a.x.min(b.x), a.y.min(b.y));
        let (w, h) = ((a.x - b.x).abs().max(1.0), (a.y - b.y).abs().max(1.0));
        Some(
            pin(container(space()).width(w).height(h).style(|theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style::default()
                    .background(palette.primary.base.color.scale_alpha(0.12))
                    .border(iced::Border {
                        color: palette.primary.base.color,
                        width: 1.5,
                        radius: 0.0.into(),
                    })
            }))
            .x(x)
            .y(y)
            .into(),
        )
    }

    /// The floating bar with the three tools.
    pub fn bar(&self) -> Element<'_, Message> {
        let ev = Message::Add;
        let tool = |tool: Tool, icon: Icon, tip: &'static str| {
            icons::toggle(icon, tip, self.tool == tool, ev(Event::Tool(tool)))
        };
        let hint = match self.tool {
            Tool::Text => "Click where the text starts.",
            Tool::Picture => "Click, or drag a box, where the picture goes.",
            Tool::Link => "Drag a box over what the link covers.",
            Tool::EditText => "Click a line of text to change it.",
        };
        let content = row![
            tool(Tool::Text, Icon::TextBox, "Add text to the page"),
            tool(Tool::Picture, Icon::AddPicture, "Add a picture to the page"),
            tool(Tool::Link, Icon::AddLink, "Add a link to a page or address"),
            tool(Tool::EditText, Icon::EditText, "Change text on the page"),
            text(hint).size(13).width(290),
            icons::button(Icon::Close, "Stop adding to pages (Esc)", Some(ev(Event::Toggle))),
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

    /// The card asking for the text or the link's target.
    pub fn card(&self) -> Option<Element<'_, Message>> {
        let pending = self.pending.as_ref()?;
        let ev = Message::Add;
        let buttons = |ready: bool, label: &'static str| {
            row![
                space::horizontal(),
                iced::widget::button("Cancel")
                    .style(iced::widget::button::secondary)
                    .on_press(ev(Event::Cancel)),
                iced::widget::button(label).on_press_maybe(ready.then(|| ev(Event::Done))),
            ]
            .spacing(8)
        };
        let body: Element<'_, Message> = match &pending.details {
            Details::Picture => return None,
            Details::Text(content) => {
                let mut swatches = row![].spacing(4).align_y(Center);
                for (i, color) in SWATCHES.iter().enumerate() {
                    swatches = swatches.push(crate::annotate::swatch(
                        *color,
                        i == self.color,
                        ev(Event::Color(i)),
                    ));
                }
                column![
                    text("Add text").size(16),
                    text_editor(content)
                        .id(editor_id())
                        .placeholder("Type here")
                        .on_action(move |a| ev(Event::Edit(a)))
                        .height(120),
                    row![
                        pick_list(SIZES, Some(self.size), move |s| ev(Event::Size(s))).text_size(13),
                        text("pt").size(13),
                        space::horizontal(),
                        swatches,
                    ]
                    .spacing(6)
                    .align_y(Center),
                    text("The text becomes part of the page, in Helvetica.").size(12),
                    buttons(!content.text().trim().is_empty(), "Add"),
                ]
                .spacing(10)
                .into()
            }
            Details::Replace { content, was, size } => column![
                text("Change text").size(16),
                text_editor(content)
                    .id(editor_id())
                    .on_action(move |a| ev(Event::Edit(a)))
                    .height(80),
                text(format!(
                    "The old text is taken out, and the new text drawn in its place in \
                     Helvetica, at {size:.0} pt, in the old text's colour."
                ))
                .size(12),
                buttons(content.text().trim_end() != was, "Change"),
            ]
            .spacing(10)
            .into(),
            Details::Link(typed) => {
                let target = link_target(typed);
                let note = match &target {
                    _ if typed.trim().is_empty() => String::new(),
                    Some(LinkTarget::Page(p)) => format!("Goes to page {}.", p + 1),
                    Some(LinkTarget::Uri(uri)) => format!("Opens {uri}"),
                    None => "Type a page number, a web address or an email address.".into(),
                };
                column![
                    text("Add a link").size(16),
                    text_input("Page number, web address or email address", typed)
                        .id(link_input_id())
                        .on_input(move |v| ev(Event::Address(v)))
                        .on_submit(ev(Event::Done)),
                    text(note).size(12),
                    buttons(target.is_some(), "Add"),
                ]
                .spacing(10)
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
        match &self.pending.as_ref()?.details {
            Details::Text(_) | Details::Replace { .. } => Some(editor_id()),
            Details::Link(_) => Some(link_input_id()),
            Details::Picture => None,
        }
    }
}

fn editor_id() -> iced::widget::Id {
    iced::widget::Id::new("add-text")
}

fn link_input_id() -> iced::widget::Id {
    iced::widget::Id::new("add-link")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_take_page_numbers_and_addresses() {
        assert_eq!(link_target("3"), Some(LinkTarget::Page(2)));
        assert_eq!(link_target("0"), None);
        assert_eq!(
            link_target("example.com/a"),
            Some(LinkTarget::Uri("https://example.com/a".into()))
        );
        assert_eq!(
            link_target("jo@example.com"),
            Some(LinkTarget::Uri("mailto:jo@example.com".into()))
        );
        assert_eq!(
            link_target("HTTP://x.org"),
            Some(LinkTarget::Uri("HTTP://x.org".into()))
        );
        assert_eq!(link_target("javascript:alert(1)"), None);
        assert_eq!(link_target("file:///etc/passwd"), None);
        assert_eq!(link_target("two words"), None);
    }

    #[test]
    fn a_click_asks_for_text_then_adds_it_there() {
        let mut adder = Adder {
            on: true,
            ..Adder::default()
        };
        adder.press(0, (30.0, 40.0));
        assert!(matches!(adder.release(), Outcome::Nothing));
        assert!(adder.card().is_some());
        adder.update(
            Event::Edit(text_editor::Action::Edit(text_editor::Edit::Insert('A'))),
            1,
        );
        let Outcome::Add(0, Addition::Text { at, text, size, .. }) = adder.update(Event::Done, 1) else {
            panic!("nothing added");
        };
        assert_eq!((at, text.as_str(), size), ((30.0, 40.0), "A", 12.0));
        assert!(!adder.busy());
    }

    #[test]
    fn a_link_to_a_missing_page_waits_for_a_better_one() {
        let mut adder = Adder {
            on: true,
            tool: Tool::Link,
            ..Adder::default()
        };
        adder.press(1, (10.0, 10.0));
        adder.hover(1, (60.0, 30.0));
        adder.release();
        adder.update(Event::Address("5".into()), 2);
        assert!(matches!(adder.update(Event::Done, 2), Outcome::Nothing));
        assert!(adder.pending.is_some());
        adder.update(Event::Address("2".into()), 2);
        assert!(matches!(
            adder.update(Event::Done, 2),
            Outcome::Add(1, Addition::Link { rect, target: LinkTarget::Page(1) }) if rect.x1 == 60.0
        ));
    }

    #[test]
    fn a_clicked_picture_keeps_its_shape() {
        let mut adder = Adder {
            on: true,
            tool: Tool::Picture,
            ..Adder::default()
        };
        adder.press(0, (10.0, 20.0));
        assert!(matches!(adder.release(), Outcome::PickPicture));
        let wide = StampImage::from_rgba(4, 2, &[0; 32]).unwrap();
        let Outcome::Add(0, Addition::Picture { rect, .. }) = adder.picture(Some(wide)) else {
            panic!("no picture");
        };
        assert_eq!((rect.x0, rect.y0, rect.x1, rect.y1), (10.0, 20.0, 160.0, 95.0));
    }
}
