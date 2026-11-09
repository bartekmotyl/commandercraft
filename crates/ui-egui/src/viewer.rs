//! The internal viewer (F3) and quick view (Ctrl+Q): text with encoding detection, or hex.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use commandercraft_core::text::{self, ENCODINGS};
use commandercraft_core::{format_bytes_grouped, format_size};
use egui::{RichText, TextStyle};
use encoding_rs::Encoding;

/// Bytes read for the full viewer and for quick view.
pub const VIEW_LIMIT: usize = 16 * 1024 * 1024;
pub const QUICK_LIMIT: usize = 1024 * 1024;
/// Longest line shown in one piece without wrapping (minified files would stall layout).
const LINE_CAP: usize = 4000;
/// Wrapping lays out every line, so very long files are shown unwrapped.
const WRAP_MAX_LINES: usize = 20_000;

pub struct Viewer {
    pub path: PathBuf,
    pub title: String,
    bytes: Vec<u8>,
    total: u64,
    pub encoding: &'static Encoding,
    detected: &'static Encoding,
    lines: Vec<String>,
    pub hex: bool,
    offset: f32,
    pending: Option<f32>,
    page: f32,
}

pub enum ViewerAction {
    None,
    Close,
}

impl Viewer {
    pub fn open(path: &Path, title: String, limit: usize) -> io::Result<Viewer> {
        let file = File::open(path)?;
        let total = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut bytes = Vec::with_capacity((total as usize).min(limit));
        file.take(limit as u64).read_to_end(&mut bytes)?;
        let detected = text::detect_encoding(&bytes);
        let hex = text::looks_binary(&bytes, detected);
        let mut v = Viewer {
            path: path.to_path_buf(),
            title,
            bytes,
            total,
            encoding: detected,
            detected,
            lines: Vec::new(),
            hex,
            offset: 0.0,
            pending: None,
            page: 400.0,
        };
        v.decode();
        Ok(v)
    }

    fn decode(&mut self) {
        let s = text::decode(&self.bytes, self.encoding);
        self.lines = s
            .split('\n')
            .map(|l| {
                let l = l.strip_suffix('\r').unwrap_or(l);
                let mut out = if l.contains('\t') { l.replace('\t', "    ") } else { l.to_string() };
                if out.len() > LINE_CAP {
                    let mut cut = LINE_CAP;
                    while !out.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    out.truncate(cut);
                    out.push_str(" …");
                }
                out
            })
            .collect();
        if self.lines.last().is_some_and(String::is_empty) {
            self.lines.pop();
        }
    }

    fn truncated(&self) -> bool {
        (self.bytes.len() as u64) < self.total
    }

    /// The viewer's controls: mode, encoding, font, wrap.
    pub fn toolbar(&mut self, ui: &mut egui::Ui, monospace: &mut bool, wrap: &mut bool) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.hex, false, "Text");
            ui.selectable_value(&mut self.hex, true, "Hex");
            ui.separator();
            ui.add_enabled_ui(!self.hex, |ui| {
                let before = self.encoding;
                egui::ComboBox::from_id_salt("cc_viewer_enc").selected_text(self.encoding.name()).show_ui(ui, |ui| {
                    for enc in ENCODINGS {
                        let label = if *enc == self.detected { format!("{} (detected)", enc.name()) } else { enc.name().to_string() };
                        ui.selectable_value(&mut self.encoding, *enc, label);
                    }
                });
                if self.encoding != before {
                    self.decode();
                }
                ui.checkbox(monospace, "Monospace");
                ui.checkbox(wrap, "Wrap");
            });
            ui.separator();
            let size = format!("{} ({} bytes)", format_size(self.total), format_bytes_grouped(self.total));
            let note =
                if self.truncated() { format!("{size} — showing the first {}", format_size(self.bytes.len() as u64)) } else { size };
            ui.label(RichText::new(note).small().weak());
        });
    }

    /// Keys for the full viewer. Returns Close for Esc / F3 / F10 / Q.
    pub fn keys(&mut self, ui: &egui::Ui, monospace: &mut bool, wrap: &mut bool) -> ViewerAction {
        let row = self.row_height(ui, *monospace);
        let (close, delta, home, end, hex, toggle_wrap, toggle_mono) = ui.input(|i| {
            let k = |key| i.key_pressed(key);
            let mut d = 0.0;
            if k(egui::Key::ArrowDown) {
                d += row;
            }
            if k(egui::Key::ArrowUp) {
                d -= row;
            }
            if k(egui::Key::PageDown) || k(egui::Key::Space) {
                d += self.page;
            }
            if k(egui::Key::PageUp) {
                d -= self.page;
            }
            (
                k(egui::Key::Escape) || k(egui::Key::F3) || k(egui::Key::F10) || k(egui::Key::Q),
                d,
                k(egui::Key::Home),
                k(egui::Key::End),
                k(egui::Key::H),
                k(egui::Key::W),
                k(egui::Key::M),
            )
        });
        if hex {
            self.hex = !self.hex;
        }
        if toggle_wrap {
            *wrap = !*wrap;
        }
        if toggle_mono {
            *monospace = !*monospace;
        }
        if home {
            self.pending = Some(0.0);
        } else if end {
            self.pending = Some(f32::MAX);
        } else if delta != 0.0 {
            self.pending = Some((self.offset + delta).max(0.0));
        }
        if close { ViewerAction::Close } else { ViewerAction::None }
    }

    fn row_height(&self, ui: &egui::Ui, monospace: bool) -> f32 {
        let style = if monospace || self.hex { TextStyle::Monospace } else { TextStyle::Body };
        ui.text_style_height(&style)
    }

    /// The content (fills the available space).
    pub fn body(&mut self, ui: &mut egui::Ui, monospace: bool, wrap: bool) {
        let row_h = self.row_height(ui, monospace);
        let wrap = wrap && !self.hex && self.lines.len() <= WRAP_MAX_LINES;
        let mut area = if wrap { egui::ScrollArea::vertical() } else { egui::ScrollArea::both() }
            .id_salt(("cc_viewer", self.path.as_os_str()))
            .auto_shrink(false);
        if let Some(y) = self.pending.take() {
            area = area.vertical_scroll_offset(y);
        }
        let (bytes, lines, hex) = (&self.bytes, &self.lines, self.hex);
        let out = if hex {
            let rows = bytes.len().div_ceil(16);
            area.show_rows(ui, row_h, rows, |ui, range| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                for r in range {
                    ui.label(RichText::new(hex_row(bytes, r)).monospace());
                }
            })
        } else if wrap {
            area.show(ui, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                for l in lines {
                    ui.label(styled(l, monospace));
                }
            })
        } else {
            area.show_rows(ui, row_h, lines.len(), |ui, range| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                for l in &lines[range] {
                    ui.label(styled(l, monospace));
                }
            })
        };
        self.offset = out.state.offset.y;
        self.page = (out.inner_rect.height() - row_h).max(row_h);
    }
}

fn styled(line: &str, monospace: bool) -> RichText {
    let t = RichText::new(if line.is_empty() { " " } else { line });
    if monospace { t.monospace() } else { t }
}

/// `00000010  48 65 6C 6C 6F 20 77 6F  72 6C 64 0A 00 00 00 00  Hello world.....`
pub fn hex_row(bytes: &[u8], row: usize) -> String {
    let start = row * 16;
    let chunk = &bytes[start..(start + 16).min(bytes.len())];
    let mut s = format!("{start:08X}  ");
    for i in 0..16 {
        match chunk.get(i) {
            Some(b) => s.push_str(&format!("{b:02X} ")),
            None => s.push_str("   "),
        }
        if i == 7 {
            s.push(' ');
        }
    }
    s.push(' ');
    s.extend(chunk.iter().map(|b| if b.is_ascii_graphic() || *b == b' ' { *b as char } else { '.' }));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_rows() {
        let b = b"Hello world\n\0\x01";
        assert_eq!(hex_row(b, 0), "00000000  48 65 6C 6C 6F 20 77 6F  72 6C 64 0A 00 01        Hello world...");
    }

    #[test]
    fn opens_text_and_binary() {
        let d = tempfile::tempdir().unwrap();
        let t = d.path().join("t.txt");
        std::fs::write(&t, "line one\r\n\tindented\r\n").unwrap();
        let v = Viewer::open(&t, "t".into(), VIEW_LIMIT).unwrap();
        assert!(!v.hex);
        assert_eq!(v.lines, ["line one", "    indented"]);
        let b = d.path().join("b.bin");
        std::fs::write(&b, [0u8, 1, 2, 3, 255]).unwrap();
        assert!(Viewer::open(&b, "b".into(), VIEW_LIMIT).unwrap().hex);
    }
}
