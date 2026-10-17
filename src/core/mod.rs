//! CEREBRO de Tinderboxd: modelos, SQLite y estado compartido.
//!
//! Regla de oro: nada en este módulo importa tipos de `egui` ni de `ratatui`.

pub mod db;
pub mod models;
pub mod state;
