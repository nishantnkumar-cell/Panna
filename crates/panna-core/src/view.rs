//! How a document asks to be shown when it opens: its page, zoom, panel and page layout,
//! whether full screen, and whether its title replaces the file name.

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Dict, Name};
use serde::{Deserialize, Serialize};

/// The zoom a document opens at.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum OpenZoom {
    /// Whatever the viewer would use.
    #[default]
    Default,
    FitPage,
    FitWidth,
    /// This much, 1.0 being actual size.
    Scale(f32),
}

/// The panel shown beside the pages when a document opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OpenPanel {
    /// Just the pages.
    #[default]
    None,
    Bookmarks,
    Pages,
    Attachments,
    Layers,
}

/// How pages are laid out when a document opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OpenLayout {
    /// Whatever the viewer would use.
    #[default]
    Default,
    /// One page per row.
    Single,
    /// Two per row: 1–2, 3–4, …
    Two,
    /// Two per row with the first page alone, as a book's cover.
    TwoCover,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct InitialView {
    /// Zero-based.
    pub page: usize,
    pub zoom: OpenZoom,
    pub panel: OpenPanel,
    pub layout: OpenLayout,
    pub full_screen: bool,
    /// The window shows the document's title rather than its file name.
    pub show_title: bool,
}

impl InitialView {
    /// What `pdf` asks for.
    pub(crate) fn read(pdf: &Pdf) -> Self {
        let mut view = Self::default();
        if let Some((page, kind, zoom)) = crate::outline::opening(pdf) {
            view.page = page;
            view.zoom = match (kind.as_str(), zoom) {
                ("Fit" | "FitB", _) => OpenZoom::FitPage,
                ("FitH" | "FitBH", _) => OpenZoom::FitWidth,
                ("XYZ", Some(zoom)) => OpenZoom::Scale(zoom),
                _ => OpenZoom::Default,
            };
        }
        let xref = pdf.xref();
        let Some(catalog) = xref.get::<Dict<'_>>(xref.root_id()) else {
            return view;
        };
        let name = |key: &[u8]| catalog.get::<Name<'_>>(key).map(|n| n.as_str().to_owned());
        match name(b"PageMode").as_deref() {
            Some("UseOutlines") => view.panel = OpenPanel::Bookmarks,
            Some("UseThumbs") => view.panel = OpenPanel::Pages,
            Some("UseAttachments") => view.panel = OpenPanel::Attachments,
            Some("UseOC") => view.panel = OpenPanel::Layers,
            Some("FullScreen") => view.full_screen = true,
            _ => {}
        }
        view.layout = match name(b"PageLayout").as_deref() {
            Some("SinglePage" | "OneColumn") => OpenLayout::Single,
            Some("TwoColumnLeft" | "TwoPageLeft") => OpenLayout::Two,
            Some("TwoColumnRight" | "TwoPageRight") => OpenLayout::TwoCover,
            _ => OpenLayout::Default,
        };
        view.show_title = catalog
            .get::<Dict<'_>>(b"ViewerPreferences")
            .and_then(|prefs| prefs.get::<bool>(b"DisplayDocTitle"))
            .unwrap_or(false);
        view
    }

    /// The catalog entries that ask for this view, after the page references in `pages`.
    pub(crate) fn entries(&self, pages: &[String]) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        let page = pages.get(self.page).or(pages.first());
        if let Some(page) = page
            && (self.page > 0 || self.zoom != OpenZoom::Default)
        {
            let dest = match self.zoom {
                OpenZoom::Default => format!("[{page} /XYZ null null null]"),
                OpenZoom::FitPage => format!("[{page} /Fit]"),
                OpenZoom::FitWidth => format!("[{page} /FitH null]"),
                OpenZoom::Scale(zoom) => format!("[{page} /XYZ null null {zoom}]"),
            };
            out.push(("OpenAction", dest));
        }
        let mode = match (self.full_screen, self.panel) {
            (true, _) => Some("FullScreen"),
            (false, OpenPanel::Bookmarks) => Some("UseOutlines"),
            (false, OpenPanel::Pages) => Some("UseThumbs"),
            (false, OpenPanel::Attachments) => Some("UseAttachments"),
            (false, OpenPanel::Layers) => Some("UseOC"),
            (false, OpenPanel::None) => None,
        };
        if let Some(mode) = mode {
            out.push(("PageMode", format!("/{mode}")));
        }
        let layout = match self.layout {
            OpenLayout::Default => None,
            OpenLayout::Single => Some("OneColumn"),
            OpenLayout::Two => Some("TwoColumnLeft"),
            OpenLayout::TwoCover => Some("TwoColumnRight"),
        };
        if let Some(layout) = layout {
            out.push(("PageLayout", format!("/{layout}")));
        }
        out
    }
}
