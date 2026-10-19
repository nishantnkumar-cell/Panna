//! Document loading and page rendering for the PDF viewer.
//!
//! Everything in this crate treats PDF bytes as untrusted input: sizes are bounded by
//! [`Limits`], and engine panics are caught and turned into errors.

#![forbid(unsafe_code)]

mod access;
mod attachments;
pub mod batch;
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
mod guard;
pub mod isolate;
mod labels;
mod layers;
mod limits;
mod links;
mod ocr;
mod office;
mod outline;
mod preflight;
mod remote;
mod sig;
mod text;
mod view;
mod worker;

pub use access::{AccessFinding, AccessRule};
pub use attachments::Attachment;
pub use calc::{CalcOp, Calculation, Expr, Formula, number as calc_number, value_text as calc_value_text};
pub use comments::{Comment, CommentId, CommentKind, ObjRef, ReviewState};
pub use compare::{Change, ChangeKind};
pub use document::{Document, DocumentInfo, Leftovers, OpenError, PageSize, RenderError, RenderedPage};
pub use edit::{
    Addition, Annotation, Changes, Cleanup, Color, Corner, HeaderFooter, Kind, LINE_HEIGHT, MAX_BLANK_PAGES,
    MAX_PICTURES, Margins, NOTE_SIZE, NewField, NewFieldKind, PageSource, PictureChange, Protection,
    RadioButton, STAMPS, SaveError, SignOptions, StampImage, Visible, Watermark, blank_pdf,
    fill_header_footer, pictures_to_pdf, rgba_to_pdf, stamp_label, text_width, today, wrap,
};
pub use export::{Export, ExportFormat};
pub use formats::Format;
pub use formdata::{DataFormat, export as export_form_data, import as import_form_data};
pub use forms::{ChoiceOption, Field, FieldKind, FieldValue, Widget};
pub use labels::{LabelRange, LabelStyle, page_label};
pub use layers::Layer;
pub use limits::Limits;
pub use links::{Link, LinkTarget, safe_uri};
pub use outline::OutlineItem;
pub use preflight::{Finding, Rule, Standard};
pub use remote::Isolated;
pub use sig::{Cert, DigitalId, HeldKey, SigStatus, SignDigest, Signature, trusted_by};
pub use text::{FindOptions, PageText, Paragraph, TextBox};
pub use view::{InitialView, OpenLayout, OpenPanel, OpenZoom};
pub use worker::{PageEdit, RenderWorker, describe_file};
