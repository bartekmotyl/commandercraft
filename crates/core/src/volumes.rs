//! Places to jump to from the drive / volume button: drive letters on Windows, mounted volumes on
//! macOS and Linux, plus the home folder.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Volume {
    pub label: String,
    pub path: PathBuf,
}

pub fn home() -> Option<PathBuf> {
    std::env::home_dir()
}

pub fn volumes() -> Vec<Volume> {
    let mut out = Vec::new();
    if let Some(h) = home() {
        out.push(Volume { label: "Home".into(), path: h });
    }
    platform(&mut out);
    out
}

#[cfg(windows)]
fn platform(out: &mut Vec<Volume>) {
    // A: and B: are skipped: probing an empty floppy drive can stall.
    for letter in b'C'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if root.exists() {
            out.push(Volume { label: format!("{}:", letter as char), path: root });
        }
    }
}

#[cfg(target_os = "macos")]
fn platform(out: &mut Vec<Volume>) {
    out.push(Volume { label: "/".into(), path: PathBuf::from("/") });
    list_dir_into("/Volumes", out);
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform(out: &mut Vec<Volume>) {
    out.push(Volume { label: "/".into(), path: PathBuf::from("/") });
    if let Ok(user) = std::env::var("USER") {
        list_dir_into(&format!("/media/{user}"), out);
        list_dir_into(&format!("/run/media/{user}"), out);
    }
    list_dir_into("/mnt", out);
}

#[cfg(unix)]
fn list_dir_into(dir: &str, out: &mut Vec<Volume>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut found: Vec<Volume> = rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| Volume { label: e.file_name().to_string_lossy().into_owned(), path: e.path() })
            .collect();
        found.sort_by(|a, b| a.label.cmp(&b.label));
        out.extend(found);
    }
}

/// The volume a path is on, for the drive button label: `C:` on Windows, `/` or the volume name
/// elsewhere.
pub fn volume_label(path: &std::path::Path) -> String {
    #[cfg(windows)]
    {
        if let Some(std::path::Component::Prefix(p)) = path.components().next() {
            return p.as_os_str().to_string_lossy().into_owned();
        }
        "\\".into()
    }
    #[cfg(not(windows))]
    {
        for prefix in ["/Volumes/", "/media/", "/mnt/"] {
            if let Ok(rest) = path.strip_prefix(prefix)
                && let Some(first) = rest.components().next()
            {
                return first.as_os_str().to_string_lossy().into_owned();
            }
        }
        "/".into()
    }
}
