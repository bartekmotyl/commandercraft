//! CommanderCraft core: everything a two-panel file manager needs that is not UI.
//!
//! - [`vfs`]: locations (a local folder, or a folder inside an archive) and listing them.
//! - [`archive`]: read-only access to zip / tar / tar.gz / tar.bz2 / tar.xz / gz / bz2 / xz.
//! - [`sort`]: column sorting of listings.
//! - [`ops`]: copy, move, delete, rename, new folder (local), with progress reporting.
//! - [`launch`]: opening files with the system default or a configured command.
//! - [`volumes`]: drives (Windows) or mounted volumes (macOS, Linux).
//! - [`settings`]: persisted preferences, bookmarks and panel tabs.

pub mod archive;
pub mod entry;
pub mod launch;
pub mod ops;
pub mod settings;
pub mod sort;
pub mod vfs;
pub mod volumes;

pub use entry::Entry;
pub use vfs::Location;

/// Human-readable byte size: `999 B`, `1.5 KB`, `12.3 MB`…
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 { format!("{value:.1} {}", UNITS[unit]) } else { format!("{value:.0} {}", UNITS[unit]) }
}

/// Byte size with thousands separators (`1 234 567`), as file lists usually show it.
pub fn format_bytes_grouped(bytes: u64) -> String {
    let digits = bytes.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(10), "10 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(50 * 1024 * 1024), "50 MB");
        assert_eq!(format_bytes_grouped(1234567), "1 234 567");
        assert_eq!(format_bytes_grouped(123), "123");
    }
}
