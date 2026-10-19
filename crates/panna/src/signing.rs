//! The Sign with a certificate card: a certificate file (.p12 or .pfx) and its password, an
//! optional reason and place, and whether to certify the document. The signed copy is saved
//! as a new file and opened.

use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::{button, checkbox, column, row, space, text, text_input};
use iced::{Element, Theme};
use panna_core::SignOptions;

use crate::Message;

/// Certificate files larger than this aren't read.
pub const MAX_FILE: u64 = 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct Signing {
    /// The certificate file's name and contents.
    pub file: Option<(String, Arc<Vec<u8>>)>,
    pub password: String,
    pub reason: String,
    pub location: String,
    pub certify: bool,
    /// The document has no signatures yet, so it can be certified.
    pub can_certify: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Choose,
    Chosen(Option<PathBuf>),
    Password(String),
    Reason(String),
    Location(String),
    Certify(bool),
    Go,
    Cancel,
}

pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("signing-password")
}

impl Signing {
    pub fn options(&self) -> SignOptions {
        SignOptions {
            reason: self.reason.clone(),
            location: self.location.clone(),
            certify: self.certify && self.can_certify,
        }
    }
}

pub fn card<'a>(state: &'a Signing) -> Element<'a, Message> {
    let ev = Message::Signing;
    let small = |s: &'a str| text(s).size(13);
    let file_name = state
        .file
        .as_ref()
        .map_or("No certificate file chosen", |(name, _)| name.as_str());
    let mut body = column![
        text("Sign with a certificate").size(16),
        small(
            "Signs with a digital ID in a certificate file (.p12 or .pfx), and saves the signed \
             copy as a new file. The signature can't be seen on the page; it's listed under \
             Signatures in the sidebar."
        ),
        row![
            text(file_name).size(13).width(iced::Fill),
            button("Choose…")
                .style(button::secondary)
                .on_press(ev(Event::Choose)),
        ]
        .spacing(8)
        .align_y(iced::Center),
        text_input("The certificate file's password", &state.password)
            .secure(true)
            .id(input_id())
            .on_input(move |v| ev(Event::Password(v)))
            .on_submit(ev(Event::Go)),
        text_input(
            "Reason (optional), such as I approve this document",
            &state.reason
        )
        .on_input(move |v| ev(Event::Reason(v)))
        .on_submit(ev(Event::Go)),
        text_input("Place (optional)", &state.location)
            .on_input(move |v| ev(Event::Location(v)))
            .on_submit(ev(Event::Go)),
    ]
    .spacing(10)
    .width(420);
    let certify = checkbox(state.certify && state.can_certify)
        .label("Certify: later changes other than filling in forms and signing will show as not allowed");
    body = body.push(if state.can_certify {
        certify.on_toggle(move |v| ev(Event::Certify(v)))
    } else {
        certify
    });
    if !state.can_certify {
        body = body.push(small("Only the first signature can certify a document.").style(text::secondary));
    }
    if let Some(error) = &state.error {
        body = body.push(small(error.as_str()).style(|theme: &Theme| text::Style {
            color: Some(theme.extended_palette().danger.base.color),
        }));
    }
    body = body.push(
        row![
            space::horizontal(),
            button("Cancel")
                .style(button::secondary)
                .on_press(ev(Event::Cancel)),
            button("Sign and save as…").on_press_maybe(state.file.is_some().then_some(ev(Event::Go))),
        ]
        .spacing(8),
    );
    iced::widget::container(body)
        .padding(16)
        .style(|theme: &Theme| iced::widget::container::rounded_box(theme))
        .into()
}
