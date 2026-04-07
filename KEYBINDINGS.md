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
