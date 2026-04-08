use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use vimltui::VimEditor;

use crate::ui::theme::Theme;

/// Build the "Fetching data... X.X s" text with animated dots.
pub fn fetching_text(since: Option<std::time::Instant>) -> String {
    let dots = match (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        / 400)
        % 4
    {
        0 => "   ",
        1 => ".  ",
        2 => ".. ",
        _ => "...",
    };
    let elapsed = since.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
    format!("  Fetching data{dots} {elapsed:.1} s")
}

/// Render a loading indicator panel with animated dots and elapsed timer.
pub fn render_loading(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    title: &str,
    since: Option<std::time::Instant>,
) {
    render_loading_with_focus(frame, theme, area, title, since, false);
}

/// Like `render_loading` but takes a `focused` flag so the placeholder
/// can highlight its border when the user navigates into it (needed so
/// the "cancel by closing the result pane" UX is discoverable — the
/// user can see which pane has focus before pressing the close key).
pub fn render_loading_with_focus(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    title: &str,
    since: Option<std::time::Instant>,
    focused: bool,
) {
    let border_color = if focused {
        theme.border_focused
    } else {
        theme.border_unfocused
    };
    let title_bar = if focused {
        format!(" {title}  [cancel: close] ")
    } else {
        format!(" {title} ")
    };
    let block = Block::default()
        .title(title_bar)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme.editor_bg));

    let msg = fetching_text(since);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            msg,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    let content = Paragraph::new(lines).block(block);
    frame.render_widget(content, area);
}

/// Render a VimEditor, or show "Fetching data..." if actively loading (streaming_since is Some).
/// If the editor is empty and NOT loading, renders the editor normally (content is genuinely empty).
pub fn render_editor_or_loading(
    frame: &mut Frame,
    editor: &mut VimEditor,
    focused: bool,
    theme: &Theme,
    area: Rect,
    title: &str,
    streaming_since: Option<std::time::Instant>,
) {
    let is_empty = editor.lines.len() == 1 && editor.lines[0].is_empty();
    if is_empty && streaming_since.is_some() {
        render_loading(frame, theme, area, title, streaming_since);
    } else {
        vimltui::render::render(
            frame,
            editor,
            focused,
            &theme.vim_theme(),
            &crate::ui::sql_highlighter::SqlHighlighter::from_theme(theme),
            area,
            title,
        );
    }
}
