//! Tinderboxd: tu Letterboxd de matches y citas.
//!
//! Biblioteca compartida por los dos ejecutables:
//! - `tinderboxd` (src/main.rs): TUI, línea de comandos y `--gui`.
//! - `tinderboxd-gui` (src/bin/tinderboxd-gui.rs): sólo la ventana, sin consola
//!   (accesos directos, instalador y doble clic en Windows).

pub mod core;
pub mod gui;
pub mod tui;
