//! Toolbar icons from Lucide (ISC license, see `icons/LICENSE-lucide`), embedded in the binary.
//!
//! Every icon button carries a hover tooltip naming the action and its shortcut, so the
//! toolbar needs no words.

use iced::widget::{self, container, svg, text, tooltip};
use iced::{Element, Length, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Open,
    PrevPage,
    NextPage,
    ZoomIn,
    ZoomOut,
    FitWidth,
    FitPage,
    RotateCcw,
    RotateCw,
    Properties,
    GripVertical,
    GripHorizontal,
    DockMenu,
    DockTop,
    DockBottom,
    DockLeft,
    DockRight,
    Float,
    Sidebar,
    Thumbnails,
    Bookmarks,
    SwapSide,
    Close,
}

impl Icon {
    fn bytes(self) -> &'static [u8] {
        match self {
            Icon::Open => include_bytes!("../icons/folder-open.svg"),
            Icon::PrevPage => include_bytes!("../icons/chevron-left.svg"),
            Icon::NextPage => include_bytes!("../icons/chevron-right.svg"),
            Icon::ZoomIn => include_bytes!("../icons/zoom-in.svg"),
            Icon::ZoomOut => include_bytes!("../icons/zoom-out.svg"),
            Icon::FitWidth => include_bytes!("../icons/move-horizontal.svg"),
            Icon::FitPage => include_bytes!("../icons/maximize.svg"),
            Icon::RotateCcw => include_bytes!("../icons/rotate-ccw.svg"),
            Icon::RotateCw => include_bytes!("../icons/rotate-cw.svg"),
            Icon::Properties => include_bytes!("../icons/info.svg"),
            Icon::GripVertical => include_bytes!("../icons/grip-vertical.svg"),
            Icon::GripHorizontal => include_bytes!("../icons/grip-horizontal.svg"),
            Icon::DockMenu => include_bytes!("../icons/pin.svg"),
            Icon::DockTop => include_bytes!("../icons/panel-top.svg"),
            Icon::DockBottom => include_bytes!("../icons/panel-bottom.svg"),
            Icon::DockLeft => include_bytes!("../icons/panel-left.svg"),
            Icon::DockRight => include_bytes!("../icons/panel-right.svg"),
            Icon::Float => include_bytes!("../icons/picture-in-picture-2.svg"),
            Icon::Sidebar => include_bytes!("../icons/panel-left.svg"),
            Icon::Thumbnails => include_bytes!("../icons/layout-grid.svg"),
            Icon::Bookmarks => include_bytes!("../icons/bookmark.svg"),
            Icon::SwapSide => include_bytes!("../icons/arrow-left-right.svg"),
            Icon::Close => include_bytes!("../icons/x.svg"),
        }
    }
}

/// The platform's main shortcut modifier, as users see it on their keyboard.
pub const MOD: &str = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };

const ICON_SIZE: f32 = 18.0;

/// A bare icon tinted with the theme's text colour (dimmed when `enabled` is false).
pub fn glyph<'a, M: 'a>(icon: Icon, enabled: bool) -> Element<'a, M> {
    svg(svg::Handle::from_memory(icon.bytes()))
        .width(ICON_SIZE)
        .height(ICON_SIZE)
        .style(move |theme: &Theme, _status| {
            let mut color = theme.palette().text;
            if !enabled {
                color.a = 0.35;
            }
            svg::Style { color: Some(color) }
        })
        .into()
}

/// An icon button with a hover tooltip. `on_press: None` shows it disabled.
pub fn button<'a, M: Clone + 'a>(icon: Icon, tip: impl Into<String>, on_press: Option<M>) -> Element<'a, M> {
    let enabled = on_press.is_some();
    let button = widget::button(glyph(icon, enabled))
        .padding(6)
        .width(Length::Shrink)
        .style(move |theme: &Theme, status| {
            use widget::button::Status;
            let palette = theme.extended_palette();
            let background = match status {
                Status::Hovered if enabled => Some(palette.background.weak.color.into()),
                Status::Pressed if enabled => Some(palette.background.strong.color.into()),
                _ => None,
            };
            widget::button::Style {
                background,
                text_color: palette.background.base.text,
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .on_press_maybe(on_press);
    with_tip(button, tip)
}

/// Wraps any element in a hover tooltip.
pub fn with_tip<'a, M: 'a>(content: impl Into<Element<'a, M>>, tip: impl Into<String>) -> Element<'a, M> {
    let label = container(text(tip.into()).size(13))
        .padding([4, 8])
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style::default()
                .background(palette.background.strong.color)
                .color(palette.background.strong.text)
                .border(iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                })
        });
    tooltip(content, label, tooltip::Position::Bottom)
        .gap(4)
        .snap_within_viewport(true)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Icon; 23] = [
        Icon::Open,
        Icon::PrevPage,
        Icon::NextPage,
        Icon::ZoomIn,
        Icon::ZoomOut,
        Icon::FitWidth,
        Icon::FitPage,
        Icon::RotateCcw,
        Icon::RotateCw,
        Icon::Properties,
        Icon::GripVertical,
        Icon::GripHorizontal,
        Icon::DockMenu,
        Icon::DockTop,
        Icon::DockBottom,
        Icon::DockLeft,
        Icon::DockRight,
        Icon::Float,
        Icon::Sidebar,
        Icon::Thumbnails,
        Icon::Bookmarks,
        Icon::SwapSide,
        Icon::Close,
    ];

    #[test]
    fn every_icon_is_a_self_contained_svg() {
        for icon in ALL {
            let svg = std::str::from_utf8(icon.bytes()).unwrap();
            assert!(svg.contains("<svg"), "{icon:?}");
            assert!(
                svg.contains("currentColor"),
                "{icon:?} must follow the theme colour"
            );
            // No scripts or external references in embedded assets.
            assert!(!svg.contains("<script") && !svg.contains("href"), "{icon:?}");
        }
    }
}
