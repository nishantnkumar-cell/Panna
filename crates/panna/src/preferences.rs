//! The Preferences card: the name put on comments, light or dark, how documents open, the
//! unit for measuring, and making Panna the default PDF app. Each change applies and is saved
//! at once.

use std::fmt;

use iced::widget::{button, column, container, pick_list, row, space, text, text_input};
use iced::{Center, Element};

use crate::Message;
use crate::measure::Unit;

/// Light or dark, or as the system is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    pub fn as_str(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == s)
    }

    /// The theme to use; `None` follows the system.
    pub fn theme(self) -> Option<iced::Theme> {
        match self {
            Appearance::System => None,
            Appearance::Light => Some(iced::Theme::Light),
            Appearance::Dark => Some(iced::Theme::Dark),
        }
    }
}

impl fmt::Display for Appearance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Appearance::System => "Match the system",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        })
    }
}

/// How a document opens when the file doesn't say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenZoom {
    #[default]
    ActualSize,
    FitWidth,
    FitPage,
}

impl OpenZoom {
    pub const ALL: [OpenZoom; 3] = [OpenZoom::ActualSize, OpenZoom::FitWidth, OpenZoom::FitPage];

    pub fn as_str(self) -> &'static str {
        match self {
            OpenZoom::ActualSize => "actual",
            OpenZoom::FitWidth => "width",
            OpenZoom::FitPage => "page",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|z| z.as_str() == s)
    }

    /// As a file's own opening zoom.
    pub fn zoom(self) -> panna_core::OpenZoom {
        match self {
            OpenZoom::ActualSize => panna_core::OpenZoom::Default,
            OpenZoom::FitWidth => panna_core::OpenZoom::FitWidth,
            OpenZoom::FitPage => panna_core::OpenZoom::FitPage,
        }
    }
}

impl fmt::Display for OpenZoom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OpenZoom::ActualSize => "Actual size",
            OpenZoom::FitWidth => "Fit width",
            OpenZoom::FitPage => "Fit page",
        })
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Author(String),
    Appearance(Appearance),
    Zoom(OpenZoom),
    Unit(Unit),
    /// Asks the system to make Panna the default PDF app, or says how.
    DefaultApp,
    Close,
}

/// What the card shows: the current preferences, and a note after asking for the default app.
pub struct View<'a> {
    pub author: &'a str,
    /// The account's name, used when no name is set.
    pub account: &'a str,
    pub appearance: Appearance,
    pub zoom: OpenZoom,
    pub unit: Unit,
    pub note: Option<&'a str>,
}

/// The account's name, for comments when no name is set.
pub fn account_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

/// The Preferences card.
pub fn card(view: View<'_>) -> Element<'_, Message> {
    let ev = Message::Preferences;
    let label = |s: &'static str| text(s).width(190);
    let mut body = column![
        text("Preferences").size(16),
        row![
            label("Your name on comments"),
            text_input(view.account, view.author)
                .on_input(move |s| ev(Event::Author(s)))
                .width(200),
        ]
        .spacing(8)
        .align_y(Center),
        row![
            label("Appearance"),
            pick_list(Appearance::ALL, Some(view.appearance), move |a| ev(
                Event::Appearance(a)
            )),
        ]
        .spacing(8)
        .align_y(Center),
        row![
            label("Open documents at"),
            pick_list(OpenZoom::ALL, Some(view.zoom), move |z| ev(Event::Zoom(z))),
        ]
        .spacing(8)
        .align_y(Center),
        text("Files that say how they open still do.")
            .size(12)
            .style(text::secondary),
        row![
            label("Measure in"),
            pick_list(Unit::ALL, Some(view.unit), move |u| ev(Event::Unit(u))),
        ]
        .spacing(8)
        .align_y(Center),
        row![
            label("Default PDF app"),
            button("Make Panna the default…")
                .style(button::secondary)
                .on_press(ev(Event::DefaultApp)),
        ]
        .spacing(8)
        .align_y(Center),
    ]
    .spacing(10)
    .width(460);
    if let Some(note) = view.note {
        body = body.push(text(note).size(12).style(text::secondary));
    }
    body = body.push(row![
        space::horizontal(),
        button("Done").on_press(ev(Event::Close))
    ]);
    container(body).padding(16).style(container::rounded_box).into()
}

/// Makes Panna the default PDF app where the system lets an app ask, or opens the setting
/// where the user chooses. Returns what to tell the user.
pub fn make_default() -> String {
    #[cfg(windows)]
    {
        // Windows lets only the user choose; this opens Default apps at Panna's entry, which
        // the installer registers.
        match crate::links::open_settings("ms-settings:defaultapps?registeredAppUser=Panna") {
            Ok(()) => "In Settings, choose Panna for .pdf files.".into(),
            Err(_) => "Open Settings → Apps → Default apps, find Panna and choose it for .pdf files.".into(),
        }
    }
    #[cfg(target_os = "macos")]
    {
        "In Finder, select a PDF, choose File → Get Info, pick Panna under \"Open with\" and \
         click Change All."
            .into()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let desktop = format!("{}.desktop", crate::APP_ID);
        let done = std::process::Command::new("xdg-mime")
            .args(["default", &desktop, "application/pdf"])
            .status()
            .is_ok_and(|s| s.success());
        if done {
            "Panna now opens PDF files.".into()
        } else {
            "Choose Panna for PDF files in your desktop's settings (Default Applications in KDE, \
             Default Apps in GNOME)."
                .into()
        }
    }
}
