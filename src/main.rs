mod core;
mod gui;
mod tui;

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{ArgGroup, Parser};

use crate::core::db::{DB_FILE_NAME, Database, default_db_path};
use crate::core::export::{self, ExportOptions, ExportTarget, ImportOptions};
use crate::core::models::{format_score_dots, format_stars};
use crate::core::state::AppState;

/// Tinderboxd: tu Letterboxd de matches y citas.
#[derive(Parser, Debug)]
#[command(name = "tinderboxd", version, about)]
#[command(group(ArgGroup::new("modo").args(["tui", "gui", "resumen", "exportar", "importar"])))]
struct Cli {
    /// Lanza la interfaz de terminal (ratatui). Es la opción por defecto.
    #[arg(long)]
    tui: bool,

    /// Lanza la interfaz de escritorio (egui).
    #[arg(long)]
    gui: bool,

    /// Imprime en consola un resumen de los datos y las estadísticas, y termina.
    #[arg(long)]
    resumen: bool,

    /// Exporta a un archivo JSON (todo, o un match/lista con --solo-match/--solo-lista).
    #[arg(long, value_name = "ARCHIVO")]
    exportar: Option<PathBuf>,

    /// Con --exportar: sólo este match (id o parte del alias).
    #[arg(long, value_name = "MATCH", requires = "exportar", conflicts_with = "solo_lista")]
    solo_match: Option<String>,

    /// Con --exportar: sólo esta lista y sus matches (id o parte del título).
    #[arg(long, value_name = "LISTA", requires = "exportar")]
    solo_lista: Option<String>,

    /// Con --exportar: no incluir las fotos en el JSON.
    #[arg(long, requires = "exportar")]
    sin_fotos: bool,

    /// Importa un archivo JSON exportado por Tinderboxd.
    #[arg(long, value_name = "ARCHIVO")]
    importar: Option<PathBuf>,

    /// Con --importar: prefijo para los alias (p. ej. el nombre de tu amigo).
    #[arg(long, value_name = "NOMBRE", requires = "importar")]
    de: Option<String>,

    /// Ruta del archivo SQLite. Por defecto, en la carpeta de datos del usuario.
    #[arg(long, default_value_os_t = default_db_path())]
    db: PathBuf,
}

/// Aviso si en la carpeta actual hay una base de datos de antes de usar la
/// carpeta de datos del usuario (cuando se creaba junto a donde se ejecutaba).
fn legacy_db_notice(cli: &Cli, is_new: bool) -> Option<String> {
    let legacy = Path::new(DB_FILE_NAME);
    let using_default = cli.db == default_db_path();
    (using_default && is_new && legacy.is_file()).then(|| {
        format!(
            "Hay un {DB_FILE_NAME} en esta carpeta, pero ahora los datos viven en {}. \
             Ábrelo con --db {DB_FILE_NAME} o muévelo allí (con su carpeta de fotos).",
            cli.db.display()
        )
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let is_new = !cli.db.exists();
    let notice = legacy_db_notice(&cli, is_new);
    let db = Database::open(&cli.db)?;
    if let Some(n) = &notice
        && (cli.exportar.is_some() || cli.importar.is_some() || cli.resumen)
    {
        eprintln!("⚠ {n}");
    }

    if let Some(path) = &cli.exportar {
        let target = match (&cli.solo_match, &cli.solo_lista) {
            (Some(q), _) => ExportTarget::Match(export::find_match(&db, q)?),
            (_, Some(q)) => ExportTarget::List(export::find_list(&db, q)?),
            _ => ExportTarget::All,
        };
        let opts = ExportOptions { include_photos: !cli.sin_fotos };
        let bundle = export::export_to_file(&db, target, opts, path)?;
        println!("✔ {}", bundle.describe());
        println!("  Guardado en {}", path.display());
        return Ok(());
    }
    if let Some(path) = &cli.importar {
        let report = export::import_file(&db, path, &ImportOptions { alias_prefix: cli.de.clone() })?;
        println!("✔ {}", report.summary());
        return Ok(());
    }

    // Los datos de ejemplo sólo se insertan al crear la base de datos: si borras
    // todos tus matches no deben reaparecer, y nunca se mezclan con una importación.
    let seeded = is_new && db.seed_if_empty()?;

    if cli.resumen {
        if seeded {
            println!("🌱 Base de datos nueva: datos de ejemplo insertados en {}", cli.db.display());
        }
        print_summary(&AppState::load(&db)?);
        Ok(())
    } else if cli.gui {
        gui::run(db, notice)
    } else {
        tui::run(db, notice)
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
