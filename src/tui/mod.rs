//! FRENTE TERMINAL (ratatui + crossterm).

mod app;
mod views;

use anyhow::Result;
use ratatui_image::picker::Picker;

use crate::core::db::Database;
use crate::core::state::AppState;

/// Lanza la TUI. `ratatui::init` instala un hook de pánico que restaura la terminal.
/// `notice` se muestra en la barra de estado al arrancar.
pub fn run(db: Database, notice: Option<String>) -> Result<()> {
    let mut state = AppState::load(&db)?;
    if let Some(n) = notice {
        state.error(n);
    }
    let mut terminal = ratatui::init();
    // ratatui-image pide consultar la terminal tras entrar en la pantalla alternativa y
    // antes de leer eventos. Si no responde, se usan bloques Unicode (sirve en cualquiera).
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let result = app::TuiApp::with_picker(db, state, picker).run(&mut terminal);
    ratatui::restore();
    result
}
