// Parte del CRUD de lectura de `core` (get_match, logs_for_match…) sólo la usan los
// tests por ahora; la consumirá `export.rs` en la Fase 4.
#[allow(dead_code)]
mod core;
mod gui;
mod tui;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use crate::core::db::{DEFAULT_DB_PATH, Database};
use crate::core::models::{format_score_dots, format_stars};
use crate::core::state::AppState;

/// Tinderboxd: tu Letterboxd de matches y citas.
#[derive(Parser, Debug)]
#[command(name = "tinderboxd", version, about)]
struct Cli {
    /// Lanza la interfaz de terminal (ratatui). Es la opción por defecto.
    #[arg(long, conflicts_with_all = ["gui", "resumen"])]
    tui: bool,

    /// Lanza la interfaz de escritorio (egui).
    #[arg(long, conflicts_with = "resumen")]
    gui: bool,

    /// Imprime en consola un resumen de los datos y las estadísticas, y termina.
    #[arg(long)]
    resumen: bool,

    /// Ruta del archivo SQLite.
    #[arg(long, default_value = DEFAULT_DB_PATH)]
    db: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let db = Database::open(&cli.db)?;
    let seeded = db.seed_if_empty()?;

    if cli.resumen {
        if seeded {
            println!("🌱 Base de datos nueva: datos de ejemplo insertados en {}", cli.db.display());
        }
        print_summary(&AppState::load(&db)?);
        Ok(())
    } else if cli.gui {
        gui::run(db)
    } else {
        tui::run(db)
    }
}

/// Volcado de los datos en consola (`--resumen`).
fn print_summary(state: &AppState) {
    println!("\n══════════════ TINDERBOXD · resumen ══════════════");

    println!("\n★ TOP 4 FAVORITOS");
    for card in state.favorites() {
        println!("  ♥ {}  {}", card.profile.headline(), card.avg_stars());
    }

    println!("\n💘 MATCHES ({})", state.cards.len());
    for card in &state.cards {
        let p = &card.profile;
        println!(
            "  [{:>2}] {:<32} {:<7} {:<15} {:<6} {} {}",
            p.id,
            p.alias,
            p.platform.as_str(),
            p.status.as_str(),
            card.avg_stars(),
            if card.liked { "♥" } else { " " },
            if p.is_favorite { "(Top 4)" } else { "" },
        );
        if let Some(bio) = &p.bio_highlight {
            println!("       “{bio}”");
        }
        if !card.tags.is_empty() {
            let tags: Vec<String> = card.tags.iter().map(|t| t.label()).collect();
            println!("       {}", tags.join("  "));
        }
    }

    println!("\n⏳ WATCHLIST");
    for card in state.watchlist_cards() {
        println!("  · {}", card.profile.headline());
    }

    println!("\n📔 DIARY ({} entradas)", state.diary.len());
    for e in &state.diary {
        let l = &e.log;
        println!(
            "  {}  {:<30} {:<13} {:<6} {}{}",
            l.date_logged,
            e.match_alias,
            l.interaction_type.as_str(),
            l.rating.stars(),
            l.heart(),
            if l.contains_spoilers { "  ⚠ spoilers" } else { "" },
        );
        println!("             {}", l.sub_scores_line());
        if let Some(text) = &l.review_text {
            println!("             {}", truncate(text, 100));
        }
    }

    println!("\n📜 LISTAS");
    for list in &state.lists {
        println!(
            "  {} {}",
            list.title,
            if list.is_ranked { "(ranking)" } else { "" }
        );
        if let Some(desc) = &list.description {
            println!("    {desc}");
        }
        for entry in state.entries_of_list(list.id) {
            println!(
                "    {}. {} — {}",
                entry.item.rank_order,
                entry.profile.alias,
                entry.item.item_note.as_deref().unwrap_or("")
            );
        }
    }

    let s = &state.stats;
    println!("\n📊 STATS");
    println!(
        "  {} matches · {} reseñas · {} con ♥ · promedio {} ({:.1}/5)",
        s.total_matches,
        s.total_logs,
        s.liked_logs,
        s.average_stars(),
        s.average_rating.unwrap_or(0.0) / 2.0
    );

    println!("\n  Distribución de calificaciones");
    let max = s.max_bucket().max(1);
    for (i, count) in s.rating_distribution.iter().enumerate() {
        let bar = "█".repeat((*count * 20 / max) as usize);
        println!("  {:>6} │{:<20} {}", format_stars(i as u8 + 1), bar, count);
    }

    println!("\n  Embudo de supervivencia");
    for step in &s.funnel {
        let bar = "▇".repeat((step.pct_of_total / 5.0).round() as usize);
        println!("  {:<13} {:<20} {:>2} ({:>5.1}%)", step.label, bar, step.count, step.pct_of_total);
    }
    println!(
        "  Chat → Primera Cita: {:.0}%   ·   Chat → Ghosting: {:.0}%",
        s.chat_to_date_pct.unwrap_or(0.0),
        s.ghosting_pct.unwrap_or(0.0)
    );

    println!("\n  Por estado");
    for (status, count) in &s.status_counts {
        println!("  {:<15} {}", status.as_str(), count);
    }

    println!("\n  Top Red Flags");
    for tc in &s.top_red_flags {
        println!("  {} ×{}", tc.tag.label(), tc.count);
    }
    println!("\n  Top Green Flags");
    for tc in &s.top_green_flags {
        println!("  {} ×{}", tc.tag.label(), tc.count);
    }

    println!("\n  Leyenda sub-métricas: 3/5 = {}", format_score_dots(3));
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max_chars).collect();
        format!("{cut}…")
    }
}
