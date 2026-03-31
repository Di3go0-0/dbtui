use ratatui::style::{Color, Modifier, Style};

use crate::ui::state::Mode;

pub struct Theme {
    // Borders
    pub border_focused: Color,
    pub border_unfocused: Color,
    pub border_insert: Color,

    // Mode indicators
    pub mode_normal: Color,
    pub mode_insert: Color,

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

    // Editor
    pub editor_line_nr: Color,
    pub editor_line_nr_active: Color,
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
    pub fn border_style(&self, focused: bool, mode: &Mode) -> Style {
        let color = if !focused {
            self.border_unfocused
        } else {
            match mode {
                Mode::Insert => self.border_insert,
                Mode::Normal => self.border_focused,
            }
        };
        Style::default().fg(color)
    }

    pub fn mode_style(&self, mode: &Mode) -> Style {
        let bg = match mode {
            Mode::Normal => self.mode_normal,
            Mode::Insert => self.mode_insert,
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
        if row_idx % 2 == 0 {
            Style::default().bg(self.grid_row_even)
        } else {
            Style::default().bg(self.grid_row_odd)
        }
    }

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
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            // Borders
            border_focused: Color::Rgb(100, 180, 255),   // Bright blue
            border_unfocused: Color::Rgb(60, 60, 80),     // Subtle gray-blue
            border_insert: Color::Rgb(100, 220, 100),     // Bright green

            // Mode indicators
            mode_normal: Color::Rgb(60, 120, 220),        // Rich blue
            mode_insert: Color::Rgb(60, 180, 80),         // Rich green

            // Tree
            tree_expanded: Color::Rgb(255, 200, 60),      // Gold
            tree_collapsed: Color::Rgb(100, 100, 120),    // Muted gray
            tree_selected_bg: Color::Rgb(30, 55, 90),     // Deep blue
            tree_selected_fg: Color::White,
            tree_category: Color::Rgb(180, 180, 200),     // Light silver
            tree_connection: Color::Rgb(255, 180, 50),    // Orange
            tree_schema: Color::Rgb(130, 170, 255),       // Light blue
            tree_table: Color::Rgb(255, 200, 60),         // Gold
            tree_view: Color::Rgb(100, 180, 255),         // Blue
            tree_package: Color::Rgb(200, 130, 255),      // Purple
            tree_procedure: Color::Rgb(100, 220, 140),    // Green
            tree_function: Color::Rgb(80, 210, 220),      // Cyan

            // Grid
            grid_header_bg: Color::Rgb(35, 45, 65),       // Dark blue
            grid_header_fg: Color::Rgb(200, 210, 230),    // Light
            grid_selected_bg: Color::Rgb(30, 55, 90),     // Deep blue
            grid_selected_fg: Color::White,
            grid_row_even: Color::Rgb(18, 18, 28),        // Dark
            grid_row_odd: Color::Rgb(24, 24, 36),         // Slightly lighter
            grid_null: Color::Rgb(80, 80, 100),           // Dim
            grid_number: Color::Rgb(220, 180, 100),       // Warm yellow

            // Tabs
            tab_active_fg: Color::Rgb(100, 180, 255),     // Blue
            tab_active_bg: Color::Rgb(30, 45, 65),        // Subtle bg
            tab_inactive_fg: Color::Rgb(80, 80, 100),     // Dim

            // Connection
            conn_connected: Color::Rgb(80, 220, 100),     // Green
            conn_disconnected: Color::Rgb(220, 80, 80),   // Red
            conn_connecting: Color::Rgb(255, 200, 60),    // Yellow

            // Status / bars
            status_bg: Color::Rgb(20, 22, 32),            // Very dark
            status_fg: Color::Rgb(180, 185, 200),         // Light
            topbar_bg: Color::Rgb(22, 35, 55),            // Dark blue accent
            topbar_fg: Color::Rgb(200, 210, 230),         // Light

            // SQL syntax
            sql_keyword: Color::Rgb(100, 150, 255),       // Blue
            sql_string: Color::Rgb(100, 220, 140),        // Green
            sql_number: Color::Rgb(220, 180, 100),        // Yellow
            sql_comment: Color::Rgb(80, 80, 100),         // Dim
            sql_operator: Color::Rgb(200, 200, 220),      // Light

            // Editor
            editor_line_nr: Color::Rgb(60, 60, 80),       // Dim
            editor_line_nr_active: Color::Rgb(130, 170, 255), // Blue
            editor_cursor_line_bg: Color::Rgb(25, 30, 45),    // Subtle highlight
            editor_bg: Color::Rgb(15, 15, 22),            // Very dark

            // Dialog
            dialog_bg: Color::Rgb(22, 25, 38),            // Dark
            dialog_field_active: Color::Rgb(100, 180, 255),   // Blue
            dialog_field_inactive: Color::Rgb(120, 120, 140), // Gray
            error_fg: Color::Rgb(255, 100, 100),          // Red
            error_bg: Color::Rgb(60, 20, 20),             // Dark red

            // Accent
            accent: Color::Rgb(100, 180, 255),            // Primary blue
            dim: Color::Rgb(70, 70, 90),                  // Muted
            separator: Color::Rgb(50, 50, 70),            // Subtle
        }
    }
}
