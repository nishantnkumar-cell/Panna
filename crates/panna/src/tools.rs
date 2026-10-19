//! The All tools card: the document tools that don't need a place on the toolbar, each with
//! its icon and name.

use iced::widget::{button, column, container, row, space, text};
use iced::{Element, Theme};

use crate::icons::{self, Icon};
use crate::{Message, export, protect};

/// The All tools card. Each tool closes the card and opens its own.
pub fn card<'a>(has_form: bool, has_doc: bool) -> Element<'a, Message> {
    let tool = |icon: Icon, label: &'static str, message: Message| -> Element<'a, Message> {
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
        .into()
    };
    let mut tools: Vec<Element<'a, Message>> = vec![
        tool(
            Icon::NewBlank,
            "New blank PDF…",
            Message::Blank(crate::create::Event::Open),
        ),
        tool(
            Icon::FromPictures,
            "Make a PDF from pictures…",
            Message::FromPictures,
        ),
        tool(
            Icon::FromClipboard,
            "Make a PDF from the picture on the clipboard…",
            Message::FromClipboard,
        ),
        tool(
            Icon::Scanner,
            "Scan pages from a scanner…",
            Message::Scan(crate::scan::Event::Open),
        ),
        tool(
            Icon::Export,
            "Export as text, Office files or pictures…",
            Message::Export(export::Event::Open),
        ),
    ];
    if has_doc {
        tools.extend([
            tool(
                Icon::Shrink,
                "Save a smaller copy…",
                Message::Shrink(crate::shrink::Event::Open),
            ),
            tool(
                Icon::ScanText,
                "Recognise text in scanned pages (OCR)",
                Message::Recognise,
            ),
            tool(
                Icon::ReadAloud,
                "Read this page aloud",
                Message::ReadAloud(crate::speech::Event::Page),
            ),
            tool(
                Icon::ReadAloud,
                "Read aloud to the end",
                Message::ReadAloud(crate::speech::Event::ToEnd),
            ),
            tool(
                Icon::EditPage,
                "Add text, pictures or links to pages",
                Message::Add(crate::add::Event::Toggle),
            ),
            tool(
                Icon::EditPictures,
                "Move, replace or delete pictures",
                Message::EditPictures(crate::pictures::Event::Toggle),
            ),
            tool(
                Icon::PrepareForm,
                "Prepare a form (add or delete fields)",
                Message::Prepare(crate::prepare::Event::Toggle),
            ),
            tool(
                Icon::Compare,
                "Compare with another file…",
                Message::Compare(crate::compare::Event::Open),
            ),
            tool(
                Icon::Preflight,
                "Check for PDF/A (archiving)",
                Message::Preflight(crate::preflight::Event::Run),
            ),
            tool(
                Icon::Print,
                "Check for PDF/X (printing)",
                Message::Preflight(crate::preflight::Event::RunPrint),
            ),
            tool(
                Icon::Signed,
                "Sign with a certificate…",
                Message::Signing(crate::signing::Event::Open),
            ),
            tool(
                Icon::Accessibility,
                "Check accessibility",
                Message::Preflight(crate::preflight::Event::RunAccessibility),
            ),
            tool(
                Icon::Redact,
                "Remove content for good (redact)",
                Message::Redact(crate::redact::Event::Toggle),
            ),
            tool(
                Icon::CommentsImport,
                "Add comments from a file…",
                Message::ImportComments,
            ),
            tool(
                Icon::CommentsExport,
                "Save comments to a file…",
                Message::ExportComments,
            ),
            tool(
                Icon::CommentSummary,
                "Summarize comments (save as a PDF)…",
                Message::SummarizeComments,
            ),
            tool(
                Icon::PrintComments,
                "Print with comments…",
                Message::PrintWithComments,
            ),
        ]);
    }
    if has_form {
        tools.extend([
            tool(
                Icon::DataImport,
                "Fill in the form from a data file…",
                Message::ImportFormData,
            ),
            tool(
                Icon::DataExport,
                "Save form data (FDF, XFDF)…",
                Message::ExportFormData,
            ),
        ]);
    }
    tools.extend([
        tool(
            Icon::Protect,
            "Password protection…",
            Message::Protect(protect::Event::Open),
        ),
        tool(
            Icon::Clean,
            "Remove hidden information…",
            Message::Clean(protect::CleanEvent::Open),
        ),
        tool(
            Icon::Preferences,
            "Preferences… (Ctrl+,)",
            Message::Preferences(crate::preferences::Event::Open),
        ),
    ]);
    // Two columns, so the card fits on a laptop screen.
    let half = tools.len().div_ceil(2);
    let right = tools.split_off(half);
    let side = |items: Vec<Element<'a, Message>>| {
        items
            .into_iter()
            .fold(column![].spacing(4).width(340), iced::widget::Column::push)
    };
    container(
        column![
            text("All tools").size(16),
            row![side(tools), side(right)].spacing(12),
            row![
                space::horizontal(),
                button("Close")
                    .style(button::secondary)
                    .on_press(Message::AllTools(false)),
            ],
        ]
        .spacing(8)
        .width(692),
    )
    .padding(16)
    .style(container::rounded_box)
    .into()
}
