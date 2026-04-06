use ratatui::style::{Color, Modifier, Style};

use crate::ui::state::Mode;

/// Available theme names
pub const THEME_NAMES: &[&str] = &[
    "Tokyo Night",
    "Catppuccin",
    "Dracula",
    "Nord",
    "Gruvbox",
    "Default",
];

pub struct Theme {
    pub name: String,

    // Borders
    pub border_focused: Color,
    pub border_unfocused: Color,
    pub border_insert: Color,

    // Mode indicators
    pub mode_normal: Color,
    pub mode_insert: Color,
    pub mode_visual: Color,

    // Tree / Sidebar
    pub tree_expanded: Color,
    pub tree_collapsed: Color,
    pub tree_selected_bg: Color,
    pub tree_selected_fg: Color,
    pub tree_category: Color,
    pub tree_connection: Color,
    pub tree_schema: Color,
    pub tree_table: Color,
    pub tree_view: Color,
    pub tree_package: Color,
    pub tree_procedure: Color,
    pub tree_function: Color,

    // Data grid
    pub grid_header_bg: Color,
    pub grid_header_fg: Color,
    pub grid_selected_bg: Color,
    pub grid_selected_fg: Color,
    pub grid_row_even: Color,
    pub grid_row_odd: Color,
    pub grid_null: Color,
    pub grid_number: Color,

    // Tabs
    pub tab_active_fg: Color,
    pub tab_active_bg: Color,
    pub tab_inactive_fg: Color,

    // Connection
    pub conn_connected: Color,
    pub conn_disconnected: Color,
    pub conn_connecting: Color,

    // Status / bars
    pub status_bg: Color,
    pub status_fg: Color,
    pub topbar_bg: Color,
    pub topbar_fg: Color,

    // SQL syntax
    pub sql_keyword: Color,
    pub sql_string: Color,
    pub sql_number: Color,
    pub sql_comment: Color,
    pub sql_operator: Color,
    pub sql_bind_var: Color,

    // Editor
    pub editor_line_nr: Color,
    pub editor_line_nr_active: Color,
    #[allow(dead_code)]
    pub editor_cursor_line_bg: Color,
    pub editor_bg: Color,

    // Dialog
    pub dialog_bg: Color,
    pub dialog_field_active: Color,
    pub dialog_field_inactive: Color,
    pub error_fg: Color,
    pub error_bg: Color,

    // Accent
    pub accent: Color,
    pub dim: Color,
    pub separator: Color,
}

impl Theme {
    pub fn vim_theme(&self) -> vimltui::VimTheme {
        vimltui::VimTheme {
            border_focused: self.border_focused,
            border_unfocused: self.border_unfocused,
            border_insert: self.border_insert,
            editor_bg: self.editor_bg,
            line_nr: self.editor_line_nr,
            line_nr_active: self.editor_line_nr_active,
            visual_bg: self.tree_selected_bg,
            visual_fg: self.tree_selected_fg,
            dim: self.dim,
            accent: self.accent,
            search_match_bg: Color::Yellow,
            search_current_bg: Color::Rgb(255, 165, 0),
            search_match_fg: Color::Black,
            yank_highlight_bg: Color::Rgb(100, 100, 60),
            substitute_preview_bg: Color::Rgb(80, 60, 100),
        }
    }

    pub fn by_name(name: &str) -> Self {
        match name {
            "Tokyo Night" => Self::tokyo_night(),
            "Catppuccin" => Self::catppuccin(),
            "Dracula" => Self::dracula(),
            "Nord" => Self::nord(),
            "Gruvbox" => Self::gruvbox(),
            _ => Self::default(),
        }
    }

    pub fn border_style(&self, focused: bool, mode: &Mode) -> Style {
        let color = if !focused {
            self.border_unfocused
        } else {
            match mode {
                Mode::Insert => self.border_insert,
                Mode::Normal | Mode::Visual => self.border_focused,
            }
        };
        Style::default().fg(color)
    }

    pub fn mode_style(&self, mode: &Mode) -> Style {
        let bg = match mode {
            Mode::Normal => self.mode_normal,
            Mode::Insert => self.mode_insert,
            Mode::Visual => self.mode_visual,
        };
        Style::default()
            .bg(bg)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_style(&self, active: bool) -> Style {
        if active {
            Style::default()
                .fg(self.tab_active_fg)
                .bg(self.tab_active_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.tab_inactive_fg)
        }
    }

    pub fn connection_indicator(&self, connected: bool) -> (&str, Style) {
        if connected {
            ("●", Style::default().fg(self.conn_connected))
        } else {
            ("○", Style::default().fg(self.conn_disconnected))
        }
    }

    pub fn grid_header_style(&self) -> Style {
        Style::default()
            .bg(self.grid_header_bg)
            .fg(self.grid_header_fg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn grid_row_style(&self, row_idx: usize) -> Style {
        if row_idx.is_multiple_of(2) {
            Style::default().bg(self.grid_row_even)
        } else {
            Style::default().bg(self.grid_row_odd)
        }
    }

    #[allow(dead_code)]
    pub fn grid_selected_style(&self) -> Style {
        Style::default()
            .bg(self.grid_selected_bg)
            .fg(self.grid_selected_fg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn null_style(&self) -> Style {
        Style::default()
            .fg(self.grid_null)
            .add_modifier(Modifier::ITALIC)
    }

    // --- Theme presets ---
    // Transparent: use Color::Reset for backgrounds to let terminal show through

    pub fn tokyo_night() -> Self {
        Self {
            name: "Tokyo Night".to_string(),
            border_focused: Color::Rgb(122, 162, 247),
            border_unfocused: Color::Rgb(59, 66, 97),
            border_insert: Color::Rgb(158, 206, 106),
            mode_normal: Color::Rgb(122, 162, 247),
            mode_insert: Color::Rgb(158, 206, 106),
            mode_visual: Color::Rgb(187, 154, 247),
            tree_expanded: Color::Rgb(224, 175, 104),
            tree_collapsed: Color::Rgb(86, 95, 137),
            tree_selected_bg: Color::Rgb(41, 46, 66),
            tree_selected_fg: Color::Rgb(192, 202, 245),
            tree_category: Color::Rgb(169, 177, 214),
            tree_connection: Color::Rgb(224, 175, 104),
            tree_schema: Color::Rgb(122, 162, 247),
            tree_table: Color::Rgb(224, 175, 104),
            tree_view: Color::Rgb(125, 207, 255),
            tree_package: Color::Rgb(187, 154, 247),
            tree_procedure: Color::Rgb(158, 206, 106),
            tree_function: Color::Rgb(125, 207, 255),
            grid_header_bg: Color::Rgb(41, 46, 66),
            grid_header_fg: Color::Rgb(192, 202, 245),
            grid_selected_bg: Color::Rgb(41, 46, 66),
            grid_selected_fg: Color::White,
            grid_row_even: Color::Reset,
            grid_row_odd: Color::Rgb(26, 27, 38),
            grid_null: Color::Rgb(86, 95, 137),
            grid_number: Color::Rgb(255, 158, 100),
            tab_active_fg: Color::Rgb(122, 162, 247),
            tab_active_bg: Color::Rgb(41, 46, 66),
            tab_inactive_fg: Color::Rgb(86, 95, 137),
            conn_connected: Color::Rgb(158, 206, 106),
            conn_disconnected: Color::Rgb(247, 118, 142),
            conn_connecting: Color::Rgb(224, 175, 104),
            status_bg: Color::Reset,
            status_fg: Color::Rgb(169, 177, 214),
            topbar_bg: Color::Reset,
            topbar_fg: Color::Rgb(192, 202, 245),
            sql_keyword: Color::Rgb(122, 162, 247),
            sql_string: Color::Rgb(158, 206, 106),
            sql_number: Color::Rgb(255, 158, 100),
            sql_comment: Color::Rgb(86, 95, 137),
            sql_operator: Color::Rgb(169, 177, 214),
            sql_bind_var: Color::Rgb(224, 175, 104),
            editor_line_nr: Color::Rgb(59, 66, 97),
            editor_line_nr_active: Color::Rgb(224, 175, 104),
            editor_cursor_line_bg: Color::Rgb(41, 46, 66),
            editor_bg: Color::Reset,
            dialog_bg: Color::Reset,
            dialog_field_active: Color::Rgb(122, 162, 247),
            dialog_field_inactive: Color::Rgb(86, 95, 137),
            error_fg: Color::Rgb(247, 118, 142),
            error_bg: Color::Rgb(50, 20, 30),
            accent: Color::Rgb(122, 162, 247),
            dim: Color::Rgb(86, 95, 137),
            separator: Color::Rgb(59, 66, 97),
        }
    }

    pub fn catppuccin() -> Self {
        Self {
            name: "Catppuccin".to_string(),
            border_focused: Color::Rgb(137, 180, 250),
            border_unfocused: Color::Rgb(69, 71, 90),
            border_insert: Color::Rgb(166, 227, 161),
            mode_normal: Color::Rgb(137, 180, 250),
            mode_insert: Color::Rgb(166, 227, 161),
            mode_visual: Color::Rgb(203, 166, 247),
            tree_expanded: Color::Rgb(249, 226, 175),
            tree_collapsed: Color::Rgb(88, 91, 112),
            tree_selected_bg: Color::Rgb(49, 50, 68),
            tree_selected_fg: Color::Rgb(205, 214, 244),
            tree_category: Color::Rgb(186, 194, 222),
            tree_connection: Color::Rgb(250, 179, 135),
            tree_schema: Color::Rgb(137, 180, 250),
            tree_table: Color::Rgb(249, 226, 175),
            tree_view: Color::Rgb(148, 226, 213),
            tree_package: Color::Rgb(203, 166, 247),
            tree_procedure: Color::Rgb(166, 227, 161),
            tree_function: Color::Rgb(148, 226, 213),
            grid_header_bg: Color::Rgb(49, 50, 68),
            grid_header_fg: Color::Rgb(205, 214, 244),
            grid_selected_bg: Color::Rgb(49, 50, 68),
            grid_selected_fg: Color::White,
            grid_row_even: Color::Reset,
            grid_row_odd: Color::Rgb(30, 30, 46),
            grid_null: Color::Rgb(108, 112, 134),
            grid_number: Color::Rgb(250, 179, 135),
            tab_active_fg: Color::Rgb(137, 180, 250),
            tab_active_bg: Color::Rgb(49, 50, 68),
            tab_inactive_fg: Color::Rgb(108, 112, 134),
            conn_connected: Color::Rgb(166, 227, 161),
            conn_disconnected: Color::Rgb(243, 139, 168),
            conn_connecting: Color::Rgb(249, 226, 175),
            status_bg: Color::Reset,
            status_fg: Color::Rgb(186, 194, 222),
            topbar_bg: Color::Reset,
            topbar_fg: Color::Rgb(205, 214, 244),
            sql_keyword: Color::Rgb(137, 180, 250),
            sql_string: Color::Rgb(166, 227, 161),
            sql_number: Color::Rgb(250, 179, 135),
            sql_comment: Color::Rgb(108, 112, 134),
            sql_operator: Color::Rgb(186, 194, 222),
            sql_bind_var: Color::Rgb(249, 226, 175),
            editor_line_nr: Color::Rgb(88, 91, 112),
            editor_line_nr_active: Color::Rgb(249, 226, 175),
            editor_cursor_line_bg: Color::Rgb(49, 50, 68),
            editor_bg: Color::Reset,
            dialog_bg: Color::Reset,
            dialog_field_active: Color::Rgb(137, 180, 250),
            dialog_field_inactive: Color::Rgb(108, 112, 134),
            error_fg: Color::Rgb(243, 139, 168),
            error_bg: Color::Rgb(50, 20, 30),
            accent: Color::Rgb(137, 180, 250),
            dim: Color::Rgb(108, 112, 134),
            separator: Color::Rgb(69, 71, 90),
        }
    }

    pub fn dracula() -> Self {
        Self {
            name: "Dracula".to_string(),
            border_focused: Color::Rgb(189, 147, 249),
            border_unfocused: Color::Rgb(68, 71, 90),
            border_insert: Color::Rgb(80, 250, 123),
            mode_normal: Color::Rgb(189, 147, 249),
            mode_insert: Color::Rgb(80, 250, 123),
            mode_visual: Color::Rgb(255, 121, 198),
            tree_expanded: Color::Rgb(241, 250, 140),
            tree_collapsed: Color::Rgb(98, 114, 164),
            tree_selected_bg: Color::Rgb(68, 71, 90),
            tree_selected_fg: Color::Rgb(248, 248, 242),
            tree_category: Color::Rgb(248, 248, 242),
            tree_connection: Color::Rgb(255, 184, 108),
            tree_schema: Color::Rgb(139, 233, 253),
            tree_table: Color::Rgb(241, 250, 140),
            tree_view: Color::Rgb(139, 233, 253),
            tree_package: Color::Rgb(189, 147, 249),
            tree_procedure: Color::Rgb(80, 250, 123),
            tree_function: Color::Rgb(139, 233, 253),
            grid_header_bg: Color::Rgb(68, 71, 90),
            grid_header_fg: Color::Rgb(248, 248, 242),
            grid_selected_bg: Color::Rgb(68, 71, 90),
            grid_selected_fg: Color::White,
            grid_row_even: Color::Reset,
            grid_row_odd: Color::Rgb(40, 42, 54),
            grid_null: Color::Rgb(98, 114, 164),
            grid_number: Color::Rgb(255, 184, 108),
            tab_active_fg: Color::Rgb(189, 147, 249),
            tab_active_bg: Color::Rgb(68, 71, 90),
            tab_inactive_fg: Color::Rgb(98, 114, 164),
            conn_connected: Color::Rgb(80, 250, 123),
            conn_disconnected: Color::Rgb(255, 85, 85),
            conn_connecting: Color::Rgb(241, 250, 140),
            status_bg: Color::Reset,
            status_fg: Color::Rgb(248, 248, 242),
            topbar_bg: Color::Reset,
            topbar_fg: Color::Rgb(248, 248, 242),
            sql_keyword: Color::Rgb(255, 121, 198),
            sql_string: Color::Rgb(241, 250, 140),
            sql_number: Color::Rgb(189, 147, 249),
            sql_comment: Color::Rgb(98, 114, 164),
            sql_operator: Color::Rgb(248, 248, 242),
            sql_bind_var: Color::Rgb(241, 250, 140),
            editor_line_nr: Color::Rgb(98, 114, 164),
            editor_line_nr_active: Color::Rgb(241, 250, 140),
            editor_cursor_line_bg: Color::Rgb(68, 71, 90),
            editor_bg: Color::Reset,
            dialog_bg: Color::Reset,
            dialog_field_active: Color::Rgb(189, 147, 249),
            dialog_field_inactive: Color::Rgb(98, 114, 164),
            error_fg: Color::Rgb(255, 85, 85),
            error_bg: Color::Rgb(60, 20, 20),
            accent: Color::Rgb(189, 147, 249),
            dim: Color::Rgb(98, 114, 164),
            separator: Color::Rgb(68, 71, 90),
        }
    }

    pub fn nord() -> Self {
        Self {
            name: "Nord".to_string(),
            border_focused: Color::Rgb(136, 192, 208),
            border_unfocused: Color::Rgb(76, 86, 106),
            border_insert: Color::Rgb(163, 190, 140),
            mode_normal: Color::Rgb(129, 161, 193),
            mode_insert: Color::Rgb(163, 190, 140),
            mode_visual: Color::Rgb(180, 142, 173),
            tree_expanded: Color::Rgb(235, 203, 139),
            tree_collapsed: Color::Rgb(76, 86, 106),
            tree_selected_bg: Color::Rgb(59, 66, 82),
            tree_selected_fg: Color::Rgb(236, 239, 244),
            tree_category: Color::Rgb(216, 222, 233),
            tree_connection: Color::Rgb(208, 135, 112),
            tree_schema: Color::Rgb(129, 161, 193),
            tree_table: Color::Rgb(235, 203, 139),
            tree_view: Color::Rgb(136, 192, 208),
            tree_package: Color::Rgb(180, 142, 173),
            tree_procedure: Color::Rgb(163, 190, 140),
            tree_function: Color::Rgb(136, 192, 208),
            grid_header_bg: Color::Rgb(59, 66, 82),
            grid_header_fg: Color::Rgb(236, 239, 244),
            grid_selected_bg: Color::Rgb(59, 66, 82),
            grid_selected_fg: Color::White,
            grid_row_even: Color::Reset,
            grid_row_odd: Color::Rgb(46, 52, 64),
            grid_null: Color::Rgb(97, 110, 136),
            grid_number: Color::Rgb(208, 135, 112),
            tab_active_fg: Color::Rgb(136, 192, 208),
            tab_active_bg: Color::Rgb(59, 66, 82),
            tab_inactive_fg: Color::Rgb(97, 110, 136),
            conn_connected: Color::Rgb(163, 190, 140),
            conn_disconnected: Color::Rgb(191, 97, 106),
            conn_connecting: Color::Rgb(235, 203, 139),
            status_bg: Color::Reset,
            status_fg: Color::Rgb(216, 222, 233),
            topbar_bg: Color::Reset,
            topbar_fg: Color::Rgb(236, 239, 244),
            sql_keyword: Color::Rgb(129, 161, 193),
            sql_string: Color::Rgb(163, 190, 140),
            sql_number: Color::Rgb(208, 135, 112),
            sql_comment: Color::Rgb(97, 110, 136),
            sql_operator: Color::Rgb(216, 222, 233),
            sql_bind_var: Color::Rgb(235, 203, 139),
            editor_line_nr: Color::Rgb(76, 86, 106),
            editor_line_nr_active: Color::Rgb(235, 203, 139),
            editor_cursor_line_bg: Color::Rgb(59, 66, 82),
            editor_bg: Color::Reset,
            dialog_bg: Color::Reset,
            dialog_field_active: Color::Rgb(136, 192, 208),
            dialog_field_inactive: Color::Rgb(97, 110, 136),
            error_fg: Color::Rgb(191, 97, 106),
            error_bg: Color::Rgb(60, 30, 30),
            accent: Color::Rgb(136, 192, 208),
            dim: Color::Rgb(97, 110, 136),
            separator: Color::Rgb(76, 86, 106),
        }
    }

    pub fn gruvbox() -> Self {
        Self {
            name: "Gruvbox".to_string(),
            border_focused: Color::Rgb(131, 165, 152),
            border_unfocused: Color::Rgb(80, 73, 69),
            border_insert: Color::Rgb(184, 187, 38),
            mode_normal: Color::Rgb(131, 165, 152),
            mode_insert: Color::Rgb(184, 187, 38),
            mode_visual: Color::Rgb(211, 134, 155),
            tree_expanded: Color::Rgb(250, 189, 47),
            tree_collapsed: Color::Rgb(102, 92, 84),
            tree_selected_bg: Color::Rgb(60, 56, 54),
            tree_selected_fg: Color::Rgb(235, 219, 178),
            tree_category: Color::Rgb(213, 196, 161),
            tree_connection: Color::Rgb(254, 128, 25),
            tree_schema: Color::Rgb(131, 165, 152),
            tree_table: Color::Rgb(250, 189, 47),
            tree_view: Color::Rgb(142, 192, 124),
            tree_package: Color::Rgb(211, 134, 155),
            tree_procedure: Color::Rgb(184, 187, 38),
            tree_function: Color::Rgb(142, 192, 124),
            grid_header_bg: Color::Rgb(60, 56, 54),
            grid_header_fg: Color::Rgb(235, 219, 178),
            grid_selected_bg: Color::Rgb(60, 56, 54),
            grid_selected_fg: Color::White,
            grid_row_even: Color::Reset,
            grid_row_odd: Color::Rgb(40, 40, 40),
            grid_null: Color::Rgb(102, 92, 84),
            grid_number: Color::Rgb(211, 134, 155),
            tab_active_fg: Color::Rgb(131, 165, 152),
            tab_active_bg: Color::Rgb(60, 56, 54),
            tab_inactive_fg: Color::Rgb(102, 92, 84),
            conn_connected: Color::Rgb(184, 187, 38),
            conn_disconnected: Color::Rgb(251, 73, 52),
            conn_connecting: Color::Rgb(250, 189, 47),
            status_bg: Color::Reset,
            status_fg: Color::Rgb(213, 196, 161),
            topbar_bg: Color::Reset,
            topbar_fg: Color::Rgb(235, 219, 178),
            sql_keyword: Color::Rgb(254, 128, 25),
            sql_string: Color::Rgb(184, 187, 38),
            sql_number: Color::Rgb(211, 134, 155),
            sql_comment: Color::Rgb(102, 92, 84),
            sql_operator: Color::Rgb(213, 196, 161),
            sql_bind_var: Color::Rgb(250, 189, 47),
            editor_line_nr: Color::Rgb(102, 92, 84),
            editor_line_nr_active: Color::Rgb(250, 189, 47),
            editor_cursor_line_bg: Color::Rgb(60, 56, 54),
            editor_bg: Color::Reset,
            dialog_bg: Color::Reset,
            dialog_field_active: Color::Rgb(131, 165, 152),
            dialog_field_inactive: Color::Rgb(102, 92, 84),
            error_fg: Color::Rgb(251, 73, 52),
            error_bg: Color::Rgb(60, 20, 20),
            accent: Color::Rgb(131, 165, 152),
            dim: Color::Rgb(102, 92, 84),
            separator: Color::Rgb(80, 73, 69),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            border_focused: Color::Rgb(100, 180, 255),
            border_unfocused: Color::Rgb(60, 60, 80),
            border_insert: Color::Rgb(100, 220, 100),
            mode_normal: Color::Rgb(60, 120, 220),
            mode_insert: Color::Rgb(60, 180, 80),
            mode_visual: Color::Rgb(200, 130, 255),
            tree_expanded: Color::Rgb(255, 200, 60),
            tree_collapsed: Color::Rgb(100, 100, 120),
            tree_selected_bg: Color::Rgb(30, 55, 90),
            tree_selected_fg: Color::White,
            tree_category: Color::Rgb(180, 180, 200),
            tree_connection: Color::Rgb(255, 180, 50),
            tree_schema: Color::Rgb(130, 170, 255),
            tree_table: Color::Rgb(255, 200, 60),
            tree_view: Color::Rgb(100, 180, 255),
            tree_package: Color::Rgb(200, 130, 255),
            tree_procedure: Color::Rgb(100, 220, 140),
            tree_function: Color::Rgb(80, 210, 220),
            grid_header_bg: Color::Rgb(35, 45, 65),
            grid_header_fg: Color::Rgb(200, 210, 230),
            grid_selected_bg: Color::Rgb(30, 55, 90),
            grid_selected_fg: Color::White,
            grid_row_even: Color::Reset,
            grid_row_odd: Color::Rgb(24, 24, 36),
            grid_null: Color::Rgb(80, 80, 100),
            grid_number: Color::Rgb(220, 180, 100),
            tab_active_fg: Color::Rgb(100, 180, 255),
            tab_active_bg: Color::Rgb(30, 45, 65),
            tab_inactive_fg: Color::Rgb(80, 80, 100),
            conn_connected: Color::Rgb(80, 220, 100),
            conn_disconnected: Color::Rgb(220, 80, 80),
            conn_connecting: Color::Rgb(255, 200, 60),
            status_bg: Color::Reset,
            status_fg: Color::Rgb(180, 185, 200),
            topbar_bg: Color::Reset,
            topbar_fg: Color::Rgb(200, 210, 230),
            sql_keyword: Color::Rgb(100, 150, 255),
            sql_string: Color::Rgb(100, 220, 140),
            sql_number: Color::Rgb(220, 180, 100),
            sql_comment: Color::Rgb(80, 80, 100),
            sql_operator: Color::Rgb(200, 200, 220),
            sql_bind_var: Color::Rgb(229, 192, 123),
            editor_line_nr: Color::Rgb(60, 60, 80),
            editor_line_nr_active: Color::Rgb(130, 170, 255),
            editor_cursor_line_bg: Color::Rgb(25, 30, 45),
            editor_bg: Color::Reset,
            dialog_bg: Color::Reset,
            dialog_field_active: Color::Rgb(100, 180, 255),
            dialog_field_inactive: Color::Rgb(120, 120, 140),
            error_fg: Color::Rgb(255, 100, 100),
            error_bg: Color::Rgb(60, 20, 20),
            accent: Color::Rgb(100, 180, 255),
            dim: Color::Rgb(70, 70, 90),
            separator: Color::Rgb(50, 50, 70),
        }
    }
}
