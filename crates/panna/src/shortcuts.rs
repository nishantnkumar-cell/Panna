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
    if key == Key::Named(Named::Tab) && !modifiers.command() && !modifiers.alt() {
        return Some(if modifiers.shift() {
            Message::FocusPrevious
        } else {
            Message::FocusNext
        });
    }
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
            Key::Character("n" | "N") => Some(Message::FocusPageBox),
            Key::Character("c" | "C") => Some(Message::Annotate(annotate::Event::Toggle)),
            Key::Character("t" | "T") => Some(Message::NextSpread),
            Key::Character("d" | "D") => Some(Message::NextColors),
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
        Key::Named(Named::Home) => Some(Message::GoToPage(0)),
        Key::Named(Named::End) => Some(Message::GoToPage(usize::MAX)),
        Key::Named(Named::Escape) => Some(Message::CloseOverlays),
        Key::Named(Named::Enter) => Some(Message::Annotate(annotate::Event::FinishShape)),
        // F4 shows or hides the navigation pane, as in Acrobat.
        Key::Named(Named::F3) => Some(Message::Find(Event::Next)),
        Key::Named(Named::F4) => Some(Message::Sidebar(crate::sidebar::Event::Toggle)),
        Key::Named(Named::Delete | Named::Backspace) => {
            Some(Message::Annotate(annotate::Event::DeletePicked))
        }
        // Single letters pick comment tools, as Acrobat's single-key accelerators do.
        Key::Character(c) if !modifiers.alt() && !modifiers.shift() => {
            annotate::Tool::from_key(c).map(Message::ToolKey)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::{Location, Modifiers, key};

    fn press(key: Key, modifiers: Modifiers) -> Option<Message> {
        map(keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    fn letter(c: &str, modifiers: Modifiers) -> Option<Message> {
        press(Key::Character(c.into()), modifiers)
    }

    #[test]
    fn letters_pick_tools_only_without_modifiers() {
        assert!(matches!(
            letter("r", Modifiers::empty()),
            Some(Message::ToolKey(annotate::Tool::Rect))
        ));
        assert!(matches!(
            letter("c", Modifiers::empty()),
            Some(Message::ToolKey(annotate::Tool::Cloud))
        ));
        assert!(letter("r", Modifiers::ALT).is_none());
        assert!(letter("q", Modifiers::empty()).is_none());
        assert!(matches!(
            letter("C", Modifiers::COMMAND | Modifiers::SHIFT),
            Some(Message::Annotate(annotate::Event::Toggle))
        ));
    }

    #[test]
    fn home_and_end_go_to_the_ends() {
        assert!(matches!(
            press(Key::Named(Named::Home), Modifiers::empty()),
            Some(Message::GoToPage(0))
        ));
        assert!(matches!(
            press(Key::Named(Named::End), Modifiers::empty()),
            Some(Message::GoToPage(usize::MAX))
        ));
    }
}
