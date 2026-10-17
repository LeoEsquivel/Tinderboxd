//! Implementación de `eframe::App`: reparte el frame entre paneles y vistas.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui;

use crate::core::db::Database;
use crate::core::state::{AppState, StatusMessage, Tab};

use super::views;

/// Segundos que permanece visible un mensaje de estado.
const STATUS_TTL: Duration = Duration::from_secs(5);

pub struct GuiApp {
    pub db: Database,
    pub state: AppState,
    /// Logs cuyos spoilers se han destapado.
    pub revealed: HashSet<i64>,
    status_seen: Option<(StatusMessage, Instant)>,
}

impl GuiApp {
    pub fn new(db: Database, state: AppState) -> Self {
        GuiApp { db, state, revealed: HashSet::new(), status_seen: None }
    }

    /// Ejecuta una acción sobre estado + BD y convierte los errores en mensaje de estado.
    pub fn act(&mut self, f: impl FnOnce(&mut AppState, &Database) -> Result<()>) {
        if let Err(e) = f(&mut self.state, &self.db) {
            self.state.error(format!("{e:#}"));
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.state.modal.is_some() {
            return;
        }
        let (new_match, new_review) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::N),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::R),
            )
        });
        if new_match {
            self.state.open_new_match();
        }
        if new_review {
            self.state.open_new_review();
        }
        const TAB_KEYS: [egui::Key; 6] =
            [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6];
        for (key, tab) in TAB_KEYS.into_iter().zip(Tab::ALL) {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, key)) {
                self.state.set_tab(tab);
            }
        }
    }

    /// Oculta el mensaje de estado tras unos segundos (los errores de formulario se quedan).
    fn expire_status(&mut self, ctx: &egui::Context) {
        let Some(current) = self.state.status.clone() else {
            self.status_seen = None;
            return;
        };
        match &self.status_seen {
            Some((seen, since)) if *seen == current => {
                if since.elapsed() >= STATUS_TTL && self.state.modal.is_none() {
                    self.state.clear_status();
                    self.status_seen = None;
                }
            }
            _ => self.status_seen = Some((current, Instant::now())),
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

impl eframe::App for GuiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_shortcuts(&ctx);
        self.expire_status(&ctx);

        egui::Panel::top("header")
            .frame(views::header_frame())
            .show(ui, |ui| views::header(ui, self));
        egui::Panel::bottom("status")
            .frame(views::status_frame())
            .show(ui, |ui| views::status_bar(ui, self));

        match self.state.active_tab {
            Tab::Dashboard => views::dashboard(ui, self),
            Tab::Cartelera => views::matches(ui, self),
            Tab::Watchlist => views::watchlist(ui, self),
            Tab::Diary => views::diary(ui, self),
            Tab::Lists => views::lists(ui, self),
            Tab::Stats => views::stats(ui, self),
        }

        views::modal(&ctx, self);
    }
}
