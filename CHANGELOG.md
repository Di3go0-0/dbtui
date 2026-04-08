# Changelog

## Unreleased — v0.3.2

### Added

### Fixed
- **Diagnostics no longer split a query on a single blank line** — `check_syntax` was splitting query blocks on every blank line, while the runtime executor (`query_block_at_cursor`) only splits on **two or more** consecutive blank lines. The mismatch meant a SELECT visually broken like `SELECT *⏎⏎FROM employees` ran fine but the editor lit up the gutter with bogus syntax errors on each half. The diagnostics splitter now uses the same 2-blank threshold (PL/SQL anonymous blocks still ignore interior blanks via `compute_plsql_mask`), so a single blank line keeps both halves in the same statement and produces zero diagnostics. New `single_blank_line_does_not_split_query` test pins the contract; the existing `multiple_query_blocks_validated_independently` test was updated from one to two blank-line separators to match.

### Changed

## v0.3.1 — 2026-04-08

### Added

#### Connection dialog
- **Reworked connection modal (Proposal B)** — grouped sections (`─ Connection ─`, `─ Authentication ─`) with right-aligned labels and a `│` separator between the label and value columns. Header row shows Name / Type / Group; the dialog title carries a static left (`New Connection` / `Edit Connection`) plus a dynamic right `[Name]` that updates live as you type. Rounded border, animated `◜◝◞◟` spinner with elapsed seconds while connecting, password row gains an inline `◉/⊘` visibility badge, and Tab cycles fields in visual order instead of struct order.
- **Experimental oil-style inline connection editor (Proposal D)** — bound under `<leader>I`. Floating buffer-like editor where every connection field is a single line (`type:`, `name:`, `host:`, `port:`, `user:`, `pass:`, `db:`, `group:`). Vim navigation: `j/k` move rows, `i`/`a`/`o` enter Insert on text fields, `Tab`/`h/l` cycles Type and Group, `Enter` saves + connects, `Esc` cancels. `Ctrl+P` toggles password visibility in either mode. Marked experimental in the title bar so the production `a` add_connection flow stays untouched.
- **Friendly connection errors** — `friendly_connect_error` translates raw driver errors into a multi-line message with a short headline, the raw detail, and an actionable `Hint:` line. Recognises 14 Oracle ORA codes (`ORA-12541` listener, `ORA-01017` bad creds, `ORA-12514` service name, `ORA-28000` locked, `ORA-28001` expired, `ORA-12545` host, `ORA-12170` timeout, `ORA-12154` resolve, `ORA-12537`, `ORA-12560`, `ORA-01034`, `ORA-01005`, `ORA-00942`) plus generic patterns for sqlx (Postgres/MySQL): connection refused, password authentication failed, timeouts, DNS lookup failures, "database does not exist", SSL/TLS, too many connections, role does not exist. The connection dialog renders the headline as a red badge and the hint in accent.

#### Query execution
- **Loading placeholder with live elapsed timer** — when you execute a query the result pane appears immediately with a `Fetching data... X.X s` panel that ticks in real time, instead of waiting until the first batch arrives. The placeholder lives in the bottom 40% split alongside the editor and persists until the first row arrives or the query is cancelled. The border highlights when the user navigates into the panel and the title gains a `[cancel: close]` hint so the cancel UX is discoverable.
- **Close-result cancels in-flight query** — pressing the close binding on a streaming result pane (whether the panel is showing the loading placeholder or partial rows) aborts the spawned task via the existing `streaming_abort` AbortHandle and clears the placeholder state. Status bar shows `Query cancelled`.
- **Run counter + timestamp on result tabs** — the result tab label now reads `Result 1 (142) 34ms 3x 12:34:51` after the third execution. The `3x` only appears when `run_count > 1` so the once-run case stays clean. The timestamp is the wall-clock time of the most recent populate, rendered via `chrono::Local`.
- **Fresh-result flash** — when a result tab is (re)populated the label briefly flashes with an accent background for ~450ms, so the user sees a visual pulse confirming the query actually ran (helps when running the same query repeatedly to watch for changes).
- **Manual refresh on script result tabs** — `r` (existing `grid.refresh_data` keybind) on a script result tab re-executes the query that produced it, instead of erroring out. The result tab is replaced in place via the existing `<leader>Enter` machinery; counter climbs, flash fires, timestamp updates.
- **Auto-refresh** — `R` (new `grid.toggle_auto_refresh`) cycles the active result tab through preset intervals `off → 2s → 5s → 10s → 30s → off`. The main loop's `tick_auto_refresh` scans every tab and dispatches a re-execute when its `next_at` instant has passed and no query is currently in flight on that tab. Result tab label gains a `↻5s` indicator while active. `next_at` is recomputed from the moment the previous run *finished*, so a slow query never piles up concurrent refreshes.
- **Richer Oracle error display** — `format_oracle_error(err, sql)` pulls the structured `DbError` out of `oracle::Error::db_error()` so the user gets the full OCI message even when `e.to_string()` would have lost the parameter. The error includes the `[ORA-NNNNN]` code suffix and, when `DbError::offset()` is set, an `at offset N near: ...<±30 bytes>...` snippet so you can pinpoint the offending token even when OCI didn't include the identifier in the message text. Adds "Possible causes" hints for the most ambiguous codes (`ORA-00904` invalid identifier, `ORA-00942` table/view does not exist, `ORA-01017`, `ORA-01031`, `ORA-12541`) so a missing EXECUTE / SELECT grant is immediately obvious instead of looking like a typo.

#### Scripts
- **Nested script collections** — the scripts tree now supports arbitrary directory nesting instead of the previous flat layout. Typing `child/` while the cursor is inside `parent/` actually creates and displays `parent/child/`, with scripts inside visible when both ancestors are expanded. Storage gains a recursive `walk_collection` walker; `ScriptNode::Collection` now carries a full relative path; `visible_scripts()` checks ancestor expansion. Indent in the renderer is computed from path depth so deep trees stay readable. `create_collection` switched to `create_dir_all`; `delete_collection` switched to `remove_dir_all` so deleting a non-empty parent works in one shot.

### Fixed
- **Empty explorer `o`/`i` create** — when the connections explorer was completely empty (no groups, no connections), pressing `o`/`i` did nothing because `handle_sidebar` short-circuited with `Action::None` on a zero-length tree before reaching the `create_new` handler. The empty-tree path now matches the binding directly and starts the inline group create flow, so users can bootstrap their very first collection from zero state. Works in both the regular sidebar and the floating oil navigator.
- **Oracle "could not use 'execute' for select"** — the SQL classifier was checking `trimmed.starts_with("SELECT")` which is fooled by leading SQL comments (e.g. a `-- note` line above the query). A query like `-- comment\nSELECT * FROM t` was routed to the DDL/DML branch and the Oracle driver rejected it with the cryptic "could not use 'execute' for select statements". New `is_row_producing_query` helper in `core/adapter.rs` skips leading whitespace, line comments (`-- ...`), and block comments (`/* ... */`, with nesting) before checking for `SELECT`/`WITH`. Wired into all three drivers' `execute()` and `execute_streaming()`.
- **PL/SQL `END;` stripped before execute** — `spawn_execute_query_at` ran `trim_end_matches(';')` on every statement to drop the trailing semicolon the drivers don't accept. PL/SQL anonymous blocks (`DECLARE...BEGIN...END;`) are the opposite: the final `END;` REQUIRES the semicolon, and stripping it leaves an incomplete block that Oracle rejects with PLS-00103. New `is_plsql_block` detects DECLARE/BEGIN starts (after skipping comments/whitespace) and skips the trim for those.
- **Path expansion in import/export** — `~/foo.dbx` was being passed verbatim to `fs::read`, which can't expand `~`. Both the Tab-completion in the dialog and the submit path now route through `expand_user_path` which resolves `~` and `~/...` against `$HOME`. Import errors now show the resolved absolute path and distinguish "file not found" from other read failures, so you can immediately tell when the typed path was wrong.
- **Loading placeholder unreachable** — `j` from the editor and `l` from the scripts panel both required `result_tabs` to be non-empty or `query_result` to be set, so the loading placeholder was visible but unreachable. `has_bottom` now also counts `tab.streaming` as a valid reason to navigate into Results, and the placeholder border highlights when focused so the cancel-by-close UX is discoverable.
- **`QueryFailed` left streaming flags dirty** — a failed query pushed the error result_tab but never cleared `tab.streaming`, `tab.streaming_since`, or `tab.streaming_abort`, leaving the tab in an inconsistent state where the next close was interpreted as cancelling a non-existent stream. QueryFailed now fully cleans up the streaming markers (and `pending_query`).
- **`<leader>Enter` no longer replaces, `<leader>/` no longer creates a new tab** — regression introduced when the loading placeholder started setting `tab.streaming = true` at Execute dispatch. The QueryBatch handler used `tab.streaming` to distinguish "stream in progress → append" from "fresh execution → replace", so with streaming set upfront the handler fell into append on every new query. New `first_batch_pending: bool` set at dispatch and cleared on the first batch is the source of truth for "fresh query".
- **Sidebar cursor jumps to row 0 after deleting an empty group/connection** — was already fixed in 0.3.0 for non-empty deletes, this round closes the empty-tree case.
- **Status bar shows multi-line connection errors collapsed** — friendly connection errors are multi-line, but the status bar can only show one line. The status bar message is now trimmed to just the headline; the detail and hint lines stay in the connection dialog.

### Changed
- **Connection dialog Tab order** — visual order is `Name → Type → Group → Host → Port → Database → Username → Password` (was raw struct order). `CONN_FIELD_VISUAL_ORDER` owns the mapping; the underlying field indices are unchanged so all event handlers still match.
- **`tab.streaming` and `tab.streaming_since` are set at Execute dispatch** instead of on the first batch arrival, so the loading placeholder appears immediately. The previous role of `tab.streaming` ("stream is in progress") is now split: `streaming` covers the whole dispatch-to-done window, and `first_batch_pending` is the new "no batches yet" signal used by the QueryBatch handler.

## v0.3.0 — 2026-04-07

### Added

#### Configurable keybindings
- **`keybindings.toml`** — full configurable keybinding system. Default config lives in `src/keybindings/defaults.rs`; user overrides go in `~/.config/dbtui/keybindings.toml` (XDG-style). Per-action bindings, multiple keys per action, vim-style notation (`<C-h>`, `<leader>`).
- **CLI flags** — `dbtui --print-keybindings` dumps the current resolved bindings as TOML to stdout; `dbtui --dump-keybindings` writes the defaults to the config path so you can start from a working file.
- **Every handler dispatches via `bindings.matches(Context::X, "action", &key)`** — `events/mod.rs` (global), `leader.rs` (root + every sub-menu), `sidebar.rs`, `scripts.rs`, `grid.rs`, `oil.rs`, and the menu overlays. Stateful chords (`dd`, `yy`) re-check bindings on the second press, so rebinding the first key also rebinds the chord completion.
- **Help surfaces read the live config** — `widgets/help.rs` and the leader popup (`layout/overlays.rs::render_leader_help`) resolve every label via `state.bindings.primary_key(...)`, so the `?` screen and the leader hint always reflect the user's actual keys.
- **`KEYBINDINGS.md`** — user-facing documentation: every context, every action, override examples, and the full default table.

#### SQL completion
- **`TABLE(pkg.fn()) tb` pseudo-column completion** — typing `tb.<cursor>` after a `FROM TABLE(schema.pkg.func(...)) tb` ref now suggests the attributes of the Oracle object type the function returns. Resolved on demand by walking `ALL_ARGUMENTS` (position=0, data_level=1) → `ALL_TYPE_ATTRS`, cached per `(schema, package, function)` in the `MetadataIndex`. New `get_function_return_columns` adapter method (Oracle implements it; MySQL/PG return `Ok(vec![])`).
- **Package member completion** — `schema.pkg.<cursor>` and `pkg.<cursor>` now suggest the functions and procedures inside the package. The first time the user touches an unloaded package, completion fires `LoadPackageMembers` through the existing async pipeline and re-fires the popup when the load returns; no need to expand the package in the explorer first. Accepting a Package suggestion appends `.` so the next suggestion can chain.
- **User-defined functions in FROM** — Oracle's top-level functions are suggested as candidates in FROM, alongside tables and views.
- **Oracle pseudo-table functions** — `TABLE(...)`, `THE(...)`, `XMLTABLE(...)`, `JSON_TABLE(...)` are surfaced in FROM-context completion.
- **Live diagnostics in Insert mode** — sqlparser/semantic checks now re-run while typing in Insert mode with a 150ms throttle, instead of only on Insert→Normal transition.
- **`Ctrl+Space` forced completion** — properly forwards `cache_action` so the on-demand cache load fires even on a manually triggered popup.

#### Explorer / Oil
- **Inline create / rename** for groups and connections — oil-style buffer entry replacing the old modal flow. `i`/`o` on a collapsed group starts an inline create; `r` on a connection starts an inline rename.
- **`r` is context-aware** — on a category, reload the children of that category; on a schema, reload every expanded category beneath it; on a leaf, open the rename modal.
- **`F` filter inside oil**, layered Esc handling so an inner rename/search input is cancelled before oil itself closes, and `Ctrl+S` opens the selected object in a new vertical group.
- **Topbar tracks the sidebar cursor too** — connection name / DB type now reflect either the active tab's connection or the connection the sidebar cursor is hovering, whichever is more relevant.

#### Other
- **`-` toggles the floating oil navigator** (was `<leader>+E`). Matches oil.nvim muscle memory; pressing `-` again closes it. Configurable via `[global] toggle_oil_navigator`.

### Fixed
- **PL/SQL diagnostic false positive** — blank lines inside a `DECLARE .. BEGIN .. END;` block were splitting the block in two, so the second half (e.g. a bare `SCHEMA.PKG.PROC(...)` call) reached sqlparser as a stray statement and tripped "Expected an SQL statement". The block splitter now tracks BEGIN/END nesting (skipping the control-flow enders `END IF/LOOP/CASE/WHILE/FOR`) and treats blanks inside the span as non-blank.
- **Keybinding case-sensitivity** — `KeyBinding::matches` was case-folding Char comparisons, so the binding `Char('e')` matched a runtime event of `Char('E')`. Pressing `<leader>E` was firing `toggle_sidebar` ("e") instead of `toggle_oil_navigator` ("E"). Char comparisons are now case-sensitive; the SHIFT modifier is still tolerated for terminals that don't report it on uppercase chars.
- **`<leader>f` / `<leader>q` submenu popups** — `check_leader_help_timeout` was only flagging `help_visible` for `b/w/s/leader_pending`. Adding `f_pending` and `q_pending` so the submenu popup actually appears.
- **`Ctrl+S` in oil** — the global Ctrl+S intercept (save script / compile to DB) was firing before oil's handler when the user opened the navigator from a script tab, so the open-in-split shortcut never triggered. Gated the intercept on `state.oil.is_none()`.
- **Visual yank from the header row** — pressing `v` on the header and then `j` was losing the header in the final yank. Tracked via a new `grid_anchor_on_header` flag; `grid_yank` (now extracted into the pure `build_yank_text` for unit testing) prepends the column names — scoped to the selected column range — whenever the flag is set. The grid renderer also paints the header cells inside the selected column range so the user can see the header is part of the selection.
- **Export / Import dialog backgrounds** — both dialogs were rendering their `Block` directly on the editor without clearing the cells underneath, so the script/grid bled through. They now match every other modal: `Clear` + `bg(theme.dialog_bg)`.
- **`CREATE OR REPLACE TYPE` (Oracle)** — sqlparser refuses to parse it; `is_unsupported_plsql_ddl` now skips linting the family of PL/SQL DDL forms the parser can't handle, so the bogus "Expected TABLE or VIEW" error no longer fires.
- **Cursor jump on group/connection delete** — sidebar cursor stayed near the deletion site instead of jumping back to row 0.
- **Per-tab streaming spinner** — `AppMessage::Error` now clears `streaming_since` so a failed DDL fetch stops spinning forever.
- **DBMS_METADATA "ORA-31603" friendly error** — the Oracle adapter surfaces a clearer message when `DBMS_METADATA` can't read DDL for the current user.
- **`TABLE(...)` alias capture** — the tokenizer now skips the parenthesised call before scanning for the alias, so `FROM TABLE(pkg.fn()) tb` actually captures `tb`.
- **`scripts pending-d/y` chord** — both presses of `dd`/`yy` go through `bindings.matches`, so users who rebind the first key also rebind the second.

### Changed
- **`-` replaces `<leader>+E`** for the floating navigator (see Added).
- **Numeric panel jumps removed** — `1`/`2`/`3`/`4` (with or without Ctrl) collided with vim count prefixes (`d3j`); spatial nav via `Ctrl+h/j/k/l` covers the same use case.
- **Bracket nav cycles result tabs on scripts** — `[`/`]` cycles the result tabs on script tabs (was sub-views), unifying with the sub-view bracket convention on table tabs.
- **`r` in the grid refreshes table data** — was inert/inconsistent. `{`/`}` no longer cycle result tabs (use `[`/`]`).
- **Help & leader popup labels** read from `state.bindings.primary_key(...)` instead of hardcoded strings.

---

## v0.2.3 — 2026-04-07

### Added
- **Oil floating navigator** — `<leader>+E` toggles a centered transparent dual-pane modal (Explorer + Scripts) with rounded borders, inspired by oil.nvim/telescope.nvim. Auto-closes when opening a tab. `Ctrl+h/l` switches panes.
- **Sidebar toggle** — `<leader>+e` shows/hides the sidebar+scripts panel. Default: hidden on startup, full-width editor.
- **Tab groups (vertical split)** — `<leader>+|` creates a vertical split (max 2 groups). Each group has its own tab bar and active tab. `Tab`/`S-Tab` cycle within the focused group only.
  - `<leader>+m` moves the active tab to the other group
  - `<leader>+w+d` closes the focused group: kills the active tab and merges the rest into the surviving group; falls back to close-tab when no split
  - `Ctrl+h`/`Ctrl+l` navigate between groups (within-tab Results↔QueryView takes priority)
  - Each group is independent — tabs are cloned with new TabIds so editing/results stay separate
  - `Ctrl+S` from oil opens the selected object in a new vertical group
- **Navigable Properties view** — properties now render in the data grid with `j/k/h/l`, visual mode (`v`), and copy (`y`)
- **Selectable header row** — `k` from the first data row moves the cursor onto the column names; `g` jumps to header, `G` to last row; `y` on header copies column names
- **`<leader>+f` file sub-menu** — `<leader>+f+e` export connections, `<leader>+f+i` import connections (moved from `<leader>+e`/`<leader>+i`)
- **`<leader>+q+q` quit** — `q` no longer quits from the sidebar; quitting now goes through `<leader>+q+q` with unsaved-changes confirmation
- **Dynamic topbar** — connection name, DB type, schema, and status now reflect the active tab's connection (was hardcoded to the first connection)
- **Diagnostic severity & colors** — errors (red), warnings (yellow), info (blue), hints (dim) with distinct underline colors and status bar prefixes (`[error]`, `[warning]`, `[lint]`)
- **Diagnostic gutter signs** — `✘` (error) and `⚠` (warning) rendered left of line numbers via vimltui `DiagnosticSign`; separate from diff signs (`│`/`▲`/`▼`) on the right
- **Diagnostic navigation** — `Ctrl+]` / `Ctrl+[` next/previous error (wraps around), syncs with diagnostic list cursor
- **Diagnostic tooltip** — `K` in Normal mode shows floating popup with full message and source label; any key dismisses
- **Diagnostic list panel** — `Spc-x` toggles bottom panel listing all diagnostics with `✘`/`⚠` icons, `row:col`, and messages; `j`/`k` navigate, `Enter` jumps to location
- **"Did you mean?" suggestions** — unknown tables/schemas fuzzy-matched against MetadataIndex: `Unknown table 'oder' — did you mean 'orders'?`
- **Column qualifier validation** — `ord.column` now errors when alias `ord` doesn't exist in scope (e.g., table aliased as `or2`)

### Fixed
- **Per-connection state isolation** — three bugs caused cross-connection contamination with multiple open connections:
  - Sidebar lock icons used a global `current_schema` instead of per-connection metadata indexes
  - Warm-up loading resolved the adapter from sidebar cursor instead of by connection name
  - `insert_leaves` matched the first Category by `(schema, kind)` regardless of connection, inserting objects under the wrong connection when both had overlapping schema names
- **Oracle TIMESTAMP / DATE display** — replaced raw String decode with `oracle_col_to_string` handling `Timestamp` (with nanoseconds), `IntervalDS/YM`, RAW/BLOB as hex
- **MySQL TIMESTAMP / DATETIME display** — type-aware decoder: chrono first for date types with binary protocol byte fallback, plus `DECIMAL`, `JSON`, `BIT`, `BLOB`/`BINARY` as hex, `YEAR`
- **PostgreSQL types** — added `JSON`/`JSONB` via serde_json and `INTERVAL` formatting (months/days/HH:MM:SS)
- **Editor focus after closing last result tab** — `sub_focus` now resets to `Editor` so you can type immediately without pressing Escape
- **Data grid columns** — last column no longer stretches to fill the row; every column takes only the width it needs
- **`:q` / `:q!` in editor** — closes the tab instead of quitting the app
- **Auto-alias avoids SQL reserved words** — `orders` no longer generates `or` (reserved); 70+ reserved words checked
- **Gutter width calculation** — completion popup, diagnostic underlines, and hover tooltip all account for diagnostic column width (+2 chars)
- **Shared tokenization** — lint passes reuse tokens from a single `tokenize_sql()` call instead of 3 separate ones

### Changed
- **Keybinding overhaul**
  - `]` / `[` direct sub-view switching (no pending bracket state)
  - `Ctrl+]` / `Ctrl+[` for diagnostic navigation (was `]d` / `[d`)
  - `<leader>+E` (Shift+E) opens oil navigator
  - `<leader>+|` creates vertical split
  - `<leader>+m` moves tab between groups
  - `<leader>+b+d` closes the active tab — also closes a result tab when `sub_focus` is on Results
  - `<leader>+w+d` closes the focused tab group (was close result tab)
  - `<leader>+f+e` / `<leader>+f+i` for export/import
  - `<leader>+q+q` to quit
  - Old `<leader>+e` (export) and `<leader>+i` (import) removed
- **Transparent modals** — `dialog_bg` set to `Color::Reset` across all 6 themes; help, leader popup, and connection/import/export dialogs now show the terminal wallpaper through the modal background
- **Default startup focus** — `Focus::TabContent` (was Sidebar) since the sidebar is hidden by default
- **`sub_focus` is global** — when split is active, only the focused group highlights its editor/results panel; the unfocused group renders all panels as inactive
- **vimltui** — bumped to 0.1.9 (from crates.io); uses new `DiagnosticSign` enum (left of number) separate from `GutterSign` (right of number)
- **Diagnostic pipeline** — `check_local()` shares tokenization across lint passes via `check_lint_with_tokens()`

---

## v0.2.2 — 2026-04-06

### Added
- **R2: AppState decomposition** — 51 flat fields split into 6 sub-structs: `ConnectionState`, `SidebarState`, `DialogState`, `LeaderState`, `ScriptsState`, `EngineState` (15 root fields remaining)
- **UPDATE/DELETE completion** — new `AfterUpdateTable`, `AfterDeleteTable` cursor contexts with SET/WHERE keyword suggestions and on-demand column loading for SET clause
- **Paren-depth tracking** in backward keyword scanner — `ORDER BY` inside `OVER()` no longer contaminates outer SELECT/FROM context
- **Window function completion** — `OVER`, `PARTITION BY`, `ORDER BY` inside `OVER()` correctly scoped with columns, aliases, and keywords
- **Auto-alias on table accept** — accepting a table in FROM/JOIN appends a 2-3 char alias derived from the name (`orders` → `or`, `customer_orders` → `co`), conflict-aware with existing aliases
- **130+ dialect-specific functions** — Oracle (53), PostgreSQL (44), MySQL (38): window functions (`LEAD`, `LAG`, `FIRST_VALUE`...), date (`DATE_TRUNC`, `ADD_MONTHS`...), JSON, regex, string, aggregate
- **Toggle comment** — `gcc` toggles `--` on current line (Normal), `gc` toggles block comment on selection (Visual); works in scripts, packages, functions, procedures
- **Auto-pair brackets** — typing `(`, `[`, `{`, `'` auto-inserts the closing pair with cursor between
- **Smart modified detection** — hash-based content comparison clears `(*)` indicator when edits revert to saved state (via undo or manual re-edit)
- **Clipboard: OSC 52** — universal terminal clipboard support via escape sequence (works in kitty, alacritty, WezTerm, tmux, SSH)
- **Confirm delete connection** — `y/n` dialog before removing a connection (from sidebar `dd` or connection menu)
- **Leader+s SQL snippets** — `+u` UPDATE, `+d` DELETE, `+p` CALL/EXEC procedure, `+f` SELECT function, `+t` CREATE TABLE — all dialect-aware (Oracle/MySQL/PG) with `$` cursor positioning
- **Star completion** — completion triggers after `*` in SELECT for column replacement
- **rows_affected feedback** — all 3 drivers report affected row count: `"Statement executed successfully (N row(s) affected)"`

### Fixed
- **Query scope** — `query_block_at_cursor` rewritten; blank lines at buffer start no longer break SQL block detection
- **DECIMAL display in MySQL** — raw byte decode via `sqlx::Decode` bypasses type-checking; values like `DECIMAL`, `NUMERIC` now display correctly instead of NULL
- **DATETIME/DATE/TIME display** — `chrono` integration for MySQL and PostgreSQL; dates render as `2024-02-10 16:20:00`
- **DML persistence** — explicit `BEGIN`/`COMMIT` transactions in MySQL and PostgreSQL `execute`/`execute_streaming`
- **Completion popup z-index** — renders above diagnostic underlines instead of behind
- **Script save in collections** — uses `file_path` (with collection prefix) instead of display name; saves to correct subdirectory
- **Diagnostic false positives** — skip semantic pass when `MetadataIndex` has no schemas loaded

### Changed
- **Scripts panel rename** — `r` instead of `cw`; insert position now within current collection instead of at end
- **Remove `n` shortcut** for new script — use `i` in scripts panel only
- **Keyword suggestions expanded** — `OVER`, `PARTITION`, `BY`, `ASC`, `DESC`, `AS`, `DISTINCT` added to Predicate and OrderGroupBy contexts
- **vimltui 0.1.8** — adds `ToggleComment` / `ToggleBlockComment` editor actions, `pending_gc` state

---

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
- **Per-connection MetadataIndex** — each connection now has its own metadata index; switching between scripts connected to different databases (e.g., Oracle and MySQL) shows the correct tables/views for each
- **Auto-load schemas on script connection** — assigning a connection to a script via `leader+c` now triggers automatic schema/table loading if metadata wasn't loaded yet (no need to expand sidebar first)
- **Streaming query cancellation** — closing a result tab or tab now aborts the streaming task (both outer relay and inner DB query), preventing background resource consumption
- **Filter persistence on connection rename** — object filter keys are now migrated when renaming a connection (fixes lost filters after rename)
- **PL/SQL completion context** — PL/SQL block keywords (`IF`, `ELSIF`, `THEN`, `BEGIN`, `LOOP`, etc.) now act as context boundaries, preventing stale `SELECT`/`FROM` context from leaking into PL/SQL code
- **CASE parentheses** — `CASE` removed from function list (no longer inserts `()` on accept)
- **NOT EXISTS parentheses** — added to keywords that auto-insert `()`

### Changed
- Completion engine replaced: old heuristic `starts_with` matching → new fuzzy matching with scoring tiers
- Diagnostics engine replaced: old single-pass → new 3-pass pipeline (syntax + semantic + lint)
- Tokenizer migrated from `src/ui/sql_tokens.rs` to `src/sql_engine/tokenizer.rs` (UI module re-exports)
- Legacy `ui/completion.rs` reduced from 1,169 to 148 lines (only UI types kept)
- Legacy `ui/diagnostics.rs` reduced from 386 to 11 lines (only Diagnostic struct kept)

### Refactored
- **events.rs** (4,576 lines) → 7 modules: `editor`, `grid`, `leader`, `overlays`, `scripts`, `sidebar`, `mod`
- **app.rs** (4,534 lines) → 5 modules: `messages`, `spawns`, `connections`, `persistence`, `mod`
- **layout.rs** (2,392 lines) ��� 3 modules: `overlays`, `tabs`, `mod`
- Metadata loading handlers consolidated via generic `handle_objects_loaded()`
- Loading state resets consolidated via `finish_loading()` helper (~20 occurrences)

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
