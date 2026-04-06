# Changelog

## v0.2.1 — 2026-04-05

### Added
- **SQL Engine** (`src/sql_engine/`) — new semantic analysis layer between core and UI:
  - `SqlDialect` trait encapsulating Oracle/PostgreSQL/MySQL differences (identifier casing, schema support, builtin functions, reserved words)
  - `MetadataIndex` — central indexed store replacing scattered tree walking for completion and diagnostics
  - `SemanticAnalyzer` — dual strategy: sqlparser AST parsing + token-based fallback for incomplete SQL and Oracle PL/SQL
  - `CompletionProvider` — fzf-inspired fuzzy matching (Exact > Prefix > Contains > Fuzzy) with ranked scoring and FK-aware JOIN suggestions
  - `DiagnosticProvider` — 3-pass pipeline: syntax (sqlparser per dialect), semantic (unknown table/schema detection), lint rules (`SELECT *`, `DELETE` without `WHERE`, `JOIN` without `ON`)
  - `DiagnosticSet` with source-based updates (syntax/semantic/lint/server can update independently)
  - 60 unit tests covering all engine components
- **Foreign key queries** — `get_foreign_keys()` implemented for Oracle (`ALL_CONSTRAINTS`), PostgreSQL (`information_schema`), MySQL (`KEY_COLUMN_USAGE`)
- **Server-side SQL validation** — `compile_check()` implemented for Oracle (execute + `USER_ERRORS`), PostgreSQL (`PREPARE`/`DEALLOCATE` in rollback transaction), MySQL (`PREPARE`/`DEALLOCATE`)
- **On-demand schema object loading** — typing `schema.` in the editor triggers lazy loading of tables/views for that schema (fixes completion for schemas not yet expanded in sidebar)
- **Encrypted export/import** (`.dbx` format):
  - Single encrypted file containing connections, scripts, groups, filters, script-connection mappings, bind variables
  - ChaCha20Poly1305 + Argon2 encryption with user-chosen password
  - Option to include or exclude credentials (passwords)
  - `manifest.json` with version, timestamp, counts for forward compatibility
  - `leader+e` for export, `leader+i` for import
  - Path field with Tab-completion (filesystem autocomplete, cycles on repeated Tab)
  - Paste support in path and password fields
  - "Show password" toggle in both export and import dialogs
  - Merge strategy on import: skip existing connections/scripts by name
- **PL/SQL keyword coverage** — ~100 new keywords for highlighter and completion:
  - Structure: `RECORD`, `PIPELINED`, `PIPE ROW`, `SUBTYPE`, `VARRAY`, `OBJECT`
  - Control flow: `ELSIF`, `CONTINUE`, `GOTO`, `EXIT`
  - Data types: `NUMBER`, `VARCHAR2`, `CLOB`, `BOOLEAN`, `PLS_INTEGER`, `BINARY_INTEGER`, `SYS_REFCURSOR`, etc.
  - DDL: `CONSTRAINT`, `PRIMARY KEY`, `FOREIGN`, `REFERENCES`, `UNIQUE`, `CASCADE`
  - Analytic: `OVER`, `PARTITION BY`, `UNBOUNDED`, `PRECEDING`, `FOLLOWING`
  - Oracle modifiers: `DETERMINISTIC`, `RESULT_CACHE`, `AUTONOMOUS_TRANSACTION`, `PARALLEL_ENABLE`
  - Oracle functions: `INITCAP`, `LTRIM`, `RTRIM`, `TRANSLATE`, `CONCAT`, `MOD`, `ABS`, `CEIL`, `FLOOR`, `ADD_MONTHS`, `ROW_NUMBER`, `RANK`, `DENSE_RANK`

### Fixed
- **Streaming query cancellation** — closing a result tab or tab now aborts the streaming task (both outer relay and inner DB query), preventing background resource consumption
- **Filter persistence on connection rename** — object filter keys are now migrated when renaming a connection (fixes lost filters after rename)
- **PL/SQL completion context** — PL/SQL block keywords (`IF`, `ELSIF`, `THEN`, `BEGIN`, `LOOP`, etc.) now act as context boundaries, preventing stale `SELECT`/`FROM` context from leaking into PL/SQL code
- **CASE parentheses** — `CASE` removed from function list (no longer inserts `()` on accept)
- **NOT EXISTS parentheses** — added to keywords that auto-insert `()`

### Changed
- Completion engine replaced: old heuristic `starts_with` matching → new fuzzy matching with scoring tiers
- Diagnostics engine replaced: old single-pass → new 3-pass pipeline (syntax + semantic + lint)
- Tokenizer migrated from `src/ui/sql_tokens.rs` to `src/sql_engine/tokenizer.rs` (UI module re-exports)
- Old `ui/completion.rs` and `ui/diagnostics.rs` marked legacy (`#[allow(dead_code)]`)

---

## v0.2.0 — 2026-04-05

### Added
- **DB-specific tree categories** — each database shows only its relevant object types:
  - Oracle: Tables, Views, Materialized Views, Indexes, Sequences, Types, Triggers, Packages, Procedures, Functions
  - MySQL: Tables, Views, Indexes, Triggers, Events, Procedures, Functions (no Packages)
  - PostgreSQL: Tables, Views, Materialized Views, Indexes, Sequences, Triggers, Procedures, Functions
- **Table DDL view** — `}` to switch to DDL sub-view on any table/view tab. PostgreSQL reconstructs from `information_schema`, MySQL uses `SHOW CREATE TABLE`, Oracle uses `DBMS_METADATA.GET_DDL` (read via `DBMS_LOB.SUBSTR` chunks to avoid ODPI-C CLOB crashes)
- **Type inspector** (Oracle) — open a TYPE to see four sub-views: Attributes (#, Name, Type, Type Mod, Length), Methods (Name, Method Type, Result, Final, Instantiable), Declaration, Body
- **Trigger inspector** (Oracle) — open a TRIGGER to see Columns (Name, Usage) and Declaration sub-views
- **Index/Sequence/Event source** — open any index, sequence, or event to view its DDL declaration
- **Materialized view support** — opens like a table (Data, Properties, DDL); shows valid/invalid status and privilege icons in Oracle
- **Oil-style quick actions on DB objects**:
  - `dd` on table/view/package → confirmation modal (red border) → executes `DROP`
  - `r` on table/view/connection → rename modal (yellow border, input field) → executes `ALTER TABLE RENAME TO` or renames connection
  - `o`/`i` on Tables/Views/Packages category → opens new script with CREATE template (dialect-aware)
- **Oil-style connection management**:
  - `yy` on connection → yank; `p` → duplicate into current group (not source group)
  - `r` on connection → rename modal (updates all references: tabs, adapters, config)
  - `o`/`i` on connection/group → open new connection dialog
- **Tree navigation: `h`/`←` collapses parent** — pressing `h` on a child node navigates to the parent and collapses it (like Neovim file explorer)
- **Empty category indicator** — `(empty)` in italic/dim when a tree category has no items
- **Paste support** — `Ctrl+V` / terminal paste works in connection dialog fields, editor search (`/`), and command mode (`:`)
- **"Fetching data..." animation everywhere** — unified loading indicator with animated dots + elapsed timer in DDL, Declaration, Body, source code, type attributes, trigger columns. Single reusable `loading.rs` module
- **Error panel for compile errors** — package/function/procedure compilation failures show Error + SQL split pane (same as script query errors), auto-switches to the body/declaration where the error occurred
- **Error panel for DB actions** — DROP/RENAME failures show the Error + SQL split pane
- **Diff signs in editor gutter** (requires vimltui 0.1.6) — GitSigns-style indicators when editing packages, functions, procedures:
  - Green `│` + green line number = new line (Added)
  - Yellow `│` + yellow line number = changed line (Modified)
  - Red `▼`/`▲` = lines deleted below/above
  - LCS-based diff with string similarity pairing and trailing whitespace tolerance
  - Signs clear on successful compile (original content updated)
- **Compile confirmation modal** — `Ctrl+S` on packages/functions/procedures shows a yellow modal listing which parts have changes before compiling
- **`Ctrl+S` global shortcut** — saves scripts to disk, opens compile modal for source tabs; works from any editor mode
- **CREATE OR REPLACE prefix** — Oracle packages, functions, procedures load with full DDL prefix like DBeaver
- **Oracle `ALL_ERRORS` check** — after compiling PL/SQL, queries `ALL_ERRORS` to detect compilation errors (Oracle accepts invalid DDL silently)
- **Auto-refresh tree after DDL** — `CREATE`/`DROP`/`ALTER`/`RENAME` from scripts automatically reloads the relevant tree category
- **DDL/DML execution from scripts** — `CREATE`, `DROP`, `ALTER`, `INSERT`, `UPDATE`, `DELETE` statements now execute correctly (uses `execute()` instead of `query()` in all three drivers)
- **Cursor shape for Replace mode** — `r` shows underline cursor while waiting for replacement char; `R` shows underline in continuous Replace mode

### Fixed
- **Modal overlays float over content** — Save Changes, Confirm Close, and all modals now use `ratatui::widgets::Clear` instead of blanking the entire screen
- **Oracle CLOB handling** — `DBMS_METADATA.GET_DDL` reads via `DBMS_LOB.SUBSTR` in 4000-char chunks, avoiding `DPI-1080`/`ORA-03135` crashes from direct CLOB `query_row_as`
- **PL/SQL diagnostics suppressed** — sqlparser-based diagnostics (underlines + status bar messages) disabled for Package/Function/Procedure/Type/Trigger tabs; prevents false "Expected TABLE or VIEW" errors
- **Number keys in editor** — `1`/`2`/`3`/`4` only jump to panels when NOT in an editor; in editor focus, they pass to vimltui as count prefix for motions (`y3j`, `d2w`)
- **Compile error text wrapping** — error messages wrap at 40 chars in the error panel
- **Escape in error panes** — only returns to editor from Normal mode; Visual/Search mode Escape handled by vimltui first

### Performance
- **Oracle dual connections** — metadata operations (`meta_conn`) run on a separate connection from user queries (`conn`), eliminating mutex contention that caused ORA-03135 on concurrent operations
- **`USER_*` views for own schema** — Oracle metadata queries use `USER_INDEXES`, `USER_SEQUENCES`, `USER_TYPES`, `USER_TRIGGERS`, `USER_MVIEWS` etc. for the connected user's schema (no privilege checking overhead), falling back to `ALL_*` for foreign schemas
- **Lazy loading for new categories** — Materialized Views, Indexes, Sequences, Types, Triggers, Events load only when expanded; warm-up pre-loads only Tables, Views, Procedures, Functions, Packages

## v0.1.5 — 2026-04-05

### Added
- **Panel jump with number keys** — `1`/`2`/`3`/`4` (without Ctrl) jump to Explorer, Scripts, Editor, or Results panel in Normal mode. Ctrl+1/2/3/4 still works
- **Own schema highlight** — the connected user's schema shows `◉` in green with bold text; other schemas show `◇` in default color. Makes it easy to spot your schema in Oracle environments with many shared schemas
- **Dynamic version display** — statusbar version and `--version` CLI flag now read from `Cargo.toml` at compile time via `env!("CARGO_PKG_VERSION")`, no more hardcoded strings
- **Inline table editing** — edit table data directly in the grid like DBeaver with Vim keybindings:
  - `i` on a cell to edit inline, `Escape`/`Enter` to confirm, `Tab` to move to next cell
  - `o` to insert a new row, `dd` to mark a row for deletion
  - `Ctrl+S` to save all pending changes (generates INSERT/UPDATE/DELETE SQL)
  - `u` to discard all pending changes
  - Color feedback: yellow = modified cell, green = new row, red = deleted row
  - Confirmation modal (yellow border) with change summary before saving
  - Error display split-pane (Error + SQL) identical to script query errors, navigable with `Ctrl+hjkl`
  - NULL cells clear on edit entry; empty input saves as NULL
  - Beam cursor in insert mode, block cursor in normal mode
  - Unsaved grid changes block app quit (navigates to unsaved tab)
  - Cursor position preserved on reload/save
  - Requires primary key for UPDATE/DELETE operations
- **Permission indicators** — lock icons show access level on shared schema objects (🔓 full, 🔒 read-only, ⚡ execute)
- **Progressive data loading** — queries stream results in 500-row batches; "Fetching data..." animation with elapsed timer
- **Row numbers** — `#` column in data grid for tables, views, and query results
- **Fixed column widths** — columns sized to content (max 40 chars) instead of expanding to fill space
- **Tab bar scroll** — auto-scrolls to active tab with yellow `◀ 3` / `2 ▶` overflow indicators

### Performance
- PostgreSQL streaming uses server-side cursors (transaction-wrapped) for immediate first-row delivery
- Oracle streams rows in batches via `spawn_blocking` with `blocking_send`
- MySQL uses sqlx `fetch()` stream with `TryStreamExt`

## v0.1.4 — 2026-04-04

### Added
- **Oil-style script collections** — scripts panel now uses vim keybindings mapped directly to filesystem operations. `i`/`o` to create (name ending in `/` creates a folder), `dd` to delete, `cw` to rename, `yy`/`p` to move scripts between collections. Collections are subdirectories in `~/.local/share/dbtui/scripts/`. Directories rendered with accent color and `▶`/`▼` expand icons
- **SQL syntax error diagnostics** — sqlparser-based validation detects misspelled keywords, missing clauses, and other syntax errors. Each query block is parsed independently with the correct dialect (PostgreSQL, MySQL, Generic for Oracle)
- **Bind variable syntax highlighting** — `:name` (Oracle/MySQL) and `$1`/`$name` (PostgreSQL) are highlighted with a distinct amber/gold color across all 6 themes
- **Ctrl+1/2/3/4 panel navigation** — jump directly to Explorer, Scripts, Editor, or Results panel. Only active in Normal mode

### Fixed
- **`cargo install dbtui` broken from crates.io** — upgraded ratatui to 0.30, crossterm to 0.29, unicode-width to 0.2, and vimltui to 0.1.5. Without a lockfile, the previous dependency ranges caused two incompatible ratatui versions to be resolved, producing type mismatches at compile time
- **Diagnostics/completion used wrong connection** — now uses the script's assigned connection instead of the global sidebar connection. Metadata is scoped to the correct Connection node in the tree
- **Diagnostics false positives on aliases** — `FROM users u` no longer marks `u` as "unknown table". Aliases (both `AS` and implicit) are extracted and excluded from validation
- **Diagnostics not refreshing on connection change** — re-runs immediately when switching a script's connection
- **Default group reappearing after deletion** — "Default" group is now persisted like any other group; only auto-created as fallback when no groups exist and connections need one
- Replaced deprecated `frame.size()` calls with `frame.area()`

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
