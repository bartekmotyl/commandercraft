//! Long file operations run on a background thread; the UI shows their progress and can cancel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use commandercraft_core::ops::{Report, Reporter};

#[derive(Default, Clone)]
pub struct Progress {
    pub current: String,
    pub files_total: u64,
    pub files_seen: u64,
    pub bytes_total: u64,
    pub bytes_done: u64,
}

pub struct Job {
    pub title: String,
    progress: Arc<Mutex<Progress>>,
    result: Arc<Mutex<Option<Report>>>,
    cancel: Arc<AtomicBool>,
    pub started: Instant,
}

impl Job {
    pub fn spawn(title: impl Into<String>, ctx: egui::Context, work: impl FnOnce(&mut dyn Reporter) -> Report + Send + 'static) -> Job {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let result = Arc::new(Mutex::new(None));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut reporter =
            SharedReporter { progress: progress.clone(), cancel: cancel.clone(), ctx: ctx.clone(), last_repaint: Instant::now() };
        let result_slot = result.clone();
        std::thread::spawn(move || {
            let report = work(&mut reporter);
            if let Ok(mut slot) = result_slot.lock() {
                *slot = Some(report);
            }
            ctx.request_repaint();
        });
        Job { title: title.into(), progress, result, cancel, started: Instant::now() }
    }

    pub fn progress(&self) -> Progress {
        self.progress.lock().map(|p| p.clone()).unwrap_or_default()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelling(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The report, once the work is done.
    pub fn take_result(&self) -> Option<Report> {
        self.result.lock().ok().and_then(|mut r| r.take())
    }
}

struct SharedReporter {
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    ctx: egui::Context,
    last_repaint: Instant,
}

impl SharedReporter {
    fn update(&mut self, f: impl FnOnce(&mut Progress)) {
        if let Ok(mut p) = self.progress.lock() {
            f(&mut p);
        }
        if self.last_repaint.elapsed() > Duration::from_millis(50) {
            self.last_repaint = Instant::now();
            self.ctx.request_repaint();
        }
    }
}

impl Reporter for SharedReporter {
    fn totals(&mut self, files: u64, bytes: u64) {
        self.update(|p| {
            p.files_total = files;
            p.bytes_total = bytes;
        });
    }
    fn file(&mut self, name: &str) {
        self.update(|p| {
            p.current = name.to_string();
            p.files_seen += 1;
        });
    }
    fn bytes(&mut self, n: u64) {
        self.update(|p| p.bytes_done += n);
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}
