//! CommanderCraft's egui interface: two file panels with tabs, a menu bar, a toolbar, a command
//! line, a function-key bar, dialogs, the internal viewer and background file operations.
//!
//! Keyboard (`Ctrl` is `⌘` on macOS; letter shortcuts accept the Control key there too):
//!
//! | Key | Action |
//! |---|---|
//! | ↑ ↓ PgUp PgDn Home End | move the cursor (with Shift: mark while moving) |
//! | Enter / Shift+Enter | open (folders, archives, files) / open with the system default |
//! | Backspace | parent folder |
//! | Tab | other panel |
//! | Space, Insert | mark / unmark and move down |
//! | Ctrl+A, Ctrl+Shift+A, Ctrl+I | mark all, unmark all, invert marks |
//! | Ctrl+← / Ctrl+→ | show the folder under the cursor (or the current one) in the left / right panel |
//! | Ctrl+↑ / Ctrl+Shift+↑ | open the folder under the cursor in a new tab / in a new tab on the other side |
//! | Ctrl+T, Ctrl+W, Ctrl+Tab, Ctrl+Shift+Tab | new tab, close tab, next / previous tab |
//! | Ctrl+D | bookmarks list (↑ ↓ Enter or 1–9; the last entry opens the manager) |
//! | F3, Ctrl+Q (Control key), F4 | internal viewer, quick view, edit |
//! | F5, F6, Shift+F6, F7 | copy, move, rename, new folder |
//! | Alt+F9 / Ctrl+Shift+E | extract archives |
//! | F8 / Delete / Ctrl+Backspace | delete to trash (with Shift: permanently) |
//! | Ctrl+F3…F6 | sort by name, extension, date, size |
//! | Ctrl+Shift+C / Ctrl+Alt+C | copy the folder path / the marked files' paths |
//! | Ctrl+Shift+V / Ctrl+Alt+V | go to the path on the clipboard / in a new tab |
//! | Ctrl+Shift+T | terminal here |
//! | typing | goes to the command line (Enter runs, Shift+Enter runs in a terminal) |
//! | Ctrl+R / F2 | refresh;  Ctrl+U swap panels;  Ctrl+Shift+H hidden files;  Ctrl+, settings |

mod app_ui;
mod commands;
mod dialogs;
mod fonts;
mod jobs;
pub mod panel;
mod theme;
mod viewer;

pub use commandercraft_core::settings::Settings;

use std::path::{Path, PathBuf};
use std::time::Instant;

use commandercraft_core::ops::NoProgress;
use commandercraft_core::settings::{PanelSession, TabSession};
use commandercraft_core::sort::Sort;
use commandercraft_core::volumes::{self, Volume};
use commandercraft_core::watch::DirWatcher;
use commandercraft_core::{Entry, Location, archive};
use dialogs::Dialog;
use egui::{Key, Modifiers};
use fonts::FontLoader;
use jobs::Job;
use panel::{Panel, Tab};
use viewer::Viewer;

enum JobKind {
    Transfer { verb: &'static str, source_side: usize },
    Delete,
    Extract,
}

/// Quick view (Ctrl+Q): one panel previews the file under the other panel's cursor.
struct QuickView {
    /// The side showing the preview.
    side: usize,
    /// What is previewed: (folder, name), to reload only when the cursor moves.
    key: Option<(String, String)>,
    viewer: Option<Viewer>,
    note: String,
}

pub struct CommanderApp {
    settings: Settings,
    panels: [Panel; 2],
    active: usize,
    keys: Vec<(Key, Modifiers)>,
    dialog: Option<Dialog>,
    dialog_fresh: bool,
    job: Option<(Job, JobKind)>,
    status: String,
    volumes: Vec<Volume>,
    temp_counter: u32,
    title: String,
    /// Theme and text sizes need applying (first frame, after settings change).
    style_dirty: bool,
    fonts: Option<FontLoader>,
    /// Scan system fonts (off in headless tests).
    load_system_fonts: bool,
    /// Write settings to disk (off in tests, which must never touch the user's settings).
    persist: bool,
    watcher: Option<DirWatcher>,
    last_auto_refresh: Instant,
    viewer: Option<Viewer>,
    quick_view: Option<QuickView>,
    cmdline: String,
    cmdline_focus: bool,
    /// The cursor was last moved by keyboard: hide the mouse hover until the mouse moves.
    keyboard_nav: bool,
    /// Where each panel was drawn (to anchor popups to the active one).
    panel_rects: [egui::Rect; 2],
}

impl CommanderApp {
    pub fn new(settings: Settings) -> Self {
        let hidden = settings.show_hidden;
        let home = volumes::home().unwrap_or_else(|| PathBuf::from("/"));
        let make = |s: &PanelSession| {
            let mut tabs: Vec<Tab> = s.tabs.iter().map(|t| Tab::open_or_nearest(Location::Local(t.path.clone()), t.sort, hidden)).collect();
            if tabs.is_empty() {
                tabs.push(Tab::open_or_nearest(Location::Local(home.clone()), Sort::default(), hidden));
            }
            Panel::new(tabs, s.active)
        };
        let panels = [make(&settings.left), make(&settings.right)];
        CommanderApp {
            active: usize::from(settings.right_active),
            settings,
            panels,
            keys: Vec::new(),
            dialog: None,
            dialog_fresh: false,
            job: None,
            status: String::new(),
            volumes: volumes::volumes(),
            temp_counter: 0,
            title: String::new(),
            style_dirty: true,
            fonts: None,
            load_system_fonts: true,
            persist: true,
            watcher: None,
            last_auto_refresh: Instant::now(),
            viewer: None,
            quick_view: None,
            cmdline: String::new(),
            cmdline_focus: false,
            keyboard_nav: false,
            panel_rects: [egui::Rect::NOTHING; 2],
        }
    }

    /// Without the system font scan, file watching and saving settings (headless tests:
    /// deterministic, fast, and the user's settings file is never written).
    pub fn without_system_services(mut self) -> Self {
        self.load_system_fonts = false;
        self.without_saving()
    }

    /// Never write the settings file (tests).
    pub fn without_saving(mut self) -> Self {
        self.persist = false;
        self
    }

    // -----------------------------------------------------------------------------------------
    // Helpers

    fn tab(&self) -> &Tab {
        self.panels[self.active].tab()
    }
    fn tab_mut(&mut self) -> &mut Tab {
        self.panels[self.active].tab_mut()
    }
    fn hidden(&self) -> bool {
        self.settings.show_hidden
    }

    fn open_dialog(&mut self, d: Dialog) {
        self.dialog = Some(d);
        self.dialog_fresh = true;
    }

    fn message(&mut self, title: &str, text: impl Into<String>) {
        self.open_dialog(Dialog::Message { title: title.into(), text: text.into() });
    }

    fn navigate(&mut self, side: usize, location: Location, focus: Option<&str>) {
        let hidden = self.hidden();
        if let Err(e) = self.panels[side].tab_mut().navigate(location, focus, hidden) {
            self.message("Cannot open", e);
        }
    }

    fn go_up(&mut self, side: usize) {
        if let Some((parent, name)) = self.panels[side].tab().location.parent() {
            self.navigate(side, parent, Some(&name));
        }
    }

    fn refresh_all(&mut self) {
        let hidden = self.hidden();
        for p in &mut self.panels {
            p.tab_mut().reload(hidden);
        }
    }

    fn reload_all_tabs(&mut self) {
        let hidden = self.hidden();
        for p in &mut self.panels {
            for t in &mut p.tabs {
                t.reload(hidden);
            }
        }
    }

    fn add_tab(&mut self, side: usize, location: Location, activate: bool) {
        let sort = self.panels[side].tab().sort;
        match Tab::open(location, sort, self.hidden()) {
            Ok(tab) => {
                let p = &mut self.panels[side];
                p.tabs.insert(p.active + 1, tab);
                if activate {
                    p.active += 1;
                }
            }
            Err(e) => self.message("Cannot open", e),
        }
    }

    fn close_tab(&mut self, side: usize, i: usize) {
        let p = &mut self.panels[side];
        if p.tabs.len() > 1 && i < p.tabs.len() {
            p.tabs.remove(i);
            if p.active > i || p.active >= p.tabs.len() {
                p.active = p.active.saturating_sub(1);
            }
        }
    }

    fn select_tab(&mut self, side: usize, i: usize) {
        let hidden = self.hidden();
        let p = &mut self.panels[side];
        if i < p.tabs.len() {
            p.active = i;
            p.tab_mut().reload(hidden);
        }
    }

    /// Where Ctrl+←/→ and Ctrl+↑ point: the folder (or archive) under the cursor, else the current folder.
    fn cursor_location(&self) -> Location {
        let tab = self.tab();
        match tab.current() {
            Some(e) if e.is_dir && !e.is_parent => tab.location.child(&e.name),
            Some(e) if !e.is_dir => match &tab.location {
                Location::Local(dir) if archive::detect(Path::new(&e.name)).is_some() => {
                    Location::Archive { archive: dir.join(&e.name), inner: String::new() }
                }
                _ => tab.location.clone(),
            },
            _ => tab.location.clone(),
        }
    }

    /// A local folder for bookmarks / the session: the folder itself, or the one holding the archive.
    fn local_folder(location: &Location) -> PathBuf {
        match location {
            Location::Local(p) => p.clone(),
            Location::Archive { archive, .. } => archive.parent().map(Path::to_path_buf).unwrap_or_default(),
        }
    }

    fn save_settings(&mut self) {
        self.store_session();
        if !self.persist {
            return;
        }
        if let Err(e) = self.settings.save() {
            self.status = format!("Settings could not be saved: {e}");
        }
    }

    fn store_session(&mut self) {
        let session = |p: &Panel| PanelSession {
            tabs: p.tabs.iter().map(|t| TabSession { path: Self::local_folder(&t.location), sort: t.sort }).collect(),
            active: p.active,
        };
        self.settings.left = session(&self.panels[0]);
        self.settings.right = session(&self.panels[1]);
        self.settings.right_active = self.active == 1;
    }

    /// Extract one archive member to a fresh temporary folder (to open it).
    fn extract_temp(&mut self, archive_path: &Path, item: &str) -> Result<PathBuf, String> {
        self.temp_counter += 1;
        let dir = temp_root().join(format!("{}-{}", std::process::id(), self.temp_counter));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let report = archive::extract(archive_path, &[item.to_string()], &dir, true, &mut NoProgress);
        if !report.errors.is_empty() {
            return Err(report.errors.join("\n"));
        }
        Ok(dir.join(item.rsplit('/').next().unwrap_or(item)))
    }

    /// A file under the cursor as a local path (archive members are extracted to a temp folder).
    fn cursor_file(&mut self) -> Option<(Entry, PathBuf)> {
        let tab = self.tab();
        let e = tab.current().filter(|e| !e.is_dir)?.clone();
        let result = match tab.location.clone() {
            Location::Local(dir) => Ok(dir.join(&e.name)),
            Location::Archive { archive, inner } => self.extract_temp(&archive, &archive::join(&inner, &e.name)),
        };
        match result {
            Ok(p) => Some((e, p)),
            Err(err) => {
                self.message("Cannot extract", err);
                None
            }
        }
    }

    fn launch(&mut self, result: std::io::Result<()>, what: &Path) {
        if let Err(e) = result {
            self.message("Cannot open", format!("{}: {e}", what.display()));
        }
    }

    // -----------------------------------------------------------------------------------------
    // For tests and embedding

    /// Queue a panel key as if it had been captured from the keyboard (headless tests do not run
    /// `raw_input_hook`).
    pub fn feed_key(&mut self, key: Key, modifiers: Modifiers) {
        self.keys.push((key, modifiers));
    }
    pub fn panels(&self) -> &[Panel; 2] {
        &self.panels
    }
    pub fn active_side(&self) -> usize {
        self.active
    }
    /// A file operation is running.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn has_dialog(&self) -> bool {
        self.dialog.is_some()
    }
    pub fn viewer_open(&self) -> bool {
        self.viewer.is_some()
    }
    pub fn quick_view_open(&self) -> bool {
        self.quick_view.is_some()
    }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    pub fn settings_mut(&mut self) -> &mut Settings {
        self.style_dirty = true;
        &mut self.settings
    }
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Remove this run's temporary extraction folders.
    fn cleanup_temp(&self) {
        let prefix = format!("{}-", std::process::id());
        if let Ok(rd) = std::fs::read_dir(temp_root()) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
    }
}

fn temp_root() -> PathBuf {
    std::env::temp_dir().join("commandercraft")
}

/// `"report.pdf"`, `folder "src"`, or `5 items (2 folders)`.
fn describe(targets: &[Entry]) -> String {
    match targets {
        [e] if e.is_dir => format!("folder \"{}\"", e.name),
        [e] => format!("\"{}\"", e.name),
        many => {
            let dirs = many.iter().filter(|e| e.is_dir).count();
            if dirs > 0 { format!("{} items ({dirs} folders)", many.len()) } else { format!("{} files", many.len()) }
        }
    }
}

impl eframe::App for CommanderApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.capture_keys(ctx, raw_input);
    }

    fn on_exit(&mut self) {
        self.save_settings();
        self.cleanup_temp();
    }
}
