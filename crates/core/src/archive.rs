//! Read-only archive support: browse and extract.
//!
//! Formats: zip (and jar), tar, tar.gz / tgz, tar.bz2 / tbz2, tar.xz / txz, and single-file
//! gz / bz2 / xz (shown as an archive holding one file). An archive is indexed once and cached
//! until its size or modification time changes, so browsing inside a large tar.gz stays fast.
//!
//! Entry paths are normalised to `/`-separated relative paths; entries with `..` or absolute paths
//! are dropped, so extraction can never write outside the destination ("zip slip").

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime};

use crate::Entry;
use crate::ops::{Report, Reporter, copy_stream};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    Gz,
    Bz2,
    Xz,
}

/// The archive kind of a file, from its name.
pub fn detect(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    let ends = |suffixes: &[&str]| suffixes.iter().any(|s| name.ends_with(s));
    Some(if ends(&[".zip", ".jar"]) {
        Kind::Zip
    } else if ends(&[".tar.gz", ".tgz"]) {
        Kind::TarGz
    } else if ends(&[".tar.bz2", ".tbz2", ".tbz"]) {
        Kind::TarBz2
    } else if ends(&[".tar.xz", ".txz"]) {
        Kind::TarXz
    } else if ends(&[".tar"]) {
        Kind::Tar
    } else if ends(&[".gz"]) {
        Kind::Gz
    } else if ends(&[".bz2"]) {
        Kind::Bz2
    } else if ends(&[".xz"]) {
        Kind::Xz
    } else {
        return None;
    })
}

/// `join("a/b", "c")` → `"a/b/c"`; `join("", "c")` → `"c"`.
pub fn join(inner: &str, name: &str) -> String {
    if inner.is_empty() { name.to_string() } else { format!("{inner}/{name}") }
}

#[derive(Clone, Debug)]
pub struct ArchiveEntry {
    /// Normalised path, e.g. `docs/2024/report.pdf`.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// Everything in an archive, folders included (also those only implied by file paths).
#[derive(Debug)]
pub struct Index {
    pub kind: Kind,
    /// Sorted by path.
    pub entries: Vec<ArchiveEntry>,
}

impl Index {
    fn build(kind: Kind, raw: Vec<ArchiveEntry>) -> Index {
        let mut map: BTreeMap<String, ArchiveEntry> = BTreeMap::new();
        for e in raw {
            // Implied parent folders.
            let mut prefix = String::new();
            let parts: Vec<&str> = e.path.split('/').collect();
            for part in &parts[..parts.len() - 1] {
                prefix = join(&prefix, part);
                map.entry(prefix.clone()).or_insert_with(|| ArchiveEntry { path: prefix.clone(), is_dir: true, size: 0, modified: None });
            }
            // A later duplicate replaces an earlier one (tar append semantics); an explicit folder
            // entry fills in the date of an implied one.
            map.insert(e.path.clone(), e);
        }
        Index { kind, entries: map.into_values().collect() }
    }

    pub fn is_dir(&self, inner: &str) -> bool {
        self.find(inner).is_some_and(|e| e.is_dir)
    }

    pub fn find(&self, inner: &str) -> Option<&ArchiveEntry> {
        self.entries.binary_search_by(|e| e.path.as_str().cmp(inner)).ok().map(|i| &self.entries[i])
    }

    /// Direct children of the folder `inner` (`""` = root).
    pub fn list(&self, inner: &str) -> Vec<Entry> {
        self.entries
            .iter()
            .filter(|e| parent_of(&e.path) == inner)
            .map(|e| {
                let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
                Entry { hidden: name.starts_with('.'), name, is_dir: e.is_dir, size: e.size, modified: e.modified, is_parent: false }
            })
            .collect()
    }

    /// Entries at or below the given paths, with the path each extracts to (relative to the
    /// destination: the selected item's own name, then whatever is below it).
    fn selection<'a>(&'a self, items: &'a [String]) -> impl Iterator<Item = (&'a ArchiveEntry, String)> + 'a {
        self.entries.iter().filter_map(move |e| target_for(&e.path, items).map(|t| (e, t)))
    }

    /// Files and bytes the given items extract to.
    pub fn totals(&self, items: &[String]) -> (u64, u64) {
        self.selection(items).filter(|(e, _)| !e.is_dir).fold((0, 0), |(n, b), (e, _)| (n + 1, b + e.size))
    }
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

/// Where the archive entry `path` goes when `items` are extracted, if it is part of them.
fn target_for(path: &str, items: &[String]) -> Option<String> {
    items.iter().find_map(|item| {
        let base = item.rsplit('/').next().unwrap_or(item);
        if path == item {
            Some(base.to_string())
        } else {
            path.strip_prefix(item.as_str()).and_then(|rest| rest.strip_prefix('/')).map(|rest| format!("{base}/{rest}"))
        }
    })
}

/// Normalise an archive member name; `None` for names that would escape the destination.
fn normalize(name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    if name.starts_with('/') || name.chars().nth(1) == Some(':') {
        return None;
    }
    let mut parts = Vec::new();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            p => parts.push(p),
        }
    }
    if parts.is_empty() { None } else { Some(parts.join("/")) }
}

// ---------------------------------------------------------------------------------------------
// Indexing

type CacheKey = (u64, Option<SystemTime>);
type Cache = HashMap<PathBuf, (CacheKey, Arc<Index>)>;
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);
const CACHE_LIMIT: usize = 16;

/// The (cached) index of an archive.
pub fn index(path: &Path) -> io::Result<Arc<Index>> {
    let meta = fs::metadata(path)?;
    let key = (meta.len(), meta.modified().ok());
    if let Ok(cache) = CACHE.lock()
        && let Some((k, idx)) = cache.get(path)
        && *k == key
    {
        return Ok(idx.clone());
    }
    let kind = detect(path).ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "not a supported archive"))?;
    let idx = Arc::new(read_index(path, kind, meta.modified().ok())?);
    if let Ok(mut cache) = CACHE.lock() {
        if cache.len() >= CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(path.to_path_buf(), (key, idx.clone()));
    }
    Ok(idx)
}

fn read_index(path: &Path, kind: Kind, archive_mtime: Option<SystemTime>) -> io::Result<Index> {
    let raw = match kind {
        Kind::Zip => {
            let mut zip = zip::ZipArchive::new(BufReader::new(File::open(path)?)).map_err(zip_err)?;
            let mut raw = Vec::with_capacity(zip.len());
            for i in 0..zip.len() {
                let f = zip.by_index_raw(i).map_err(zip_err)?;
                let Some(p) = normalize(f.name()) else {
                    continue;
                };
                raw.push(ArchiveEntry { path: p, is_dir: f.is_dir(), size: f.size(), modified: f.last_modified().and_then(zip_time) });
            }
            raw
        }
        Kind::Tar | Kind::TarGz | Kind::TarBz2 | Kind::TarXz => {
            let mut tar = tar::Archive::new(decoder(path, kind)?);
            let mut raw = Vec::new();
            for item in tar.entries()? {
                let item = item?;
                if let Some(e) = tar_entry(&item) {
                    raw.push(e);
                }
            }
            raw
        }
        Kind::Gz | Kind::Bz2 | Kind::Xz => {
            let size = if kind == Kind::Gz { gzip_size(path).unwrap_or(0) } else { 0 };
            vec![ArchiveEntry { path: single_name(path), is_dir: false, size, modified: archive_mtime }]
        }
    };
    Ok(Index::build(kind, raw))
}

fn tar_entry<R: Read>(item: &tar::Entry<'_, R>) -> Option<ArchiveEntry> {
    let header = item.header();
    let is_dir = match header.entry_type() {
        tar::EntryType::Directory => true,
        tar::EntryType::Regular | tar::EntryType::Continuous | tar::EntryType::GNUSparse => false,
        // Links, devices, PAX/GNU metadata records: not shown (and not extracted).
        _ => return None,
    };
    let path = normalize(&String::from_utf8_lossy(&item.path_bytes()))?;
    let modified = header.mtime().ok().map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s));
    Some(ArchiveEntry { path, is_dir, size: if is_dir { 0 } else { header.size().unwrap_or(0) }, modified })
}

/// The decompressed stream of a (possibly compressed) tar or single-file archive.
fn decoder(path: &Path, kind: Kind) -> io::Result<Box<dyn Read>> {
    let file = BufReader::with_capacity(256 * 1024, File::open(path)?);
    Ok(match kind {
        Kind::Tar => Box::new(file),
        Kind::TarGz | Kind::Gz => Box::new(flate2::bufread::MultiGzDecoder::new(file)),
        Kind::TarBz2 | Kind::Bz2 => Box::new(bzip2::bufread::MultiBzDecoder::new(file)),
        Kind::TarXz | Kind::Xz => Box::new(liblzma::bufread::XzDecoder::new_multi_decoder(file)),
        Kind::Zip => {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "zip is not a stream"));
        }
    })
}

/// `notes.txt.gz` → `notes.txt`.
fn single_name(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => format!("{name}.out"),
    }
}

/// The uncompressed size gzip stores in its last four bytes (modulo 4 GiB).
fn gzip_size(path: &Path) -> io::Result<u64> {
    use std::io::{Seek, SeekFrom};
    let mut f = File::open(path)?;
    f.seek(SeekFrom::End(-4))?;
    let mut b = [0u8; 4];
    f.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b) as u64)
}

fn zip_time(t: zip::DateTime) -> Option<SystemTime> {
    use chrono::{Local, NaiveDate, TimeZone};
    let naive = NaiveDate::from_ymd_opt(t.year().into(), t.month().into(), t.day().into())?.and_hms_opt(
        t.hour().into(),
        t.minute().into(),
        t.second().into(),
    )?;
    // Zip stores local time without a zone.
    Local.from_local_datetime(&naive).earliest().map(SystemTime::from)
}

fn zip_err(e: zip::result::ZipError) -> io::Error {
    match e {
        zip::result::ZipError::Io(e) => e,
        other => io::Error::other(other.to_string()),
    }
}

// ---------------------------------------------------------------------------------------------
// Extraction

/// Extract `items` (paths inside the archive, files or folders) into the folder `dest`.
/// Existing files are skipped unless `overwrite`.
pub fn extract(archive: &Path, items: &[String], dest: &Path, overwrite: bool, reporter: &mut dyn Reporter) -> Report {
    let mut report = Report::default();
    if let Err(e) = extract_inner(archive, items, dest, overwrite, reporter, &mut report) {
        report.errors.push(format!("{}: {e}", archive.display()));
    }
    report
}

fn extract_inner(
    archive: &Path,
    items: &[String],
    dest: &Path,
    overwrite: bool,
    r: &mut dyn Reporter,
    report: &mut Report,
) -> io::Result<()> {
    let index = index(archive)?;
    let (files, bytes) = index.totals(items);
    r.totals(files, bytes);
    // Folders first, so empty ones are created too.
    for (e, target) in index.selection(items).filter(|(e, _)| e.is_dir) {
        let out = dest.join(&target);
        match fs::create_dir_all(&out) {
            Ok(()) => report.dirs += 1,
            Err(err) => report.errors.push(format!("{}: {err}", e.path)),
        }
    }
    let write = |target: &str,
                 modified: Option<SystemTime>,
                 reader: &mut dyn Read,
                 r: &mut dyn Reporter,
                 report: &mut Report|
     -> io::Result<bool> {
        let out = dest.join(target);
        r.file(target);
        if out.exists() && !overwrite {
            report.skipped += 1;
            return Ok(true);
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&out)?;
        let done = copy_stream(reader, &mut file, r)?;
        if !done {
            drop(file);
            let _ = fs::remove_file(&out); // a cancelled, half-written file is useless
            return Ok(false);
        }
        if let Some(t) = modified {
            let _ = file.set_modified(t);
        }
        report.files += 1;
        Ok(true)
    };
    match index.kind {
        Kind::Zip => {
            let mut zip = zip::ZipArchive::new(BufReader::new(File::open(archive)?)).map_err(zip_err)?;
            for i in 0..zip.len() {
                let mut f = match zip.by_index(i) {
                    Ok(f) => f,
                    Err(e) => {
                        report.errors.push(format!("entry {i}: {e}"));
                        continue;
                    }
                };
                if f.is_dir() {
                    continue;
                }
                let Some(path) = normalize(f.name()) else {
                    continue;
                };
                let Some(target) = target_for(&path, items) else {
                    continue;
                };
                let modified = f.last_modified().and_then(zip_time);
                match write(&target, modified, &mut f, r, report) {
                    Ok(true) => {}
                    Ok(false) => {
                        report.cancelled = true;
                        return Ok(());
                    }
                    Err(e) => report.errors.push(format!("{path}: {e}")),
                }
            }
        }
        Kind::Tar | Kind::TarGz | Kind::TarBz2 | Kind::TarXz => {
            let mut tar = tar::Archive::new(decoder(archive, index.kind)?);
            for item in tar.entries()? {
                let mut item = item?;
                let Some(e) = tar_entry(&item) else { continue };
                if e.is_dir {
                    continue;
                }
                let Some(target) = target_for(&e.path, items) else {
                    continue;
                };
                match write(&target, e.modified, &mut item, r, report) {
                    Ok(true) => {}
                    Ok(false) => {
                        report.cancelled = true;
                        return Ok(());
                    }
                    Err(err) => report.errors.push(format!("{}: {err}", e.path)),
                }
            }
        }
        Kind::Gz | Kind::Bz2 | Kind::Xz => {
            let e = &index.entries[0];
            if let Some(target) = target_for(&e.path, items) {
                let mut stream = decoder(archive, index.kind)?;
                match write(&target, e.modified, &mut stream, r, report) {
                    Ok(true) => {}
                    Ok(false) => {
                        report.cancelled = true;
                        return Ok(());
                    }
                    Err(err) => report.errors.push(format!("{}: {err}", e.path)),
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::NoProgress;
    use std::io::Write;

    #[test]
    fn detection() {
        assert_eq!(detect(Path::new("a.ZIP")), Some(Kind::Zip));
        assert_eq!(detect(Path::new("a.tar.gz")), Some(Kind::TarGz));
        assert_eq!(detect(Path::new("a.tgz")), Some(Kind::TarGz));
        assert_eq!(detect(Path::new("a.tar.bz2")), Some(Kind::TarBz2));
        assert_eq!(detect(Path::new("a.txt.gz")), Some(Kind::Gz));
        assert_eq!(detect(Path::new("a.txt")), None);
    }

    #[test]
    fn normalization() {
        assert_eq!(normalize("./a//b/"), Some("a/b".into()));
        assert_eq!(normalize("a\\b"), Some("a/b".into()));
        assert_eq!(normalize("../evil"), None);
        assert_eq!(normalize("/etc/passwd"), None);
        assert_eq!(normalize("C:/x"), None);
        assert_eq!(target_for("docs/a/b.txt", &["docs/a".into()]), Some("a/b.txt".into()));
        assert_eq!(target_for("docs/ab.txt", &["docs/a".into()]), None);
    }

    fn make_zip(path: &Path) {
        let mut w = zip::ZipWriter::new(File::create(path).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        w.start_file("readme.txt", o).unwrap();
        w.write_all(b"top").unwrap();
        w.start_file("docs/2024/report.txt", o).unwrap(); // implies docs/ and docs/2024/
        w.write_all(b"report body").unwrap();
        w.add_directory("empty/", o).unwrap();
        w.start_file("../evil.txt", o).unwrap();
        w.write_all(b"x").unwrap();
        w.finish().unwrap();
    }

    fn make_tar_gz(path: &Path) {
        let gz = flate2::write::GzEncoder::new(File::create(path).unwrap(), flate2::Compression::fast());
        let mut b = tar::Builder::new(gz);
        let data = b"hello from tar";
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_mtime(1_700_000_000);
        h.set_cksum();
        b.append_data(&mut h, "src/main.rs", &data[..]).unwrap();
        b.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn zip_browse_and_extract() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("t.zip");
        make_zip(&zip);
        let idx = index(&zip).unwrap();
        let mut root: Vec<String> = idx.list("").into_iter().map(|e| e.name).collect();
        root.sort();
        assert_eq!(root, ["docs", "empty", "readme.txt"]);
        assert!(idx.is_dir("docs/2024"));
        assert_eq!(idx.list("docs/2024")[0].size, 11);

        let out = dir.path().join("out");
        fs::create_dir(&out).unwrap();
        let rep = extract(&zip, &["docs".into(), "readme.txt".into(), "empty".into()], &out, false, &mut NoProgress);
        assert!(rep.errors.is_empty(), "{:?}", rep.errors);
        assert_eq!(rep.files, 2);
        assert_eq!(fs::read_to_string(out.join("docs/2024/report.txt")).unwrap(), "report body");
        assert!(out.join("empty").is_dir());
        assert!(!dir.path().join("evil.txt").exists());

        // Again: everything exists, nothing overwritten.
        let rep = extract(&zip, &["readme.txt".into()], &out, false, &mut NoProgress);
        assert_eq!((rep.files, rep.skipped), (0, 1));
    }

    #[test]
    fn tar_gz_browse_and_extract() {
        let dir = tempfile::tempdir().unwrap();
        let tgz = dir.path().join("t.tar.gz");
        make_tar_gz(&tgz);
        let idx = index(&tgz).unwrap();
        assert_eq!(idx.list("")[0].name, "src");
        let main = &idx.list("src")[0];
        assert_eq!((main.name.as_str(), main.size), ("main.rs", 14));

        let rep = extract(&tgz, &["src/main.rs".into()], dir.path(), false, &mut NoProgress);
        assert!(rep.errors.is_empty(), "{:?}", rep.errors);
        assert_eq!(fs::read_to_string(dir.path().join("main.rs")).unwrap(), "hello from tar");
        let mtime = fs::metadata(dir.path().join("main.rs")).unwrap().modified().unwrap();
        assert_eq!(mtime, SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000));
    }

    #[test]
    fn single_gz() {
        let dir = tempfile::tempdir().unwrap();
        let gz = dir.path().join("notes.txt.gz");
        let mut enc = flate2::write::GzEncoder::new(File::create(&gz).unwrap(), flate2::Compression::fast());
        enc.write_all(b"plain notes").unwrap();
        enc.finish().unwrap();
        let idx = index(&gz).unwrap();
        let e = &idx.list("")[0];
        assert_eq!((e.name.as_str(), e.size), ("notes.txt", 11));
        let out = dir.path().join("o");
        fs::create_dir(&out).unwrap();
        extract(&gz, &["notes.txt".into()], &out, false, &mut NoProgress);
        assert_eq!(fs::read_to_string(out.join("notes.txt")).unwrap(), "plain notes");
    }
}
