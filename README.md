# CommanderCraft

A keyboard-driven, two-panel file manager in Rust, in the tradition of the classic orthodox file
managers — with a native GUI on Windows, macOS and Linux. Clean-room: built from observed behaviour
only, no code, icons or texts from existing products.

```
cargo run --release -p commandercraft
```

## Layout

| Crate | Role |
|---|---|
| `crates/core` | UI-agnostic: locations / listing (`vfs`), read-only archives, sorting, file operations with progress, folder watching, encoding detection, launching programs / terminals, volumes, settings |
| `crates/ui-egui` | egui interface: panels, tabs, viewer, dialogs, themes and fonts, background jobs, key handling; headless tests in `tests/` |
| `apps/commandercraft` | the desktop binary (eframe + wgpu) |

## Features (prototype)

- Two panels with tabs and a draggable splitter (double-click it for half and half); bookmarks; drive / volume menu; typed paths (click the path bar)
- Columns Name / Ext / Size / Date, click a header to sort (again to reverse); marked files get a highlighted row
- Copy / move (background, with progress, cancel and overwrite option), rename, new folder, delete to trash or permanently
- Archives: zip, tar, tar.gz/tgz, tar.bz2/tbz2, tar.xz/txz, gz, bz2, xz — enter them like folders, copy out of them, open files inside them, extract whole archives (optionally each into its own folder)
- Internal viewer (F3) and quick view (Ctrl+Q): text with automatic encoding detection (BOM, UTF-8, UTF-16, legacy code pages, Shift_JIS, GBK, …) or hex; monospace and wrap toggles
- Panels refresh by themselves when their folders change on disk
- Command line: start typing to run a shell command in the current folder; terminal here; copy paths; go to a path from the clipboard
- Light, dark or system theme; interface and monospace font and text size from the system's fonts; CJK file names render with system fallback fonts
- Open files with the system default or a per-extension command; F4 uses a configurable editor
- Session (tabs, sort, splitter, bookmarks, preferences) saved to the settings folder on exit

## Keys (`Ctrl` is `⌘` on macOS; letter shortcuts accept Control there too)

| Key | Action |
|---|---|
| ↑ ↓ PgUp PgDn Home End | move (with Shift: mark) |
| Enter / Shift+Enter | open / open with the system default |
| Backspace | parent folder |
| Tab | other panel |
| Space, Insert | mark and move down |
| Ctrl+A / Ctrl+Shift+A / Ctrl+I | mark all / unmark all / invert |
| Ctrl+← / Ctrl+→ | folder under cursor (or current) into the left / right panel |
| Ctrl+↑ / Ctrl+Shift+↑ | folder under cursor in a new tab / new tab on the other side |
| Ctrl+T, Ctrl+W, Ctrl+Tab | new, close, next tab (middle-click a folder: background tab) |
| Ctrl+D | bookmarks |
| F3 / Ctrl+Q (Control key) / F4 | viewer / quick view / edit |
| F5 / F6 / Shift+F6 / F7 | copy / move / rename / new folder |
| Alt+F9 or Ctrl+Shift+E | extract archives |
| F8, Delete, Ctrl+Backspace | delete to trash (Shift: permanently) |
| Ctrl+F3…F6 | sort by name / ext / date / size |
| Ctrl+Shift+C / Ctrl+Alt+C | copy folder path / paths of marked files |
| Ctrl+Shift+V / Ctrl+Alt+V | go to the path on the clipboard / in a new tab |
| Ctrl+Shift+T | terminal in the current folder |
| typing | command line: Enter runs the command, Shift+Enter runs it in a terminal, Esc clears |
| Ctrl+R or F2, Ctrl+U, Ctrl+Shift+H, Ctrl+, | refresh, swap panels, hidden files, settings |

In the viewer: ↑ ↓ PgUp PgDn Space Home End scroll, H hex, W wrap, M monospace, Esc / F3 / Q close.

Mouse: click = cursor, Ctrl-click / right-click = mark, Shift-click = mark range, double-click = open.

## Not yet

Drag and drop, OS context menus, writing into archives, quick search, multi-rename, plugins.

## License

MIT — see [LICENSE](LICENSE).
