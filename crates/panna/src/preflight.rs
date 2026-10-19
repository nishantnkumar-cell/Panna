//! The PDF/A check's report: what keeps the document from being archival PDF, grouped by
//! rule, each with the page it's on.

use iced::widget::{Column, button, column, container, row, scrollable, space, text};
use iced::{Element, Fill, Theme};
use panna_core::{Finding, Rule};

use crate::Message;

#[derive(Debug, Clone)]
pub enum Event {
    Run,
    /// Save a PDF/A copy.
    Convert,
    /// Close the report and show this page.
    Show(usize),
    Close,
}

#[derive(Debug)]
pub struct Report {
    findings: Vec<Finding>,
    /// The PDF/A copy these findings are about, when one was just saved.
    saved: Option<String>,
}

impl Report {
    pub fn new(findings: Vec<Finding>) -> Self {
        Self {
            findings,
            saved: None,
        }
    }

    /// The findings on a PDF/A copy just saved as `name`.
    pub fn saved(name: String, findings: Vec<Finding>) -> Self {
        Self {
            findings,
            saved: Some(name),
        }
    }

    pub fn card(&self) -> Element<'_, Message> {
        let close = Message::Preflight(Event::Close);
        let (title, summary) = if let Some(name) = &self.saved {
            let title = format!("Saved {name}");
            if self.findings.is_empty() {
                (
                    title,
                    "The copy meets PDF/A-2b's main rules. Not every rule of the standard is \
                     checked.",
                )
            } else {
                (
                    title,
                    "The copy is closer to PDF/A, but these couldn't be fixed by Panna:",
                )
            }
        } else if self.findings.is_empty() {
            (
                "Meets PDF/A-2b's main rules".to_owned(),
                "Fonts are embedded, colours are tied to an output profile, the metadata says \
                 it's PDF/A, and nothing runs or comes from outside the file. Not every rule \
                 of the standard is checked.",
            )
        } else {
            (
                "Not yet PDF/A".to_owned(),
                "These keep the document from being archival PDF (PDF/A-2b):",
            )
        };
        let mut list = Column::new().spacing(10);
        let mut rules: Vec<Rule> = self.findings.iter().map(|f| f.rule).collect();
        rules.dedup();
        // Tall enough for the list, up to a point; beyond it, it scrolls.
        let height = (rules.len() as f32 * 30.0 + self.findings.len() as f32 * 20.0).min(360.0);
        for rule in rules {
            let mut group = Column::new().spacing(2).push(text(rule.title()).size(14));
            for finding in self.findings.iter().filter(|f| f.rule == rule) {
                let detail = text(finding.detail.as_str()).size(12).style(text::secondary);
                let line: Element<'_, Message> = match finding.page {
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
        let body = column![
            text(title).size(16),
            text(summary).size(13),
            scrollable(list).height(height),
            row![
                space::horizontal(),
                button("Save a PDF/A copy…")
                    .style(button::secondary)
                    .on_press_maybe(
                        (self.saved.is_none() && !self.findings.is_empty())
                            .then_some(Message::Preflight(Event::Convert))
                    ),
                button("Close").on_press(close),
            ]
            .spacing(8),
        ]
        .spacing(10);
        container(container(body).width(420))
            .padding(16)
            .style(|theme: &Theme| container::rounded_box(theme))
            .into()
    }
}
