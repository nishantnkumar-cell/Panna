//! Toolbar icons from Lucide (ISC license, see `icons/LICENSE-lucide`), embedded in the binary.
//!
//! Every icon button carries a hover tooltip naming the action and its shortcut, so the
//! toolbar needs no words.

use iced::widget::{self, container, svg, text, tooltip};
use iced::{Element, Length, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Open,
    PrevPage,
    NextPage,
    ZoomIn,
    ZoomOut,
    FitWidth,
    FitPage,
    RotateCcw,
    RotateCw,
    Properties,
    GripVertical,
    GripHorizontal,
    DockMenu,
    DockTop,
    DockBottom,
    DockLeft,
    DockRight,
    Float,
    Sidebar,
    SwapSide,
    Close,
    Search,
    FindPrev,
    FindNext,
    MatchCase,
    WholeWord,
    Pointer,
    Highlighter,
    Underline,
    Strikethrough,
    StickyNote,
    TextBox,
    Pen,
    Square,
    Circle,
    Arrow,
    Line,
    Undo,
    Redo,
    Save,
    Annotate,
    Delete,
    Check,
    Eraser,
    SinglePage,
    TwoPages,
    CoverPage,
    Polygon,
    Cloud,
    Reply,
    Moon,
    FullScreen,
    Present,
    Stamp,
    PageAdd,
    PageInsertFile,
    PageReplace,
    PageLabels,
    CommentSummary,
    PrintComments,
    Preferences,
    PageExtract,
    MoveUp,
    MoveDown,
    Print,
    Crop,
    PageNumbers,
    Watermark,
    Export,
    FromPictures,
    NewBlank,
    FromClipboard,
    Flatten,
    Protect,
    Clean,
    AllTools,
    Paperclip,
    Signed,
    SignatureProblem,
    SignatureUnknown,
    Camera,
    Layers,
    Ruler,
    Pentagon,
    Area,
    DataExport,
    CommentsImport,
    CommentsExport,
    Signature,
    Calendar,
    AddPicture,
    AddLink,
    EditPage,
    Shrink,
    ScanText,
    Scanner,
    Redact,
    MarkResults,
    EditPictures,
    ReplacePicture,
    ReadAloud,
    StopReading,
    DetectFields,
    Preflight,
    Accessibility,
    Compare,
    PrepareForm,
    TextField,
    CheckBoxField,
    DropdownField,
    ListField,
    RadioField,
    DataImport,
    EditText,
    BookmarkAdd,
    Rename,
    Indent,
    Outdent,
    Hand,
    ZoomBox,
}

impl Icon {
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Icon::Open => include_bytes!("../icons/folder-open.svg"),
            Icon::PrevPage => include_bytes!("../icons/chevron-left.svg"),
            Icon::NextPage => include_bytes!("../icons/chevron-right.svg"),
            Icon::ZoomIn => include_bytes!("../icons/zoom-in.svg"),
            Icon::ZoomOut => include_bytes!("../icons/zoom-out.svg"),
            Icon::FitWidth => include_bytes!("../icons/move-horizontal.svg"),
            Icon::FitPage => include_bytes!("../icons/maximize.svg"),
            Icon::RotateCcw => include_bytes!("../icons/rotate-ccw.svg"),
            Icon::RotateCw => include_bytes!("../icons/rotate-cw.svg"),
            Icon::Properties => include_bytes!("../icons/info.svg"),
            Icon::GripVertical => include_bytes!("../icons/grip-vertical.svg"),
            Icon::GripHorizontal => include_bytes!("../icons/grip-horizontal.svg"),
            Icon::DockMenu => include_bytes!("../icons/pin.svg"),
            Icon::DockTop => include_bytes!("../icons/panel-top.svg"),
            Icon::DockBottom => include_bytes!("../icons/panel-bottom.svg"),
            Icon::DockLeft => include_bytes!("../icons/panel-left.svg"),
            Icon::DockRight => include_bytes!("../icons/panel-right.svg"),
            Icon::Float => include_bytes!("../icons/picture-in-picture-2.svg"),
            Icon::Sidebar => include_bytes!("../icons/panel-left.svg"),
            Icon::SwapSide => include_bytes!("../icons/arrow-left-right.svg"),
            Icon::Close => include_bytes!("../icons/x.svg"),
            Icon::Search => include_bytes!("../icons/search.svg"),
            Icon::FindPrev => include_bytes!("../icons/chevron-up.svg"),
            Icon::FindNext => include_bytes!("../icons/chevron-down.svg"),
            Icon::MatchCase => include_bytes!("../icons/case-sensitive.svg"),
            Icon::WholeWord => include_bytes!("../icons/whole-word.svg"),
            Icon::Pointer => include_bytes!("../icons/mouse-pointer-2.svg"),
            Icon::Highlighter => include_bytes!("../icons/highlighter.svg"),
            Icon::Underline => include_bytes!("../icons/underline.svg"),
            Icon::Strikethrough => include_bytes!("../icons/strikethrough.svg"),
            Icon::StickyNote => include_bytes!("../icons/sticky-note.svg"),
            Icon::TextBox => include_bytes!("../icons/type.svg"),
            Icon::Pen => include_bytes!("../icons/pen-line.svg"),
            Icon::Square => include_bytes!("../icons/square.svg"),
            Icon::Circle => include_bytes!("../icons/circle.svg"),
            Icon::Arrow => include_bytes!("../icons/move-up-right.svg"),
            Icon::Line => include_bytes!("../icons/slash.svg"),
            Icon::Undo => include_bytes!("../icons/undo-2.svg"),
            Icon::Redo => include_bytes!("../icons/redo-2.svg"),
            Icon::Save => include_bytes!("../icons/save.svg"),
            Icon::Annotate => include_bytes!("../icons/message-square-plus.svg"),
            Icon::Delete => include_bytes!("../icons/trash-2.svg"),
            Icon::Check => include_bytes!("../icons/check.svg"),
            Icon::Eraser => include_bytes!("../icons/eraser.svg"),
            Icon::SinglePage => include_bytes!("../icons/file.svg"),
            Icon::TwoPages => include_bytes!("../icons/book-open.svg"),
            Icon::CoverPage => include_bytes!("../icons/book.svg"),
            Icon::Polygon => include_bytes!("../icons/pentagon.svg"),
            Icon::Cloud => include_bytes!("../icons/cloud.svg"),
            Icon::Reply => include_bytes!("../icons/reply.svg"),
            Icon::Moon => include_bytes!("../icons/moon.svg"),
            Icon::FullScreen => include_bytes!("../icons/fullscreen.svg"),
            Icon::Present => include_bytes!("../icons/presentation.svg"),
            Icon::Stamp => include_bytes!("../icons/stamp.svg"),
            Icon::PageAdd => include_bytes!("../icons/file-plus.svg"),
            Icon::PageInsertFile => include_bytes!("../icons/file-input.svg"),
            Icon::PageReplace => include_bytes!("../icons/replace.svg"),
            Icon::PageLabels => include_bytes!("../icons/list-ordered.svg"),
            Icon::CommentSummary => include_bytes!("../icons/message-square-text.svg"),
            Icon::PrintComments => include_bytes!("../icons/printer-check.svg"),
            Icon::Preferences => include_bytes!("../icons/settings.svg"),
            Icon::PageExtract => include_bytes!("../icons/file-output.svg"),
            Icon::MoveUp => include_bytes!("../icons/arrow-up.svg"),
            Icon::MoveDown => include_bytes!("../icons/arrow-down.svg"),
            Icon::Print => include_bytes!("../icons/printer.svg"),
            Icon::Crop => include_bytes!("../icons/crop.svg"),
            Icon::PageNumbers => include_bytes!("../icons/file-digit.svg"),
            Icon::Watermark => include_bytes!("../icons/file-type.svg"),
            Icon::Export => include_bytes!("../icons/file-output.svg"),
            Icon::FromPictures => include_bytes!("../icons/file-image.svg"),
            Icon::NewBlank => include_bytes!("../icons/file.svg"),
            Icon::FromClipboard => include_bytes!("../icons/clipboard-paste.svg"),
            Icon::Flatten => include_bytes!("../icons/layers.svg"),
            Icon::Protect => include_bytes!("../icons/lock.svg"),
            Icon::Clean => include_bytes!("../icons/shield-check.svg"),
            Icon::AllTools => include_bytes!("../icons/layout-grid.svg"),
            Icon::Paperclip => include_bytes!("../icons/paperclip.svg"),
            Icon::Signed => include_bytes!("../icons/badge-check.svg"),
            Icon::SignatureProblem => include_bytes!("../icons/shield-alert.svg"),
            Icon::SignatureUnknown => include_bytes!("../icons/shield-question.svg"),
            Icon::Camera => include_bytes!("../icons/camera.svg"),
            Icon::Layers => include_bytes!("../icons/layers-2.svg"),
            Icon::Ruler => include_bytes!("../icons/ruler.svg"),
            Icon::Pentagon => include_bytes!("../icons/pentagon.svg"),
            Icon::Area => include_bytes!("../icons/land-plot.svg"),
            Icon::DataExport => include_bytes!("../icons/file-down.svg"),
            Icon::CommentsImport => include_bytes!("../icons/import.svg"),
            Icon::Signature => include_bytes!("../icons/signature.svg"),
            Icon::Calendar => include_bytes!("../icons/calendar.svg"),
            Icon::AddPicture => include_bytes!("../icons/image-plus.svg"),
            Icon::AddLink => include_bytes!("../icons/link.svg"),
            Icon::EditPage => include_bytes!("../icons/file-pen-line.svg"),
            Icon::Shrink => include_bytes!("../icons/shrink.svg"),
            Icon::ScanText => include_bytes!("../icons/scan-text.svg"),
            Icon::Scanner => include_bytes!("../icons/scanner.svg"),
            Icon::Redact => include_bytes!("../icons/eye-off.svg"),
            Icon::MarkResults => include_bytes!("../icons/text-search.svg"),
            Icon::EditPictures => include_bytes!("../icons/images.svg"),
            Icon::ReplacePicture => include_bytes!("../icons/replace.svg"),
            Icon::ReadAloud => include_bytes!("../icons/volume-2.svg"),
            Icon::StopReading => include_bytes!("../icons/circle-stop.svg"),
            Icon::DetectFields => include_bytes!("../icons/wand-sparkles.svg"),
            Icon::Preflight => include_bytes!("../icons/file-check.svg"),
            Icon::Accessibility => include_bytes!("../icons/accessibility.svg"),
            Icon::Compare => include_bytes!("../icons/file-diff.svg"),
            Icon::PrepareForm => include_bytes!("../icons/clipboard-pen-line.svg"),
            Icon::TextField => include_bytes!("../icons/text-cursor-input.svg"),
            Icon::CheckBoxField => include_bytes!("../icons/square-check.svg"),
            Icon::DropdownField => include_bytes!("../icons/square-chevron-down.svg"),
            Icon::ListField => include_bytes!("../icons/list.svg"),
            Icon::RadioField => include_bytes!("../icons/circle-dot.svg"),
            Icon::CommentsExport => include_bytes!("../icons/message-square-share.svg"),
            Icon::DataImport => include_bytes!("../icons/file-up.svg"),
            Icon::EditText => include_bytes!("../icons/text-cursor-input.svg"),
            Icon::BookmarkAdd => include_bytes!("../icons/bookmark-plus.svg"),
            Icon::Rename => include_bytes!("../icons/pencil.svg"),
            Icon::Indent => include_bytes!("../icons/indent-increase.svg"),
            Icon::Outdent => include_bytes!("../icons/indent-decrease.svg"),
            Icon::Hand => include_bytes!("../icons/hand.svg"),
            Icon::ZoomBox => include_bytes!("../icons/scan-search.svg"),
        }
    }
}

/// The platform's main shortcut modifier, as users see it on their keyboard.
pub const MOD: &str = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };

const ICON_SIZE: f32 = 18.0;

/// A bare icon tinted with the theme's text colour (dimmed when `enabled` is false).
pub fn glyph<'a, M: 'a>(icon: Icon, enabled: bool) -> Element<'a, M> {
    svg(svg::Handle::from_memory(icon.bytes()))
        .width(ICON_SIZE)
        .height(ICON_SIZE)
        // The tint's alpha is ignored by the software renderer, so dimming uses opacity.
        .opacity(if enabled { 1.0_f32 } else { 0.35 })
        .style(|theme: &Theme, _status| svg::Style {
            color: Some(theme.palette().text),
        })
        .into()
}

/// An icon button with a hover tooltip. `on_press: None` shows it disabled.
pub fn button<'a, M: Clone + 'a>(icon: Icon, tip: impl Into<String>, on_press: Option<M>) -> Element<'a, M> {
    styled(icon, tip, on_press, false)
}

/// An on/off icon button, shown pressed in while `active`.
pub fn toggle<'a, M: Clone + 'a>(
    icon: Icon,
    tip: impl Into<String>,
    active: bool,
    on_press: M,
) -> Element<'a, M> {
    styled(icon, tip, Some(on_press), active)
}

fn styled<'a, M: Clone + 'a>(
    icon: Icon,
    tip: impl Into<String>,
    on_press: Option<M>,
    active: bool,
) -> Element<'a, M> {
    let enabled = on_press.is_some();
    let button = widget::button(glyph(icon, enabled))
        .padding(6)
        .width(Length::Shrink)
        .style(move |theme: &Theme, status| {
            use widget::button::Status;
            let palette = theme.extended_palette();
            let background = match status {
                Status::Hovered if enabled => Some(palette.background.weak.color.into()),
                Status::Pressed if enabled => Some(palette.background.strong.color.into()),
                _ if active => Some(palette.primary.weak.color.into()),
                _ => None,
            };
            widget::button::Style {
                background,
                text_color: palette.background.base.text,
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .on_press_maybe(on_press);
    with_tip(button, tip)
}

/// Wraps any element in a hover tooltip.
pub fn with_tip<'a, M: 'a>(content: impl Into<Element<'a, M>>, tip: impl Into<String>) -> Element<'a, M> {
    let label = container(text(tip.into()).size(13))
        .padding([4, 8])
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style::default()
                .background(palette.background.strong.color)
                .color(palette.background.strong.text)
                .border(iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                })
        });
    tooltip(content, label, tooltip::Position::Bottom)
        .gap(4)
        .snap_within_viewport(true)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Icon; 120] = [
        Icon::Open,
        Icon::PrevPage,
        Icon::NextPage,
        Icon::ZoomIn,
        Icon::ZoomOut,
        Icon::FitWidth,
        Icon::FitPage,
        Icon::RotateCcw,
        Icon::RotateCw,
        Icon::Properties,
        Icon::GripVertical,
        Icon::GripHorizontal,
        Icon::DockMenu,
        Icon::DockTop,
        Icon::DockBottom,
        Icon::DockLeft,
        Icon::DockRight,
        Icon::Float,
        Icon::Sidebar,
        Icon::SwapSide,
        Icon::Close,
        Icon::Search,
        Icon::FindPrev,
        Icon::FindNext,
        Icon::MatchCase,
        Icon::WholeWord,
        Icon::Pointer,
        Icon::Highlighter,
        Icon::Underline,
        Icon::Strikethrough,
        Icon::StickyNote,
        Icon::TextBox,
        Icon::Pen,
        Icon::Square,
        Icon::Circle,
        Icon::Arrow,
        Icon::Line,
        Icon::Undo,
        Icon::Redo,
        Icon::Save,
        Icon::Annotate,
        Icon::Delete,
        Icon::Check,
        Icon::Eraser,
        Icon::SinglePage,
        Icon::TwoPages,
        Icon::CoverPage,
        Icon::Polygon,
        Icon::Cloud,
        Icon::Reply,
        Icon::Moon,
        Icon::FullScreen,
        Icon::Present,
        Icon::Stamp,
        Icon::PageAdd,
        Icon::PageInsertFile,
        Icon::PageReplace,
        Icon::PageLabels,
        Icon::CommentSummary,
        Icon::PrintComments,
        Icon::Preferences,
        Icon::PageExtract,
        Icon::MoveUp,
        Icon::MoveDown,
        Icon::Print,
        Icon::Crop,
        Icon::PageNumbers,
        Icon::Watermark,
        Icon::Export,
        Icon::FromPictures,
        Icon::NewBlank,
        Icon::FromClipboard,
        Icon::Flatten,
        Icon::Protect,
        Icon::Clean,
        Icon::AllTools,
        Icon::Paperclip,
        Icon::Signed,
        Icon::SignatureProblem,
        Icon::SignatureUnknown,
        Icon::Camera,
        Icon::Layers,
        Icon::Ruler,
        Icon::Pentagon,
        Icon::Area,
        Icon::DataExport,
        Icon::CommentsImport,
        Icon::CommentsExport,
        Icon::Signature,
        Icon::Calendar,
        Icon::AddPicture,
        Icon::AddLink,
        Icon::EditPage,
        Icon::Shrink,
        Icon::ScanText,
        Icon::Scanner,
        Icon::Redact,
        Icon::MarkResults,
        Icon::EditPictures,
        Icon::ReplacePicture,
        Icon::ReadAloud,
        Icon::StopReading,
        Icon::DetectFields,
        Icon::Preflight,
        Icon::Accessibility,
        Icon::Compare,
        Icon::PrepareForm,
        Icon::TextField,
        Icon::CheckBoxField,
        Icon::DropdownField,
        Icon::ListField,
        Icon::RadioField,
        Icon::DataImport,
        Icon::EditText,
        Icon::BookmarkAdd,
        Icon::Rename,
        Icon::Indent,
        Icon::Outdent,
        Icon::Hand,
        Icon::ZoomBox,
    ];

    #[test]
    fn every_icon_is_a_self_contained_svg() {
        for icon in ALL {
            let svg = std::str::from_utf8(icon.bytes()).unwrap();
            assert!(svg.contains("<svg"), "{icon:?}");
            assert!(
                svg.contains("currentColor"),
                "{icon:?} must follow the theme colour"
            );
            // No scripts or external references in embedded assets.
            assert!(!svg.contains("<script") && !svg.contains("href"), "{icon:?}");
        }
    }
}
