//! Opening files: with the system default application, or a configured command.
//!
//! A command is a program with optional arguments, e.g. `code --new-window` or
//! `"C:\Program Files\Notepad++\notepad++.exe" -multiInst`. `{file}` marks where the file path goes;
//! without it the path is appended. On macOS an `.app` bundle path works too
//! (`/Applications/BBEdit.app`), and is opened through `open -a`.

use std::io;
use std::path::Path;
use std::process::Command;

pub fn open_default(path: &Path) -> io::Result<()> {
    open::that_detached(path)
}

/// Open `path` with `command`; an empty command means the system default.
pub fn open_with(command: &str, path: &Path) -> io::Result<()> {
    let parts = split_command(command);
    let Some((program, args)) = parts.split_first() else {
        return open_default(path);
    };
    let file = path.to_string_lossy();
    let mut has_placeholder = false;
    let mut args: Vec<String> = args
        .iter()
        .map(|a| {
            if a.contains("{file}") {
                has_placeholder = true;
                a.replace("{file}", &file)
            } else {
                a.clone()
            }
        })
        .collect();
    if !has_placeholder {
        args.push(file.into_owned());
    }
    let mut cmd = if cfg!(target_os = "macos") && program.trim_end_matches('/').ends_with(".app") {
        let mut c = Command::new("open");
        c.arg("-a").arg(program);
        if !args.is_empty() {
            // `open -a App file --args …` would pass extras to the app; keep it simple: file only.
            c.arg(args.pop().unwrap_or_default());
        }
        c
    } else {
        let mut c = Command::new(program);
        c.args(&args);
        c
    };
    if let Some(dir) = path.parent() {
        cmd.current_dir(dir);
    }
    cmd.spawn().map(|_| ())
}

/// Split a command line on whitespace, keeping `"quoted parts"` (and `'single'` ones) together.
pub fn split_command(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if started || !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitting() {
        assert_eq!(split_command("code -n"), ["code", "-n"]);
        assert_eq!(
            split_command(r#""C:\Program Files\Notepad++\notepad++.exe" -multiInst "{file}""#),
            [r"C:\Program Files\Notepad++\notepad++.exe", "-multiInst", "{file}"]
        );
        assert_eq!(split_command("  "), Vec::<String>::new());
        assert_eq!(split_command("a ''"), ["a", ""]);
    }
}
