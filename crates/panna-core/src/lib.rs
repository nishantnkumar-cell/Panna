//! Document loading and page rendering for the PDF viewer.
//!
//! Everything in this crate treats PDF bytes as untrusted input: sizes are bounded by
//! [`Limits`], and engine panics are caught and turned into errors.

#![forbid(unsafe_code)]

mod document;
mod limits;
mod outline;
mod text;
mod worker;

pub use document::{Document, DocumentInfo, OpenError, PageSize, RenderError, RenderedPage};
pub use limits::Limits;
pub use outline::OutlineItem;
pub use text::{FindOptions, PageText, TextBox};
pub use worker::RenderWorker;
