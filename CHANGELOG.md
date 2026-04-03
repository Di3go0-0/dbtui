# Changelog

## v0.1.0 — 2026-04-03

First release.

### Features

- **Multi-database support** — Oracle, PostgreSQL, MySQL
- **Vim editor** — Full modal editing (Normal, Insert, Visual), motions, operators, undo/redo, search, relative line numbers
- **Command mode** — `:w`, `:q`, `:q!`, `:wq`, `:{number}` to jump to line
- **Schema explorer** — Browse connections, schemas, tables, views, packages, functions, procedures with tree navigation
- **Object filtering** — Per-connection filters for schemas and objects
- **SQL scripts** — Create, edit, save, rename, duplicate, delete scripts
- **Smart query execution** — `<Space>Enter` executes query block at cursor, visual selection executes selected text
- **Result tabs** — Multiple result sets per script, `<Space>/` opens new result tab, `{`/`}` to switch
- **Error display** — Split pane with error message (left) and failed SQL (right), red borders, line number extraction
- **Data grid** — Cell-level navigation (h/j/k/l/e/b), visual selection (`v`), copy to clipboard (`y`), horizontal scroll for wide tables
- **Connection picker** — `<Space>c` assigns connections to scripts, saved/restored between sessions
- **Leader key system** — Global `<Space>` command palette with help popup, works from any panel
- **Theme system** — 6 themes (Tokyo Night, Catppuccin, Dracula, Nord, Gruvbox, Default) with transparent backgrounds
- **Spatial navigation** — `Ctrl+h/j/k/l` or `Ctrl+arrows` to move between panels following visual layout
- **Tab management** — `[`/`]` to switch tabs, `<Space>bd` to close, duplicate prevention
- **Package source** — View/edit package declarations and bodies, function/procedure source code
- **Oracle source fetch** — Row-by-row from ALL_SOURCE, tab expansion, null/CR stripping
- **Encrypted storage** — Connection credentials encrypted at rest (ChaCha20Poly1305 + Argon2)
- **Cursor shape** — Beam in Insert mode, block in Normal/Visual
- **Unsaved changes guard** — Warns before quitting with modified buffers
- **Clipboard** — `p`/`P` paste from system clipboard, `y` copies to system clipboard
- **Syntax highlighting** — SQL keywords, strings, numbers, comments, operators
