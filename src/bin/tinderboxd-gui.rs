//! Tinderboxd en ventana, sin consola: el ejecutable de los accesos directos y
//! del instalador. Para la TUI o la línea de comandos usa `tinderboxd`.

// En Windows, sin esto aparecería una consola negra detrás de la ventana.
#![cfg_attr(windows, windows_subsystem = "windows")]

use tinderboxd::core::db::{Database, default_db_path};

fn main() {
    if let Err(e) = run() {
        // Sin consola no se vería el error: mostrarlo en un diálogo.
        let message = format!("{e:#}");
        eprintln!("{message}");
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Tinderboxd")
            .set_description(message)
            .show();
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let path = default_db_path();
    let is_new = !path.exists();
    let db = Database::open(&path)?;
    // Igual que `tinderboxd`: los datos de ejemplo sólo al crear la base de datos.
    if is_new {
        db.seed_if_empty()?;
    }
    tinderboxd::gui::run(db, None)
}
