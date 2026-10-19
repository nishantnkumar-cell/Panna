//! Document loading and page rendering for the PDF viewer.
//!
//! Everything in this crate treats PDF bytes as untrusted input: sizes are bounded by
//! [`Limits`], and engine panics are caught and turned into errors.

#![forbid(unsafe_code)]

mod access;
mod attachments;
mod calc;
mod commentdata;
mod comments;
mod compare;
mod detect;
mod document;
mod edit;
mod export;
mod formats;
mod formdata;
mod forms;
mod layers;
mod limits;
mod links;
mod outline;
mod preflight;
mod sig;
mod text;
mod worker;

pub use access::{AccessFinding, AccessRule};
pub use attachments::Attachment;
pub use calc::{CalcOp, Calculation, Expr, Formula, number as calc_number, value_text as calc_value_text};
pub use comments::{Comment, CommentId, CommentKind, ObjRef, ReviewState};
pub use compare::{Change, ChangeKind};
pub use document::{Document, DocumentInfo, OpenError, PageSize, RenderError, RenderedPage};
pub use edit::{
    Addition, Annotation, Changes, Cleanup, Color, HeaderFooter, Kind, LINE_HEIGHT, MAX_PICTURES, Margins,
    NOTE_SIZE, NewField, NewFieldKind, PageSource, PictureChange, Protection, STAMPS, SaveError, StampImage,
    Watermark, fill_header_footer, pictures_to_pdf, stamp_label, text_width, today, wrap,
};
pub use export::{Export, ExportFormat};
pub use formats::Format;
pub use formdata::{DataFormat, export as export_form_data, import as import_form_data};
pub use forms::{ChoiceOption, Field, FieldKind, FieldValue, Widget};
pub use layers::Layer;
pub use limits::Limits;
pub use links::{Link, LinkTarget, safe_uri};
pub use outline::OutlineItem;
pub use preflight::{Finding, Rule};
pub use sig::{SigStatus, Signature};
pub use text::{FindOptions, PageText, TextBox};
pub use worker::{PageEdit, RenderWorker};
