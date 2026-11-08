//! One side of the window: its tabs, and drawing the tab strip, path bar, file table and status line.

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::{DateTime, Local};
use commandercraft_core::settings::Bookmark;
use commandercraft_core::sort::{Sort, SortColumn, sort_entries};
use commandercraft_core::volumes::{self, Volume};
use commandercraft_core::{Entry, Location, archive, format_bytes_grouped, format_size, vfs};
use egui::{Align, Color32, Layout, RichText, Sense};
use egui_extras::{Column, TableBuilder};

/// A folder shown in a panel, with cursor, marked entries and sort order.
pub struct Tab {
    pub location: Location,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    /// Names of marked (selected) entries.
    pub marked: HashSet<String>,
    pub sort: Sort,
    /// Scroll the cursor row into view on the next frame.
    pub scroll_to_cursor: bool,
}

impl Tab {
    pub fn open(location: Location, sort: Sort, show_hidden: bool) -> Result<Tab, String> {
        let entries = listing(&location, sort, show_hidden)?;
        Ok(Tab { location, entries, cursor: 0, marked: HashSet::new(), sort, scroll_to_cursor: true })
    }

    /// Open `location`, or the nearest parent that can be listed, or the home folder.
    pub fn open_or_nearest(location: Location, sort: Sort, show_hidden: bool) -> Tab {
        let mut loc = Some(location);
        while let Some(l) = loc {
            if let Ok(t) = Tab::open(l.clone(), sort, show_hidden) {
                return t;
            }
            loc = l.parent().map(|(p, _)| p);
        }
        let home = volumes::home().unwrap_or_else(|| PathBuf::from("/"));
        Tab::open(Location::Local(home.clone()), sort, show_hidden).unwrap_or(Tab {
            location: Location::Local(home),
            entries: vec![],
            cursor: 0,
            marked: HashSet::new(),
            sort,
            scroll_to_cursor: true,
        })
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| !e.is_parent && e.name == name)
    }

    /// Go to another folder, putting the cursor on `focus` if it is there.
    pub fn navigate(&mut self, location: Location, focus: Option<&str>, show_hidden: bool) -> Result<(), String> {
        let entries = listing(&location, self.sort, show_hidden)?;
        self.location = location;
        self.entries = entries;
        self.marked.clear();
        self.cursor = focus.and_then(|n| self.index_of(n)).unwrap_or(0);
        self.scroll_to_cursor = true;
        Ok(())
    }

    /// Re-read the folder, keeping the cursor on the same name and the marks that still exist.
    /// If the folder is gone, go to the nearest parent that exists.
    pub fn reload(&mut self, show_hidden: bool) {
        let name = self.current().map(|e| e.name.clone());
        match listing(&self.location, self.sort, show_hidden) {
            Ok(entries) => {
                self.entries = entries;
                self.marked.retain(|m| self.entries.iter().any(|e| &e.name == m));
                self.cursor = name.and_then(|n| self.index_of(&n)).unwrap_or(self.cursor).min(self.entries.len().saturating_sub(1));
            }
            Err(_) => *self = Tab::open_or_nearest(self.location.clone(), self.sort, show_hidden),
        }
    }

    pub fn set_sort(&mut self, sort: Sort) {
        let name = self.current().map(|e| e.name.clone());
        self.sort = sort;
        sort_entries(&mut self.entries, sort);
        self.cursor = name.and_then(|n| self.index_of(&n)).unwrap_or(0);
        self.scroll_to_cursor = true;
    }

    pub fn set_cursor(&mut self, i: usize) {
        self.cursor = i.min(self.entries.len().saturating_sub(1));
        self.scroll_to_cursor = true;
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let i = (self.cursor as isize + delta).clamp(0, self.entries.len().saturating_sub(1) as isize);
        self.set_cursor(i as usize);
    }

    pub fn toggle_mark(&mut self, i: usize) {
        if let Some(e) = self.entries.get(i).filter(|e| !e.is_parent)
            && !self.marked.remove(&e.name)
        {
            self.marked.insert(e.name.clone());
        }
    }

    pub fn mark_range(&mut self, a: usize, b: usize) {
        let (lo, hi) = (a.min(b), a.max(b));
        for e in self.entries.iter().take(hi + 1).skip(lo).filter(|e| !e.is_parent) {
            self.marked.insert(e.name.clone());
        }
    }

    pub fn mark_all(&mut self, files_only: bool) {
        self.marked = self.entries.iter().filter(|e| !e.is_parent && !(files_only && e.is_dir)).map(|e| e.name.clone()).collect();
    }

    pub fn invert_marks(&mut self) {
        self.marked = self.entries.iter().filter(|e| !e.is_parent && !self.marked.contains(&e.name)).map(|e| e.name.clone()).collect();
    }

    /// What an operation acts on: the marked entries, or else the one under the cursor.
    pub fn targets(&self) -> Vec<Entry> {
        if self.marked.is_empty() {
            self.current().filter(|e| !e.is_parent).cloned().into_iter().collect()
        } else {
            self.entries.iter().filter(|e| self.marked.contains(&e.name)).cloned().collect()
        }
    }

    /// Local paths of [`Tab::targets`] (empty inside archives).
    pub fn target_paths(&self) -> Vec<PathBuf> {
        match self.location.local_path() {
            Some(dir) => self.targets().iter().map(|e| dir.join(&e.name)).collect(),
            None => vec![],
        }
    }

    pub fn title(&self) -> String {
        self.location.title()
    }
}

fn listing(location: &Location, sort: Sort, show_hidden: bool) -> Result<Vec<Entry>, String> {
    let mut v = vfs::list(location, show_hidden).map_err(|e| format!("{}: {e}", location.display()))?;
    if location.parent().is_some() {
        v.push(Entry::parent_link());
    }
    sort_entries(&mut v, sort);
    Ok(v)
}

/// The tabs of one side.
pub struct Panel {
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// The path bar is being edited (text so far).
    pub path_edit: Option<String>,
    path_edit_fresh: bool,
    /// Rows that fit in the table, for Page Up / Page Down.
    pub page_rows: usize,
}

impl Panel {
    pub fn new(tabs: Vec<Tab>, active: usize) -> Panel {
        let active = active.min(tabs.len().saturating_sub(1));
        Panel { tabs, active, path_edit: None, path_edit_fresh: false, page_rows: 20 }
    }
    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }
    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
}

/// What the user asked for in a panel; carried out by the app (it may involve the other panel).
pub enum Action {
    Activate,
    /// Enter / double-click on the entry at this index.
    Open(usize),
    /// Middle-click on a folder: open it in a new background tab.
    OpenInNewTab(usize),
    GoTo(Location),
    GoUp,
    SelectTab(usize),
    CloseTab(usize),
    NewTab,
    Sort(SortColumn),
    AddBookmark,
    ManageBookmarks,
}

pub struct Context<'a> {
    pub active: bool,
    pub bookmarks: &'a [Bookmark],
    pub volumes: &'a [Volume],
    pub id: &'a str,
}

const ROW_H: f32 = 18.0;
const MARKED: Color32 = Color32::from_rgb(230, 70, 60);

pub fn show(ui: &mut egui::Ui, panel: &mut Panel, cx: &Context<'_>) -> Vec<Action> {
    let mut actions = Vec::new();
    let accent = if cx.active { ui.visuals().selection.stroke.color } else { ui.visuals().widgets.noninteractive.bg_stroke.color };
    let frame =
        egui::Frame::new().stroke(egui::Stroke::new(if cx.active { 1.5 } else { 1.0 }, accent)).inner_margin(3.0).corner_radius(3.0);
    let resp = frame.show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        tab_strip(ui, panel, &mut actions);
        path_bar(ui, panel, cx, &mut actions);
        ui.add_space(2.0);
        let status_h = 18.0;
        let header_h = 20.0 + 3.0 * ui.spacing().item_spacing.y;
        let table_h = (ui.available_height() - status_h - header_h - 6.0).max(ROW_H * 3.0);
        panel.page_rows = ((table_h - ROW_H) / (ROW_H + ui.spacing().item_spacing.y)).max(1.0) as usize;
        ui.push_id(cx.id, |ui| file_table(ui, panel.tab_mut(), cx, table_h, &mut actions));
        ui.add_space(2.0);
        status_line(ui, panel.tab());
    });
    let rect = resp.response.rect;
    if ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| rect.contains(p))) {
        actions.insert(0, Action::Activate);
    }
    actions
}

fn tab_strip(ui: &mut egui::Ui, panel: &Panel, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, tab) in panel.tabs.iter().enumerate() {
            let mut title = tab.title();
            if title.chars().count() > 24 {
                title = title.chars().take(22).collect::<String>() + "…";
            }
            if matches!(tab.location, Location::Archive { .. }) {
                title = format!("📦 {title}");
            }
            let r = ui.selectable_label(i == panel.active, title).on_hover_text(tab.location.display());
            if r.clicked() {
                actions.push(Action::SelectTab(i));
            }
            if r.middle_clicked() {
                actions.push(Action::CloseTab(i));
            }
            r.context_menu(|ui| {
                if ui.button("Close tab").clicked() {
                    actions.push(Action::CloseTab(i));
                    ui.close();
                }
            });
        }
        if ui.small_button("+").on_hover_text("New tab").clicked() {
            actions.push(Action::NewTab);
        }
    });
}

fn path_bar(ui: &mut egui::Ui, panel: &mut Panel, cx: &Context<'_>, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let loc = panel.tab().location.clone();
        let vol_label = match &loc {
            Location::Local(p) => volumes::volume_label(p),
            Location::Archive { archive, .. } => volumes::volume_label(archive),
        };
        ui.menu_button(vol_label, |ui| {
            for v in cx.volumes {
                if ui.button(format!("{}   {}", v.label, v.path.display())).clicked() {
                    actions.push(Action::GoTo(Location::Local(v.path.clone())));
                }
            }
        })
        .response
        .on_hover_text("Drives and volumes");
        ui.menu_button("★", |ui| {
            if ui.button("Add current folder").clicked() {
                actions.push(Action::AddBookmark);
            }
            if ui.button("Manage bookmarks…").clicked() {
                actions.push(Action::ManageBookmarks);
            }
            if !cx.bookmarks.is_empty() {
                ui.separator();
            }
            for b in cx.bookmarks {
                if ui.button(&b.name).on_hover_text(b.path.display().to_string()).clicked() {
                    actions.push(Action::GoTo(Location::Local(b.path.clone())));
                }
            }
        })
        .response
        .on_hover_text("Bookmarks");
        if ui.small_button("..").on_hover_text("Parent folder").clicked() {
            actions.push(Action::GoUp);
        }
        match &mut panel.path_edit {
            Some(text) => {
                let r = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                if panel.path_edit_fresh {
                    r.request_focus();
                    panel.path_edit_fresh = false;
                }
                if r.lost_focus() {
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        actions.push(Action::GoTo(parse_location(text)));
                    }
                    panel.path_edit = None;
                }
            }
            None => {
                let r = ui
                    .add(egui::Label::new(RichText::new(loc.display()).strong()).truncate().sense(Sense::click()))
                    .on_hover_text("Click to type a path");
                if r.clicked() {
                    panel.path_edit = Some(loc.display());
                    panel.path_edit_fresh = true;
                }
            }
        }
    });
}

/// A typed path: a folder, or an archive file (opened at its root).
fn parse_location(text: &str) -> Location {
    let path = PathBuf::from(text.trim());
    if path.is_file() && archive::detect(&path).is_some() {
        Location::Archive { archive: path, inner: String::new() }
    } else {
        Location::Local(path)
    }
}

fn file_table(ui: &mut egui::Ui, tab: &mut Tab, cx: &Context<'_>, height: f32, actions: &mut Vec<Action>) {
    ui.style_mut().interaction.selectable_labels = false;
    let ctx = ui.ctx().clone();
    let weak = ui.visuals().weak_text_color();
    let cursor_stroke = egui::Stroke::new(1.0, ui.visuals().selection.stroke.color);
    let mut builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .auto_shrink(false)
        .min_scrolled_height(height)
        .max_scroll_height(height)
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::remainder().at_least(120.0).clip(true))
        .column(Column::initial(48.0).at_least(24.0).clip(true))
        .column(Column::initial(84.0).at_least(40.0).clip(true))
        .column(Column::initial(118.0).at_least(40.0).clip(true));
    if std::mem::take(&mut tab.scroll_to_cursor) {
        builder = builder.scroll_to_row(tab.cursor, None);
    }
    let sort = tab.sort;
    builder
        .header(20.0, |mut header| {
            for (column, label) in
                [(SortColumn::Name, "Name"), (SortColumn::Ext, "Ext"), (SortColumn::Size, "Size"), (SortColumn::Modified, "Date")]
            {
                header.col(|ui| {
                    let arrow = if sort.column == column { if sort.descending { " ⏷" } else { " ⏶" } } else { "" };
                    if ui.add(egui::Button::new(RichText::new(format!("{label}{arrow}")).strong()).frame(false)).clicked() {
                        actions.push(Action::Sort(column));
                    }
                });
            }
        })
        .body(|body| {
            body.rows(ROW_H, tab.entries.len(), |mut row| {
                let i = row.index();
                let e = &tab.entries[i];
                let is_cursor = i == tab.cursor;
                row.set_selected(is_cursor && cx.active);
                let marked = tab.marked.contains(&e.name);
                let color = if marked {
                    Some(MARKED)
                } else if e.hidden {
                    Some(weak)
                } else {
                    None
                };
                let text = |s: String| {
                    let t = RichText::new(s);
                    let t = if let Some(c) = color { t.color(c) } else { t };
                    if marked { t.strong() } else { t }
                };
                // The inactive panel shows its cursor as an outline.
                let outline = |ui: &mut egui::Ui| {
                    if is_cursor && !cx.active {
                        let r = ui.max_rect().shrink2(egui::vec2(0.0, 1.0));
                        ui.painter().hline(r.x_range(), r.top(), cursor_stroke);
                        ui.painter().hline(r.x_range(), r.bottom(), cursor_stroke);
                    }
                };
                let (stem, ext) = e.stem_ext();
                row.col(|ui| {
                    outline(ui);
                    let icon = if e.is_parent {
                        "⮪"
                    } else if e.is_dir {
                        "🗀"
                    } else if archive::detect(std::path::Path::new(&e.name)).is_some() {
                        "📦"
                    } else {
                        "🗋"
                    };
                    let name = if ext.is_empty() { e.name.as_str() } else { stem };
                    ui.label(text(format!("{icon} {name}")));
                });
                row.col(|ui| {
                    outline(ui);
                    ui.label(text(ext.to_string()));
                });
                row.col(|ui| {
                    outline(ui);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let s = if e.is_parent {
                            String::new()
                        } else if e.is_dir {
                            "<DIR>".into()
                        } else {
                            format_bytes_grouped(e.size)
                        };
                        ui.label(text(s));
                    });
                });
                row.col(|ui| {
                    outline(ui);
                    let s = e.modified.map(|t| DateTime::<Local>::from(t).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
                    ui.label(text(s));
                });

                let real_dir = e.is_dir && !e.is_parent;
                let r = row.response();
                if r.clicked() {
                    let mods = ctx.input(|i| i.modifiers);
                    if mods.command {
                        tab.toggle_mark(i);
                        tab.cursor = i;
                    } else if mods.shift {
                        tab.mark_range(tab.cursor, i);
                        tab.cursor = i;
                    } else {
                        tab.cursor = i;
                    }
                }
                if r.secondary_clicked() {
                    tab.toggle_mark(i);
                    tab.cursor = i;
                }
                if r.double_clicked() {
                    actions.push(Action::Open(i));
                }
                if r.middle_clicked() && real_dir {
                    actions.push(Action::OpenInNewTab(i));
                }
            });
        });
}

fn status_line(ui: &mut egui::Ui, tab: &Tab) {
    let files = tab.entries.iter().filter(|e| !e.is_parent);
    let (count, bytes) = files.clone().fold((0usize, 0u64), |(n, b), e| (n + 1, b + e.size));
    let (mcount, mbytes) = files.filter(|e| tab.marked.contains(&e.name)).fold((0usize, 0u64), |(n, b), e| (n + 1, b + e.size));
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} of {} marked  ·  {} of {}", mcount, count, format_size(mbytes), format_size(bytes))).small());
    });
}
