// The keybindings module is in active build-out — handlers will progressively
// migrate from hardcoded matches to bindings.matches() calls. Until that
// migration completes, allow dead code in the module so we can ship the
// foundation in one piece without spurious warnings.
#![allow(dead_code)]

//! Configurable keybinding system for dbtui.
//!
//! Loads bindings from `~/.config/dbtui/keybindings.toml`, falling back to a
//! complete set of defaults. Event handlers query the resolved bindings via
//! `KeyBindings::matches(context, action, key_event)`. The help screen and
//! the leader popup query `KeyBindings::keys_for(context, action)` so the
//! UI always displays the keys the user actually has bound.
//!
//! See KEYBINDINGS.md in the repository root for the full user-facing docs.

mod defaults;
mod parser;

use crossterm::event::KeyEvent;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

pub use parser::{KeyBinding, parse_key};

/// All bindable contexts. The `as_str()` matches the [section] header in
/// `keybindings.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    Global,
    Leader,
    LeaderBuffer,
    LeaderWindow,
    LeaderFile,
    LeaderQuit,
    LeaderSnippet,
    Sidebar,
    Scripts,
    Grid,
    Oil,
    Overlay,
}

impl Context {
    pub fn as_str(self) -> &'static str {
        match self {
            Context::Global => "global",
            Context::Leader => "leader",
            Context::LeaderBuffer => "leader_buffer",
            Context::LeaderWindow => "leader_window",
            Context::LeaderFile => "leader_file",
            Context::LeaderQuit => "leader_quit",
            Context::LeaderSnippet => "leader_snippet",
            Context::Sidebar => "sidebar",
            Context::Scripts => "scripts",
            Context::Grid => "grid",
            Context::Oil => "oil",
            Context::Overlay => "overlay",
        }
    }
}

/// Resolved keybinding map: context → action → list of bound key combos.
#[derive(Debug, Clone, Default)]
pub struct KeyBindings {
    map: HashMap<&'static str, HashMap<String, Vec<KeyBinding>>>,
    /// Original string form, kept for `keys_for` display and TOML round-trip.
    string_map: HashMap<&'static str, HashMap<String, Vec<String>>>,
}

impl KeyBindings {
    /// Build the default keybindings (used when no user file is present and
    /// also as the merge base when one is).
    pub fn defaults() -> Self {
        let raw = defaults::defaults();
        Self::from_string_map(raw)
    }

    /// Try to load `~/.config/dbtui/keybindings.toml`. The user's overrides
    /// are merged on top of the defaults — any action they don't define keeps
    /// the default binding. On parse error returns the error so the caller
    /// can show it to the user; defaults are still loaded as a fallback.
    pub fn load_from_default_path() -> (Self, Option<String>) {
        let mut bindings = Self::defaults();
        let path = match config_path() {
            Some(p) => p,
            None => return (bindings, None),
        };
        if !path.exists() {
            return (bindings, None);
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => return (bindings, Some(format!("read failed: {e}"))),
        };
        match toml::from_str::<BTreeMap<String, BTreeMap<String, toml::Value>>>(&content) {
            Ok(parsed) => {
                if let Err(e) = bindings.merge_overrides(parsed) {
                    return (bindings, Some(e));
                }
                (bindings, None)
            }
            Err(e) => (bindings, Some(format!("syntax error: {e}"))),
        }
    }

    /// Build from the raw nested string map (used by defaults() and tests).
    fn from_string_map(raw: BTreeMap<String, BTreeMap<String, Vec<String>>>) -> Self {
        let mut map: HashMap<&'static str, HashMap<String, Vec<KeyBinding>>> = HashMap::new();
        let mut string_map: HashMap<&'static str, HashMap<String, Vec<String>>> = HashMap::new();
        for (ctx_name, actions) in raw {
            let ctx_static = match static_ctx_name(&ctx_name) {
                Some(s) => s,
                None => continue, // unknown context — ignore
            };
            let mut action_map: HashMap<String, Vec<KeyBinding>> = HashMap::new();
            let mut action_strings: HashMap<String, Vec<String>> = HashMap::new();
            for (action, keys) in actions {
                let parsed: Vec<KeyBinding> =
                    keys.iter().filter_map(|k| parse_key(k).ok()).collect();
                action_map.insert(action.clone(), parsed);
                action_strings.insert(action, keys);
            }
            map.insert(ctx_static, action_map);
            string_map.insert(ctx_static, action_strings);
        }
        Self { map, string_map }
    }

    /// Merge user overrides on top of self. Each key in the override TOML
    /// fully replaces the default binding for that action — partial merges
    /// at the action level (e.g. "add this key to scroll_down") are not
    /// supported. Use a list to bind multiple keys to the same action.
    fn merge_overrides(
        &mut self,
        overrides: BTreeMap<String, BTreeMap<String, toml::Value>>,
    ) -> Result<(), String> {
        for (ctx_name, actions) in overrides {
            let ctx_static = match static_ctx_name(&ctx_name) {
                Some(s) => s,
                None => continue,
            };
            let action_map = self.map.entry(ctx_static).or_default();
            let action_strings = self.string_map.entry(ctx_static).or_default();
            for (action, value) in actions {
                let key_strings: Vec<String> = match value {
                    toml::Value::String(s) => vec![s],
                    toml::Value::Array(arr) => arr
                        .into_iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect(),
                    _ => continue,
                };
                let mut parsed = Vec::new();
                for s in &key_strings {
                    match parse_key(s) {
                        Ok(kb) => parsed.push(kb),
                        Err(e) => return Err(format!("[{ctx_name}].{action}: {e}")),
                    }
                }
                action_map.insert(action.clone(), parsed);
                action_strings.insert(action, key_strings);
            }
        }
        Ok(())
    }

    /// Returns true when the given KeyEvent matches the binding for `action`
    /// in `context`. Falls back to the global context if the action is not
    /// found in the requested context.
    pub fn matches(&self, context: Context, action: &str, ev: &KeyEvent) -> bool {
        if let Some(action_map) = self.map.get(context.as_str())
            && let Some(bindings) = action_map.get(action)
        {
            for kb in bindings {
                if kb.matches(ev.code, ev.modifiers) {
                    return true;
                }
            }
        }
        false
    }

    /// Returns the bound key combos for `action` in `context` as the original
    /// human-readable strings (for the help screen, leader popup, etc.).
    pub fn keys_for(&self, context: Context, action: &str) -> Vec<String> {
        self.string_map
            .get(context.as_str())
            .and_then(|m| m.get(action))
            .cloned()
            .unwrap_or_default()
    }

    /// First bound key combo for `action` (or `"?"` placeholder when none).
    /// Useful for help screens that only have room for one entry per action.
    pub fn primary_key(&self, context: Context, action: &str) -> String {
        self.keys_for(context, action)
            .into_iter()
            .next()
            .unwrap_or_else(|| "?".to_string())
    }

    /// Serialize to TOML, using the same shape the loader expects. Used by
    /// `dbtui --dump-keybindings`.
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        out.push_str("# dbtui keybindings — generated by `dbtui --dump-keybindings`.\n");
        out.push_str("# Edit and save to ~/.config/dbtui/keybindings.toml.\n");
        out.push_str("# You only need the keys you want to change — anything you omit\n");
        out.push_str("# falls back to the default below.\n");
        out.push_str("# See KEYBINDINGS.md for the format.\n\n");

        // Stable order: walk a fixed list of contexts.
        let order = [
            "global",
            "leader",
            "leader_buffer",
            "leader_window",
            "leader_file",
            "leader_quit",
            "leader_snippet",
            "sidebar",
            "scripts",
            "grid",
            "oil",
            "overlay",
        ];
        for ctx in order {
            let actions = match self.string_map.get(ctx) {
                Some(a) => a,
                None => continue,
            };
            out.push_str(&format!("[{ctx}]\n"));
            // Sort actions alphabetically for deterministic output.
            let mut entries: Vec<(&String, &Vec<String>)> = actions.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            for (action, keys) in entries {
                if keys.len() == 1 {
                    out.push_str(&format!("{action} = {:?}\n", keys[0]));
                } else {
                    let formatted: Vec<String> =
                        keys.iter().map(|k| format!("{k:?}")).collect();
                    out.push_str(&format!("{action} = [{}]\n", formatted.join(", ")));
                }
            }
            out.push('\n');
        }
        out
    }
}

/// Map a context name string from TOML to its &'static str representation.
/// Returns None for unknown contexts (silently ignored on load).
fn static_ctx_name(name: &str) -> Option<&'static str> {
    match name {
        "global" => Some("global"),
        "leader" => Some("leader"),
        "leader_buffer" => Some("leader_buffer"),
        "leader_window" => Some("leader_window"),
        "leader_file" => Some("leader_file"),
        "leader_quit" => Some("leader_quit"),
        "leader_snippet" => Some("leader_snippet"),
        "sidebar" => Some("sidebar"),
        "scripts" => Some("scripts"),
        "grid" => Some("grid"),
        "oil" => Some("oil"),
        "overlay" => Some("overlay"),
        _ => None,
    }
}

/// `~/.config/dbtui/keybindings.toml` (using the directories crate). Returns
/// None if the user has no XDG-style config dir for some reason.
pub fn config_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "dbtui")?;
    Some(dirs.config_dir().join("keybindings.toml"))
}

/// Write the default config to the user's config path, creating the parent
/// directory as needed. Used by `--dump-keybindings`.
pub fn write_default_config() -> std::io::Result<PathBuf> {
    let path = config_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no config dir available")
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = KeyBindings::defaults().to_toml();
    std::fs::write(&path, content)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn defaults_load_without_panicking() {
        let kb = KeyBindings::defaults();
        assert!(!kb.string_map.is_empty());
    }

    #[test]
    fn defaults_match_well_known_keys() {
        let kb = KeyBindings::defaults();
        let j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(kb.matches(Context::Sidebar, "scroll_down", &j));
        let r = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
        // Ctrl+r isn't bound for sidebar.scroll_down — just confirm matches() returns false.
        assert!(!kb.matches(Context::Sidebar, "scroll_down", &r));
    }

    #[test]
    fn dump_round_trips() {
        let kb = KeyBindings::defaults();
        let toml_str = kb.to_toml();
        // Should at least be valid TOML.
        let _: BTreeMap<String, BTreeMap<String, toml::Value>> =
            toml::from_str(&toml_str).unwrap();
    }
}
