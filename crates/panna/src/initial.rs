//! The "When this file opens" part of the document properties: the page, zoom, panel and
//! page layout the file asks for, and full screen. Applying it is a page change, saved
//! into the file.

use std::fmt;

use iced::widget::{button, checkbox, column, pick_list, row, text, text_input};
use iced::{Center, Element};
use panna_core::{InitialView, OpenLayout, OpenPanel, OpenZoom};

use crate::Message;

#[derive(Debug, Clone)]
pub enum Event {
    Page(String),
    Zoom(ZoomChoice),
    Panel(PanelChoice),
    Layout(LayoutChoice),
    FullScreen(bool),
    /// Fills it in from how the document is shown now.
    UseCurrent,
    Apply,
}

/// What the app does after an event.
pub enum Outcome {
    None,
    /// Fill in from the current view.
    UseCurrent,
    Apply(InitialView),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomChoice(pub OpenZoom);

impl ZoomChoice {
    const ALL: [ZoomChoice; 9] = [
        ZoomChoice(OpenZoom::Default),
        ZoomChoice(OpenZoom::FitPage),
        ZoomChoice(OpenZoom::FitWidth),
        ZoomChoice(OpenZoom::Scale(0.5)),
        ZoomChoice(OpenZoom::Scale(0.75)),
        ZoomChoice(OpenZoom::Scale(1.0)),
        ZoomChoice(OpenZoom::Scale(1.25)),
        ZoomChoice(OpenZoom::Scale(1.5)),
        ZoomChoice(OpenZoom::Scale(2.0)),
    ];
}

impl fmt::Display for ZoomChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            OpenZoom::Default => write!(f, "As the viewer chooses"),
            OpenZoom::FitPage => write!(f, "Fit page"),
            OpenZoom::FitWidth => write!(f, "Fit width"),
            OpenZoom::Scale(zoom) => write!(f, "{:.0}%", zoom * 100.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelChoice(pub OpenPanel);

impl PanelChoice {
    const ALL: [PanelChoice; 5] = [
        PanelChoice(OpenPanel::None),
        PanelChoice(OpenPanel::Bookmarks),
        PanelChoice(OpenPanel::Pages),
        PanelChoice(OpenPanel::Attachments),
        PanelChoice(OpenPanel::Layers),
    ];
}

impl fmt::Display for PanelChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            OpenPanel::None => "Pages only",
            OpenPanel::Bookmarks => "Bookmarks",
            OpenPanel::Pages => "Page thumbnails",
            OpenPanel::Attachments => "Attached files",
            OpenPanel::Layers => "Layers",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutChoice(pub OpenLayout);

impl LayoutChoice {
    const ALL: [LayoutChoice; 4] = [
        LayoutChoice(OpenLayout::Default),
        LayoutChoice(OpenLayout::Single),
        LayoutChoice(OpenLayout::Two),
        LayoutChoice(OpenLayout::TwoCover),
    ];
}

impl fmt::Display for LayoutChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            OpenLayout::Default => "As the viewer chooses",
            OpenLayout::Single => "Single page",
            OpenLayout::Two => "Two pages",
            OpenLayout::TwoCover => "Two pages, cover alone",
        })
    }
}

/// The choices being made, for a document with `pages` pages.
#[derive(Debug, Clone, Default)]
pub struct Editor {
    view: InitialView,
    /// The page number as typed, from 1.
    page: String,
    pages: usize,
}

impl Editor {
    /// Starts from what the document asks for now.
    pub fn load(&mut self, view: InitialView, pages: usize) {
        self.view = view;
        self.pages = pages;
        self.page = (view.page + 1).to_string();
    }

    /// The page typed, from 0, if it's one of the document's.
    fn page(&self) -> Option<usize> {
        let n: usize = self.page.trim().parse().ok()?;
        (1..=self.pages).contains(&n).then(|| n - 1)
    }

    pub fn update(&mut self, event: Event) -> Outcome {
        match event {
            Event::Page(page) => self.page = page,
            Event::Zoom(choice) => self.view.zoom = choice.0,
            Event::Panel(choice) => self.view.panel = choice.0,
            Event::Layout(choice) => self.view.layout = choice.0,
            Event::FullScreen(on) => self.view.full_screen = on,
            Event::UseCurrent => return Outcome::UseCurrent,
            Event::Apply => {
                let Some(page) = self.page() else {
                    return Outcome::None;
                };
                return Outcome::Apply(InitialView { page, ..self.view });
            }
        }
        Outcome::None
    }

    /// Whether the choices differ from what the file asks for.
    fn changed(&self, saved: &InitialView) -> bool {
        self.page()
            .is_some_and(|page| InitialView { page, ..self.view } != *saved)
    }

    pub fn view<'a>(&'a self, saved: &InitialView, busy: bool) -> Element<'a, Message> {
        let ev = |e: Event| Message::InitialView(e);
        let label = |s: &'static str| text(s).width(110).style(text::secondary);
        let page_ok = self.page().is_some();
        let page = text_input("1", &self.page)
            .on_input(move |s| ev(Event::Page(s)))
            .width(70)
            .size(13)
            .style(move |theme, status| {
                let mut style = text_input::default(theme, status);
                if !page_ok {
                    style.border.color = theme.extended_palette().danger.base.color;
                }
                style
            });
        column![
            text("When this file opens").size(15),
            row![label("Page"), page, text(format!("of {}", self.pages)).size(13)]
                .spacing(12)
                .align_y(Center),
            row![
                label("Zoom"),
                pick_list(ZoomChoice::ALL, Some(ZoomChoice(self.view.zoom)), move |c| ev(
                    Event::Zoom(c)
                ))
                .text_size(13)
            ]
            .spacing(12)
            .align_y(Center),
            row![
                label("Show"),
                pick_list(PanelChoice::ALL, Some(PanelChoice(self.view.panel)), move |c| ev(
                    Event::Panel(c)
                ))
                .text_size(13)
            ]
            .spacing(12)
            .align_y(Center),
            row![
                label("Page layout"),
                pick_list(
                    LayoutChoice::ALL,
                    Some(LayoutChoice(self.view.layout)),
                    move |c| ev(Event::Layout(c))
                )
                .text_size(13)
            ]
            .spacing(12)
            .align_y(Center),
            row![
                label(""),
                checkbox(self.view.full_screen)
                    .label("Full screen")
                    .size(16)
                    .text_size(13)
                    .on_toggle(move |on| ev(Event::FullScreen(on)))
            ]
            .spacing(12),
            row![
                button(text("Use the current view").size(13))
                    .style(button::secondary)
                    .on_press_maybe((!busy).then_some(ev(Event::UseCurrent))),
                button(text("Apply").size(13))
                    .on_press_maybe((!busy && self.changed(saved)).then_some(ev(Event::Apply))),
            ]
            .spacing(8),
        ]
        .spacing(8)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_typed_page_must_be_in_the_document() {
        let mut editor = Editor::default();
        editor.load(InitialView::default(), 3);
        assert!(!editor.changed(&InitialView::default()));
        editor.update(Event::Page("4".into()));
        assert!(matches!(editor.update(Event::Apply), Outcome::None));
        editor.update(Event::Page("3".into()));
        editor.update(Event::Zoom(ZoomChoice(OpenZoom::FitWidth)));
        match editor.update(Event::Apply) {
            Outcome::Apply(view) => {
                assert_eq!(view.page, 2);
                assert_eq!(view.zoom, OpenZoom::FitWidth);
            }
            _ => panic!("not applied"),
        }
    }
}
