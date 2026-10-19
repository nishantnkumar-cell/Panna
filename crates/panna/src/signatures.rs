//! The Signatures tab: each digital signature in the document, how it checks out, and what
//! its certificate says about the signer.

use iced::widget::{Column, button, column, row, scrollable, text};
use iced::{Center, Element, Fill};
use panna_core::{SigStatus, Signature};

use crate::Message;
use crate::icons::{self, Icon};

const PAD: f32 = 12.0;

/// The icon for a signature, or for the tab: a problem if anything doesn't check out, a
/// question if something couldn't be checked or changed afterwards.
fn status_icon(sig: &Signature) -> Icon {
    match sig.status {
        SigStatus::Invalid(_) => Icon::SignatureProblem,
        SigStatus::Unsupported(_) => Icon::SignatureUnknown,
        SigStatus::Valid if sig.changed_after || sig.outside_validity => Icon::SignatureUnknown,
        SigStatus::Valid => Icon::Signed,
    }
}

pub fn icon(sigs: &[Signature]) -> Icon {
    let icons: Vec<Icon> = sigs.iter().map(status_icon).collect();
    if icons.contains(&Icon::SignatureProblem) {
        Icon::SignatureProblem
    } else if icons.contains(&Icon::SignatureUnknown) {
        Icon::SignatureUnknown
    } else {
        Icon::Signed
    }
}

fn status_text(sig: &Signature) -> String {
    match &sig.status {
        SigStatus::Valid if sig.changed_after => {
            "Valid for the version that was signed. The document was changed afterwards, such as by \
             filling in a form or adding a comment."
                .into()
        }
        SigStatus::Valid => "Valid: the document hasn't changed since it was signed.".into(),
        SigStatus::Invalid(why) => format!("Not valid. {why}."),
        SigStatus::Unsupported(why) if why.is_empty() => "Can't be checked.".into(),
        SigStatus::Unsupported(why) => format!("Can't be checked. {why}."),
    }
}

pub fn panel(sigs: &[Signature]) -> Element<'_, Message> {
    let items = sigs.iter().map(|sig| {
        let signer = if sig.signer.is_empty() {
            "Signed".to_owned()
        } else {
            format!("Signed by {}", sig.signer)
        };
        let mut about = column![
            row![
                icons::glyph(status_icon(sig), true),
                text(signer).size(13).wrapping(text::Wrapping::WordOrGlyph)
            ]
            .spacing(6)
            .align_y(Center),
            text(status_text(sig))
                .size(12)
                .wrapping(text::Wrapping::WordOrGlyph),
        ]
        .spacing(3);
        let mut lines = Vec::new();
        if let Some(level) = sig.certifies {
            lines.push(
                match level {
                    1 => "Certifies the document: no changes are allowed.",
                    2 => "Certifies the document: filling in forms and signing are allowed.",
                    _ => "Certifies the document: filling in forms, signing and comments are allowed.",
                }
                .to_owned(),
            );
        }
        if sig.outside_validity {
            lines.push("The certificate wasn't valid when this was signed.".into());
        }
        lines.push(if sig.issuer.is_empty() {
            "The signer made their own certificate, so who they are isn't confirmed.".into()
        } else {
            format!(
                "Certificate issued by {}. Whether to trust it isn't checked: that needs a list of \
                 trusted certificates.",
                sig.issuer
            )
        });
        if let Some(time) = &sig.time {
            lines.push(format!("Signed on {time}, by the signer's clock"));
        }
        if !sig.reason.is_empty() {
            lines.push(format!("Reason: {}", sig.reason));
        }
        if !sig.location.is_empty() {
            lines.push(format!("Location: {}", sig.location));
        }
        for line in lines {
            about = about.push(
                text(line)
                    .size(12)
                    .style(text::secondary)
                    .wrapping(text::Wrapping::WordOrGlyph),
            );
        }
        let field = match sig.page {
            Some(page) => format!("{} · page {}", sig.field, page + 1),
            None => sig.field.clone(),
        };
        let field = text(field).size(12).style(text::secondary);
        about = about.push(match sig.page {
            Some(page) => Element::from(
                button(field)
                    .padding(0)
                    .style(button::text)
                    .on_press(Message::GoToPage(page)),
            ),
            None => field.into(),
        });
        about.into()
    });
    scrollable(Column::with_children(items).spacing(14).padding(PAD).width(Fill))
        .height(Fill)
        .into()
}
