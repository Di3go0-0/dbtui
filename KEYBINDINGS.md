# Keybindings

Every keybinding in dbtui can be customised via `~/.config/dbtui/keybindings.toml`.

## Quick start

Generate the default config in your config dir:

```bash
dbtui --dump-keybindings
```

This writes the default config to `~/.config/dbtui/keybindings.toml`. You only need to keep the keys you want to change — any action you omit falls back to the default.

To print the defaults to stdout (without touching disk):

```bash
dbtui --print-keybindings
```

## Key format

| Format             | Example                      | Description                            |
|--------------------|------------------------------|----------------------------------------|
| Single char        | `"j"`, `"?"`, `"|"`           | Regular key                            |
| Shifted (uppercase)| `"G"`, `"E"`                  | Use the uppercase letter directly      |
| Shifted (explicit) | `"Shift+g"`                   | Equivalent to `"G"`                    |
| Ctrl               | `"Ctrl+r"`                    | Control modifier                       |
| Alt                | `"Alt+x"`                     | Alt modifier                           |
| Combined           | `"Ctrl+Shift+e"`              | Multiple modifiers                     |
| Special            | `"Esc"`, `"Enter"`, `"Tab"`   | Named keys                             |
| Whitespace         | `"Space"`, `"BackTab"`        | Special keys with names                |
| Arrows             | `"Up"`, `"Down"`, `"Left"`    | Arrow keys                             |
| Navigation         | `"Home"`, `"End"`, `"PageUp"` | Navigation keys                        |
| Function           | `"F1"` … `"F12"`              | Function keys                          |
| Vim shorthand      | `"<C-h>"`, `"<S-Tab>"`        | Vim-style angle bracket form           |
| Leader             | `"<leader>"`                  | Equivalent to `"Space"` (dbtui leader) |
| Multiple           | `["j", "Down"]`               | Bind multiple keys to one action       |

## Contexts

Keybindings are organised by context. Each context is a TOML section. Inside a section you map an action name to a key (or list of keys).

| Section            | When it applies                                                     |
|--------------------|---------------------------------------------------------------------|
| `[global]`         | Always active. Tab navigation, sub-views, diagnostics, panel jumps. |
| `[leader]`         | First key after the leader (`Space`).                               |
| `[leader_buffer]`  | Second key after `<leader>b` (buffer / tab management).             |
| `[leader_window]`  | Second key after `<leader>w` (window / tab group management).       |
| `[leader_file]`    | Second key after `<leader>f` (file ops — export / import).          |
| `[leader_quit]`    | Second key after `<leader>q` (quit confirmation).                   |
| `[leader_snippet]` | Second key after `<leader>s` (SQL snippet templates).               |
| `[sidebar]`        | Explorer (sidebar tree). Also reused inside the oil floating nav.   |
| `[scripts]`        | Scripts panel. Also reused inside the oil floating nav.             |
| `[grid]`           | Data grid (table data, query results, properties).                  |
| `[oil]`            | Oil floating navigator chrome (Esc, pane switching, open in split). |
| `[overlay]`        | Modals (help, theme picker, connection menu, etc.).                 |

## Example: custom config

```toml
# ~/.config/dbtui/keybindings.toml
# Only override what you need.

[global]
help = ["?", "F1"]
toggle_oil_navigator = "o"

[grid]
refresh_data = "F5"

[leader_quit]
quit_app = "q"
```

Anything you don't list keeps its default. Anything you list completely replaces the default for that action — to bind multiple keys to the same action, use a list.

## Pending key sequences (leader sub-menus)

Some bindings live in nested sub-menus. The first key picks the sub-menu, the second key picks the action.

| Sequence       | Action                                       |
|----------------|----------------------------------------------|
| `<leader>b d`  | Close current tab                            |
| `<leader>w d`  | Close current tab group                      |
| `<leader>f e`  | Export connections                           |
| `<leader>f i`  | Import connections                           |
| `<leader>q q`  | Quit dbtui                                   |
| `<leader>s s`  | Insert SELECT snippet                        |
| `<leader>s u`  | Insert UPDATE snippet                        |
| `<leader>s d`  | Insert DELETE snippet                        |
| `<leader>s p`  | Insert CALL/EXEC procedure snippet           |
| `<leader>s f`  | Insert SELECT function snippet               |
| `<leader>s t`  | Insert CREATE TABLE snippet                  |

To rebind a sub-menu trigger, edit the corresponding `[leader]` action (e.g. `open_quit_submenu = "Q"`). To rebind the inner key, edit the matching `[leader_*]` section (e.g. `[leader_quit] quit_app = "x"` to make the full sequence `Space Q x`).

## Syntax errors

If your config file has a TOML syntax error or an unrecognised key string, dbtui falls back to the defaults and shows the error in the status bar on startup so you can fix it.

## Full default config

Run `dbtui --print-keybindings` to see the current defaults verbatim. The output is valid TOML and can be saved as your starting `keybindings.toml`.

The default mappings reflect dbtui's vim-style philosophy: hjkl navigation, leader-prefixed commands, bracket pairs (`[`/`]`) for sequential motion, uppercase letters for "go big" actions (`G` for bottom, `F` for filter), and `-` to toggle the floating navigator (matches oil.nvim muscle memory).

## Reference

Every action grouped by context. Defaults are shown next to the action name. Override any of them in `~/.config/dbtui/keybindings.toml`.

### `[global]` — always active

| Action                  | Default keys              | Description                                       |
|-------------------------|---------------------------|---------------------------------------------------|
| `leader`                | `Space`                   | Enter the leader chord menu                       |
| `help`                  | `?`                       | Toggle the help overlay                           |
| `next_tab`              | `Tab`                     | Cycle to next tab in the focused group            |
| `prev_tab`              | `BackTab`                 | Cycle to previous tab in the focused group        |
| `next_sub_view`         | `]`                       | Cycle to next sub-view (or next result tab)       |
| `prev_sub_view`         | `[`                       | Cycle to previous sub-view (or prev result tab)   |
| `next_diagnostic`       | `Ctrl+]`                  | Jump to next diagnostic                           |
| `prev_diagnostic`       | `Ctrl+[`                  | Jump to previous diagnostic                       |
| `navigate_left`         | `Ctrl+h`, `Left`          | Spatial focus left (panel / tab group / pane)     |
| `navigate_right`        | `Ctrl+l`, `Right`         | Spatial focus right                               |
| `navigate_down`         | `Ctrl+j`, `Down`          | Spatial focus down                                |
| `navigate_up`           | `Ctrl+k`, `Up`            | Spatial focus up                                  |
| `filter_objects`        | `F`                       | Open the per-category object filter               |
| `add_connection`        | `a`                       | Open the new-connection dialog                    |
| `toggle_oil_navigator`  | `-`                       | Toggle the floating oil navigator                 |

### `[leader]` — first key after `Space`

| Action                            | Default | Description                                          |
|-----------------------------------|---------|------------------------------------------------------|
| `toggle_sidebar`                  | `e`     | Show / hide the sidebar + scripts panel              |
| `vertical_split`                  | `\|`    | Create a vertical split (max 2 tab groups)           |
| `move_tab_to_other_group`         | `m`     | Move the active tab to the other group               |
| `open_theme_picker`               | `t`     | Open the theme picker                                |
| `open_script_connection_picker`   | `c`     | Pick which connection a script tab runs against      |
| `toggle_diagnostic_list`          | `x`     | Toggle the bottom diagnostic list panel              |
| `execute_query`                   | `Enter` | Execute the query block under the cursor             |
| `execute_query_new_tab`           | `/`     | Execute the query and put results in a new tab       |
| `open_buffer_submenu`             | `b`     | Enter the `[leader_buffer]` sub-menu                 |
| `open_window_submenu`             | `w`     | Enter the `[leader_window]` sub-menu                 |
| `open_snippet_submenu`            | `s`     | Enter the `[leader_snippet]` sub-menu                |
| `open_file_submenu`               | `f`     | Enter the `[leader_file]` sub-menu                   |
| `open_quit_submenu`               | `q`     | Enter the `[leader_quit]` sub-menu                   |
| `inline_new_connection`           | `I`     | Open the experimental oil-style inline connection editor (Proposal D) |

### `[leader_buffer]` — second key after `<leader>b`

| Action      | Default | Description                                                                       |
|-------------|---------|-----------------------------------------------------------------------------------|
| `close_tab` | `d`     | Close the active tab (or active result tab when focus is on Results)              |

### `[leader_window]` — second key after `<leader>w`

| Action        | Default | Description                                                                     |
|---------------|---------|---------------------------------------------------------------------------------|
| `close_group` | `d`     | Close the focused tab group; remaining tabs merge into the surviving group      |

### `[leader_file]` — second key after `<leader>f`

| Action                | Default | Description                       |
|-----------------------|---------|-----------------------------------|
| `export_connections`  | `e`     | Open the export-connections dialog |
| `import_connections`  | `i`     | Open the import-connections dialog |

### `[leader_quit]` — second key after `<leader>q`

| Action     | Default | Description                                                                 |
|------------|---------|-----------------------------------------------------------------------------|
| `quit_app` | `q`     | Quit dbtui (with unsaved-changes confirmation if any tab is dirty)          |

### `[leader_snippet]` — second key after `<leader>s`

| Action                  | Default | Description                                  |
|-------------------------|---------|----------------------------------------------|
| `snippet_select`        | `s`     | Insert a `SELECT` template                   |
| `snippet_update`        | `u`     | Insert an `UPDATE` template                  |
| `snippet_delete`        | `d`     | Insert a `DELETE` template                   |
| `snippet_call_proc`     | `p`     | Insert a CALL/EXEC procedure template        |
| `snippet_select_func`   | `f`     | Insert a `SELECT func()` template            |
| `snippet_create_table`  | `t`     | Insert a `CREATE TABLE` template             |

### `[sidebar]` — explorer tree (also reused inside oil)

| Action               | Default keys      | Description                                                 |
|----------------------|-------------------|-------------------------------------------------------------|
| `scroll_down`        | `j`, `Down`       | Move cursor down                                            |
| `scroll_up`          | `k`, `Up`         | Move cursor up                                              |
| `half_page_down`     | `Ctrl+d`          | Half-page down                                              |
| `half_page_up`       | `Ctrl+u`          | Half-page up                                                |
| `scroll_top`         | `g`               | Jump to top                                                 |
| `scroll_bottom`      | `G`               | Jump to bottom                                              |
| `expand_or_open`     | `l`, `Enter`      | Expand node or open the underlying object in a tab          |
| `collapse_or_parent` | `h`, `Left`       | Collapse node or jump to parent                             |
| `create_new`         | `i`, `o`          | Context-aware create (collection / connection / template)   |
| `group_menu`         | `m`               | Open the group/connection action menu                       |
| `rename_or_refresh`  | `r`               | Inline rename, or refresh the schema/category               |
| `yank_pending`       | `y`               | First press of `yy` — yank a connection                     |
| `paste`              | `p`               | Paste a yanked connection into the current group            |
| `delete_pending`     | `d`               | First press of `dd` — delete connection / object            |
| `start_search`       | `/`               | Start a fuzzy search over the visible tree                  |
| `next_match`         | `n`               | Jump to next search match (during search)                   |
| `prev_match`         | `N`               | Jump to previous search match                               |

### `[scripts]` — scripts panel (also reused inside oil)

| Action            | Default keys | Description                                                  |
|-------------------|--------------|--------------------------------------------------------------|
| `scroll_down`     | `j`, `Down`  | Move cursor down                                             |
| `scroll_up`       | `k`, `Up`    | Move cursor up                                               |
| `scroll_top`      | `g`          | Jump to top                                                  |
| `scroll_bottom`   | `G`          | Jump to bottom                                               |
| `expand_or_open`  | `l`, `Enter` | Expand collection or open the script                         |
| `create_new`      | `i`, `o`     | Inline create (use `name/` to create a folder)               |
| `rename`          | `r`          | Inline rename selected script / collection                   |
| `delete_pending`  | `d`          | First press of `dd` — delete script / collection             |
| `yank_pending`    | `y`          | First press of `yy` — yank a script for move/copy            |
| `paste`           | `p`          | Move the yanked script to the current location               |

### `[grid]` — data grid (table data, query results, properties)

| Action            | Default keys      | Description                                                       |
|-------------------|-------------------|-------------------------------------------------------------------|
| `scroll_down`     | `j`, `Down`       | Move row cursor down                                              |
| `scroll_up`       | `k`, `Up`         | Move row cursor up (or onto the header from row 0)                |
| `scroll_left`     | `h`, `Left`       | Move column cursor left                                           |
| `scroll_right`    | `l`, `Right`      | Move column cursor right                                          |
| `next_cell`       | `e`               | Next cell, wrapping across rows                                   |
| `prev_cell`       | `b`               | Previous cell, wrapping across rows                               |
| `scroll_top`      | `g`               | Jump to header / first row                                        |
| `scroll_bottom`   | `G`               | Jump to last row                                                  |
| `half_page_down`  | `Ctrl+d`          | Half-page down                                                    |
| `half_page_up`    | `Ctrl+u`          | Half-page up                                                      |
| `toggle_visual`   | `v`               | Toggle visual selection (preserves the header anchor)             |
| `yank`            | `y`               | Copy current cell, row, or selection to the system clipboard      |
| `refresh_data`    | `r`               | Re-fetch table data, or re-execute the query that produced a script result tab |
| `toggle_auto_refresh` | `R`           | Cycle auto-refresh interval on a script result tab: off → 2s → 5s → 10s → 30s → off |
| `edit_cell`       | `i`               | Enter inline cell-edit mode (table tabs only)                     |
| `new_row`         | `o`               | Insert a new row below the cursor (table tabs only)               |
| `delete_pending`  | `d`               | First press of `dd` — mark row for deletion (table tabs)          |
| `undo_changes`    | `u`               | Discard all pending edits and reload                              |
| `save_changes`    | `Ctrl+s`          | Save pending grid changes back to the database                    |
| `exit_grid`       | `Esc`             | Exit visual mode, or return focus to the editor                   |

### `[oil]` — floating navigator chrome

| Action               | Default keys      | Description                                                |
|----------------------|-------------------|------------------------------------------------------------|
| `close`              | `Esc`, `q`        | Close the floating navigator                               |
| `switch_pane_left`   | `Ctrl+h`, `Left`  | Switch focus to the Explorer pane                          |
| `switch_pane_right`  | `Ctrl+l`, `Right` | Switch focus to the Scripts pane                           |
| `open_in_split`      | `Ctrl+s`          | Open the selected object in a new vertical tab group       |

The global `toggle_oil_navigator` key (default `-`) also closes oil while it's open.

### `[overlay]` — modals (help, theme picker, menus)

| Action      | Default keys | Description                                |
|-------------|--------------|--------------------------------------------|
| `close`     | `Esc`, `q`   | Close the modal                            |
| `confirm`   | `Enter`      | Confirm the selected option                |
| `nav_down`  | `j`, `Down`  | Move selection down inside the modal       |
| `nav_up`    | `k`, `Up`    | Move selection up inside the modal         |

> Text-entry overlays (rename, create, form fields) accept arbitrary character input and are not configurable through the schema.
