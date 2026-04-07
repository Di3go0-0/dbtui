mod core;
mod drivers;
mod keybindings;
mod sql_engine;
mod ui;

use std::io;
use std::sync::Arc;

use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::ui::app::App;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("dbtui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--dump-keybindings") {
        // Write the default keybindings file to the user's config dir
        // (creating it if needed) and print the path so they can find it.
        match keybindings::write_default_config() {
            Ok(path) => {
                println!("Wrote default keybindings to: {}", path.display());
                println!(
                    "Edit this file to override any binding. See KEYBINDINGS.md \
                     for the format and the full list of actions."
                );
                return Ok(());
            }
            Err(e) => {
                eprintln!("Failed to write keybindings file: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.iter().any(|a| a == "--print-keybindings") {
        // Print to stdout without touching disk — useful for piping.
        print!("{}", keybindings::KeyBindings::defaults().to_toml());
        return Ok(());
    }

    let mut app = App::new();

    // Load saved connections and theme from disk
    app.load_saved_connections();
    app.load_theme_preference();

    // If env var set, auto-connect (for quick usage)
    if let Ok(conn_str) = std::env::var("DBTUI_POSTGRES_URL") {
        match drivers::PostgresAdapter::connect(&conn_str).await {
            Ok(adapter) => {
                app.add_connection(Arc::new(adapter), "postgres");
            }
            Err(e) => {
                eprintln!("Connection failed: {e}");
            }
        }
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    // Enable keyboard enhancement so the terminal distinguishes keys that
    // share escape codes in legacy mode (e.g. Ctrl+Delete vs Ctrl+H).
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run app
    let result = app.run(&mut terminal).await;

    // Restore terminal
    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        crossterm::cursor::SetCursorStyle::DefaultUserShape,
        DisableBracketedPaste,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Application error: {e}");
    }

    Ok(())
}
