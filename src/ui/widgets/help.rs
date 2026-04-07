use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::keybindings::Context;
use crate::ui::state::AppState;
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, state: &AppState, theme: &Theme) {
    let area = frame.area();
    let dialog = centered_rect(64, 50, area);

    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .title(" Help - Keybindings ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused))
        .style(Style::default().bg(Color::Reset));

    let header = Style::default()
        .fg(theme.tab_active_fg)
        .add_modifier(Modifier::BOLD);
    let key_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(theme.status_fg);

    // Helper to resolve the primary key for a (context, action) pair. Falls
    // back to "?" if nothing is bound, which keeps the help popup aligned
    // even when users leave an action unbound.
    let pk = |ctx: Context, action: &str| state.bindings.primary_key(ctx, action);
    // Two keys in a row, joined with " / ".
    let two = |a: String, b: String| format!("{a} / {b}");
    // Leader combo: "Space <suffix>".
    let spc = |suffix: &str| format!("Space {suffix}");

    let row = |k: String, d: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {k:<16}"), key_style),
            Span::styled(d, desc_style),
        ])
    };

    let lines = vec![
        Line::from(Span::styled(" Navigation", header)),
        row(
            format!(
                "{}/{}/{}/{}",
                pk(Context::Sidebar, "collapse_or_parent"),
                pk(Context::Sidebar, "scroll_down"),
                pk(Context::Sidebar, "scroll_up"),
                pk(Context::Sidebar, "expand_or_open"),
            ),
            "Move cursor",
        ),
        row(
            two(
                pk(Context::Global, "navigate_left"),
                pk(Context::Global, "navigate_right"),
            ),
            "Switch panels",
        ),
        row(
            two(
                pk(Context::Global, "next_tab"),
                pk(Context::Global, "prev_tab"),
            ),
            "Next / prev tab",
        ),
        row(
            two(
                pk(Context::Global, "next_sub_view"),
                pk(Context::Global, "prev_sub_view"),
            ),
            "Next / prev sub-view",
        ),
        row(
            two(
                pk(Context::Sidebar, "scroll_top"),
                pk(Context::Sidebar, "scroll_bottom"),
            ),
            "Top / bottom",
        ),
        row(
            two(
                pk(Context::Sidebar, "half_page_down"),
                pk(Context::Sidebar, "half_page_up"),
            ),
            "Half page down / up",
        ),
        row(
            pk(Context::Sidebar, "start_search"),
            "Search sidebar tree",
        ),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Editor", header)),
        row("i / a / o".to_string(), "Enter insert mode"),
        row("Esc".to_string(), "Back to normal mode"),
        row("Ctrl+S".to_string(), "Save / validate"),
        row("Space Space s".to_string(), "Compile to database"),
        row(
            pk(Context::Leader, "execute_query"),
            "Execute query (leader)",
        ),
        row(
            pk(Context::Leader, "execute_query_new_tab"),
            "Execute \u{2192} new tab",
        ),
        row("v / V".to_string(), "Visual / visual line mode"),
        row("u / Ctrl+r".to_string(), "Undo / redo"),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Explorer (Sidebar & Oil)", header)),
        row(
            pk(Context::Sidebar, "expand_or_open"),
            "Open / expand selected",
        ),
        row(
            pk(Context::Sidebar, "collapse_or_parent"),
            "Collapse / parent",
        ),
        row(
            pk(Context::Sidebar, "create_new"),
            "New connection / object from template",
        ),
        row(
            pk(Context::Sidebar, "group_menu"),
            "Group menu (new collection, rename, delete)",
        ),
        row(
            pk(Context::Sidebar, "rename_or_refresh"),
            "Rename / refresh",
        ),
        row(
            pk(Context::Sidebar, "delete_pending"),
            "Delete connection / object (dd)",
        ),
        row(
            pk(Context::Sidebar, "yank_pending"),
            "Yank connection (for duplicate)",
        ),
        row(pk(Context::Sidebar, "paste"), "Paste yanked connection"),
        row(pk(Context::Sidebar, "start_search"), "Search tree"),
        row(
            pk(Context::Global, "filter_objects"),
            "Filter objects (per category)",
        ),
        row(
            pk(Context::Oil, "open_in_split"),
            "Open in vertical split (oil)",
        ),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Tabs & Views", header)),
        row(
            pk(Context::Sidebar, "expand_or_open"),
            "Open object from tree",
        ),
        row(
            spc(&format!(
                "b {}",
                pk(Context::LeaderBuffer, "close_tab")
            )),
            "Close buffer",
        ),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Scripts Panel", header)),
        row(
            pk(Context::Scripts, "create_new"),
            "New (name/ = folder)",
        ),
        row(pk(Context::Scripts, "delete_pending"), "Delete"),
        row(pk(Context::Scripts, "rename"), "Rename"),
        row(pk(Context::Scripts, "yank_pending"), "Yank (copy)"),
        row(pk(Context::Scripts, "paste"), "Paste (move)"),
        row(
            pk(Context::Scripts, "expand_or_open"),
            "Open / expand",
        ),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Diagnostics", header)),
        row(
            pk(Context::Global, "next_diagnostic"),
            "Next error",
        ),
        row(
            pk(Context::Global, "prev_diagnostic"),
            "Previous error",
        ),
        row("K".to_string(), "Show error details"),
        row(
            spc(&pk(Context::Leader, "toggle_diagnostic_list")),
            "Toggle error list",
        ),
        row("gcc".to_string(), "Toggle line comment"),
        row("gc (visual)".to_string(), "Toggle block comment"),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Tab Groups (Split)", header)),
        row(
            spc(&pk(Context::Leader, "vertical_split")),
            "Create vertical split",
        ),
        row(
            two(
                pk(Context::Global, "navigate_left"),
                pk(Context::Global, "navigate_right"),
            ),
            "Switch between groups",
        ),
        row(
            spc(&pk(Context::Leader, "move_tab_to_other_group")),
            "Move tab to other group",
        ),
        row(
            spc(&format!(
                "w {}",
                pk(Context::LeaderWindow, "close_group")
            )),
            "Close current group",
        ),
        Line::from(Span::raw("")),
        Line::from(Span::styled(" Global", header)),
        row(
            spc(&pk(Context::Leader, "toggle_sidebar")),
            "Toggle sidebar",
        ),
        row(
            pk(Context::Global, "toggle_oil_navigator"),
            "Toggle floating navigator",
        ),
        row(
            spc(&format!(
                "f {}",
                pk(Context::LeaderFile, "export_connections")
            )),
            "Export connections",
        ),
        row(
            spc(&format!(
                "f {}",
                pk(Context::LeaderFile, "import_connections")
            )),
            "Import connections",
        ),
        row(pk(Context::Global, "add_connection"), "Add connection"),
        row(
            pk(Context::Global, "filter_objects"),
            "Filter objects",
        ),
        row(pk(Context::Global, "help"), "Toggle this help"),
        row(":q".to_string(), "Close tab"),
        row(
            spc(&format!(
                "q {}",
                pk(Context::LeaderQuit, "quit_app")
            )),
            "Quit app",
        ),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            " Press Esc or ? to close",
            Style::default().fg(theme.grid_null),
        )),
    ];

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, dialog);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
