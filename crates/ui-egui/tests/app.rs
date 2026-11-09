//! Headless end-to-end tests: the real app, driven by keys, on a temporary folder tree.
//!
//! Set `CC_SNAPSHOT_DIR` to also save screenshots of each step (needs a GPU adapter).

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use commandercraft_core::settings::{PanelSession, Settings, TabSession};
use commandercraft_ui_egui::CommanderApp;
use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;

fn fixture(root: &Path) {
    let left = root.join("left");
    fs::create_dir_all(left.join("sub/deeper")).unwrap();
    fs::create_dir_all(root.join("right")).unwrap();
    fs::write(left.join("a.txt"), b"alpha").unwrap();
    fs::write(left.join("b.md"), b"bravo bravo").unwrap();
    fs::write(left.join("sub/inside.txt"), b"in sub").unwrap();
    let mut z = zip::ZipWriter::new(File::create(left.join("pack.zip")).unwrap());
    let o = zip::write::SimpleFileOptions::default();
    z.start_file("docs/readme.txt", o).unwrap();
    z.write_all(b"zipped readme").unwrap();
    z.start_file("top.txt", o).unwrap();
    z.write_all(b"top").unwrap();
    z.finish().unwrap();
}

fn harness(root: &Path) -> Harness<'static, CommanderApp> {
    let tab = |p: &Path| PanelSession { tabs: vec![TabSession { path: p.to_path_buf(), sort: Default::default() }], active: 0 };
    let settings = Settings { left: tab(&root.join("left")), right: tab(&root.join("right")), ..Default::default() };
    Harness::builder()
        .with_size(vec2(1100.0, 600.0))
        .build_ui_state(|ui, app: &mut CommanderApp| app.show(ui), CommanderApp::new(settings).without_system_services())
}

fn key(h: &mut Harness<'_, CommanderApp>, key: Key) {
    key_mod(h, key, Modifiers::NONE);
}
fn key_mod(h: &mut Harness<'_, CommanderApp>, key: Key, m: Modifiers) {
    h.state_mut().feed_key(key, m);
    h.run_steps(3);
}
fn names(h: &Harness<'_, CommanderApp>, side: usize) -> Vec<String> {
    h.state().panels()[side].tab().entries.iter().map(|e| e.name.clone()).collect()
}
fn cursor_name(h: &Harness<'_, CommanderApp>) -> String {
    let s = h.state();
    s.panels()[s.active_side()].tab().current().map(|e| e.name.clone()).unwrap_or_default()
}
fn select(h: &mut Harness<'_, CommanderApp>, name: &str) {
    let s = h.state();
    let i = s.panels()[s.active_side()].tab().index_of(name).unwrap_or_else(|| panic!("{name} not listed"));
    let side = s.active_side();
    let tab = &s.panels()[side].tab();
    let delta = i as isize - tab.cursor as isize;
    for _ in 0..delta.unsigned_abs() {
        key(h, if delta > 0 { Key::ArrowDown } else { Key::ArrowUp });
    }
    assert_eq!(cursor_name(h), name);
}
fn wait_idle(h: &mut Harness<'_, CommanderApp>) {
    let start = Instant::now();
    while h.state().busy() {
        assert!(start.elapsed() < Duration::from_secs(10), "job did not finish");
        std::thread::sleep(Duration::from_millis(20));
        h.step();
    }
    h.run_steps(2);
}
fn snapshot(h: &mut Harness<'_, CommanderApp>, name: &str) {
    if let Ok(dir) = std::env::var("CC_SNAPSHOT_DIR") {
        h.run_steps(2);
        match h.render() {
            Ok(img) => img.save(Path::new(&dir).join(format!("{name}.png"))).unwrap(),
            Err(e) => eprintln!("no snapshot: {e}"),
        }
    }
}

#[test]
fn browse_copy_archive_tabs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fixture(root);
    let mut h = harness(root);
    h.run_steps(3);

    // Listing: `..`, folders, then files.
    assert_eq!(names(&h, 0), ["..", "sub", "a.txt", "b.md", "pack.zip"]);
    assert_eq!(h.state().active_side(), 0);
    snapshot(&mut h, "01-start");

    // Mark two files with Space, copy them to the right panel with F5 + Enter.
    select(&mut h, "a.txt");
    key(&mut h, Key::Space);
    key(&mut h, Key::Space);
    assert_eq!(h.state().panels()[0].tab().marked.len(), 2);
    snapshot(&mut h, "02-marked");
    key(&mut h, Key::F5);
    assert!(h.state().has_dialog());
    snapshot(&mut h, "03-copy-dialog");
    h.key_press(Key::Enter);
    h.run_steps(3);
    wait_idle(&mut h);
    assert!(!h.state().has_dialog(), "copy should finish without a message");
    assert_eq!(fs::read(root.join("right/a.txt")).unwrap(), b"alpha");
    assert_eq!(fs::read(root.join("right/b.md")).unwrap(), b"bravo bravo");
    assert!(h.state().panels()[0].tab().marked.is_empty(), "marks are cleared after a copy");
    assert_eq!(names(&h, 1), ["..", "a.txt", "b.md"]);

    // Enter the archive, then its folder; copy (extract) a file to the right.
    select(&mut h, "pack.zip");
    key(&mut h, Key::Enter);
    assert_eq!(names(&h, 0), ["..", "docs", "top.txt"]);
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::Enter);
    assert_eq!(names(&h, 0), ["..", "readme.txt"]);
    snapshot(&mut h, "04-inside-zip");
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::F5);
    h.key_press(Key::Enter);
    h.run_steps(3);
    wait_idle(&mut h);
    assert_eq!(fs::read_to_string(root.join("right/readme.txt")).unwrap(), "zipped readme");

    // Moving out of an archive is refused (read-only).
    key(&mut h, Key::F6);
    assert!(h.state().has_dialog());
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(!h.state().has_dialog());

    // Backspace twice: back to the folder holding the archive, cursor on it.
    key(&mut h, Key::Backspace);
    key(&mut h, Key::Backspace);
    assert_eq!(cursor_name(&h), "pack.zip");

    // Ctrl+→ on a folder shows it in the right panel; focus stays left.
    select(&mut h, "sub");
    key_mod(&mut h, Key::ArrowRight, Modifiers::COMMAND);
    assert_eq!(names(&h, 1), ["..", "deeper", "inside.txt"]);
    assert_eq!(h.state().active_side(), 0);

    // Ctrl+↑ opens the folder under the cursor in a new tab.
    key_mod(&mut h, Key::ArrowUp, Modifiers::COMMAND);
    assert_eq!(h.state().panels()[0].tabs.len(), 2);
    assert_eq!(names(&h, 0), ["..", "deeper", "inside.txt"]);
    snapshot(&mut h, "05-tabs");
    key_mod(&mut h, Key::W, Modifiers::COMMAND);
    assert_eq!(h.state().panels()[0].tabs.len(), 1);

    // Sorting: Ctrl+F6 = by size (largest first).
    key_mod(&mut h, Key::F6, Modifiers::COMMAND);
    let files: Vec<String> = names(&h, 0).into_iter().skip(2).collect();
    assert_eq!(files[0], "pack.zip");

    // Tab switches panels.
    key(&mut h, Key::Tab);
    assert_eq!(h.state().active_side(), 1);
}

#[test]
fn mkdir_rename_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fixture(root);
    let mut h = harness(root);
    h.run_steps(3);

    // F7, type a name, Enter.
    key(&mut h, Key::F7);
    h.event(egui::Event::Text("fresh".into()));
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(root.join("left/fresh").is_dir());
    assert_eq!(cursor_name(&h), "fresh");

    // Shift+F6 renames; the stem is pre-selected, so typing replaces it and keeps the extension.
    select(&mut h, "a.txt");
    key_mod(&mut h, Key::F6, Modifiers::SHIFT);
    h.run_steps(2);
    h.event(egui::Event::Text("renamed".into()));
    h.run_steps(2);
    snapshot(&mut h, "06-rename");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(root.join("left/renamed.txt").is_file(), "{:?}", names(&h, 0));

    // Shift+F8 deletes permanently (no trash in a temp folder test).
    select(&mut h, "fresh");
    key_mod(&mut h, Key::F8, Modifiers::SHIFT);
    snapshot(&mut h, "07-delete");
    h.key_press(Key::Enter);
    h.run_steps(3);
    wait_idle(&mut h);
    assert!(!root.join("left/fresh").exists());
}

#[test]
fn viewer_quick_view_and_bookmarks() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fixture(root);
    fs::write(root.join("left/notes.txt"), "first line\nsecond line\nzażółć gęślą jaźń\n").unwrap();
    let mut h = harness(root);
    h.state_mut().settings_mut().appearance.theme = commandercraft_core::settings::ThemeChoice::Dark;
    h.run_steps(3);

    // F3 opens the internal viewer; Esc closes it.
    select(&mut h, "notes.txt");
    key(&mut h, Key::F3);
    assert!(h.state().viewer_open());
    snapshot(&mut h, "10-viewer-dark");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!h.state().viewer_open());

    // Ctrl+Q (the Control key on every platform) toggles quick view in the other panel.
    key_mod(&mut h, Key::Q, Modifiers::CTRL);
    assert!(h.state().quick_view_open());
    snapshot(&mut h, "11-quick-view-dark");
    // Tab does not move the focus into the preview.
    key(&mut h, Key::Tab);
    assert_eq!(h.state().active_side(), 0);
    key_mod(&mut h, Key::Q, Modifiers::CTRL);
    assert!(!h.state().quick_view_open());

    for (name, dir) in [("Projects", "left"), ("Downloads and other long names", "right")] {
        let path = root.join(dir);
        h.state_mut().settings_mut().bookmarks.push(commandercraft_core::settings::Bookmark { name: name.into(), path });
    }
    // Ctrl+D (Control, also on macOS) opens the bookmarks window.
    key_mod(&mut h, Key::D, Modifiers::CTRL);
    assert!(h.state().has_dialog());
    snapshot(&mut h, "12-bookmarks");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!h.state().has_dialog());
}

#[test]
fn rename_uses_the_active_right_panel() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fixture(root);
    fs::write(root.join("right/r.txt"), b"right side").unwrap();
    let mut h = harness(root);
    h.run_steps(3);
    key(&mut h, Key::Tab);
    assert_eq!(h.state().active_side(), 1);
    select(&mut h, "r.txt");
    key_mod(&mut h, Key::F6, Modifiers::SHIFT);
    h.event(egui::Event::Text("moved".into()));
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(root.join("right/moved.txt").is_file());
    assert!(root.join("left/a.txt").is_file(), "the left panel must be untouched");
    assert_eq!(h.state().active_side(), 1);
}

#[test]
fn extract_archives_into_own_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fixture(root);
    fs::copy(root.join("left/pack.zip"), root.join("left/second.zip")).unwrap();
    let mut h = harness(root);
    h.state_mut().settings_mut().appearance.theme = commandercraft_core::settings::ThemeChoice::Light;
    h.run_steps(3);
    select(&mut h, "pack.zip");
    key(&mut h, Key::Space);
    key(&mut h, Key::Space);
    key_mod(&mut h, Key::F9, Modifiers::ALT);
    assert!(h.state().has_dialog());
    snapshot(&mut h, "13-extract-light");
    h.key_press(Key::Enter);
    h.run_steps(3);
    wait_idle(&mut h);
    assert_eq!(fs::read_to_string(root.join("right/pack/docs/readme.txt")).unwrap(), "zipped readme");
    assert_eq!(fs::read_to_string(root.join("right/second/top.txt")).unwrap(), "top");
    assert_eq!(names(&h, 1), ["..", "pack", "second"]);
    snapshot(&mut h, "14-after-extract-light");
}

#[test]
fn menu_click_does_not_steal_panel_focus_and_hover_survives_keys() {
    use egui_kittest::kittest::Queryable;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fixture(root);
    fs::write(root.join("right/r.txt"), b"right side").unwrap();
    let mut h = harness(root);
    h.run_steps(3);
    key(&mut h, Key::Tab);
    select(&mut h, "r.txt");

    // Files ▸ Rename: the menu item lies over the left panel; the right panel must stay active.
    h.get_by_label("Files").click();
    h.run_steps(3);
    let items: Vec<_> = h.get_all_by_label_contains("Rename").collect();
    let menu_item = items.iter().min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).expect("menu item");
    assert!(menu_item.rect().center().x < 550.0, "the menu item should lie over the left panel");
    menu_item.click();
    h.run_steps(3);
    assert_eq!(h.state().active_side(), 1, "clicking a menu item over the left panel activated it");
    assert!(h.state().has_dialog());
    h.key_press(Key::Escape);
    h.run_steps(3);

    // Hover a row in the left panel, then move the cursor by keyboard: the hover stays painted.
    h.event(egui::Event::PointerMoved(egui::pos2(120.0, 183.0)));
    h.run_steps(2);
    key(&mut h, Key::ArrowUp);
    snapshot(&mut h, "15-hover-after-keys");
}

/// Needs system CJK fonts (and a GPU for the screenshot): `cargo test -- --ignored cjk`.
#[test]
#[ignore]
fn cjk_file_names_render_with_system_fonts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fixture(root);
    for name in ["日本語のファイル.txt", "中文文件.txt", "한국어.txt"] {
        fs::write(root.join("left").join(name), b"x").unwrap();
    }
    let tab = |p: &Path| PanelSession { tabs: vec![TabSession { path: p.to_path_buf(), sort: Default::default() }], active: 0 };
    let settings = Settings { left: tab(&root.join("left")), right: tab(&root.join("right")), ..Default::default() };
    let mut h = Harness::builder()
        .with_size(vec2(1100.0, 600.0))
        .build_ui_state(|ui, app: &mut CommanderApp| app.show(ui), CommanderApp::new(settings));
    // The font scan runs in the background.
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(8) {
        h.step();
        std::thread::sleep(Duration::from_millis(50));
    }
    h.run_steps(3);
    snapshot(&mut h, "16-cjk");
}

#[test]
fn ctrl_page_down_opens_any_archive_and_rows_paint_fully() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fixture(root);
    fs::copy(root.join("left/pack.zip"), root.join("left/book.epub")).unwrap();
    let mut h = harness(root);
    h.state_mut().settings_mut().appearance.theme = commandercraft_core::settings::ThemeChoice::Dark;
    h.run_steps(3);

    // Mouse over a row, then keys: the hover highlight goes away until the mouse moves again.
    h.event(egui::Event::PointerMoved(egui::pos2(120.0, 240.0)));
    h.run_steps(2);
    // The cursor on an odd (striped) row: the highlight must cover all four columns.
    select(&mut h, "b.md");
    snapshot(&mut h, "17-odd-row-dark-no-hover");

    // Enter on an .epub opens it with an app; Ctrl+PageDown goes into it as a zip.
    select(&mut h, "book.epub");
    key_mod(&mut h, Key::PageDown, Modifiers::COMMAND);
    assert_eq!(names(&h, 0), ["..", "docs", "top.txt"]);
    key_mod(&mut h, Key::PageUp, Modifiers::COMMAND);
    assert_eq!(cursor_name(&h), "book.epub");

    // A file that is no archive: a message, and the panel stays.
    select(&mut h, "a.txt");
    key_mod(&mut h, Key::PageDown, Modifiers::COMMAND);
    assert!(h.state().has_dialog());
    assert_eq!(cursor_name(&h), "a.txt");
}
