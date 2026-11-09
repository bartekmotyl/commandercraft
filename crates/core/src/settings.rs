//! Preferences and session state, stored as JSON in the per-user settings folder:
//! `%APPDATA%\CommanderCraft` (Windows), `~/Library/Application Support/CommanderCraft` (macOS),
//! `$XDG_CONFIG_HOME/commandercraft` or `~/.config/commandercraft` (Linux).
//! `COMMANDERCRAFT_CONFIG_DIR` overrides the folder.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::sort::Sort;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub bookmarks: Vec<Bookmark>,
    /// Command for F4 (Edit); empty = the system default application.
    pub editor: String,
    /// Commands for opening files by extension (Enter / double-click); others use the system default.
    pub associations: Vec<Association>,
    pub show_hidden: bool,
    /// Delete moves to the trash unless this is set (Shift+Delete is always permanent).
    pub delete_permanently: bool,
    pub left: PanelSession,
    pub right: PanelSession,
    pub right_active: bool,
    pub appearance: Appearance,
    /// Terminal command (`{dir}` = folder); empty = the system default terminal.
    pub terminal: String,
    /// The left panel's share of the width (0.15 – 0.85).
    pub split: f32,
    pub viewer_monospace: bool,
    pub viewer_wrap: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeChoice,
    /// Interface font family; empty = built-in.
    pub font: String,
    /// Monospace font family (viewer, monospace lists); empty = built-in.
    pub mono_font: String,
    /// Text size in points.
    pub font_size: f32,
    /// Show file lists in the monospace font.
    pub list_monospace: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance { theme: ThemeChoice::System, font: String::new(), mono_font: String::new(), font_size: 13.0, list_monospace: false }
    }
}

impl Settings {
    /// [`Settings::split`] with the default and limits applied.
    pub fn split_ratio(&self) -> f32 {
        if self.split.is_finite() && self.split > 0.0 { self.split.clamp(0.15, 0.85) } else { 0.5 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Association {
    /// Comma- or space-separated extensions without dots: `txt, md, log`.
    pub extensions: String,
    pub command: String,
}

impl Association {
    pub fn matches(&self, ext_lower: &str) -> bool {
        !ext_lower.is_empty()
            && self.extensions.split([',', ' ', ';']).any(|e| e.trim().trim_start_matches('.').eq_ignore_ascii_case(ext_lower))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelSession {
    pub tabs: Vec<TabSession>,
    pub active: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TabSession {
    pub path: PathBuf,
    pub sort: Sort,
}

impl Settings {
    /// The configured command for a file name, if any association matches its extension.
    pub fn command_for(&self, file_name: &str) -> Option<&str> {
        let ext = crate::entry::split_ext(file_name).1.to_lowercase();
        self.associations.iter().find(|a| a.matches(&ext) && !a.command.trim().is_empty()).map(|a| a.command.as_str())
    }

    pub fn load() -> Settings {
        file_path().and_then(|p| fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let path = file_path().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no settings folder"))?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        // Write then rename, so a crash never leaves half a file.
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(tmp, path)
    }
}

pub fn config_dir() -> Option<PathBuf> {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = var("COMMANDERCRAFT_CONFIG_DIR") {
        return Some(dir);
    }
    if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library/Application Support/CommanderCraft"))
    } else if cfg!(windows) {
        var("APPDATA").map(|a| a.join("CommanderCraft"))
    } else {
        var("XDG_CONFIG_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| var("HOME").map(|h| h.join(".config")))
            .map(|c| c.join("commandercraft"))
    }
}

fn file_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn associations() {
        let s = Settings {
            associations: vec![Association { extensions: "txt, .md;LOG".into(), command: "code".into() }],
            ..Default::default()
        };
        assert_eq!(s.command_for("a.MD"), Some("code"));
        assert_eq!(s.command_for("x.log"), Some("code"));
        assert_eq!(s.command_for("x.rs"), None);
        assert_eq!(s.command_for("txt"), None);
    }

    #[test]
    fn json_roundtrip_tolerates_missing_fields() {
        let s: Settings = serde_json::from_str(r#"{"editor":"vim"}"#).unwrap();
        assert_eq!(s.editor, "vim");
        assert!(s.bookmarks.is_empty());
    }
}
