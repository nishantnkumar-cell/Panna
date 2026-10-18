//! The All tools card: the document tools that don't need a place on the toolbar, each with
//! its icon and name.

use iced::widget::{button, column, container, row, space, text};
use iced::{Element, Theme};

use crate::icons::{self, Icon};
use crate::{Message, export, protect};

/// The All tools card. Each tool closes the card and opens its own.
pub fn card<'a>(has_form: bool, has_doc: bool) -> Element<'a, Message> {
    let tool = |icon: Icon, label: &'static str, message: Message| {
        button(
            row![icons::glyph(icon, true), text(label)]
                .spacing(10)
                .align_y(iced::Center),
        )
        .width(iced::Fill)
        .padding([6, 8])
        .style(|theme: &Theme, status| match status {
            button::Status::Hovered | button::Status::Pressed => button::secondary(theme, status),
            _ => button::text(theme, status),
        })
        .on_press(message)
    };
    container(
        column![
            text("All tools").size(16),
            tool(
                Icon::FromPictures,
                "Make a PDF from pictures…",
                Message::FromPictures
            ),
            tool(
                Icon::Export,
                "Export as text or pictures…",
                Message::Export(export::Event::Open)
            ),
        ]
        .push(has_doc.then(|| {
            column![
                tool(Icon::Shrink, "Save a smaller copy…", Message::Compact),
                tool(
                    Icon::EditPage,
                    "Add text, pictures or links to pages",
                    Message::Add(crate::add::Event::Toggle)
                ),
                tool(
                    Icon::CommentsImport,
                    "Add comments from a file…",
                    Message::ImportComments
                ),
                tool(
                    Icon::CommentsExport,
                    "Save comments to a file…",
                    Message::ExportComments
                ),
            ]
            .spacing(4)
        }))
        .push(has_form.then(|| {
            column![
                tool(
                    Icon::DataImport,
                    "Fill in the form from a data file…",
                    Message::ImportFormData
                ),
                tool(
                    Icon::DataExport,
                    "Save form data (FDF, XFDF)…",
                    Message::ExportFormData
                ),
            ]
            .spacing(4)
        }))
        .push(
            column![
                tool(
                    Icon::Protect,
                    "Password protection…",
                    Message::Protect(protect::Event::Open)
                ),
                tool(
                    Icon::Clean,
                    "Remove hidden information…",
                    Message::Clean(protect::CleanEvent::Open)
                ),
                row![
                    space::horizontal(),
                    button("Close")
                        .style(button::secondary)
                        .on_press(Message::AllTools(false)),
                ],
            ]
            .spacing(4),
        )
        .spacing(4)
        .width(340),
    )
    .padding(16)
    .style(container::rounded_box)
    .into()
}
