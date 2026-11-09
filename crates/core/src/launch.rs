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

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Open a terminal in `dir`. `command` (e.g. `wezterm start --cwd {dir}`) overrides the default:
/// Windows Terminal or cmd on Windows, Terminal.app on macOS, the first terminal found on Linux.
/// `{dir}` is replaced by the folder; the program always starts in it.
pub fn open_terminal(dir: &Path, command: &str) -> io::Result<()> {
    let d = dir.to_string_lossy();
    let parts: Vec<String> = split_command(command).into_iter().map(|p| p.replace("{dir}", &d)).collect();
    if let Some((program, args)) = parts.split_first() {
        if cfg!(target_os = "macos") && program.trim_end_matches('/').ends_with(".app") {
            return Command::new("open").arg("-a").arg(program).arg(dir).spawn().map(|_| ());
        }
        return Command::new(program).args(args).current_dir(dir).spawn().map(|_| ());
    }
    default_terminal(dir)
}

#[cfg(windows)]
fn default_terminal(dir: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    if Command::new("wt").arg("-d").arg(dir).spawn().is_ok() {
        return Ok(());
    }
    Command::new("cmd").args(["/C", "start", "", "cmd.exe"]).current_dir(dir).creation_flags(CREATE_NO_WINDOW).spawn().map(|_| ())
}

#[cfg(target_os = "macos")]
fn default_terminal(dir: &Path) -> io::Result<()> {
    Command::new("open").args(["-a", "Terminal"]).arg(dir).spawn().map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
const LINUX_TERMINALS: [&str; 8] =
    ["x-terminal-emulator", "gnome-terminal", "konsole", "xfce4-terminal", "kitty", "alacritty", "wezterm", "xterm"];

#[cfg(all(unix, not(target_os = "macos")))]
fn default_terminal(dir: &Path) -> io::Result<()> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no terminal found");
    for t in LINUX_TERMINALS {
        match Command::new(t).current_dir(dir).spawn() {
            Ok(_) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Run a shell command line in `dir` without a window (for starting programs: `code .`, `wt`…).
pub fn run_shell(dir: &Path, line: &str) -> io::Result<()> {
    shell(line).current_dir(dir).stdin(std::process::Stdio::null()).spawn().map(|_| ())
}

#[cfg(windows)]
fn shell(line: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut c = Command::new("cmd");
    c.arg("/C").raw_arg(line).creation_flags(CREATE_NO_WINDOW);
    c
}

#[cfg(unix)]
fn shell(line: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(line);
    c
}

/// Run a shell command line in `dir` inside a terminal window that stays open, to see its output.
pub fn run_in_terminal(dir: &Path, line: &str) -> io::Result<()> {
    run_in_terminal_impl(dir, line)
}

#[cfg(windows)]
fn run_in_terminal_impl(dir: &Path, line: &str) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    Command::new("cmd")
        .arg("/C")
        .raw_arg(format!("start \"\" cmd.exe /K {line}"))
        .current_dir(dir)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn run_in_terminal_impl(dir: &Path, line: &str) -> io::Result<()> {
    let script = format!("cd {} && {}", sh_quote(&dir.to_string_lossy()), line);
    let apple = format!("tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell", applescript_escape(&script));
    Command::new("osascript").arg("-e").arg(apple).spawn().map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn run_in_terminal_impl(dir: &Path, line: &str) -> io::Result<()> {
    let script = format!("{line}; exec \"${{SHELL:-sh}}\"");
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no terminal found");
    for t in LINUX_TERMINALS {
        match Command::new(t).current_dir(dir).args(["-e", "sh", "-c", &script]).spawn() {
            Ok(_) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// `it's` → `'it'\''s'`: one argument for a POSIX shell.
#[cfg(unix)]
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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
