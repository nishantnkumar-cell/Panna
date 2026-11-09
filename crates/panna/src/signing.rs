//! The Sign with a certificate card: a certificate file (.p12 or .pfx) and its password, or on
//! Windows a certificate from the system's store, an
//! optional reason and place, whether to show the signature in a corner of the page, and
//! whether to certify the document. The signed copy is saved
//! as a new file and opened.

use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::{button, checkbox, column, pick_list, row, space, text, text_input};
use iced::{Element, Theme};
use panna_core::{Corner, SignOptions, Visible};

use crate::Message;
use crate::certstore::StoreId;

/// Certificate files larger than this aren't read.
pub const MAX_FILE: u64 = 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct Signing {
    /// The certificate file's name and contents.
    pub file: Option<(String, Arc<Vec<u8>>)>,
    pub password: zeroize::Zeroizing<String>,
    /// The system store's certificates, once read, and the one chosen instead of a file.
    pub store: Vec<StoreId>,
    pub stored: Option<StoreId>,
    pub reason: String,
    pub location: String,
    pub certify: bool,
    /// Show the signature in a corner of `page`, the page shown when the card opened.
    pub shown: bool,
    pub corner: Place,
    pub page: usize,
    /// The document has no signatures yet, so it can be certified.
    pub can_certify: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Choose,
    Chosen(Option<PathBuf>),
    Found(Result<Vec<StoreId>, String>),
    Stored(StoreId),
    Password(String),
    Reason(String),
    Location(String),
    Certify(bool),
    Shown(bool),
    Corner(Place),
    Go,
    Cancel,
}

/// A corner, as the list names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Place(pub Corner);

impl Place {
    const ALL: [Place; 4] = [
        Place(Corner::BottomRight),
        Place(Corner::BottomLeft),
        Place(Corner::TopRight),
        Place(Corner::TopLeft),
    ];
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            Corner::BottomRight => "bottom right",
            Corner::BottomLeft => "bottom left",
            Corner::TopRight => "top right",
            Corner::TopLeft => "top left",
        })
    }
}

pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("signing-password")
}

impl Signing {
    /// The digital ID chosen, if any.
    pub fn id(&self) -> Option<panna_core::DigitalId> {
        if let Some(stored) = &self.stored {
            return Some(panna_core::DigitalId::Held(stored.held()));
        }
        self.file.as_ref().map(|(_, bytes)| panna_core::DigitalId::File {
            pkcs12: Arc::clone(bytes),
            password: self.password.clone(),
        })
    }

    pub fn options(&self) -> SignOptions {
        SignOptions {
            reason: self.reason.clone(),
            location: self.location.clone(),
            certify: self.certify && self.can_certify,
            visible: self.shown.then_some(Visible {
                page: self.page,
                corner: self.corner.0,
            }),
        }
    }
}

pub fn card<'a>(state: &'a Signing) -> Element<'a, Message> {
    let ev = Message::Signing;
    let small = |s: &'a str| text(s).size(13);
    let file_name = match (&state.file, &state.stored) {
        (_, Some(_)) => "Signing with a certificate from Windows",
        (Some((name, _)), None) => name.as_str(),
        (None, None) => "No certificate file chosen",
    };
    let mut body = column![
        text("Sign with a certificate").size(16),
        small(
            "Signs with a digital ID in a certificate file (.p12 or .pfx), or one installed in \
             Windows, and saves the signed copy as a new file. The signature is listed under Signatures in the sidebar, and \
             can be shown on the page too."
        ),
        row![
            text(file_name).size(13).width(iced::Fill),
            button("Choose…")
                .style(button::secondary)
                .on_press(ev(Event::Choose)),
        ]
        .spacing(8)
        .align_y(iced::Center),
    ]
    .spacing(10)
    .width(420);
    if !state.store.is_empty() {
        body = body.push(
            pick_list(state.store.as_slice(), state.stored.as_ref(), move |v| {
                ev(Event::Stored(v))
            })
            .placeholder("Or a certificate installed in Windows")
            .text_size(13)
            .width(iced::Fill),
        );
    }
    if state.stored.is_none() {
        body = body.push(
            text_input("The certificate file's password", &state.password)
                .secure(true)
                .id(input_id())
                .on_input(move |v| ev(Event::Password(v)))
                .on_submit(ev(Event::Go)),
        );
    }
    body = body.extend([
        text_input(
            "Reason (optional), such as I approve this document",
            &state.reason,
        )
        .on_input(move |v| ev(Event::Reason(v)))
        .on_submit(ev(Event::Go))
        .into(),
        text_input("Place (optional)", &state.location)
            .on_input(move |v| ev(Event::Location(v)))
            .on_submit(ev(Event::Go))
            .into(),
        row![
            checkbox(state.shown)
                .label(format!("Show on page {}, at the", state.page + 1))
                .on_toggle(move |v| ev(Event::Shown(v))),
            pick_list(Place::ALL, Some(state.corner), move |v| ev(Event::Corner(v)))
                .text_size(13)
                .padding([2, 6]),
        ]
        .spacing(6)
        .align_y(iced::Center)
        .into(),
    ]);
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
            button("Sign and save as…")
                .on_press_maybe((state.file.is_some() || state.stored.is_some()).then_some(ev(Event::Go)),),
        ]
        .spacing(8),
    );
    iced::widget::container(body)
        .padding(16)
        .style(|theme: &Theme| iced::widget::container::rounded_box(theme))
        .into()
}
