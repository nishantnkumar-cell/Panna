//! The Comments tab: every comment in the document, in page order, like Acrobat's comment list.
//! Clicking one scrolls to it and selects it, ready to move or restyle; any can be deleted.

use iced::widget::{Column, button, column, container, pick_list, row, scrollable, space, text};
use iced::{Center, Element, Fill, Theme};
use panna_core::{Annotation, Comment, CommentId, CommentKind, ReviewState};

use crate::Message;
use crate::annotate;
use crate::icons::{self, Icon};
use crate::selection::TextSlot;

/// Longest snippet shown per comment.
const SNIPPET: usize = 120;

fn icon(kind: &CommentKind) -> Icon {
    match kind {
        CommentKind::Highlight => Icon::Highlighter,
        CommentKind::Underline => Icon::Underline,
        CommentKind::StrikeOut => Icon::Strikethrough,
        CommentKind::Note => Icon::StickyNote,
        CommentKind::TextBox => Icon::TextBox,
        CommentKind::Ink => Icon::Pen,
        CommentKind::Rect => Icon::Square,
        CommentKind::Ellipse => Icon::Circle,
        CommentKind::Line => Icon::Line,
        CommentKind::Polygon => Icon::Polygon,
        CommentKind::Cloud => Icon::Cloud,
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
    let (replies, mut entries): (Vec<_>, Vec<_>) =
        entries.into_iter().partition(|(c, _)| c.reply_to.is_some());
    entries.sort_by_key(|(c, _)| c.page);
    if entries.is_empty() {
        return container(
            text("No comments yet. Use the comment tools to add some.")
                .size(13)
                .style(text::secondary),
        )
        .padding(12)
        .into();
    }
    let count = match entries.len() {
        1 => "1 comment".to_owned(),
        n => format!("{n} comments"),
    };
    let items = entries.into_iter().map(|(comment, unsaved)| {
        // Text markups quote the words they mark, once that page's text is loaded.
        let quote = texts
            .get(comment.page)
            .and_then(|slot| slot.text.as_ref())
            .filter(|_| !comment.marked.is_empty())
            .map(|t| t.text_in(&comment.marked))
            .filter(|q| !q.is_empty());
        let parent = comment.id.map(|id| id.annot).filter(|_| unsaved.is_none());
        let thread: Vec<&(Comment, Option<usize>)> = replies
            .iter()
            .filter(|(r, _)| parent.is_some() && r.reply_to == parent)
            .collect();
        entry(comment, unsaved, quote, &thread)
    });
    column![
        container(text(count).size(12).style(text::secondary)).padding([6, 12]),
        scrollable(
            Column::with_children(items)
                .spacing(2)
                .padding([0, 6])
                .width(Fill)
        )
        .height(Fill),
    ]
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
    match (comment.author.is_empty(), unsaved) {
        (true, false) => None,
        (false, false) => Some(comment.author.clone()),
        (true, true) => Some("Not saved yet".to_owned()),
        (false, true) => Some(format!("{} · not saved yet", comment.author)),
    }
}

fn entry<'a>(
    comment: Comment,
    unsaved: Option<usize>,
    quote: Option<String>,
    thread: &[&(Comment, Option<usize>)],
) -> Element<'a, Message> {
    // The latest status set in the thread, unsaved ones last.
    let status = thread
        .iter()
        .filter_map(|(r, _)| r.state)
        .next_back()
        .unwrap_or(ReviewState::None);
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
    for (reply, unsaved) in thread.iter().filter(|(r, _)| r.state.is_none()) {
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
}
