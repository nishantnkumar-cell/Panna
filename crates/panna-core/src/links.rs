//! Links on pages: jumps within the document, and web or mail addresses.
//!
//! Only plain jumps and web and mail addresses are kept. Links that would launch programs,
//! open files or run scripts are dropped, so the viewer can never be asked to follow them.

use hayro::hayro_interpret::TransformExt;
use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Name, Rect};
use hayro::kurbo::Point;

use crate::TextBox;
use crate::outline::{Ctx, destination};

const MAX_LINKS: usize = 50_000;
const MAX_PER_PAGE: usize = 2_000;
const MAX_URI: usize = 2_048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// A page of this document, zero-based.
    Page(usize),
    /// An `http`, `https` or `mailto` address.
    Uri(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub page: usize,
    /// Its area, in the same top-left, unrotated-view space as page text.
    pub at: TextBox,
    pub target: LinkTarget,
}

/// Whether an address is one Panna will hand to the browser or mail program: a web or mail
/// address with no spaces, quotes or control characters.
pub fn safe_uri(uri: &str) -> bool {
    let lower = uri.to_ascii_lowercase();
    let scheme_ok = ["http://", "https://", "mailto:"]
        .iter()
        .any(|s| lower.starts_with(s) && lower.len() > s.len());
    scheme_ok
        && uri.len() <= MAX_URI
        && uri
            .chars()
            .all(|c| c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '<' | '>' | '\\' | '`'))
}

pub(crate) fn read(pdf: &Pdf) -> Vec<Link> {
    let Some(ctx) = Ctx::new(pdf) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, page) in pdf.pages().iter().enumerate() {
        let Some(annots) = page.raw().get::<Array<'_>>(b"Annots") else {
            continue;
        };
        let to_view = page.initial_transform(true).to_kurbo();
        for annot in annots.iter::<Dict<'_>>().take(MAX_PER_PAGE) {
            if out.len() >= MAX_LINKS {
                return out;
            }
            if annot
                .get::<Name<'_>>(b"Subtype")
                .is_none_or(|s| s.as_str() != "Link")
            {
                continue;
            }
            let Some(target) = target(&ctx, &annot) else {
                continue;
            };
            let Some(rect) = annot.get::<Rect>(b"Rect") else {
                continue;
            };
            let a = to_view * Point::new(rect.x0, rect.y0);
            let b = to_view * Point::new(rect.x1, rect.y1);
            out.push(Link {
                page: index,
                at: TextBox {
                    x0: a.x.min(b.x) as f32,
                    y0: a.y.min(b.y) as f32,
                    x1: a.x.max(b.x) as f32,
                    y1: a.y.max(b.y) as f32,
                },
                target,
            });
        }
    }
    out
}

fn target(ctx: &Ctx<'_>, annot: &Dict<'_>) -> Option<LinkTarget> {
    if let Some(page) = destination(ctx, annot) {
        return Some(LinkTarget::Page(page));
    }
    let action = annot.get::<Dict<'_>>(b"A")?;
    if action.get::<Name<'_>>(b"S")?.as_str() != "URI" {
        return None;
    }
    let uri = action.get::<hayro::hayro_syntax::object::String<'_>>(b"URI")?;
    let uri = std::str::from_utf8(uri.as_bytes()).ok()?.trim().to_owned();
    safe_uri(&uri).then_some(LinkTarget::Uri(uri))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_mail_addresses_are_followed() {
        assert!(safe_uri("https://example.org/a?b=c#d"));
        assert!(safe_uri("mailto:someone@example.org"));
        assert!(!safe_uri("https://"));
        assert!(!safe_uri("file:///etc/passwd"));
        assert!(!safe_uri("javascript:alert(1)"));
        assert!(!safe_uri("https://example.org/a b"));
        assert!(!safe_uri("https://example.org/\"&calc"));
    }
}
