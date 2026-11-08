//! One row of a file listing.

use std::time::SystemTime;

/// A file or folder in a listing (local or inside an archive).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    /// Bytes; 0 for folders and when unknown.
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Dot-file, or the Windows hidden attribute.
    pub hidden: bool,
    /// The `..` row that leads to the parent folder.
    pub is_parent: bool,
}

impl Entry {
    /// The `..` row.
    pub fn parent_link() -> Self {
        Entry { name: "..".into(), is_dir: true, size: 0, modified: None, hidden: false, is_parent: true }
    }

    /// Name without extension, and the extension (folders have none).
    pub fn stem_ext(&self) -> (&str, &str) {
        if self.is_dir { (&self.name, "") } else { split_ext(&self.name) }
    }

    /// The extension, lowercase (empty for folders and names without one).
    pub fn ext_lower(&self) -> String {
        self.stem_ext().1.to_lowercase()
    }
}

/// `"report.final.pdf"` → `("report.final", "pdf")`; `".bashrc"` and `"README"` have no extension.
pub fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        assert_eq!(split_ext("a.tar.gz"), ("a.tar", "gz"));
        assert_eq!(split_ext(".bashrc"), (".bashrc", ""));
        assert_eq!(split_ext("README"), ("README", ""));
        assert_eq!(split_ext("trailing."), ("trailing.", ""));
    }
}
