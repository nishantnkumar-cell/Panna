//! User preferences stored as `key=value` lines in the OS config directory. Nothing leaves
//! the machine. Unknown or malformed lines are ignored so a damaged file can't stop the app.

use std::path::PathBuf;

use iced::Point;

use crate::toolbar::Dock;

/// Settings files bigger than this are ignored rather than parsed.
const MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub toolbar_dock: Dock,
    pub toolbar_pos: Point,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            toolbar_dock: Dock::Top,
            toolbar_pos: Point::new(40.0, 60.0),
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

    fn parse(s: &str) -> Self {
        let mut out = Self::default();
        for line in s.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "toolbar.dock" => out.toolbar_dock = Dock::parse(value).unwrap_or(out.toolbar_dock),
                "toolbar.x" => out.toolbar_pos.x = finite(value).unwrap_or(out.toolbar_pos.x),
                "toolbar.y" => out.toolbar_pos.y = finite(value).unwrap_or(out.toolbar_pos.y),
                _ => {}
            }
        }
        out
    }

    fn serialize(&self) -> String {
        format!(
            "toolbar.dock={}\ntoolbar.x={}\ntoolbar.y={}\n",
            self.toolbar_dock.as_str(),
            self.toolbar_pos.x,
            self.toolbar_pos.y
        )
    }
}

fn finite(s: &str) -> Option<f32> {
    s.parse::<f32>()
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
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn ignores_garbage() {
        let s = Settings::parse("toolbar.dock=sideways\ntoolbar.x=NaN\ntoolbar.y=1e30\nno equals\n=\n");
        assert_eq!(s, Settings::default());
    }
}
