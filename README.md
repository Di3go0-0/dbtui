# dbtui

A terminal-based database client with Vim-style navigation. Browse schemas, write SQL, execute queries, and explore data — all from the terminal.

Built with Rust, Ratatui, and Tokio. Vim editing powered by [vimltui](https://github.com/Di3go0-0/vimltui) ([crates.io](https://crates.io/crates/vimltui)).

## Features

- **Multi-database support** — Oracle, PostgreSQL, MySQL, SQL Server
- **Vim editing** — Powered by [vimltui](https://github.com/Di3go0-0/vimltui): full modal editing (Normal, Insert, Visual), operator+motion composition, f/F/t/T, dot repeat, search highlighting, registers, system clipboard
- **Schema explorer** — Browse connections, schemas, tables, views, packages, functions, and procedures
- **SQL editor** — Syntax highlighting, relative line numbers, search (`/`), and command mode (`:`)
- **Smart query execution** — Execute the query block at cursor (`<Space>Enter`) or visual selection
- **Result tabs** — Multiple result sets per script, switch with `{`/`}`
- **SQL completion (CMP)** — Context-aware autocompletion: tables after FROM, columns after SELECT/WHERE, Oracle schema hierarchy, alias resolution
- **SQL diagnostics (LCP)** — Real-time syntax, reference and lint checks underlined on the token at fault, with a gutter sign, a list panel (`<Space>x`) and `Ctrl+]` / `Ctrl+[` to jump between them
- **Server error marking** — When a statement or a compile fails, the line and column the database reported are marked in the editor; `K` shows the message
- **Bind variables** — `:variableName` prompts with persistent value memory across sessions
- **Error display** — Split pane showing error message and failed SQL side by side, with real line numbers
- **Data grid** — Cell-level navigation, visual selection (`v`), copy to clipboard (`y`)
- **Horizontal scroll** — Navigate wide tables with many columns
- **Per-tab connections** — Each tab tracks its own connection, auto-reconnect on script open
- **Theme system** — 6 built-in themes with transparent backgrounds
- **Leader key menu** — `<Space>` opens a command palette with all available actions
- **Encrypted export** — Connections, scripts and settings export to a single password-protected `.dbx` bundle (ChaCha20Poly1305 + Argon2)

## Installation

### With Nix (recommended)

This repository ships a `flake.nix`, so you get a fully reproducible build and dev shell — no need to install Rust or system libraries manually.

#### Run directly from GitHub (no clone needed)

You can build and run dbtui straight from the repository URL:

```bash
nix run github:Di3go0-0/dbtui
```

Nix fetches the source, builds the package, and launches the binary in one step. This is the quickest way to try dbtui out.

> `nix run` requires flakes and the `nix-command` feature to be enabled. This is the default on NixOS 24.05+ and on Determinate Nix. On vanilla Nix 2.x, add `experimental-features = nix-command flakes` to your `nix.conf`.

#### Build from a local clone

```bash
git clone https://github.com/Di3go0-0/dbtui.git
cd dbtui

# Build and run in one step
nix run

# Or build the binary into ./result
nix build
./result/bin/dbtui
```

Drop into a development shell with the full Rust toolchain (`cargo`, `rustc`, `rust-analyzer`, `clippy`) and all required system libraries already wired up:

```bash
nix develop
cargo run
```

Nix handles the C compiler, OpenSSL, `libaio` (for the Oracle driver), and the X11/Wayland libraries needed for clipboard support. You still need `wl-copy`, `xclip`, or `xsel` on `PATH` at runtime — but the dev shell provides them.

### From source with Cargo

```bash
git clone https://github.com/Di3go0-0/dbtui.git
cd dbtui
cargo build --release
```

The binary will be at `target/release/dbtui`.

### Dependencies

- Rust 2024 edition
- A C compiler — the Oracle driver builds a vendored copy of ODPI-C
- For clipboard support: `wl-copy` (Wayland), `xclip`, or `xsel`
- To *connect* to Oracle: the Oracle Instant Client libraries, which are loaded
  at runtime. They are not needed to build or run dbtui — only to open an
  Oracle connection. The other drivers work without them.

## Quick Start

```bash
# Run dbtui (after `cargo build --release`)
./target/release/dbtui

# Or with Nix, without building anything yourself
nix run

# Or with a PostgreSQL connection via environment variable
DBTUI_POSTGRES_URL="postgres://user:pass@localhost/db" ./target/release/dbtui
```

Press `a` to add a new connection, or `?` for help.

## Keybindings

### Global

| Key | Action |
|-----|--------|
| `q` | Quit (warns if unsaved changes) |
| `:q!` | Force quit |
| `?` | Help |
| `a` | Add connection (from sidebar) |
| `F` | Filter schemas/objects |
| `[` / `]` | Previous/next tab |
| `Ctrl+h/j/k/l` | Navigate between panels |
| `Ctrl+arrows` | Navigate between panels |

### Leader Commands (`<Space>`)

| Sequence | Action |
|----------|--------|
| `<Space>Enter` | Execute query at cursor |
| `<Space>/` | Execute query in new result tab |
| `<Space>c` | Pick connection for script |
| `<Space>t` | Pick theme |
| `<Space>bd` | Close tab |
| `<Space>wd` | Close result tab |
| `<Space><Space>s` | Compile to database |

### SQL Completion (Insert mode)

| Key | Action |
|-----|--------|
| `Ctrl+Space` | Open/refresh completion popup |
| `Ctrl+N` / `Ctrl+P` | Next/previous suggestion |
| `Ctrl+Y` / `Enter` | Accept suggestion |
| `Esc` | Close popup |

### Editor (Vim)

| Key | Action |
|-----|--------|
| `i` / `a` / `o` / `O` | Enter Insert mode |
| `Esc` | Return to Normal mode |
| `v` / `V` / `Ctrl+v` | Visual mode (char/line/block) |
| `h/j/k/l` | Movement |
| `w` / `b` / `e` | Word motions |
| `gg` / `G` | Top/bottom of file |
| `d` / `y` / `c` | Delete/yank/change with motions |
| `p` / `P` | Paste from system clipboard |
| `/` / `?` | Search forward/backward |
| `n` / `N` | Next/previous match |
| `u` / `Ctrl+r` | Undo/redo |
| `:w` | Save |
| `:q` | Close buffer |
| `:wq` | Save and close |
| `:{number}` | Jump to line |
| `Ctrl+d` / `Ctrl+u` | Half-page scroll |

### Data Grid

| Key | Action |
|-----|--------|
| `h/j/k/l` | Navigate cells |
| `e` / `b` | Next/previous cell (wraps rows) |
| `v` | Toggle visual selection |
| `y` | Copy selection (or row) to clipboard |
| `{` / `}` | Switch result tabs |
| `g` / `G` | First/last row |
| `Esc` | Exit grid / exit visual mode |

### Diagnostics

| Key | Action |
|-----|--------|
| `Ctrl+]` / `Ctrl+[` | Jump to next/previous diagnostic |
| `K` | Show the diagnostic on the current line |
| `<Space>x` | Toggle the diagnostics list |

### Connection Dialog

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Next/previous field |
| `Ctrl+t` | Cycle database type |
| `Ctrl+g` | Cycle group |
| `Ctrl+p` | Show/hide the password |
| `Ctrl+u` | Clear the current field |
| `Ctrl+s` | Save without connecting |
| `Enter` | Connect |

Fields accept pasted text. A value longer than the field scrolls to keep the
cursor in view, and the password row shows its character count once it no
longer fits, so a long pasted secret can be checked without revealing it.

### Scripts Panel

| Key | Action |
|-----|--------|
| `Enter` | Open script |
| `n` | New script |
| `d` | Delete script |
| `D` | Duplicate script |
| `r` | Rename script |

## Supported Databases

| Database | Schemas | Tables | Views | Packages | Functions | Procedures | Query Execution |
|----------|---------|--------|-------|----------|-----------|------------|-----------------|
| Oracle | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| PostgreSQL | Yes | Yes | Yes | No | Yes | Yes | Yes |
| MySQL | Yes | Yes | Yes | No | Yes | Yes | Yes |
| SQL Server | Yes | Yes | Yes | No | Yes | Yes | Yes |

SQL Server is the one engine with a catalog level above schemas, so its tree is
`Connection → Databases → Schema → Category`. Leave the database field empty in
the connection dialog to browse every database the login can open.

## Themes

Switch themes with `<Space>t`:

- **Tokyo Night** — Blue and purple tones
- **Catppuccin** — Pastel lavender palette
- **Dracula** — Purple and pink accents
- **Nord** — Arctic blue tones
- **Gruvbox** — Warm brown and orange
- **Default** — Classic blue

All themes use transparent backgrounds — your terminal's background shows through.

## Architecture

```
src/
  core/       — Database adapter trait, models, errors, storage, encryption
  drivers/    — Per-database implementations (Oracle, PostgreSQL, MySQL, SQL Server)
  ui/         — Terminal UI (Ratatui rendering, input handling, widgets)
    completion.rs — Context-aware SQL autocompletion engine
    diagnostics.rs — SQL validation against database metadata
    sql_tokens.rs  — Shared SQL tokenizer for completion and diagnostics
    widgets/  — Sidebar, data grid, status bar, dialogs
    tabs/     — Tab management and workspace state
  main.rs     — Tokio runtime, terminal setup, event loop
```

**Hard constraints:**
- No database logic in `src/ui/`
- No UI logic in `src/drivers/` or `src/core/`
- No blocking in UI thread (all DB operations async via Tokio + mpsc channels)
- No `.unwrap()` or `.expect()` in production code

## Data Storage

Configuration and scripts are stored in the XDG data directory:

```
~/.local/share/dbtui/
  connections.json   — Connection configs, passwords included (owner-only, mode 0600)
  scripts/           — SQL script files
  object_filters.json — Saved schema/object filters
  script_connections.json — Script-to-connection mappings
  bind_variables.json    — Persisted bind variable values
  theme.txt          — Selected theme
```

`connections.json` is plain JSON: it is protected by file permissions, not by
encryption. Use the encrypted export (`.dbx`) to move connections between
machines.

## License

MIT License. See [LICENSE](LICENSE) for details.
