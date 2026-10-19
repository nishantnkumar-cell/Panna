//! The reports of the PDF/A, PDF/X and accessibility checks: what keeps the document from being
//! archival PDF, or from working well with screen readers, grouped by rule, each with the
//! page it's on. Figures without a description can be given one there.

use std::collections::HashMap;
use std::path::PathBuf;

use iced::widget::{Column, button, column, container, row, scrollable, space, text, text_input};
use iced::{Element, Fill, Theme};
use panna_core::{AccessFinding, AccessRule, Finding};

use crate::Message;

#[derive(Debug, Clone)]
pub enum Event {
    Run,
    /// Check for PDF/X.
    RunPrint,
    /// Run the accessibility check.
    RunAccessibility,
    /// Save a PDF/A copy.
    Convert,
    /// Save a copy with tags added.
    Tag,
    /// Close the report and show this page.
    Show(usize),
    /// A figure's description typed.
    Alt((u32, u16), String),
    /// Save the descriptions typed.
    Describe,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    PdfA,
    PdfX,
    Accessibility,
}

/// One finding, under its rule's title.
#[derive(Debug)]
struct Item {
    rule: &'static str,
    detail: String,
    page: Option<usize>,
    /// A figure to describe.
    element: Option<(u32, u16)>,
}

#[derive(Debug)]
pub struct Report {
    kind: Kind,
    items: Vec<Item>,
    /// The PDF/A or tagged copy these findings are about, when one was just saved.
    saved: Option<String>,
    /// The document has no tags, which Panna can add.
    untagged: bool,
    /// Where the tagged copy just saved is, so descriptions go into it.
    pub copy: Option<PathBuf>,
    /// Descriptions typed for figures.
    pub alts: HashMap<(u32, u16), String>,
    pub busy: bool,
}

impl Report {
    pub fn new(findings: Vec<Finding>) -> Self {
        Self::of(Kind::PdfA, findings)
    }

    /// The findings of the PDF/X check.
    pub fn print(findings: Vec<Finding>) -> Self {
        Self::of(Kind::PdfX, findings)
    }

    fn of(kind: Kind, findings: Vec<Finding>) -> Self {
        Self {
            kind,
            items: findings
                .into_iter()
                .map(|f| Item {
                    rule: f.rule.title(),
                    detail: f.detail,
                    page: f.page,
                    element: None,
                })
                .collect(),
            saved: None,
            untagged: false,
            copy: None,
            alts: HashMap::new(),
            busy: false,
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
            untagged: findings.iter().any(|f| f.rule == AccessRule::NotTagged),
            items: findings
                .into_iter()
                .map(|f| Item {
                    rule: f.rule.title(),
                    detail: f.detail,
                    page: f.page,
                    element: f.element,
                })
                .collect(),
            saved: None,
            copy: None,
            alts: HashMap::new(),
            busy: false,
        }
    }

    /// The accessibility check of a tagged copy just saved as `name` at `path`.
    pub fn tagged(name: String, path: PathBuf, findings: Vec<AccessFinding>) -> Self {
        Self {
            saved: Some(name),
            copy: Some(path),
            ..Self::accessibility(findings)
        }
    }

    /// The descriptions typed, to save.
    pub fn descriptions(&self) -> Vec<((u32, u16), String)> {
        let mut alts: Vec<_> = self
            .alts
            .iter()
            .filter(|(_, alt)| !alt.trim().is_empty())
            .map(|(e, alt)| (*e, alt.clone()))
            .collect();
        alts.sort();
        alts
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
            (Kind::PdfX, _) if clean => (
                "Meets PDF/X-4's main rules".to_owned(),
                "Fonts are embedded, the printing condition is named, every page gives its \
                 trimmed size, and the metadata says it's PDF/X-4. Not every rule of the \
                 standard is checked.",
            ),
            (Kind::PdfX, _) => (
                "Not yet PDF/X".to_owned(),
                "These keep the document from being PDF/X-4, for printing. Making PDF/X needs \
                 the printer's colour profile, so Panna checks but doesn't convert:",
            ),
            (Kind::Accessibility, Some(name)) if clean => (
                format!("Saved {name}"),
                "Tags were added, and no other problems were found. Whether the reading order \
                 and headings make sense still needs a person to check.",
            ),
            (Kind::Accessibility, Some(name)) => (
                format!("Saved {name}"),
                "Tags were added: each run of text is a paragraph, or a heading where the type is \
                 larger. These still need fixing in the program that made the document:",
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
        let lines: f32 = self
            .items
            .iter()
            .map(|i| if i.element.is_some() { 32.0 } else { 20.0 })
            .sum();
        let height = (rules.len() as f32 * 30.0 + lines).min(360.0);
        for rule in rules {
            let mut group = Column::new().spacing(2).push(text(rule).size(14));
            for item in self.items.iter().filter(|f| f.rule == rule) {
                let detail = text(item.detail.as_str()).size(12).style(text::secondary);
                let detail = match item.element {
                    Some(element) => {
                        let typed = self.alts.get(&element).map_or("", String::as_str);
                        let input = text_input("Describe what it shows", typed)
                            .size(12)
                            .on_input(move |v| Message::Preflight(Event::Alt(element, v)));
                        Element::from(row![detail.width(80), input].spacing(6).align_y(iced::Center))
                    }
                    None => detail.into(),
                };
                let line: Element<'_, Message> = match item.page {
                    Some(page) => row![
                        container(detail).width(Fill),
                        button(text(format!("Page {}", page + 1)).size(12))
                            .padding([1, 6])
                            .style(button::text)
                            .on_press(Message::Preflight(Event::Show(page))),
                    ]
                    .spacing(6)
                    .into(),
                    None => detail,
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
        if self.items.iter().any(|i| i.element.is_some()) {
            let label = if self.copy.is_some() {
                "Save descriptions"
            } else {
                "Save a described copy…"
            };
            buttons = buttons.push(
                button(label).style(button::secondary).on_press_maybe(
                    (!self.busy && !self.descriptions().is_empty())
                        .then_some(Message::Preflight(Event::Describe)),
                ),
            );
        }
        if self.untagged && self.saved.is_none() {
            buttons = buttons.push(
                button("Save a tagged copy…")
                    .style(button::secondary)
                    .on_press(Message::Preflight(Event::Tag)),
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
