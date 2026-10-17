//! User preferences stored as `key=value` lines in the OS config directory. Nothing leaves
//! the machine. Unknown or malformed lines are ignored so a damaged file can't stop the app.

use std::path::PathBuf;

use iced::Point;

use crate::toolbar::Dock;

/// Settings files bigger than this are ignored rather than parsed.
const MAX_BYTES: u64 = 64 * 1024;
const MAX_RECENT: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub toolbar_dock: Dock,
    pub toolbar_pos: Point,
    /// Most recently opened first.
    pub recent: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            toolbar_dock: Dock::Top,
            toolbar_pos: Point::new(40.0, 60.0),
            recent: Vec::new(),
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
            match key.trim() {
                "toolbar.dock" => out.toolbar_dock = Dock::parse(value.trim()).unwrap_or(out.toolbar_dock),
                "toolbar.x" => out.toolbar_pos.x = finite(value).unwrap_or(out.toolbar_pos.x),
                "toolbar.y" => out.toolbar_pos.y = finite(value).unwrap_or(out.toolbar_pos.y),
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
        let mut out = format!(
            "toolbar.dock={}\ntoolbar.x={}\ntoolbar.y={}\n",
            self.toolbar_dock.as_str(),
            self.toolbar_pos.x,
            self.toolbar_pos.y
        );
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

fn finite(s: &str) -> Option<f32> {
    s.trim()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite() && v.abs() < 100_000.0)
}

fn file_path() -> Option<PathBuf> {
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
    Some(base.join("Panna").join("settings.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let s = Settings {
            toolbar_dock: Dock::Floating,
            toolbar_pos: Point::new(120.5, 33.0),
            recent: vec![PathBuf::from("/a/b c=d.pdf"), PathBuf::from("C:\\x.pdf")],
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn ignores_garbage() {
        let s =
            Settings::parse("toolbar.dock=sideways\ntoolbar.x=NaN\ntoolbar.y=1e30\nno equals\n=\nrecent=\n");
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
            recent: vec![PathBuf::from("evil\ntoolbar.dock=left")],
            ..Settings::default()
        };
        assert_eq!(Settings::parse(&s.serialize()).toolbar_dock, Dock::Top);
    }
}
