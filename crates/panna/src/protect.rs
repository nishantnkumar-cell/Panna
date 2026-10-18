//! The Password protection card: a password to open the file, and limits on what others may
//! do with it.

use iced::widget::{button, checkbox, column, container, row, space, text, text_input};
use iced::{Element, Theme};
use panna_core::Protection;

use crate::Message;

/// The card, as filled in so far.
#[derive(Debug, Clone, Default)]
pub struct Protecting {
    pub open: String,
    pub open_again: String,
    /// Restrict printing, changes or copying.
    pub restrict: bool,
    pub owner: String,
    pub owner_again: String,
    pub print: bool,
    pub change: bool,
    pub copy: bool,
    /// The file has protection now, which can be removed.
    pub protected: bool,
}

impl Protecting {
    pub fn new(protected: bool) -> Self {
        Self {
            print: true,
            protected,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    OpenPassword(String),
    OpenAgain(String),
    Restrict(bool),
    Owner(String),
    OwnerAgain(String),
    Print(bool),
    Change(bool),
    Copy(bool),
    Go,
    Remove,
    Cancel,
}

pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("protect-open")
}

/// The protection to give the file, or what's missing.
pub fn settings(state: &Protecting) -> Result<Protection, String> {
    if state.open.is_empty() && !state.restrict {
        return Err("Enter a password, or choose what to restrict.".into());
    }
    if state.open != state.open_again {
        return Err("The two passwords to open it don't match.".into());
    }
    if state.restrict && state.owner != state.owner_again {
        return Err("The two permissions passwords don't match.".into());
    }
    if state.restrict && !state.owner.is_empty() && state.owner == state.open {
        return Err(
            "Use a different password for permissions, or anyone who can open it could lift them.".into(),
        );
    }
    let restrict = state.restrict;
    Ok(Protection {
        open_password: state.open.clone(),
        owner_password: if restrict {
            state.owner.clone()
        } else {
            String::new()
        },
        allow_printing: !restrict || state.print,
        allow_changes: !restrict || state.change,
        allow_copying: !restrict || state.copy,
    })
}

/// The Password protection card.
pub fn card<'a>(state: &Protecting) -> Element<'a, Message> {
    let ev = Message::Protect;
    let parsed = settings(state);
    let typed = !state.open.is_empty() || state.restrict;
    let small = |s: String| text(s).size(13);
    let hint: Element<'a, Message> = match &parsed {
        Err(e) if typed => small(e.clone())
            .style(|theme: &Theme| text::Style {
                color: Some(theme.extended_palette().danger.base.color),
            })
            .into(),
        Ok(_) if state.restrict && state.owner.is_empty() => small(
            "Without a permissions password, nobody can lift these restrictions later, you included.".into(),
        )
        .into(),
        _ => space().height(0).into(),
    };
    let password = |placeholder: &'static str, value: &str, on: fn(String) -> Event| {
        text_input(placeholder, value)
            .secure(true)
            .on_input(move |v| ev(on(v)))
            .on_submit(ev(Event::Go))
    };
    let mut body = column![
        text("Password protection").size(16),
        small("Takes effect when you save. Keep the passwords safe: a lost one can't be recovered.".into()),
        password("Password to open (optional)", &state.open, Event::OpenPassword).id(input_id()),
        password("Type it again", &state.open_again, Event::OpenAgain),
        checkbox(state.restrict)
            .label("Restrict printing, changes or copying")
            .on_toggle(move |v| ev(Event::Restrict(v))),
    ]
    .spacing(10)
    .width(400);
    if state.restrict {
        body = body
            .push(
                column![
                    checkbox(state.print)
                        .label("Allow printing")
                        .on_toggle(move |v| ev(Event::Print(v))),
                    checkbox(state.change)
                        .label("Allow changes, comments and filling in forms")
                        .on_toggle(move |v| ev(Event::Change(v))),
                    checkbox(state.copy)
                        .label("Allow copying text and pictures")
                        .on_toggle(move |v| ev(Event::Copy(v))),
                ]
                .spacing(8)
                .padding([0, 24]),
            )
            .push(password(
                "Permissions password, to lift the restrictions",
                &state.owner,
                Event::Owner,
            ))
            .push(password("Type it again", &state.owner_again, Event::OwnerAgain));
    }
    let remove: Element<'a, Message> = if state.protected {
        button("Remove protection")
            .style(button::secondary)
            .on_press(ev(Event::Remove))
            .into()
    } else {
        space().width(0).into()
    };
    body = body.push(hint).push(
        row![
            remove,
            space::horizontal(),
            button("Cancel")
                .style(button::secondary)
                .on_press(ev(Event::Cancel)),
            button("Protect").on_press_maybe(parsed.is_ok().then_some(ev(Event::Go))),
        ]
        .spacing(8),
    );
    container(body).padding(16).style(container::rounded_box).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_checked_before_protecting() {
        let mut state = Protecting::new(false);
        assert!(settings(&state).is_err());
        state.open = "a".into();
        assert!(settings(&state).is_err());
        state.open_again = "a".into();
        let p = settings(&state).unwrap();
        assert!(p.allow_printing && p.allow_changes && p.allow_copying && p.owner_password.is_empty());
        state.restrict = true;
        state.owner = "a".into();
        state.owner_again = "a".into();
        assert!(settings(&state).is_err());
        state.owner = "b".into();
        state.owner_again = "b".into();
        let p = settings(&state).unwrap();
        assert!(p.allow_printing && !p.allow_changes && !p.allow_copying);
        assert_eq!(p.owner_password, "b");
    }
}
