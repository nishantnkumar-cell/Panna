//! Interface languages. Strings stay in English in the code; where they reach the screen
//! (Panna's own [`text`], menu items and icon tooltips) they are looked up in the chosen
//! language's catalogue. A catalogue entry with `{name}` placeholders also translates
//! formatted messages: "Page {n} of {count}" turns "Page 3 of 6" into the translation with 3
//! and 6 put back in. Anything not in the catalogue shows in English.
//!
//! Catalogues are plain text in `i18n/`, built into the program: each entry is the English
//! on one line and its translation on the next, starting `= `, with a blank line between
//! entries and `#` starting a comment. `\n` stands for a line break.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

/// Iced's text module, so `text::Wrapping` and the like still work where [`text`] is used.
pub mod text {
    pub use iced::widget::text::*;
}

/// A language the interface can be shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Language {
    #[default]
    English,
    Hindi,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::Hindi];

    /// The name kept in the settings.
    pub fn as_str(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Hindi => "hi",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == s)
    }

    fn catalogue(self) -> &'static str {
        match self {
            Language::English => "",
            Language::Hindi => include_str!("../i18n/hi.txt"),
        }
    }
}

impl std::fmt::Display for Language {
    /// Each language by its own name, so it can be found whatever language is showing.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Language::English => "English",
            Language::Hindi => "हिन्दी (Hindi)",
        })
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Shows the interface in `language` from the next drawing on.
pub fn set_language(language: Language) {
    let n = Language::ALL.iter().position(|l| *l == language).unwrap_or(0);
    CURRENT.store(n as u8, Ordering::Relaxed);
}

pub fn language() -> Language {
    Language::ALL
        .get(usize::from(CURRENT.load(Ordering::Relaxed)))
        .copied()
        .unwrap_or_default()
}

/// A formatted message: the literal pieces between its placeholders, in order, and the
/// placeholders' names.
#[derive(Debug, PartialEq)]
struct Template {
    pieces: Vec<String>,
    names: Vec<String>,
    translation: String,
}

impl Template {
    /// `english` cut at its `{name}` placeholders, if it has any.
    fn new(english: &str, translation: &str) -> Option<Template> {
        let mut pieces = vec![String::new()];
        let mut names = Vec::new();
        let mut rest = english;
        while let Some(open) = rest.find('{') {
            let close = rest.get(open..)?.find('}')? + open;
            let name = rest.get(open + 1..close)?;
            if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return None;
            }
            pieces.last_mut()?.push_str(rest.get(..open)?);
            pieces.push(String::new());
            names.push(name.to_owned());
            rest = rest.get(close + 1..)?;
        }
        pieces.last_mut()?.push_str(rest);
        // Two placeholders side by side can't be told apart.
        let separated = pieces.iter().skip(1).rev().skip(1).all(|p| !p.is_empty());
        (!names.is_empty() && separated).then(|| Template {
            pieces,
            names,
            translation: translation.to_owned(),
        })
    }

    /// The translation of `s`, if `s` is this message with its placeholders filled in.
    fn apply(&self, s: &str) -> Option<String> {
        let (first, rest) = self.pieces.split_first()?;
        let mut s = s.strip_prefix(first.as_str())?;
        let mut values = Vec::with_capacity(self.names.len());
        for (i, piece) in rest.iter().enumerate() {
            let last = i + 1 == rest.len();
            let at = if last {
                s.len()
                    .checked_sub(piece.len())
                    .filter(|&at| s.get(at..) == Some(piece.as_str()))?
            } else {
                s.find(piece.as_str())?
            };
            let value = s.get(..at)?;
            if value.is_empty() {
                return None;
            }
            values.push(value);
            s = s.get(at + piece.len()..)?;
        }
        let mut out = self.translation.clone();
        for (name, value) in self.names.iter().zip(values) {
            // A value that is itself a message, such as an error, is translated too.
            out = out.replace(&format!("{{{name}}}"), &translate(value));
        }
        Some(out)
    }
}

/// One language's entries: whole messages, and messages with placeholders.
#[derive(Debug, Default)]
struct Catalogue {
    exact: HashMap<&'static str, &'static str>,
    templates: Vec<Template>,
}

fn parse(source: &'static str) -> Catalogue {
    let mut out = Catalogue::default();
    let mut english: Option<&'static str> = None;
    for line in source.lines() {
        if line.starts_with('#') {
            continue;
        }
        if let Some(translation) = line.strip_prefix("= ") {
            let Some(english) = english.take() else { continue };
            match Template::new(&unescape(english), &unescape(translation)) {
                Some(template) => out.templates.push(template),
                None if !english.contains('\\') && !translation.contains('\\') => {
                    out.exact.insert(english, translation);
                }
                None => {
                    // Line breaks: kept as owned strings for the life of the program.
                    let e: &'static str = Box::leak(unescape(english).into_boxed_str());
                    let t: &'static str = Box::leak(unescape(translation).into_boxed_str());
                    out.exact.insert(e, t);
                }
            }
        } else if !line.trim().is_empty() {
            english = Some(line);
        }
    }
    // Longer messages first, so "Saved {n} pages" is tried before "{n} pages".
    out.templates
        .sort_by_key(|t| std::cmp::Reverse(t.pieces.iter().map(String::len).sum::<usize>()));
    out
}

fn unescape(s: &str) -> String {
    s.replace("\\n", "\n")
}

fn catalogue(language: Language) -> &'static Catalogue {
    static HINDI: OnceLock<Catalogue> = OnceLock::new();
    static NONE: OnceLock<Catalogue> = OnceLock::new();
    match language {
        Language::English => NONE.get_or_init(Catalogue::default),
        Language::Hindi => HINDI.get_or_init(|| parse(Language::Hindi.catalogue())),
    }
}

/// Formatted messages translated lately, so a message drawn every frame is matched once.
fn recent() -> &'static Mutex<HashMap<String, Option<String>>> {
    static RECENT: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    RECENT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `s` in the interface's language, or `s` itself when there's no translation.
pub fn translate(s: &str) -> Cow<'_, str> {
    let language = language();
    if language == Language::English || s.is_empty() {
        return Cow::Borrowed(s);
    }
    let catalogue = catalogue(language);
    if let Some(t) = catalogue.exact.get(s) {
        return Cow::Borrowed(t);
    }
    if catalogue.templates.is_empty() || s.len() > 2_000 {
        return Cow::Borrowed(s);
    }
    if let Ok(recent) = recent().lock()
        && let Some(found) = recent.get(s)
    {
        return found.clone().map_or(Cow::Borrowed(s), Cow::Owned);
    }
    let found = catalogue.templates.iter().find_map(|t| t.apply(s));
    if let Ok(mut recent) = recent().lock() {
        if recent.len() > 4_000 {
            recent.clear();
        }
        recent.insert(s.to_owned(), found.clone());
    }
    found.map_or(Cow::Borrowed(s), Cow::Owned)
}

/// A string written in the code, in the interface's language, for places that keep a
/// borrowed `&str` (placeholders, dialog titles).
pub fn tr_str(s: &'static str) -> &'static str {
    if language() == Language::English {
        return s;
    }
    catalogue(language()).exact.get(s).copied().unwrap_or(s)
}

/// Iced's `text`, in the interface's language.
pub fn text<'a, Theme, Renderer>(
    content: impl text::IntoFragment<'a>,
) -> iced::widget::Text<'a, Theme, Renderer>
where
    Theme: text::Catalog + 'a,
    Renderer: iced::advanced::text::Renderer,
{
    let fragment = content.into_fragment();
    if language() == Language::English {
        return iced::widget::Text::new(fragment);
    }
    let translated = match translate(&fragment) {
        // Not translated: keep the text as it came, without copying it.
        Cow::Borrowed(t) if std::ptr::eq(t, &*fragment) => None,
        t => Some(t.into_owned()),
    };
    iced::widget::Text::new(translated.map_or(fragment, Cow::Owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_fill_in_their_values() {
        let t = Template::new("Page {n} of {count}", "पृष्ठ {n} / {count}").unwrap();
        assert_eq!(t.apply("Page 3 of 6").as_deref(), Some("पृष्ठ 3 / 6"));
        assert_eq!(t.apply("Page  of 6"), None);
        assert_eq!(t.apply("Pages 3 of 6"), None);
        let end = Template::new("{n} files", "{n} फ़ाइलें").unwrap();
        assert_eq!(end.apply("12 files").as_deref(), Some("12 फ़ाइलें"));
        assert!(Template::new("No placeholders", "x").is_none());
        assert!(Template::new("{a}{b}", "x").is_none());
    }

    #[test]
    fn the_hindi_catalogue_reads_cleanly() {
        let source = Language::Hindi.catalogue();
        let c = parse(source);
        assert!(c.exact.len() + c.templates.len() > 100);
        // Every entry has its translation, and every placeholder is kept.
        let mut english: Option<&str> = None;
        for line in source.lines().filter(|l| !l.starts_with('#')) {
            if let Some(t) = line.strip_prefix("= ") {
                let e = english
                    .take()
                    .unwrap_or_else(|| panic!("no English before {t:?}"));
                for name in e
                    .split('{')
                    .skip(1)
                    .filter_map(|p| p.split_once('}'))
                    .map(|p| p.0)
                {
                    assert!(
                        t.contains(&format!("{{{name}}}")),
                        "{e:?} loses {{{name}}} in {t:?}"
                    );
                }
                assert!(!t.trim().is_empty(), "{e:?} has no translation");
            } else if !line.trim().is_empty() {
                assert!(english.replace(line).is_none(), "no translation before {line:?}");
            }
        }
        assert!(english.is_none());
    }
}
