//! Locations a panel can show, and listing them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Entry, archive};

/// A folder a panel shows: on disk, or inside an archive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Location {
    Local(PathBuf),
    /// `inner` is a `/`-separated path inside the archive; `""` is its root.
    Archive {
        archive: PathBuf,
        inner: String,
    },
}

impl Location {
    /// The text shown in the path bar: `C:\data\backup.zip\docs\2024`.
    pub fn display(&self) -> String {
        match self {
            Location::Local(p) => p.display().to_string(),
            Location::Archive { archive, inner } => {
                let mut s = archive.display().to_string();
                for part in inner.split('/').filter(|p| !p.is_empty()) {
                    s.push(std::path::MAIN_SEPARATOR);
                    s.push_str(part);
                }
                s
            }
        }
    }

    /// Short name for a tab: the last folder name (or the drive / `/` at a root).
    pub fn title(&self) -> String {
        match self {
            Location::Local(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()),
            Location::Archive { archive, inner } => match inner.rsplit('/').next().filter(|s| !s.is_empty()) {
                Some(last) => last.to_string(),
                None => archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            },
        }
    }

    pub fn local_path(&self) -> Option<&Path> {
        match self {
            Location::Local(p) => Some(p),
            Location::Archive { .. } => None,
        }
    }

    /// The location one level up, and the name of the entry to put the cursor on there.
    /// The archive root's parent is the folder holding the archive.
    pub fn parent(&self) -> Option<(Location, String)> {
        match self {
            Location::Local(p) => {
                let name = p.file_name()?.to_string_lossy().into_owned();
                Some((Location::Local(p.parent()?.to_path_buf()), name))
            }
            Location::Archive { archive, inner } if inner.is_empty() => {
                let name = archive.file_name()?.to_string_lossy().into_owned();
                Some((Location::Local(archive.parent()?.to_path_buf()), name))
            }
            Location::Archive { archive, inner } => {
                let (up, name) = match inner.rsplit_once('/') {
                    Some((up, name)) => (up.to_string(), name.to_string()),
                    None => (String::new(), inner.clone()),
                };
                Some((Location::Archive { archive: archive.clone(), inner: up }, name))
            }
        }
    }

    /// The location of a sub-folder named `name`.
    pub fn child(&self, name: &str) -> Location {
        match self {
            Location::Local(p) => Location::Local(p.join(name)),
            Location::Archive { archive, inner } => Location::Archive { archive: archive.clone(), inner: archive::join(inner, name) },
        }
    }

    /// The path of an entry named `name` inside this location, as the archive knows it
    /// (for archive locations) — see [`Location::child`] for folders.
    pub fn inner_path(&self, name: &str) -> Option<String> {
        match self {
            Location::Archive { inner, .. } => Some(archive::join(inner, name)),
            Location::Local(_) => None,
        }
    }

    /// Whether this location exists (the folder for local paths, the archive file otherwise).
    pub fn exists(&self) -> bool {
        match self {
            Location::Local(p) => p.is_dir(),
            Location::Archive { archive, .. } => archive.is_file(),
        }
    }
}

/// List a location (without `..`; callers add it where there is a parent).
pub fn list(location: &Location, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let mut entries = match location {
        Location::Local(path) => list_local(path)?,
        Location::Archive { archive, inner } => {
            let index = archive::index(archive)?;
            if !inner.is_empty() && !index.is_dir(inner) {
                return Err(io::Error::new(io::ErrorKind::NotFound, format!("no folder {inner} in the archive")));
            }
            index.list(inner)
        }
    };
    if !show_hidden {
        entries.retain(|e| !e.hidden);
    }
    Ok(entries)
}

fn list_local(path: &Path) -> io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in fs::read_dir(path)? {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        // `DirEntry::metadata` does not follow symlinks: a link to a folder should act like one.
        let meta = match item.metadata() {
            Ok(m) if m.file_type().is_symlink() => fs::metadata(item.path()).unwrap_or(m),
            Ok(m) => m,
            Err(_) => {
                out.push(Entry { name, is_dir: false, size: 0, modified: None, hidden: false, is_parent: false });
                continue;
            }
        };
        let hidden = name.starts_with('.') || is_hidden_attr(&meta);
        out.push(Entry {
            name,
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified: meta.modified().ok(),
            hidden,
            is_parent: false,
        });
    }
    Ok(out)
}

#[cfg(windows)]
fn is_hidden_attr(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

#[cfg(not(windows))]
fn is_hidden_attr(_meta: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_navigation() {
        let zip = PathBuf::from("data").join("a.zip");
        let root = Location::Archive { archive: zip.clone(), inner: String::new() };
        let docs = root.child("docs");
        let deep = docs.child("2024");
        assert_eq!(deep, Location::Archive { archive: zip.clone(), inner: "docs/2024".into() });
        assert_eq!(deep.parent(), Some((docs.clone(), "2024".into())));
        assert_eq!(docs.parent(), Some((root.clone(), "docs".into())));
        assert_eq!(root.parent(), Some((Location::Local(PathBuf::from("data")), "a.zip".into())));
        assert_eq!(deep.title(), "2024");
        assert_eq!(root.title(), "a.zip");
        assert!(deep.display().ends_with(&format!("a.zip{0}docs{0}2024", std::path::MAIN_SEPARATOR)));
    }

    #[test]
    fn local_listing() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("f.txt"), b"hello").unwrap();
        fs::write(dir.path().join(".hidden"), b"").unwrap();
        let loc = Location::Local(dir.path().to_path_buf());
        let mut all = list(&loc, true).unwrap();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(all.len(), 3);
        let f = all.iter().find(|e| e.name == "f.txt").unwrap();
        assert_eq!((f.is_dir, f.size), (false, 5));
        assert!(all.iter().find(|e| e.name == "sub").unwrap().is_dir);
        assert_eq!(list(&loc, false).unwrap().len(), 2);
    }
}
