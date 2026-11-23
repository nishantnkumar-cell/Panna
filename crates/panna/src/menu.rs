//! The menu bar along the top of the window: File, Edit, View and so on, each opening a list
//! of commands with their icons and shortcuts.
//!
//! iced has no menus of its own, so a menu is drawn as a panel pinned under its title. Every
//! row has a fixed height, and each panel is as wide as its longest label and shortcut, measured
//! with the same text shaping the rows are drawn with, so a translated or long label never runs
//! past the panel's edge.

use crate::i18n::text;
use iced::advanced::text::{Paragraph as _, Text};
use iced::widget::{
    Column, Row, button, container, mouse_area, opaque, pin, rich_text, row, scrollable, space, span,
};
use iced::{Center, Element, Fill, Pixels, Point, Size, Theme};

use crate::icons::{self, Icon};

/// Height of the menu bar.
pub const HEIGHT: f32 = 28.0;
/// Height of a menu row, and of the gap a separator takes.
const ROW: f32 = 26.0;
const SEP: f32 = 9.0;
/// Padding inside a menu panel.
const PAD: f32 = 4.0;
/// The narrowest a menu or submenu gets, so short menus don't look cramped.
const MIN_WIDTH: f32 = 200.0;
const SUB_MIN_WIDTH: f32 = 160.0;
/// Text sizes of a row's label and shortcut, and of a menu title.
const LABEL_SIZE: f32 = 13.0;
const KEYS_SIZE: f32 = 12.0;
/// Row padding (left and right), the leading icon or tick, the trailing arrow, and the gaps
/// between them, besides the label and shortcut themselves.
const ROW_PAD: f32 = 8.0;
const LEAD: f32 = 18.0;
const GAP: f32 = 10.0;
/// Room kept between a label and its shortcut.
const KEYS_GAP: f32 = 24.0;
/// Room for the scroll bar of a menu too tall for the window.
const SCROLLBAR: f32 = 12.0;
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
        text_width(&crate::i18n::translate(self.name()), LABEL_SIZE) + 20.0
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

    /// How wide the row needs to be to show its label and shortcut on one line.
    fn width(&self) -> f32 {
        match self {
            Entry::Item { label, keys, .. } => row_width(label, keys, false),
            Entry::Sub { label, .. } => row_width(label, "", true),
            Entry::Separator => 0.0,
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

    /// The menu bar: a title per menu, and `end` at the far right.
    pub fn bar<'a, M: Clone + 'a>(
        &self,
        on_event: impl Fn(Event) -> M + 'a,
        end: Element<'a, M>,
    ) -> Element<'a, M> {
        let mut titles = Row::new().push(space().width(LEFT));
        for menu in Menu::ALL {
            let open = self.open == Some(menu);
            let name = menu.name();
            let k = menu.mnemonic();
            let (before, rest) = name.split_at(k);
            let (letter, after) = rest.split_at(1);
            // macOS has no Alt key letters for menus, so nothing is underlined there.
            // In another language the English letter isn't in the name: Alt still opens the menu.
            let translated = crate::i18n::language() != crate::i18n::Language::English;
            let label: Element<'a, M> = if cfg!(target_os = "macos") || translated {
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
        let titles = titles
            .push(space::horizontal())
            .push(end)
            .push(space().width(LEFT));
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

        let width = panel_width(&entries, MIN_WIDTH, window.width);
        // Taller than the window (a small screen, or a large text size): the menu scrolls.
        let room = window.height - HEIGHT - PAD * 4.0;
        let total: f32 = entries.iter().map(Entry::height).sum();
        let tall = window.height > 0.0 && total > room;
        let outer = if tall { width + SCROLLBAR } else { width };
        let x = menu.x().min((window.width - outer).max(0.0));
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
                        sub_at = Some(Point::new(x + outer - PAD, y - PAD));
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
        let menu_panel = if tall {
            panel(
                scrollable(column.padding(iced::Padding::ZERO.right(SCROLLBAR)))
                    .height(room)
                    .into(),
                outer,
            )
        } else {
            panel(column.into(), width)
        };
        layers.push(pin(menu_panel).position(Point::new(x, HEIGHT)).into());

        if let (Some(at), Some(items)) = (sub_at, sub_items) {
            let sub_hi = self.sub.and_then(|(_, hi)| hi);
            let sub_width = panel_width(&items, SUB_MIN_WIDTH, window.width);
            let items_height: f32 = items.iter().map(Entry::height).sum();
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
            let x = if at.x + sub_width > window.width {
                (x - sub_width + PAD).max(0.0)
            } else {
                at.x
            };
            // Kept inside the window when it opens low down.
            let sub_height = items_height + PAD * 2.0;
            let y = if window.height > 0.0 {
                at.y.min(window.height - sub_height).max(HEIGHT)
            } else {
                at.y
            };
            layers.push(
                pin(panel(column.into(), sub_width))
                    .position(Point::new(x, y))
                    .into(),
            );
        }
        layers
    }
}

/// How wide `content` is on one line at `size`, as the rows draw it (translated first, as
/// the rows' own text does).
fn text_width(content: &str, size: f32) -> f32 {
    type Paragraph = <iced::Renderer as iced::advanced::text::Renderer>::Paragraph;
    let paragraph = Paragraph::with_text(Text {
        content,
        bounds: Size::INFINITE,
        size: Pixels(size),
        line_height: iced::widget::text::LineHeight::default(),
        font: iced::Font::default(),
        align_x: iced::widget::text::Alignment::Default,
        align_y: iced::alignment::Vertical::Top,
        shaping: iced::widget::text::Shaping::Advanced,
        wrapping: iced::widget::text::Wrapping::None,
    });
    // Never less than a rough guess from the length, in case no font could be measured.
    paragraph
        .min_bounds()
        .width
        .max(content.chars().count() as f32 * size * 0.45)
        .ceil()
}

/// The width a row needs: padding, icon, label, room, then the shortcut or submenu arrow.
fn row_width(label: &str, keys: &str, submenu: bool) -> f32 {
    let label = text_width(&crate::i18n::translate(label), LABEL_SIZE);
    let tail = if submenu {
        LEAD
    } else if keys.is_empty() {
        0.0
    } else {
        text_width(&crate::i18n::translate(keys), KEYS_SIZE)
    };
    // Gaps between lead, label, the stretch and the tail.
    ROW_PAD * 2.0 + LEAD + GAP * 3.0 + label + KEYS_GAP + tail
}

/// A panel just wide enough for its widest row, within the window.
fn panel_width<M>(entries: &[Entry<M>], min: f32, window: f32) -> f32 {
    let widest = entries.iter().map(Entry::width).fold(0.0_f32, f32::max);
    let width = (widest + PAD * 2.0).max(min).ceil();
    if window > 0.0 { width.min(window) } else { width }
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
            .size(KEYS_SIZE)
            .wrapping(iced::widget::text::Wrapping::None)
            .style(|theme: &Theme| text::Style {
                color: Some(theme.extended_palette().background.strong.text),
            })
            .into()
    };
    button(
        row![
            lead,
            text(label)
                .size(LABEL_SIZE)
                .wrapping(iced::widget::text::Wrapping::None)
                .style(dim),
            space::horizontal(),
            tail
        ]
        .spacing(10)
        .align_y(Center),
    )
    .width(Fill)
    .height(ROW)
    .padding([4.0, ROW_PAD])
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

fn panel<'a, M: 'a>(column: Element<'a, M>, width: f32) -> Element<'a, M> {
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
    fn panels_fit_their_longest_row() {
        let short: Vec<Entry<u8>> = vec![plain("One", "", Some(1))];
        assert_eq!(panel_width(&short, MIN_WIDTH, 2000.0), MIN_WIDTH);
        let long_label = "Export as text, Office, web, e-book or pictures and a little more";
        let long: Vec<Entry<u8>> = vec![
            plain("One", "", Some(1)),
            plain(long_label, "Ctrl+Shift+Alt+E", Some(2)),
        ];
        let width = panel_width(&long, MIN_WIDTH, 2000.0);
        let needed = text_width(long_label, LABEL_SIZE) + text_width("Ctrl+Shift+Alt+E", KEYS_SIZE);
        assert!(width > needed + LEAD + ROW_PAD * 2.0, "{width} <= {needed}");
        // Never wider than the window.
        assert_eq!(panel_width(&long, MIN_WIDTH, 300.0), 300.0);
        // A submenu row leaves room for its arrow.
        let subs: Vec<Entry<u8>> = vec![sub(None, "One", vec![])];
        assert!(subs[0].width() > short[0].width());
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
