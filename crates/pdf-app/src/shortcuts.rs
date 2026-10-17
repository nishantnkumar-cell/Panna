//! Keyboard shortcuts. `command()` is Ctrl on Windows and Linux, Cmd on macOS.

use iced::keyboard::{self, Key, key::Named};

use crate::Message;

pub fn map(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    if modifiers.command() {
        return match key.as_ref() {
            Key::Character("o") => Some(Message::OpenDialog),
            Key::Character("=" | "+") => Some(Message::ZoomIn),
            Key::Character("-") => Some(Message::ZoomOut),
            Key::Character("0") => Some(Message::ZoomReset),
            Key::Character("2") => Some(Message::FitWidth),
            _ => None,
        };
    }
    match key.as_ref() {
        Key::Named(Named::PageDown) => Some(Message::NextPage),
        Key::Named(Named::PageUp) => Some(Message::PrevPage),
        _ => None,
    }
}
