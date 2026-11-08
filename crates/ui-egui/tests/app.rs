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
    Harness::builder().with_size(vec2(1100.0, 600.0)).build_ui_state(|ui, app: &mut CommanderApp| app.show(ui), CommanderApp::new(settings))
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
