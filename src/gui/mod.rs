//! FRENTE ESCRITORIO (eframe / egui).

mod app;
mod views;

use anyhow::{Result, anyhow};

use crate::core::db::Database;
use crate::core::state::AppState;

/// Abre la ventana nativa de Tinderboxd.
pub fn run(db: Database) -> Result<()> {
    let state = AppState::load(&db)?;
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Tinderboxd")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    #[cfg(target_os = "linux")]
    prefer_x11_on_wsl(&mut options);

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

/// El compositor Wayland de WSLg corta la conexión de winit ("Connection reset by
/// peer"); su servidor X11 funciona bien. En Linux nativo se respeta la elección de winit.
#[cfg(target_os = "linux")]
fn prefer_x11_on_wsl(options: &mut eframe::NativeOptions) {
    if std::env::var_os("WSL_DISTRO_NAME").is_some() && std::env::var_os("DISPLAY").is_some() {
        options.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        }));
    }
}
