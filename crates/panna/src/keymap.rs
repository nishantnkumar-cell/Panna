//! Customisable keyboard shortcuts. The defaults stay in [`crate::shortcuts`]; a key the user
//! gives an action here wins over them, and an action's own default key stops working once the
//! action has been given another (or none).

use std::collections::BTreeMap;

use crate::i18n::text;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Column, button, column, container, row, scrollable, space};
use iced::{Center, Element, Fill};

use crate::Message;
use crate::icons::{self, Icon};

/// A key with the modifiers held. `command` is Ctrl on Windows and Linux, Cmd on macOS.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Chord {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    /// A lower-case character, or a named key such as `F3` or `PageDown`.
    pub key: String,
}

/// Named keys that can be part of a shortcut, as stored and as shown.
const NAMED: &[(Named, &str, &str)] = &[
    (Named::F1, "F1", "F1"),
    (Named::F2, "F2", "F2"),
    (Named::F3, "F3", "F3"),
    (Named::F4, "F4", "F4"),
    (Named::F5, "F5", "F5"),
    (Named::F6, "F6", "F6"),
    (Named::F7, "F7", "F7"),
    (Named::F8, "F8", "F8"),
    (Named::F9, "F9", "F9"),
    (Named::F10, "F10", "F10"),
    (Named::F11, "F11", "F11"),
    (Named::F12, "F12", "F12"),
    (Named::PageUp, "PageUp", "Page Up"),
    (Named::PageDown, "PageDown", "Page Down"),
    (Named::Home, "Home", "Home"),
    (Named::End, "End", "End"),
    (Named::Insert, "Insert", "Insert"),
    (Named::Delete, "Delete", "Delete"),
    (Named::Tab, "Tab", "Tab"),
    (Named::Enter, "Enter", "Enter"),
    (Named::Space, "Space", "Space"),
    (Named::ArrowUp, "Up", "↑"),
    (Named::ArrowDown, "Down", "↓"),
    (Named::ArrowLeft, "Left", "←"),
    (Named::ArrowRight, "Right", "→"),
];

impl Chord {
    /// The chord a key press makes, or `None` for keys that can't be shortcuts on their own
    /// (modifiers, Escape, Backspace).
    pub fn from_event(event: &keyboard::Event) -> Option<Self> {
        let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
            return None;
        };
        let key = match key.as_ref() {
            Key::Character(c) => {
                let mut chars = c.chars();
                let (Some(ch), None) = (chars.next(), chars.next()) else {
                    return None;
                };
                normalise(ch)?.to_string()
            }
            Key::Named(named) => NAMED.iter().find(|(n, ..)| *n == named)?.1.to_owned(),
            Key::Unidentified => return None,
        };
        Some(Self {
            command: modifiers.command(),
            shift: modifiers.shift(),
            alt: modifiers.alt(),
            key,
        })
    }

    /// As stored: `Ctrl+Shift+h`, `F7`.
    pub fn as_str(&self) -> String {
        let mut out = String::new();
        if self.command {
            out.push_str("Ctrl+");
        }
        if self.shift {
            out.push_str("Shift+");
        }
        if self.alt {
            out.push_str("Alt+");
        }
        out.push_str(&self.key);
        out
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let mut chord = Self {
            command: false,
            shift: false,
            alt: false,
            key: String::new(),
        };
        let mut rest = s;
        loop {
            if let Some(r) = rest.strip_prefix("Ctrl+") {
                chord.command = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("Shift+") {
                chord.shift = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("Alt+") {
                chord.alt = true;
                rest = r;
            } else {
                break;
            }
        }
        let mut chars = rest.chars();
        chord.key = match (chars.next(), chars.next()) {
            (Some(ch), None) => normalise(ch)?.to_string(),
            _ => NAMED.iter().find(|(_, name, _)| *name == rest)?.1.to_owned(),
        };
        Some(chord)
    }

    /// As shown in menus: `Ctrl+Shift+H`, or `⌘Shift+H` on macOS.
    pub fn label(&self) -> String {
        let mut out = String::new();
        if self.command {
            out.push_str(icons::MOD);
        }
        if self.shift {
            out.push_str("Shift+");
        }
        if self.alt {
            out.push_str(if cfg!(target_os = "macos") { "⌥" } else { "Alt+" });
        }
        match NAMED.iter().find(|(_, name, _)| *name == self.key) {
            Some((_, _, shown)) => out.push_str(shown),
            None => match self.key.as_str() {
                "=" => out.push('+'),
                "-" => out.push('−'),
                key => out.push_str(&key.to_uppercase()),
            },
        }
        out
    }
}

/// One character as a shortcut key: lower case, with the shifted `+` and `_` on the same keys
/// as `=` and `-`. Control characters and spaces can't be keys.
fn normalise(ch: char) -> Option<char> {
    if ch.is_control() || ch.is_whitespace() {
        return None;
    }
    Some(match ch {
        '+' => '=',
        '_' | '−' => '-',
        c => c.to_lowercase().next().unwrap_or(c),
    })
}

/// A default chord, written compactly in the table below.
#[derive(Debug, Clone, Copy)]
pub struct DefaultKey {
    command: bool,
    shift: bool,
    key: &'static str,
}

impl DefaultKey {
    pub fn chord(self) -> Chord {
        Chord {
            command: self.command,
            shift: self.shift,
            alt: false,
            key: self.key.to_owned(),
        }
    }
}

macro_rules! actions {
    ($($name:ident, $id:literal, $label:literal, $default:expr, $message:expr;)*) => {
        /// The actions whose shortcuts can be changed.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        pub enum Action { $($name),* }

        impl Action {
            pub const ALL: &[Action] = &[$(Action::$name),*];

            /// The name it's stored under.
            pub fn id(self) -> &'static str {
                match self { $(Action::$name => $id),* }
            }

            pub fn label(self) -> &'static str {
                match self { $(Action::$name => $label),* }
            }

            pub fn default_chord(self) -> Chord {
                match self { $(Action::$name => $default.chord()),* }
            }

            pub fn message(self) -> Message {
                match self { $(Action::$name => $message),* }
            }
        }
    };
}

const fn ctrl(key: &'static str) -> DefaultKey {
    DefaultKey {
        command: true,
        shift: false,
        key,
    }
}
const fn ctrl_shift(key: &'static str) -> DefaultKey {
    DefaultKey {
        command: true,
        shift: true,
        key,
    }
}
const fn plain(key: &'static str) -> DefaultKey {
    DefaultKey {
        command: false,
        shift: false,
        key,
    }
}
const fn shift(key: &'static str) -> DefaultKey {
    DefaultKey {
        command: false,
        shift: true,
        key,
    }
}

use crate::find::Event as Find;
actions! {
    New, "new", "New blank PDF", ctrl("n"),
        Message::Blank(crate::create::Event::Open);
    Open, "open", "Open", ctrl("o"), Message::OpenDialog;
    Save, "save", "Save", ctrl("s"), Message::Save;
    SaveAs, "save_as", "Save as", ctrl_shift("s"), Message::SaveAs;
    Print, "print", "Print", ctrl("p"), Message::Print;
    Close, "close", "Close the document", ctrl("w"), Message::CloseTab;
    Properties, "properties", "Document properties", ctrl("d"), Message::ToggleProperties;
    Find, "find", "Find", ctrl("f"), Message::Find(Find::Open);
    FindNext, "find_next", "Find next", plain("F3"), Message::Find(Find::Next);
    FindPrev, "find_prev", "Find previous", shift("F3"), Message::Find(Find::Prev);
    FolderSearch, "folder_search", "Search a folder of PDFs", ctrl_shift("f"),
        Message::FolderSearch(crate::folder_search::Event::Open);
    Spelling, "spelling", "Check spelling", plain("F7"), Message::Spelling(crate::spelling::Event::Open);
    Undo, "undo", "Undo", ctrl("z"), Message::Annotate(crate::annotate::Event::Undo);
    Redo, "redo", "Redo", ctrl("y"), Message::Annotate(crate::annotate::Event::Redo);
    Copy, "copy", "Copy", ctrl("c"), Message::Copy;
    SelectAll, "select_all", "Select all", ctrl("a"), Message::SelectAll;
    Preferences, "preferences", "Preferences", ctrl("k"),
        Message::Preferences(crate::preferences::Event::Open);
    ZoomIn, "zoom_in", "Zoom in", ctrl("="), Message::ZoomIn;
    ZoomOut, "zoom_out", "Zoom out", ctrl("-"), Message::ZoomOut;
    ActualSize, "actual_size", "Actual size", ctrl("1"), Message::ZoomReset;
    FitWidth, "fit_width", "Fit width", ctrl("2"), Message::FitWidth;
    FitVisible, "fit_visible", "Fit visible", ctrl("3"), Message::FitVisible;
    FitPage, "fit_page", "Fit page", ctrl("0"), Message::FitPage;
    Reflow, "reflow", "Reflow", ctrl("4"), Message::ToggleReflow;
    NextPage, "next_page", "Next page", plain("PageDown"), Message::NextPage;
    PrevPage, "prev_page", "Previous page", plain("PageUp"), Message::PrevPage;
    FirstPage, "first_page", "First page", plain("Home"), Message::GoToPage(0);
    LastPage, "last_page", "Last page", plain("End"), Message::GoToPage(usize::MAX);
    GoToPage, "go_to_page", "Go to page", ctrl_shift("n"), Message::FocusPageBox;
    RotateCw, "rotate_view_cw", "Rotate the view clockwise", ctrl_shift("="), Message::RotateCw;
    RotateCcw, "rotate_view_ccw", "Rotate the view anticlockwise", ctrl_shift("-"), Message::RotateCcw;
    InsertBlank, "insert_blank", "Insert a blank page", ctrl_shift("t"),
        Message::Pages(crate::pages::Op::InsertBlank);
    InsertFile, "insert_file", "Insert a file", ctrl_shift("i"), Message::Pages(crate::pages::Op::InsertFile);
    DeletePages, "delete_pages", "Delete pages", ctrl_shift("d"), Message::Pages(crate::pages::Op::DeletePages);
    RotatePages, "rotate_pages", "Rotate pages", ctrl_shift("r"), Message::Pages(crate::pages::Op::RotatePages);
    AddBookmark, "bookmark", "Add a bookmark", ctrl("b"), Message::AddBookmark;
    NavPane, "nav_pane", "Navigation pane", plain("F4"), Message::Sidebar(crate::sidebar::Event::Toggle);
    AllTools, "all_tools", "All tools", shift("F4"), Message::ToggleAllTools;
    CommentTools, "comment_tools", "Comment tools", ctrl_shift("c"),
        Message::Annotate(crate::annotate::Event::Toggle);
    ReadMode, "read_mode", "Read mode", ctrl("h"), Message::ToggleReadMode;
    Present, "present", "Full screen, pages only", ctrl("l"), Message::Present;
    FullScreen, "full_screen", "Full screen with the tools", plain("F11"), Message::ToggleFullScreen;
    AutoScroll, "auto_scroll", "Scroll automatically", ctrl_shift("h"), Message::AutoScroll;
    ReadAloud, "read_aloud", "Read aloud, or stop", ctrl_shift("y"), Message::ReadAloudToggle;
    Rulers, "rulers", "Rulers", ctrl("r"), Message::ToggleRulers;
    Grid, "grid", "Grid", ctrl("u"), Message::ToggleGrid;
    Snap, "snap", "Snap to grid and guides", ctrl_shift("u"), Message::ToggleSnap;
}

/// What a key press does under the user's shortcuts.
#[derive(Debug)]
pub enum Resolved {
    /// The user's shortcuts give it an action.
    Bound(Action),
    /// It's the default key of an action the user moved elsewhere: it does nothing.
    Blocked,
    /// The default shortcuts decide.
    Default,
}

/// The keys the user has changed: an action to its key, or `None` for no key.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Keymap {
    pub changed: BTreeMap<Action, Option<Chord>>,
}

impl Keymap {
    /// The key an action has now.
    pub fn chord(&self, action: Action) -> Option<Chord> {
        match self.changed.get(&action) {
            Some(chord) => chord.clone(),
            None => Some(action.default_chord()),
        }
    }

    /// The action a key belongs to now, if any.
    fn action_for(&self, chord: &Chord) -> Option<Action> {
        Action::ALL
            .iter()
            .copied()
            .find(|a| self.chord(*a).as_ref() == Some(chord))
    }

    pub fn resolve(&self, event: &keyboard::Event) -> Resolved {
        if self.changed.is_empty() {
            return Resolved::Default;
        }
        let Some(chord) = Chord::from_event(event) else {
            return Resolved::Default;
        };
        if let Some(action) = self.action_for(&chord) {
            return Resolved::Bound(action);
        }
        if Action::ALL
            .iter()
            .any(|a| self.changed.contains_key(a) && a.default_chord() == chord)
        {
            return Resolved::Blocked;
        }
        Resolved::Default
    }

    /// Gives `action` the key `chord`, taking it from any action that had it. Returns that
    /// action.
    pub fn set(&mut self, action: Action, chord: Option<Chord>) -> Option<Action> {
        let taken = chord
            .as_ref()
            .and_then(|c| self.action_for(c))
            .filter(|other| *other != action);
        if let Some(other) = taken {
            self.put(other, None);
        }
        self.put(action, chord);
        taken
    }

    fn put(&mut self, action: Action, chord: Option<Chord>) {
        if chord.as_ref() == Some(&action.default_chord()) {
            self.changed.remove(&action);
        } else {
            self.changed.insert(action, chord);
        }
    }

    /// Back to the default key. Another action holding that key loses it.
    pub fn reset(&mut self, action: Action) -> Option<Action> {
        self.set(action, Some(action.default_chord()))
    }

    /// The label a menu shows for a default key label, following the user's changes.
    pub fn menu_label(&self, default: &str) -> Option<String> {
        if self.changed.is_empty() || default.is_empty() {
            return None;
        }
        let action = Action::ALL
            .iter()
            .copied()
            .find(|a| self.changed.contains_key(a) && a.default_chord().label() == default)?;
        Some(self.chord(action).map(|c| c.label()).unwrap_or_default())
    }

    /// Settings lines: `keys.bind.save=Ctrl+s`, or nothing after `=` for no key.
    pub fn serialize(&self, out: &mut String) {
        for (action, chord) in &self.changed {
            out.push_str("keys.bind.");
            out.push_str(action.id());
            out.push('=');
            if let Some(chord) = chord {
                out.push_str(&chord.as_str());
            }
            out.push('\n');
        }
    }

    /// Reads one settings line; `true` if it was a shortcut's.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        let Some(id) = key.strip_prefix("keys.bind.") else {
            return false;
        };
        let Some(action) = Action::ALL.iter().copied().find(|a| a.id() == id) else {
            return true;
        };
        if value.trim().is_empty() {
            self.put(action, None);
        } else if let Some(chord) = Chord::parse(value) {
            // A damaged file can't give two actions one key.
            if self.action_for(&chord).is_none_or(|a| a == action) {
                self.put(action, Some(chord));
            }
        }
        true
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    /// Waits for the next key press to become this action's shortcut.
    Change(Action),
    Remove(Action),
    Reset(Action),
    ResetAll,
}

/// The Keyboard shortcuts card: every action that can be changed, with its key.
pub fn card<'a>(keymap: &Keymap, waiting: Option<Action>, note: Option<&'a str>) -> Element<'a, Message> {
    let ev = Message::Keys;
    let rows = Action::ALL.iter().map(|&action| {
        let current = keymap.chord(action);
        let shown = if waiting == Some(action) {
            "Press a key…".to_owned()
        } else {
            current.as_ref().map_or_else(|| "None".to_owned(), Chord::label)
        };
        let changed = keymap.changed.contains_key(&action);
        let mut line = row![
            text(action.label()).size(13).width(Fill),
            button(text(shown).size(13))
                .width(150)
                .style(if waiting == Some(action) {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(ev(Event::Change(action))),
            icons::button(
                Icon::Close,
                "No shortcut",
                current.is_some().then_some(ev(Event::Remove(action)))
            ),
            icons::button(
                Icon::Undo,
                "Back to the default",
                changed.then_some(ev(Event::Reset(action)))
            ),
        ]
        .spacing(6)
        .align_y(Center);
        if changed {
            line = line.push(text("changed").size(11).style(text::secondary));
        } else {
            line = line.push(space::horizontal().width(48));
        }
        line.into()
    });
    let mut body = column![
        text("Keyboard shortcuts").size(16),
        text(
            "Click a shortcut and press the new keys. Esc stops waiting; Backspace leaves the action \
             without one. Single-letter tool keys are set in Preferences."
        )
        .size(12)
        .style(text::secondary),
        text(format!(
            "Keys that stay as they are: Tab and Shift+Tab between fields; Alt and a menu's letter, or \
             F10, for the menus; {} for the previous and next view; Enter and Shift+Enter a screen \
             on or back; Space (held) for the hand; H Z V S X Q K U D and Shift+U, Shift+D, Shift+K \
             for the tools. While scrolling automatically, 0 to 9 set the speed, ↑ and ↓ change \
             it, − turns back and Esc stops.",
            if cfg!(target_os = "macos") {
                "⌘← and ⌘→"
            } else {
                "Alt+← and Alt+→"
            }
        ))
        .size(12)
        .style(text::secondary),
        scrollable(Column::with_children(rows).spacing(4).padding([0, 12])).height(420),
    ]
    .spacing(10)
    .width(560);
    if let Some(note) = note {
        body = body.push(text(note).size(12).style(text::secondary));
    }
    body = body.push(row![
        button(crate::i18n::text("Reset all"))
            .style(button::secondary)
            .on_press_maybe((!keymap.changed.is_empty()).then_some(ev(Event::ResetAll))),
        space::horizontal(),
        button(crate::i18n::text("Done")).on_press(Message::Shortcuts(false)),
    ]);
    container(body).padding(16).style(container::rounded_box).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::{Location, Modifiers, key};

    fn event(key: Key, modifiers: Modifiers) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }
    }

    fn press(chord: &Chord) -> keyboard::Event {
        let key = match NAMED.iter().find(|(_, name, _)| *name == chord.key) {
            Some((named, ..)) => Key::Named(*named),
            None => Key::Character(chord.key.as_str().into()),
        };
        let mut modifiers = Modifiers::empty();
        if chord.command {
            modifiers |= Modifiers::COMMAND;
        }
        if chord.shift {
            modifiers |= Modifiers::SHIFT;
        }
        event(key, modifiers)
    }

    /// Each action's default key does the same as the built-in shortcuts, so the table can't
    /// drift from them.
    #[test]
    fn defaults_match_the_built_in_shortcuts() {
        for action in Action::ALL {
            let built_in = crate::shortcuts::map(press(&action.default_chord()));
            assert_eq!(
                format!("{built_in:?}"),
                format!("{:?}", Some(action.message())),
                "{action:?}"
            );
        }
    }

    #[test]
    fn defaults_are_all_different() {
        for (i, a) in Action::ALL.iter().enumerate() {
            for b in Action::ALL.iter().skip(i + 1) {
                assert_ne!(a.default_chord(), b.default_chord(), "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn a_moved_shortcut_wins_and_the_old_key_stops() {
        let mut keys = Keymap::default();
        assert!(matches!(
            keys.resolve(&press(&Chord::parse("Ctrl+b").unwrap())),
            Resolved::Default
        ));
        let taken = keys.set(Action::AutoScroll, Chord::parse("Ctrl+b"));
        assert_eq!(taken, Some(Action::AddBookmark));
        assert!(matches!(
            keys.resolve(&press(&Chord::parse("Ctrl+b").unwrap())),
            Resolved::Bound(Action::AutoScroll)
        ));
        assert!(matches!(
            keys.resolve(&press(&Chord::parse("Ctrl+Shift+h").unwrap())),
            Resolved::Blocked
        ));
        assert_eq!(keys.chord(Action::AddBookmark), None);
        // Shift+B types a capital B; the chord is the same key.
        let shifted = event(Key::Character("B".into()), Modifiers::COMMAND);
        assert!(matches!(
            keys.resolve(&shifted),
            Resolved::Bound(Action::AutoScroll)
        ));
        keys.reset(Action::AutoScroll);
        keys.reset(Action::AddBookmark);
        assert!(keys.changed.is_empty());
    }

    #[test]
    fn round_trips_and_ignores_damage() {
        let mut keys = Keymap::default();
        keys.set(Action::Save, Chord::parse("Ctrl+Shift+F12"));
        keys.set(Action::Print, None);
        let mut out = String::new();
        keys.serialize(&mut out);
        let mut back = Keymap::default();
        for line in out.lines() {
            let (k, v) = line.split_once('=').unwrap();
            assert!(back.parse_line(k, v));
        }
        assert_eq!(back, keys);
        let mut damaged = Keymap::default();
        assert!(damaged.parse_line("keys.bind.nonsense", "Ctrl+q"));
        assert!(damaged.parse_line("keys.bind.save", "Ctrl+Hyper+q"));
        assert!(!damaged.parse_line("keys.single", "true"));
        assert!(damaged.changed.is_empty());
    }

    #[test]
    fn labels() {
        assert_eq!(
            Chord::parse("Ctrl+Shift+h").unwrap().label(),
            format!("{}Shift+H", icons::MOD)
        );
        assert_eq!(Chord::parse("PageDown").unwrap().label(), "Page Down");
        assert_eq!(
            Chord::parse("Ctrl+=").unwrap().label(),
            format!("{}+", icons::MOD)
        );
        assert_eq!(Chord::parse("Ctrl+Hyper+q"), None);
    }
}
