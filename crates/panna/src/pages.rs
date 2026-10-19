//! Page tools in the Pages tab: turning, moving, adding, removing and extracting pages.
//!
//! Each tool works on the current page and becomes a new page list for the worker, which
//! rewrites the document in memory until it is saved.

use iced::widget::{button, checkbox, column, container, radio, row, slider, space, text, text_input};
use iced::{Element, Theme};
use panna_core::{PageSize, PageSource};

use crate::Message;
use crate::icons::{self, Icon};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    RotateCcw,
    RotateCw,
    MoveUp,
    MoveDown,
    /// A blank page after the current one, the same size.
    InsertBlank,
    /// Asks for a PDF and puts its pages after the current one.
    InsertFile,
    /// Asks for a PDF and which pages of it replace which pages here.
    Replace,
    /// Asks which pages to save as new files.
    Extract,
    /// Asks how much to trim from the page edges.
    Crop,
    /// Asks what text to put at the top and bottom of pages.
    HeaderFooter,
    /// Asks what watermark or page colour to add.
    Watermark,
    /// Asks whether to draw comments and form fields into the pages.
    Flatten,
    Delete,
    Undo,
}

/// The page list after `op` on page `current` of `count`, and the page to show afterwards.
/// `None` when it doesn't apply (moving the first page up, removing the only page) or the
/// tool needs more than a page list.
pub fn plan(op: Op, count: usize, current: usize, size: PageSize) -> Option<(Vec<PageSource>, usize)> {
    if current >= count {
        return None;
    }
    let mut pages: Vec<PageSource> = (0..count)
        .map(|index| PageSource::Page {
            doc: 0,
            index,
            turns: 0,
        })
        .collect();
    let show = match op {
        Op::RotateCw | Op::RotateCcw => {
            if let Some(PageSource::Page { turns, .. }) = pages.get_mut(current) {
                *turns = if op == Op::RotateCw { 1 } else { 3 };
            }
            current
        }
        Op::MoveUp if current > 0 => {
            pages.swap(current, current - 1);
            current - 1
        }
        Op::MoveDown if current + 1 < count => {
            pages.swap(current, current + 1);
            current + 1
        }
        Op::InsertBlank => {
            pages.insert(
                current + 1,
                PageSource::Blank {
                    width: size.width,
                    height: size.height,
                },
            );
            current + 1
        }
        Op::Delete if count > 1 => {
            pages.remove(current);
            current.min(count - 2)
        }
        _ => return None,
    };
    Some((pages, show))
}

/// The row of page tools above the thumbnails.
pub fn tools<'a>(count: usize, current: usize, can_undo: bool, busy: bool) -> Element<'a, Message> {
    let ev = |op: Op, ok: bool| (ok && !busy && count > 0).then_some(Message::Pages(op));
    let m = icons::MOD;
    row![
        icons::button(Icon::RotateCcw, "Turn this page left", ev(Op::RotateCcw, true)),
        icons::button(Icon::RotateCw, "Turn this page right", ev(Op::RotateCw, true)),
        icons::button(Icon::MoveUp, "Move this page up", ev(Op::MoveUp, current > 0)),
        icons::button(
            Icon::MoveDown,
            "Move this page down",
            ev(Op::MoveDown, current + 1 < count)
        ),
        icons::button(
            Icon::PageAdd,
            "Add a blank page after this one",
            ev(Op::InsertBlank, true)
        ),
        icons::button(
            Icon::PageInsertFile,
            "Insert pages from another PDF after this one",
            ev(Op::InsertFile, true)
        ),
        icons::button(
            Icon::PageReplace,
            "Replace pages with pages from another PDF",
            ev(Op::Replace, true)
        ),
        icons::button(
            Icon::PageExtract,
            "Save pages as a new PDF, or split into files",
            ev(Op::Extract, true)
        ),
        icons::button(Icon::Crop, "Crop pages", ev(Op::Crop, true)),
        icons::button(
            Icon::PageNumbers,
            "Add page numbers, headers and footers",
            ev(Op::HeaderFooter, true)
        ),
        icons::button(
            Icon::Watermark,
            "Add a watermark or page colour",
            ev(Op::Watermark, true)
        ),
        icons::button(
            Icon::Flatten,
            "Flatten comments and form fields into the pages",
            ev(Op::Flatten, true)
        ),
        icons::button(Icon::Delete, "Delete this page", ev(Op::Delete, count > 1)),
        icons::button(
            Icon::Undo,
            format!("Undo page change ({m}Z)"),
            ev(Op::Undo, can_undo)
        ),
    ]
    .spacing(0)
    .padding([2, 4])
    // Onto a second line when the sidebar is too narrow for them all.
    .wrap()
    .into()
}

/// The Extract pages card: which pages, and whether to make one file of them or one each.
#[derive(Debug, Clone, Default)]
pub struct Extracting {
    pub range: String,
    pub split: bool,
}

#[derive(Debug, Clone)]
pub enum ExtractEvent {
    Range(String),
    Split(bool),
    Go,
    Cancel,
}

pub fn extract_input_id() -> iced::widget::Id {
    iced::widget::Id::new("extract-range")
}

/// Page numbers like "1-3, 5, 8-" as page indices in the order given. A range may be open at
/// either end ("8-" to the last page, "-3" from the first) and may count down.
pub fn parse_range(spec: &str, count: usize) -> Result<Vec<usize>, String> {
    let number = |part: &str, default: usize| -> Result<usize, String> {
        let part = part.trim();
        if part.is_empty() {
            return Ok(default);
        }
        match part.parse::<usize>() {
            Ok(n) if (1..=count).contains(&n) => Ok(n),
            Ok(_) => Err(format!("Page numbers go from 1 to {count}.")),
            Err(_) => Err(format!("“{part}” isn't a page number.")),
        }
    };
    let mut pages = Vec::new();
    for part in spec.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        match part.split_once(['-', '–']) {
            Some((from, to)) => {
                let (from, to) = (number(from, 1)?, number(to, count)?);
                if from <= to {
                    pages.extend(from - 1..to);
                } else {
                    pages.extend((to - 1..from).rev());
                }
            }
            None => pages.push(number(part, 1)? - 1),
        }
    }
    if pages.is_empty() {
        return Err("Enter the pages, such as 1-3, 5.".into());
    }
    Ok(pages)
}

/// The Extract pages card for a document of `count` pages.
pub fn extract_card<'a>(state: &Extracting, count: usize) -> Element<'a, Message> {
    let ev = Message::Extract;
    let parsed = parse_range(&state.range, count);
    let hint = match &parsed {
        Ok(pages) if pages.len() == 1 => text("1 page").size(13),
        Ok(pages) => text(format!("{} pages", pages.len())).size(13),
        Err(e) => text(e.clone()).size(13).style(|theme: &Theme| text::Style {
            color: Some(theme.extended_palette().danger.base.color),
        }),
    };
    let split = Some(state.split);
    container(
        column![
            text("Extract pages").size(16),
            text_input(&format!("Pages, such as 1-3, 5 (1 to {count})"), &state.range)
                .id(extract_input_id())
                .on_input(move |v| ev(ExtractEvent::Range(v)))
                .on_submit(ev(ExtractEvent::Go)),
            hint,
            radio("Save them as one PDF", false, split, |v| ev(ExtractEvent::Split(
                v
            ))),
            radio("Save each page as its own PDF", true, split, |v| {
                ev(ExtractEvent::Split(v))
            }),
            row![
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(ev(ExtractEvent::Cancel)),
                button("Extract…").on_press_maybe(parsed.is_ok().then_some(ev(ExtractEvent::Go))),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .width(340),
    )
    .padding(16)
    .style(container::rounded_box)
    .into()
}

/// The Crop pages card: margins in millimetres as typed, and whether every page is cropped.
#[derive(Debug, Clone)]
pub struct Cropping {
    /// Top, right, bottom, left.
    pub margins: [String; 4],
    pub all: bool,
}

impl Default for Cropping {
    fn default() -> Self {
        Self {
            margins: std::array::from_fn(|_| "0".to_owned()),
            all: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CropEvent {
    Margin(usize, String),
    All(bool),
    Go,
    Cancel,
}

pub fn crop_input_id() -> iced::widget::Id {
    iced::widget::Id::new("crop-top")
}

const POINTS_PER_MM: f32 = 72.0 / 25.4;

/// The margins in points, or what's wrong with them.
pub fn crop_margins(state: &Cropping) -> Result<panna_core::Margins, String> {
    let mut out = [0.0; 4];
    for (slot, typed) in out.iter_mut().zip(&state.margins) {
        let typed = typed.trim().replace(',', ".");
        let mm: f32 = if typed.is_empty() {
            0.0
        } else {
            typed
                .parse()
                .map_err(|_| format!("“{typed}” isn't a number of millimetres."))?
        };
        if !(0.0..=2000.0).contains(&mm) {
            return Err("Margins go from 0 to 2000 mm.".into());
        }
        *slot = mm * POINTS_PER_MM;
    }
    if out.iter().all(|m| *m == 0.0) {
        return Err("Enter how much to trim from at least one edge.".into());
    }
    Ok(out)
}

/// The Crop pages card.
pub fn crop_card<'a>(state: &Cropping) -> Element<'a, Message> {
    let ev = Message::Crop;
    let parsed = crop_margins(state);
    let field = |i: usize, label: &'static str| {
        let input = text_input("0", state.margins.get(i).map_or("", String::as_str))
            .on_input(move |v| ev(CropEvent::Margin(i, v)))
            .on_submit(ev(CropEvent::Go))
            .width(70);
        let input = if i == 0 { input.id(crop_input_id()) } else { input };
        row![text(label).width(60), input, text("mm").size(13)]
            .spacing(6)
            .align_y(iced::Center)
    };
    let untouched = state
        .margins
        .iter()
        .all(|m| m.trim().parse::<f32>().is_ok_and(|v| v == 0.0));
    let hint: Element<'a, Message> = match &parsed {
        Ok(_) => space().height(0).into(),
        Err(_) if untouched => space().height(0).into(),
        Err(e) => text(e.clone())
            .size(13)
            .style(|theme: &Theme| text::Style {
                color: Some(theme.extended_palette().danger.base.color),
            })
            .into(),
    };
    let all = Some(state.all);
    container(
        column![
            text("Crop pages").size(16),
            text("Trim from the edges as the page is shown.").size(13),
            row![field(0, "Top"), field(1, "Right")].spacing(16),
            row![field(2, "Bottom"), field(3, "Left")].spacing(16),
            hint,
            radio("This page", false, all, |v| ev(CropEvent::All(v))),
            radio("All pages", true, all, |v| ev(CropEvent::All(v))),
            row![
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(ev(CropEvent::Cancel)),
                button("Crop").on_press_maybe(parsed.is_ok().then_some(ev(CropEvent::Go))),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .width(380),
    )
    .padding(16)
    .style(container::rounded_box)
    .into()
}

/// The Headers and footers card, as typed.
#[derive(Debug, Clone)]
pub struct Marking {
    /// Header left, centre and right, then footer left, centre and right.
    pub text: [String; 6],
    /// Text size in points.
    pub size: String,
    /// Distance from the edges in millimetres.
    pub margin: String,
    /// The first page's number.
    pub first: String,
    pub range: String,
}

impl Marking {
    /// Page numbers in the footer's centre on every page of `count`.
    pub fn new(count: usize) -> Self {
        let mut text: [String; 6] = Default::default();
        text[4] = "Page {page} of {pages}".into();
        Self {
            text,
            size: "10".into(),
            margin: "12".into(),
            first: "1".into(),
            range: format!("1-{count}"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum MarkEvent {
    Text(usize, String),
    Size(String),
    Margin(String),
    First(String),
    Range(String),
    Go,
    Cancel,
}

pub fn mark_input_id() -> iced::widget::Id {
    iced::widget::Id::new("mark-footer-centre")
}

/// The pages to mark, each once in the order given, and what to put on them; or what's wrong.
pub fn mark_settings(
    state: &Marking,
    count: usize,
) -> Result<(Vec<usize>, panna_core::HeaderFooter), String> {
    let number = |typed: &str, what: &str, range: std::ops::RangeInclusive<f32>| {
        let typed = typed.trim().replace(',', ".");
        match typed.parse::<f32>() {
            Ok(v) if range.contains(&v) => Ok(v),
            _ => Err(format!(
                "The {what} goes from {} to {}.",
                range.start(),
                range.end()
            )),
        }
    };
    let size = number(&state.size, "text size", 4.0..=72.0)?;
    let margin = number(&state.margin, "margin", 0.0..=500.0)?;
    let first = state
        .first
        .trim()
        .parse::<u32>()
        .map_err(|_| "The first page number is a whole number, such as 1.".to_owned())?;
    let mut seen = std::collections::HashSet::new();
    let pages: Vec<usize> = parse_range(&state.range, count)?
        .into_iter()
        .filter(|p| seen.insert(*p))
        .collect();
    let marks = panna_core::HeaderFooter {
        text: state.text.clone(),
        size,
        margin: margin * POINTS_PER_MM,
        first,
    };
    if marks.is_empty() {
        return Err("Enter text for the header or the footer.".into());
    }
    Ok((pages, marks))
}

/// The Headers and footers card for a document of `count` pages.
pub fn mark_card<'a>(state: &Marking, count: usize) -> Element<'a, Message> {
    let ev = Message::Mark;
    let parsed = mark_settings(state, count);
    let line = |label: &'static str, from: usize| {
        let field = |i: usize, placeholder: &'static str| {
            let input = text_input(placeholder, state.text.get(i).map_or("", String::as_str))
                .on_input(move |v| ev(MarkEvent::Text(i, v)))
                .on_submit(ev(MarkEvent::Go))
                .width(160);
            if i == 4 { input.id(mark_input_id()) } else { input }
        };
        row![
            text(label).width(60),
            field(from, "Left"),
            field(from + 1, "Centre"),
            field(from + 2, "Right"),
        ]
        .spacing(6)
        .align_y(iced::Center)
    };
    let small = |label: &'static str, value: &str, unit: &'static str, on: fn(String) -> MarkEvent| {
        row![
            text(label),
            text_input("", value)
                .on_input(move |v| ev(on(v)))
                .on_submit(ev(MarkEvent::Go))
                .width(56),
            text(unit).size(13),
        ]
        .spacing(6)
        .align_y(iced::Center)
    };
    // What the first page gets, from the first field that has anything in it.
    let example = match &parsed {
        Ok((pages, marks)) => {
            let first = marks.text.iter().find(|t| !t.trim().is_empty());
            text(first.map_or(String::new(), |t| {
                let filled = panna_core::fill_header_footer(t, u64::from(marks.first), pages.len(), "today");
                format!("On the first page: {filled}")
            }))
            .size(13)
        }
        Err(e) => text(e.clone()).size(13).style(|theme: &Theme| text::Style {
            color: Some(theme.extended_palette().danger.base.color),
        }),
    };
    container(
        column![
            text("Page numbers, headers and footers").size(16),
            line("Header", 0),
            line("Footer", 3),
            text("{page} is the page number and {pages} the number of pages; {page:6} pads it to 6 digits, for Bates numbers. {date} is today's date.")
                .size(13),
            row![
                small("Size", &state.size, "pt", MarkEvent::Size),
                small("Margin", &state.margin, "mm", MarkEvent::Margin),
                small("First number", &state.first, "", MarkEvent::First),
            ]
            .spacing(16),
            row![
                text("Pages").width(60),
                text_input(&format!("Such as 1-3, 5 (1 to {count})"), &state.range)
                    .on_input(move |v| ev(MarkEvent::Range(v)))
                    .on_submit(ev(MarkEvent::Go)),
            ]
            .spacing(6)
            .align_y(iced::Center),
            example,
            row![
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(ev(MarkEvent::Cancel)),
                button("Add").on_press_maybe(parsed.is_ok().then_some(ev(MarkEvent::Go))),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .width(580),
    )
    .padding(16)
    .style(container::rounded_box)
    .into()
}

/// The Flatten card: what to draw into the pages.
#[derive(Debug, Clone, Copy)]
pub struct Flattening {
    pub comments: bool,
    pub fields: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum FlattenEvent {
    Comments(bool),
    Fields(bool),
    Go,
    Cancel,
}

/// The Flatten card.
pub fn flatten_card<'a>(state: Flattening) -> Element<'a, Message> {
    let ev = Message::Flatten;
    let any = state.comments || state.fields;
    container(
        column![
            text("Flatten into the pages").size(16),
            text(
                "They become part of the pages: they look the same in every app and when \
                 printed, but can't be changed any more. Undo brings them back until you save."
            )
            .size(13),
            checkbox(state.comments)
                .label("Comments, highlights, drawings and stamps")
                .on_toggle(move |v| ev(FlattenEvent::Comments(v))),
            checkbox(state.fields)
                .label("Form fields, with what's filled in")
                .on_toggle(move |v| ev(FlattenEvent::Fields(v))),
            row![
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(ev(FlattenEvent::Cancel)),
                button("Flatten").on_press_maybe(any.then_some(ev(FlattenEvent::Go))),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .width(380),
    )
    .padding(16)
    .style(container::rounded_box)
    .into()
}

/// Replacing pages with another file's: the pages here, from 1, and the other file's page
/// they start from, as typed.
#[derive(Debug, Clone)]
pub struct Replacing {
    pub path: std::path::PathBuf,
    pub first: String,
    pub last: String,
    pub from: String,
    /// Pages in this document.
    pub count: usize,
}

#[derive(Debug, Clone)]
pub enum ReplaceEvent {
    First(String),
    Last(String),
    From(String),
    Go,
    Cancel,
}

impl Replacing {
    pub fn new(path: std::path::PathBuf, current: usize, count: usize) -> Self {
        let page = (current + 1).to_string();
        Self {
            path,
            first: page.clone(),
            last: page,
            from: "1".into(),
            count,
        }
    }

    /// The first page replaced and how many, from 0, and the other file's page they start
    /// from, from 0; `None` while what's typed isn't a range of this document's pages.
    pub fn range(&self) -> Option<(usize, usize, usize)> {
        let number = |s: &str| s.trim().parse::<usize>().ok().filter(|n| *n >= 1);
        let (first, last, from) = (number(&self.first)?, number(&self.last)?, number(&self.from)?);
        (first <= last && last <= self.count).then(|| (first - 1, last - first + 1, from - 1))
    }
}

pub fn replace_input_id() -> iced::widget::Id {
    iced::widget::Id::new("replace-first")
}

/// The Replace pages card.
pub fn replace_card(state: &Replacing) -> Element<'_, Message> {
    let ev = Message::Replace;
    let name = state
        .path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let field = |value: &str, on: fn(String) -> ReplaceEvent| {
        text_input("", value)
            .on_input(move |s| ev(on(s)))
            .on_submit(ev(ReplaceEvent::Go))
            .width(60)
    };
    let ok = state.range().is_some();
    container(
        column![
            text("Replace pages").size(16),
            row![
                text("Pages"),
                field(&state.first, ReplaceEvent::First).id(replace_input_id()),
                text("to"),
                field(&state.last, ReplaceEvent::Last),
                text(format!("of {}", state.count)),
            ]
            .spacing(8)
            .align_y(iced::Center),
            row![
                text(format!("with pages from “{name}” starting at page")),
                field(&state.from, ReplaceEvent::From),
            ]
            .spacing(8)
            .align_y(iced::Center),
            text(
                "Bookmarks and links to the replaced pages go to the new ones. Undo brings \
                 the old pages back until you save."
            )
            .size(13)
            .style(text::secondary),
            row![
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(ev(ReplaceEvent::Cancel)),
                button("Replace").on_press_maybe(ok.then_some(ev(ReplaceEvent::Go))),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .width(420),
    )
    .padding(16)
    .style(container::rounded_box)
    .into()
}

/// Watermark text colours: grey, red, blue, green, black.
pub const INKS: [panna_core::Color; 5] = [
    panna_core::Color([0.5, 0.5, 0.5]),
    panna_core::Color([0.86, 0.15, 0.15]),
    panna_core::Color([0.12, 0.44, 0.92]),
    panna_core::Color([0.18, 0.63, 0.26]),
    panna_core::Color([0.0, 0.0, 0.0]),
];
const INK_NAMES: [&str; 5] = ["Grey", "Red", "Blue", "Green", "Black"];

/// Page colours: cream, pale yellow, pale blue, pale green, pale grey.
pub const PAPERS: [panna_core::Color; 5] = [
    panna_core::Color([1.0, 0.98, 0.92]),
    panna_core::Color([1.0, 0.97, 0.75]),
    panna_core::Color([0.88, 0.94, 1.0]),
    panna_core::Color([0.9, 0.97, 0.9]),
    panna_core::Color([0.93, 0.93, 0.93]),
];
const PAPER_NAMES: [&str; 5] = ["Cream", "Pale yellow", "Pale blue", "Pale green", "Pale grey"];

/// The Watermark card, as chosen so far.
#[derive(Debug, Clone)]
pub struct Watermarking {
    pub text: String,
    /// Which of [`INKS`].
    pub ink: usize,
    /// Points, or empty to fit the page.
    pub size: String,
    /// Degrees anticlockwise.
    pub angle: String,
    pub opacity: f32,
    pub behind: bool,
    /// Which of [`PAPERS`], if any.
    pub paper: Option<usize>,
    pub range: String,
    /// A picture to use instead of text, with its file's name.
    pub picture: Option<(String, panna_core::StampImage)>,
    /// The picture's width, as a share of the page's.
    pub scale: f32,
}

impl Watermarking {
    pub fn new(count: usize) -> Self {
        Self {
            text: String::new(),
            ink: 0,
            size: String::new(),
            angle: "45".into(),
            opacity: 0.3,
            behind: false,
            paper: None,
            range: format!("1-{count}"),
            picture: None,
            scale: 0.5,
        }
    }
}

#[derive(Debug, Clone)]
pub enum WatermarkEvent {
    Text(String),
    Ink(usize),
    Size(String),
    Angle(String),
    Opacity(f32),
    Behind(bool),
    /// Turns the page colour on or off.
    UsePaper(bool),
    Paper(usize),
    Range(String),
    ChoosePicture,
    PictureChosen(Option<std::path::PathBuf>),
    PictureLoaded(Result<(String, panna_core::StampImage), String>),
    RemovePicture,
    Scale(f32),
    Go,
    Cancel,
}

pub fn watermark_input_id() -> iced::widget::Id {
    iced::widget::Id::new("watermark-text")
}

/// The pages to mark, each once, and the watermark; or what's wrong.
pub fn watermark_settings(
    state: &Watermarking,
    count: usize,
) -> Result<(Vec<usize>, panna_core::Watermark), String> {
    let size = match state.size.trim().replace(',', ".") {
        s if s.is_empty() => 0.0,
        s => match s.parse::<f32>() {
            Ok(v) if (4.0..=500.0).contains(&v) => v,
            _ => {
                return Err("The text size goes from 4 to 500 pt, or leave it empty to fit the page.".into());
            }
        },
    };
    let angle = match state.angle.trim().replace(',', ".") {
        s if s.is_empty() => 0.0,
        s => match s.parse::<f32>() {
            Ok(v) if (-360.0..=360.0).contains(&v) => v,
            _ => return Err("The angle goes from -360 to 360 degrees.".into()),
        },
    };
    let mut seen = std::collections::HashSet::new();
    let pages: Vec<usize> = parse_range(&state.range, count)?
        .into_iter()
        .filter(|p| seen.insert(*p))
        .collect();
    let mark = panna_core::Watermark {
        text: state.text.clone(),
        size,
        angle,
        opacity: state.opacity,
        color: INKS.get(state.ink).copied().unwrap_or(INKS[0]),
        behind: state.behind,
        background: state.paper.and_then(|i| PAPERS.get(i).copied()),
        picture: state.picture.as_ref().map(|(_, image)| image.clone()),
        picture_scale: state.scale,
    };
    if mark.is_empty() {
        return Err("Enter the watermark's text, choose a picture, or choose a page colour.".into());
    }
    Ok((pages, mark))
}

/// A row of round colour buttons, with `picked` outlined.
fn swatches<'a>(
    colors: &[panna_core::Color],
    names: &[&'static str],
    picked: Option<usize>,
    on: impl Fn(usize) -> Message,
) -> Element<'a, Message> {
    let mut out = iced::widget::Row::new().spacing(6).align_y(iced::Center);
    for (i, panna_core::Color([r, g, b])) in colors.iter().enumerate() {
        let c = iced::Color::from_rgb(*r, *g, *b);
        let selected = picked == Some(i);
        let swatch = button(space().width(18).height(18))
            .padding(0)
            .style(move |theme: &Theme, _| button::Style {
                background: Some(c.into()),
                border: iced::Border {
                    color: if selected {
                        theme.extended_palette().primary.strong.color
                    } else {
                        theme.extended_palette().background.strong.color
                    },
                    width: if selected { 3.0 } else { 1.0 },
                    radius: 10.0.into(),
                },
                ..button::Style::default()
            })
            .on_press(on(i));
        out = out.push(icons::with_tip(swatch, names.get(i).copied().unwrap_or("")));
    }
    out.into()
}

/// The Watermark card for a document of `count` pages.
pub fn watermark_card<'a>(state: &Watermarking, count: usize) -> Element<'a, Message> {
    let ev = Message::Watermark;
    let parsed = watermark_settings(state, count);
    let untouched = state.text.trim().is_empty() && state.paper.is_none() && state.picture.is_none();
    let hint: Element<'a, Message> = match &parsed {
        Err(e) if !untouched => text(e.clone())
            .size(13)
            .style(|theme: &Theme| text::Style {
                color: Some(theme.extended_palette().danger.base.color),
            })
            .into(),
        _ => space().height(0).into(),
    };
    let behind = Some(state.behind);
    let small = |label: &'static str,
                 placeholder: &'static str,
                 value: &str,
                 unit: &'static str,
                 on: fn(String) -> WatermarkEvent| {
        row![
            text(label),
            text_input(placeholder, value)
                .on_input(move |v| ev(on(v)))
                .on_submit(ev(WatermarkEvent::Go))
                .width(56),
            text(unit).size(13),
        ]
        .spacing(6)
        .align_y(iced::Center)
    };
    // A picture takes the place of the text, its colour and size.
    let what: Element<'a, Message> = match &state.picture {
        Some((name, _)) => column![
            row![
                text(format!("Picture: {name}")).size(13).width(iced::Fill),
                button("Remove")
                    .style(button::secondary)
                    .on_press(ev(WatermarkEvent::RemovePicture)),
            ]
            .spacing(8)
            .align_y(iced::Center),
            row![
                text("Width").width(60),
                slider(0.05..=1.0, state.scale, move |v| ev(WatermarkEvent::Scale(v))).step(0.05_f32),
                text(format!("{}%", (state.scale * 100.0).round())).width(44),
            ]
            .spacing(6)
            .align_y(iced::Center),
            small("Angle", "0", &state.angle, "°", WatermarkEvent::Angle),
        ]
        .spacing(10)
        .into(),
        None => column![
            row![
                text_input("Text, such as DRAFT or CONFIDENTIAL", &state.text)
                    .id(watermark_input_id())
                    .on_input(move |v| ev(WatermarkEvent::Text(v)))
                    .on_submit(ev(WatermarkEvent::Go)),
                button("Picture…")
                    .style(button::secondary)
                    .on_press(ev(WatermarkEvent::ChoosePicture)),
            ]
            .spacing(8)
            .align_y(iced::Center),
            row![
                text("Colour").width(60),
                swatches(&INKS, &INK_NAMES, Some(state.ink), move |i| ev(
                    WatermarkEvent::Ink(i)
                )),
            ]
            .spacing(6)
            .align_y(iced::Center),
            row![
                small("Size", "Fit", &state.size, "pt", WatermarkEvent::Size),
                small("Angle", "0", &state.angle, "°", WatermarkEvent::Angle),
            ]
            .spacing(16),
        ]
        .spacing(10)
        .into(),
    };
    container(
        column![
            text("Watermark and page colour").size(16),
            what,
            row![
                text("Opacity").width(60),
                slider(0.05..=1.0, state.opacity, move |v| ev(WatermarkEvent::Opacity(v))).step(0.05_f32),
                text(format!("{}%", (state.opacity * 100.0).round())).width(44),
            ]
            .spacing(6)
            .align_y(iced::Center),
            radio("Over the page", false, behind, move |v| ev(
                WatermarkEvent::Behind(v)
            )),
            radio("Behind the page's text and pictures", true, behind, move |v| ev(
                WatermarkEvent::Behind(v)
            )),
            row![
                checkbox(state.paper.is_some())
                    .label("Page colour")
                    .on_toggle(move |v| ev(WatermarkEvent::UsePaper(v))),
                swatches(&PAPERS, &PAPER_NAMES, state.paper, move |i| ev(
                    WatermarkEvent::Paper(i)
                )),
            ]
            .spacing(12)
            .align_y(iced::Center),
            row![
                text("Pages").width(60),
                text_input(&format!("Such as 1-3, 5 (1 to {count})"), &state.range)
                    .on_input(move |v| ev(WatermarkEvent::Range(v)))
                    .on_submit(ev(WatermarkEvent::Go)),
            ]
            .spacing(6)
            .align_y(iced::Center),
            hint,
            row![
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(ev(WatermarkEvent::Cancel)),
                button("Add").on_press_maybe(parsed.is_ok().then_some(ev(WatermarkEvent::Go))),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .width(420),
    )
    .padding(16)
    .style(container::rounded_box)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: PageSize = PageSize {
        width: 100.0,
        height: 200.0,
    };

    fn indices(plan: &[PageSource]) -> Vec<Option<usize>> {
        plan.iter()
            .map(|p| match p {
                PageSource::Page { index, .. } => Some(*index),
                PageSource::Blank { .. } => None,
            })
            .collect()
    }

    #[test]
    fn plans_for_each_tool() {
        let (p, show) = plan(Op::MoveUp, 3, 2, SIZE).unwrap();
        assert_eq!((indices(&p), show), (vec![Some(0), Some(2), Some(1)], 1));
        let (p, show) = plan(Op::MoveDown, 3, 0, SIZE).unwrap();
        assert_eq!((indices(&p), show), (vec![Some(1), Some(0), Some(2)], 1));
        let (p, show) = plan(Op::Delete, 3, 2, SIZE).unwrap();
        assert_eq!((indices(&p), show), (vec![Some(0), Some(1)], 1));
        let (p, show) = plan(Op::InsertBlank, 2, 0, SIZE).unwrap();
        assert_eq!((indices(&p), show), (vec![Some(0), None, Some(1)], 1));
        let (p, _) = plan(Op::RotateCcw, 2, 1, SIZE).unwrap();
        assert!(matches!(p[1], PageSource::Page { turns: 3, .. }));
    }

    #[test]
    fn crop_margins_in_millimetres() {
        let mut state = Cropping::default();
        assert!(crop_margins(&state).is_err());
        state.margins[0] = "25,4".into();
        state.margins[3] = " ".into();
        let m = crop_margins(&state).unwrap();
        assert!((m[0] - 72.0).abs() < 0.01 && m[3] == 0.0);
        state.margins[1] = "-1".into();
        assert!(crop_margins(&state).is_err());
        state.margins[1] = "lots".into();
        assert!(crop_margins(&state).is_err());
    }

    #[test]
    fn page_ranges() {
        assert_eq!(parse_range("1-3, 5", 6), Ok(vec![0, 1, 2, 4]));
        assert_eq!(parse_range(" 5- ;-2", 6), Ok(vec![4, 5, 0, 1]));
        assert_eq!(parse_range("3–1", 6), Ok(vec![2, 1, 0]));
        assert_eq!(parse_range("4", 6), Ok(vec![3]));
        assert!(parse_range("0", 6).is_err());
        assert!(parse_range("7", 6).is_err());
        assert!(parse_range("two", 6).is_err());
        assert!(parse_range(" , ", 6).is_err());
    }

    #[test]
    fn tools_that_dont_apply_give_no_plan() {
        assert!(plan(Op::MoveUp, 3, 0, SIZE).is_none());
        assert!(plan(Op::MoveDown, 3, 2, SIZE).is_none());
        assert!(plan(Op::Delete, 1, 0, SIZE).is_none());
        assert!(plan(Op::Extract, 3, 0, SIZE).is_none());
        assert!(plan(Op::RotateCw, 3, 5, SIZE).is_none());
    }
}
