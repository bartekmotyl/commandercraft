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
| `crates/core` | UI-agnostic: locations / listing (`vfs`), read-only archives, sorting, file operations with progress, launching, volumes, settings |
| `crates/ui-egui` | egui interface: panels, tabs, dialogs, background jobs, key handling; headless tests in `tests/` |
| `apps/commandercraft` | the desktop binary (eframe + wgpu) |

## Features (prototype)

- Two panels, each with tabs; bookmarks; drive / volume menu; typed paths (click the path bar)
- Columns Name / Ext / Size / Date, click a header to sort (again to reverse)
- Copy / move (background, with progress, cancel and overwrite option), rename, new folder, delete to trash or permanently
- Archives: zip, tar, tar.gz/tgz, tar.bz2/tbz2, tar.xz/txz, gz, bz2, xz — enter them like folders, copy out of them, open files inside them
- Open files with the system default or a per-extension command; F4 uses a configurable editor
- Session (tabs, sort, bookmarks, preferences) saved to the settings folder on exit

## Keys (`Ctrl` is `⌘` on macOS)

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
| F3 / F4 | view / edit |
| F5 / F6 / Shift+F6 / F7 | copy / move / rename / new folder |
| F8, Delete, Ctrl+Backspace | delete to trash (Shift: permanently) |
| Ctrl+F3…F6 | sort by name / ext / date / size |
| Ctrl+R or F2, Ctrl+U, Ctrl+Shift+H, Ctrl+, | refresh, swap panels, hidden files, settings |

Mouse: click = cursor, Ctrl-click / right-click = mark, Shift-click = mark range, double-click = open.

## Not yet

Drag and drop, OS context menus, writing into archives, file watching (panels refresh on window
focus and after operations), quick search, multi-rename, viewer, plugins.

## License

MIT — see [LICENSE](LICENSE).
