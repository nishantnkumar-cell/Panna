//! Keyboard shortcuts, following Acrobat's where one exists. `command()` is Ctrl on Windows
//! and Linux, Cmd on macOS.

use iced::keyboard::{self, Key, key::Named};

use crate::Message;
use crate::annotate;
use crate::find::Event;
use crate::menu::{self, Menu, Nav};

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
    // Alt and a menu's underlined letter opens it, and F10 opens the first, as on Windows and
    // Linux. macOS keeps Option for typing accented letters.
    if !cfg!(target_os = "macos") && !modifiers.command() {
        if modifiers.alt()
            && let Key::Character(c) = key.as_ref()
        {
            return Menu::from_key(c).map(|m| Message::Menu(menu::Event::OpenKey(m)));
        }
        if key == Key::Named(Named::F10) && !modifiers.shift() {
            return Some(Message::Menu(menu::Event::OpenKey(Menu::File)));
        }
    }
    if modifiers.shift() && !modifiers.command() && key == Key::Named(Named::F3) {
        return Some(Message::Find(Event::Prev));
    }
    if modifiers.command() && key == Key::Named(Named::Tab) {
        return Some(Message::NextTab(!modifiers.shift()));
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
            // Acrobat's Read Out Loud keys.
            Key::Character("v" | "V") => Some(Message::ReadAloud(crate::speech::Event::Page)),
            Key::Character("b" | "B") => Some(Message::ReadAloud(crate::speech::Event::ToEnd)),
            Key::Character("e" | "E") => Some(Message::ReadAloud(crate::speech::Event::Stop)),
            _ => None,
        };
    }
    if modifiers.command() {
        return match key.as_ref() {
            Key::Character("o") => Some(Message::OpenDialog),
            Key::Character("w") => Some(Message::CloseTab),
            Key::Character("d") => Some(Message::ToggleProperties),
            Key::Character("l") => Some(Message::Present),
            Key::Character("=" | "+") => Some(Message::ZoomIn),
            Key::Character("-") => Some(Message::ZoomOut),
            Key::Character("0") => Some(Message::FitPage),
            Key::Character("1") => Some(Message::ZoomReset),
            Key::Character("2") => Some(Message::FitWidth),
            Key::Character("f") => Some(Message::Find(Event::Open)),
            Key::Character("g") => Some(Message::Find(Event::Next)),
            Key::Character("c") => Some(Message::Copy),
            Key::Character("s") => Some(Message::Save),
            Key::Character("p") => Some(Message::Print),
            Key::Character(",") => Some(Message::Preferences(crate::preferences::Event::Open)),
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
        Key::Named(Named::F11) => Some(Message::ToggleFullScreen),
        // Arrows and Space step through pages while presenting, and do nothing here otherwise.
        Key::Named(Named::ArrowRight | Named::ArrowDown | Named::Space) => Some(Message::Advance(true)),
        Key::Named(Named::ArrowLeft | Named::ArrowUp) => Some(Message::Advance(false)),
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

/// Keys while a menu is open: arrows and Enter move around it, Escape, F10 or Alt closes it.
pub fn menu_keys(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };
    let nav = match key {
        Key::Named(Named::ArrowUp) => Nav::Up,
        Key::Named(Named::ArrowDown) => Nav::Down,
        Key::Named(Named::ArrowLeft) => Nav::Left,
        Key::Named(Named::ArrowRight) => Nav::Right,
        Key::Named(Named::Enter | Named::Space) => Nav::Enter,
        Key::Named(Named::Escape | Named::F10 | Named::Alt) => {
            return Some(Message::Menu(menu::Event::Close));
        }
        _ => return None,
    };
    Some(Message::MenuNav(nav))
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
        assert!(!matches!(letter("r", Modifiers::ALT), Some(Message::ToolKey(_))));
        assert!(letter("q", Modifiers::empty()).is_none());
        assert!(matches!(
            letter("C", Modifiers::COMMAND | Modifiers::SHIFT),
            Some(Message::Annotate(annotate::Event::Toggle))
        ));
    }

    #[test]
    fn presenting_keys() {
        assert!(matches!(letter("l", Modifiers::COMMAND), Some(Message::Present)));
        assert!(matches!(
            press(Key::Named(Named::F11), Modifiers::empty()),
            Some(Message::ToggleFullScreen)
        ));
        assert!(matches!(
            press(Key::Named(Named::Space), Modifiers::empty()),
            Some(Message::Advance(true))
        ));
        assert!(matches!(
            press(Key::Named(Named::ArrowLeft), Modifiers::empty()),
            Some(Message::Advance(false))
        ));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn alt_letters_open_menus() {
        assert!(matches!(
            letter("v", Modifiers::ALT),
            Some(Message::Menu(menu::Event::OpenKey(Menu::View)))
        ));
        assert!(matches!(
            press(Key::Named(Named::F10), Modifiers::empty()),
            Some(Message::Menu(menu::Event::OpenKey(Menu::File)))
        ));
        assert!(matches!(
            menu_keys(keyboard::Event::KeyPressed {
                modified_key: Key::Named(Named::ArrowDown),
                key: Key::Named(Named::ArrowDown),
                physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
                location: Location::Standard,
                modifiers: Modifiers::empty(),
                text: None,
                repeat: false,
            }),
            Some(Message::MenuNav(Nav::Down))
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
