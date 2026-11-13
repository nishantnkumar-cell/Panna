//! Reflow, as Acrobat's View › Zoom › Reflow: a page's text shown as paragraphs that wrap to
//! the window, in larger or smaller type as the zoom changes, for reading on a small screen
//! or at a large size. A page at a time; pictures and layout aren't shown.

/// A reflowed page's paragraphs: the document's id, the page, and the paragraphs with their
/// type sizes.
pub type Reflowed = (u64, usize, std::sync::Arc<Vec<(String, f32)>>);

use iced::widget::{Column, column, container, row, scrollable, space, text};
use iced::{Center, Element, Fill, font};

use crate::Message;
use crate::icons::{self, Icon};

/// Body text's size in reflow at 100%, in pixels.
const BODY: f32 = 17.0;

/// The size most of the text is in, by characters.
fn body_size(paragraphs: &[(String, f32)]) -> f32 {
    let mut sizes: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (t, size) in paragraphs {
        if size.is_finite() && *size > 0.0 {
            *sizes.entry((size * 2.0).round() as u32).or_default() += t.chars().count();
        }
    }
    sizes
        .into_iter()
        .max_by_key(|(size, count)| (*count, std::cmp::Reverse(*size)))
        .map_or(11.0, |(size, _)| size as f32 / 2.0)
        .max(1.0)
}

/// How much bigger than body text a paragraph is shown, and whether it's a heading.
fn scale(size: f32, body: f32) -> (f32, bool) {
    if !size.is_finite() || size <= 0.0 {
        return (1.0, false);
    }
    let ratio = (size / body).clamp(0.75, 2.2);
    (ratio, ratio >= 1.2)
}

/// The reflowed page: `paragraphs` once its text is read (`None` before), page `page` of
/// `count`, at `zoom`.
pub fn view<'a>(
    paragraphs: Option<&'a [(String, f32)]>,
    page: usize,
    count: usize,
    zoom: f32,
) -> Element<'a, Message> {
    let bar = row![
        icons::with_tip(
            icons::glyph::<Message>(Icon::Reflow, true),
            "Reflow: the text wraps to fit the window"
        ),
        icons::button(
            Icon::PrevPage,
            "Previous page",
            (page > 0).then_some(Message::PrevPage)
        ),
        text(format!("Page {} of {count}", page + 1)).size(13),
        icons::button(
            Icon::NextPage,
            "Next page",
            (page + 1 < count).then_some(Message::NextPage)
        ),
        space::horizontal(),
        icons::button(Icon::ZoomOut, "Smaller text", Some(Message::ZoomOut)),
        icons::button(Icon::ZoomIn, "Larger text", Some(Message::ZoomIn)),
        icons::button(
            Icon::Close,
            format!("Leave reflow ({}4)", icons::MOD),
            Some(Message::ToggleReflow)
        ),
    ]
    .spacing(6)
    .align_y(Center)
    .padding([4, 12]);
    let zoom = zoom.clamp(0.5, 4.0);
    let body: Element<'a, Message> = match paragraphs {
        None => container(text("Reading the page's text…").size(14)).padding(24).into(),
        Some([]) => container(
            text("This page has no text to reflow. A scanned page needs its text recognised first (Tools › Make searchable).")
                .size(14),
        )
        .padding(24)
        .into(),
        Some(paragraphs) => {
            let base = body_size(paragraphs);
            let column = paragraphs.iter().fold(Column::new().spacing(12), |col, (t, size)| {
                let (ratio, heading) = scale(*size, base);
                let mut line = text(t.as_str()).size(BODY * zoom * ratio).line_height(1.45);
                if heading {
                    line = line.font(iced::Font {
                        weight: font::Weight::Bold,
                        ..iced::Font::DEFAULT
                    });
                }
                col.push(line)
            });
            scrollable(
                container(column.width(Fill).max_width(BODY * zoom * 38.0).padding([24, 24]))
                    .width(Fill)
                    .center_x(Fill),
            )
            .height(Fill)
            .into()
        }
    };
    column![bar, body].height(Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_size_is_the_size_most_text_is_in_and_larger_text_is_a_heading() {
        let paragraphs = vec![
            ("A Title".to_owned(), 24.0),
            (
                "Body text that goes on for a while, longer than the title.".to_owned(),
                10.0,
            ),
            ("More body.".to_owned(), 10.0),
        ];
        let body = body_size(&paragraphs);
        assert_eq!(body, 10.0);
        assert_eq!(scale(24.0, body), (2.2, true));
        assert_eq!(scale(10.0, body), (1.0, false));
        assert_eq!(scale(f32::NAN, body), (1.0, false));
    }
}
