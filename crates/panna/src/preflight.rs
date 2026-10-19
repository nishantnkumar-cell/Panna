//! The reports of the PDF/A and accessibility checks: what keeps the document from being
//! archival PDF, or from working well with screen readers, grouped by rule, each with the
//! page it's on.

use iced::widget::{Column, button, column, container, row, scrollable, space, text};
use iced::{Element, Fill, Theme};
use panna_core::{AccessFinding, Finding};

use crate::Message;

#[derive(Debug, Clone)]
pub enum Event {
    Run,
    /// Run the accessibility check.
    RunAccessibility,
    /// Save a PDF/A copy.
    Convert,
    /// Close the report and show this page.
    Show(usize),
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    PdfA,
    Accessibility,
}

/// One finding, under its rule's title.
#[derive(Debug)]
struct Item {
    rule: &'static str,
    detail: String,
    page: Option<usize>,
}

#[derive(Debug)]
pub struct Report {
    kind: Kind,
    items: Vec<Item>,
    /// The PDF/A copy these findings are about, when one was just saved.
    saved: Option<String>,
}

impl Report {
    pub fn new(findings: Vec<Finding>) -> Self {
        Self {
            kind: Kind::PdfA,
            items: findings
                .into_iter()
                .map(|f| Item {
                    rule: f.rule.title(),
                    detail: f.detail,
                    page: f.page,
                })
                .collect(),
            saved: None,
        }
    }

    /// The findings on a PDF/A copy just saved as `name`.
    pub fn saved(name: String, findings: Vec<Finding>) -> Self {
        Self {
            saved: Some(name),
            ..Self::new(findings)
        }
    }

    pub fn accessibility(findings: Vec<AccessFinding>) -> Self {
        Self {
            kind: Kind::Accessibility,
            items: findings
                .into_iter()
                .map(|f| Item {
                    rule: f.rule.title(),
                    detail: f.detail,
                    page: f.page,
                })
                .collect(),
            saved: None,
        }
    }

    pub fn card(&self) -> Element<'_, Message> {
        let close = Message::Preflight(Event::Close);
        let clean = self.items.is_empty();
        let (title, summary) = match (self.kind, &self.saved) {
            (Kind::PdfA, Some(name)) if clean => (
                format!("Saved {name}"),
                "The copy meets PDF/A-2b's main rules. Not every rule of the standard is checked.",
            ),
            (Kind::PdfA, Some(name)) => (
                format!("Saved {name}"),
                "The copy is closer to PDF/A, but these couldn't be fixed by Panna:",
            ),
            (Kind::PdfA, None) if clean => (
                "Meets PDF/A-2b's main rules".to_owned(),
                "Fonts are embedded, colours are tied to an output profile, the metadata says \
                 it's PDF/A, and nothing runs or comes from outside the file. Not every rule \
                 of the standard is checked.",
            ),
            (Kind::PdfA, None) => (
                "Not yet PDF/A".to_owned(),
                "These keep the document from being archival PDF (PDF/A-2b):",
            ),
            (Kind::Accessibility, _) if clean => (
                "No accessibility problems found".to_owned(),
                "The document is tagged, has a language and a title, and its figures, tables \
                 and fields are described. Whether the reading order and descriptions make \
                 sense still needs a person to check.",
            ),
            (Kind::Accessibility, _) => (
                "Accessibility problems".to_owned(),
                "These make the document harder to use with a screen reader or other \
                 assistive technology:",
            ),
        };
        let mut list = Column::new().spacing(10);
        let mut rules: Vec<&str> = self.items.iter().map(|f| f.rule).collect();
        rules.dedup();
        // Tall enough for the list, up to a point; beyond it, it scrolls.
        let height = (rules.len() as f32 * 30.0 + self.items.len() as f32 * 20.0).min(360.0);
        for rule in rules {
            let mut group = Column::new().spacing(2).push(text(rule).size(14));
            for item in self.items.iter().filter(|f| f.rule == rule) {
                let detail = text(item.detail.as_str()).size(12).style(text::secondary);
                let line: Element<'_, Message> = match item.page {
                    Some(page) => row![
                        detail.width(Fill),
                        button(text(format!("Page {}", page + 1)).size(12))
                            .padding([1, 6])
                            .style(button::text)
                            .on_press(Message::Preflight(Event::Show(page))),
                    ]
                    .spacing(6)
                    .into(),
                    None => detail.into(),
                };
                group = group.push(line);
            }
            list = list.push(group);
        }
        let mut buttons = row![space::horizontal()].spacing(8);
        if self.kind == Kind::PdfA {
            buttons = buttons.push(
                button("Save a PDF/A copy…")
                    .style(button::secondary)
                    .on_press_maybe(
                        (self.saved.is_none() && !clean).then_some(Message::Preflight(Event::Convert)),
                    ),
            );
        }
        let body = column![
            text(title).size(16),
            text(summary).size(13),
            scrollable(list).height(height),
            buttons.push(button("Close").on_press(close)),
        ]
        .spacing(10);
        container(container(body).width(420))
            .padding(16)
            .style(|theme: &Theme| container::rounded_box(theme))
            .into()
    }
}
