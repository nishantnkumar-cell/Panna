//! The Signatures tab: each digital signature in the document, how it checks out, and what
//! its certificate says about the signer, and whether it leads to a certificate the user
//! trusts.

use iced::widget::{Column, button, column, row, scrollable, text};
use iced::{Center, Element, Fill};
use panna_core::{Cert, SigStatus, Signature, trusted_by};

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

pub fn panel<'a>(sigs: &'a [Signature], trusted: &'a [Cert]) -> Element<'a, Message> {
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
        let trust = trusted_by(sig, trusted);
        let mut trust_button = None;
        match trust {
            Some(cert) => {
                lines.push(
                    if sig
                        .chain
                        .first()
                        .is_some_and(|c| c.fingerprint == cert.fingerprint)
                    {
                        "Trusted: you've said you trust this certificate.".to_owned()
                    } else {
                        format!(
                            "Trusted: the certificate leads to {}, which you trust.",
                            cert.name
                        )
                    },
                );
                trust_button = Some(
                    button(text(format!("Stop trusting {}", cert.name)).size(12))
                        .padding([2, 8])
                        .style(button::secondary)
                        .on_press(Message::Trust(crate::trust::Event::Remove(
                            cert.fingerprint.clone(),
                        ))),
                );
            }
            None => {
                lines.push(if sig.issuer.is_empty() {
                    "The signer made their own certificate, so who they are isn't confirmed. Trust \
                     it only if you know it's theirs."
                        .into()
                } else {
                    format!(
                        "Certificate issued by {}, which you haven't said you trust.",
                        sig.issuer
                    )
                });
                // The highest certificate the signature carries: the signer's own, or the
                // authority that issued it.
                if let Some(top) = sig.chain.last().filter(|_| sig.status == SigStatus::Valid) {
                    trust_button = Some(
                        button(text(format!("Trust {}", top.name)).size(12))
                            .padding([2, 8])
                            .style(button::secondary)
                            .on_press(Message::Trust(crate::trust::Event::Add(top.clone()))),
                    );
                }
            }
        }
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
        if let Some(b) = trust_button {
            about = about.push(b);
        }
        about.into()
    });
    let count = match trusted.len() {
        0 => "You don't trust any certificates yet.".to_owned(),
        1 => "You trust 1 certificate.".to_owned(),
        n => format!("You trust {n} certificates."),
    };
    let footer = column![
        text(count).size(12).style(text::secondary),
        button(text("Add a trusted certificate…").size(12))
            .padding([2, 8])
            .style(button::secondary)
            .on_press(Message::Trust(crate::trust::Event::Import)),
    ]
    .spacing(4);
    scrollable(
        Column::with_children(items)
            .push(footer)
            .spacing(14)
            .padding(PAD)
            .width(Fill),
    )
    .height(Fill)
    .into()
}
