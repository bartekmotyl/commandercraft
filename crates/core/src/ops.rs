//! File operations on local files: copy, move, delete, rename, new folder.
//!
//! Operations keep going after a failing item and collect the errors in a [`Report`], so one locked
//! file does not stop a copy of a thousand. Progress and cancellation go through a [`Reporter`],
//! which lets the UI run them on a background thread.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// Receives progress; also asked whether to stop.
pub trait Reporter {
    /// Called once, before any work, with the files and bytes to process.
    fn totals(&mut self, _files: u64, _bytes: u64) {}
    /// Starting the file (or folder) `name`.
    fn file(&mut self, _name: &str) {}
    /// `n` more bytes are done.
    fn bytes(&mut self, _n: u64) {}
    /// Return true to stop as soon as possible.
    fn cancelled(&self) -> bool {
        false
    }
}

/// A reporter that ignores everything (tests, quick operations).
pub struct NoProgress;
impl Reporter for NoProgress {}

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub files: u64,
    pub dirs: u64,
    /// Existing targets left alone (overwrite off).
    pub skipped: u64,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

impl Report {
    pub fn merge(&mut self, other: Report) {
        self.files += other.files;
        self.dirs += other.dirs;
        self.skipped += other.skipped;
        self.errors.extend(other.errors);
        self.cancelled |= other.cancelled;
    }
}

/// Copy `reader` to `writer`, reporting bytes. Returns false if cancelled.
pub fn copy_stream(reader: &mut dyn Read, writer: &mut dyn Write, r: &mut dyn Reporter) -> io::Result<bool> {
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if r.cancelled() {
            return Ok(false);
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(true),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        writer.write_all(&buf[..n])?;
        r.bytes(n as u64);
    }
}

/// Files and bytes below the given paths (folders counted recursively, links not followed).
pub fn totals(paths: &[PathBuf]) -> (u64, u64) {
    fn walk(p: &Path, acc: &mut (u64, u64)) {
        let Ok(meta) = fs::symlink_metadata(p) else {
            return;
        };
        if meta.is_dir() {
            if let Ok(rd) = fs::read_dir(p) {
                for e in rd.flatten() {
                    walk(&e.path(), acc);
                }
            }
        } else {
            acc.0 += 1;
            acc.1 += meta.len();
        }
    }
    let mut acc = (0, 0);
    for p in paths {
        walk(p, &mut acc);
    }
    acc
}

fn name_of(p: &Path) -> io::Result<&std::ffi::OsStr> {
    p.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{} has no name", p.display())))
}

/// Refuse copying a folder into itself, and an item onto itself.
fn check_target(src: &Path, target: &Path) -> io::Result<()> {
    let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let src_c = canon(src);
    let target_parent = target.parent().map(canon).unwrap_or_default();
    let target_c = target_parent.join(target.file_name().unwrap_or_default());
    if src_c == target_c {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "source and target are the same"));
    }
    if src.is_dir() && target_c.starts_with(&src_c) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "cannot copy a folder into itself"));
    }
    Ok(())
}

/// Copy files and folders into the folder `dest`.
pub fn copy(sources: &[PathBuf], dest: &Path, overwrite: bool, r: &mut dyn Reporter) -> Report {
    let (files, bytes) = totals(sources);
    r.totals(files, bytes);
    let mut report = Report::default();
    for src in sources {
        if r.cancelled() {
            report.cancelled = true;
            break;
        }
        let target = match name_of(src) {
            Ok(n) => dest.join(n),
            Err(e) => {
                report.errors.push(e.to_string());
                continue;
            }
        };
        if let Err(e) = check_target(src, &target) {
            report.errors.push(format!("{}: {e}", src.display()));
            continue;
        }
        copy_item(src, &target, overwrite, r, &mut report);
    }
    report
}

fn copy_item(src: &Path, target: &Path, overwrite: bool, r: &mut dyn Reporter, report: &mut Report) {
    if r.cancelled() {
        report.cancelled = true;
        return;
    }
    let meta = match fs::symlink_metadata(src) {
        Ok(m) => m,
        Err(e) => {
            report.errors.push(format!("{}: {e}", src.display()));
            return;
        }
    };
    if meta.is_dir() {
        r.file(&src.display().to_string());
        if let Err(e) = fs::create_dir_all(target) {
            report.errors.push(format!("{}: {e}", target.display()));
            return;
        }
        report.dirs += 1;
        match fs::read_dir(src) {
            Ok(rd) => {
                for e in rd.flatten() {
                    copy_item(&e.path(), &target.join(e.file_name()), overwrite, r, report);
                    if report.cancelled {
                        return;
                    }
                }
            }
            Err(e) => report.errors.push(format!("{}: {e}", src.display())),
        }
        if let Ok(t) = meta.modified() {
            let _ = File::open(target).and_then(|f| f.set_modified(t));
        }
    } else {
        r.file(&src.display().to_string());
        if target.exists() && !overwrite {
            report.skipped += 1;
            r.bytes(meta.len());
            return;
        }
        match copy_file(src, target, &meta, r) {
            Ok(true) => report.files += 1,
            Ok(false) => report.cancelled = true,
            Err(e) => report.errors.push(format!("{}: {e}", src.display())),
        }
    }
}

/// Copy one file. Small files use the OS copy (which clones on APFS / ReFS and keeps permissions);
/// big ones are streamed so progress and cancel work within the file.
fn copy_file(src: &Path, target: &Path, meta: &fs::Metadata, r: &mut dyn Reporter) -> io::Result<bool> {
    const STREAM_ABOVE: u64 = 16 * 1024 * 1024;
    if meta.len() <= STREAM_ABOVE {
        fs::copy(src, target)?;
        r.bytes(meta.len());
    } else {
        let mut input = File::open(src)?;
        let mut output = File::create(target)?;
        if !copy_stream(&mut input, &mut output, r)? {
            drop(output);
            let _ = fs::remove_file(target);
            return Ok(false);
        }
        let _ = fs::set_permissions(target, meta.permissions());
    }
    if let Ok(t) = meta.modified() {
        let _ = File::options().write(true).open(target).and_then(|f| f.set_modified(t));
    }
    Ok(true)
}

/// Move files and folders into the folder `dest`: a rename where possible, otherwise copy and
/// delete (the source is only deleted when its copy had no errors or skipped files).
pub fn move_to(sources: &[PathBuf], dest: &Path, overwrite: bool, r: &mut dyn Reporter) -> Report {
    let (files, bytes) = totals(sources);
    r.totals(files, bytes);
    let mut report = Report::default();
    for src in sources {
        if r.cancelled() {
            report.cancelled = true;
            break;
        }
        let target = match name_of(src) {
            Ok(n) => dest.join(n),
            Err(e) => {
                report.errors.push(e.to_string());
                continue;
            }
        };
        if let Err(e) = check_target(src, &target) {
            report.errors.push(format!("{}: {e}", src.display()));
            continue;
        }
        r.file(&src.display().to_string());
        let target_exists = fs::symlink_metadata(&target).is_ok();
        if target_exists && target.is_dir() != src.is_dir() {
            // Never replace a whole folder by a file (or the other way round).
            report.errors.push(format!(
                "{}: a {} with this name exists",
                target.display(),
                if target.is_dir() { "folder" } else { "file" }
            ));
            continue;
        }
        if target_exists && !src.is_dir() {
            if !overwrite {
                report.skipped += 1;
                continue;
            }
            if let Err(e) = remove_path(&target) {
                report.errors.push(format!("{}: {e}", target.display()));
                continue;
            }
        }
        // Folders that exist on both sides are merged by copy + delete.
        if !(target_exists && src.is_dir()) && fs::rename(src, &target).is_ok() {
            let (f, b) = totals(std::slice::from_ref(&target));
            r.bytes(b);
            report.files += f.max(1);
            continue;
        }
        let mut item = Report::default();
        copy_item(src, &target, overwrite, r, &mut item);
        let clean = item.errors.is_empty() && item.skipped == 0 && !item.cancelled;
        report.merge(item);
        if clean && let Err(e) = remove_path(src) {
            report.errors.push(format!("{}: {e}", src.display()));
        }
    }
    report
}

fn remove_path(p: &Path) -> io::Result<()> {
    if fs::symlink_metadata(p)?.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }
}

/// Delete files and folders: to the trash / recycle bin, or permanently.
pub fn delete(paths: &[PathBuf], to_trash: bool, r: &mut dyn Reporter) -> Report {
    r.totals(paths.len() as u64, 0);
    let mut report = Report::default();
    for p in paths {
        if r.cancelled() {
            report.cancelled = true;
            break;
        }
        r.file(&p.display().to_string());
        let result = if to_trash { trash::delete(p).map_err(|e| io::Error::other(e.to_string())) } else { remove_path(p) };
        match result {
            Ok(()) => report.files += 1,
            Err(e) => report.errors.push(format!("{}: {e}", p.display())),
        }
    }
    report
}

/// Create the folder `name` inside `dir` (nested names like `a/b` allowed).
pub fn mkdir(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty name"));
    }
    let path = dir.join(name);
    if path.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", path.display())));
    }
    fs::create_dir_all(&path)?;
    Ok(path)
}

/// Rename `src` within its folder.
pub fn rename(src: &Path, new_name: &str) -> io::Result<PathBuf> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.contains(['/', '\\']) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid name"));
    }
    let target = src.with_file_name(new_name);
    // A case-only rename on a case-insensitive file system "exists" already: allow it.
    let case_only = target.to_string_lossy().to_lowercase() == src.to_string_lossy().to_lowercase();
    if target.exists() && !case_only {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{new_name} already exists")));
    }
    fs::rename(src, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path) {
        fs::create_dir_all(root.join("src/sub")).unwrap();
        fs::write(root.join("src/a.txt"), b"aaa").unwrap();
        fs::write(root.join("src/sub/b.txt"), b"bbbb").unwrap();
    }

    #[test]
    fn copy_folder_and_skip_existing() {
        let d = tempfile::tempdir().unwrap();
        tree(d.path());
        let dest = d.path().join("dest");
        fs::create_dir(&dest).unwrap();
        let rep = copy(&[d.path().join("src")], &dest, false, &mut NoProgress);
        assert!(rep.errors.is_empty(), "{:?}", rep.errors);
        assert_eq!(rep.files, 2);
        assert_eq!(fs::read(dest.join("src/sub/b.txt")).unwrap(), b"bbbb");
        assert!(d.path().join("src/a.txt").exists());

        fs::write(dest.join("src/a.txt"), b"changed").unwrap();
        let rep = copy(&[d.path().join("src")], &dest, false, &mut NoProgress);
        assert_eq!((rep.files, rep.skipped), (0, 2));
        assert_eq!(fs::read(dest.join("src/a.txt")).unwrap(), b"changed");
        let rep = copy(&[d.path().join("src")], &dest, true, &mut NoProgress);
        assert_eq!(rep.files, 2);
        assert_eq!(fs::read(dest.join("src/a.txt")).unwrap(), b"aaa");
    }

    #[test]
    fn refuses_copy_into_itself() {
        let d = tempfile::tempdir().unwrap();
        tree(d.path());
        let rep = copy(&[d.path().join("src")], &d.path().join("src/sub"), false, &mut NoProgress);
        assert_eq!(rep.errors.len(), 1);
        let rep = copy(&[d.path().join("src/a.txt")], &d.path().join("src"), false, &mut NoProgress);
        assert_eq!(rep.errors.len(), 1);
    }

    #[test]
    fn move_and_merge() {
        let d = tempfile::tempdir().unwrap();
        tree(d.path());
        let dest = d.path().join("dest");
        fs::create_dir_all(dest.join("src")).unwrap();
        fs::write(dest.join("src/existing.txt"), b"keep").unwrap();
        let rep = move_to(&[d.path().join("src")], &dest, false, &mut NoProgress);
        assert!(rep.errors.is_empty(), "{:?}", rep.errors);
        assert!(!d.path().join("src").exists());
        assert!(dest.join("src/sub/b.txt").exists());
        assert!(dest.join("src/existing.txt").exists());
    }

    #[test]
    fn delete_rename_mkdir() {
        let d = tempfile::tempdir().unwrap();
        tree(d.path());
        let made = mkdir(d.path(), "new/nested").unwrap();
        assert!(made.is_dir());
        assert!(mkdir(d.path(), "new").is_err());
        let renamed = rename(&d.path().join("src/a.txt"), "c.txt").unwrap();
        assert!(renamed.exists());
        assert!(rename(&renamed, "../x").is_err());
        let rep = delete(&[d.path().join("src"), d.path().join("new")], false, &mut NoProgress);
        assert!(rep.errors.is_empty());
        assert!(!d.path().join("src").exists());
    }
}
