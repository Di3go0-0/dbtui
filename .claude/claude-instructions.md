# Project: dbtui - Professional Database TUI (Rust + Ratatui)

## 🔥 Core Principles

- Vim-first navigation (hjkl everywhere)
- TUI-first, Neovim-second (wrapper, not dependency)
- Core must be UI-agnostic
- Database-agnostic via adapters
- Must scale to enterprise databases (Oracle-level complexity)

---

## 🧱 Architecture (STRICT)

The project is divided into 3 layers:

### 1. Core (src/core)
- Connection management
- Query execution
- Metadata abstraction
- Global session state

### 2. Drivers (src/drivers)
- Each DB is implemented as an adapter
- Must implement a common trait

### 3. UI (src/ui)
- Built with Ratatui
- Pure rendering + input handling
- No DB logic

---

## 🧩 Database Adapter Contract (CRITICAL)

All databases MUST implement this trait:

```rust
pub trait DatabaseAdapter {
    fn name(&self) -> &str;

    fn get_schemas(&self) -> Result<Vec<Schema>>;
    fn get_tables(&self, schema: &str) -> Result<Vec<Table>>;

    // Optional features (must return empty if not supported)
    fn get_packages(&self, schema: &str) -> Result<Vec<Package>>;
    fn get_views(&self, schema: &str) -> Result<Vec<View>>;

    fn execute(&self, query: &str) -> Result<QueryResult>;
}
```

## Adapter Rules (VERY IMPORTANT)

NEVER assume all DBs support schemas
NEVER assume packages exist
NEVER panic or unwrap
If a feature is not supported → return Ok(vec![])
Oracle is considered the "superset" model
Adapters MUST normalize metadata into common structs
🧠 Core Models (NORMALIZED)

All DBs must map to these structures:

🧠 Core Models (NORMALIZED)

All DBs must map to these structures:

```rust
  pub struct Schema {
      pub name: String,
  }

  pub struct Table {
      pub name: String,
      pub schema: String,
  }

  pub struct View {
      pub name: String,
      pub schema: String,
  }

  pub struct Package {
      pub name: String,
      pub schema: String,
      pub has_body: bool,
  }

  pub struct Function {
      pub name: String,
      pub schema: String,
  }

  pub struct QueryResult {
      pub columns: Vec<String>,
      pub rows: Vec<Vec<String>>,
  }
```



## Rules:
  All queries execute against current session
  UI must always reflect session state
  Switching schema updates context globally
  🔄 Async & Performance Model
  DB calls MUST be async or run in background threads
  UI must NEVER block
  Large queries must support:
  pagination OR
  streaming
  🖥️ UI Architecture (Ratatui)
  Root Layout

  Vertical split:

  Top Bar (connection info)
  Main Content
  Status Bar
  Main Content

  Horizontal split:

  Left Sidebar (20%)
  Center Panel (80%)

## Sidebar Tree Structure

```
Connection
 └── Schema
      ├── Tables
      ├── Views
      ├── Packages
      ├── Functions
```

Rules:
Lazy loading (do NOT load everything at once)
Expand on demand (Enter)

🧾 Center Panel (Tabs)

Tabs must include:

1. Data
Table/grid visualization
Scrollable
Supports filtering (SQL WHERE expression)
Supports sorting (ASC/DESC)
2. Properties
Key-value metadata
Based on selected object
3. Package (Oracle only)
Split view:
Declaration
Body
⌨️ Keybindings (STRICT - Vim Style)
h/j/k/l → navigation
Ctrl + h/j/k/l → switch panels
Enter → expand/select
/ → filter/search
q → go back / close panel
g / G → top/bottom
: → command mode (future)
🧠 Query Execution Model
Query editor is decoupled from UI state
Uses global session connection
Supports:
multiple queries (future)
execution feedback (success/error)
🧪 Neovim Integration (IMPORTANT)

DO NOT reimplement Vim.

Allowed approaches:

Embed Neovim via RPC (preferred)
Spawn Neovim as subprocess
Use Neovim terminal buffer

The system MUST work without Neovim.

🚫 Hard Constraints
No DB logic inside UI
No UI logic inside drivers
No blocking operations in UI thread
No assumptions about DB capabilities
No duplicated logic between adapters
🚀 MVP Scope
Phase 1
Single DB (Postgres or MySQL)
Schema + tables explorer
Execute queries
Display results
Phase 2
Multi-connection
Filtering (WHERE)
Sorting
Phase 3
Oracle support
Packages
Dependencies
Advanced metadata



---

# ⚠️ Mejora tu `task.md` (pequeño upgrade)

Te lo dejo más accionable:

# Current Tasks

## MVP 1 (FOUNDATION)
- [ ] Define DatabaseAdapter trait
- [ ] Define core models (Schema, Table, QueryResult)
- [ ] Implement Postgres adapter
- [ ] Implement basic connection manager
- [ ] Build base TUI layout (Ratatui)
- [ ] Sidebar navigation (static data first)
- [ ] Execute simple query (SELECT 1)

---

## MVP 2 (USABLE)
- [ ] Load real schemas and tables
- [ ] Table data viewer (grid)
- [ ] Filtering (WHERE input)
- [ ] Sorting (ASC/DESC)
- [ ] Multi-connection support

---

## MVP 3 (ADVANCED)
- [ ] Oracle adapter
- [ ] Packages visualization
- [ ] Declaration / Body viewer
- [ ] Dependencies graph
- [ ] Properties panel
