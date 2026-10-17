//! The navigation sidebar: page thumbnails and bookmarks, docked to the left or right edge.

use std::ops::Range;

use iced::widget::scrollable::Viewport;
use iced::widget::{Column, button, column, container, image, row, scrollable, space, text};
use iced::{Center, Element, Fill, Length, Theme};
use panna_core::{OutlineItem, PageSize};

use crate::Message;
use crate::icons::{self, Icon};

pub const WIDTH: f32 = 200.0;
/// Widest thumbnail; tall pages are narrower so no thumbnail exceeds `THUMB_MAX_H`.
pub const THUMB_W: f32 = 132.0;
const THUMB_MAX_H: f32 = 200.0;
const LABEL_H: f32 = 18.0;
const GAP: f32 = 12.0;
const PAD: f32 = 12.0;
/// Thumbnails kept beyond the visible ones, in each direction.
const KEEP: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Thumbnails,
    Bookmarks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Side {
    #[default]
    Left,
    Right,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }

    pub fn parse(s: &str) -> Option<Side> {
        match s {
            "left" => Some(Side::Left),
            "right" => Some(Side::Right),
            _ => None,
        }
    }
}

impl Tab {
    pub fn as_str(self) -> &'static str {
        match self {
            Tab::Thumbnails => "thumbnails",
            Tab::Bookmarks => "bookmarks",
        }
    }

    pub fn parse(s: &str) -> Option<Tab> {
        match s {
            "thumbnails" => Some(Tab::Thumbnails),
            "bookmarks" => Some(Tab::Bookmarks),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Toggle,
    Tab(Tab),
    SwapSide,
    Scrolled(Viewport),
}

#[derive(Debug, Clone, Copy)]
pub struct Sidebar {
    pub open: bool,
    pub side: Side,
    pub tab: Tab,
    offset: f32,
    height: f32,
}

impl Sidebar {
    pub fn new(open: bool, side: Side, tab: Tab) -> Self {
        Self {
            open,
            side,
            tab,
            offset: 0.0,
            height: 800.0,
        }
    }

    /// Applies an event. Returns true when a saved preference changed.
    pub fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Toggle => self.open = !self.open,
            Event::Tab(tab) => self.tab = tab,
            Event::SwapSide => {
                self.side = match self.side {
                    Side::Left => Side::Right,
                    Side::Right => Side::Left,
                }
            }
            Event::Scrolled(viewport) => {
                self.offset = viewport.absolute_offset().y;
                self.height = viewport.bounds().height;
                return false;
            }
        }
        true
    }

    /// Whether thumbnails are on screen and worth rendering.
    pub fn showing_thumbnails(&self) -> bool {
        self.open && self.tab == Tab::Thumbnails
    }

    /// Thumbnails to keep rendered: the visible ones plus a margin.
    pub fn wanted(&self, layout: &ThumbLayout) -> Range<usize> {
        let visible = layout.visible(self.offset, self.height);
        visible.start.saturating_sub(KEEP)..(visible.end + KEEP).min(layout.len())
    }
}

/// Thumbnail sizes and positions inside the sidebar's scroll area.
#[derive(Debug, Default, Clone)]
pub struct ThumbLayout {
    sizes: Vec<(f32, f32)>,
    tops: Vec<f32>,
}

impl ThumbLayout {
    pub fn new(pages: &[PageSize]) -> Self {
        let mut tops = Vec::with_capacity(pages.len());
        let mut y = PAD;
        let sizes = pages
            .iter()
            .map(|p| {
                let (w, h) = thumb_size(*p);
                tops.push(y);
                y += h + LABEL_H + GAP;
                (w, h)
            })
            .collect();
        Self { sizes, tops }
    }

    pub fn len(&self) -> usize {
        self.sizes.len()
    }

    /// Thumbnail size in logical pixels for page `index`.
    pub fn size(&self, index: usize) -> (f32, f32) {
        self.sizes.get(index).copied().unwrap_or((THUMB_W, THUMB_W))
    }

    fn visible(&self, offset: f32, height: f32) -> Range<usize> {
        if self.tops.is_empty() {
            return 0..0;
        }
        let first = self.tops.partition_point(|&top| top <= offset).saturating_sub(1);
        let last = self.tops.partition_point(|&top| top < offset + height.max(1.0));
        first..last.max(first + 1).min(self.tops.len())
    }
}

fn thumb_size(page: PageSize) -> (f32, f32) {
    if page.width <= 0.0 || page.height <= 0.0 || !page.width.is_finite() || !page.height.is_finite() {
        return (THUMB_W, THUMB_W);
    }
    let scale = (THUMB_W / page.width).min(THUMB_MAX_H / page.height);
    ((page.width * scale).max(8.0), (page.height * scale).max(8.0))
}

fn thumbs_id() -> iced::widget::Id {
    iced::widget::Id::new("thumbnails")
}

pub struct ViewData<'a> {
    pub layout: &'a ThumbLayout,
    pub thumbs_images: &'a [crate::Thumb],
    pub outline: &'a [OutlineItem],
    pub current: usize,
}

impl Sidebar {
    pub fn view<'a>(&self, data: ViewData<'a>) -> Element<'a, Message> {
        let ev = Message::Sidebar;
        let tab_button = |tab: Tab, icon: Icon, tip: &str| {
            // The active tab shows disabled, like a selected segment.
            icons::button(
                icon,
                tip.to_owned(),
                (self.tab != tab).then(|| ev(Event::Tab(tab))),
            )
        };
        let side_tip = match self.side {
            Side::Left => "Move sidebar to the right",
            Side::Right => "Move sidebar to the left",
        };
        let header = row![
            tab_button(Tab::Thumbnails, Icon::Thumbnails, "Page thumbnails"),
            tab_button(Tab::Bookmarks, Icon::Bookmarks, "Bookmarks"),
            space::horizontal(),
            icons::button(Icon::SwapSide, side_tip, Some(ev(Event::SwapSide))),
            icons::button(Icon::Close, "Hide sidebar (F4)", Some(ev(Event::Toggle))),
        ]
        .spacing(2)
        .padding([4, 6])
        .align_y(Center);

        let content = match self.tab {
            Tab::Thumbnails => self.thumbnails(&data),
            Tab::Bookmarks => bookmarks(data.outline),
        };
        container(column![header, content])
            .width(WIDTH)
            .height(Fill)
            .style(|theme: &Theme| {
                container::Style::default().background(theme.extended_palette().background.weak.color)
            })
            .into()
    }

    fn thumbnails<'a>(&self, data: &ViewData<'a>) -> Element<'a, Message> {
        let items = (0..data.layout.len()).map(|index| {
            let (w, h) = data.layout.size(index);
            let current = index == data.current;
            let picture: Element<'a, Message> =
                match data.thumbs_images.get(index).and_then(|t| t.image.as_ref()) {
                    Some(handle) => image(handle.clone()).width(w).height(h).into(),
                    None => space().width(w).height(h).into(),
                };
            let frame =
                container(picture)
                    .padding(if current { 3 } else { 1 })
                    .style(move |theme: &Theme| {
                        let palette = theme.extended_palette();
                        container::Style::default()
                            .background(iced::Color::WHITE)
                            .border(iced::Border {
                                color: if current {
                                    palette.primary.base.color
                                } else {
                                    palette.background.strong.color
                                },
                                width: if current { 3.0 } else { 1.0 },
                                radius: 2.0.into(),
                            })
                    });
            let entry = column![frame, text((index + 1).to_string()).size(12).height(LABEL_H)]
                .align_x(Center)
                .spacing(2);
            button(entry)
                .padding(0)
                .style(button::text)
                .on_press(Message::GoToPage(index))
                .into()
        });
        scrollable(
            Column::with_children(items)
                .spacing(GAP - 2.0)
                .padding(PAD)
                .align_x(Center)
                .width(Fill),
        )
        .id(thumbs_id())
        .on_scroll(|v| Message::Sidebar(Event::Scrolled(v)))
        .height(Fill)
        .into()
    }
}

fn bookmarks(outline: &[OutlineItem]) -> Element<'_, Message> {
    if outline.is_empty() {
        return container(
            text("This document has no bookmarks.")
                .size(13)
                .style(text::secondary),
        )
        .padding(PAD)
        .into();
    }
    let items = outline.iter().map(|item| {
        let title = if item.title.is_empty() {
            "(untitled)"
        } else {
            item.title.as_str()
        };
        let indent = f32::from(item.depth.min(8)) * 12.0;
        let label = text(title).size(13).wrapping(text::Wrapping::WordOrGlyph);
        row![
            space().width(indent),
            button(label)
                .padding([3, 6])
                .width(Length::Fill)
                .style(button::text)
                .on_press_maybe(item.page.map(Message::GoToPage)),
        ]
        .into()
    });
    scrollable(Column::with_children(items).padding([4, 6]).width(Fill))
        .height(Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter(n: usize) -> ThumbLayout {
        ThumbLayout::new(&vec![
            PageSize {
                width: 612.0,
                height: 792.0
            };
            n
        ])
    }

    #[test]
    fn thumbnails_fit_the_box() {
        let (w, h) = thumb_size(PageSize {
            width: 612.0,
            height: 792.0,
        });
        assert!(w <= THUMB_W + 0.01 && h <= THUMB_MAX_H + 0.01);
        let (w, h) = thumb_size(PageSize {
            width: 100.0,
            height: 10_000.0,
        });
        assert!(h <= THUMB_MAX_H + 0.01 && w >= 8.0);
        assert_eq!(
            thumb_size(PageSize {
                width: f32::NAN,
                height: 1.0
            }),
            (THUMB_W, THUMB_W)
        );
    }

    #[test]
    fn visible_range_follows_scroll() {
        let layout = letter(100);
        assert_eq!(layout.visible(0.0, 400.0).start, 0);
        let top_50 = layout.tops[50];
        let range = layout.visible(top_50 + 1.0, 300.0);
        assert!(range.contains(&50) && range.end <= 53);
        assert_eq!(ThumbLayout::default().visible(0.0, 100.0), 0..0);
    }

    #[test]
    fn wanted_adds_margin_and_clamps() {
        let mut bar = Sidebar::new(true, Side::Left, Tab::Thumbnails);
        bar.height = 300.0;
        let layout = letter(4);
        assert_eq!(bar.wanted(&layout), 0..4);
    }
}
