//! The Comments tab: every comment in the document, like Acrobat's comment list. It sorts by
//! page, author, date or type, filters by type, author, status and checkmark, and clicking a
//! comment scrolls to it and selects it, ready to move or restyle; any can be deleted.

use iced::widget::{Column, button, column, container, pick_list, row, scrollable, space, text};
use iced::{Center, Element, Fill, Theme};
use panna_core::{Annotation, Comment, CommentId, CommentKind, ReviewState};

use crate::Message;
use crate::annotate;
use crate::icons::{self, Icon};
use crate::selection::TextSlot;

/// Longest snippet shown per comment.
const SNIPPET: usize = 120;

/// How the list is ordered, as Acrobat's Sort By menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Page,
    Author,
    Date,
    Type,
}

impl Sort {
    const ALL: [Sort; 4] = [Sort::Page, Sort::Author, Sort::Date, Sort::Type];
}

impl std::fmt::Display for Sort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Sort::Page => "By page",
            Sort::Author => "By author",
            Sort::Date => "By date, newest first",
            Sort::Type => "By type",
        })
    }
}

/// Which comments a filter lets through: everything, or those with this label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Only {
    All(&'static str),
    This(String),
}

impl Only {
    fn passes(&self, value: &str) -> bool {
        match self {
            Only::All(_) => true,
            Only::This(v) => v == value,
        }
    }
}

impl std::fmt::Display for Only {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Only::All(label) => f.write_str(label),
            Only::This(v) => f.write_str(v),
        }
    }
}

const ALL_TYPES: &str = "All types";
const ALL_AUTHORS: &str = "All reviewers";
const ALL_STATUSES: &str = "Any status";
const ALL_CHECKS: &str = "Any check";
const CHECKED: &str = "Checked";
const UNCHECKED: &str = "Unchecked";
/// How an empty author is listed.
const NO_AUTHOR: &str = "No name";

/// The list's sort and filters, kept while the document is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListView {
    pub sort: Sort,
    /// The filter row is showing.
    pub filtering: bool,
    pub kind: Only,
    pub author: Only,
    pub status: Only,
    pub check: Only,
}

impl Default for ListView {
    fn default() -> Self {
        Self {
            sort: Sort::Page,
            filtering: false,
            kind: Only::All(ALL_TYPES),
            author: Only::All(ALL_AUTHORS),
            status: Only::All(ALL_STATUSES),
            check: Only::All(ALL_CHECKS),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ListEvent {
    Sort(Sort),
    ToggleFilters,
    Kind(Only),
    Author(Only),
    Status(Only),
    Check(Only),
    ShowAll,
}

impl ListView {
    pub fn update(&mut self, event: ListEvent) {
        match event {
            ListEvent::Sort(sort) => self.sort = sort,
            ListEvent::ToggleFilters => self.filtering = !self.filtering,
            ListEvent::Kind(o) => self.kind = o,
            ListEvent::Author(o) => self.author = o,
            ListEvent::Status(o) => self.status = o,
            ListEvent::Check(o) => self.check = o,
            ListEvent::ShowAll => {
                *self = Self {
                    sort: self.sort,
                    filtering: self.filtering,
                    ..Self::default()
                };
            }
        }
    }

    /// Whether any filter is hiding comments.
    fn filtered(&self) -> bool {
        [&self.kind, &self.author, &self.status, &self.check]
            .iter()
            .any(|o| matches!(o, Only::This(_)))
    }

    fn passes(&self, c: &Comment, status: ReviewState, checked: bool) -> bool {
        let author = if c.author.is_empty() { NO_AUTHOR } else { &c.author };
        self.kind.passes(c.kind.label())
            && self.author.passes(author)
            && self.status.passes(status.label())
            && self.check.passes(if checked { CHECKED } else { UNCHECKED })
    }
}

/// The latest review status and checkmark in a thread, unsaved ones last.
fn marks(thread: &[&(Comment, Option<usize>)]) -> (ReviewState, bool) {
    let status = thread
        .iter()
        .filter_map(|(r, _)| r.state)
        .next_back()
        .unwrap_or(ReviewState::None);
    let checked = thread
        .iter()
        .filter_map(|(r, _)| r.checked)
        .next_back()
        .unwrap_or(false);
    (status, checked)
}

/// The options for a filter: everything, then each value found, in order, once.
fn options(all: &'static str, values: impl Iterator<Item = String>) -> Vec<Only> {
    let mut found: Vec<String> = values.collect();
    found.sort();
    found.dedup();
    std::iter::once(Only::All(all))
        .chain(found.into_iter().map(Only::This))
        .collect()
}

fn icon(kind: &CommentKind) -> Icon {
    match kind {
        CommentKind::Highlight => Icon::Highlighter,
        CommentKind::Underline => Icon::Underline,
        CommentKind::StrikeOut => Icon::Strikethrough,
        CommentKind::Note => Icon::StickyNote,
        CommentKind::TextBox => Icon::TextBox,
        CommentKind::Callout => Icon::Callout,
        CommentKind::File => Icon::Paperclip,
        CommentKind::Ink => Icon::Pen,
        CommentKind::Rect => Icon::Square,
        CommentKind::Ellipse => Icon::Circle,
        CommentKind::Line => Icon::Line,
        CommentKind::Polygon => Icon::Polygon,
        CommentKind::Cloud => Icon::Cloud,
        CommentKind::PolyLine => Icon::PolyLine,
        CommentKind::Caret => Icon::InsertText,
        CommentKind::Reply => Icon::Reply,
        CommentKind::Stamp => Icon::Stamp,
        CommentKind::Other(_) => Icon::Annotate,
    }
}

fn snippet(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(SNIPPET) {
        Some((cut, _)) => format!("{}…", flat.get(..cut).unwrap_or_default()),
        None => flat,
    }
}

/// `saved` are the comments in the file; `unsaved` the ones added since, which can be deleted.
/// Replies and review statuses are shown in their comment's thread rather than on their own.
pub fn panel<'a>(
    saved: &'a [Comment],
    unsaved: &'a [Annotation],
    removed: &[CommentId],
    texts: &[TextSlot],
    view: &ListView,
) -> Element<'a, Message> {
    // Saved comments first within each page, then new ones in the order they were added.
    // Comments taken out but not yet saved are already gone from the list.
    let mut entries: Vec<(Comment, Option<usize>)> = saved
        .iter()
        .filter(|c| c.id.is_none_or(|id| !removed.contains(&id)))
        .cloned()
        .map(|c| (c, None))
        .collect();
    entries.extend(
        unsaved
            .iter()
            .enumerate()
            .map(|(i, a)| (Comment::from(a), Some(i))),
    );
    let (replies, entries): (Vec<_>, Vec<_>) = entries.into_iter().partition(|(c, _)| c.reply_to.is_some());
    if entries.is_empty() {
        return container(
            text("No comments yet. Use the comment tools to add some.")
                .size(13)
                .style(text::secondary),
        )
        .padding(12)
        .into();
    }
    let total = entries.len();
    // Each comment with its thread and the marks set in it.
    let mut listed: Vec<_> = entries
        .into_iter()
        .map(|(comment, unsaved)| {
            let parent = comment.id.map(|id| id.annot).filter(|_| unsaved.is_none());
            let thread: Vec<&(Comment, Option<usize>)> = replies
                .iter()
                .filter(|(r, _)| parent.is_some() && r.reply_to == parent)
                .collect();
            let (status, checked) = marks(&thread);
            (comment, unsaved, thread, status, checked)
        })
        .collect();
    let kinds = options(ALL_TYPES, listed.iter().map(|l| l.0.kind.label().to_owned()));
    let authors = options(
        ALL_AUTHORS,
        listed.iter().map(|l| {
            if l.0.author.is_empty() {
                NO_AUTHOR.to_owned()
            } else {
                l.0.author.clone()
            }
        }),
    );
    listed.retain(|(c, _, _, status, checked)| view.passes(c, *status, *checked));
    // Stable sorts, so ties keep page order, saved comments before new ones.
    listed.sort_by_key(|l| l.0.page);
    match view.sort {
        Sort::Page => {}
        Sort::Author => listed.sort_by_key(|l| l.0.author.to_lowercase()),
        // Comments not saved yet are the newest.
        Sort::Date => listed.sort_by(|a, b| (b.1.is_some(), &b.0.date).cmp(&(a.1.is_some(), &a.0.date))),
        Sort::Type => listed.sort_by(|a, b| a.0.kind.label().cmp(b.0.kind.label())),
    }
    let count = match (listed.len(), view.filtered()) {
        (n, true) => format!("{n} of {total}"),
        (1, false) => "1 comment".to_owned(),
        (n, false) => format!("{n} comments"),
    };
    let shown = listed.len();
    let items = listed
        .into_iter()
        .map(|(comment, unsaved, thread, status, checked)| {
            // Text markups quote the words they mark, once that page's text is loaded.
            let quote = texts
                .get(comment.page)
                .and_then(|slot| slot.text.as_ref())
                .filter(|_| !comment.marked.is_empty())
                .map(|t| t.text_in(&comment.marked))
                .filter(|q| !q.is_empty());
            entry(comment, unsaved, quote, &thread, status, checked)
        });
    let ev = Message::CommentList;
    let filter_tip = if view.filtering {
        "Hide the filters"
    } else {
        "Filter comments by type, reviewer, status or checkmark"
    };
    let mut panel = column![
        row![
            text(count).size(12).style(text::secondary),
            space::horizontal(),
            icons::button(
                Icon::CommentSummary,
                "Save a list of the comments, by page, as a PDF…",
                Some(Message::SummarizeComments)
            ),
            icons::button(
                Icon::PrintComments,
                "Print with each page's comments after it…",
                Some(Message::PrintWithComments)
            ),
        ]
        .spacing(2)
        .align_y(Center)
        .padding([2, 6]),
        row![
            pick_list(Sort::ALL, Some(view.sort), move |s| ev(ListEvent::Sort(s)))
                .text_size(12)
                .padding([2, 6])
                .width(Fill),
            icons::toggle(
                Icon::Filter,
                filter_tip,
                view.filtering || view.filtered(),
                ev(ListEvent::ToggleFilters),
            ),
        ]
        .spacing(4)
        .align_y(Center)
        .padding([0, 6]),
    ]
    .spacing(2);
    if view.filtering {
        let statuses = options(
            ALL_STATUSES,
            ReviewState::ALL.iter().map(|s| s.label().to_owned()),
        );
        let checks = vec![
            Only::All(ALL_CHECKS),
            Only::This(CHECKED.to_owned()),
            Only::This(UNCHECKED.to_owned()),
        ];
        // No tooltips here: they would cover the open menus. Each shows what it filters by.
        let pick = |options: Vec<Only>, now: &Only, on: fn(Only) -> ListEvent| {
            pick_list(options, Some(now.clone()), move |o| ev(on(o)))
                .text_size(12)
                .padding([2, 6])
                .width(Fill)
        };
        panel = panel.push(
            column![
                row![
                    pick(kinds, &view.kind, ListEvent::Kind),
                    pick(authors, &view.author, ListEvent::Author),
                ]
                .spacing(4),
                row![
                    pick(statuses, &view.status, ListEvent::Status),
                    pick(checks, &view.check, ListEvent::Check),
                    button(text("Show all").size(12))
                        .style(button::secondary)
                        .padding([3, 8])
                        .on_press_maybe(view.filtered().then_some(ev(ListEvent::ShowAll))),
                ]
                .spacing(4)
                .align_y(Center),
            ]
            .spacing(4)
            .padding([2, 6]),
        );
    }
    if shown == 0 {
        panel = panel.push(
            container(
                text("No comments match the filters.")
                    .size(13)
                    .style(text::secondary),
            )
            .padding(12),
        );
    }
    panel
        .push(
            scrollable(
                Column::with_children(items)
                    .spacing(2)
                    .padding([0, 6])
                    .width(Fill),
            )
            .height(Fill),
        )
        .into()
}

/// The delete action for a listed comment or reply.
fn delete_event(comment: &Comment, unsaved: Option<usize>) -> Option<annotate::Event> {
    match (unsaved, comment.id) {
        (Some(index), _) => Some(annotate::Event::Delete(index)),
        (None, Some(id)) => Some(annotate::Event::Remove(id)),
        (None, None) => None,
    }
}

fn byline(comment: &Comment, unsaved: bool) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if !comment.author.is_empty() {
        parts.push(&comment.author);
    }
    if unsaved {
        parts.push(if parts.is_empty() {
            "Not saved yet"
        } else {
            "not saved yet"
        });
    } else if !comment.date.is_empty() {
        parts.push(&comment.date);
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn entry<'a>(
    comment: Comment,
    unsaved: Option<usize>,
    quote: Option<String>,
    thread: &[&(Comment, Option<usize>)],
    status: ReviewState,
    checked: bool,
) -> Element<'a, Message> {
    let mut heading = row![
        icons::glyph(icon(&comment.kind), true),
        text(comment.kind.label().to_owned()).size(13),
    ]
    .spacing(6)
    .align_y(Center);
    if status != ReviewState::None {
        heading = heading.push(container(text(status.label()).size(11)).padding([1, 6]).style(
            |theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style::default()
                    .background(palette.primary.weak.color)
                    .color(palette.primary.weak.text)
                    .border(iced::Border::default().rounded(8))
            },
        ));
    }
    heading = heading.push(space::horizontal()).push(
        text(format!("Page {}", comment.page + 1))
            .size(11)
            .style(text::secondary),
    );
    let picked = match (unsaved, comment.id) {
        (Some(index), _) => Some(annotate::Picked::New(index)),
        (None, Some(id)) if comment.editable.is_some() => Some(annotate::Picked::Saved(id)),
        _ => None,
    };
    if let Some(event) = delete_event(&comment, unsaved) {
        heading = heading.push(icons::button(
            Icon::Delete,
            "Delete this comment",
            Some(Message::Annotate(event)),
        ));
    }
    let mut body = column![heading].spacing(2);
    if let Some(quote) = quote {
        body = body.push(
            text(format!("“{}”", snippet(&quote)))
                .size(12)
                .style(text::secondary),
        );
    }
    if !comment.contents.is_empty() {
        body = body.push(text(snippet(&comment.contents)).size(12));
    }
    if let Some(byline) = byline(&comment, unsaved.is_some()) {
        body = body.push(text(byline).size(11).style(text::secondary));
    }
    let card = button(body)
        .padding([5, 6])
        .width(Fill)
        .style(|theme: &Theme, status| {
            let mut style = button::text(theme, status);
            if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                style.background = Some(theme.extended_palette().background.base.color.into());
            }
            style
        })
        .on_press(Message::ShowComment(comment.page, comment.at, picked));
    // Only comments saved in the file can be replied to: a reply points at the comment's object.
    let Some(id) = comment.id.filter(|_| unsaved.is_none()) else {
        return card.into();
    };
    let target = annotate::Thread {
        page: comment.page,
        comment: id.annot,
        at: comment.at,
    };
    let mut replies = Column::new().spacing(4).padding(iced::Padding {
        left: 28.0,
        ..iced::Padding::ZERO
    });
    for (reply, unsaved) in thread
        .iter()
        .filter(|(r, _)| r.state.is_none() && r.checked.is_none())
    {
        let mut line = column![text(snippet(&reply.contents)).size(12)].spacing(1);
        if let Some(byline) = byline(reply, unsaved.is_some()) {
            line = line.push(text(byline).size(11).style(text::secondary));
        }
        let mut reply_row = row![line, space::horizontal()].spacing(4).align_y(Center);
        if let Some(event) = delete_event(reply, *unsaved) {
            reply_row = reply_row.push(icons::button(
                Icon::Delete,
                "Delete this reply",
                Some(Message::Annotate(event)),
            ));
        }
        replies = replies.push(reply_row);
    }
    let actions = row![
        icons::toggle(
            Icon::Check,
            if checked {
                "Take the checkmark off (Shift+K)"
            } else {
                "Add a checkmark, for your own tracking (Shift+K)"
            },
            checked,
            Message::Annotate(annotate::Event::ToggleCheck(target)),
        ),
        space::horizontal(),
        icons::button(
            Icon::Reply,
            "Reply",
            Some(Message::Annotate(annotate::Event::Reply(target)))
        ),
        icons::with_tip(
            pick_list(ReviewState::ALL, Some(status), move |state| {
                Message::Annotate(annotate::Event::SetStatus(target, state))
            })
            .text_size(12)
            .padding([2, 6]),
            "Review status",
        ),
    ]
    .spacing(4)
    .align_y(Center);
    column![card, replies, actions].spacing(2).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_are_flattened_and_cut() {
        assert_eq!(snippet("a\n  b\tc"), "a b c");
        let long = "x".repeat(SNIPPET + 10);
        assert_eq!(snippet(&long).chars().count(), SNIPPET + 1);
    }

    #[test]
    fn filters_pass_only_what_matches() {
        let note = Annotation {
            page: 0,
            kind: panna_core::Kind::Note { x: 1.0, y: 1.0 },
            color: crate::annotate::SWATCHES[0],
            opacity: 1.0,
            width: 1.0,
            contents: String::new(),
            author: String::new(),
            dashed: false,
            fill: None,
            ends: Default::default(),
        };
        let c = Comment::from(&note);
        let mut view = ListView::default();
        assert!(view.passes(&c, ReviewState::None, false) && !view.filtered());
        view.update(ListEvent::Author(Only::This(NO_AUTHOR.into())));
        view.update(ListEvent::Check(Only::This(CHECKED.into())));
        assert!(view.filtered());
        assert!(view.passes(&c, ReviewState::None, true));
        assert!(!view.passes(&c, ReviewState::None, false));
        view.update(ListEvent::Kind(Only::This("Highlight".into())));
        assert!(!view.passes(&c, ReviewState::None, true));
        view.update(ListEvent::ShowAll);
        assert!(!view.filtered() && view.passes(&c, ReviewState::Accepted, false));
        let found = options(ALL_TYPES, ["b", "a", "b"].into_iter().map(String::from));
        assert_eq!(found.len(), 3);
        assert_eq!(found[1], Only::This("a".into()));
    }
}
