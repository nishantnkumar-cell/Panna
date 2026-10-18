//! Keyboard shortcuts, following Acrobat's where one exists. `command()` is Ctrl on Windows
//! and Linux, Cmd on macOS.

use iced::keyboard::{self, Key, key::Named};

use crate::Message;
use crate::annotate;
use crate::find::Event;

pub fn map(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    if modifiers.shift() && !modifiers.command() && key == Key::Named(Named::F3) {
        return Some(Message::Find(Event::Prev));
    }
    if modifiers.command() && modifiers.shift() {
        return match key.as_ref() {
            Key::Character("g" | "G") => Some(Message::Find(Event::Prev)),
            Key::Character("z" | "Z") => Some(Message::Annotate(annotate::Event::Redo)),
            Key::Character("s" | "S") => Some(Message::SaveAs),
            Key::Character("=" | "+") => Some(Message::RotateCw),
            Key::Character("-" | "_") => Some(Message::RotateCcw),
            _ => None,
        };
    }
    if modifiers.command() {
        return match key.as_ref() {
            Key::Character("o") => Some(Message::OpenDialog),
            Key::Character("d") => Some(Message::ToggleProperties),
            Key::Character("=" | "+") => Some(Message::ZoomIn),
            Key::Character("-") => Some(Message::ZoomOut),
            Key::Character("0") => Some(Message::FitPage),
            Key::Character("1") => Some(Message::ZoomReset),
            Key::Character("2") => Some(Message::FitWidth),
            Key::Character("f") => Some(Message::Find(Event::Open)),
            Key::Character("g") => Some(Message::Find(Event::Next)),
            Key::Character("c") => Some(Message::Copy),
            Key::Character("s") => Some(Message::Save),
            Key::Character("z") => Some(Message::Annotate(annotate::Event::Undo)),
            Key::Character("y") => Some(Message::Annotate(annotate::Event::Redo)),
            _ => None,
        };
    }
    match key.as_ref() {
        Key::Named(Named::PageDown) => Some(Message::NextPage),
        Key::Named(Named::PageUp) => Some(Message::PrevPage),
        Key::Named(Named::Escape) => Some(Message::CloseOverlays),
        // F4 shows or hides the navigation pane, as in Acrobat.
        Key::Named(Named::F3) => Some(Message::Find(Event::Next)),
        Key::Named(Named::F4) => Some(Message::Sidebar(crate::sidebar::Event::Toggle)),
        Key::Named(Named::Delete | Named::Backspace) => {
            Some(Message::Annotate(annotate::Event::DeletePicked))
        }
        _ => None,
    }
}
