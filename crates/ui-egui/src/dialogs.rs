//! Modal dialogs: copy / move, new folder, rename, delete, messages, bookmarks, settings.
//! Enter confirms, Escape (or a click outside) cancels.

use std::path::{Path, PathBuf};

use commandercraft_core::settings::{Association, Bookmark, Settings};
use egui::text::{CCursor, CCursorRange};
use egui::{Id, Key, RichText};

/// What a copy / move acts on.
pub enum Sources {
    Local(Vec<PathBuf>),
    /// Paths inside an archive (copy = extract).
    Archive {
        archive: PathBuf,
        items: Vec<String>,
    },
}

pub enum Dialog {
    Transfer { mv: bool, sources: Sources, summary: String, dest: String, overwrite: bool },
    Mkdir { dir: PathBuf, name: String },
    Rename { src: PathBuf, name: String },
    Delete { paths: Vec<PathBuf>, permanent: bool, summary: String },
    Message { title: String, text: String },
    Bookmarks,
    Settings(Box<Settings>),
}

pub enum Outcome {
    Open,
    Cancel,
    Confirm,
    /// Bookmarks: go to this folder.
    GoTo(PathBuf),
}

/// `fresh` is true on the first frame, to focus the text field.
pub fn show(ctx: &egui::Context, dialog: &mut Dialog, fresh: bool, bookmarks: &mut Vec<Bookmark>, current: Option<&Path>) -> Outcome {
    let resp = egui::Modal::new(Id::new("cc_dialog")).show(ctx, |ui| {
        ui.set_min_width(420.0);
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let mut out = Outcome::Open;
        match dialog {
            Dialog::Transfer { mv, summary, dest, overwrite, .. } => {
                ui.heading(if *mv { "Move" } else { "Copy" });
                ui.label(format!("{} {summary} to:", if *mv { "Move" } else { "Copy" }));
                text_field(ui, dest, fresh, None);
                ui.checkbox(overwrite, "Overwrite existing files");
                out = buttons(ui, if *mv { "Move" } else { "Copy" }, enter);
            }
            Dialog::Mkdir { dir, name } => {
                ui.heading("New folder");
                ui.label(format!("Create a folder in {}:", dir.display()));
                text_field(ui, name, fresh, None);
                out = buttons(ui, "Create", enter);
            }
            Dialog::Rename { src, name } => {
                ui.heading("Rename");
                ui.label(format!("New name for {}:", src.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
                // Pre-select the name without extension, so typing replaces just that.
                let stem_len =
                    if src.is_dir() { name.chars().count() } else { commandercraft_core::entry::split_ext(name).0.chars().count() };
                text_field(ui, name, fresh, Some(stem_len));
                out = buttons(ui, "Rename", enter);
            }
            Dialog::Delete { permanent, summary, .. } => {
                ui.heading("Delete");
                if *permanent {
                    ui.label(format!("Permanently delete {summary}?"));
                    ui.label(RichText::new("This cannot be undone.").color(ui.visuals().warn_fg_color));
                } else {
                    ui.label(format!("Move {summary} to the trash?"));
                }
                out = buttons(ui, "Delete", enter);
            }
            Dialog::Message { title, text } => {
                ui.heading(title.as_str());
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| ui.label(text.as_str()));
                ui.add_space(6.0);
                if ui.button("OK").clicked() || enter {
                    out = Outcome::Cancel;
                }
            }
            Dialog::Bookmarks => out = bookmarks_ui(ui, bookmarks, current),
            Dialog::Settings(draft) => out = settings_ui(ui, draft),
        }
        out
    });
    if resp.should_close() && matches!(resp.inner, Outcome::Open) { Outcome::Cancel } else { resp.inner }
}

fn text_field(ui: &mut egui::Ui, text: &mut String, fresh: bool, select_chars: Option<usize>) {
    let id = Id::new("cc_dialog_text");
    let r = ui.add(egui::TextEdit::singleline(text).id(id).desired_width(f32::INFINITY));
    if fresh {
        r.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            let end = select_chars.unwrap_or(text.chars().count());
            state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(end))));
            state.store(ui.ctx(), id);
        }
    }
}

fn buttons(ui: &mut egui::Ui, ok: &str, enter: bool) -> Outcome {
    ui.add_space(6.0);
    let mut out = Outcome::Open;
    ui.horizontal(|ui| {
        if ui.button(ok).clicked() || enter {
            out = Outcome::Confirm;
        }
        if ui.button("Cancel").clicked() {
            out = Outcome::Cancel;
        }
    });
    out
}

fn bookmarks_ui(ui: &mut egui::Ui, bookmarks: &mut Vec<Bookmark>, current: Option<&Path>) -> Outcome {
    let mut out = Outcome::Open;
    ui.heading("Bookmarks");
    ui.label(RichText::new("Click a name to go there, or press its number.").small());
    const DIGITS: [Key; 9] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    let digit =
        if ui.ctx().text_edit_focused() { None } else { ui.input(|i| DIGITS.iter().position(|k| i.key_pressed(*k)).map(|p| p + 1)) };
    let mut remove = None;
    let mut swap = None;
    egui::Grid::new("cc_bookmarks").num_columns(4).striped(true).show(ui, |ui| {
        let n = bookmarks.len();
        for (i, b) in bookmarks.iter_mut().enumerate() {
            ui.label(if i < 9 { format!("{}", i + 1) } else { String::new() });
            if ui.link(b.path.display().to_string()).clicked() || digit == Some(i + 1) {
                out = Outcome::GoTo(b.path.clone());
            }
            ui.add(egui::TextEdit::singleline(&mut b.name).desired_width(140.0).hint_text("name"));
            ui.horizontal(|ui| {
                if ui.add_enabled(i > 0, egui::Button::new("⏶").small()).clicked() {
                    swap = Some(i - 1);
                }
                if ui.add_enabled(i + 1 < n, egui::Button::new("⏷").small()).clicked() {
                    swap = Some(i);
                }
                if ui.small_button("✖").on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
            });
            ui.end_row();
        }
    });
    if let Some(i) = swap {
        bookmarks.swap(i, i + 1);
    }
    if let Some(i) = remove {
        bookmarks.remove(i);
    }
    if bookmarks.is_empty() {
        ui.label("No bookmarks yet.");
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if let Some(dir) = current
            && ui.button("Add current folder").clicked()
        {
            add_bookmark(bookmarks, dir);
        }
        if ui.button("Close").clicked() {
            out = Outcome::Cancel;
        }
    });
    out
}

pub fn add_bookmark(bookmarks: &mut Vec<Bookmark>, dir: &Path) {
    if bookmarks.iter().any(|b| b.path == dir) {
        return;
    }
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string());
    bookmarks.push(Bookmark { name, path: dir.to_path_buf() });
}

fn settings_ui(ui: &mut egui::Ui, s: &mut Settings) -> Outcome {
    ui.heading("Settings");
    ui.add_space(4.0);
    ui.label("Editor (F4) — empty uses the system default:");
    ui.add(egui::TextEdit::singleline(&mut s.editor).desired_width(f32::INFINITY).hint_text(editor_hint()));
    ui.add_space(8.0);
    ui.label("Open files with (Enter / double-click), by extension:");
    ui.label(RichText::new("Put {file} where the path goes; otherwise it is appended.").small());
    let mut remove = None;
    egui::Grid::new("cc_assoc").num_columns(3).show(ui, |ui| {
        ui.label(RichText::new("Extensions").small());
        ui.label(RichText::new("Command").small());
        ui.end_row();
        for (i, a) in s.associations.iter_mut().enumerate() {
            ui.add(egui::TextEdit::singleline(&mut a.extensions).desired_width(110.0).hint_text("txt, md"));
            ui.add(egui::TextEdit::singleline(&mut a.command).desired_width(320.0).hint_text(editor_hint()));
            if ui.small_button("✖").clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        s.associations.remove(i);
    }
    if ui.button("Add").clicked() {
        s.associations.push(Association::default());
    }
    ui.add_space(8.0);
    ui.checkbox(&mut s.show_hidden, "Show hidden files");
    ui.checkbox(&mut s.delete_permanently, "Delete permanently instead of moving to the trash");
    ui.add_space(6.0);
    let mut out = Outcome::Open;
    ui.horizontal(|ui| {
        if ui.button("Save").clicked() {
            out = Outcome::Confirm;
        }
        if ui.button("Cancel").clicked() {
            out = Outcome::Cancel;
        }
    });
    out
}

fn editor_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "/Applications/TextEdit.app   or   code -n"
    } else if cfg!(windows) {
        r#""C:\Program Files\Notepad++\notepad++.exe"   or   code -n"#
    } else {
        "gedit   or   code -n"
    }
}
