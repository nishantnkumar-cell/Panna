//! The Export card: pages as plain text, a Word document or pictures.

use iced::widget::{button, column, container, radio, row, space, text, text_input};
use iced::{Element, Theme};
use panna_core::ExportFormat;

use crate::Message;
use crate::pages::parse_range;

/// The Export card, as chosen so far.
#[derive(Debug, Clone)]
pub struct Exporting {
    pub format: ExportFormat,
    /// Dots per inch, as typed.
    pub dpi: String,
    pub range: String,
}

impl Exporting {
    pub fn new(count: usize) -> Self {
        Self {
            format: ExportFormat::Text,
            dpi: "150".into(),
            range: format!("1-{count}"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    /// Shows the card.
    Open,
    Format(ExportFormat),
    Dpi(String),
    Range(String),
    Go,
    Cancel,
}

pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::new("export-range")
}

/// The pages to export, each once in the order given, and the resolution; or what's wrong.
pub fn settings(state: &Exporting, count: usize) -> Result<(Vec<usize>, f32), String> {
    let dpi = match state.dpi.trim().parse::<f32>() {
        Ok(v) if (36.0..=1200.0).contains(&v) => v,
        _ if !state.format.pictures() => 0.0,
        _ => return Err("The resolution goes from 36 to 1200 dots per inch.".into()),
    };
    let mut seen = std::collections::HashSet::new();
    let pages = parse_range(&state.range, count)?
        .into_iter()
        .filter(|p| seen.insert(*p))
        .collect();
    Ok((pages, dpi))
}

/// The file name extension for `format`.
pub fn extension(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Text => "txt",
        ExportFormat::Png => "png",
        ExportFormat::Jpeg => "jpg",
        ExportFormat::Word => "docx",
        ExportFormat::PowerPoint => "pptx",
        ExportFormat::Excel => "xlsx",
    }
}

/// The Export card for a document of `count` pages.
pub fn card<'a>(state: &Exporting, count: usize) -> Element<'a, Message> {
    let ev = Message::Export;
    let parsed = settings(state, count);
    let hint = match &parsed {
        Ok((pages, _)) if state.format == ExportFormat::Text => {
            text(format!("{} into one text file", plural(pages.len()))).size(13)
        }
        Ok((pages, _)) if state.format == ExportFormat::Word => text(format!(
            "{} into one Word document: the text, in paragraphs and headings. Pictures and \
             layout aren't kept.",
            plural(pages.len())
        ))
        .size(13),
        Ok((pages, _)) if state.format == ExportFormat::PowerPoint => text(format!(
            "{} into one PowerPoint presentation, a picture of each page as a slide",
            plural(pages.len())
        ))
        .size(13),
        Ok((pages, _)) if state.format == ExportFormat::Excel => text(format!(
            "{} into one Excel workbook: a sheet for each page, a row for each line, and columns \
             where the words line up",
            plural(pages.len())
        ))
        .size(13),
        Ok((pages, _)) => text(format!("{}, a picture each", plural(pages.len()))).size(13),
        Err(e) => text(e.clone()).size(13).style(|theme: &Theme| text::Style {
            color: Some(theme.extended_palette().danger.base.color),
        }),
    };
    let format = Some(state.format);
    let pictures = state.format.pictures();
    let dpi: Element<'a, Message> = if pictures {
        row![
            text("Resolution").width(80),
            text_input("150", &state.dpi)
                .on_input(move |v| ev(Event::Dpi(v)))
                .on_submit(ev(Event::Go))
                .width(64),
            text("dots per inch").size(13),
        ]
        .spacing(6)
        .align_y(iced::Center)
        .into()
    } else {
        space().height(0).into()
    };
    container(
        column![
            text("Export pages").size(16),
            radio("Plain text", ExportFormat::Text, format, move |v| ev(
                Event::Format(v)
            )),
            radio("Word document (.docx)", ExportFormat::Word, format, move |v| ev(
                Event::Format(v)
            )),
            radio(
                "PowerPoint presentation (.pptx)",
                ExportFormat::PowerPoint,
                format,
                move |v| ev(Event::Format(v))
            ),
            radio(
                "Excel workbook (.xlsx)",
                ExportFormat::Excel,
                format,
                move |v| ev(Event::Format(v))
            ),
            radio(
                "PNG pictures (sharp, larger)",
                ExportFormat::Png,
                format,
                move |v| ev(Event::Format(v))
            ),
            radio(
                "JPEG pictures (photos, smaller)",
                ExportFormat::Jpeg,
                format,
                move |v| ev(Event::Format(v))
            ),
            dpi,
            row![
                text("Pages").width(80),
                text_input(&format!("Such as 1-3, 5 (1 to {count})"), &state.range)
                    .id(input_id())
                    .on_input(move |v| ev(Event::Range(v)))
                    .on_submit(ev(Event::Go)),
            ]
            .spacing(6)
            .align_y(iced::Center),
            hint,
            row![
                space::horizontal(),
                button("Cancel")
                    .style(button::secondary)
                    .on_press(ev(Event::Cancel)),
                button("Export…").on_press_maybe(parsed.is_ok().then_some(ev(Event::Go))),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .width(360),
    )
    .padding(16)
    .style(container::rounded_box)
    .into()
}

fn plural(n: usize) -> String {
    if n == 1 {
        "1 page".into()
    } else {
        format!("{n} pages")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_only_matters_for_pictures() {
        let mut state = Exporting::new(5);
        state.dpi = "nonsense".into();
        assert_eq!(settings(&state, 5), Ok(((0..5).collect(), 0.0)));
        state.format = ExportFormat::Png;
        assert!(settings(&state, 5).is_err());
        state.dpi = "300".into();
        state.range = "3, 1-3".into();
        assert_eq!(settings(&state, 5), Ok((vec![2, 0, 1], 300.0)));
    }
}
