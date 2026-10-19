//! Checking how well a document works with screen readers and other assistive technology,
//! along the lines of Acrobat's accessibility checker: tags giving its structure and
//! reading order, a language and a title, descriptions for figures and form fields, table
//! headers, and text that maps to Unicode.

use std::collections::{HashMap, HashSet};

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, Name, Object, Stream};

/// Structure elements, fonts and drawings looked at at most, so a hostile file stays quick
/// to check.
const MAX_VISITS: usize = 50_000;
/// How deep structure elements and drawings inside drawings are followed.
const MAX_DEPTH: u16 = 64;
/// Findings of one kind listed at most.
const MAX_PER_RULE: usize = 20;
/// Documents longer than this should have bookmarks.
const BOOKMARKS_AFTER: usize = 20;

/// What a rule is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum AccessRule {
    NotTagged,
    NoLanguage,
    NoTitle,
    TitleNotShown,
    NoBookmarks,
    ImageOnly,
    FigureNoAlt,
    TableNoHeaders,
    FieldNoDescription,
    AnnotationNotTagged,
    TabOrder,
    NoUnicode,
}

impl AccessRule {
    /// What the rule asks, for people.
    pub fn title(self) -> &'static str {
        match self {
            AccessRule::NotTagged => "The document must be tagged, so its structure can be read",
            AccessRule::NoLanguage => "The document's language must be set",
            AccessRule::NoTitle => "The document must have a title",
            AccessRule::TitleNotShown => "The title, not the file name, should be shown",
            AccessRule::NoBookmarks => "Long documents should have bookmarks",
            AccessRule::ImageOnly => "Pages must have real text, not just pictures of it",
            AccessRule::FigureNoAlt => "Figures need alternate text",
            AccessRule::TableNoHeaders => "Tables need header cells",
            AccessRule::FieldNoDescription => "Form fields need a description (tooltip)",
            AccessRule::AnnotationNotTagged => "Comments, links and fields must be tagged",
            AccessRule::TabOrder => "Tab order must follow the document's structure",
            AccessRule::NoUnicode => "Text must map to Unicode characters",
        }
    }
}

/// One thing that makes the document harder to use with assistive technology.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AccessFinding {
    pub rule: AccessRule,
    /// Where or what, such as a field's name.
    pub detail: String,
    pub page: Option<usize>,
    /// For a figure without a description, its structure element (object number and
    /// generation), to give one to with [`crate::Document::described`].
    pub element: Option<(u32, u16)>,
}

/// What the checker can't read from the objects alone.
pub(crate) struct Known<'a> {
    pub title: bool,
    pub bookmarks: usize,
    /// Pages that draw pictures but have no text.
    pub image_only: &'a [usize],
    /// Form fields with no description, with the page of their first widget.
    pub undescribed: &'a [(String, usize)],
}

struct Check {
    out: Vec<AccessFinding>,
    seen: HashSet<(i32, i32)>,
    visits: usize,
    pages: HashMap<(i32, i32), usize>,
    roles: HashMap<String, String>,
}

impl Check {
    fn add(&mut self, rule: AccessRule, detail: impl Into<String>, page: Option<usize>) {
        self.add_for(rule, detail, page, None);
    }

    fn add_for(
        &mut self,
        rule: AccessRule,
        detail: impl Into<String>,
        page: Option<usize>,
        element: Option<(u32, u16)>,
    ) {
        let detail = detail.into();
        let same = self.out.iter().filter(|f| f.rule == rule).count();
        if same < MAX_PER_RULE
            && !self
                .out
                .iter()
                .any(|f| f.rule == rule && f.detail == detail && f.page == page && f.element == element)
        {
            self.out.push(AccessFinding {
                rule,
                detail,
                page,
                element,
            });
        }
    }

    /// Whether `dict` is new to the check, and the check still has room.
    fn first_visit(&mut self, dict: &Dict<'_>) -> bool {
        self.visits += 1;
        if self.visits > MAX_VISITS {
            return false;
        }
        match dict.obj_id() {
            Some(id) => self.seen.insert((id.obj_number, id.gen_number)),
            None => true,
        }
    }

    /// A structure type as a standard one, through the document's role map.
    fn standard(&self, kind: &str) -> String {
        let mut kind = kind.to_owned();
        for _ in 0..8 {
            match self.roles.get(&kind) {
                Some(next) if *next != kind => kind = next.clone(),
                _ => break,
            }
        }
        kind
    }

    fn page_of(&self, elem: &Dict<'_>) -> Option<usize> {
        let r = elem.get_ref(b"Pg")?;
        self.pages.get(&(r.obj_number, r.gen_number)).copied()
    }

    /// Looks at a structure element and what's inside it; says whether a header cell was
    /// among them.
    fn element(&mut self, elem: &Dict<'_>, page: Option<usize>, depth: u16) -> bool {
        if depth > MAX_DEPTH || !self.first_visit(elem) {
            return false;
        }
        let page = self.page_of(elem).or(page);
        let kind = elem
            .get::<Name<'_>>(b"S")
            .map(|n| self.standard(n.as_str()))
            .unwrap_or_default();
        let described = |key: &[u8]| {
            elem.get::<Object<'_>>(key).is_some_and(|o| match o {
                Object::String(s) => !crate::document::decode_text_string(s.as_bytes())
                    .trim()
                    .is_empty(),
                _ => false,
            })
        };
        if matches!(kind.as_str(), "Figure" | "Formula") && !described(b"Alt") && !described(b"ActualText") {
            let what = if kind == "Figure" { "A figure" } else { "A formula" };
            let element = elem.obj_id().and_then(|id| {
                Some((
                    u32::try_from(id.obj_number).ok()?,
                    u16::try_from(id.gen_number).ok()?,
                ))
            });
            self.add_for(AccessRule::FigureNoAlt, what, page, element);
        }
        let mut header = kind == "TH";
        let mut children = Vec::new();
        match elem.get::<Object<'_>>(b"K") {
            Some(Object::Dict(d)) => children.push(d),
            Some(Object::Array(a)) => children.extend(a.iter::<Dict<'_>>()),
            _ => {}
        }
        for child in children {
            // Marked content and object references have no type of their own.
            if child.contains_key(b"S") {
                header |= self.element(&child, page, depth + 1);
            }
        }
        if kind == "Table" && !header {
            self.add(AccessRule::TableNoHeaders, "A table", page);
        }
        header
    }

    fn resources(&mut self, res: &Dict<'_>, page: usize, depth: u16) {
        if depth > MAX_DEPTH {
            return;
        }
        if let Some(fonts) = res.get::<Dict<'_>>(b"Font") {
            for key in fonts.keys().collect::<Vec<_>>() {
                if let Some(font) = fonts.get::<Dict<'_>>(key.as_ref()) {
                    self.font(&font, page);
                }
            }
        }
        if let Some(xobjects) = res.get::<Dict<'_>>(b"XObject") {
            for key in xobjects.keys().collect::<Vec<_>>() {
                let Some(stream) = xobjects.get::<Stream<'_>>(key.as_ref()) else {
                    continue;
                };
                let dict = stream.dict().clone();
                if self.first_visit(&dict)
                    && let Some(inner) = dict.get::<Dict<'_>>(b"Resources")
                {
                    self.resources(&inner, page, depth + 1);
                }
            }
        }
    }

    /// Composite fonts with glyph numbers for codes, and Type 3 fonts, say nothing about
    /// which characters they draw unless they carry a /ToUnicode map.
    fn font(&mut self, font: &Dict<'_>, page: usize) {
        if !self.first_visit(font) || font.contains_key(b"ToUnicode") {
            return;
        }
        let subtype = font.get::<Name<'_>>(b"Subtype").map(|n| n.as_str().to_owned());
        let unmapped = match subtype.as_deref() {
            Some("Type3") => true,
            Some("Type0") => match font.get::<Object<'_>>(b"Encoding") {
                Some(Object::Name(n)) => n.as_str().starts_with("Identity"),
                // An embedded CMap maps codes to glyphs, not characters.
                Some(Object::Stream(_)) => true,
                _ => false,
            },
            _ => false,
        };
        if unmapped {
            let name = font
                .get::<Name<'_>>(b"BaseFont")
                .map(|n| n.as_str().to_owned())
                .unwrap_or_else(|| "A font".into());
            let shown = name.split_once('+').map_or(name.as_str(), |(_, n)| n).to_owned();
            self.add(AccessRule::NoUnicode, shown, Some(page));
        }
    }
}

pub(crate) fn check(pdf: &Pdf, known: &Known<'_>) -> Vec<AccessFinding> {
    let mut c = Check {
        out: Vec::new(),
        seen: HashSet::new(),
        visits: 0,
        pages: HashMap::new(),
        roles: HashMap::new(),
    };
    let xref = pdf.xref();
    let catalog = xref.get::<Dict<'_>>(xref.root_id()).unwrap_or_default();
    let pages = pdf.pages();
    for (index, page) in pages.iter().enumerate() {
        if let Some(id) = page.raw().obj_id() {
            c.pages.insert((id.obj_number, id.gen_number), index);
        }
    }

    let marked = catalog
        .get::<Dict<'_>>(b"MarkInfo")
        .and_then(|m| m.get::<bool>(b"Marked"))
        .unwrap_or(false);
    let tree = catalog.get::<Dict<'_>>(b"StructTreeRoot");
    let tagged = marked && tree.is_some();
    match &tree {
        None => c.add(
            AccessRule::NotTagged,
            "No tags give its headings, paragraphs and reading order",
            None,
        ),
        Some(root) => {
            if !marked {
                c.add(
                    AccessRule::NotTagged,
                    "The document has tags but isn't marked as tagged",
                    None,
                );
            }
            if let Some(roles) = root.get::<Dict<'_>>(b"RoleMap") {
                for key in roles.keys().take(1_000).collect::<Vec<_>>() {
                    if let Some(to) = roles.get::<Name<'_>>(key.as_ref()) {
                        c.roles.insert(key.as_str().to_owned(), to.as_str().to_owned());
                    }
                }
            }
            let mut top = Vec::new();
            match root.get::<Object<'_>>(b"K") {
                Some(Object::Dict(d)) => top.push(d),
                Some(Object::Array(a)) => top.extend(a.iter::<Dict<'_>>()),
                _ => {}
            }
            if top.is_empty() {
                c.add(AccessRule::NotTagged, "The tags are empty", None);
            }
            for elem in top {
                c.element(&elem, None, 0);
            }
        }
    }

    let lang = catalog
        .get::<Object<'_>>(b"Lang")
        .is_some_and(|o| matches!(o, Object::String(s) if !s.as_bytes().iter().all(u8::is_ascii_whitespace)));
    if !lang {
        c.add(
            AccessRule::NoLanguage,
            "Screen readers need it to pronounce the text",
            None,
        );
    }
    if !known.title {
        c.add(
            AccessRule::NoTitle,
            "Neither the document information nor the metadata has one",
            None,
        );
    }
    let shows_title = catalog
        .get::<Dict<'_>>(b"ViewerPreferences")
        .and_then(|v| v.get::<bool>(b"DisplayDocTitle"))
        .unwrap_or(false);
    if !shows_title {
        c.add(
            AccessRule::TitleNotShown,
            "Viewers show the file name in the window title",
            None,
        );
    }
    if pages.len() > BOOKMARKS_AFTER && known.bookmarks == 0 {
        c.add(
            AccessRule::NoBookmarks,
            format!("{} pages and no bookmarks", pages.len()),
            None,
        );
    }
    for &page in known.image_only {
        c.add(
            AccessRule::ImageOnly,
            "Pictures but no text: possibly scanned",
            Some(page),
        );
    }
    for (name, page) in known.undescribed {
        c.add(AccessRule::FieldNoDescription, name.clone(), Some(*page));
    }

    for (index, page) in pages.iter().enumerate() {
        let res = page.resources();
        let fonts = res.fonts.clone();
        for key in fonts.keys().collect::<Vec<_>>() {
            if let Some(font) = fonts.get::<Dict<'_>>(key.as_ref()) {
                c.font(&font, index);
            }
        }
        let xobjects = res.x_objects.clone();
        for key in xobjects.keys().collect::<Vec<_>>() {
            let Some(stream) = xobjects.get::<Stream<'_>>(key.as_ref()) else {
                continue;
            };
            let dict = stream.dict().clone();
            if c.first_visit(&dict)
                && let Some(inner) = dict.get::<Dict<'_>>(b"Resources")
            {
                c.resources(&inner, index, 1);
            }
        }

        let raw = page.raw();
        let Some(annots) = raw.get::<Array<'_>>(b"Annots") else {
            continue;
        };
        let mut any = false;
        for annot in annots.iter::<Dict<'_>>().take(4_000) {
            let subtype = annot
                .get::<Name<'_>>(b"Subtype")
                .map(|n| n.as_str().to_owned())
                .unwrap_or_default();
            if subtype == "Popup" {
                continue;
            }
            any = true;
            if tagged && !annot.contains_key(b"StructParent") {
                let what = match subtype.as_str() {
                    "Link" => "A link",
                    "Widget" => "A form field",
                    _ => "A comment",
                };
                c.add(AccessRule::AnnotationNotTagged, what, Some(index));
            }
        }
        let by_structure = raw.get::<Name<'_>>(b"Tabs").is_some_and(|t| t.as_str() == "S");
        if any && !by_structure {
            c.add(
                AccessRule::TabOrder,
                "Tabbing goes through the page's links and fields in another order",
                Some(index),
            );
        }
    }
    c.out.sort_by_key(|f| (f.rule, f.page));
    c.out
}
