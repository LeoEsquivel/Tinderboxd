//! FRENTE TERMINAL (ratatui + crossterm).

mod app;
mod views;

use anyhow::Result;

use crate::core::db::Database;
use crate::core::state::AppState;

/// Lanza la TUI. `ratatui::init` instala un hook de pánico que restaura la terminal.
pub fn run(db: Database) -> Result<()> {
    let state = AppState::load(&db)?;
    let mut terminal = ratatui::init();
    let result = app::TuiApp::new(db, state).run(&mut terminal);
    ratatui::restore();
    result
}
