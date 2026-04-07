//! Parse human-friendly key strings into crossterm KeyEvent components.
//!
//! Accepted forms (case-insensitive for special names, case-sensitive for chars):
//!   - Single char:     "j", "G", "?"
//!   - Modifier:        "Ctrl+r", "Alt+x", "Shift+g"
//!   - Combined:        "Ctrl+Shift+e", "Alt+Shift+m"
//!   - Special:         "Esc", "Enter", "Tab", "Space", "Backspace"
//!   - Arrows:          "Up", "Down", "Left", "Right"
//!   - Navigation:      "Home", "End", "Insert", "Delete", "PageUp", "PageDown"
//!   - Function:        "F1" .. "F12"
//!   - Bracketed name:  "<C-h>", "<S-Tab>", "<leader>" (for vim-style users)
//!
//! Note: "<leader>" is parsed as the literal Space key — dbtui's leader.

use crossterm::event::{KeyCode, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyBinding {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyBinding {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// Plain key, no modifiers.
    pub fn plain(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::NONE)
    }

    pub fn ctrl(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::CONTROL)
    }

    pub fn shift(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::SHIFT)
    }

    pub fn alt(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::ALT)
    }

    /// True if a real KeyEvent matches this binding. Treats Char keys as
    /// case-insensitive when SHIFT is involved (because terminals report
    /// Shift+a as Char('A') without the SHIFT modifier on most setups).
    pub fn matches(&self, ev_code: KeyCode, ev_mods: KeyModifiers) -> bool {
        if self.code == ev_code && self.modifiers == ev_mods {
            return true;
        }
        // Char + SHIFT special case: "Shift+a" stored as Char('A') NONE,
        // and the runtime delivers it the same way. Tolerate both shapes.
        if let (KeyCode::Char(want), KeyCode::Char(got)) = (self.code, ev_code) {
            let want_lo = want.to_ascii_lowercase();
            let got_lo = got.to_ascii_lowercase();
            if want_lo != got_lo {
                return false;
            }
            // Strip SHIFT from both — Char('A') already encodes it.
            let mw = self.modifiers - KeyModifiers::SHIFT;
            let me = ev_mods - KeyModifiers::SHIFT;
            return mw == me;
        }
        false
    }
}

/// Parse a single key string like "Ctrl+r" or "j" or "<C-h>" into a KeyBinding.
pub fn parse_key(s: &str) -> Result<KeyBinding, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("empty key string".to_string());
    }

    // <leader>, <C-h>, <S-Tab>, etc. — vim-style notation.
    if trimmed.starts_with('<') && trimmed.ends_with('>') {
        return parse_angle_bracket(&trimmed[1..trimmed.len() - 1]);
    }

    // "Ctrl+Shift+e", "Alt+x", or just "j"
    let mut mods = KeyModifiers::NONE;
    let mut last_part = trimmed;
    while let Some((mod_part, rest)) = last_part.split_once('+') {
        match mod_part.to_ascii_lowercase().as_str() {
            "ctrl" | "c" | "control" => mods |= KeyModifiers::CONTROL,
            "alt" | "a" | "meta" | "m" => mods |= KeyModifiers::ALT,
            "shift" | "s" => mods |= KeyModifiers::SHIFT,
            _ => return Err(format!("unknown modifier '{mod_part}'")),
        }
        last_part = rest;
    }

    let code = parse_keycode(last_part)?;

    // Char + Shift normalisation: "Shift+a" → Char('A') without SHIFT
    if let KeyCode::Char(c) = code
        && mods.contains(KeyModifiers::SHIFT)
        && c.is_ascii_alphabetic()
    {
        return Ok(KeyBinding::new(
            KeyCode::Char(c.to_ascii_uppercase()),
            mods - KeyModifiers::SHIFT,
        ));
    }

    Ok(KeyBinding::new(code, mods))
}

fn parse_angle_bracket(inner: &str) -> Result<KeyBinding, String> {
    let lower = inner.to_ascii_lowercase();
    // Vim shorthand: <C-x>, <S-x>, <A-x>, <leader>
    if lower == "leader" {
        return Ok(KeyBinding::plain(KeyCode::Char(' ')));
    }
    if lower == "space" {
        return Ok(KeyBinding::plain(KeyCode::Char(' ')));
    }
    if lower == "cr" || lower == "enter" || lower == "return" {
        return Ok(KeyBinding::plain(KeyCode::Enter));
    }
    if lower == "esc" || lower == "escape" {
        return Ok(KeyBinding::plain(KeyCode::Esc));
    }
    if lower == "tab" {
        return Ok(KeyBinding::plain(KeyCode::Tab));
    }
    if lower == "bs" || lower == "backspace" {
        return Ok(KeyBinding::plain(KeyCode::Backspace));
    }
    // Modifier-prefixed: <C-h>, <S-Tab>, <C-S-h>, etc.
    let mut mods = KeyModifiers::NONE;
    let mut rest = inner.to_string();
    while let Some((head, tail)) = rest.split_once('-') {
        let head_lo = head.to_ascii_lowercase();
        match head_lo.as_str() {
            "c" => mods |= KeyModifiers::CONTROL,
            "s" => mods |= KeyModifiers::SHIFT,
            "a" | "m" => mods |= KeyModifiers::ALT,
            _ => break,
        }
        rest = tail.to_string();
    }
    let code = parse_keycode(&rest)?;
    Ok(KeyBinding::new(code, mods))
}

fn parse_keycode(s: &str) -> Result<KeyCode, String> {
    let lower = s.to_ascii_lowercase();
    match lower.as_str() {
        "esc" | "escape" => return Ok(KeyCode::Esc),
        "enter" | "return" | "cr" => return Ok(KeyCode::Enter),
        "tab" => return Ok(KeyCode::Tab),
        "backtab" | "shift+tab" => return Ok(KeyCode::BackTab),
        "space" | "spc" => return Ok(KeyCode::Char(' ')),
        "backspace" | "bs" => return Ok(KeyCode::Backspace),
        "delete" | "del" => return Ok(KeyCode::Delete),
        "insert" | "ins" => return Ok(KeyCode::Insert),
        "home" => return Ok(KeyCode::Home),
        "end" => return Ok(KeyCode::End),
        "pageup" | "pgup" => return Ok(KeyCode::PageUp),
        "pagedown" | "pgdn" | "pgdown" => return Ok(KeyCode::PageDown),
        "up" => return Ok(KeyCode::Up),
        "down" => return Ok(KeyCode::Down),
        "left" => return Ok(KeyCode::Left),
        "right" => return Ok(KeyCode::Right),
        _ => {}
    }
    // Function keys F1..F12
    if let Some(rest) = lower.strip_prefix('f')
        && let Ok(n) = rest.parse::<u8>()
        && (1..=12).contains(&n)
    {
        return Ok(KeyCode::F(n));
    }
    // Single character — preserve original case (j vs J).
    let mut chars = s.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Ok(KeyCode::Char(c));
    }
    Err(format!("unknown key '{s}'"))
}

/// Format a KeyBinding back to a human-friendly string suitable for the
/// TOML config and the help screen.
pub fn format_key(kb: &KeyBinding) -> String {
    let mut parts: Vec<String> = Vec::new();
    if kb.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl".to_string());
    }
    if kb.modifiers.contains(KeyModifiers::ALT) {
        parts.push("Alt".to_string());
    }
    // Shift only when the code itself isn't already an uppercase letter.
    let shift_implicit = matches!(kb.code, KeyCode::Char(c) if c.is_ascii_uppercase());
    if kb.modifiers.contains(KeyModifiers::SHIFT) && !shift_implicit {
        parts.push("Shift".to_string());
    }
    parts.push(format_code(kb.code));
    parts.join("+")
}

fn format_code(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "BackTab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Insert => "Insert".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::F(n) => format!("F{n}"),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_char() {
        let kb = parse_key("j").unwrap();
        assert_eq!(kb.code, KeyCode::Char('j'));
        assert_eq!(kb.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parses_ctrl() {
        let kb = parse_key("Ctrl+r").unwrap();
        assert_eq!(kb.code, KeyCode::Char('r'));
        assert_eq!(kb.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn parses_shift_lowercase() {
        let kb = parse_key("Shift+g").unwrap();
        assert_eq!(kb.code, KeyCode::Char('G'));
        assert_eq!(kb.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parses_uppercase_as_shifted() {
        let kb = parse_key("G").unwrap();
        assert_eq!(kb.code, KeyCode::Char('G'));
    }

    #[test]
    fn parses_special() {
        assert_eq!(parse_key("Esc").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_key("Tab").unwrap().code, KeyCode::Tab);
        assert_eq!(parse_key("Space").unwrap().code, KeyCode::Char(' '));
    }

    #[test]
    fn parses_arrows() {
        assert_eq!(parse_key("Down").unwrap().code, KeyCode::Down);
    }

    #[test]
    fn parses_function_keys() {
        assert_eq!(parse_key("F5").unwrap().code, KeyCode::F(5));
    }

    #[test]
    fn parses_angle_bracket_leader() {
        let kb = parse_key("<leader>").unwrap();
        assert_eq!(kb.code, KeyCode::Char(' '));
    }

    #[test]
    fn parses_angle_bracket_ctrl() {
        let kb = parse_key("<C-h>").unwrap();
        assert_eq!(kb.code, KeyCode::Char('h'));
        assert_eq!(kb.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn formats_round_trip() {
        let cases = ["j", "G", "Ctrl+r", "F12", "Esc", "Space"];
        for c in cases {
            let kb = parse_key(c).unwrap();
            let f = format_key(&kb);
            let kb2 = parse_key(&f).unwrap();
            assert_eq!(kb, kb2, "round-trip failed for {c}");
        }
    }

    #[test]
    fn matches_handles_shift_normalisation() {
        let bound = parse_key("Shift+g").unwrap();
        // Runtime delivers as Char('G') NONE
        assert!(bound.matches(KeyCode::Char('G'), KeyModifiers::NONE));
        // Or with SHIFT explicitly
        assert!(bound.matches(KeyCode::Char('G'), KeyModifiers::SHIFT));
    }
}
