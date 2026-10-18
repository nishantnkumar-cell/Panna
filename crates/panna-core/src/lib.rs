//! Document loading and page rendering for the PDF viewer.
//!
//! Everything in this crate treats PDF bytes as untrusted input: sizes are bounded by
//! [`Limits`], and engine panics are caught and turned into errors.

#![forbid(unsafe_code)]

mod comments;
mod document;
mod edit;
mod formats;
mod forms;
mod limits;
mod links;
mod outline;
mod text;
mod worker;

pub use comments::{Comment, CommentId, CommentKind, ObjRef, ReviewState};
pub use document::{Document, DocumentInfo, OpenError, PageSize, RenderError, RenderedPage};
pub use edit::{
    Annotation, Changes, Color, Kind, LINE_HEIGHT, NOTE_SIZE, PageSource, STAMPS, SaveError, stamp_label,
    text_width, wrap,
};
pub use formats::Format;
pub use forms::{ChoiceOption, Field, FieldKind, FieldValue, Widget};
pub use limits::Limits;
pub use links::{Link, LinkTarget, safe_uri};
pub use outline::OutlineItem;
pub use text::{FindOptions, PageText, TextBox};
pub use worker::{PageEdit, RenderWorker};
