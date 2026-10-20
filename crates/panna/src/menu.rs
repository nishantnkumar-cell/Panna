//! The menu bar along the top of the window: File, Edit, View and so on, each opening a list
//! of commands with their icons and shortcuts.
//!
//! iced has no menus of its own, so a menu is drawn as a panel pinned under its title. Every
//! title and row has a fixed size, which is what lets the panel and its submenus be placed
//! without measuring anything.

use iced::widget::{
    Column, Row, button, container, mouse_area, opaque, pin, rich_text, row, space, span, text,
};
use iced::{Center, Element, Fill, Point, Size, Theme};

use crate::icons::{self, Icon};

/// Height of the menu bar.
pub const HEIGHT: f32 = 28.0;
/// Height of a menu row, and of the gap a separator takes.
const ROW: f32 = 26.0;
const SEP: f32 = 9.0;
/// Padding inside a menu panel.
const PAD: f32 = 4.0;
const WIDTH: f32 = 330.0;
const SUB_WIDTH: f32 = 230.0;
/// Space before the first title.
const LEFT: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    File,
    Edit,
    View,
    Comment,
    Pages,
    Forms,
    Protect,
    Tools,
    Help,
}

impl Menu {
    pub const ALL: [Menu; 9] = [
        Menu::File,
        Menu::Edit,
        Menu::View,
        Menu::Comment,
        Menu::Pages,
        Menu::Forms,
        Menu::Protect,
        Menu::Tools,
        Menu::Help,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Menu::File => "File",
            Menu::Edit => "Edit",
            Menu::View => "View",
            Menu::Comment => "Comment",
            Menu::Pages => "Pages",
            Menu::Forms => "Forms",
            Menu::Protect => "Protect",
            Menu::Tools => "Tools",
            Menu::Help => "Help",
        }
    }

    /// Where in the name the Alt key letter is, unique among the menus.
    fn mnemonic(self) -> usize {
        match self {
            // F and P are taken by File and Pages.
            Menu::Forms => 1,
            Menu::Protect => 1,
            _ => 0,
        }
    }

    /// The menu whose Alt key letter is `c`.
    pub fn from_key(c: &str) -> Option<Menu> {
        let mut chars = c.chars();
        let c = chars.next()?.to_ascii_lowercase();
        if chars.next().is_some() {
            return None;
        }
        Menu::ALL.into_iter().find(|m| {
            m.name()
                .chars()
                .nth(m.mnemonic())
                .is_some_and(|k| k.to_ascii_lowercase() == c)
        })
    }

    fn title_width(self) -> f32 {
        self.name().chars().count() as f32 * 7.2 + 20.0
    }

    /// Left edge of the title, and so of the menu under it.
    fn x(self) -> f32 {
        LEFT + Menu::ALL
            .iter()
            .take_while(|m| **m != self)
            .map(|m| m.title_width())
            .sum::<f32>()
    }

    fn step(self, forward: bool) -> Menu {
        let i = Menu::ALL.iter().position(|m| *m == self).unwrap_or(0);
        let n = Menu::ALL.len();
        let next = if forward { (i + 1) % n } else { (i + n - 1) % n };
        Menu::ALL.get(next).copied().unwrap_or(self)
    }
}

/// One row of a menu.
pub enum Entry<M> {
    Item {
        icon: Option<Icon>,
        label: String,
        keys: String,
        /// `None` shows the row disabled.
        action: Option<M>,
        /// A tick, for settings that are on or off (`Some(false)` leaves room for one).
        checked: Option<bool>,
    },
    Sub {
        icon: Option<Icon>,
        label: &'static str,
        items: Vec<Entry<M>>,
    },
    Separator,
}

impl<M> Entry<M> {
    fn height(&self) -> f32 {
        match self {
            Entry::Separator => SEP,
            _ => ROW,
        }
    }

    fn enabled(&self) -> bool {
        match self {
            Entry::Item { action, .. } => action.is_some(),
            Entry::Sub { .. } => true,
            Entry::Separator => false,
        }
    }
}

/// A command, with the shortcut shown beside it (empty for none).
pub fn item<M>(icon: Icon, label: impl Into<String>, keys: impl Into<String>, action: Option<M>) -> Entry<M> {
    Entry::Item {
        icon: Some(icon),
        label: label.into(),
        keys: keys.into(),
        action,
        checked: None,
    }
}

/// A command without an icon.
pub fn plain<M>(label: impl Into<String>, keys: impl Into<String>, action: Option<M>) -> Entry<M> {
    Entry::Item {
        icon: None,
        label: label.into(),
        keys: keys.into(),
        action,
        checked: None,
    }
}

/// A setting that's on or off, ticked while on.
pub fn check<M>(label: impl Into<String>, keys: impl Into<String>, on: bool, action: Option<M>) -> Entry<M> {
    Entry::Item {
        icon: None,
        label: label.into(),
        keys: keys.into(),
        action,
        checked: Some(on),
    }
}

pub fn sub<M>(icon: Option<Icon>, label: &'static str, items: Vec<Entry<M>>) -> Entry<M> {
    Entry::Sub { icon, label, items }
}

#[derive(Debug, Clone, Copy)]
pub enum Event {
    /// A title was clicked: opens its menu, or closes it when already open.
    Title(Menu),
    /// The pointer moved onto a title: switches menus while one is open.
    HoverTitle(Menu),
    /// Opens a menu from the keyboard (Alt and its letter, or F10).
    OpenKey(Menu),
    HoverRow(usize),
    HoverSubRow(usize),
    Close,
}

/// Keys for moving around an open menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Left,
    Right,
    Enter,
}

#[derive(Debug, Clone, Default)]
pub struct State {
    open: Option<Menu>,
    /// The highlighted row of the open menu.
    hi: Option<usize>,
    /// The row whose submenu is open, and the highlighted row in it.
    sub: Option<(usize, Option<usize>)>,
}

impl State {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn open(&self) -> Option<Menu> {
        self.open
    }

    /// Applies an event; `entries` lists the rows of a menu.
    pub fn update<M>(&mut self, event: Event, entries: impl Fn(Menu) -> Vec<Entry<M>>) {
        match event {
            Event::Title(menu) if self.open == Some(menu) => *self = State::default(),
            Event::Title(menu) => self.show(menu, None),
            Event::HoverTitle(menu) if self.open.is_some() && self.open != Some(menu) => {
                self.show(menu, None);
            }
            Event::HoverTitle(_) => {}
            Event::OpenKey(menu) => {
                let first = first_enabled(&entries(menu), 0, true);
                self.show(menu, first);
            }
            Event::HoverRow(i) => {
                let Some(menu) = self.open else { return };
                self.hi = Some(i);
                let rows = entries(menu);
                self.sub = matches!(rows.get(i), Some(Entry::Sub { .. })).then_some((i, None));
            }
            Event::HoverSubRow(i) => {
                if let Some((row, _)) = self.sub {
                    self.hi = Some(row);
                    self.sub = Some((row, Some(i)));
                }
            }
            Event::Close => *self = State::default(),
        }
    }

    fn show(&mut self, menu: Menu, hi: Option<usize>) {
        *self = State {
            open: Some(menu),
            hi,
            sub: None,
        };
    }

    /// Moves around the open menu with the keyboard. Returns the command Enter picked, which
    /// also closes the menu.
    pub fn nav<M>(&mut self, nav: Nav, entries: impl Fn(Menu) -> Vec<Entry<M>>) -> Option<M> {
        let menu = self.open?;
        let mut rows = entries(menu);
        if let Some((row, sub_hi)) = self.sub
            && let Some(Entry::Sub { items, .. }) = rows.get_mut(row)
        {
            let moved = match nav {
                Nav::Up => Some(step_row(items, sub_hi, false)),
                Nav::Down => Some(step_row(items, sub_hi, true)),
                Nav::Left => {
                    self.sub = None;
                    return None;
                }
                Nav::Right => None,
                Nav::Enter => {
                    let picked = sub_hi.and_then(|i| take_action(items, i));
                    if picked.is_some() {
                        *self = State::default();
                    }
                    return picked;
                }
            };
            if let Some(hi) = moved {
                self.sub = Some((row, hi));
                return None;
            }
        }
        match nav {
            Nav::Up => self.hi = step_row(&rows, self.hi, false),
            Nav::Down => self.hi = step_row(&rows, self.hi, true),
            Nav::Left | Nav::Right => {
                let on_sub = self
                    .hi
                    .and_then(|i| rows.get(i))
                    .is_some_and(|e| matches!(e, Entry::Sub { .. }));
                if nav == Nav::Right && on_sub {
                    let row = self.hi.unwrap_or(0);
                    let first = match rows.get(row) {
                        Some(Entry::Sub { items, .. }) => first_enabled(items, 0, true),
                        _ => None,
                    };
                    self.sub = Some((row, first));
                } else {
                    let next = menu.step(nav == Nav::Right);
                    let first = first_enabled(&entries(next), 0, true);
                    self.show(next, first);
                }
            }
            Nav::Enter => {
                let i = self.hi?;
                if matches!(rows.get(i), Some(Entry::Sub { .. })) {
                    let first = match rows.get(i) {
                        Some(Entry::Sub { items, .. }) => first_enabled(items, 0, true),
                        _ => None,
                    };
                    self.sub = Some((i, first));
                    return None;
                }
                let picked = take_action(&mut rows, i);
                if picked.is_some() {
                    *self = State::default();
                }
                return picked;
            }
        }
        None
    }

    /// The menu bar: a title per menu.
    pub fn bar<'a, M: Clone + 'a>(&self, on_event: impl Fn(Event) -> M + 'a) -> Element<'a, M> {
        let mut titles = Row::new().push(space().width(LEFT));
        for menu in Menu::ALL {
            let open = self.open == Some(menu);
            let name = menu.name();
            let k = menu.mnemonic();
            let (before, rest) = name.split_at(k);
            let (letter, after) = rest.split_at(1);
            // macOS has no Alt key letters for menus, so nothing is underlined there.
            let label: Element<'a, M> = if cfg!(target_os = "macos") {
                text(name).size(13).into()
            } else {
                rich_text::<(), M, Theme, iced::Renderer>([
                    span(before),
                    span(letter).underline(true),
                    span(after),
                ])
                .size(13)
                .into()
            };
            let title = button(container(label).center_x(Fill))
                .width(menu.title_width())
                .height(HEIGHT - 4.0)
                .padding([4, 0])
                .style(move |theme: &Theme, status| row_style(theme, status, open, true))
                .on_press(on_event(Event::Title(menu)));
            titles = titles.push(mouse_area(title).on_enter(on_event(Event::HoverTitle(menu))));
        }
        container(titles.align_y(Center))
            .height(HEIGHT)
            .width(Fill)
            .padding([2, 0])
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style::default().background(palette.background.weak.color)
            })
            .into()
    }

    /// The open menu, to lay over the whole window: a catcher that closes it on a click
    /// elsewhere, the menu, and its open submenu. Empty while no menu is open.
    pub fn panels<'a, M: Clone + 'a>(
        &self,
        entries: Vec<Entry<M>>,
        on_event: impl Fn(Event) -> M + Clone + 'a,
        pick: impl Fn(M) -> M + Clone + 'a,
        window: Size,
    ) -> Vec<Element<'a, M>> {
        let Some(menu) = self.open else {
            return Vec::new();
        };
        let catcher =
            opaque(mouse_area(container(space()).width(Fill).height(Fill)).on_press(on_event(Event::Close)));
        let mut layers: Vec<Element<'a, M>> = vec![pin(catcher).y(HEIGHT).width(Fill).height(Fill).into()];

        let x = menu.x().min((window.width - WIDTH).max(0.0));
        let mut sub_at = None;
        let mut sub_items = None;
        let mut y = HEIGHT + PAD;
        let mut column = Column::new();
        for (i, entry) in entries.into_iter().enumerate() {
            let height = entry.height();
            let hi = self.hi == Some(i);
            match entry {
                Entry::Separator => column = column.push(separator()),
                Entry::Item {
                    icon,
                    label,
                    keys,
                    action,
                    checked,
                } => {
                    column = column.push(
                        mouse_area(row_view(icon, checked, label, keys, action.map(&pick), hi, false))
                            .on_enter(on_event(Event::HoverRow(i))),
                    );
                }
                Entry::Sub { icon, label, items } => {
                    if self.sub.is_some_and(|(row, _)| row == i) {
                        sub_at = Some(Point::new(x + WIDTH - PAD, y - PAD));
                        sub_items = Some(items);
                    }
                    column = column.push(
                        mouse_area(row_view(
                            icon,
                            None,
                            label.to_string(),
                            String::new(),
                            Some(on_event(Event::HoverRow(i))),
                            hi,
                            true,
                        ))
                        .on_enter(on_event(Event::HoverRow(i))),
                    );
                }
            }
            y += height;
        }
        layers.push(pin(panel(column, WIDTH)).position(Point::new(x, HEIGHT)).into());

        if let (Some(at), Some(items)) = (sub_at, sub_items) {
            let sub_hi = self.sub.and_then(|(_, hi)| hi);
            let mut column = Column::new();
            for (i, entry) in items.into_iter().enumerate() {
                match entry {
                    Entry::Item {
                        icon,
                        label,
                        keys,
                        action,
                        checked,
                    } => {
                        column = column.push(
                            mouse_area(row_view(
                                icon,
                                checked,
                                label,
                                keys,
                                action.map(&pick),
                                sub_hi == Some(i),
                                false,
                            ))
                            .on_enter(on_event(Event::HoverSubRow(i))),
                        );
                    }
                    // Submenus don't nest.
                    Entry::Separator | Entry::Sub { .. } => column = column.push(separator()),
                }
            }
            // Open to the left when there's no room on the right.
            let x = if at.x + SUB_WIDTH > window.width {
                (x - SUB_WIDTH + PAD).max(0.0)
            } else {
                at.x
            };
            layers.push(pin(panel(column, SUB_WIDTH)).position(Point::new(x, at.y)).into());
        }
        layers
    }
}

/// The next enabled row after (or before) `from`, wrapping around.
fn step_row<M>(rows: &[Entry<M>], from: Option<usize>, forward: bool) -> Option<usize> {
    let n = rows.len();
    if n == 0 {
        return None;
    }
    let start = match (from, forward) {
        (None, true) => 0,
        (None, false) => n - 1,
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
    };
    first_enabled(rows, start, forward)
}

/// The first enabled row from `start` on (or back), wrapping around.
fn first_enabled<M>(rows: &[Entry<M>], start: usize, forward: bool) -> Option<usize> {
    let n = rows.len();
    (0..n)
        .map(|k| {
            if forward {
                (start + k) % n
            } else {
                (start + n - k) % n
            }
        })
        .find(|&i| rows.get(i).is_some_and(Entry::enabled))
}

fn take_action<M>(rows: &mut [Entry<M>], i: usize) -> Option<M> {
    match rows.get_mut(i) {
        Some(Entry::Item { action, .. }) => action.take(),
        _ => None,
    }
}

fn separator<'a, M: 'a>() -> Element<'a, M> {
    container(container(space().width(Fill).height(1)).style(|theme: &Theme| {
        container::Style::default().background(theme.extended_palette().background.strong.color)
    }))
    .padding([4, 8])
    .height(SEP)
    .into()
}

#[allow(clippy::too_many_arguments)]
fn row_view<'a, M: Clone + 'a>(
    icon: Option<Icon>,
    checked: Option<bool>,
    label: String,
    keys: String,
    on_press: Option<M>,
    hi: bool,
    submenu: bool,
) -> Element<'a, M> {
    let enabled = on_press.is_some();
    let lead: Element<'a, M> = match (checked, icon) {
        (Some(true), _) => icons::glyph(Icon::Check, enabled),
        (_, Some(icon)) => icons::glyph(icon, enabled),
        _ => space().width(18).into(),
    };
    let dim = move |theme: &Theme| {
        let palette = theme.extended_palette();
        text::Style {
            color: Some(if enabled {
                palette.background.base.text
            } else {
                palette.background.strong.color
            }),
        }
    };
    let tail: Element<'a, M> = if submenu {
        icons::glyph(Icon::NextPage, true)
    } else {
        text(keys)
            .size(12)
            .style(|theme: &Theme| text::Style {
                color: Some(theme.extended_palette().background.strong.text),
            })
            .into()
    };
    button(
        row![lead, text(label).size(13).style(dim), space::horizontal(), tail]
            .spacing(10)
            .align_y(Center),
    )
    .width(Fill)
    .height(ROW)
    .padding([4, 8])
    .style(move |theme: &Theme, status| row_style(theme, status, hi, enabled))
    .on_press_maybe(on_press)
    .into()
}

fn row_style(theme: &Theme, status: button::Status, hi: bool, enabled: bool) -> button::Style {
    let palette = theme.extended_palette();
    let lit = enabled && (hi || matches!(status, button::Status::Hovered | button::Status::Pressed));
    button::Style {
        background: lit.then(|| palette.primary.weak.color.into()),
        text_color: if lit {
            palette.primary.weak.text
        } else {
            palette.background.base.text
        },
        border: iced::Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn panel<'a, M: 'a>(column: Column<'a, M>, width: f32) -> Element<'a, M> {
    opaque(
        container(column)
            .width(width)
            .padding(PAD)
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style::default()
                    .background(palette.background.base.color)
                    .border(iced::Border {
                        color: palette.background.strong.color,
                        width: 1.0,
                        radius: 6.0.into(),
                    })
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Entry<u8>> {
        vec![
            plain("One", "", Some(1)),
            plain("Off", "", None),
            Entry::Separator,
            sub(
                None,
                "More",
                vec![plain("Three", "", Some(3)), plain("Four", "", Some(4))],
            ),
            plain("Two", "", Some(2)),
        ]
    }

    #[test]
    fn alt_letters_are_unique() {
        for menu in Menu::ALL {
            let letter = menu.name().chars().nth(menu.mnemonic()).unwrap().to_string();
            assert_eq!(Menu::from_key(&letter), Some(menu), "{menu:?}");
        }
        assert_eq!(Menu::from_key("o"), Some(Menu::Forms));
        assert_eq!(Menu::from_key("r"), Some(Menu::Protect));
        assert_eq!(Menu::from_key("z"), None);
    }

    #[test]
    fn titles_sit_side_by_side() {
        assert_eq!(Menu::File.x(), LEFT);
        assert_eq!(Menu::Edit.x(), LEFT + Menu::File.title_width());
    }

    #[test]
    fn keys_skip_disabled_rows_and_pick_commands() {
        let mut state = State::default();
        state.update(Event::OpenKey(Menu::Edit), |_| rows());
        assert_eq!(state.hi, Some(0));
        assert_eq!(state.nav(Nav::Down, |_| rows()), None);
        assert_eq!(state.hi, Some(3), "skips the disabled row and the separator");
        // Into the submenu and pick from it.
        state.nav(Nav::Right, |_| rows());
        assert_eq!(state.sub, Some((3, Some(0))));
        state.nav(Nav::Down, |_| rows());
        assert_eq!(state.nav(Nav::Enter, |_| rows()), Some(4));
        assert!(!state.is_open());
    }

    #[test]
    fn left_and_right_switch_menus() {
        let mut state = State::default();
        state.update(Event::OpenKey(Menu::File), |_| rows());
        state.nav(Nav::Left, |_| rows());
        assert_eq!(state.open(), Some(Menu::Help));
        state.nav(Nav::Right, |_| rows());
        assert_eq!(state.open(), Some(Menu::File));
    }

    #[test]
    fn hovering_titles_switches_only_while_open() {
        let mut state = State::default();
        state.update(Event::HoverTitle(Menu::View), |_| rows());
        assert!(!state.is_open());
        state.update(Event::Title(Menu::File), |_| rows());
        state.update(Event::HoverTitle(Menu::View), |_| rows());
        assert_eq!(state.open(), Some(Menu::View));
        state.update(Event::Title(Menu::View), |_| rows());
        assert!(!state.is_open());
    }
}
