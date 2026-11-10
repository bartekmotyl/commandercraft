//! What the commands do, and which keys trigger them.

use std::path::{Path, PathBuf};

use commandercraft_core::ops::{self, Report};
use commandercraft_core::sort::SortColumn;
use commandercraft_core::{Location, archive, launch, volumes};
use egui::{Key, Modifiers};

use crate::dialogs::{self, Dialog, Sources};
use crate::jobs::Job;
use crate::panel::Action;
use crate::viewer::{QUICK_LIMIT, VIEW_LIMIT, Viewer};
use crate::{CommanderApp, JobKind, QuickView, describe};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Cmd {
    Open,
    OpenSystem,
    OpenAsArchive,
    View,
    QuickView,
    Edit,
    Copy,
    Move,
    Rename,
    Mkdir,
    Delete { permanent: bool },
    Extract,
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
    ManageBookmarks,
    AddBookmark,
    GoTo(PathBuf),
    GoToClipboard { new_tab: bool },
    CopyPath,
    CopySelectedPaths,
    Terminal,
    Settings,
    GoUp,
    GoRoot,
    SwitchPanel,
    Quit,
}

impl CommanderApp {
    pub(crate) fn run(&mut self, cmd: Cmd, ctx: &egui::Context) {
        let a = self.active;
        match cmd {
            Cmd::Open => self.open_entry(false),
            Cmd::OpenSystem => self.open_entry(true),
            Cmd::OpenAsArchive => self.open_as_archive(),
            Cmd::View => {
                if let Some((e, path)) = self.cursor_file() {
                    match Viewer::open(&path, e.name.clone(), VIEW_LIMIT) {
                        Ok(v) => self.viewer = Some(v),
                        Err(err) => self.message("Cannot view", format!("{}: {err}", path.display())),
                    }
                }
            }
            Cmd::QuickView => {
                self.quick_view = match self.quick_view {
                    Some(_) => None,
                    None => Some(QuickView { side: 1 - a, key: None, viewer: None, note: String::new() }),
                };
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
                    return self.message("Delete", "Archives are read-only.");
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
            Cmd::Extract => self.start_extract(),
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
                if side != a || loc != self.tab().location {
                    self.navigate(side, loc, None);
                }
            }
            Cmd::Swap => {
                self.panels.swap(0, 1);
                if let Some(q) = &mut self.quick_view {
                    q.side = 1 - q.side;
                }
            }
            Cmd::MarkAll => self.tab_mut().mark_all(false),
            Cmd::UnmarkAll => self.tab_mut().marked.clear(),
            Cmd::InvertMarks => self.tab_mut().invert_marks(),
            Cmd::Sort(col) => {
                let sort = self.tab().sort.toggled(col);
                self.tab_mut().set_sort(sort);
            }
            Cmd::ToggleHidden => {
                self.settings.show_hidden = !self.settings.show_hidden;
                self.reload_all_tabs();
            }
            Cmd::Bookmarks => {
                // Below the tab strip and path bar of the active panel.
                let r = self.panel_rects[a];
                let anchor = if r.is_finite() && r.width() > 0.0 { r.min + egui::vec2(10.0, 58.0) } else { egui::pos2(40.0, 120.0) };
                self.open_dialog(Dialog::QuickBookmarks { selected: 0, anchor });
            }
            Cmd::ManageBookmarks => self.open_dialog(Dialog::Bookmarks),
            Cmd::AddBookmark => {
                let dir = Self::local_folder(&self.tab().location);
                dialogs::add_bookmark(&mut self.settings.bookmarks, &dir);
                self.status = format!("Bookmarked {}", dir.display());
                self.save_settings();
            }
            Cmd::GoTo(path) => self.navigate(a, Location::Local(path), None),
            Cmd::GoToClipboard { new_tab } => self.go_to_clipboard(new_tab),
            Cmd::CopyPath => {
                let text = self.tab().location.display();
                self.status = format!("Copied {text}");
                ctx.copy_text(text);
            }
            Cmd::CopySelectedPaths => {
                let loc = self.tab().location.clone();
                let paths: Vec<String> = self
                    .tab()
                    .targets()
                    .iter()
                    .map(|e| match &loc {
                        Location::Local(dir) => dir.join(&e.name).display().to_string(),
                        Location::Archive { .. } => format!("{}{}{}", loc.display(), std::path::MAIN_SEPARATOR, e.name),
                    })
                    .collect();
                if !paths.is_empty() {
                    self.status = format!("Copied {} path{}", paths.len(), if paths.len() == 1 { "" } else { "s" });
                    ctx.copy_text(paths.join("\n"));
                }
            }
            Cmd::Terminal => {
                let dir = Self::local_folder(&self.tab().location);
                let r = launch::open_terminal(&dir, &self.settings.terminal);
                self.launch(r, &dir);
            }
            Cmd::Settings => self.open_dialog(Dialog::Settings(Box::new(self.settings.clone()))),
            Cmd::GoUp => self.go_up(a),
            Cmd::GoRoot => {
                let root = match &self.tab().location {
                    Location::Local(p) => Location::Local(p.ancestors().last().unwrap_or(p).to_path_buf()),
                    Location::Archive { archive, .. } => Location::Archive { archive: archive.clone(), inner: String::new() },
                };
                self.navigate(a, root, None);
            }
            Cmd::SwitchPanel => {
                // With quick view on, the preview side cannot take the focus.
                if self.quick_view.is_none() {
                    self.active = 1 - a;
                }
            }
            Cmd::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// Enter: go into folders and archives, open files.
    pub(crate) fn open_entry(&mut self, with_system: bool) {
        let a = self.active;
        let tab = self.tab();
        let Some(e) = tab.current().cloned() else { return };
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

    /// Ctrl+PageDown: go into the file under the cursor as an archive, whatever its extension
    /// (`.epub`, `.docx`, `.jar`… are zips); the format is recognised from the content.
    fn open_as_archive(&mut self) {
        let a = self.active;
        let Some(e) = self.tab().current().cloned() else { return };
        if e.is_dir {
            return self.open_entry(false);
        }
        let path = match self.tab().location.clone() {
            Location::Local(dir) => dir.join(&e.name),
            Location::Archive { archive, inner } => match self.extract_temp(&archive, &archive::join(&inner, &e.name)) {
                Ok(p) => p,
                Err(err) => return self.message("Cannot extract", err),
            },
        };
        if archive::detect_content(&path).is_none() {
            return self.message(
                "Open as archive",
                format!("\"{}\" is not an archive CommanderCraft can open (zip, tar, gzip, bzip2, xz).", e.name),
            );
        }
        self.navigate(a, Location::Archive { archive: path, inner: String::new() }, None);
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

    /// Alt+F9: the marked archives (or the one under the cursor; inside an archive: that archive).
    fn start_extract(&mut self) {
        let archives: Vec<PathBuf> = match &self.tab().location {
            Location::Archive { archive, .. } => vec![archive.clone()],
            Location::Local(_) => self.tab().target_paths().into_iter().filter(|p| p.is_file() && archive::detect(p).is_some()).collect(),
        };
        if archives.is_empty() {
            return self.message("Extract", "Mark one or more archives (zip, tar, tar.gz, …) first.");
        }
        let dest = match &self.panels[1 - self.active].tab().location {
            Location::Local(p) => p.clone(),
            Location::Archive { .. } => Self::local_folder(&self.tab().location),
        };
        let summary = match archives.as_slice() {
            [one] => format!("\"{}\"", one.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()),
            many => format!("{} archives", many.len()),
        };
        let d = Dialog::Extract { own_folder: archives.len() > 1, archives, summary, dest: dest.display().to_string(), overwrite: false };
        self.open_dialog(d);
    }

    fn go_to_clipboard(&mut self, new_tab: bool) {
        let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
        let Some(path) = clipboard_path(&text) else {
            return self.message("Go to clipboard path", "The clipboard does not hold a path that exists.");
        };
        let (location, focus) = if path.is_dir() {
            (Location::Local(path), None)
        } else {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            (Location::Local(path.parent().map(Path::to_path_buf).unwrap_or_default()), name)
        };
        let a = self.active;
        if new_tab {
            self.add_tab(a, location, true);
            if let Some(n) = focus {
                let tab = self.tab_mut();
                if let Some(i) = tab.index_of(&n) {
                    tab.set_cursor(i);
                }
            }
        } else {
            self.navigate(a, location, focus.as_deref());
        }
    }

    pub(crate) fn confirm(&mut self, dialog: Dialog, ctx: &egui::Context) {
        let hidden = self.hidden();
        let a = self.active;
        match dialog {
            Dialog::Transfer { mv, sources, dest, overwrite, .. } => {
                let dest = PathBuf::from(dest.trim());
                if !dest.is_dir() {
                    return self.message("Copy", format!("The target folder does not exist:\n{}", dest.display()));
                }
                let ctx = ctx.clone();
                let (verb, job) = match sources {
                    Sources::Local(paths) if mv => ("Moved", Job::spawn("Moving", ctx, move |r| ops::move_to(&paths, &dest, overwrite, r))),
                    Sources::Local(paths) => ("Copied", Job::spawn("Copying", ctx, move |r| ops::copy(&paths, &dest, overwrite, r))),
                    Sources::Archive { archive, items } => {
                        ("Extracted", Job::spawn("Extracting", ctx, move |r| archive::extract(&archive, &items, &dest, overwrite, r)))
                    }
                };
                self.job = Some((job, JobKind::Transfer { verb, source_side: a }));
            }
            Dialog::Extract { archives, dest, own_folder, overwrite, .. } => {
                let dest = PathBuf::from(dest.trim());
                if let Err(e) = std::fs::create_dir_all(&dest) {
                    return self.message("Extract", format!("{}: {e}", dest.display()));
                }
                let job = Job::spawn("Extracting", ctx.clone(), move |r| archive::extract_all(&archives, &dest, own_folder, overwrite, r));
                self.job = Some((job, JobKind::Extract));
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
                    self.panels[1 - a].tab_mut().reload(hidden);
                }
                Err(e) => self.message("Rename", e.to_string()),
            },
            Dialog::Settings(draft) => {
                let hidden_changed = draft.show_hidden != self.settings.show_hidden;
                // Bookmarks, tabs and the splitter may have changed while the dialog was open.
                let keep = (std::mem::take(&mut self.settings.bookmarks), self.settings.split);
                self.settings = *draft;
                (self.settings.bookmarks, self.settings.split) = keep;
                if hidden_changed {
                    self.reload_all_tabs();
                }
                self.style_dirty = true;
                self.save_settings();
            }
            Dialog::Message { .. } | Dialog::Bookmarks | Dialog::QuickBookmarks { .. } => {}
        }
    }

    pub(crate) fn finish_job(&mut self, report: Report, kind: JobKind) {
        let verb = match kind {
            JobKind::Transfer { verb, source_side } => {
                if report.errors.is_empty() && !report.cancelled {
                    self.panels[source_side].tab_mut().marked.clear();
                }
                verb
            }
            JobKind::Extract => "Extracted",
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

    pub(crate) fn panel_action(&mut self, side: usize, action: Action) {
        match action {
            Action::Activate => {
                if self.quick_view.as_ref().is_none_or(|q| q.side != side) {
                    self.active = side;
                }
            }
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
    }

    /// Keep the quick view in step with the cursor of the active panel.
    pub(crate) fn update_quick_view(&mut self) {
        let Some(q) = &self.quick_view else { return };
        let tab = self.panels[self.active].tab();
        let Some(e) = tab.current().cloned() else { return };
        let key = (tab.location.display(), e.name.clone());
        if q.key.as_ref() == Some(&key) {
            return;
        }
        let location = tab.location.clone();
        let (viewer, note) = if e.is_dir {
            (None, if e.is_parent { String::new() } else { format!("Folder \"{}\"", e.name) })
        } else {
            let path = match &location {
                Location::Local(dir) => Some(dir.join(&e.name)),
                Location::Archive { archive, inner } if e.size <= 4 * 1024 * 1024 => {
                    self.extract_temp(archive, &archive::join(inner, &e.name)).ok()
                }
                Location::Archive { .. } => None,
            };
            match path.map(|p| Viewer::open(&p, e.name.clone(), QUICK_LIMIT)) {
                Some(Ok(v)) => (Some(v), String::new()),
                Some(Err(err)) => (None, err.to_string()),
                None => (None, "Too large to preview from inside an archive.".into()),
            }
        };
        if let Some(q) = &mut self.quick_view {
            q.key = Some(key);
            q.viewer = viewer;
            q.note = note;
        }
    }

    // -----------------------------------------------------------------------------------------
    // Keyboard

    /// Panel keys are taken from egui's input before widgets see them (so Tab and the arrows do not
    /// move widget focus), unless a text field, dialog, menu, viewer or job has the keyboard.
    /// Typed text goes to the command line.
    pub(crate) fn capture_keys(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.dialog.is_some()
            || self.job.is_some()
            || self.viewer.is_some()
            || ctx.text_edit_focused()
            || egui::Popup::is_any_open(ctx)
            || self.panels.iter().any(|p| p.path_edit.is_some())
        {
            return;
        }
        let keys = &mut self.keys;
        let cmdline = &mut self.cmdline;
        let mut focus_cmdline = false;
        raw.events.retain(|e| match e {
            // Alt+F4 stays with the system (close window).
            egui::Event::Key { key: Key::F4, modifiers, .. } if modifiers.alt => true,
            egui::Event::Key { key, pressed, modifiers, .. } => {
                if *pressed {
                    keys.push((*key, *modifiers));
                }
                false
            }
            egui::Event::Text(t) if !(t == " " && cmdline.is_empty()) => {
                cmdline.push_str(t);
                focus_cmdline = true;
                false
            }
            _ => true,
        });
        if focus_cmdline {
            self.cmdline_focus = true;
        }
    }

    pub(crate) fn handle_key(&mut self, key: Key, m: Modifiers, ctx: &egui::Context) {
        let cmd = m.command;
        // Letter shortcuts: ⌘ or Control on macOS, Ctrl elsewhere.
        let letter = m.command || m.ctrl;
        let shift = m.shift;
        let alt = m.alt;
        let page = self.panels[self.active].page_rows.max(1) as isize;
        self.keyboard_nav = true;
        let c = match key {
            Key::ArrowUp if cmd => Some(Cmd::OpenInNewTab { other_side: shift }),
            Key::ArrowLeft if cmd => Some(Cmd::OpenInSide(0)),
            Key::ArrowRight if cmd => Some(Cmd::OpenInSide(1)),
            Key::ArrowUp => return self.move_by(-1, shift),
            Key::ArrowDown => return self.move_by(1, shift),
            Key::PageDown if cmd => Some(Cmd::OpenAsArchive),
            Key::PageUp if cmd => Some(Cmd::GoUp),
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
            Key::F9 if alt => Some(Cmd::Extract),
            Key::F2 => Some(Cmd::Refresh),
            Key::F3 => Some(Cmd::View),
            Key::F4 => Some(Cmd::Edit),
            Key::F5 => Some(Cmd::Copy),
            Key::F6 if shift => Some(Cmd::Rename),
            Key::F6 => Some(Cmd::Move),
            Key::F7 => Some(Cmd::Mkdir),
            Key::F8 | Key::Delete => Some(Cmd::Delete { permanent: shift }),
            Key::C if letter && alt => Some(Cmd::CopySelectedPaths),
            Key::C if letter && shift => Some(Cmd::CopyPath),
            Key::V if letter && alt => Some(Cmd::GoToClipboard { new_tab: true }),
            Key::V if letter && shift => Some(Cmd::GoToClipboard { new_tab: false }),
            Key::T if letter && shift => Some(Cmd::Terminal),
            Key::E if letter && shift => Some(Cmd::Extract),
            Key::A if letter && shift => Some(Cmd::UnmarkAll),
            Key::A if letter => Some(Cmd::MarkAll),
            Key::I if letter => Some(Cmd::InvertMarks),
            Key::T if letter => Some(Cmd::NewTab),
            Key::W if letter => Some(Cmd::CloseTab),
            Key::D if letter => Some(Cmd::Bookmarks),
            Key::R if letter => Some(Cmd::Refresh),
            Key::U if letter => Some(Cmd::Swap),
            Key::H if letter && shift => Some(Cmd::ToggleHidden),
            Key::Comma if letter => Some(Cmd::Settings),
            Key::Backslash if letter => Some(Cmd::GoRoot),
            // Ctrl+Q is quick view; on macOS ⌘Q still quits.
            Key::Q if m.ctrl => Some(Cmd::QuickView),
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
}

/// A path from clipboard text: the first line, without quotes or a `file://` prefix; must exist.
pub(crate) fn clipboard_path(text: &str) -> Option<PathBuf> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_matches(|c| c == '"' || c == '\'');
    let line = line.strip_prefix("file://").map(|s| s.replace("%20", " ")).unwrap_or_else(|| line.to_string());
    let path = PathBuf::from(line);
    path.exists().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_paths() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().display().to_string();
        assert_eq!(clipboard_path(&format!("  \"{p}\"  \nother")), Some(d.path().to_path_buf()));
        assert_eq!(clipboard_path("definitely/not/here/xyz"), None);
        assert_eq!(clipboard_path(""), None);
    }
}
