pub mod buffer;
pub mod input;
pub mod motions;
pub mod operators;
pub mod render;
pub mod search;
pub mod visual;

use crossterm::event::KeyEvent;
use ratatui::style::Color;
use ratatui::text::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Insert,
    Visual(VisualKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualKind {
    Char,
    Line,
    Block,
}

#[derive(Debug, Clone)]
pub struct VimModeConfig {
    pub insert_allowed: bool,
    pub visual_allowed: bool,
}

impl Default for VimModeConfig {
    fn default() -> Self {
        Self {
            insert_allowed: true,
            visual_allowed: true,
        }
    }
}

impl VimModeConfig {
    pub fn read_only() -> Self {
        Self {
            insert_allowed: false,
            visual_allowed: true,
        }
    }
}

/// Actions returned from VimEditor.handle_key() to inform the parent.
/// These are generic - no app-specific variants.
#[allow(dead_code)]
pub enum EditorAction {
    /// The editor consumed the key
    Handled,
    /// The editor does not handle this key - bubble up to parent
    Unhandled(KeyEvent),
    /// Save buffer (:w or Ctrl+S)
    Save,
    /// Close buffer (:q)
    Close,
    /// Force close without saving (:q!)
    ForceClose,
    /// Save and close (:wq, :x)
    SaveAndClose,
}

/// Leader key configuration
pub const LEADER_KEY: char = ' ';

/// Operator waiting for a motion
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Yank,
    Change,
    Indent,
    Dedent,
    Uppercase,
    Lowercase,
}

/// The result of a motion: a range in the buffer
#[derive(Debug, Clone)]
pub struct MotionRange {
    pub start_row: usize,
    pub start_col: usize,
    pub end_row: usize,
    pub end_col: usize,
    pub linewise: bool,
}

/// Snapshot for undo/redo
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

/// Register content
#[derive(Debug, Clone, Default)]
pub struct Register {
    pub content: String,
    #[allow(dead_code)]
    pub linewise: bool,
}

/// Search state
#[derive(Debug, Clone)]
pub struct SearchState {
    pub pattern: String,
    pub forward: bool,
    pub active: bool,
    pub input_buffer: String,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            forward: true,
            active: false,
            input_buffer: String::new(),
        }
    }
}

/// Edit record for repeat (.)
#[derive(Debug, Clone)]
pub struct EditRecord {
    pub keys: Vec<KeyEvent>,
}

/// Direction for f/F/t/T char find
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindDirection {
    Forward,
    Backward,
}

/// Theme colors used by the vim editor renderer.
/// Each application maps its own theme to this struct.
#[derive(Debug, Clone)]
pub struct VimTheme {
    pub border_focused: Color,
    pub border_unfocused: Color,
    pub border_insert: Color,
    pub editor_bg: Color,
    pub line_nr: Color,
    pub line_nr_active: Color,
    pub visual_bg: Color,
    pub visual_fg: Color,
    pub dim: Color,
    pub accent: Color,
    /// Background for search matches (all occurrences)
    pub search_match_bg: Color,
    /// Background for the current search match (where the cursor jumped to)
    pub search_current_bg: Color,
    /// Foreground for search match text
    pub search_match_fg: Color,
}

/// Trait for language-specific syntax highlighting.
/// Each application implements this for its language (SQL, JSON, HTTP, etc.).
pub trait SyntaxHighlighter {
    fn highlight_line<'a>(&self, line: &'a str, spans: &mut Vec<Span<'a>>);
    fn highlight_segment<'a>(&self, text: &'a str, spans: &mut Vec<Span<'a>>) {
        // Default: delegate to highlight_line
        self.highlight_line(text, spans);
    }
}

/// No-op highlighter (plain text, no coloring)
#[allow(dead_code)]
pub struct PlainHighlighter;

impl SyntaxHighlighter for PlainHighlighter {
    fn highlight_line<'a>(&self, line: &'a str, spans: &mut Vec<Span<'a>>) {
        if !line.is_empty() {
            spans.push(Span::raw(line));
        }
    }
}

pub const SCROLLOFF: usize = 3;
