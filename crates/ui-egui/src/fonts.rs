//! System fonts: the families the settings offer, the chosen interface / monospace fonts, and CJK
//! fallbacks so Chinese, Japanese and Korean file names render (egui's built-in fonts lack them).
//!
//! Scanning the system fonts takes a moment, so it runs on a background thread; until it is done
//! the built-in fonts are used.

use std::sync::{Arc, mpsc};

use commandercraft_core::settings::Appearance;
use egui::{FontData, FontDefinitions, FontFamily};

pub struct SystemFonts {
    db: fontdb::Database,
    /// All family names, sorted.
    pub families: Vec<String>,
    /// Families with a monospaced face, sorted.
    pub mono_families: Vec<String>,
}

/// CJK fallback candidates, best first, per script group (first found in each group is used).
#[cfg(target_os = "macos")]
const CJK: &[&[&str]] = &[
    &["Hiragino Sans", "Hiragino Kaku Gothic ProN", "Hiragino Kaku Gothic Pro"],
    &["PingFang SC", "Hiragino Sans GB", "STHeiti", "Heiti SC"],
    &["Apple SD Gothic Neo", "AppleGothic"],
    &["Arial Unicode MS"],
];
#[cfg(windows)]
const CJK: &[&[&str]] = &[
    &["Yu Gothic UI", "Yu Gothic", "Meiryo UI", "Meiryo", "MS UI Gothic"],
    &["Microsoft YaHei UI", "Microsoft YaHei", "SimSun"],
    &["Malgun Gothic", "Gulim"],
    &["Microsoft JhengHei UI", "Microsoft JhengHei"],
];
#[cfg(all(unix, not(target_os = "macos")))]
const CJK: &[&[&str]] = &[
    &["Noto Sans CJK JP", "Noto Sans JP", "Source Han Sans JP", "IPAGothic", "TakaoGothic"],
    &["Noto Sans CJK SC", "Noto Sans SC", "Source Han Sans SC", "WenQuanYi Micro Hei", "WenQuanYi Zen Hei", "Droid Sans Fallback"],
    &["Noto Sans CJK KR", "Noto Sans KR", "NanumGothic", "UnDotum"],
];

impl SystemFonts {
    pub fn scan() -> SystemFonts {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let mut families: Vec<String> = Vec::new();
        let mut mono: Vec<String> = Vec::new();
        for face in db.faces() {
            if let Some((name, _)) = face.families.first() {
                families.push(name.clone());
                if face.monospaced {
                    mono.push(name.clone());
                }
            }
        }
        for v in [&mut families, &mut mono] {
            v.sort_by_key(|s| s.to_lowercase());
            v.dedup();
        }
        SystemFonts { db, families, mono_families: mono }
    }

    /// The regular face of a family.
    pub fn load(&self, family: &str) -> Option<FontData> {
        if family.trim().is_empty() {
            return None;
        }
        let query = fontdb::Query { families: &[fontdb::Family::Name(family)], ..Default::default() };
        let id = self.db.query(&query)?;
        self.db.with_face_data(id, |data, index| {
            let mut fd = FontData::from_owned(data.to_vec());
            fd.index = index;
            fd
        })
    }

    pub fn has(&self, family: &str) -> bool {
        self.families.iter().any(|f| f == family)
    }

    fn cjk_fallbacks(&self) -> Vec<(&'static str, FontData)> {
        CJK.iter().filter_map(|group| group.iter().find(|f| self.has(f)).and_then(|f| self.load(f).map(|d| (*f, d)))).collect()
    }

    /// Font definitions for the appearance settings: chosen fonts first, egui's built-in fonts
    /// (which carry the icons and emoji) next, CJK fallbacks last.
    pub fn definitions(&self, a: &Appearance) -> FontDefinitions {
        let mut defs = FontDefinitions::default();
        let put_first = |defs: &mut FontDefinitions, key: &str, family: FontFamily, data: FontData| {
            defs.font_data.insert(key.to_string(), Arc::new(data));
            defs.families.entry(family).or_default().insert(0, key.to_string());
        };
        if let Some(d) = self.load(&a.font) {
            put_first(&mut defs, "user-ui", FontFamily::Proportional, d);
        }
        if let Some(d) = self.load(&a.mono_font) {
            put_first(&mut defs, "user-mono", FontFamily::Monospace, d);
        }
        for (name, data) in self.cjk_fallbacks() {
            let key = format!("cjk-{name}");
            defs.font_data.insert(key.clone(), Arc::new(data));
            for fam in [FontFamily::Proportional, FontFamily::Monospace] {
                defs.families.entry(fam).or_default().push(key.clone());
            }
        }
        defs
    }
}

/// Scans system fonts in the background.
pub struct FontLoader {
    rx: Option<mpsc::Receiver<SystemFonts>>,
    pub fonts: Option<Arc<SystemFonts>>,
}

impl FontLoader {
    pub fn start(ctx: &egui::Context) -> FontLoader {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(SystemFonts::scan());
            ctx.request_repaint();
        });
        FontLoader { rx: Some(rx), fonts: None }
    }

    /// An idle loader (tests): built-in fonts only.
    pub fn none() -> FontLoader {
        FontLoader { rx: None, fonts: None }
    }

    /// True once, when the scan has just finished.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        match rx.try_recv() {
            Ok(fonts) => {
                self.fonts = Some(Arc::new(fonts));
                self.rx = None;
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                false
            }
        }
    }

    pub fn apply(&self, ctx: &egui::Context, a: &Appearance) {
        if let Some(f) = &self.fonts {
            ctx.set_fonts(f.definitions(a));
        }
    }
}
