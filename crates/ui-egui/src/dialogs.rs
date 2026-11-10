//! Dialogs: copy / move, extract, new folder, rename, delete, messages, bookmarks, settings.
//! Enter confirms, Escape (or a click outside a modal) cancels.

use std::path::{Path, PathBuf};

use commandercraft_core::settings::{Association, Bookmark, Settings, ThemeChoice};
use egui::text::{CCursor, CCursorRange};
use egui::{Id, Key, RichText};

use crate::fonts::SystemFonts;

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
    Transfer {
        mv: bool,
        sources: Sources,
        summary: String,
        dest: String,
        overwrite: bool,
    },
    Extract {
        archives: Vec<PathBuf>,
        summary: String,
        dest: String,
        own_folder: bool,
        overwrite: bool,
    },
    Mkdir {
        dir: PathBuf,
        name: String,
    },
    Rename {
        src: PathBuf,
        name: String,
    },
    Delete {
        paths: Vec<PathBuf>,
        permanent: bool,
        summary: String,
    },
    Message {
        title: String,
        text: String,
    },
    /// The bookmarks manager.
    Bookmarks,
    /// Ctrl+D: a small list of bookmarks at `anchor`, with "Manage bookmarks…" last.
    QuickBookmarks {
        selected: usize,
        anchor: egui::Pos2,
    },
    Settings(Box<Settings>),
}

pub enum Outcome {
    Open,
    Cancel,
    Confirm,
    /// Bookmarks: go to this folder.
    GoTo(PathBuf),
    /// Quick bookmarks: open the manager.
    Manage,
}

pub struct Env<'a> {
    pub bookmarks: &'a mut Vec<Bookmark>,
    pub current: Option<&'a Path>,
    pub fonts: Option<&'a SystemFonts>,
}

/// `fresh` is true on the first frame, to focus the text field.
pub fn show(ctx: &egui::Context, dialog: &mut Dialog, fresh: bool, env: Env<'_>) -> Outcome {
    match dialog {
        Dialog::Bookmarks => return bookmarks_window(ctx, env.bookmarks, env.current),
        Dialog::QuickBookmarks { selected, anchor } => return quick_bookmarks(ctx, env.bookmarks, selected, *anchor),
        _ => {}
    }
    let resp = egui::Modal::new(Id::new("cc_dialog")).show(ctx, |ui| {
        ui.set_min_width(460.0);
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
            Dialog::Extract { summary, dest, own_folder, overwrite, archives } => {
                ui.heading("Extract");
                ui.label(format!("Extract {summary} to:"));
                text_field(ui, dest, fresh, None);
                ui.radio_value(own_folder, false, "Directly into this folder");
                let example = archives.first().map(|a| commandercraft_core::archive::base_name(a)).unwrap_or_default();
                ui.radio_value(
                    own_folder,
                    true,
                    format!("Into a folder named after each archive  ({example}{}…)", std::path::MAIN_SEPARATOR),
                );
                ui.checkbox(overwrite, "Overwrite existing files");
                out = buttons(ui, "Extract", enter);
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
            Dialog::Settings(draft) => out = settings_ui(ui, draft, env.fonts),
            Dialog::Bookmarks | Dialog::QuickBookmarks { .. } => {}
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

/// The quick bookmarks list: ↑ ↓ and Enter, a click, or a digit picks; Esc or a click outside closes.
fn quick_bookmarks(ctx: &egui::Context, bookmarks: &[Bookmark], selected: &mut usize, anchor: egui::Pos2) -> Outcome {
    let manage = bookmarks.len(); // index of the last entry
    let (up, down, enter, escape, digit) = ctx.input(|i| {
        const DIGITS: [Key; 9] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
        (
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::ArrowDown),
            i.key_pressed(Key::Enter),
            i.key_pressed(Key::Escape),
            DIGITS.iter().position(|k| i.key_pressed(*k)),
        )
    });
    if up {
        *selected = selected.checked_sub(1).unwrap_or(manage);
    }
    if down {
        *selected = if *selected >= manage { 0 } else { *selected + 1 };
    }
    *selected = (*selected).min(manage);
    let pick = |i: usize| if i == manage { Outcome::Manage } else { Outcome::GoTo(bookmarks[i].path.clone()) };
    let mut out = Outcome::Open;
    if let Some(d) = digit.filter(|d| *d < bookmarks.len()) {
        out = pick(d);
    } else if enter {
        out = pick(*selected);
    } else if escape {
        out = Outcome::Cancel;
    }

    let area = egui::Area::new(Id::new("cc_quick_bookmarks")).order(egui::Order::Foreground).fixed_pos(anchor).show(ctx, |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            let labels: Vec<String> =
                bookmarks.iter().map(|b| if b.name.trim().is_empty() { b.path.display().to_string() } else { b.name.clone() }).collect();
            // As wide as the longest entry (plus its number), within limits.
            let font = egui::TextStyle::Button.resolve(ui.style());
            let longest = labels
                .iter()
                .map(String::as_str)
                .chain(["Manage bookmarks…"])
                .map(|s| ctx.fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), egui::Color32::WHITE).size().x))
                .fold(0.0, f32::max);
            let width = (longest + 48.0).clamp(180.0, 480.0);
            ui.set_width(width);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            let entry = |text: String, key: String, selected: bool| {
                egui::Button::new(text)
                    .selected(selected)
                    .frame_when_inactive(selected)
                    .right_text(RichText::new(key).weak())
                    .min_size(egui::vec2(width, 0.0))
            };
            for (i, (b, label)) in bookmarks.iter().zip(labels).enumerate() {
                let key = if i < 9 { format!("{}", i + 1) } else { String::new() };
                let r = ui.add(entry(label, key, i == *selected)).on_hover_text(b.path.display().to_string());
                if r.hovered() && ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                    *selected = i;
                }
                if r.clicked() {
                    out = pick(i);
                }
            }
            if bookmarks.is_empty() {
                ui.label(RichText::new("No bookmarks yet").weak());
            }
            ui.separator();
            let r = ui.add(entry("Manage bookmarks…".into(), String::new(), *selected == manage));
            if r.hovered() && ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                *selected = manage;
            }
            if r.clicked() {
                out = Outcome::Manage;
            }
        });
    });
    // A press anywhere else closes the list.
    let press = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
    if matches!(out, Outcome::Open) && press.is_some_and(|p| !area.response.rect.contains(p)) {
        out = Outcome::Cancel;
    }
    out
}

/// The bookmarks manager: rename, reorder, remove, add.
fn bookmarks_window(ctx: &egui::Context, bookmarks: &mut Vec<Bookmark>, current: Option<&Path>) -> Outcome {
    let mut out = Outcome::Open;
    let mut open = true;
    // Sized to its content: nothing in it may depend on the available width, or the window would
    // widen itself a little every frame.
    let screen = ctx.content_rect();
    egui::Window::new("Manage bookmarks")
        .id(Id::new("cc_bookmarks_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .max_width(screen.width() * 0.9)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(screen.center())
        .show(ctx, |ui| {
            ui.label(RichText::new("Click a path to go there, or press its number. Names can be edited.").small());
            ui.add_space(4.0);
            const DIGITS: [Key; 9] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
            let digit = if ui.ctx().text_edit_focused() {
                None
            } else {
                ui.input(|i| DIGITS.iter().position(|k| i.key_pressed(*k)).map(|p| p + 1))
            };
            let mut remove = None;
            let mut swap = None;
            // Column widths from the content: the longest name, and the longest path.
            let font = egui::TextStyle::Body.resolve(ui.style());
            let ctx2 = ui.ctx().clone();
            let text_w = |s: &str| ctx2.fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), egui::Color32::WHITE).size().x);
            let name_w = bookmarks.iter().map(|b| text_w(&b.name)).fold(0.0, f32::max).clamp(140.0, 360.0) + 12.0;
            let path_w = bookmarks.iter().map(|b| text_w(&b.path.display().to_string())).fold(0.0, f32::max).clamp(120.0, 900.0);
            egui::ScrollArea::vertical().max_height(screen.height() * 0.6).auto_shrink([true, true]).show(ui, |ui| {
                // Rows with explicit widths (a Grid would shrink the name fields).
                let n = bookmarks.len();
                let row_h = ui.spacing().interact_size.y;
                for (i, b) in bookmarks.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_sized([18.0, row_h], egui::Label::new(if i < 9 { format!("{}", i + 1) } else { String::new() }));
                        ui.add_sized([name_w, row_h], egui::TextEdit::singleline(&mut b.name).hint_text("name"));
                        let link = ui
                            .allocate_ui_with_layout(egui::vec2(path_w, row_h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.set_width(path_w);
                                let color = ui.visuals().hyperlink_color;
                                ui.add(
                                    egui::Label::new(RichText::new(b.path.display().to_string()).color(color))
                                        .truncate()
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                            })
                            .inner
                            .on_hover_text(b.path.display().to_string());
                        if link.clicked() || digit == Some(i + 1) {
                            out = Outcome::GoTo(b.path.clone());
                        }
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
                }
                if bookmarks.is_empty() {
                    ui.label("No bookmarks yet.");
                }
            });
            if let Some(i) = swap {
                bookmarks.swap(i, i + 1);
            }
            if let Some(i) = remove {
                bookmarks.remove(i);
            }
            ui.separator();
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
        });
    if !open || (matches!(out, Outcome::Open) && ctx.input(|i| i.key_pressed(Key::Escape))) {
        out = Outcome::Cancel;
    }
    out
}

pub fn add_bookmark(bookmarks: &mut Vec<Bookmark>, dir: &Path) {
    if bookmarks.iter().any(|b| b.path == dir) {
        return;
    }
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string());
    bookmarks.push(Bookmark { name, path: dir.to_path_buf() });
}

fn font_combo(ui: &mut egui::Ui, id: &str, value: &mut String, families: &[String]) {
    let shown = if value.is_empty() { "Built-in".to_string() } else { value.clone() };
    egui::ComboBox::from_id_salt(id).selected_text(shown).width(260.0).height(360.0).show_ui(ui, |ui| {
        ui.selectable_value(value, String::new(), "Built-in");
        for f in families {
            ui.selectable_value(value, f.clone(), f);
        }
    });
}

fn settings_ui(ui: &mut egui::Ui, s: &mut Settings, fonts: Option<&SystemFonts>) -> Outcome {
    ui.heading("Settings");
    ui.add_space(4.0);
    egui::ScrollArea::vertical().max_height(ui.ctx().content_rect().height() * 0.7).show(ui, |ui| {
        ui.label(RichText::new("Appearance").strong());
        egui::Grid::new("cc_appearance").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Theme");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut s.appearance.theme, ThemeChoice::System, "System");
                ui.selectable_value(&mut s.appearance.theme, ThemeChoice::Light, "Light");
                ui.selectable_value(&mut s.appearance.theme, ThemeChoice::Dark, "Dark");
            });
            ui.end_row();
            ui.label("Font");
            match fonts {
                Some(f) => font_combo(ui, "cc_font", &mut s.appearance.font, &f.families),
                None => {
                    ui.label(RichText::new("(loading system fonts…)").weak());
                }
            }
            ui.end_row();
            ui.label("Monospace font");
            match fonts {
                Some(f) => font_combo(ui, "cc_mono_font", &mut s.appearance.mono_font, &f.mono_families),
                None => {
                    ui.label("");
                }
            }
            ui.end_row();
            ui.label("Text size");
            ui.add(egui::Slider::new(&mut s.appearance.font_size, 9.0..=24.0).step_by(0.5).suffix(" pt"));
            ui.end_row();
            ui.label("File lists");
            ui.checkbox(&mut s.appearance.list_monospace, "Use the monospace font");
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.label(RichText::new("Programs").strong());
        ui.label("Editor (F4) — empty uses the system default:");
        ui.add(egui::TextEdit::singleline(&mut s.editor).desired_width(f32::INFINITY).hint_text(editor_hint()));
        ui.label("Terminal — empty uses the system default; {dir} is the folder:");
        ui.add(egui::TextEdit::singleline(&mut s.terminal).desired_width(f32::INFINITY).hint_text(terminal_hint()));
        ui.add_space(6.0);
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
        ui.add_space(10.0);
        ui.label(RichText::new("Behaviour").strong());
        ui.checkbox(&mut s.show_hidden, "Show hidden files");
        ui.checkbox(&mut s.delete_permanently, "Delete permanently instead of moving to the trash");
    });
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

fn terminal_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "/Applications/iTerm.app   or   wezterm start --cwd {dir}"
    } else if cfg!(windows) {
        "wt -d {dir}   or   pwsh -NoExit"
    } else {
        "konsole --workdir {dir}"
    }
}
