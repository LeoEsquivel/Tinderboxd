//! Renderizado de la TUI: pestañas, cuadrícula de matches, tablas, gráficas y modales.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Bar, BarChart, BarGroup, Block, BorderType, Borders, Cell, Clear, List, ListItem, ListState,
    Padding, Paragraph, Row, Table, TableState, Wrap,
};

use crate::core::models::{
    DiaryEntry, MatchCard, MatchStatus, OwnerFilter, Tag, TagCategory, format_score_dots,
};
use crate::core::state::{Modal, SortBy, Tab, today};

use ratatui_image::{Resize, StatefulImage};
use ratatui_image::protocol::StatefulProtocol;

use super::app::{Field, FieldKind, ListPane, TuiApp, form_fields};

// ---------------------------------------------------------------------------
// Paleta Letterboxd
// ---------------------------------------------------------------------------

const BG: Color = Color::Rgb(0x14, 0x18, 0x1c);
const PANEL: Color = Color::Rgb(0x1c, 0x22, 0x28);
const BORDER: Color = Color::Rgb(0x2c, 0x34, 0x40);
const HIGHLIGHT: Color = Color::Rgb(0x2c, 0x34, 0x40);
const TEXT: Color = Color::Rgb(0x99, 0xaa, 0xbb);
const BRIGHT: Color = Color::Rgb(0xdd, 0xe6, 0xee);
const DIM: Color = Color::Rgb(0x67, 0x78, 0x89);
const GREEN: Color = Color::Rgb(0x00, 0xe0, 0x54);
const ORANGE: Color = Color::Rgb(0xff, 0x80, 0x00);
const BLUE: Color = Color::Rgb(0x40, 0xbc, 0xf4);
const RED: Color = Color::Rgb(0xff, 0x55, 0x55);

/// Ancho y alto de una tarjeta de la cuadrícula de Matches.
const CARD_W: u16 = 28;
const CARD_H: u16 = 6;

/// Celdas para la foto de la ficha (las celdas son ~1:2, así que queda vertical).
const PHOTO_W: u16 = 18;
const PHOTO_H: u16 = 12;

fn s(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(text.into(), Style::new().fg(color))
}

fn bold(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(text.into(), Style::new().fg(color).add_modifier(Modifier::BOLD))
}

fn panel(title: &str, focused: bool) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if focused { GREEN } else { BORDER }))
        .title(Line::from(bold(format!(" {title} "), BRIGHT)))
        .padding(Padding::horizontal(1))
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn status_color(status: MatchStatus) -> Color {
    match status {
        MatchStatus::Watchlist => BLUE,
        MatchStatus::Activa => GREEN,
        MatchStatus::SoloAmigos => TEXT,
        MatchStatus::Unmatched => DIM,
        MatchStatus::Ghosteado | MatchStatus::GhostingMutuo => RED,
    }
}

fn tag_color(category: TagCategory) -> Color {
    match category {
        TagCategory::RedFlag => RED,
        TagCategory::GreenFlag => GREEN,
        TagCategory::Neutral => TEXT,
    }
}

/// Flag con marcador de ancho fijo: los emojis descuadran bordes en algunas terminales.
fn tag_span(tag: &Tag) -> Span<'static> {
    s(format!("● {}", tag.name), tag_color(tag.category))
}

fn heart(liked: bool) -> Span<'static> {
    if liked { s(" ♥", ORANGE) } else { s("", TEXT) }
}

fn empty_message(f: &mut Frame, area: Rect, text: &str) {
    let p = Paragraph::new(s(text, DIM)).alignment(Alignment::Center);
    f.render_widget(p, area.centered_vertically(Constraint::Length(1)));
}

fn table_state(selected: usize, len: usize) -> TableState {
    TableState::default().with_selected((len > 0).then_some(selected))
}

// ---------------------------------------------------------------------------
// Raíz
// ---------------------------------------------------------------------------

pub fn render(f: &mut Frame, app: &TuiApp) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().bg(BG).fg(TEXT)), area);

    let [header, body, footer] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(0), Constraint::Length(1)]).areas(area);

    render_header(f, header, app);
    match app.state.active_tab {
        Tab::Dashboard => render_dashboard(f, body, app),
        Tab::Cartelera => render_matches(f, body, app),
        Tab::Watchlist => render_watchlist(f, body, app),
        Tab::Diary => render_diary(f, body, app),
        Tab::Lists => render_lists(f, body, app),
        Tab::Stats => render_stats(f, body, app),
    }
    render_footer(f, footer, app);

    if let Some(modal) = &app.state.modal {
        render_modal(f, area, app, modal);
    }
    if app.show_help {
        render_help(f, area);
    }
}

fn render_header(f: &mut Frame, area: Rect, app: &TuiApp) {
    let mut spans = vec![
        s(" ●", ORANGE),
        s("●", GREEN),
        s("● ", BLUE),
        bold("TINDERBOXD", BRIGHT),
        s("   ", TEXT),
    ];
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let active = *tab == app.state.active_tab;
        spans.push(s(format!("{} ", i + 1), DIM));
        let style = if active {
            Style::new().fg(BG).bg(GREEN).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(TEXT)
        };
        spans.push(Span::styled(format!(" {} ", tab.title()), style));
        spans.push(s("  ", TEXT));
    }

    // Perfil visible, a la derecha.
    let read_only = app.state.is_read_only_view();
    let mut profile = vec![
        s("[u] Perfil: ", DIM),
        bold(app.state.filter_label(), if read_only { ORANGE } else { GREEN }),
    ];
    if read_only {
        profile.push(s(" · sólo lectura", ORANGE));
    }
    let profile = Line::from(profile);

    let block = Block::new().borders(Borders::BOTTOM).border_style(Style::new().fg(BORDER));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [tabs_area, profile_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(profile.width() as u16 + 1)]).areas(inner);
    f.render_widget(Paragraph::new(Line::from(spans)), tabs_area);
    f.render_widget(Paragraph::new(profile), profile_area);
}

fn render_footer(f: &mut Frame, area: Rect, app: &TuiApp) {
    if let Some(msg) = &app.state.status
        && app.state.modal.is_none()
    {
        let color = if msg.is_error { RED } else { GREEN };
        f.render_widget(Paragraph::new(bold(format!(" {}", msg.text), color)), area);
        return;
    }

    let hints: &[(&str, &str)] = if app.searching {
        &[("Enter", "aplicar"), ("Esc", "borrar búsqueda"), ("Ctrl+U", "limpiar")]
    } else if app.state.is_read_only_view() {
        &[
            ("u", "cambiar perfil"),
            ("X", "borrar este perfil"),
            ("E", "exportar"),
            ("hjkl", "mover"),
            ("Tab", "pestaña"),
            ("?", "ayuda"),
            ("q", "salir"),
        ]
    } else {
        match app.state.active_tab {
            Tab::Dashboard | Tab::Diary => &[
                ("j/k", "mover"),
                ("r", "registrar"),
                ("e", "editar"),
                ("d", "borrar"),
                ("v", "spoilers"),
                ("Tab", "pestaña"),
                ("?", "ayuda"),
                ("q", "salir"),
            ],
            Tab::Cartelera => &[
                ("hjkl", "mover"),
                ("n", "nuevo"),
                ("r", "registrar"),
                ("e", "editar"),
                ("f", "Top 4"),
                ("s", "estado"),
                ("/", "buscar"),
                ("p c o", "filtros"),
                ("?", "ayuda"),
            ],
            Tab::Watchlist => &[
                ("j/k", "mover"),
                ("n", "nuevo"),
                ("r", "registrar"),
                ("s", "estado"),
                ("e", "editar"),
                ("/", "buscar"),
                ("?", "ayuda"),
                ("q", "salir"),
            ],
            Tab::Lists if app.list_pane == ListPane::Entries => &[
                ("j/k", "mover"),
                ("J/K", "reordenar"),
                ("x", "quitar"),
                ("r", "registrar"),
                ("Esc", "volver"),
            ],
            Tab::Lists => &[
                ("j/k", "mover"),
                ("Enter", "ver items"),
                ("n", "nueva"),
                ("a", "añadir match"),
                ("e", "editar"),
                ("d", "borrar"),
                ("?", "ayuda"),
            ],
            Tab::Stats => &[
                ("Tab", "pestaña"),
                ("E", "exportar todo"),
                ("I", "importar"),
                ("?", "ayuda"),
                ("q", "salir"),
            ],
        }
    };
    let mut spans = vec![s(" ", TEXT)];
    for (key, label) in hints {
        spans.push(bold(*key, BLUE));
        spans.push(s(format!(" {label}   "), DIM));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------------------------------------------------------------------------
// Dashboard / Perfil
// ---------------------------------------------------------------------------

fn render_dashboard(f: &mut Frame, area: Rect, app: &TuiApp) {
    let st = &app.state;
    let [top, bottom] = Layout::vertical([Constraint::Length(CARD_H + 3), Constraint::Min(0)]).areas(area);

    // Top 4
    let title = format!(
        "Top 4 · {} matches · {} reseñas · promedio {}",
        st.stats.total_matches,
        st.stats.total_logs,
        st.stats.average_stars()
    );
    let block = panel(&title, false);
    let inner = block.inner(top);
    f.render_widget(block, top);
    let slots = Layout::horizontal([Constraint::Ratio(1, 4); 4]).spacing(1).split(inner);
    let favorites = st.favorites();
    for (i, slot) in slots.iter().enumerate() {
        match favorites.get(i) {
            Some(card) => render_card(f, *slot, card, false, owner_tag(app, card.profile.owner_id)),
            None => {
                let b = Block::bordered()
                    .border_type(BorderType::Plain)
                    .border_style(Style::new().fg(BORDER));
                let inner = b.inner(*slot);
                f.render_widget(b, *slot);
                empty_message(f, inner, "— vacío —");
            }
        }
    }

    // Actividad reciente + detalle
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(bottom);
    render_diary_table(f, left, app, st.recent_logs(), "Actividad reciente");
    render_log_detail(f, right, app, st.selected_entry());
}

// ---------------------------------------------------------------------------
// Matches (cuadrícula)
// ---------------------------------------------------------------------------

/// En la vista «Todos», de quién es un dato importado (lo tuyo no se etiqueta).
fn owner_tag(app: &TuiApp, owner_id: Option<i64>) -> Option<String> {
    (app.state.owner_filter == OwnerFilter::All && owner_id.is_some()).then(|| app.state.owner_name(owner_id))
}

/// Tarjeta compacta tipo póster.
fn render_card(f: &mut Frame, area: Rect, card: &MatchCard, selected: bool, owner: Option<String>) {
    let p = &card.profile;
    let block = Block::bordered()
        .border_type(if selected { BorderType::Thick } else { BorderType::Rounded })
        .border_style(Style::new().fg(if selected { GREEN } else { BORDER }))
        .style(Style::new().bg(if selected { HIGHLIGHT } else { PANEL }));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width as usize;

    let mut info = p.platform.to_string();
    if let Some(age) = p.age {
        info.push_str(&format!(" · {age}"));
    }
    let mut status_line = vec![s(p.status.as_str(), status_color(p.status))];
    if p.is_favorite {
        status_line.push(s(" ◆", BLUE));
    }
    let stars = if card.log_count == 0 { s("sin reseñas", DIM) } else { s(card.avg_stars(), GREEN) };

    let mut info_line = vec![s(truncate(&info, w), DIM)];
    if let Some(name) = owner {
        info_line.push(s(format!(" · de {name}"), ORANGE));
    }

    let lines = vec![
        Line::from(bold(truncate(&p.alias, w), BRIGHT)),
        Line::from(info_line),
        Line::from(status_line),
        Line::from(vec![stars, heart(card.liked)]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn filter_bar(app: &TuiApp, show_status: bool) -> Line<'static> {
    let fl = &app.state.filters;
    let label = |text: &str, active: bool| -> Span<'static> {
        if active { bold(text.to_string(), GREEN) } else { s(text.to_string(), DIM) }
    };
    let search = if app.searching {
        format!("/{}▏", fl.search)
    } else if fl.search.is_empty() {
        "/ buscar".to_string()
    } else {
        format!("/{}", fl.search)
    };
    let mut spans = vec![
        s(" ", TEXT),
        if app.searching { bold(search, BRIGHT) } else { label(&search, !fl.search.is_empty()) },
        s("   ", TEXT),
        label(
            &format!("[p] {}", fl.platform.map(|p| p.as_str()).unwrap_or("Todas las apps")),
            fl.platform.is_some(),
        ),
        s("   ", TEXT),
    ];
    if show_status {
        spans.push(label(
            &format!("[c] {}", fl.status.map(|s| s.as_str()).unwrap_or("Todos los estados")),
            fl.status.is_some(),
        ));
        spans.push(s("   ", TEXT));
    }
    spans.extend([
        label(&format!("[o] {}", fl.sort.label()), fl.sort != SortBy::default()),
        s("   ", TEXT),
        label("[L] ♥", fl.only_liked),
        s("  ", TEXT),
        label("[*] Top 4", fl.only_favorites),
    ]);
    if fl.is_active() {
        spans.push(s("   [x] limpiar", DIM));
    }
    Line::from(spans)
}

fn render_matches(f: &mut Frame, area: Rect, app: &TuiApp) {
    let st = &app.state;
    let [bar, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    f.render_widget(Paragraph::new(filter_bar(app, true)), bar);

    let [grid_area, detail] =
        Layout::horizontal([Constraint::Min(CARD_W + 4), Constraint::Percentage(38)]).areas(body);

    let cards = st.visible_cards();
    let block = panel(&format!("Matches ({}/{})", cards.len(), st.cards.len()), false);
    let inner = block.inner(grid_area);
    f.render_widget(block, grid_area);

    let cols = (inner.width / CARD_W).max(1) as usize;
    app.grid_cols.set(cols);
    if cards.is_empty() {
        empty_message(f, inner, "Ningún match con estos filtros (x para limpiar, n para añadir)");
    } else {
        let visible_rows = (inner.height / CARD_H).max(1) as usize;
        let cursor = st.cursor();
        let first_row = (cursor / cols).saturating_sub(visible_rows - 1);
        let card_w = inner.width / cols as u16;
        for (i, card) in cards.iter().enumerate().skip(first_row * cols).take(visible_rows * cols) {
            let row = (i / cols - first_row) as u16;
            let col = (i % cols) as u16;
            let rect = Rect::new(inner.x + col * card_w, inner.y + row * CARD_H, card_w, CARD_H);
            render_card(f, rect, card, i == cursor, owner_tag(app, card.profile.owner_id));
        }
    }

    render_match_detail(f, detail, app, st.selected_card());
}

fn render_match_detail(f: &mut Frame, area: Rect, app: &TuiApp, card: Option<&MatchCard>) {
    let block = panel("Ficha", false);
    let Some(card) = card else {
        let inner = block.inner(area);
        f.render_widget(block, area);
        empty_message(f, inner, "Selecciona un match");
        return;
    };
    let p = &card.profile;

    let mut info = vec![s(p.platform.as_str(), BLUE)];
    if let Some(age) = p.age {
        info.push(s(format!(" · {age} años"), DIM));
    }
    if let Some(date) = p.match_date {
        info.push(s(format!(" · match {date}"), DIM));
    }

    let mut status = vec![bold(p.status.as_str(), status_color(p.status))];
    if p.is_favorite {
        status.push(s("  ◆ Top 4", BLUE));
    }
    if p.owner_id.is_some() {
        status.push(s(format!("  · de {} (sólo lectura)", app.state.owner_name(p.owner_id)), ORANGE));
    }

    // Cabecera (junto a la foto, si la hay) y cuerpo (debajo).
    let mut header = vec![
        Line::from(bold(p.alias.clone(), BRIGHT)),
        Line::from(info),
        Line::from(status),
        Line::from(""),
    ];
    if card.log_count > 0 {
        header.push(Line::from(vec![
            s(card.avg_stars(), GREEN),
            heart(card.liked),
            s(
                format!(
                    "  promedio {:.1}/5 · {} {}",
                    card.avg_rating.unwrap_or(0.0) / 2.0,
                    card.log_count,
                    if card.log_count == 1 { "interacción" } else { "interacciones" }
                ),
                DIM,
            ),
        ]));
    } else {
        header.push(Line::from(s("Sin reseñas todavía (r para registrar)", DIM)));
    }

    let mut lines = Vec::new();
    if let Some(bio) = &p.bio_highlight {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("“{bio}”"),
            Style::new().fg(TEXT).add_modifier(Modifier::ITALIC),
        )));
    }
    if !card.tags.is_empty() {
        lines.push(Line::from(""));
        let mut spans = Vec::new();
        for tag in &card.tags {
            spans.push(tag_span(tag));
            spans.push(s("  ", TEXT));
        }
        lines.push(Line::from(spans));
    }

    let logs = app.state.logs_for(p.id);
    if !logs.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(bold("Historial", BRIGHT)));
        for log in logs {
            lines.push(Line::from(vec![
                s(format!("{}  ", log.date_logged.format("%d/%m/%y")), DIM),
                s(format!("{:<13}", log.interaction_type.as_str()), TEXT),
                s(format!("{:<6}", log.rating.stars()), GREEN),
                heart(log.liked),
            ]));
        }
    }

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Con un modal o la ayuda abiertos no se dibuja: un gráfico Sixel/Kitty quedaría encima.
    let overlay_open = app.state.modal.is_some() || app.show_help;
    let photo = p
        .avatar_path
        .as_deref()
        .filter(|_| app.show_photos && !overlay_open && inner.width >= 40 && inner.height >= PHOTO_H + 3);
    let body_area = match photo.and_then(|path| app.photo(path)) {
        Some(mut protocol) => {
            let [top, rest] = Layout::vertical([Constraint::Length(PHOTO_H), Constraint::Min(0)]).areas(inner);
            let [img_area, header_area] =
                Layout::horizontal([Constraint::Length(PHOTO_W), Constraint::Min(0)]).spacing(2).areas(top);
            // Scale (no Fit): también agranda fotos pequeñas para llenar el hueco.
            let widget = StatefulImage::<StatefulProtocol>::default()
                .resize(Resize::Scale(Some(image::imageops::FilterType::Triangle)));
            f.render_stateful_widget(widget, img_area, &mut *protocol);
            f.render_widget(Paragraph::new(header).wrap(Wrap { trim: false }), header_area);
            rest
        }
        None => {
            if let Some(path) = &p.avatar_path
                && !overlay_open
            {
                let hint = if app.show_photos { "no se pudo mostrar" } else { "P para mostrarla" };
                let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
                header.push(Line::from(s(format!("Foto: {name} ({hint})"), DIM)));
            }
            header.append(&mut lines);
            lines = header;
            inner
        }
    };
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body_area);
}

// ---------------------------------------------------------------------------
// Watchlist
// ---------------------------------------------------------------------------

fn render_watchlist(f: &mut Frame, area: Rect, app: &TuiApp) {
    let st = &app.state;
    let [bar, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    f.render_widget(Paragraph::new(filter_bar(app, false)), bar);
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).areas(body);

    let cards = st.watchlist_cards();
    let block = panel(&format!("Sala de espera ({})", cards.len()), false);
    if cards.is_empty() {
        let inner = block.inner(left);
        f.render_widget(block, left);
        empty_message(f, inner, "Nadie en la sala de espera (n para añadir un match)");
    } else {
        let today = today();
        let rows: Vec<Row> = cards
            .iter()
            .map(|c| {
                let p = &c.profile;
                let waiting = p
                    .match_date
                    .map(|d| match (today - d).num_days() {
                        0 => "hoy".to_string(),
                        1 => "hace 1 día".to_string(),
                        n => format!("hace {n} días"),
                    })
                    .unwrap_or_default();
                Row::new(vec![
                    Cell::from(bold(p.alias.clone(), BRIGHT)),
                    Cell::from(s(p.platform.as_str(), BLUE)),
                    Cell::from(s(p.age.map(|a| a.to_string()).unwrap_or_default(), TEXT)),
                    Cell::from(s(waiting, DIM)),
                    Cell::from(s(if c.log_count > 0 { c.avg_stars() } else { String::new() }, GREEN)),
                ])
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Min(20),
                Constraint::Length(7),
                Constraint::Length(4),
                Constraint::Length(13),
                Constraint::Length(6),
            ],
        )
        .header(
            Row::new(["Alias", "App", "Edad", "Esperando", "★"])
                .style(Style::new().fg(DIM).add_modifier(Modifier::BOLD)),
        )
        .block(block)
        .row_highlight_style(Style::new().bg(HIGHLIGHT))
        .highlight_symbol(Span::styled("▌", Style::new().fg(GREEN)));
        f.render_stateful_widget(table, left, &mut table_state(st.cursor(), cards.len()));
    }

    render_match_detail(f, right, app, st.selected_card());
}

// ---------------------------------------------------------------------------
// Diary
// ---------------------------------------------------------------------------

fn render_diary(f: &mut Frame, area: Rect, app: &TuiApp) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).areas(area);
    render_diary_table(f, left, app, &app.state.diary, &format!("Diary ({})", app.state.diary.len()));
    render_log_detail(f, right, app, app.state.selected_entry());
}

/// Tabla de entradas del Diary (también la usa el Dashboard).
fn render_diary_table(f: &mut Frame, area: Rect, app: &TuiApp, entries: &[DiaryEntry], title: &str) {
    let block = panel(title, false);
    if entries.is_empty() {
        let inner = block.inner(area);
        f.render_widget(block, area);
        empty_message(f, inner, "Aún no hay interacciones (r para registrar la primera)");
        return;
    }

    let mut last_month = String::new();
    let rows: Vec<Row> = entries
        .iter()
        .map(|e| {
            let l = &e.log;
            let month = l.date_logged.format("%m/%Y").to_string();
            let month_cell = if month != last_month {
                last_month = month.clone();
                bold(month, BLUE)
            } else {
                s("", DIM)
            };
            Row::new(vec![
                Cell::from(month_cell),
                Cell::from(bold(l.date_logged.format("%d").to_string(), BRIGHT)),
                Cell::from(Line::from(match owner_tag(app, e.owner_id) {
                    Some(name) => vec![s(e.match_alias.clone(), BRIGHT), s(format!(" · {name}"), ORANGE)],
                    None => vec![s(e.match_alias.clone(), BRIGHT)],
                })),
                Cell::from(s(l.interaction_type.as_str(), TEXT)),
                Cell::from(Line::from(vec![s(l.rating.stars(), GREEN), heart(l.liked)])),
                Cell::from(s(if l.contains_spoilers { "⚠" } else { "" }, ORANGE)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(7),
            Constraint::Length(3),
            Constraint::Min(16),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Length(1),
        ],
    )
    .header(
        Row::new(["Mes", "Día", "Match", "Interacción", "Rating", ""])
            .style(Style::new().fg(DIM).add_modifier(Modifier::BOLD)),
    )
    .block(block)
    .row_highlight_style(Style::new().bg(HIGHLIGHT))
    .highlight_symbol(Span::styled("▌", Style::new().fg(GREEN)));
    f.render_stateful_widget(table, area, &mut table_state(app.state.cursor(), entries.len()));
}

fn render_log_detail(f: &mut Frame, area: Rect, app: &TuiApp, entry: Option<&DiaryEntry>) {
    let block = panel("Reseña", false);
    let Some(e) = entry else {
        let inner = block.inner(area);
        f.render_widget(block, area);
        empty_message(f, inner, "Selecciona una entrada");
        return;
    };
    let l = &e.log;

    let mut lines = vec![
        Line::from(vec![bold(e.match_alias.clone(), BRIGHT), s(format!("  {}", e.platform), BLUE)]),
        Line::from(s(format!("{} · {}", l.interaction_type, l.date_logged.format("%d/%m/%Y")), DIM)),
        Line::from(""),
        Line::from(vec![
            bold(l.rating.stars_padded(), GREEN),
            heart(l.liked),
            s(format!("  {:.1}/5", l.rating.as_stars()), DIM),
        ]),
        Line::from(""),
    ];
    for (label, score) in [
        ("Conversación", l.script_score),
        ("Fotos vs. realidad", l.cinematography_score),
        ("Química", l.direction_score),
    ] {
        lines.push(Line::from(vec![
            s(format!("{label:<20}"), TEXT),
            match score {
                Some(v) => s(format_score_dots(v), GREEN),
                None => s("—", DIM),
            },
        ]));
    }
    lines.push(Line::from(""));
    match &l.review_text {
        Some(_) if l.contains_spoilers && !app.reveal_spoilers => {
            lines.push(Line::from(bold("⚠ Esta reseña contiene spoilers.", ORANGE)));
            lines.push(Line::from(s("Pulsa v para mostrarla.", DIM)));
        }
        Some(text) => lines.push(Line::from(s(text.clone(), BRIGHT))),
        None => lines.push(Line::from(s("Sin texto.", DIM))),
    }

    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

// ---------------------------------------------------------------------------
// Listas
// ---------------------------------------------------------------------------

fn render_lists(f: &mut Frame, area: Rect, app: &TuiApp) {
    let st = &app.state;
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).areas(area);
    let entries_focused = app.list_pane == ListPane::Entries;

    let block = panel(&format!("Listas ({})", st.lists.len()), !entries_focused);
    if st.lists.is_empty() {
        let inner = block.inner(left);
        f.render_widget(block, left);
        empty_message(f, inner, "Sin listas (n para crear una)");
    } else {
        let items: Vec<ListItem> = st
            .lists
            .iter()
            .map(|l| {
                let count = st.entries_of_list(l.id).len();
                ListItem::new(Text::from(vec![
                    Line::from(bold(l.title.clone(), BRIGHT)),
                    Line::from(s(
                        format!(
                            "{count} matches{}",
                            if l.is_ranked { " · ranking" } else { "" }
                        ),
                        DIM,
                    )),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::new().bg(HIGHLIGHT))
            .highlight_symbol(Span::styled("▌", Style::new().fg(GREEN)));
        let mut state = ListState::default().with_selected(Some(st.cursor()));
        f.render_stateful_widget(list, left, &mut state);
    }

    let Some(list) = st.selected_list() else {
        let block = panel("Items", false);
        let inner = block.inner(right);
        f.render_widget(block, right);
        empty_message(f, inner, "Selecciona una lista");
        return;
    };
    let block = panel(&list.title, entries_focused);
    let inner = block.inner(right);
    f.render_widget(block, right);

    let [desc_area, table_area] = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(inner);
    f.render_widget(
        Paragraph::new(s(list.description.clone().unwrap_or_default(), DIM)).wrap(Wrap { trim: true }),
        desc_area,
    );

    let entries = st.entries_of_list(list.id);
    if entries.is_empty() {
        empty_message(f, table_area, "Lista vacía (a para añadir el match seleccionado)");
        return;
    }
    let rows: Vec<Row> = entries
        .iter()
        .map(|e| {
            let card = st.card(e.profile.id);
            let rank = if list.is_ranked { format!("{}.", e.item.rank_order) } else { "·".to_string() };
            Row::new(vec![
                Cell::from(bold(rank, GREEN)),
                Cell::from(bold(e.profile.alias.clone(), BRIGHT)),
                Cell::from(s(card.map(|c| c.avg_stars()).unwrap_or_default(), GREEN)),
                Cell::from(s(e.item.item_note.clone().unwrap_or_default(), TEXT)),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [Constraint::Length(3), Constraint::Length(30), Constraint::Length(6), Constraint::Min(10)],
    )
    .row_highlight_style(Style::new().bg(HIGHLIGHT))
    .highlight_symbol(Span::styled("▌", Style::new().fg(GREEN)));
    let mut state = TableState::default().with_selected(entries_focused.then_some(app.entry_cursor));
    f.render_stateful_widget(table, table_area, &mut state);
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

fn render_stats(f: &mut Frame, area: Rect, app: &TuiApp) {
    let stats = &app.state.stats;
    let [summary, middle, bottom] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(12), Constraint::Length(10)]).areas(area);

    // Resumen
    let line = Line::from(vec![
        bold(stats.total_matches.to_string(), BRIGHT),
        s(" matches    ", DIM),
        bold(stats.total_logs.to_string(), BRIGHT),
        s(" reseñas    ", DIM),
        bold(stats.liked_logs.to_string(), ORANGE),
        s(" con ♥    ", DIM),
        s("promedio ", DIM),
        bold(stats.average_stars(), GREEN),
        s(format!(" {:.1}/5", stats.average_rating.unwrap_or(0.0) / 2.0), DIM),
    ]);
    f.render_widget(Paragraph::new(line).block(panel("Resumen", false)), summary);

    // Distribución de calificaciones
    let [dist_area, funnel_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(middle);
    let labels = ["½", "1", "1½", "2", "2½", "3", "3½", "4", "4½", "5"];
    let bars: Vec<Bar> = stats
        .rating_distribution
        .iter()
        .zip(labels)
        .map(|(count, label)| {
            Bar::default()
                .value(*count as u64)
                .label(Line::from(label))
                .text_value(if *count > 0 { count.to_string() } else { String::new() })
        })
        .collect();
    let block = panel("Distribución de calificaciones (★)", false);
    let inner_w = block.inner(dist_area).width;
    let bar_width = ((inner_w.saturating_sub(9)) / 10).clamp(1, 6);
    let chart = BarChart::default()
        .block(block)
        .data(BarGroup::default().bars(&bars))
        .bar_width(bar_width)
        .bar_gap(1)
        .bar_style(Style::new().fg(GREEN))
        .value_style(Style::new().fg(BG).bg(GREEN).add_modifier(Modifier::BOLD))
        .label_style(Style::new().fg(TEXT));
    f.render_widget(chart, dist_area);

    // Embudo
    let block = panel("Embudo de supervivencia", false);
    let bar_max = block.inner(funnel_area).width.saturating_sub(26) as f64;
    let mut lines: Vec<Line> = stats
        .funnel
        .iter()
        .map(|step| {
            let len = (step.pct_of_total / 100.0 * bar_max).round() as usize;
            Line::from(vec![
                s(format!("{:<13}", step.label), TEXT),
                s("█".repeat(len.max(usize::from(step.count > 0))), BLUE),
                s(format!(" {} ({:.0}%)", step.count, step.pct_of_total), DIM),
            ])
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        s("Chat → Primera Cita  ", TEXT),
        bold(format!("{:.0}%", stats.chat_to_date_pct.unwrap_or(0.0)), GREEN),
    ]));
    lines.push(Line::from(vec![
        s("Chat → Ghosting      ", TEXT),
        bold(format!("{:.0}%", stats.ghosting_pct.unwrap_or(0.0)), RED),
    ]));
    f.render_widget(Paragraph::new(lines).block(block), funnel_area);

    // Estados y flags
    let [status_area, red_area, green_area] = Layout::horizontal([Constraint::Ratio(1, 3); 3]).areas(bottom);
    let max_status = stats.status_counts.iter().map(|(_, c)| *c).max().unwrap_or(0).max(1);
    let lines: Vec<Line> = stats
        .status_counts
        .iter()
        .map(|(status, count)| {
            Line::from(vec![
                s(format!("{:<15}", status.as_str()), status_color(*status)),
                s("■".repeat((*count * 10 / max_status) as usize), status_color(*status)),
                s(format!(" {count}"), DIM),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(panel("Por estado", false)), status_area);

    for (area, title, items, color) in [
        (red_area, "Top Red Flags", &stats.top_red_flags, RED),
        (green_area, "Top Green Flags", &stats.top_green_flags, GREEN),
    ] {
        let lines: Vec<Line> = if items.is_empty() {
            vec![Line::from(s("Sin datos", DIM))]
        } else {
            items
                .iter()
                .map(|tc| {
                    Line::from(vec![
                        s(format!("×{} ", tc.count), color),
                        s(tc.tag.name.clone(), BRIGHT),
                    ])
                })
                .collect()
        };
        f.render_widget(
            Paragraph::new(lines).block(panel(title, false)).wrap(Wrap { trim: true }),
            area,
        );
    }
}

// ---------------------------------------------------------------------------
// Modales
// ---------------------------------------------------------------------------

fn modal_title(app: &TuiApp, modal: &Modal) -> String {
    match modal {
        Modal::MatchForm(d) if d.editing_id.is_some() => "Editar match".into(),
        Modal::MatchForm(_) => "Nuevo match".into(),
        Modal::ReviewForm(d) if d.editing_id.is_some() => "Editar reseña".into(),
        Modal::ReviewForm(_) => "Registrar interacción".into(),
        Modal::ListForm(d) if d.editing_id.is_some() => "Editar lista".into(),
        Modal::ListForm(_) => "Nueva lista".into(),
        Modal::ListItemForm(d) => format!(
            "Añadir a «{}»",
            app.state.lists.iter().find(|l| l.id == d.list_id).map(|l| l.title.as_str()).unwrap_or("lista")
        ),
        Modal::TagForm(_) => "Nuevo flag".into(),
        Modal::ImportForm(_) => "Importar JSON".into(),
        Modal::ConfirmDelete(_) => "Confirmar".into(),
    }
}

fn alias_of(app: &TuiApp, id: Option<i64>) -> String {
    id.and_then(|id| app.state.card(id))
        .map(|c| c.profile.alias.clone())
        .unwrap_or_else(|| "(elige con ←/→)".into())
}

fn choice(text: impl Into<String>, color: Color, focused: bool) -> Vec<Span<'static>> {
    if focused {
        vec![s("◀ ", DIM), bold(text, color), s(" ▶", DIM)]
    } else {
        vec![s(text, color)]
    }
}

fn checkbox(on: bool, on_label: &str, color: Color) -> Vec<Span<'static>> {
    if on {
        vec![bold(format!("[x] {on_label}"), color)]
    } else {
        vec![s("[ ]", DIM)]
    }
}

/// Valor visible de un campo del formulario.
fn field_value(app: &TuiApp, modal: &Modal, field: Field, focused: bool) -> Vec<Span<'static>> {
    let text = |value: &str| -> Vec<Span<'static>> {
        let mut v = vec![s(value.to_string(), BRIGHT)];
        if focused {
            v.push(s("▏", GREEN));
        }
        v
    };
    match (modal, field) {
        (Modal::MatchForm(d), Field::Alias) => text(&d.alias),
        (Modal::MatchForm(d), Field::Age) => text(&d.age),
        (Modal::MatchForm(d), Field::MatchDate) => text(&d.match_date),
        (Modal::MatchForm(d), Field::Bio) => text(&d.bio_highlight),
        (Modal::MatchForm(d), Field::Avatar) => text(&d.avatar_path),
        (Modal::MatchForm(d), Field::Platform) => choice(d.platform.as_str(), BLUE, focused),
        (Modal::MatchForm(d), Field::Status) => choice(d.status.as_str(), status_color(d.status), focused),
        (Modal::MatchForm(d), Field::Favorite) => checkbox(d.is_favorite, "◆ en el Top 4", BLUE),
        (Modal::MatchForm(d), Field::Tags) => {
            if app.state.tags.is_empty() {
                return vec![s("No hay flags (crea uno con t)", DIM)];
            }
            let mut spans = Vec::new();
            for (i, tag) in app.state.tags.iter().enumerate() {
                let on = d.tag_ids.contains(&tag.id);
                let mut style = Style::new().fg(if on { tag_color(tag.category) } else { DIM });
                if on {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if focused && i == app.tag_cursor {
                    style = style.bg(HIGHLIGHT).add_modifier(Modifier::UNDERLINED);
                }
                let mark = if on { "■" } else { "□" };
                spans.push(Span::styled(format!("{mark} {}", tag.name), style));
                spans.push(s("  ", TEXT));
            }
            spans
        }
        (Modal::ReviewForm(d), Field::ReviewMatch) => choice(alias_of(app, d.match_id), BRIGHT, focused),
        (Modal::ReviewForm(d), Field::Interaction) => choice(d.interaction_type.as_str(), BLUE, focused),
        (Modal::ReviewForm(d), Field::LogDate) => text(&d.date_logged),
        (Modal::ReviewForm(d), Field::Rating) => {
            let mut v = choice(format!("{:<5}", d.rating_stars()), GREEN, focused);
            v.push(s(format!("  {:.1}/5", d.rating as f32 / 2.0), DIM));
            v
        }
        (Modal::ReviewForm(d), Field::Score(sub)) => match d.score(sub) {
            Some(v) => choice(format_score_dots(v), GREEN, focused),
            None => choice("—", DIM, focused),
        },
        (Modal::ReviewForm(d), Field::Liked) => checkbox(d.liked, "♥", ORANGE),
        (Modal::ReviewForm(d), Field::Spoilers) => checkbox(d.contains_spoilers, "⚠ contiene spoilers", ORANGE),
        (Modal::ReviewForm(d), Field::ReviewText) => text(&d.review_text),
        (Modal::ListForm(d), Field::ListTitle) => text(&d.title),
        (Modal::ListForm(d), Field::ListDescription) => text(&d.description),
        (Modal::ListForm(d), Field::ListRanked) => checkbox(d.is_ranked, "numerada", GREEN),
        (Modal::ListItemForm(d), Field::ItemMatch) => choice(alias_of(app, d.match_id), BRIGHT, focused),
        (Modal::ListItemForm(d), Field::ItemNote) => text(&d.note),
        (Modal::TagForm(d), Field::TagName) => text(&d.name),
        (Modal::TagForm(d), Field::TagKind) => {
            choice(format!("● {}", d.category), tag_color(d.category), focused)
        }
        (Modal::ImportForm(d), Field::ImportPath) => text(&d.path),
        (Modal::ImportForm(d), Field::ImportPrefix) => text(&d.owner_name),
        _ => vec![],
    }
}

fn render_modal(f: &mut Frame, area: Rect, app: &TuiApp, modal: &Modal) {
    if let Modal::ConfirmDelete(target) = modal {
        let rect = area.centered(Constraint::Length(60), Constraint::Length(7));
        f.render_widget(Clear, rect);
        let block = panel("Confirmar", true)
            .border_style(Style::new().fg(RED))
            .style(Style::new().bg(PANEL));
        let lines = vec![
            Line::from(""),
            Line::from(s(app.state.describe_delete(*target), BRIGHT)),
            Line::from(""),
            Line::from(vec![bold("[s]", RED), s(" Sí, borrar     ", TEXT), bold("[n]", BLUE), s(" Cancelar", TEXT)]),
        ];
        f.render_widget(
            Paragraph::new(lines).block(block).alignment(Alignment::Center).wrap(Wrap { trim: true }),
            rect,
        );
        return;
    }

    let fields = form_fields(modal);
    let focused_field = app.focused_field();
    let width = area.width.saturating_sub(4).min(84);
    let extra = if matches!(modal, Modal::ImportForm(_)) { 6 } else { 0 };
    let rect = area.centered(
        Constraint::Length(width),
        Constraint::Length((fields.len() as u16 + 9 + extra).min(area.height)),
    );
    f.render_widget(Clear, rect);
    let block = panel(&modal_title(app, modal), true).style(Style::new().bg(PANEL));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let [form_area, hint_area] = Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).areas(inner);

    let mut lines = vec![Line::from("")];
    for field in fields {
        let focused = Some(*field) == focused_field;
        let mut spans = vec![
            if focused { bold("▸ ", GREEN) } else { s("  ", TEXT) },
            if focused {
                bold(format!("{:<22}", field.label()), GREEN)
            } else {
                s(format!("{:<22}", field.label()), TEXT)
            },
        ];
        spans.extend(field_value(app, modal, *field, focused));
        lines.push(Line::from(spans));
    }
    if let Modal::ImportForm(d) = modal {
        lines.push(Line::from(""));
        lines.push(match &d.preview {
            Some(Ok(summary)) => Line::from(s(format!("  {summary}"), GREEN)),
            Some(Err(e)) => Line::from(s(format!("  {e}"), RED)),
            None => Line::from(s("  Escribe la ruta del archivo (Tab para comprobarlo)", DIM)),
        });
        lines.push(Line::from(s(
            "  Con un perfil, sus datos y estadísticas quedan aparte de los tuyos (u para cambiar).",
            DIM,
        )));
        lines.push(Line::from(s("  Vacío: restaura en tus datos (respaldo propio).", DIM)));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), form_area);

    let hint = match focused_field.map(Field::kind) {
        Some(FieldKind::Text) => "escribe · Backspace borra · Ctrl+U limpia",
        Some(FieldKind::Choice) => "←/→ o h/l cambian el valor",
        Some(FieldKind::Toggle) => "Espacio alterna",
        Some(FieldKind::Tags) => "←/→ elige flag · Espacio lo marca",
        None => "",
    };
    let mut hint_lines = vec![Line::from(vec![
        s(format!("{hint}   "), DIM),
        bold("Tab/↑↓", BLUE),
        s(" campo  ", DIM),
        bold("Enter", BLUE),
        s(" guardar  ", DIM),
        bold("Esc", BLUE),
        s(" cancelar", DIM),
    ])];
    if let Some(msg) = &app.state.status
        && msg.is_error
    {
        hint_lines.insert(0, Line::from(bold(format!("✖ {}", msg.text), RED)));
    }
    f.render_widget(Paragraph::new(hint_lines).wrap(Wrap { trim: true }), hint_area);
}

fn render_help(f: &mut Frame, area: Rect) {
    let rows: &[(&str, &str)] = &[
        ("Tab / Shift+Tab / 1-6", "Cambiar de pestaña (h/l fuera de la cuadrícula)"),
        ("j k ↑ ↓", "Moverse por la lista"),
        ("h j k l", "Moverse por la cuadrícula de Matches"),
        ("g / G", "Ir al inicio / final"),
        ("n", "Nuevo match (en Listas: nueva lista)"),
        ("r", "Registrar interacción del match seleccionado"),
        ("e", "Editar lo seleccionado"),
        ("d / Supr", "Borrar lo seleccionado"),
        ("f", "Añadir/quitar del Top 4"),
        ("s", "Pasar al siguiente estado"),
        ("t", "Crear un flag (Red/Green/Neutral)"),
        ("v", "Mostrar/ocultar spoilers"),
        ("P", "Mostrar/ocultar fotos en la ficha"),
        ("E", "Exportar a JSON (match/lista seleccionados o todo)"),
        ("I", "Importar un JSON de Tinderboxd"),
        ("u", "Cambiar de perfil: Tú → importados → Todos"),
        ("X", "Borrar el perfil importado que estás viendo"),
        ("/", "Buscar por alias, bio o flag"),
        ("p · c · o", "Filtrar app · filtrar estado · cambiar orden"),
        ("L · * · x", "Sólo ♥ · sólo Top 4 · limpiar filtros"),
        ("Enter (Listas)", "Entrar a los items · J/K reordenan · x quita"),
        ("a (Listas)", "Añadir un match a la lista"),
        ("q / Ctrl+C", "Salir"),
    ];
    let rect = area.centered(Constraint::Length(76), Constraint::Length(rows.len() as u16 + 4));
    f.render_widget(Clear, rect);
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| Line::from(vec![bold(format!("{k:<24}"), BLUE), s(*v, TEXT)]))
        .collect();
    let block = panel("Atajos · cualquier tecla para cerrar", true).style(Style::new().bg(PANEL));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}
