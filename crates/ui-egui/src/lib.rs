//! CommanderCraft's egui interface: two file panels with tabs, a menu bar, a function-key bar,
//! dialogs and background file operations.
//!
//! Keyboard (`Ctrl` is `⌘` on macOS):
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
//! | Ctrl+D | bookmarks |
//! | F3, F4 | view (system default app), edit (configured editor) |
//! | F5, F6, Shift+F6, F7 | copy, move, rename, new folder |
//! | F8 / Delete / Ctrl+Backspace | delete to trash (with Shift: permanently) |
//! | Ctrl+F3…F6 | sort by name, extension, date, size |
//! | Ctrl+R / F2 | refresh;  Ctrl+U swap panels;  Ctrl+Shift+H hidden files;  Ctrl+, settings |

mod dialogs;
mod jobs;
pub mod panel;

pub use commandercraft_core::settings::Settings;

use std::path::{Path, PathBuf};
use std::time::Duration;

use commandercraft_core::ops::{self, NoProgress, Report};
use commandercraft_core::settings::{PanelSession, TabSession};
use commandercraft_core::sort::{Sort, SortColumn};
use commandercraft_core::volumes::{self, Volume};
use commandercraft_core::{Entry, Location, archive, launch};
use dialogs::{Dialog, Outcome, Sources};
use egui::{Key, Modifiers};
use jobs::Job;
use panel::{Action, Panel, Tab};

#[derive(Clone, Debug, PartialEq)]
enum Cmd {
    Open,
    OpenSystem,
    View,
    Edit,
    Copy,
    Move,
    Rename,
    Mkdir,
    Delete { permanent: bool },
    Refresh,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    OpenInNewTab { other_side: bool },
    OpenInSide(usize),
    Swap,
    MarkAll,
    UnmarkAll,
    InvertMarks,
    Sort(SortColumn),
    ToggleHidden,
    Bookmarks,
    AddBookmark,
    GoTo(PathBuf),
    Settings,
    GoUp,
    GoRoot,
    SwitchPanel,
    Quit,
}

enum JobKind {
    Transfer { verb: &'static str, source_side: usize },
    Delete,
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
        }
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

    fn launch(&mut self, result: std::io::Result<()>, path: &Path) {
        if let Err(e) = result {
            self.message("Cannot open", format!("{}: {e}", path.display()));
        }
    }

    // -----------------------------------------------------------------------------------------
    // Commands

    fn run(&mut self, cmd: Cmd, ctx: &egui::Context) {
        let a = self.active;
        let hidden = self.hidden();
        match cmd {
            Cmd::Open => self.open_entry(false),
            Cmd::OpenSystem => self.open_entry(true),
            Cmd::View => {
                if let Some((_, path)) = self.cursor_file() {
                    let r = launch::open_default(&path);
                    self.launch(r, &path);
                }
            }
            Cmd::Edit => {
                if let Some((_, path)) = self.cursor_file() {
                    let r = launch::open_with(&self.settings.editor, &path);
                    self.launch(r, &path);
                }
            }
            Cmd::Copy => self.start_transfer(false),
            Cmd::Move => self.start_transfer(true),
            Cmd::Rename => match (self.tab().location.local_path(), self.tab().current()) {
                (Some(dir), Some(e)) if !e.is_parent => {
                    let d = Dialog::Rename { src: dir.join(&e.name), name: e.name.clone() };
                    self.open_dialog(d);
                }
                (None, _) => self.message("Rename", "Archives are read-only."),
                _ => {}
            },
            Cmd::Mkdir => match self.tab().location.local_path() {
                Some(dir) => {
                    let d = Dialog::Mkdir { dir: dir.to_path_buf(), name: String::new() };
                    self.open_dialog(d);
                }
                None => self.message("New folder", "Archives are read-only."),
            },
            Cmd::Delete { permanent } => {
                if self.tab().location.local_path().is_none() {
                    self.message("Delete", "Archives are read-only.");
                    return;
                }
                let targets = self.tab().targets();
                if targets.is_empty() {
                    return;
                }
                let d = Dialog::Delete {
                    paths: self.tab().target_paths(),
                    permanent: permanent || self.settings.delete_permanently,
                    summary: describe(&targets),
                };
                self.open_dialog(d);
            }
            Cmd::Refresh => {
                self.volumes = volumes::volumes();
                self.refresh_all();
            }
            Cmd::NewTab => {
                let loc = self.tab().location.clone();
                self.add_tab(a, loc, true);
            }
            Cmd::CloseTab => {
                let i = self.panels[a].active;
                self.close_tab(a, i);
            }
            Cmd::NextTab | Cmd::PrevTab => {
                let n = self.panels[a].tabs.len();
                let i = self.panels[a].active;
                let next = if cmd == Cmd::NextTab { (i + 1) % n } else { (i + n - 1) % n };
                self.select_tab(a, next);
            }
            Cmd::OpenInNewTab { other_side } => {
                let loc = self.cursor_location();
                if other_side {
                    self.add_tab(1 - a, loc, false);
                } else {
                    self.add_tab(a, loc, true);
                }
            }
            Cmd::OpenInSide(side) => {
                let loc = self.cursor_location();
                if side == a {
                    if loc != self.tab().location {
                        self.navigate(side, loc, None);
                    }
                } else {
                    self.navigate(side, loc, None);
                }
            }
            Cmd::Swap => self.panels.swap(0, 1),
            Cmd::MarkAll => self.tab_mut().mark_all(false),
            Cmd::UnmarkAll => self.tab_mut().marked.clear(),
            Cmd::InvertMarks => self.tab_mut().invert_marks(),
            Cmd::Sort(col) => {
                let sort = self.tab().sort.toggled(col);
                self.tab_mut().set_sort(sort);
            }
            Cmd::ToggleHidden => {
                self.settings.show_hidden = !hidden;
                let hidden = !hidden;
                for p in &mut self.panels {
                    for t in &mut p.tabs {
                        t.reload(hidden);
                    }
                }
            }
            Cmd::Bookmarks => self.open_dialog(Dialog::Bookmarks),
            Cmd::AddBookmark => {
                let dir = Self::local_folder(&self.tab().location);
                dialogs::add_bookmark(&mut self.settings.bookmarks, &dir);
                self.status = format!("Bookmarked {}", dir.display());
                self.save_settings();
            }
            Cmd::GoTo(path) => self.navigate(a, Location::Local(path), None),
            Cmd::Settings => self.open_dialog(Dialog::Settings(Box::new(self.settings.clone()))),
            Cmd::GoUp => self.go_up(a),
            Cmd::GoRoot => {
                let root = match &self.tab().location {
                    Location::Local(p) => Location::Local(p.ancestors().last().unwrap_or(p).to_path_buf()),
                    Location::Archive { archive, .. } => Location::Archive { archive: archive.clone(), inner: String::new() },
                };
                self.navigate(a, root, None);
            }
            Cmd::SwitchPanel => self.active = 1 - a,
            Cmd::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// Enter: go into folders and archives, open files.
    fn open_entry(&mut self, with_system: bool) {
        let a = self.active;
        let tab = self.tab();
        let Some(e) = tab.current().cloned() else {
            return;
        };
        let location = tab.location.clone();
        if e.is_parent {
            return self.go_up(a);
        }
        if e.is_dir {
            return self.navigate(a, location.child(&e.name), None);
        }
        let is_archive = archive::detect(Path::new(&e.name)).is_some();
        let path = match &location {
            Location::Local(dir) => dir.join(&e.name),
            Location::Archive { archive, inner } => match self.extract_temp(archive, &archive::join(inner, &e.name)) {
                Ok(p) => p,
                Err(err) => return self.message("Cannot extract", err),
            },
        };
        if is_archive && !with_system {
            return self.navigate(a, Location::Archive { archive: path, inner: String::new() }, None);
        }
        let result = match self.settings.command_for(&e.name) {
            Some(cmd) if !with_system => launch::open_with(cmd, &path),
            _ => launch::open_default(&path),
        };
        self.launch(result, &path);
    }

    fn start_transfer(&mut self, mv: bool) {
        let targets = self.tab().targets();
        if targets.is_empty() {
            return;
        }
        let Location::Local(dest) = self.panels[1 - self.active].tab().location.clone() else {
            return self.message("Copy", "Copying into archives is not supported yet.");
        };
        let sources = match self.tab().location.clone() {
            Location::Local(_) => Sources::Local(self.tab().target_paths()),
            Location::Archive { archive, inner } => {
                if mv {
                    return self.message("Move", "Archives are read-only: use Copy (F5) to extract.");
                }
                Sources::Archive { archive, items: targets.iter().map(|e| archive::join(&inner, &e.name)).collect() }
            }
        };
        let d = Dialog::Transfer { mv, sources, summary: describe(&targets), dest: dest.display().to_string(), overwrite: false };
        self.open_dialog(d);
    }

    fn confirm(&mut self, dialog: Dialog, ctx: &egui::Context) {
        let hidden = self.hidden();
        let a = self.active;
        match dialog {
            Dialog::Transfer { mv, sources, dest, overwrite, .. } => {
                let dest = PathBuf::from(dest.trim());
                if !dest.is_dir() {
                    return self.message("Copy", format!("The target folder does not exist:\n{}", dest.display()));
                }
                let ctx = ctx.clone();
                let (title, verb, job) = match sources {
                    Sources::Local(paths) if mv => {
                        ("Moving", "Moved", Job::spawn("Moving", ctx, move |r| ops::move_to(&paths, &dest, overwrite, r)))
                    }
                    Sources::Local(paths) => {
                        ("Copying", "Copied", Job::spawn("Copying", ctx, move |r| ops::copy(&paths, &dest, overwrite, r)))
                    }
                    Sources::Archive { archive, items } => (
                        "Extracting",
                        "Extracted",
                        Job::spawn("Extracting", ctx, move |r| archive::extract(&archive, &items, &dest, overwrite, r)),
                    ),
                };
                let _ = title;
                self.job = Some((job, JobKind::Transfer { verb, source_side: a }));
            }
            Dialog::Delete { paths, permanent, .. } => {
                let job = Job::spawn("Deleting", ctx.clone(), move |r| ops::delete(&paths, !permanent, r));
                self.job = Some((job, JobKind::Delete));
            }
            Dialog::Mkdir { dir, name } => match ops::mkdir(&dir, &name) {
                Ok(_) => {
                    let first = name.trim().split(['/', '\\']).next().unwrap_or_default().to_string();
                    let tab = self.tab_mut();
                    tab.reload(hidden);
                    if let Some(i) = tab.index_of(&first) {
                        tab.set_cursor(i);
                    }
                }
                Err(e) => self.message("New folder", e.to_string()),
            },
            Dialog::Rename { src, name } => match ops::rename(&src, &name) {
                Ok(_) => {
                    let tab = self.tab_mut();
                    tab.reload(hidden);
                    if let Some(i) = tab.index_of(name.trim()) {
                        tab.set_cursor(i);
                    }
                    let other = &mut self.panels[1 - a];
                    other.tab_mut().reload(hidden);
                }
                Err(e) => self.message("Rename", e.to_string()),
            },
            Dialog::Settings(draft) => {
                let hidden_changed = draft.show_hidden != self.settings.show_hidden;
                // The bookmarks / tabs in the draft may be stale; keep the live ones.
                let bookmarks = std::mem::take(&mut self.settings.bookmarks);
                self.settings = *draft;
                self.settings.bookmarks = bookmarks;
                if hidden_changed {
                    let hidden = self.hidden();
                    for p in &mut self.panels {
                        for t in &mut p.tabs {
                            t.reload(hidden);
                        }
                    }
                }
                self.save_settings();
            }
            Dialog::Message { .. } | Dialog::Bookmarks => {}
        }
    }

    fn finish_job(&mut self, report: Report, kind: JobKind) {
        let verb = match kind {
            JobKind::Transfer { verb, source_side } => {
                if report.errors.is_empty() && !report.cancelled {
                    self.panels[source_side].tab_mut().marked.clear();
                }
                verb
            }
            JobKind::Delete => "Deleted",
        };
        self.refresh_all();
        let mut msg = format!("{verb} {} file{}", report.files, if report.files == 1 { "" } else { "s" });
        if report.skipped > 0 {
            msg += &format!(", skipped {} that already exist", report.skipped);
        }
        if report.cancelled {
            msg += " — cancelled";
        }
        if report.errors.is_empty() {
            self.status = msg;
        } else {
            let mut text = format!("{msg}.\n\n{} problem(s):\n", report.errors.len());
            for e in report.errors.iter().take(50) {
                text += &format!("• {e}\n");
            }
            self.message("Some items failed", text);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Keyboard

    /// Panel keys are taken from egui's input before widgets see them (so Tab and the arrows do not
    /// move widget focus), unless a text field, dialog, menu or job has the keyboard.
    fn capture_keys(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.dialog.is_some()
            || self.job.is_some()
            || ctx.text_edit_focused()
            || egui::Popup::is_any_open(ctx)
            || self.panels.iter().any(|p| p.path_edit.is_some())
        {
            return;
        }
        let keys = &mut self.keys;
        raw.events.retain(|e| match e {
            egui::Event::Key { key, pressed, modifiers, .. } if !modifiers.alt => {
                if *pressed {
                    keys.push((*key, *modifiers));
                }
                false
            }
            _ => true,
        });
    }

    fn handle_key(&mut self, key: Key, m: Modifiers, ctx: &egui::Context) {
        let cmd = m.command;
        let shift = m.shift;
        let page = self.panels[self.active].page_rows.max(1) as isize;
        let c = match key {
            Key::ArrowUp if cmd => Some(Cmd::OpenInNewTab { other_side: shift }),
            Key::ArrowLeft if cmd => Some(Cmd::OpenInSide(0)),
            Key::ArrowRight if cmd => Some(Cmd::OpenInSide(1)),
            Key::ArrowUp => return self.move_by(-1, shift),
            Key::ArrowDown => return self.move_by(1, shift),
            Key::PageUp => return self.move_by(-page, shift),
            Key::PageDown => return self.move_by(page, shift),
            Key::Home => return self.move_by(isize::MIN / 2, shift),
            Key::End => return self.move_by(isize::MAX / 2, shift),
            Key::Enter if shift => Some(Cmd::OpenSystem),
            Key::Enter => Some(Cmd::Open),
            Key::Backspace if cmd => Some(Cmd::Delete { permanent: shift }),
            Key::Backspace => Some(Cmd::GoUp),
            Key::Tab if m.ctrl => Some(if shift { Cmd::PrevTab } else { Cmd::NextTab }),
            Key::Tab => Some(Cmd::SwitchPanel),
            Key::Space | Key::Insert => {
                let tab = self.tab_mut();
                let i = tab.cursor;
                tab.toggle_mark(i);
                tab.move_cursor(1);
                None
            }
            Key::F3 if cmd => Some(Cmd::Sort(SortColumn::Name)),
            Key::F4 if cmd => Some(Cmd::Sort(SortColumn::Ext)),
            Key::F5 if cmd => Some(Cmd::Sort(SortColumn::Modified)),
            Key::F6 if cmd => Some(Cmd::Sort(SortColumn::Size)),
            Key::F2 => Some(Cmd::Refresh),
            Key::F3 => Some(Cmd::View),
            Key::F4 => Some(Cmd::Edit),
            Key::F5 => Some(Cmd::Copy),
            Key::F6 if shift => Some(Cmd::Rename),
            Key::F6 => Some(Cmd::Move),
            Key::F7 => Some(Cmd::Mkdir),
            Key::F8 | Key::Delete => Some(Cmd::Delete { permanent: shift }),
            Key::A if cmd && shift => Some(Cmd::UnmarkAll),
            Key::A if cmd => Some(Cmd::MarkAll),
            Key::I if cmd => Some(Cmd::InvertMarks),
            Key::T if cmd => Some(Cmd::NewTab),
            Key::W if cmd => Some(Cmd::CloseTab),
            Key::D if cmd => Some(Cmd::Bookmarks),
            Key::R if cmd => Some(Cmd::Refresh),
            Key::U if cmd => Some(Cmd::Swap),
            Key::H if cmd && shift => Some(Cmd::ToggleHidden),
            Key::Comma if cmd => Some(Cmd::Settings),
            Key::Backslash if cmd => Some(Cmd::GoRoot),
            Key::Q if cmd => Some(Cmd::Quit),
            _ => None,
        };
        if let Some(c) = c {
            self.run(c, ctx);
        }
    }

    /// Move the cursor; with Shift, mark the rows passed over (or the row itself at an edge).
    fn move_by(&mut self, delta: isize, shift: bool) {
        let tab = self.tab_mut();
        let old = tab.cursor;
        tab.move_cursor(delta);
        let new = tab.cursor;
        if shift {
            if old == new {
                tab.toggle_mark(new);
            } else if new > old {
                tab.mark_range(old, new - 1);
            } else {
                tab.mark_range(new + 1, old);
            }
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

    // -----------------------------------------------------------------------------------------
    // Drawing

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true)))) && self.job.is_none() {
            self.volumes = volumes::volumes();
            self.refresh_all();
        }
        for (key, mods) in std::mem::take(&mut self.keys) {
            self.handle_key(key, mods, &ctx);
        }

        let mut cmds: Vec<Cmd> = Vec::new();
        egui::Panel::top("cc_menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| self.menus(ui, &mut cmds));
        });
        egui::Panel::bottom("cc_fkeys").show(ui, |ui| self.function_keys(ui, &mut cmds));
        let mut actions: [Vec<Action>; 2] = [vec![], vec![]];
        egui::CentralPanel::default().show(ui, |ui| {
            ui.columns(2, |cols| {
                for (side, col) in cols.iter_mut().enumerate() {
                    let cx = panel::Context {
                        active: side == self.active,
                        bookmarks: &self.settings.bookmarks,
                        volumes: &self.volumes,
                        id: if side == 0 { "left" } else { "right" },
                    };
                    actions[side] = panel::show(col, &mut self.panels[side], &cx);
                }
            });
        });
        for (side, list) in actions.into_iter().enumerate() {
            for action in list {
                self.panel_action(side, action);
            }
        }
        for c in cmds {
            self.run(c, &ctx);
        }

        if let Some(mut d) = self.dialog.take() {
            let fresh = std::mem::take(&mut self.dialog_fresh);
            let current = Self::local_folder(&self.tab().location);
            let is_bookmarks = matches!(d, Dialog::Bookmarks);
            match dialogs::show(&ctx, &mut d, fresh, &mut self.settings.bookmarks, Some(&current)) {
                Outcome::Open => self.dialog = Some(d),
                Outcome::Cancel => {
                    if is_bookmarks {
                        self.save_settings();
                    }
                }
                Outcome::Confirm => self.confirm(d, &ctx),
                Outcome::GoTo(path) => {
                    self.save_settings();
                    self.navigate(self.active, Location::Local(path), None);
                }
            }
        }

        self.job_ui(&ctx);

        let title = format!("CommanderCraft — {}", self.tab().location.display());
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn panel_action(&mut self, side: usize, action: Action) {
        let hidden = self.hidden();
        match action {
            Action::Activate => self.active = side,
            Action::Open(i) => {
                self.active = side;
                self.panels[side].tab_mut().cursor = i;
                self.open_entry(false);
            }
            Action::OpenInNewTab(i) => {
                let tab = self.panels[side].tab();
                if let Some(e) = tab.entries.get(i) {
                    let loc = tab.location.child(&e.name);
                    self.add_tab(side, loc, false);
                }
            }
            Action::GoTo(loc) => {
                self.active = side;
                self.navigate(side, loc, None);
            }
            Action::GoUp => {
                self.active = side;
                self.go_up(side);
            }
            Action::SelectTab(i) => {
                self.active = side;
                self.select_tab(side, i);
            }
            Action::CloseTab(i) => self.close_tab(side, i),
            Action::NewTab => {
                let loc = self.panels[side].tab().location.clone();
                self.add_tab(side, loc, true);
            }
            Action::Sort(col) => {
                self.active = side;
                let tab = self.panels[side].tab_mut();
                let sort = tab.sort.toggled(col);
                tab.set_sort(sort);
            }
            Action::AddBookmark => {
                self.active = side;
                let dir = Self::local_folder(&self.panels[side].tab().location);
                dialogs::add_bookmark(&mut self.settings.bookmarks, &dir);
                self.save_settings();
            }
            Action::ManageBookmarks => {
                self.active = side;
                self.open_dialog(Dialog::Bookmarks);
            }
        }
        let _ = hidden;
    }

    fn menus(&self, ui: &mut egui::Ui, out: &mut Vec<Cmd>) {
        let c = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };
        let item = |ui: &mut egui::Ui, label: &str, shortcut: String, cmd: Cmd, out: &mut Vec<Cmd>| {
            if ui.add(egui::Button::new(label).shortcut_text(shortcut)).clicked() {
                out.push(cmd);
            }
        };
        ui.menu_button("Files", |ui| {
            item(ui, "Open", "Enter".into(), Cmd::Open, out);
            item(ui, "Open with system default", "Shift+Enter".into(), Cmd::OpenSystem, out);
            item(ui, "View", "F3".into(), Cmd::View, out);
            item(ui, "Edit", "F4".into(), Cmd::Edit, out);
            ui.separator();
            item(ui, "Copy", "F5".into(), Cmd::Copy, out);
            item(ui, "Move", "F6".into(), Cmd::Move, out);
            item(ui, "Rename", "Shift+F6".into(), Cmd::Rename, out);
            item(ui, "New folder", "F7".into(), Cmd::Mkdir, out);
            item(ui, "Delete to trash", "F8".into(), Cmd::Delete { permanent: false }, out);
            item(ui, "Delete permanently", "Shift+F8".into(), Cmd::Delete { permanent: true }, out);
            ui.separator();
            item(ui, "Quit", format!("{c}Q"), Cmd::Quit, out);
        });
        ui.menu_button("Mark", |ui| {
            item(ui, "Mark all", format!("{c}A"), Cmd::MarkAll, out);
            item(ui, "Unmark all", format!("{c}Shift+A"), Cmd::UnmarkAll, out);
            item(ui, "Invert marks", format!("{c}I"), Cmd::InvertMarks, out);
        });
        ui.menu_button("View", |ui| {
            let mut hidden = self.settings.show_hidden;
            if ui.checkbox(&mut hidden, format!("Show hidden files  ({c}Shift+H)")).clicked() {
                out.push(Cmd::ToggleHidden);
            }
            ui.separator();
            item(ui, "Sort by name", format!("{c}F3"), Cmd::Sort(SortColumn::Name), out);
            item(ui, "Sort by extension", format!("{c}F4"), Cmd::Sort(SortColumn::Ext), out);
            item(ui, "Sort by date", format!("{c}F5"), Cmd::Sort(SortColumn::Modified), out);
            item(ui, "Sort by size", format!("{c}F6"), Cmd::Sort(SortColumn::Size), out);
            ui.separator();
            item(ui, "Refresh", format!("{c}R"), Cmd::Refresh, out);
            item(ui, "Swap panels", format!("{c}U"), Cmd::Swap, out);
        });
        ui.menu_button("Tabs", |ui| {
            item(ui, "New tab", format!("{c}T"), Cmd::NewTab, out);
            item(ui, "Open folder in new tab", format!("{c}↑"), Cmd::OpenInNewTab { other_side: false }, out);
            item(ui, "Open folder in new tab on other side", format!("{c}Shift+↑"), Cmd::OpenInNewTab { other_side: true }, out);
            item(ui, "Close tab", format!("{c}W"), Cmd::CloseTab, out);
            ui.separator();
            item(ui, "Next tab", "Ctrl+Tab".into(), Cmd::NextTab, out);
            item(ui, "Previous tab", "Ctrl+Shift+Tab".into(), Cmd::PrevTab, out);
        });
        ui.menu_button("Bookmarks", |ui| {
            item(ui, "Add current folder", String::new(), Cmd::AddBookmark, out);
            item(ui, "Manage…", format!("{c}D"), Cmd::Bookmarks, out);
            if !self.settings.bookmarks.is_empty() {
                ui.separator();
            }
            for b in &self.settings.bookmarks {
                if ui.button(&b.name).on_hover_text(b.path.display().to_string()).clicked() {
                    out.push(Cmd::GoTo(b.path.clone()));
                }
            }
        });
        ui.menu_button("Settings", |ui| {
            item(ui, "Settings…", format!("{c},"), Cmd::Settings, out);
        });
    }

    fn function_keys(&self, ui: &mut egui::Ui, out: &mut Vec<Cmd>) {
        if !self.status.is_empty() {
            ui.label(egui::RichText::new(&self.status).small().weak());
        }
        let keys = [
            ("F3", "View", Cmd::View),
            ("F4", "Edit", Cmd::Edit),
            ("F5", "Copy", Cmd::Copy),
            ("F6", "Move", Cmd::Move),
            ("Shift+F6", "Rename", Cmd::Rename),
            ("F7", "New folder", Cmd::Mkdir),
            ("F8", "Delete", Cmd::Delete { permanent: false }),
        ];
        ui.columns(keys.len(), |cols| {
            for ((k, label, cmd), col) in keys.into_iter().zip(cols.iter_mut()) {
                let w = col.available_width();
                if col.add_sized([w, 22.0], egui::Button::new(format!("{k}  {label}"))).clicked() {
                    out.push(cmd);
                }
            }
        });
        ui.add_space(2.0);
    }

    fn job_ui(&mut self, ctx: &egui::Context) {
        let Some((job, _)) = &self.job else { return };
        if let Some(report) = job.take_result() {
            if let Some((_, kind)) = self.job.take() {
                self.finish_job(report, kind);
            }
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        if job.started.elapsed() < Duration::from_millis(250) {
            return; // quick operations finish without a flash of the dialog
        }
        let p = job.progress();
        egui::Modal::new(egui::Id::new("cc_job")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(&job.title);
            ui.add(egui::Label::new(&p.current).truncate());
            let fraction = if p.bytes_total > 0 {
                p.bytes_done as f32 / p.bytes_total as f32
            } else if p.files_total > 0 {
                p.files_seen as f32 / p.files_total as f32
            } else {
                0.0
            };
            ui.add(egui::ProgressBar::new(fraction.clamp(0.0, 1.0)).show_percentage());
            ui.label(format!(
                "{} / {}   ·   {} files",
                commandercraft_core::format_size(p.bytes_done),
                commandercraft_core::format_size(p.bytes_total),
                p.files_total
            ));
            ui.add_space(4.0);
            if job.is_cancelling() {
                ui.label("Cancelling…");
            } else if ui.button("Cancel").clicked() {
                job.cancel();
            }
        });
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
