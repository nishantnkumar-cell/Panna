//! The Comments tab: every comment in the document, in page order, like Acrobat's comment list.
//! Clicking one scrolls to it and selects it, ready to move or restyle; any can be deleted.

use iced::widget::{Column, button, column, container, row, scrollable, space, text};
use iced::{Center, Element, Fill, Theme};
use panna_core::{Annotation, Comment, CommentId, CommentKind};

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
        entry(comment, unsaved, quote)
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

fn entry<'a>(comment: Comment, unsaved: Option<usize>, quote: Option<String>) -> Element<'a, Message> {
    let mut heading = row![
        icons::glyph(icon(&comment.kind), true),
        text(comment.kind.label().to_owned()).size(13),
        space::horizontal(),
        text(format!("Page {}", comment.page + 1))
            .size(11)
            .style(text::secondary),
    ]
    .spacing(6)
    .align_y(Center);
    let delete = match (unsaved, comment.id) {
        (Some(index), _) => Some(annotate::Event::Delete(index)),
        (None, Some(id)) => Some(annotate::Event::Remove(id)),
        (None, None) => None,
    };
    let picked = match (unsaved, comment.id) {
        (Some(index), _) => Some(annotate::Picked::New(index)),
        (None, Some(id)) if comment.editable.is_some() => Some(annotate::Picked::Saved(id)),
        _ => None,
    };
    if let Some(event) = delete {
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
    let byline = match (comment.author.is_empty(), unsaved.is_some()) {
        (true, false) => None,
        (false, false) => Some(comment.author.clone()),
        (true, true) => Some("Not saved yet".to_owned()),
        (false, true) => Some(format!("{} · not saved yet", comment.author)),
    };
    if let Some(byline) = byline {
        body = body.push(text(byline).size(11).style(text::secondary));
    }
    button(body)
        .padding([5, 6])
        .width(Fill)
        .style(|theme: &Theme, status| {
            let mut style = button::text(theme, status);
            if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                style.background = Some(theme.extended_palette().background.base.color.into());
            }
            style
        })
        .on_press(Message::ShowComment(comment.page, comment.at, picked))
        .into()
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
