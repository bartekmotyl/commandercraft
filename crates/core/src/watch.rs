//! Watching the folders shown in the panels, so they refresh when files change.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notify::{RecursiveMode, Watcher as _};

pub struct DirWatcher {
    watcher: Option<notify::RecommendedWatcher>,
    watched: Vec<PathBuf>,
    changed: Arc<Mutex<HashSet<PathBuf>>>,
}

impl DirWatcher {
    /// `on_change` is called (from a background thread) whenever something changed.
    pub fn new(on_change: impl Fn() + Send + Sync + 'static) -> DirWatcher {
        let changed: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
        let sink = changed.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            if let Ok(mut set) = sink.lock() {
                for p in event.paths {
                    if let Some(parent) = p.parent() {
                        set.insert(parent.to_path_buf());
                    }
                    set.insert(p);
                }
            }
            on_change();
        })
        .ok();
        DirWatcher { watcher, watched: Vec::new(), changed }
    }

    /// Watch exactly these folders (non-recursively).
    pub fn watch(&mut self, dirs: &[PathBuf]) {
        let Some(w) = &mut self.watcher else { return };
        if dirs == self.watched.as_slice() {
            return;
        }
        for old in &self.watched {
            if !dirs.contains(old) {
                let _ = w.unwatch(old);
            }
        }
        for new in dirs {
            if !self.watched.contains(new) {
                let _ = w.watch(new, RecursiveMode::NonRecursive);
            }
        }
        self.watched = dirs.to_vec();
    }

    /// Whether `dir` changed since the last call for it (clears its flag).
    pub fn take_changed(&self, dir: &Path) -> bool {
        self.changed.lock().map(|mut s| s.remove(dir)).unwrap_or(false)
    }

    /// Forget changes in folders that are no longer watched.
    pub fn prune(&self) {
        if let Ok(mut s) = self.changed.lock() {
            s.retain(|p| self.watched.iter().any(|w| w == p));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn notices_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::fs::canonicalize(dir.path()).unwrap();
        let mut w = DirWatcher::new(|| {});
        w.watch(std::slice::from_ref(&path));
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(path.join("new.txt"), b"x").unwrap();
        let start = Instant::now();
        while !w.take_changed(&path) {
            assert!(start.elapsed() < Duration::from_secs(5), "no change event");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
