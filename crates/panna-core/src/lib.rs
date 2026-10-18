//! Document loading and page rendering for the PDF viewer.
//!
//! Everything in this crate treats PDF bytes as untrusted input: sizes are bounded by
//! [`Limits`], and engine panics are caught and turned into errors.

#![forbid(unsafe_code)]

mod comments;
mod document;
mod edit;
mod forms;
mod limits;
mod outline;
mod text;
mod worker;

pub use comments::{Comment, CommentId, CommentKind, ObjRef};
pub use document::{Document, DocumentInfo, OpenError, PageSize, RenderError, RenderedPage};
pub use edit::{Annotation, Changes, Color, Kind, LINE_HEIGHT, NOTE_SIZE, SaveError, text_width, wrap};
pub use forms::{ChoiceOption, Field, FieldKind, FieldValue, Widget};
pub use limits::Limits;
pub use outline::OutlineItem;
pub use text::{FindOptions, PageText, TextBox};
pub use worker::RenderWorker;
