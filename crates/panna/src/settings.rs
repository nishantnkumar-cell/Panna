//! User preferences stored as `key=value` lines in the OS config directory. Nothing leaves
//! the machine. Unknown or malformed lines are ignored so a damaged file can't stop the app.

use std::path::PathBuf;

use panna_core::{Color, RedactLook};

use crate::colors::PageColors;
use crate::layout::Spread;
use crate::measure::Unit;
use crate::preferences::{Appearance, OpenZoom};
use crate::sidebar::{Side, Tab};
use crate::toolbar;

/// Settings files bigger than this are ignored rather than parsed.
const MAX_BYTES: u64 = 64 * 1024;
const MAX_RECENT: usize = 10;
/// Words kept in the personal spelling dictionary, at most.
pub const MAX_WORDS: usize = 5_000;
/// Longest name kept for comments.
const MAX_AUTHOR: usize = 200;
/// Longest wait between pages when presenting on a timer, in seconds.
pub const MAX_SECONDS: u32 = 3600;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub bars: toolbar::Layout,
    pub sidebar_open: bool,
    pub sidebar_side: Side,
    pub sidebar_tab: Tab,
    pub spread: Spread,
    pub colors: PageColors,
    /// Pages scroll past continuously; off shows one page or spread at a time.
    pub scrolling: bool,
    /// The name put on comments; empty for the account's name.
    pub author: String,
    pub appearance: Appearance,
    pub open_zoom: OpenZoom,
    pub unit: Unit,
    /// Single letters pick tools, as Acrobat's "Use single-key accelerators to access tools".
    pub single_keys: bool,
    /// Full screen: go back to the first page after the last.
    pub present_loop: bool,
    /// Full screen: turn the page every this many seconds; 0 for never.
    pub present_seconds: u32,
    /// How redaction boxes look: fill colour and overlay text.
    pub redact: RedactLook,
    /// Most recently opened first.
    pub recent: Vec<PathBuf>,
    /// Words added to the spelling dictionary, in lower case.
    pub words: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bars: toolbar::Layout::default(),
            sidebar_open: false,
            sidebar_side: Side::Left,
            sidebar_tab: Tab::Thumbnails,
            spread: Spread::Single,
            colors: PageColors::Normal,
            scrolling: true,
            author: String::new(),
            appearance: Appearance::System,
            open_zoom: OpenZoom::ActualSize,
            unit: Unit::default(),
            single_keys: true,
            present_loop: false,
            present_seconds: 0,
            redact: RedactLook::default(),
            recent: Vec::new(),
            words: Vec::new(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = file_path() else {
            return Self::default();
        };
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() <= MAX_BYTES => {}
            _ => return Self::default(),
        }
        std::fs::read_to_string(path).map_or_else(|_| Self::default(), |s| Self::parse(&s))
    }

    /// Best effort: a failed save only loses preferences, so errors are ignored.
    pub fn save(&self) {
        let Some(path) = file_path() else { return };
        let Some(dir) = path.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        // Write then rename so a crash mid-write never leaves a half-written file.
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, self.serialize()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    pub fn add_recent(&mut self, path: PathBuf) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        // Windows canonical paths carry a `\\?\` prefix that users never type.
        let path = match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
            Some(plain) if !plain.starts_with("UNC\\") => PathBuf::from(plain),
            _ => path,
        };
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
    }

    fn parse(s: &str) -> Self {
        let mut out = Self::default();
        for line in s.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            if out.bars.parse_line(key, value, finite) {
                continue;
            }
            match key {
                "sidebar.open" => out.sidebar_open = value.trim() == "true",
                "sidebar.side" => out.sidebar_side = Side::parse(value.trim()).unwrap_or(out.sidebar_side),
                "sidebar.tab" => out.sidebar_tab = Tab::parse(value.trim()).unwrap_or(out.sidebar_tab),
                "view.spread" => out.spread = Spread::parse(value.trim()).unwrap_or(out.spread),
                "view.colors" => out.colors = PageColors::parse(value.trim()).unwrap_or(out.colors),
                "view.scrolling" => out.scrolling = value.trim() != "false",
                "view.appearance" => {
                    out.appearance = Appearance::parse(value.trim()).unwrap_or(out.appearance);
                }
                "view.open_zoom" => out.open_zoom = OpenZoom::parse(value.trim()).unwrap_or(out.open_zoom),
                "measure.unit" => out.unit = Unit::parse(value.trim()).unwrap_or(out.unit),
                "keys.single" => out.single_keys = value.trim() != "false",
                "present.loop" => out.present_loop = value.trim() == "true",
                "present.seconds" => {
                    out.present_seconds = value.trim().parse::<u32>().map_or(0, |s| s.min(MAX_SECONDS));
                }
                "redact.fill" => out.redact.fill = parse_hex(value.trim()).unwrap_or(out.redact.fill),
                "redact.overlay" => {
                    out.redact.overlay = value.trim().chars().take(crate::redact::MAX_OVERLAY).collect();
                }
                "redact.repeat" => out.redact.repeat = value.trim() == "true",
                "comments.author" => out.author = value.trim().chars().take(MAX_AUTHOR).collect(),
                "spelling.word" if out.words.len() < MAX_WORDS => {
                    let word = value.trim();
                    if !word.is_empty() && word.chars().count() <= 64 && !word.contains(char::is_whitespace) {
                        out.words.push(word.to_lowercase());
                    }
                }
                // Paths keep their exact spelling, so no trimming.
                "recent" if !value.is_empty() && out.recent.len() < MAX_RECENT => {
                    out.recent.push(PathBuf::from(value));
                }
                _ => {}
            }
        }
        out
    }

    fn serialize(&self) -> String {
        let mut out = String::new();
        self.bars.serialize(&mut out);
        out.push_str(&format!(
            "sidebar.open={}\nsidebar.side={}\nsidebar.tab={}\nview.spread={}\nview.colors={}\nview.scrolling={}\n",
            self.sidebar_open,
            self.sidebar_side.as_str(),
            self.sidebar_tab.as_str(),
            self.spread.as_str(),
            self.colors.as_str(),
            self.scrolling,
        ));
        out.push_str(&format!(
            "view.appearance={}\nview.open_zoom={}\nmeasure.unit={}\nkeys.single={}\npresent.loop={}\npresent.seconds={}\n",
            self.appearance.as_str(),
            self.open_zoom.as_str(),
            self.unit.as_str(),
            self.single_keys,
            self.present_loop,
            self.present_seconds,
        ));
        out.push_str(&format!(
            "redact.fill={}\nredact.repeat={}\n",
            hex(self.redact.fill),
            self.redact.repeat
        ));
        if !self.redact.overlay.contains(['\n', '\r']) {
            out.push_str(&format!("redact.overlay={}\n", self.redact.overlay));
        }
        // A name with a line break can't be stored in this format.
        if !self.author.contains(['\n', '\r']) {
            out.push_str(&format!("comments.author={}\n", self.author));
        }
        for word in &self.words {
            out.push_str(&format!("spelling.word={word}\n"));
        }
        // Paths that aren't UTF-8 or contain line breaks can't be stored in this format.
        for path in &self.recent {
            if let Some(p) = path.to_str().filter(|p| !p.contains(['\n', '\r'])) {
                out.push_str("recent=");
                out.push_str(p);
                out.push('\n');
            }
        }
        out
    }
}

/// A colour as #rrggbb.
fn hex(Color(rgb): Color) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(rgb[0]), byte(rgb[1]), byte(rgb[2]))
}

fn parse_hex(s: &str) -> Option<Color> {
    let digits = s.strip_prefix('#')?;
    if digits.len() != 6 || !digits.is_ascii() {
        return None;
    }
    let part = |i: usize| {
        u8::from_str_radix(digits.get(i..i + 2)?, 16)
            .ok()
            .map(|v| f32::from(v) / 255.0)
    };
    Some(Color([part(0)?, part(2)?, part(4)?]))
}

fn finite(s: &str) -> Option<f32> {
    s.trim()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite() && v.abs() < 100_000.0)
}

fn file_path() -> Option<PathBuf> {
    Some(config_dir()?.join("settings.txt"))
}

/// Panna's folder in the OS config directory.
pub fn config_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("Panna"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let s = Settings {
            bars: {
                let mut bars = toolbar::Layout::default();
                bars.bottom = toolbar::Align::Left;
                bars.locked = true;
                bars
            },
            sidebar_open: true,
            sidebar_side: Side::Right,
            sidebar_tab: Tab::Bookmarks,
            spread: Spread::TwoCover,
            colors: PageColors::Sepia,
            scrolling: false,
            author: "Asha Rao".into(),
            appearance: Appearance::Dark,
            open_zoom: OpenZoom::FitWidth,
            unit: Unit::Inches,
            single_keys: false,
            present_loop: true,
            present_seconds: 7,
            redact: RedactLook {
                fill: Color([1.0, 1.0, 1.0]),
                overlay: "Withheld".into(),
                repeat: true,
            },
            recent: vec![PathBuf::from("/a/b c=d.pdf"), PathBuf::from("C:\\x.pdf")],
            words: vec!["panna".into(), "acrobat's".into()],
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn ignores_garbage() {
        let s = Settings::parse(
            "bar.file.dock=sideways\nbar.file.x=NaN\nbar.page.y=1e30\nno equals\n=\nrecent=\n",
        );
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn recent_is_deduplicated_and_capped() {
        let mut s = Settings::default();
        for i in 0..15 {
            s.add_recent(PathBuf::from(format!("/nonexistent/{i}.pdf")));
        }
        s.add_recent(PathBuf::from("/nonexistent/10.pdf"));
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0], PathBuf::from("/nonexistent/10.pdf"));
        assert_eq!(s.recent.iter().filter(|p| p.ends_with("10.pdf")).count(), 1);
    }

    #[test]
    fn line_breaks_in_paths_are_not_saved() {
        let s = Settings {
            recent: vec![PathBuf::from("evil\nbars.locked=true")],
            ..Settings::default()
        };
        assert!(!Settings::parse(&s.serialize()).bars.locked);
    }
}
