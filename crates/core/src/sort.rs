//! Sorting a listing by a column.
//!
//! `..` always comes first, then folders, then files. Folders are ordered by name unless sorted by
//! date; files by the chosen column with the name as tie-breaker. Names compare "naturally"
//! (`file2` before `file10`) and case-insensitively.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::Entry;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortColumn {
    #[default]
    Name,
    Ext,
    Size,
    Modified,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sort {
    pub column: SortColumn,
    pub descending: bool,
}

impl Sort {
    /// Clicking a column header: the same column flips the direction, another one starts ascending
    /// (except dates and sizes, which start with the newest / largest, as people usually want).
    pub fn toggled(self, column: SortColumn) -> Sort {
        if self.column == column {
            Sort { column, descending: !self.descending }
        } else {
            Sort { column, descending: matches!(column, SortColumn::Size | SortColumn::Modified) }
        }
    }
}

pub fn sort_entries(entries: &mut [Entry], sort: Sort) {
    entries.sort_by(|a, b| compare(a, b, sort));
}

fn compare(a: &Entry, b: &Entry, sort: Sort) -> Ordering {
    // Groups never flip with the direction.
    let group = |e: &Entry| {
        if e.is_parent {
            0
        } else if e.is_dir {
            1
        } else {
            2
        }
    };
    let g = group(a).cmp(&group(b));
    if g != Ordering::Equal {
        return g;
    }
    let by_name = natural_cmp(&a.name, &b.name);
    let ord = match (sort.column, a.is_dir) {
        (SortColumn::Modified, _) => a.modified.cmp(&b.modified).then(by_name),
        (_, true) | (SortColumn::Name, _) => by_name,
        (SortColumn::Ext, false) => natural_cmp(a.stem_ext().1, b.stem_ext().1).then(by_name),
        (SortColumn::Size, false) => a.size.cmp(&b.size).then(by_name),
    };
    if sort.descending { ord.reverse() } else { ord }
}

/// Case-insensitive comparison where runs of digits compare by value.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b), // equal ignoring case: fall back to exact order
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = take_number(&mut ai);
                let nb = take_number(&mut bi);
                // Compare by length after stripping leading zeros, then lexically: no overflow.
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

fn take_number(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        s.push(c);
        it.next();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn file(name: &str, size: u64) -> Entry {
        Entry { name: name.into(), is_dir: false, size, modified: None, hidden: false, is_parent: false }
    }
    fn dir(name: &str) -> Entry {
        Entry { is_dir: true, ..file(name, 0) }
    }
    fn names(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn natural() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("File", "file"), Ordering::Less);
        assert_eq!(natural_cmp("a", "B"), Ordering::Less);
        // Same value: the exact text decides, so the order is still total.
        assert_eq!(natural_cmp("x007", "x7"), Ordering::Less);
        assert_eq!(natural_cmp("x99999999999999999999999", "x100000000000000000000000"), Ordering::Less);
    }

    #[test]
    fn groups_and_columns() {
        let mut v = vec![file("b.txt", 5), dir("zdir"), file("a.rs", 50), Entry::parent_link(), dir("adir")];
        sort_entries(&mut v, Sort::default());
        assert_eq!(names(&v), ["..", "adir", "zdir", "a.rs", "b.txt"]);

        sort_entries(&mut v, Sort { column: SortColumn::Size, descending: true });
        assert_eq!(names(&v), ["..", "zdir", "adir", "a.rs", "b.txt"]);

        sort_entries(&mut v, Sort { column: SortColumn::Ext, descending: false });
        assert_eq!(names(&v), ["..", "adir", "zdir", "a.rs", "b.txt"]);
    }

    #[test]
    fn by_date() {
        let t = SystemTime::UNIX_EPOCH;
        let mut old = file("old", 1);
        old.modified = Some(t);
        let mut new = file("new", 1);
        new.modified = Some(t + Duration::from_secs(10));
        let mut v = vec![old, new];
        sort_entries(&mut v, Sort::default().toggled(SortColumn::Modified));
        assert_eq!(names(&v), ["new", "old"]);
    }
}
