//! A comment summary: the comments in a document listed by page, on pages of their own, for
//! reading or printing.
//!
//! Text is set in the standard Helvetica fonts, so characters outside WinAnsiEncoding print as
//! `?`, as in the comments Panna draws itself.

use std::fmt::Write;

use super::annot::{literal, n, text_width, win_ansi, wrap};
use super::convert::file_of;
use crate::{Comment, CommentKind};

/// Pages smaller than this on either side get a Letter-sized summary page instead.
const SMALLEST: f32 = 250.0;
const LETTER: (f32, f32) = (612.0, 792.0);
const BODY: f64 = 10.0;
const HEADING: f64 = 12.0;
const TITLE: f64 = 16.0;
/// Helvetica-Bold runs about 6% wider than the Helvetica widths we have.
const BOLD_WIDER: f64 = 1.06;
/// How far a comment's text, and a reply, sit in from the line above.
const INDENT: f64 = 14.0;

/// How the summary is laid out.
pub(crate) enum Layout<'a> {
    /// One list for the whole document, under a title naming it.
    Whole { title: &'a str },
    /// A list for each page with comments, starting a page of its own, to print after it.
    EachPage,
}

/// What the summary pages of [`summary`] list.
pub(crate) struct Summary {
    pub file: Vec<u8>,
    /// For each page of `file`, the document page whose comments it lists (or begins with).
    pub lists: Vec<usize>,
}

/// The comments of a document with pages of `sizes` (as shown, in points) and page `labels`,
/// listed by page.
pub(crate) fn summary(
    comments: &[Comment],
    sizes: &[(f32, f32)],
    labels: &[String],
    layout: Layout<'_>,
) -> Summary {
    let size_of = |page: usize| {
        sizes
            .get(page)
            .copied()
            .filter(|&(w, h)| w >= SMALLEST && h >= SMALLEST)
            .unwrap_or(LETTER)
    };
    let label = |page: usize| {
        let number = (page + 1).to_string();
        match labels.get(page) {
            Some(l) if *l != number && !l.is_empty() => format!("{l} (page {number})"),
            _ => number,
        }
    };
    let groups = by_page(comments);
    let mut out = Sheets::default();
    match layout {
        Layout::Whole { title } => {
            out.begin(size_of(0), groups.first().map_or(0, |g| g.0));
            out.line(&format!("Comments on {title}"), Font::Bold, TITLE, 0.0);
            let count: usize = groups.iter().map(|g| g.1.len()).sum();
            let pages = groups.len();
            out.line(
                &match (count, pages) {
                    (0, _) => "There are no comments.".to_owned(),
                    (1, _) => "1 comment".to_owned(),
                    (c, 1) => format!("{c} comments on 1 page"),
                    (c, p) => format!("{c} comments on {p} pages"),
                },
                Font::Regular,
                BODY,
                0.0,
            );
            for (page, entries) in &groups {
                out.gap(HEADING);
                out.keep(HEADING * 4.0, *page);
                out.line(&format!("Page {}", label(*page)), Font::Bold, HEADING, 0.0);
                list(&mut out, entries, *page, None);
            }
        }
        Layout::EachPage => {
            for (page, entries) in &groups {
                let heading = format!("Comments on page {}", label(*page));
                out.begin(size_of(*page), *page);
                out.line(&heading, Font::Bold, HEADING, 0.0);
                list(&mut out, entries, *page, Some(&heading));
            }
        }
    }
    out.finish()
}

/// One comment in the list, with how deep in its thread it is.
type Entry<'a> = (&'a Comment, usize);

/// The comments on each page that has any, in page order. On each page they run from the top
/// down, as they sit on it, each followed by its replies.
fn by_page(comments: &[Comment]) -> Vec<(usize, Vec<Entry<'_>>)> {
    let mut pages: std::collections::BTreeMap<usize, Vec<&Comment>> = std::collections::BTreeMap::new();
    for c in comments {
        pages.entry(c.page).or_default().push(c);
    }
    pages
        .into_iter()
        .map(|(page, on_page)| {
            let is_reply = |c: &Comment| {
                c.reply_to.is_some_and(|to| {
                    on_page
                        .iter()
                        .any(|p| p.id.is_some_and(|id| id.annot == to) && !std::ptr::eq(*p, c))
                })
            };
            let mut top: Vec<&Comment> = on_page.iter().copied().filter(|c| !is_reply(c)).collect();
            top.sort_by(|a, b| a.at.y0.total_cmp(&b.at.y0).then(a.at.x0.total_cmp(&b.at.x0)));
            let mut entries = Vec::with_capacity(on_page.len());
            let mut listed = std::collections::HashSet::new();
            for c in top {
                thread(c, 0, &on_page, &mut entries, &mut listed);
            }
            (page, entries)
        })
        .collect()
}

/// Deepest a reply is shown indented; deeper ones line up with it.
const DEEPEST: usize = 4;

/// Adds `c` and, under it, the replies to it, each once even if the file loops them.
fn thread<'a>(
    c: &'a Comment,
    depth: usize,
    on_page: &[&'a Comment],
    entries: &mut Vec<Entry<'a>>,
    listed: &mut std::collections::HashSet<*const Comment>,
) {
    if !listed.insert(std::ptr::from_ref(c)) {
        return;
    }
    entries.push((c, depth.min(DEEPEST)));
    let Some(id) = c.id else { return };
    for reply in on_page.iter().filter(|r| r.reply_to == Some(id.annot)) {
        thread(reply, depth + 1, on_page, entries, listed);
    }
}

/// Writes `entries`, numbering the comments that start a thread. When a page fills, the next
/// starts with `continued` (if set) marked as continued.
fn list(out: &mut Sheets, entries: &[Entry<'_>], page: usize, continued: Option<&str>) {
    out.continued = continued.map(|h| format!("{h} (continued)"));
    let mut number = 0;
    for &(c, depth) in entries {
        let indent = INDENT * depth as f64;
        let head = if depth == 0 {
            number += 1;
            let mut head = format!("{number}. {}", c.kind.label());
            if !c.author.is_empty() {
                let _ = write!(head, " by {}", c.author);
            }
            head
        } else {
            match (c.state, c.author.is_empty()) {
                (Some(state), false) => format!("{} set the status to {state}", c.author),
                (Some(state), true) => format!("Status set to {state}"),
                (None, false) if c.kind == CommentKind::Reply => format!("Reply by {}", c.author),
                (None, true) if c.kind == CommentKind::Reply => "Reply".to_owned(),
                (None, false) => format!("{} by {}", c.kind.label(), c.author),
                (None, true) => c.kind.label().to_owned(),
            }
        };
        out.gap(BODY * 0.6);
        out.keep(BODY * 2.4, page);
        out.line(&head, Font::Bold, BODY, indent);
        let text = c.contents.replace("\r\n", "\n").replace('\r', "\n");
        let text = text.trim();
        if !text.is_empty() {
            let width = out.width() - indent - INDENT;
            for line in wrap(text, width / BODY) {
                out.line(&line, Font::Regular, BODY, indent + INDENT);
            }
        }
    }
    out.continued = None;
}

#[derive(Clone, Copy)]
enum Font {
    Regular,
    Bold,
}

/// The summary's pages as they're written.
#[derive(Default)]
struct Sheets {
    /// Each finished page: its size, its drawing, and the page it lists.
    done: Vec<((f32, f32), String, usize)>,
    /// The page being written.
    size: (f32, f32),
    draw: String,
    lists: usize,
    /// The top of the next line, from the page's bottom.
    y: f64,
    /// A heading to repeat when the list runs onto another page.
    continued: Option<String>,
    started: bool,
}

impl Sheets {
    fn margin(&self) -> f64 {
        (f64::from(self.size.0.min(self.size.1)) * 0.09).clamp(28.0, 72.0)
    }

    fn width(&self) -> f64 {
        f64::from(self.size.0) - 2.0 * self.margin()
    }

    /// Starts a new page of `size`, listing page `lists`.
    fn begin(&mut self, size: (f32, f32), lists: usize) {
        if self.started {
            let draw = std::mem::take(&mut self.draw);
            self.done.push((self.size, draw, self.lists));
        }
        self.started = true;
        self.size = size;
        self.lists = lists;
        self.y = f64::from(size.1) - self.margin();
    }

    /// Room for `height` more on this page, or a new page (with the continued heading).
    fn keep(&mut self, height: f64, lists: usize) {
        if self.y - height >= self.margin() {
            return;
        }
        self.begin(self.size, lists);
        if let Some(heading) = self.continued.clone() {
            self.line(&heading, Font::Bold, HEADING, 0.0);
            self.gap(BODY * 0.6);
        }
    }

    fn gap(&mut self, height: f64) {
        self.y -= height;
    }

    /// One line of `text` at `size`, `indent` in from the margin, cut to fit the page.
    fn line(&mut self, text: &str, font: Font, size: f64, indent: f64) {
        let lines = size * 1.25;
        let lists = self.lists;
        self.keep(lines, lists);
        let room = (self.width() - indent).max(size);
        let wider = match font {
            Font::Regular => 1.0,
            Font::Bold => BOLD_WIDER,
        };
        let mut text = text.to_owned();
        if text_width(&text) * size * wider > room {
            while !text.is_empty() && text_width(&format!("{text}…")) * size * wider > room {
                text.pop();
            }
            text.push('…');
        }
        self.y -= lines;
        let name = match font {
            Font::Regular => "F1",
            Font::Bold => "F2",
        };
        let _ = writeln!(
            self.draw,
            "BT /{name} {} Tf {} {} Td ({}) Tj ET",
            n(size),
            n(self.margin() + indent),
            n(self.y + size * 0.25),
            literal(&win_ansi(&text))
        );
    }

    /// The finished file; it has no pages if none were begun.
    fn finish(mut self) -> Summary {
        if self.started {
            let draw = std::mem::take(&mut self.draw);
            self.done.push((self.size, draw, self.lists));
        }
        // Objects: 1 catalog, 2 page tree, 3 and 4 the fonts, then each page and its content.
        let font = |base: &str| {
            format!("<< /Type /Font /Subtype /Type1 /BaseFont /{base} /Encoding /WinAnsiEncoding >>")
                .into_bytes()
        };
        let mut objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            Vec::new(),
            font("Helvetica"),
            font("Helvetica-Bold"),
        ];
        let mut kids = String::new();
        let mut lists = Vec::with_capacity(self.done.len());
        for ((w, h), draw, page) in &self.done {
            let at = objects.len() + 1;
            let _ = write!(kids, "{at} 0 R ");
            objects.push(
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents {} 0 R \
                     /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> >>",
                    n(f64::from(*w)),
                    n(f64::from(*h)),
                    at + 1
                )
                .into_bytes(),
            );
            objects.push(format!("<< /Length {} >>\nstream\n{draw}endstream", draw.len()).into_bytes());
            lists.push(*page);
        }
        if let Some(tree) = objects.get_mut(1) {
            *tree = format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kids.trim_end(),
                lists.len()
            )
            .into_bytes();
        }
        Summary {
            file: file_of(&objects),
            lists,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommentId, ObjRef, TextBox};

    fn comment(page: usize, y: f32, number: u32, reply_to: Option<u32>, contents: &str) -> Comment {
        Comment {
            id: Some(CommentId {
                page,
                annot: ObjRef {
                    number,
                    generation: 0,
                },
                popup: None,
            }),
            page,
            kind: if reply_to.is_some() {
                CommentKind::Reply
            } else {
                CommentKind::Note
            },
            contents: contents.into(),
            author: "Asha".into(),
            at: TextBox {
                x0: 10.0,
                y0: y,
                x1: 30.0,
                y1: y + 20.0,
            },
            marked: Vec::new(),
            reply_to: reply_to.map(|number| ObjRef {
                number,
                generation: 0,
            }),
            state: None,
            editable: None,
        }
    }

    fn text_of(file: &[u8]) -> String {
        String::from_utf8_lossy(file).into_owned()
    }

    #[test]
    fn comments_list_top_down_with_replies_under_them() {
        let comments = [
            comment(0, 500.0, 10, None, "Lower"),
            comment(0, 100.0, 11, None, "Upper"),
            comment(0, 0.0, 12, Some(10), "Answer"),
            comment(2, 50.0, 13, None, "Later page"),
        ];
        let groups = by_page(&comments);
        let order: Vec<(usize, Vec<(&str, usize)>)> = groups
            .iter()
            .map(|(p, e)| (*p, e.iter().map(|(c, d)| (c.contents.as_str(), *d)).collect()))
            .collect();
        assert_eq!(
            order,
            vec![
                (0, vec![("Upper", 0), ("Lower", 0), ("Answer", 1)]),
                (2, vec![("Later page", 0)])
            ]
        );
    }

    #[test]
    fn a_reply_loop_lists_each_comment_once() {
        let comments = [
            comment(0, 0.0, 10, Some(11), "A"),
            comment(0, 0.0, 11, Some(10), "B"),
        ];
        let total: usize = by_page(&comments).iter().map(|g| g.1.len()).sum();
        assert!(total <= 2);
    }

    #[test]
    fn whole_summary_names_the_document_and_each_page() {
        let comments = [
            comment(0, 0.0, 10, None, "Fix (this)"),
            comment(1, 0.0, 11, None, "Ok"),
        ];
        let labels = vec!["i".to_owned(), "2".to_owned()];
        let s = summary(
            &comments,
            &[(612.0, 792.0); 2],
            &labels,
            Layout::Whole { title: "Report" },
        );
        let text = text_of(&s.file);
        assert!(text.contains("(Comments on Report)"));
        assert!(text.contains("(2 comments on 2 pages)"));
        assert!(text.contains("(Page i \\(page 1\\))"));
        assert!(text.contains("(Page 2)"));
        assert!(text.contains("(Fix \\(this\\))"));
        assert_eq!(s.lists, vec![0]);
        let pdf = hayro::hayro_syntax::Pdf::new_with_password(std::sync::Arc::new(s.file), "").unwrap();
        assert_eq!(pdf.pages().len(), 1);
    }

    #[test]
    fn each_page_layout_starts_a_page_per_commented_page_and_continues_long_lists() {
        let long = "word ".repeat(4000);
        let comments = [
            comment(1, 0.0, 10, None, &long),
            comment(4, 0.0, 11, None, "Short"),
        ];
        let s = summary(&comments, &[(612.0, 792.0); 5], &[], Layout::EachPage);
        assert!(s.lists.len() >= 3, "{:?}", s.lists);
        assert_eq!(s.lists.first(), Some(&1));
        assert_eq!(s.lists.last(), Some(&4));
        assert!(s.lists.windows(2).all(|w| w[0] <= w[1]));
        let text = text_of(&s.file);
        assert!(text.contains("(Comments on page 2 \\(continued\\))"));
        let pdf = hayro::hayro_syntax::Pdf::new_with_password(std::sync::Arc::new(s.file), "").unwrap();
        assert_eq!(pdf.pages().len(), s.lists.len());
    }

    #[test]
    fn no_comments_still_makes_a_summary_page_but_no_pages_to_print() {
        let s = summary(&[], &[], &[], Layout::Whole { title: "Empty" });
        assert!(text_of(&s.file).contains("(There are no comments.)"));
        assert_eq!(s.lists.len(), 1);
        assert!(summary(&[], &[], &[], Layout::EachPage).lists.is_empty());
    }
}
