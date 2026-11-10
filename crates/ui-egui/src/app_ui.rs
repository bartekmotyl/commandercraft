//! Drawing the window: menu bar, toolbar, the two panels with a splitter (or the viewer), the
//! command line and function keys; plus per-frame housekeeping (theme, fonts, auto refresh).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use commandercraft_core::settings::ThemeChoice;
use commandercraft_core::sort::SortColumn;
use commandercraft_core::watch::DirWatcher;
use commandercraft_core::{Location, launch, volumes};
use egui::{RichText, Sense};

use crate::commands::Cmd;
use crate::dialogs::{self, Dialog, Outcome};
use crate::fonts::FontLoader;
use crate::panel::{self, Action};
use crate::viewer::ViewerAction;
use crate::{CommanderApp, theme};

/// Refresh panels at most this often when their folders keep changing.
const AUTO_REFRESH_GAP: Duration = Duration::from_millis(400);

impl CommanderApp {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.housekeeping(&ctx);
        for (key, mods) in std::mem::take(&mut self.keys) {
            self.handle_key(key, mods, &ctx);
        }
        if ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
            self.keyboard_nav = false;
        }
        self.update_quick_view();

        let mut cmds: Vec<Cmd> = Vec::new();
        egui::Panel::top("cc_menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| self.menus(ui, &mut cmds));
        });
        egui::Panel::top("cc_toolbar").show(ui, |ui| self.toolbar(ui, &mut cmds));
        egui::Panel::bottom("cc_fkeys").show(ui, |ui| {
            self.command_line(ui);
            self.function_keys(ui, &mut cmds);
        });
        let mut actions: [Vec<Action>; 2] = [vec![], vec![]];
        egui::CentralPanel::default().show(ui, |ui| {
            if self.viewer.is_some() {
                self.viewer_ui(ui);
            } else {
                actions = self.panels_ui(ui);
            }
        });
        for (side, list) in actions.into_iter().enumerate() {
            for action in list {
                self.panel_action(side, action);
            }
        }
        for c in cmds {
            self.run(c, &ctx);
        }
        self.dialog_ui(&ctx);
        self.job_ui(&ctx);

        let title = match &self.viewer {
            Some(v) => format!("{} — CommanderCraft", v.path.display()),
            None => format!("CommanderCraft — {}", self.tab().location.display()),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    /// Theme and fonts, refreshing on focus, and refreshing folders that changed on disk.
    fn housekeeping(&mut self, ctx: &egui::Context) {
        if self.fonts.is_none() {
            self.fonts = Some(if self.load_system_fonts { FontLoader::start(ctx) } else { FontLoader::none() });
        }
        if let Some(f) = &mut self.fonts
            && f.poll()
        {
            self.style_dirty = true;
        }
        if std::mem::take(&mut self.style_dirty) {
            theme::apply(ctx, &self.settings.appearance);
            if let Some(f) = &self.fonts {
                f.apply(ctx, &self.settings.appearance);
            }
        }
        if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true)))) && self.job.is_none() {
            self.volumes = volumes::volumes();
            self.refresh_all();
        }
        self.auto_refresh(ctx);
    }

    fn auto_refresh(&mut self, ctx: &egui::Context) {
        if !self.load_system_fonts {
            return; // headless tests: no watcher threads
        }
        if self.watcher.is_none() {
            let c = ctx.clone();
            self.watcher = Some(DirWatcher::new(move || c.request_repaint_after(AUTO_REFRESH_GAP)));
        }
        let mut dirs: Vec<PathBuf> = self.panels.iter().filter_map(|p| p.tab().location.local_path().map(|p| p.to_path_buf())).collect();
        dirs.dedup();
        let Some(w) = &mut self.watcher else { return };
        w.watch(&dirs);
        if self.last_auto_refresh.elapsed() < AUTO_REFRESH_GAP {
            ctx.request_repaint_after(AUTO_REFRESH_GAP);
            return;
        }
        let changed: Vec<PathBuf> = dirs.into_iter().filter(|d| w.take_changed(d)).collect();
        w.prune();
        if changed.is_empty() {
            return;
        }
        self.last_auto_refresh = Instant::now();
        let hidden = self.hidden();
        for p in &mut self.panels {
            let tab = p.tab_mut();
            if tab.location.local_path().is_some_and(|d| changed.iter().any(|c| c == d)) {
                tab.reload(hidden);
            }
        }
    }

    fn panels_ui(&mut self, ui: &mut egui::Ui) -> [Vec<Action>; 2] {
        let avail = ui.available_rect_before_wrap();
        let handle_w = 6.0;
        let ratio = self.settings.split_ratio();
        let left_w = ((avail.width() - handle_w) * ratio).round();
        let left = egui::Rect::from_min_size(avail.min, egui::vec2(left_w, avail.height()));
        let handle = egui::Rect::from_min_max(egui::pos2(left.max.x, avail.min.y), egui::pos2(left.max.x + handle_w, avail.max.y));
        let right = egui::Rect::from_min_max(egui::pos2(handle.max.x, avail.min.y), avail.max);

        // Splitter: drag to resize, double-click for half and half.
        let r = ui.interact(handle, egui::Id::new("cc_split"), Sense::click_and_drag());
        if r.hovered() || r.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            ui.painter().vline(handle.center().x, handle.y_range().shrink(4.0), egui::Stroke::new(2.0, theme::current(ui.ctx()).accent));
        }
        if r.dragged()
            && let Some(p) = r.interact_pointer_pos()
        {
            self.settings.split = ((p.x - avail.min.x - handle_w / 2.0) / (avail.width() - handle_w)).clamp(0.15, 0.85);
        }
        if r.double_clicked() {
            self.settings.split = 0.5;
        }

        let palette = theme::current(ui.ctx());
        let row_h = theme::row_height(&self.settings.appearance);
        let mut actions: [Vec<Action>; 2] = [vec![], vec![]];
        for (side, rect) in [(0, left), (1, right)] {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
            if self.quick_view.as_ref().is_some_and(|q| q.side == side) {
                self.quick_view_ui(&mut child, palette);
                continue;
            }
            let cx = panel::Context {
                active: side == self.active,
                bookmarks: &self.settings.bookmarks,
                volumes: &self.volumes,
                id: if side == 0 { "left" } else { "right" },
                row_h,
                monospace: self.settings.appearance.list_monospace,
                palette,
                show_hover: !self.keyboard_nav,
            };
            self.panel_rects[side] = rect;
            actions[side] = panel::show(&mut child, &mut self.panels[side], &cx);
        }
        actions
    }

    fn quick_view_ui(&mut self, ui: &mut egui::Ui, palette: theme::Palette) {
        let mono = &mut self.settings.viewer_monospace;
        let wrap = &mut self.settings.viewer_wrap;
        let Some(q) = &mut self.quick_view else { return };
        egui::Frame::new().stroke(egui::Stroke::new(1.0, palette.border)).inner_margin(4.0).corner_radius(4.0).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Quick view").strong());
                if let Some(v) = &q.viewer {
                    ui.label(RichText::new(&v.title).weak());
                }
            });
            match &mut q.viewer {
                Some(v) => {
                    v.toolbar(ui, mono, wrap);
                    ui.separator();
                    v.body(ui, *mono, *wrap);
                }
                None => {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new(&q.note).weak()));
                }
            }
        });
    }

    fn viewer_ui(&mut self, ui: &mut egui::Ui) {
        let mono = &mut self.settings.viewer_monospace;
        let wrap = &mut self.settings.viewer_wrap;
        let Some(v) = &mut self.viewer else { return };
        let mut close = matches!(v.keys(ui, mono, wrap), ViewerAction::Close);
        ui.horizontal(|ui| {
            ui.label(RichText::new(v.path.display().to_string()).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("✖ Close").on_hover_text("Esc, F3, Q").clicked() {
                    close = true;
                }
                ui.label(RichText::new("H hex · W wrap · M monospace").small().weak());
            });
        });
        v.toolbar(ui, mono, wrap);
        ui.separator();
        v.body(ui, *mono, *wrap);
        if close {
            self.viewer = None;
        }
    }

    fn dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.dialog.take() else { return };
        let fresh = std::mem::take(&mut self.dialog_fresh);
        let current = Self::local_folder(&self.tab().location);
        let is_bookmarks = matches!(d, Dialog::Bookmarks);
        let fonts = self.fonts.as_ref().and_then(|f| f.fonts.clone());
        let env = dialogs::Env { bookmarks: &mut self.settings.bookmarks, current: Some(&current), fonts: fonts.as_deref() };
        match dialogs::show(ctx, &mut d, fresh, env) {
            Outcome::Open => self.dialog = Some(d),
            Outcome::Cancel => {
                if is_bookmarks {
                    self.save_settings();
                }
            }
            Outcome::Confirm => self.confirm(d, ctx),
            Outcome::Manage => self.open_dialog(Dialog::Bookmarks),
            Outcome::GoTo(path) => {
                self.save_settings();
                self.navigate(self.active, Location::Local(path), None);
            }
        }
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
            item(ui, "Open as archive", format!("{c}PgDn"), Cmd::OpenAsArchive, out);
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
            item(ui, "Extract archives…", "Alt+F9".into(), Cmd::Extract, out);
            ui.separator();
            item(ui, "Copy folder path", format!("{c}Shift+C"), Cmd::CopyPath, out);
            item(ui, "Copy paths of marked files", format!("{c}Alt+C"), Cmd::CopySelectedPaths, out);
            item(ui, "Go to path on clipboard", format!("{c}Shift+V"), Cmd::GoToClipboard { new_tab: false }, out);
            item(ui, "Go to path on clipboard in new tab", format!("{c}Alt+V"), Cmd::GoToClipboard { new_tab: true }, out);
            ui.separator();
            item(ui, "Terminal here", format!("{c}Shift+T"), Cmd::Terminal, out);
            ui.separator();
            item(ui, "Quit", if cfg!(target_os = "macos") { "⌘Q".into() } else { "Alt+F4".into() }, Cmd::Quit, out);
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
            let mut qv = self.quick_view.is_some();
            if ui.checkbox(&mut qv, "Quick view  (Ctrl+Q)").clicked() {
                out.push(Cmd::QuickView);
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
            item(ui, "Show list", format!("{c}D"), Cmd::Bookmarks, out);
            item(ui, "Manage…", String::new(), Cmd::ManageBookmarks, out);
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

    fn toolbar(&mut self, ui: &mut egui::Ui, out: &mut Vec<Cmd>) {
        let c = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let tool = |ui: &mut egui::Ui, label: &str, tip: String, cmd: Cmd, out: &mut Vec<Cmd>| {
                if ui.button(label).on_hover_text(tip).clicked() {
                    out.push(cmd);
                }
            };
            tool(ui, "⟳", format!("Refresh  ({c}R)"), Cmd::Refresh, out);
            if ui
                .selectable_label(self.settings.show_hidden, "👁 Hidden")
                .on_hover_text(format!("Show hidden files  ({c}Shift+H)"))
                .clicked()
            {
                out.push(Cmd::ToggleHidden);
            }
            if ui
                .selectable_label(self.quick_view.is_some(), "Quick view")
                .on_hover_text("Preview the file under the cursor  (Ctrl+Q)")
                .clicked()
            {
                out.push(Cmd::QuickView);
            }
            ui.separator();
            tool(ui, "+ Tab", format!("New tab  ({c}T)"), Cmd::NewTab, out);
            tool(ui, "★ Bookmarks", format!("Bookmarks  ({c}D)"), Cmd::Bookmarks, out);
            tool(ui, "↔ Swap", format!("Swap panels  ({c}U)"), Cmd::Swap, out);
            ui.separator();
            tool(ui, "🔍 View", "Internal viewer  (F3)".into(), Cmd::View, out);
            tool(ui, "📦 Extract", "Extract archives  (Alt+F9)".into(), Cmd::Extract, out);
            tool(ui, "📋 Path", format!("Copy folder path  ({c}Shift+C)"), Cmd::CopyPath, out);
            tool(ui, ">_ Terminal", format!("Terminal here  ({c}Shift+T)"), Cmd::Terminal, out);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙").on_hover_text(format!("Settings  ({c},)")).clicked() {
                    out.push(Cmd::Settings);
                }
                let theme = &mut self.settings.appearance.theme;
                let before = *theme;
                ui.selectable_value(theme, ThemeChoice::Dark, "🌙").on_hover_text("Dark theme");
                ui.selectable_value(theme, ThemeChoice::Light, "☀").on_hover_text("Light theme");
                ui.selectable_value(theme, ThemeChoice::System, "Auto").on_hover_text("Follow the system theme");
                if *theme != before {
                    self.style_dirty = true;
                }
            });
        });
    }

    /// The command line: runs a shell command in the active panel's folder.
    fn command_line(&mut self, ui: &mut egui::Ui) {
        let dir = Self::local_folder(&self.tab().location);
        let id = egui::Id::new("cc_cmdline");
        let mut run: Option<bool> = None;
        ui.horizontal(|ui| {
            let mut prompt = dir.display().to_string();
            if prompt.chars().count() > 48 {
                prompt = format!("…{}", prompt.chars().rev().take(46).collect::<Vec<_>>().into_iter().rev().collect::<String>());
            }
            ui.label(RichText::new(format!("{prompt}>")).weak());
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.cmdline)
                    .id(id)
                    .desired_width(f32::INFINITY)
                    .hint_text("type a command — Enter runs it here, Shift+Enter in a terminal"),
            );
            if std::mem::take(&mut self.cmdline_focus) {
                r.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                    let end = egui::text::CCursor::new(self.cmdline.chars().count());
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                    state.store(ui.ctx(), id);
                }
            }
            if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.cmdline.clear();
                r.surrender_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                run = Some(ui.input(|i| i.modifiers.shift));
            }
        });
        if let Some(in_terminal) = run {
            let line = std::mem::take(&mut self.cmdline);
            if !line.trim().is_empty() {
                let r = if in_terminal { launch::run_in_terminal(&dir, &line) } else { launch::run_shell(&dir, &line) };
                match r {
                    Ok(()) => self.status = format!("Started: {line}"),
                    Err(e) => self.message("Command", format!("{line}\n\n{e}")),
                }
            }
        }
    }

    fn function_keys(&self, ui: &mut egui::Ui, out: &mut Vec<Cmd>) {
        if !self.status.is_empty() {
            ui.label(RichText::new(&self.status).small().weak());
        }
        let keys = [
            ("F3", "View", Cmd::View),
            ("F4", "Edit", Cmd::Edit),
            ("F5", "Copy", Cmd::Copy),
            ("F6", "Move", Cmd::Move),
            ("Shift+F6", "Rename", Cmd::Rename),
            ("F7", "New folder", Cmd::Mkdir),
            ("F8", "Delete", Cmd::Delete { permanent: false }),
            ("Alt+F9", "Extract", Cmd::Extract),
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
}
