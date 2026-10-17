//! FRENTE ESCRITORIO (eframe / egui).

mod app;
mod views;

use anyhow::{Result, anyhow};

use crate::core::db::Database;
use crate::core::state::AppState;

/// Abre la ventana nativa de Tinderboxd.
pub fn run(db: Database) -> Result<()> {
    let state = AppState::load(&db)?;
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Tinderboxd")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Tinderboxd",
        options,
        Box::new(|cc| {
            views::apply_theme(&cc.egui_ctx);
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(app::GuiApp::new(db, state)))
        }),
    )
    .map_err(|e| anyhow!("no se pudo abrir la ventana: {e}"))
}
