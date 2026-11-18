//! The About box: version, licences, where the documentation is, and Check for updates.
//!
//! Checking for updates is the only thing in Panna that uses the network, and it happens
//! only when the user clicks the button (see the `panna-update` crate).

use crate::i18n::text;
use iced::widget::{Column, button, column, container, row, scrollable, space};
use iced::{Center, Element, Fill, Task};

use crate::Message;
use crate::icons::{self, Icon};

/// The project's documentation (the GitHub wiki).
pub const WIKI: &str = "https://github.com/nishantnkumar-cell/Panna/wiki";
/// The project's home.
pub const HOME: &str = "https://github.com/nishantnkumar-cell/Panna";

const VERSION: &str = env!("CARGO_PKG_VERSION");
const LICENSE_MIT: &str = include_str!("../../../LICENSE-MIT");
const LICENSE_APACHE: &str = include_str!("../../../LICENSE-APACHE");
const THIRD_PARTY: &str = include_str!("../third-party.tsv");

/// The About box's state while it is open.
#[derive(Debug, Default)]
pub struct About {
    update: Update,
    licences: bool,
    /// The last link that couldn't be opened.
    failed: Option<String>,
}

#[derive(Debug, Default)]
enum Update {
    #[default]
    NotAsked,
    Checking,
    Done(Result<panna_update::Outcome, panna_update::Error>),
}

#[derive(Debug, Clone)]
pub enum Event {
    Close,
    Check,
    Checked(Result<panna_update::Outcome, panna_update::Error>),
    Licences(bool),
    Open(String),
}

/// Whether Panna runs as a Flatpak, which has no network access by design.
fn in_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists()
}

/// Handles an About box event. `state` is `None` while the box is closed.
pub fn update(state: &mut Option<About>, event: Event) -> Task<Message> {
    let Some(about) = state.as_mut() else {
        return Task::none();
    };
    match event {
        Event::Close => *state = None,
        Event::Licences(show) => about.licences = show,
        Event::Open(uri) => {
            about.failed = crate::links::open(&uri).is_err().then_some(uri);
        }
        Event::Check => {
            if matches!(about.update, Update::Checking) {
                return Task::none();
            }
            about.update = Update::Checking;
            let (tx, rx) = futures_channel::oneshot::channel();
            std::thread::spawn(move || {
                let _ = tx.send(panna_update::check(VERSION));
            });
            return Task::perform(rx, |result| {
                Message::AboutBox(Event::Checked(result.unwrap_or_else(|_| {
                    Err(panna_update::Error::Offline(
                        "the check stopped unexpectedly".into(),
                    ))
                })))
            });
        }
        Event::Checked(result) => about.update = Update::Done(result),
    }
    Task::none()
}

fn link_button<'a>(icon: Icon, label: &'a str, uri: &str) -> Element<'a, Message> {
    button(
        row![icons::glyph::<Message>(icon, true), text(label).size(13)]
            .spacing(6)
            .align_y(Center),
    )
    .style(button::secondary)
    .on_press(Message::AboutBox(Event::Open(uri.to_owned())))
    .into()
}

/// The help icon at the right end of the menu bar, sized to fit it.
pub fn bar_button<'a>() -> Element<'a, Message> {
    icons::with_tip(
        button(icons::glyph::<Message>(Icon::About, true))
            .padding(2)
            .style(|theme: &iced::Theme, status| {
                let palette = theme.extended_palette();
                let background = match status {
                    button::Status::Hovered => Some(palette.background.strong.color.into()),
                    button::Status::Pressed => Some(palette.primary.weak.color.into()),
                    _ => None,
                };
                button::Style {
                    background,
                    text_color: palette.background.base.text,
                    border: iced::Border {
                        radius: 4.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            })
            .on_press(Message::About),
        "About Panna, licences and updates",
    )
}

/// The About card.
pub fn card(about: &About) -> Element<'_, Message> {
    let ev = Message::AboutBox;
    let mut body = column![
        row![
            text("Panna").size(22),
            text(format!("Version {VERSION}")).size(13).style(text::secondary),
        ]
        .spacing(10)
        .align_y(iced::Alignment::End),
        text("An offline PDF viewer and editor, written in Rust. Nothing you open leaves this computer.")
            .size(13),
        row![
            link_button(Icon::ReadMode, "Documentation", WIKI),
            link_button(Icon::AddLink, "Project page", HOME),
            button(
                text(if about.licences {
                    "Hide licences"
                } else {
                    "Licences"
                })
                .size(13)
            )
            .style(button::secondary)
            .on_press(ev(Event::Licences(!about.licences))),
        ]
        .spacing(8),
    ]
    .spacing(12)
    .width(520);

    body = body.push(updates(about));

    if about.licences {
        body = body.push(licences());
    }
    if let Some(uri) = &about.failed {
        body = body.push(
            text(format!("Couldn't open a browser. The address is {uri}"))
                .size(12)
                .style(text::danger),
        );
    }
    body = body.push(row![
        space::horizontal(),
        button(text("Close")).on_press(ev(Event::Close))
    ]);
    container(body).padding(16).style(container::rounded_box).into()
}

/// The Updates section: the button, what it found, and what it sends.
fn updates(about: &About) -> Element<'_, Message> {
    let ev = Message::AboutBox;
    let releases = panna_update::RELEASES_PAGE;
    if in_flatpak() {
        return column![
            text("Updates").size(15),
            text(
                "This Flatpak has no network access, so Panna can't check by itself. The releases \
                 page lists every version."
            )
            .size(12)
            .style(text::secondary),
            link_button(Icon::Download, "Open the releases page", releases),
        ]
        .spacing(6)
        .into();
    }
    let checking = matches!(about.update, Update::Checking);
    let mut section = column![
        text("Updates").size(15),
        row![
            button(
                row![
                    icons::glyph::<Message>(Icon::Refresh, !checking),
                    text(if checking {
                        "Checking…"
                    } else {
                        "Check for updates"
                    })
                    .size(13)
                ]
                .spacing(6)
                .align_y(Center),
            )
            .on_press_maybe((!checking).then_some(ev(Event::Check))),
        ],
    ]
    .spacing(6);
    match &about.update {
        Update::NotAsked | Update::Checking => {}
        Update::Done(Ok(panna_update::Outcome::UpToDate)) => {
            section = section.push(text(format!("Panna {VERSION} is the newest version.")).size(13));
        }
        Update::Done(Ok(panna_update::Outcome::Newer(release))) => {
            let kind = if release.prerelease {
                "Pre-release"
            } else {
                "Release"
            };
            section = section.push(
                row![
                    text(format!("{kind} {} is available.", release.version)).size(13),
                    link_button(Icon::Download, "Download", &release.page),
                ]
                .spacing(10)
                .align_y(Center),
            );
        }
        Update::Done(Err(e)) => {
            section = section.push(text(e.to_string()).size(13).style(text::danger));
            section = section.push(link_button(Icon::Download, "Open the releases page", releases));
        }
    }
    section
        .push(
            text(
                "Checking asks github.com for the list of Panna releases. Nothing about you or your \
                 files is sent, and Panna never checks on its own.",
            )
            .size(11)
            .style(text::secondary),
        )
        .into()
}

/// Panna's licence and every crate it is built from, with theirs.
fn licences<'a>() -> Element<'a, Message> {
    // Licence texts and crate names stay untranslated, so iced's own text is used here.
    use iced::widget::text as plain;
    let crates: Vec<(&str, &str, &str)> = THIRD_PARTY
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| {
            let mut parts = l.splitn(3, '\t');
            Some((parts.next()?, parts.next()?, parts.next()?))
        })
        .collect();
    let mut list = Column::new()
        .spacing(2)
        .push(
            text(
                "Panna is free software: you may use it under the MIT licence or the Apache \
                 License 2.0, whichever you prefer. Both texts are below the list.",
            )
            .size(12),
        )
        .push(space().height(6))
        .push(
            text(format!(
                "Panna is built from {} open-source crates:",
                crates.len()
            ))
            .size(12),
        );
    for (name, version, licence) in crates {
        list = list.push(
            row![
                plain(format!("{name} {version}")).size(11).width(Fill),
                plain(licence).size(11).style(text::secondary),
            ]
            .spacing(8),
        );
    }
    for licence in [LICENSE_MIT, LICENSE_APACHE] {
        list = list.push(space().height(10));
        for paragraph in paragraphs(licence) {
            list = list.push(plain(paragraph).size(11));
        }
    }
    column![
        text("Licences").size(15),
        scrollable(list.spacing(4).padding([0, 12])).height(260),
    ]
    .spacing(6)
    .into()
}

/// A plain-text licence as paragraphs, its hard line breaks joined so it wraps to the box.
fn paragraphs(licence: &str) -> Vec<String> {
    // A Windows checkout may have turned the line breaks into CRLF.
    licence
        .replace("\r\n", "\n")
        .split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crate_list_is_built_in_and_readable() {
        let rows: Vec<&str> = THIRD_PARTY.lines().filter(|l| !l.starts_with('#')).collect();
        assert!(rows.len() > 100);
        for row in rows {
            assert_eq!(row.split('\t').count(), 3, "{row:?}");
        }
        assert!(THIRD_PARTY.contains("\nureq\t"));
        assert!(LICENSE_MIT.starts_with("MIT License"));
        assert!(LICENSE_APACHE.contains("Version 2.0"));
        let mit = paragraphs(&LICENSE_MIT.replace('\n', "\r\n"));
        assert_eq!(mit.first().map(String::as_str), Some("MIT License"));
        assert!(mit.iter().all(|p| !p.contains('\n')));
    }

    #[test]
    fn links_are_web_addresses() {
        for uri in [WIKI, HOME, panna_update::RELEASES_PAGE] {
            assert!(panna_core::safe_uri(uri), "{uri}");
        }
    }
}
