pub mod buffer;
pub mod input;
pub mod motions;
pub mod operators;
pub mod render;
pub mod search;
pub mod visual;

pub use buffer::VimEditor;

use crossterm::event::KeyEvent;

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

/// Actions returned from VimEditor.handle_key() to inform the parent
pub enum EditorAction {
    /// The editor consumed the key
    Handled,
    /// The editor does not handle this key - bubble up to parent
    Unhandled(KeyEvent),
    /// User wants to execute a query (Ctrl+Enter in script)
    ExecuteQuery(String),
    /// User wants to close the buffer (<leader>bd)
    CloseBuffer,
    /// User wants to save the buffer (Ctrl+S)
    SaveBuffer,
    /// User wants to compile to database (<leader><leader>s)
    CompileToDb,
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
#[derive(Debug, Clone)]
pub struct Register {
    pub content: String,
    pub linewise: bool,
}

impl Default for Register {
    fn default() -> Self {
        Self {
            content: String::new(),
            linewise: false,
        }
    }
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

pub const SCROLLOFF: usize = 3;
