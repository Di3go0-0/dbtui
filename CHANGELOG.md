# Changelog

## v0.1.4 — 2026-04-04

### Fixed
- **`cargo install dbtui` broken from crates.io** — upgraded ratatui to 0.30, crossterm to 0.29, unicode-width to 0.2, and vimltui to 0.1.5. Without a lockfile, the previous dependency ranges caused two incompatible ratatui versions to be resolved, producing type mismatches at compile time.
- Replaced deprecated `frame.size()` calls with `frame.area()`.

## v0.1.3 — 2026-04-04

### Added
- **Connection groups** — Organize connections in collapsible groups in the sidebar. Context menu (`m` on group) to rename, delete, or create groups. Group field in connection dialog (`Ctrl+G` to cycle). Empty groups persist across restarts via `groups.json`. Groups start collapsed on launch
- **Query elapsed time** — Result tabs show execution time (e.g. `Results (42) 128ms`); status bar shows time on query completion
- **Leader snippets menu** (`<leader>s`) — SQL template shortcuts; `<leader>s s` inserts a `SELECT * FROM` template at cursor and enters Insert mode
- **Auto-correct keyword case** — Typing `select` + Space auto-corrects to `SELECT`; same for all SQL keywords and functions. Table names corrected to match DB metadata case
- **Auto-insert parentheses** — Accepting a function (`COUNT`, `SUM`, `UPPER`, etc.) or `IN`/`EXISTS` from completion inserts `()` with cursor between parens

### Fixed
- Completion context detection: typing "or" after `FROM` no longer triggers `Predicate` context (was matching `OR` keyword instead of continuing `TableRef`)
- `WHERE` now appears in completion suggestions from `ON`/predicate context
- `JOIN`/`OUTER` suggested after `LEFT`/`RIGHT`/etc. in `TableRef` context
- Keywords no longer vanish from completion when prefix matches exactly (e.g. typing "IN" kept showing "IN")
- Diagnostic underline panic when editor lines change (e.g. after inserting snippet template)
- Tree drain operations use depth-based traversal instead of next-connection scan (supports nested group hierarchy)

## v0.1.2 — 2026-04-03

### Added
- **Context-aware SQL completion (CMP)** — Suggests the right thing at the right place:
  - Tables/views after `FROM`/`JOIN`, columns after `SELECT`/`WHERE`
  - Oracle `SCHEMA.OBJECT` hierarchy, `alias.column` resolution
  - Dialect-aware: Oracle/PostgreSQL with schemas, MySQL direct
  - Ctrl+Space (open), Ctrl+N/P (navigate), Ctrl+Y or Enter (accept)
  - Auto-trigger while typing, max 4 visible items
  - Query block scoping (blank line separation)
- **SQL diagnostics (LCP)** — Red underline on invalid table/view references, message in status bar
- **Bind variables prompt** — Modal to fill `:variableName` parameters before execution, values persisted across sessions
- **Metadata warm-up** — Tables/views auto-load on connect (no manual tree expansion needed for CMP)
- **Column cache** — On-demand column loading for `alias.` completion
- **Per-tab connections** — Each tab tracks its own connection, shown as `[conn_name]` in tab bar
- **Auto-connect on script open** — Scripts reconnect to their saved connection automatically
- **"Loading context..." indicator** — Status bar shows loading state during auto-connect
- **Unsaved changes modal** — Lists all unsaved files with quit/cancel options
- **Shared SQL tokenizer** — `sql_tokens` module used by both CMP and diagnostics

### Fixed
- Editor command mode (`:`) and search mode (`/`) no longer intercepted by global keys
- Leader key works in Visual mode (query execution with visual selection)
- Script connection preserved when opening (was lost on tab creation)
- Query errors now show real editor line number instead of "line 1"
- Diagnostic underlines stay within editor area in split view
- Completion popup floats above results panel

### Performance
- Prioritized schema loading: user's schema loads first, others sequentially
- Batch tree node insertion with `splice()` instead of O(n²) individual inserts
- `visible_tree()` uses reusable buffer instead of per-node `format!()` allocations
- Completion clones only query block lines, not entire editor buffer
- Diagnostics only run on Insert→Normal transition, not every keystroke

## v0.1.1 — 2026-04-03

### Changed
- **Vim editor powered by [vimltui](https://crates.io/crates/vimltui)** — extracted vim module as a reusable crate
- Generic `EditorAction` (no app-specific variants)
- `SyntaxHighlighter` trait with SQL highlighter extracted to separate module
- Leader key handling moved to app layer

### Added
- `f`/`F`/`t`/`T` character find motions
- `r` replace char, `s` substitute
- Auto-indent on `o`/`O`
- Search match highlighting (yellow for all matches, orange for current)
- Escape clears search highlights

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
