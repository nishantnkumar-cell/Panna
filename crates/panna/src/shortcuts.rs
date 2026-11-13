//! Keyboard shortcuts, following Acrobat's where one exists. `command()` is Ctrl on Windows
//! and Linux, Cmd on macOS.

use iced::keyboard::{self, Key, key::Named};

use crate::Message;
use crate::annotate;
use crate::find::Event;
use crate::menu::{self, Menu, Nav};

pub fn map(event: keyboard::Event) -> Option<Message> {
    // Holding Space picks the Hand tool until it's let go, as in Acrobat.
    if let keyboard::Event::KeyReleased {
        key: Key::Named(Named::Space),
        ..
    } = event
    {
        return Some(Message::SpaceHand(false));
    }
    let keyboard::Event::KeyPressed {
        key,
        modifiers,
        repeat,
        ..
    } = event
    else {
        return None;
    };
    if key == Key::Named(Named::Tab) && !modifiers.command() && !modifiers.alt() {
        return Some(if modifiers.shift() {
            Message::FocusPrevious
        } else {
            Message::FocusNext
        });
    }
    // Previous and next view retrace where you've been: Alt+Left and Alt+Right in Acrobat on
    // Windows, Cmd+Left and Cmd+Right on macOS.
    let history = if cfg!(target_os = "macos") {
        modifiers.command() && !modifiers.shift()
    } else {
        modifiers.alt() && !modifiers.command() && !modifiers.shift()
    };
    if history {
        match key.as_ref() {
            Key::Named(Named::ArrowLeft) => return Some(Message::PrevView),
            Key::Named(Named::ArrowRight) => return Some(Message::NextView),
            _ => {}
        }
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
    if modifiers.shift() && !modifiers.command() {
        match key.as_ref() {
            Key::Named(Named::F3) => return Some(Message::Find(Event::Prev)),
            // Shift+F4 opens or closes the tools pane.
            Key::Named(Named::F4) => return Some(Message::ToggleAllTools),
            Key::Named(Named::Enter) => return Some(Message::EnterKey(true)),
            _ => {}
        }
    }
    if modifiers.command() && key == Key::Named(Named::Tab) {
        return Some(Message::NextTab(!modifiers.shift()));
    }
    if modifiers.command() && modifiers.shift() {
        return match key.as_ref() {
            Key::Character("g" | "G") => Some(Message::Find(Event::Prev)),
            // Acrobat's Advanced Search.
            Key::Character("f" | "F") => Some(Message::FolderSearch(crate::folder_search::Event::Open)),
            Key::Character("z" | "Z") => Some(Message::Annotate(annotate::Event::Redo)),
            Key::Character("s" | "S") => Some(Message::SaveAs),
            Key::Character("=" | "+") => Some(Message::RotateCw),
            Key::Character("-" | "_") => Some(Message::RotateCcw),
            Key::Character("n" | "N") => Some(Message::FocusPageBox),
            Key::Character("c" | "C") => Some(Message::Annotate(annotate::Event::Toggle)),
            // Acrobat's page keys: insert blank pages, insert files, delete pages.
            Key::Character("t" | "T") => Some(Message::Pages(crate::pages::Op::InsertBlank)),
            Key::Character("i" | "I") => Some(Message::Pages(crate::pages::Op::InsertFile)),
            Key::Character("d" | "D") => Some(Message::Pages(crate::pages::Op::DeletePages)),
            Key::Character("r" | "R") => Some(Message::Pages(crate::pages::Op::RotatePages)),
            // Automatic scrolling.
            Key::Character("h" | "H") => Some(Message::AutoScroll),
            // Snap to grid, as in Acrobat.
            Key::Character("u" | "U") => Some(Message::ToggleSnap),
            // Acrobat's Read Out Loud keys.
            Key::Character("y" | "Y") => Some(Message::ReadAloudToggle),
            Key::Character("v" | "V") => Some(Message::ReadAloud(crate::speech::Event::Page)),
            Key::Character("b" | "B") => Some(Message::ReadAloud(crate::speech::Event::ToEnd)),
            Key::Character("e" | "E") => Some(Message::ReadAloud(crate::speech::Event::Stop)),
            Key::Named(Named::PageUp | Named::ArrowUp) => Some(Message::GoToPage(0)),
            Key::Named(Named::PageDown | Named::ArrowDown) => Some(Message::GoToPage(usize::MAX)),
            Key::Named(Named::F5) => Some(Message::ShowSidebar),
            _ => None,
        };
    }
    if modifiers.command() {
        return match key.as_ref() {
            Key::Character("o") => Some(Message::OpenDialog),
            Key::Character("w") => Some(Message::CloseTab),
            Key::Named(Named::F4) => Some(Message::CloseTab),
            Key::Character("d") => Some(Message::ToggleProperties),
            Key::Character("l") => Some(Message::Present),
            Key::Character("h") => Some(Message::ToggleReadMode),
            Key::Character("=" | "+") => Some(Message::ZoomIn),
            Key::Character("-") => Some(Message::ZoomOut),
            Key::Character("0") => Some(Message::FitPage),
            Key::Character("1") => Some(Message::ZoomReset),
            Key::Character("2") => Some(Message::FitWidth),
            Key::Character("3") => Some(Message::FitVisible),
            Key::Character("4") => Some(Message::ToggleReflow),
            Key::Character("r") => Some(Message::ToggleRulers),
            Key::Character("u") => Some(Message::ToggleGrid),
            Key::Character("f") => Some(Message::Find(Event::Open)),
            Key::Character("g") => Some(Message::Find(Event::Next)),
            Key::Character("c") => Some(Message::Copy),
            Key::Character("a") => Some(Message::SelectAll),
            Key::Character("s") => Some(Message::Save),
            Key::Character("p") => Some(Message::Print),
            Key::Character("b") => Some(Message::AddBookmark),
            // Edit › Preferences is Ctrl+K in Acrobat; Ctrl+, is the usual key elsewhere.
            Key::Character("," | "k") => Some(Message::Preferences(crate::preferences::Event::Open)),
            Key::Character("z") => Some(Message::Annotate(annotate::Event::Undo)),
            Key::Character("y") => Some(Message::Annotate(annotate::Event::Redo)),
            Key::Named(Named::PageUp) => Some(Message::PrevPage),
            Key::Named(Named::PageDown) => Some(Message::NextPage),
            _ => None,
        };
    }
    match key.as_ref() {
        Key::Named(Named::PageDown) => Some(Message::NextPage),
        Key::Named(Named::PageUp) => Some(Message::PrevPage),
        Key::Named(Named::Home) => Some(Message::GoToPage(0)),
        Key::Named(Named::End) => Some(Message::GoToPage(usize::MAX)),
        Key::Named(Named::Escape) => Some(Message::CloseOverlays),
        // Enter finishes a polygon, or else moves on a screen (Shift+Enter back), as in Acrobat.
        Key::Named(Named::Enter) => Some(Message::EnterKey(false)),
        // F4 shows or hides the navigation pane, as in Acrobat.
        Key::Named(Named::F3) => Some(Message::Find(Event::Next)),
        Key::Named(Named::F7) => Some(Message::Spelling(crate::spelling::Event::Open)),
        Key::Named(Named::F4) => Some(Message::Sidebar(crate::sidebar::Event::Toggle)),
        Key::Named(Named::F11) => Some(Message::ToggleFullScreen),
        // Left and Right turn pages; Up and Down scroll (and turn pages while presenting).
        Key::Named(Named::ArrowRight) => Some(Message::NextPage),
        Key::Named(Named::ArrowLeft) => Some(Message::PrevPage),
        Key::Named(Named::ArrowDown) => Some(Message::ArrowScroll(true)),
        Key::Named(Named::ArrowUp) => Some(Message::ArrowScroll(false)),
        Key::Named(Named::Space) if !repeat => Some(Message::SpaceHand(true)),
        Key::Named(Named::Delete | Named::Backspace) => {
            Some(Message::Annotate(annotate::Event::DeletePicked))
        }
        // Single letters pick tools, as Acrobat's single-key accelerators do.
        Key::Character(c) if !modifiers.alt() => {
            let mut chars = c.chars();
            match (chars.next(), chars.next()) {
                (Some(letter), None) if letter.is_ascii_alphabetic() => Some(Message::Accelerator {
                    key: letter.to_ascii_lowercase(),
                    shift: modifiers.shift(),
                }),
                _ => None,
            }
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
    fn letters_are_accelerators_only_without_modifiers() {
        assert!(matches!(
            letter("S", Modifiers::SHIFT),
            Some(Message::Accelerator {
                key: 's',
                shift: true
            })
        ));
        assert!(matches!(
            letter("q", Modifiers::empty()),
            Some(Message::Accelerator {
                key: 'q',
                shift: false
            })
        ));
        assert!(!matches!(
            letter("r", Modifiers::ALT),
            Some(Message::Accelerator { .. })
        ));
        assert!(letter("1", Modifiers::empty()).is_none());
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
            Some(Message::SpaceHand(true))
        ));
        assert!(matches!(
            press(Key::Named(Named::ArrowLeft), Modifiers::empty()),
            Some(Message::PrevPage)
        ));
        assert!(matches!(
            press(Key::Named(Named::Enter), Modifiers::SHIFT),
            Some(Message::EnterKey(true))
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
    fn acrobat_page_keys() {
        let cs = Modifiers::COMMAND | Modifiers::SHIFT;
        assert!(matches!(
            letter("D", cs),
            Some(Message::Pages(crate::pages::Op::DeletePages))
        ));
        assert!(matches!(
            letter("T", cs),
            Some(Message::Pages(crate::pages::Op::InsertBlank))
        ));
        assert!(matches!(
            letter("I", cs),
            Some(Message::Pages(crate::pages::Op::InsertFile))
        ));
        assert!(matches!(
            press(Key::Named(Named::PageUp), cs),
            Some(Message::GoToPage(0))
        ));
        assert!(matches!(
            press(Key::Named(Named::ArrowDown), cs),
            Some(Message::GoToPage(usize::MAX))
        ));
        assert!(matches!(
            press(Key::Named(Named::PageDown), Modifiers::COMMAND),
            Some(Message::NextPage)
        ));
        assert!(matches!(
            letter("k", Modifiers::COMMAND),
            Some(Message::Preferences(_))
        ));
        assert!(matches!(
            letter("b", Modifiers::COMMAND),
            Some(Message::AddBookmark)
        ));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn alt_arrows_retrace_views() {
        assert!(matches!(
            press(Key::Named(Named::ArrowLeft), Modifiers::ALT),
            Some(Message::PrevView)
        ));
        assert!(matches!(
            press(Key::Named(Named::ArrowRight), Modifiers::ALT),
            Some(Message::NextView)
        ));
    }

    #[test]
    fn letting_go_of_space_drops_the_hand() {
        let released = map(keyboard::Event::KeyReleased {
            key: Key::Named(Named::Space),
            modified_key: Key::Named(Named::Space),
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers: Modifiers::empty(),
        });
        assert!(matches!(released, Some(Message::SpaceHand(false))));
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
