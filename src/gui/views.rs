//! Vistas de la GUI: tema Letterboxd, cuadrícula de pósters, modales y gráficas.

use std::path::Path;

use chrono::Datelike;
use eframe::egui::{
    self, Align, Align2, Button, Color32, ComboBox, CornerRadius, FontId, Frame, Key, Layout, Margin,
    Modifiers, Rect, Response, RichText, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, Ui, Vec2,
    pos2, vec2,
};

use crate::core::db::MAX_FAVORITES;
use crate::core::models::{
    DiaryEntry, InteractionLog, MatchCard, MatchProfile, MatchStatus, Platform, StatsSummary, Tag,
    TagCategory, format_stars,
};
use crate::core::state::{
    AppState, DeleteTarget, ListDraft, ListItemDraft, MatchDraft, Modal, ReviewDraft, SortBy,
    SubScore, Tab, TagDraft, today,
};

use super::app::GuiApp;

// ---------------------------------------------------------------------------
// Paleta Letterboxd
// ---------------------------------------------------------------------------

const BG: Color32 = Color32::from_rgb(0x14, 0x18, 0x1c);
const HEADER: Color32 = Color32::from_rgb(0x10, 0x14, 0x18);
const PANEL: Color32 = Color32::from_rgb(0x1c, 0x22, 0x28);
const INPUT: Color32 = Color32::from_rgb(0x2c, 0x34, 0x40);
const HOVER: Color32 = Color32::from_rgb(0x34, 0x3e, 0x4b);
const BORDER: Color32 = Color32::from_rgb(0x2c, 0x34, 0x40);
const TEXT: Color32 = Color32::from_rgb(0x99, 0xaa, 0xbb);
const BRIGHT: Color32 = Color32::from_rgb(0xdd, 0xe6, 0xee);
const DIM: Color32 = Color32::from_rgb(0x67, 0x78, 0x89);
const GREEN: Color32 = Color32::from_rgb(0x00, 0xe0, 0x54);
const ORANGE: Color32 = Color32::from_rgb(0xff, 0x80, 0x00);
const BLUE: Color32 = Color32::from_rgb(0x40, 0xbc, 0xf4);
const RED: Color32 = Color32::from_rgb(0xff, 0x55, 0x55);

/// Tonos apagados para los pósters sin foto.
const POSTER_TONES: [Color32; 6] = [
    Color32::from_rgb(0x3a, 0x4a, 0x5c),
    Color32::from_rgb(0x5c, 0x3a, 0x4a),
    Color32::from_rgb(0x3a, 0x5c, 0x4a),
    Color32::from_rgb(0x5c, 0x4f, 0x3a),
    Color32::from_rgb(0x4a, 0x3a, 0x5c),
    Color32::from_rgb(0x3a, 0x5a, 0x5c),
];

const POSTER: Vec2 = Vec2::new(140.0, 210.0);
const MONTHS: [&str; 12] =
    ["ENE", "FEB", "MAR", "ABR", "MAY", "JUN", "JUL", "AGO", "SEP", "OCT", "NOV", "DIC"];

/// Fuente de símbolos del sistema (★ ♥ ◆ ⚠…): las fuentes de egui no traen ★ y
/// dibujarían ♥ como emoji a color en vez de usar el naranja de la paleta.
fn install_symbol_font(ctx: &egui::Context) {
    let windows = std::env::var("WINDIR").map(|w| format!(r"{w}\Fonts\seguisym.ttf")).unwrap_or_default();
    let candidates = [
        windows.as_str(),
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    ];
    let Some(bytes) = candidates.iter().filter(|p| !p.is_empty()).find_map(|p| std::fs::read(p).ok()) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("symbols".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            // Tras la fuente principal y antes de las de emoji.
            list.insert(1.min(list.len()), "symbols".into());
        }
    }
    ctx.set_fonts(fonts);
}

pub fn apply_theme(ctx: &egui::Context) {
    install_symbol_font(ctx);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = INPUT;
    v.faint_bg_color = PANEL;
    v.code_bg_color = PANEL;
    v.hyperlink_color = BLUE;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(10);
    v.selection.bg_fill = GREEN.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, GREEN);

    let w = &mut v.widgets;
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.bg_fill = PANEL;
    for (state, fill) in [(&mut w.inactive, INPUT), (&mut w.hovered, HOVER), (&mut w.open, HOVER)] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.fg_stroke = Stroke::new(1.0, BRIGHT);
        state.corner_radius = CornerRadius::same(4);
    }
    w.hovered.bg_stroke = Stroke::new(1.0, GREEN.gamma_multiply(0.6));
    w.active.bg_fill = GREEN.gamma_multiply(0.5);
    w.active.weak_bg_fill = GREEN.gamma_multiply(0.5);
    w.active.fg_stroke = Stroke::new(1.0, BRIGHT);

    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        use egui::TextStyle;
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.button_padding = vec2(10.0, 5.0);
        style.text_styles.insert(TextStyle::Heading, FontId::proportional(22.0));
        style.text_styles.insert(TextStyle::Body, FontId::proportional(14.5));
        style.text_styles.insert(TextStyle::Button, FontId::proportional(14.5));
        style.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
        style.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.5));
    });
}

pub fn header_frame() -> Frame {
    Frame::new().fill(HEADER).inner_margin(Margin::symmetric(16, 10))
}

pub fn status_frame() -> Frame {
    Frame::new().fill(HEADER).inner_margin(Margin::symmetric(16, 6))
}

fn central_frame() -> Frame {
    Frame::new().fill(BG).inner_margin(Margin::same(18))
}

fn side_frame() -> Frame {
    Frame::new().fill(PANEL).inner_margin(Margin::same(16))
}

fn card_frame() -> Frame {
    Frame::new()
        .fill(PANEL)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(14))
        .stroke(Stroke::new(1.0, BORDER))
}

// ---------------------------------------------------------------------------
// Primitivas
// ---------------------------------------------------------------------------

fn status_color(status: MatchStatus) -> Color32 {
    match status {
        MatchStatus::Watchlist => BLUE,
        MatchStatus::Activa => GREEN,
        MatchStatus::SoloAmigos => TEXT,
        MatchStatus::Unmatched => DIM,
        MatchStatus::Ghosteado | MatchStatus::GhostingMutuo => RED,
    }
}

fn tag_color(category: TagCategory) -> Color32 {
    match category {
        TagCategory::RedFlag => RED,
        TagCategory::GreenFlag => GREEN,
        TagCategory::Neutral => TEXT,
    }
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

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn primary_button(text: &str) -> Button<'static> {
    Button::new(RichText::new(text.to_string()).strong().color(BG))
        .fill(GREEN)
        .corner_radius(CornerRadius::same(4))
}

fn secondary_button(text: &str) -> Button<'static> {
    Button::new(RichText::new(text.to_string()).color(BRIGHT))
        .fill(INPUT)
        .corner_radius(CornerRadius::same(4))
}

fn danger_button(text: &str) -> Button<'static> {
    Button::new(RichText::new(text.to_string()).color(RED))
        .fill(INPUT)
        .corner_radius(CornerRadius::same(4))
}

fn section_title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.5).strong().color(DIM));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, BORDER);
    ui.add_space(6.0);
}

fn empty_state(ui: &mut Ui, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.label(RichText::new(text).size(15.0).color(DIM));
    });
}

fn stars_label(ui: &mut Ui, half_stars: Option<f64>, size: f32) {
    if let Some(v) = half_stars {
        ui.label(RichText::new(format_stars(v.round().clamp(1.0, 10.0) as u8)).size(size).color(GREEN));
    }
}

/// Flag como "pastilla". `on = false` la pinta apagada (para selectores).
fn tag_pill(ui: &mut Ui, tag: &Tag, on: bool) -> Response {
    let color = tag_color(tag.category);
    let text = RichText::new(format!("● {}", tag.name)).size(12.5).color(if on { color } else { DIM });
    ui.add(
        Button::new(text)
            .fill(if on { color.gamma_multiply(0.15) } else { INPUT })
            .stroke(Stroke::new(1.0, if on { color.gamma_multiply(0.6) } else { BORDER }))
            .corner_radius(CornerRadius::same(12)),
    )
}

fn initials(alias: &str) -> String {
    const SKIP: &[&str] = &["la", "el", "los", "las", "de", "del", "y", "a"];
    let out: String = alias
        .split_whitespace()
        .filter(|w| !SKIP.contains(&w.to_lowercase().as_str()))
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    if out.is_empty() { "?".into() } else { out }
}

/// UV que recorta la imagen para cubrir `target` sin deformarla.
fn cover_uv(image: Vec2, target: Vec2) -> Rect {
    let (ir, tr) = (image.x / image.y, target.x / target.y);
    if ir > tr {
        let w = tr / ir;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = ir / tr;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    }
}

/// Póster: la foto del match si existe; si no, un póster generado con sus iniciales.
fn paint_poster(ui: &Ui, rect: Rect, p: &MatchProfile) {
    let radius = CornerRadius::same(4);
    let painter = ui.painter();

    let mut painted = false;
    if let Some(path) = p.avatar_path.as_deref().filter(|path| Path::new(path).exists()) {
        let image = egui::Image::new(format!("file://{path}"));
        if let Ok(egui::load::TexturePoll::Ready { texture }) = image.load_for_size(ui.ctx(), rect.size()) {
            image.uv(cover_uv(texture.size, rect.size())).corner_radius(radius).paint_at(ui, rect);
            painted = true;
        }
    }
    if !painted {
        let tone = POSTER_TONES[p.id.rem_euclid(POSTER_TONES.len() as i64) as usize];
        painter.rect_filled(rect, radius, tone);
        let lower = Rect::from_min_max(pos2(rect.left(), rect.center().y), rect.max);
        painter.rect_filled(lower, CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 }, Color32::from_black_alpha(50));
        let size = (rect.width() * 0.32).max(14.0);
        painter.text(
            rect.center() - vec2(0.0, rect.height() * 0.06),
            Align2::CENTER_CENTER,
            initials(&p.alias),
            FontId::proportional(size),
            Color32::from_white_alpha(170),
        );
        if rect.height() > 80.0 {
            painter.text(
                pos2(rect.center().x, rect.bottom() - 12.0),
                Align2::CENTER_BOTTOM,
                p.platform.as_str().to_uppercase(),
                FontId::proportional(11.0),
                Color32::from_white_alpha(120),
            );
        }
    }
    painter.rect_stroke(rect, radius, Stroke::new(1.0, BORDER), StrokeKind::Inside);
}

/// Póster con pie (alias, estrellas, corazón) para la cuadrícula.
fn poster_tile(ui: &mut Ui, card: &MatchCard, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(POSTER.x, POSTER.y + 44.0), Sense::click());
    let poster = Rect::from_min_size(rect.min, POSTER);
    paint_poster(ui, poster, &card.profile);

    let painter = ui.painter();
    if card.profile.is_favorite {
        painter.text(poster.right_top() + vec2(-8.0, 6.0), Align2::RIGHT_TOP, "◆", FontId::proportional(15.0), BLUE);
    }
    let stroke = if selected {
        Some(Stroke::new(3.0, GREEN))
    } else if resp.hovered() {
        Some(Stroke::new(2.0, GREEN.gamma_multiply(0.7)))
    } else {
        None
    };
    if let Some(stroke) = stroke {
        painter.rect_stroke(poster, CornerRadius::same(4), stroke, StrokeKind::Outside);
    }

    let caption = ui.painter_at(rect);
    caption.text(
        pos2(rect.left(), poster.bottom() + 6.0),
        Align2::LEFT_TOP,
        truncate(&card.profile.alias, 19),
        FontId::proportional(13.5),
        BRIGHT,
    );
    let (stars, color) = if card.log_count == 0 {
        ("sin reseñas".to_string(), DIM)
    } else {
        (card.avg_stars(), GREEN)
    };
    caption.text(pos2(rect.left(), poster.bottom() + 25.0), Align2::LEFT_TOP, stars, FontId::proportional(13.0), color);
    if card.liked {
        caption.text(pos2(rect.right(), poster.bottom() + 25.0), Align2::RIGHT_TOP, "♥", FontId::proportional(13.0), ORANGE);
    }
    resp
}

/// Selector de 5 estrellas con medias estrellas (clic en la mitad izquierda = ½).
fn star_picker(ui: &mut Ui, rating: &mut u8) {
    let star = 30.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(star * 5.0, star), Sense::click());
    let hover = resp.hover_pos().map(|p| {
        let x = (p.x - rect.left()).clamp(0.0, rect.width() - 0.01);
        ((x / (star / 2.0)).floor() as u8 + 1).min(10)
    });
    if resp.clicked()
        && let Some(v) = hover
    {
        *rating = v;
    }
    let shown = hover.unwrap_or(*rating);
    let painter = ui.painter_at(rect);
    let font = FontId::proportional(star);
    for i in 0..5u8 {
        let center = pos2(rect.left() + star * (i as f32 + 0.5), rect.center().y);
        painter.text(center, Align2::CENTER_CENTER, "★", font.clone(), HOVER);
        if shown >= i * 2 + 2 {
            painter.text(center, Align2::CENTER_CENTER, "★", font.clone(), GREEN);
        } else if shown == i * 2 + 1 {
            let half = Rect::from_min_max(pos2(center.x - star / 2.0, rect.top()), pos2(center.x, rect.bottom()));
            painter.with_clip_rect(half.intersect(rect)).text(center, Align2::CENTER_CENTER, "★", font.clone(), GREEN);
        }
    }
}

/// Selector de sub-métrica 1..=5; clic en el valor actual lo deja vacío.
fn dots_picker(ui: &mut Ui, value: Option<u8>) -> Option<u8> {
    ui.horizontal(|ui| {
        let mut out = value;
        for i in 1..=5u8 {
            let (rect, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            let on = value.is_some_and(|v| v >= i);
            let painter = ui.painter();
            if on {
                painter.circle_filled(rect.center(), 6.5, GREEN);
            } else {
                painter.circle_stroke(rect.center(), 6.0, Stroke::new(1.5, if resp.hovered() { GREEN } else { DIM }));
            }
            if resp.clicked() {
                out = if value == Some(i) { None } else { Some(i) };
            }
        }
        ui.label(RichText::new(value.map(|v| format!("{v}/5")).unwrap_or_else(|| "—".into())).color(DIM));
        out
    })
    .inner
}

fn enum_combo<T: Copy + PartialEq>(ui: &mut Ui, id: &str, value: &mut T, all: &[T], label: impl Fn(&T) -> String) {
    ComboBox::from_id_salt(id).selected_text(label(value)).width(220.0).show_ui(ui, |ui| {
        for v in all {
            ui.selectable_value(value, *v, label(v));
        }
    });
}

fn match_combo(ui: &mut Ui, id: &str, value: &mut Option<i64>, cards: &[MatchCard]) {
    let selected = value
        .and_then(|id| cards.iter().find(|c| c.profile.id == id))
        .map(|c| c.profile.alias.clone())
        .unwrap_or_else(|| "Elige un match…".into());
    ComboBox::from_id_salt(id).selected_text(selected).width(300.0).show_ui(ui, |ui| {
        for c in cards {
            ui.selectable_value(value, Some(c.profile.id), c.profile.headline());
        }
    });
}

fn form_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(TEXT));
}

// ---------------------------------------------------------------------------
// Cabecera y barra de estado
// ---------------------------------------------------------------------------

pub fn header(ui: &mut Ui, app: &mut GuiApp) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(44.0, 22.0), Sense::hover());
        for (i, color) in [ORANGE, GREEN, BLUE].into_iter().enumerate() {
            ui.painter().circle_filled(pos2(rect.left() + 8.0 + i as f32 * 13.0, rect.center().y), 7.5, color);
        }
        ui.label(RichText::new("TINDERBOXD").size(20.0).strong().color(BRIGHT));
        ui.add_space(24.0);

        for tab in Tab::ALL {
            let active = app.state.active_tab == tab;
            let text = RichText::new(tab.title().to_uppercase())
                .size(13.0)
                .strong()
                .color(if active { BRIGHT } else { TEXT });
            let button = Button::new(text)
                .fill(if active { INPUT } else { Color32::TRANSPARENT })
                .stroke(if active { Stroke::new(1.0, GREEN) } else { Stroke::NONE })
                .corner_radius(CornerRadius::same(4));
            let shortcut = format!("Ctrl+{}", tab.index() + 1);
            if ui.add(button).on_hover_text(shortcut).clicked() {
                app.state.set_tab(tab);
            }
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(primary_button("+ Registrar")).on_hover_text("Ctrl+R").clicked() {
                app.state.open_new_review();
            }
            if ui.add(secondary_button("+ Match")).on_hover_text("Ctrl+N").clicked() {
                app.state.open_new_match();
            }
            if ui.add(secondary_button("+ Flag")).clicked() {
                app.state.open_new_tag();
            }
        });
    });
}

pub fn status_bar(ui: &mut Ui, app: &mut GuiApp) {
    ui.horizontal(|ui| {
        match &app.state.status {
            Some(msg) if app.state.modal.is_none() => {
                let color = if msg.is_error { RED } else { GREEN };
                ui.label(RichText::new(&msg.text).strong().color(color));
            }
            _ => {
                let s = &app.state.stats;
                ui.label(
                    RichText::new(format!(
                        "{} · {} · promedio {}",
                        plural(s.total_matches, "match", "matches"),
                        plural(s.total_logs, "reseña", "reseñas"),
                        s.average_stars()
                    ))
                    .color(DIM),
                );
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new("Ctrl+N nuevo match · Ctrl+R registrar · Esc cierra ventanas").small().color(DIM));
        });
    });
}

// ---------------------------------------------------------------------------
// Perfil
// ---------------------------------------------------------------------------

pub fn dashboard(ui: &mut Ui, app: &mut GuiApp) {
    egui::CentralPanel::default().frame(central_frame()).show(ui, |ui| {
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            section_title(ui, "TOP 4 FAVORITOS");
            let favorites: Vec<MatchCard> = app.state.favorites().into_iter().cloned().collect();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                for i in 0..MAX_FAVORITES {
                    match favorites.get(i) {
                        Some(card) => {
                            let resp = poster_tile(ui, card, false);
                            if resp.clicked() {
                                app.state.set_tab(Tab::Cartelera);
                                app.state.select_match(card.profile.id);
                            }
                            if let Some(bio) = &card.profile.bio_highlight {
                                resp.on_hover_text(format!("“{bio}”"));
                            }
                        }
                        None => {
                            let (rect, resp) = ui.allocate_exact_size(vec2(POSTER.x, POSTER.y + 44.0), Sense::click());
                            let poster = Rect::from_min_size(rect.min, POSTER);
                            ui.painter().rect_stroke(poster, CornerRadius::same(4), Stroke::new(1.0, BORDER), StrokeKind::Inside);
                            ui.painter().text(poster.center(), Align2::CENTER_CENTER, "+", FontId::proportional(30.0), DIM);
                            if resp.on_hover_text("Marca un match con ◆ para añadirlo al Top 4").clicked() {
                                app.state.set_tab(Tab::Cartelera);
                            }
                        }
                    }
                }
            });

            ui.add_space(22.0);
            ui.columns(2, |cols| {
                section_title(&mut cols[0], "ACTIVIDAD RECIENTE");
                let recent: Vec<DiaryEntry> = app.state.recent_logs().to_vec();
                if recent.is_empty() {
                    empty_state(&mut cols[0], "Aún no hay interacciones registradas");
                }
                for e in &recent {
                    let selected = app.state.selected_log == Some(e.log.id);
                    let resp = diary_row(&mut cols[0], e, true, selected);
                    if resp.clicked() {
                        app.state.selected_log = Some(e.log.id);
                        app.state.selected_match = Some(e.log.match_id);
                    }
                    if resp.double_clicked() {
                        app.state.open_edit_selected_log();
                    }
                }

                section_title(&mut cols[1], "RESEÑA");
                match app.state.selected_entry().cloned() {
                    Some(e) => {
                        card_frame().show(&mut cols[1], |ui| review_detail(ui, app, &e));
                    }
                    None => empty_state(&mut cols[1], "Elige una interacción"),
                }
            });
        });
    });
}

// ---------------------------------------------------------------------------
// Matches (cuadrícula de pósters)
// ---------------------------------------------------------------------------

fn filter_bar(ui: &mut Ui, app: &mut GuiApp, show_status: bool) {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        let f = &mut app.state.filters;
        changed |= ui
            .add(TextEdit::singleline(&mut f.search).hint_text("Buscar alias, bio o flag…").desired_width(230.0))
            .changed();

        ComboBox::from_id_salt("f_platform")
            .selected_text(f.platform.map(|p| p.as_str()).unwrap_or("Todas las apps"))
            .show_ui(ui, |ui| {
                changed |= ui.selectable_value(&mut f.platform, None, "Todas las apps").changed();
                for p in Platform::ALL {
                    changed |= ui.selectable_value(&mut f.platform, Some(*p), p.as_str()).changed();
                }
            });

        if show_status {
            ComboBox::from_id_salt("f_status")
                .selected_text(f.status.map(|s| s.as_str()).unwrap_or("Todos los estados"))
                .show_ui(ui, |ui| {
                    changed |= ui.selectable_value(&mut f.status, None, "Todos los estados").changed();
                    for s in MatchStatus::ALL {
                        changed |= ui.selectable_value(&mut f.status, Some(*s), s.as_str()).changed();
                    }
                });
        }

        ComboBox::from_id_salt("f_sort").selected_text(format!("Orden: {}", f.sort.label())).show_ui(ui, |ui| {
            for s in SortBy::ALL {
                changed |= ui.selectable_value(&mut f.sort, s, s.label()).changed();
            }
        });

        changed |= ui.toggle_value(&mut f.only_liked, RichText::new("♥ Me gustaron").color(ORANGE)).changed();
        changed |= ui.toggle_value(&mut f.only_favorites, RichText::new("◆ Top 4").color(BLUE)).changed();
        if f.is_active() && ui.button("Limpiar filtros").clicked() {
            f.clear();
            changed = true;
        }
    });
    if changed {
        app.state.filters_changed();
    }
}

pub fn matches(ui: &mut Ui, app: &mut GuiApp) {
    detail_panel(ui, app);
    egui::CentralPanel::default().frame(central_frame()).show(ui, |ui| {
        filter_bar(ui, app, true);
        ui.add_space(10.0);
        let cards: Vec<MatchCard> = app.state.visible_cards().into_iter().cloned().collect();
        section_title(ui, &format!("MATCHES · {} DE {}", cards.len(), app.state.cards.len()));
        if cards.is_empty() {
            empty_state(ui, "Ningún match con estos filtros");
            return;
        }
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(18.0, 18.0);
                for card in &cards {
                    let id = card.profile.id;
                    let resp = poster_tile(ui, card, app.state.selected_match == Some(id));
                    if resp.clicked() {
                        app.state.select_match(id);
                    }
                    if resp.double_clicked() {
                        app.state.select_match(id);
                        app.state.open_edit_selected_match();
                    }
                    let hover = match &card.profile.bio_highlight {
                        Some(bio) => format!("{}\n“{bio}”", card.profile.headline()),
                        None => card.profile.headline(),
                    };
                    resp.on_hover_text(hover);
                }
            });
        });
    });
}

fn detail_panel(ui: &mut Ui, app: &mut GuiApp) {
    egui::Panel::right("match_detail")
        .resizable(true)
        .default_size(370.0)
        .min_size(300.0)
        .frame(side_frame())
        .show(ui, |ui| match app.state.selected_card().cloned() {
            Some(card) => {
                ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| match_detail(ui, app, &card));
            }
            None => empty_state(ui, "Selecciona un match"),
        });
}

fn match_detail(ui: &mut Ui, app: &mut GuiApp, card: &MatchCard) {
    let p = &card.profile;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(110.0, 165.0), Sense::hover());
        paint_poster(ui, rect, p);
        ui.vertical(|ui| {
            ui.label(RichText::new(&p.alias).size(20.0).strong().color(BRIGHT));
            let mut info = p.platform.to_string();
            if let Some(age) = p.age {
                info.push_str(&format!(" · {age} años"));
            }
            ui.label(RichText::new(info).color(BLUE));
            if let Some(date) = p.match_date {
                ui.label(RichText::new(format!("Match el {}", date.format("%d/%m/%Y"))).color(DIM));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                stars_label(ui, card.avg_rating, 22.0);
                if card.liked {
                    ui.label(RichText::new("♥").size(20.0).color(ORANGE));
                }
            });
            ui.label(RichText::new(plural(card.log_count, "interacción", "interacciones")).color(DIM));
        });
    });

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        let mut status = p.status;
        ComboBox::from_id_salt("detail_status")
            .selected_text(RichText::new(status.as_str()).color(status_color(status)))
            .show_ui(ui, |ui| {
                for s in MatchStatus::ALL {
                    ui.selectable_value(&mut status, *s, RichText::new(s.as_str()).color(status_color(*s)));
                }
            });
        if status != p.status {
            let id = p.id;
            app.act(move |st, db| {
                db.set_status(id, status)?;
                st.refresh(db)?;
                st.info(format!("Nuevo estado: {status}"));
                Ok(())
            });
        }
        let fav = if p.is_favorite { "◆ En el Top 4" } else { "◇ Añadir al Top 4" };
        if ui.add(Button::new(RichText::new(fav).color(BLUE)).fill(INPUT)).clicked() {
            app.act(|st, db| st.toggle_favorite_selected(db));
        }
    });

    if let Some(bio) = &p.bio_highlight {
        ui.add_space(10.0);
        ui.label(RichText::new(format!("“{bio}”")).italics().color(TEXT));
    }

    if !card.tags.is_empty() {
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            for tag in &card.tags {
                tag_pill(ui, tag, true);
            }
        });
    }

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.add(primary_button("Registrar interacción")).clicked() {
            app.state.open_new_review();
        }
        if ui.add(secondary_button("Editar")).clicked() {
            app.state.open_edit_selected_match();
        }
        if ui.add(danger_button("Borrar")).clicked() {
            app.state.open_delete(DeleteTarget::Match(p.id));
        }
    });

    let logs: Vec<InteractionLog> = app.state.logs_for(p.id).into_iter().cloned().collect();
    if !logs.is_empty() {
        ui.add_space(16.0);
        section_title(ui, "HISTORIAL");
        for log in &logs {
            let resp = ui
                .horizontal(|ui| {
                    ui.label(RichText::new(log.date_logged.format("%d/%m/%y").to_string()).color(DIM));
                    ui.label(RichText::new(log.interaction_type.as_str()).color(TEXT));
                    ui.label(RichText::new(log.rating.stars()).color(GREEN));
                    if log.liked {
                        ui.label(RichText::new("♥").color(ORANGE));
                    }
                })
                .response
                .interact(Sense::click())
                .on_hover_text("Clic para editar esta reseña");
            if resp.clicked() {
                app.state.selected_log = Some(log.id);
                app.state.open_edit_selected_log();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Watchlist
// ---------------------------------------------------------------------------

pub fn watchlist(ui: &mut Ui, app: &mut GuiApp) {
    detail_panel(ui, app);
    egui::CentralPanel::default().frame(central_frame()).show(ui, |ui| {
        filter_bar(ui, app, false);
        ui.add_space(10.0);
        let cards: Vec<MatchCard> = app.state.watchlist_cards().into_iter().cloned().collect();
        section_title(ui, &format!("SALA DE ESPERA · {}", cards.len()));
        ui.label(RichText::new("Matches pendientes de primera cita o primer mensaje.").color(DIM));
        ui.add_space(6.0);
        if cards.is_empty() {
            empty_state(ui, "Nadie en la sala de espera");
            return;
        }
        let today = today();
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for card in &cards {
                let p = &card.profile;
                let selected = app.state.selected_match == Some(p.id);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 104.0), Sense::click());
                let painter = ui.painter_at(rect);
                let fill = if selected { HOVER } else if resp.hovered() { PANEL } else { BG };
                painter.rect_filled(rect, CornerRadius::same(6), fill);
                if selected {
                    painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, GREEN), StrokeKind::Inside);
                }
                paint_poster(ui, Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(60.0, 90.0)), p);

                let x = rect.left() + 84.0;
                painter.text(pos2(x, rect.top() + 12.0), Align2::LEFT_TOP, &p.alias, FontId::proportional(17.0), BRIGHT);
                let mut info = p.platform.to_string();
                if let Some(age) = p.age {
                    info.push_str(&format!(" · {age} años"));
                }
                painter.text(pos2(x, rect.top() + 36.0), Align2::LEFT_TOP, info, FontId::proportional(13.5), BLUE);
                if let Some(d) = p.match_date {
                    let days = (today - d).num_days();
                    let waiting = match days {
                        0 => "Match de hoy".to_string(),
                        1 => "Esperando desde hace 1 día".to_string(),
                        n => format!("Esperando desde hace {n} días"),
                    };
                    painter.text(pos2(x, rect.top() + 56.0), Align2::LEFT_TOP, waiting, FontId::proportional(13.0), DIM);
                }
                if let Some(bio) = &p.bio_highlight {
                    painter.text(
                        pos2(x, rect.top() + 76.0),
                        Align2::LEFT_TOP,
                        format!("“{}”", truncate(bio, 90)),
                        FontId::proportional(13.0),
                        TEXT,
                    );
                }
                if card.log_count > 0 {
                    painter.text(
                        pos2(rect.right() - 14.0, rect.top() + 14.0),
                        Align2::RIGHT_TOP,
                        card.avg_stars(),
                        FontId::proportional(16.0),
                        GREEN,
                    );
                }
                if resp.clicked() {
                    app.state.select_match(p.id);
                }
                if resp.double_clicked() {
                    app.state.select_match(p.id);
                    app.state.open_new_review();
                }
                ui.add_space(4.0);
            }
        });
    });
}

// ---------------------------------------------------------------------------
// Diary
// ---------------------------------------------------------------------------

/// Fila del Diary: insignia de mes, día grande, match, interacción, estrellas, ♥.
fn diary_row(ui: &mut Ui, e: &DiaryEntry, show_month: bool, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    let painter = ui.painter_at(rect);
    let l = &e.log;
    if selected {
        painter.rect_filled(rect, CornerRadius::same(4), HOVER);
    } else if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(4), PANEL);
    }
    painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, BORDER));

    let y = rect.center().y;
    if show_month {
        let badge = Rect::from_center_size(pos2(rect.left() + 28.0, y), vec2(44.0, 34.0));
        painter.rect_filled(badge, CornerRadius::same(4), INPUT);
        painter.text(
            badge.center_top() + vec2(0.0, 4.0),
            Align2::CENTER_TOP,
            MONTHS[l.date_logged.month0() as usize],
            FontId::proportional(11.5),
            BRIGHT,
        );
        painter.text(
            badge.center_bottom() - vec2(0.0, 3.0),
            Align2::CENTER_BOTTOM,
            l.date_logged.year().to_string(),
            FontId::proportional(10.0),
            DIM,
        );
    }
    painter.text(pos2(rect.left() + 76.0, y), Align2::CENTER_CENTER, l.date_logged.day().to_string(), FontId::proportional(20.0), BRIGHT);

    let right_cols = 190.0;
    let text_clip = Rect::from_min_max(pos2(rect.left() + 100.0, rect.top()), pos2(rect.right() - right_cols, rect.bottom()));
    let tp = painter.with_clip_rect(text_clip);
    tp.text(pos2(text_clip.left(), y - 9.0), Align2::LEFT_CENTER, &e.match_alias, FontId::proportional(15.0), BRIGHT);
    tp.text(
        pos2(text_clip.left(), y + 10.0),
        Align2::LEFT_CENTER,
        format!("{} · {}", l.interaction_type, e.platform),
        FontId::proportional(12.5),
        DIM,
    );

    let x = rect.right() - right_cols + 10.0;
    painter.text(pos2(x, y), Align2::LEFT_CENTER, l.rating.stars(), FontId::proportional(17.0), GREEN);
    if l.liked {
        painter.text(pos2(x + 100.0, y), Align2::LEFT_CENTER, "♥", FontId::proportional(16.0), ORANGE);
    }
    if l.contains_spoilers {
        painter.text(pos2(x + 125.0, y), Align2::LEFT_CENTER, "⚠", FontId::proportional(15.0), ORANGE);
    }
    if l.review_text.is_some() {
        painter.text(pos2(x + 152.0, y), Align2::LEFT_CENTER, "≡", FontId::proportional(17.0), DIM);
    }
    resp
}

pub fn diary(ui: &mut Ui, app: &mut GuiApp) {
    egui::Panel::right("review_detail")
        .resizable(true)
        .default_size(390.0)
        .min_size(300.0)
        .frame(side_frame())
        .show(ui, |ui| match app.state.selected_entry().cloned() {
            Some(e) => {
                ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| review_detail(ui, app, &e));
            }
            None => empty_state(ui, "Selecciona una entrada del Diary"),
        });

    egui::CentralPanel::default().frame(central_frame()).show(ui, |ui| {
        let entries = app.state.diary.clone();
        section_title(ui, &format!("DIARY · {}", plural(entries.len() as u32, "ENTRADA", "ENTRADAS")));
        if entries.is_empty() {
            empty_state(ui, "Aún no hay interacciones. Usa «+ Registrar».");
            return;
        }
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let mut last_month = None;
            for e in &entries {
                let month = (e.log.date_logged.year(), e.log.date_logged.month());
                let show_month = last_month != Some(month);
                last_month = Some(month);
                let resp = diary_row(ui, e, show_month, app.state.selected_log == Some(e.log.id));
                if resp.clicked() {
                    app.state.selected_log = Some(e.log.id);
                    app.state.selected_match = Some(e.log.match_id);
                }
                if resp.double_clicked() {
                    app.state.open_edit_selected_log();
                }
            }
        });
    });
}

fn review_detail(ui: &mut Ui, app: &mut GuiApp, e: &DiaryEntry) {
    let l = &e.log;
    ui.label(RichText::new(&e.match_alias).size(19.0).strong().color(BRIGHT));
    ui.label(
        RichText::new(format!("{} · {} · {}", l.interaction_type, e.platform, l.date_logged.format("%d/%m/%Y")))
            .color(DIM),
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(l.rating.stars_padded()).size(26.0).color(GREEN));
        if l.liked {
            ui.label(RichText::new("♥").size(24.0).color(ORANGE));
        }
        ui.label(RichText::new(format!("{:.1}/5", l.rating.as_stars())).color(DIM));
    });
    ui.add_space(6.0);
    egui::Grid::new(("subscores", l.id)).num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
        for (which, score) in [
            (SubScore::Script, l.script_score),
            (SubScore::Cinematography, l.cinematography_score),
            (SubScore::Direction, l.direction_score),
        ] {
            ui.label(RichText::new(which.label()).color(TEXT));
            ui.horizontal(|ui| {
                for i in 1..=5u8 {
                    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    if score.is_some_and(|v| v >= i) {
                        ui.painter().circle_filled(rect.center(), 5.0, GREEN);
                    } else {
                        ui.painter().circle_stroke(rect.center(), 4.5, Stroke::new(1.2, DIM));
                    }
                }
            });
            ui.end_row();
        }
    });

    ui.add_space(10.0);
    match &l.review_text {
        Some(_) if l.contains_spoilers && !app.revealed.contains(&l.id) => {
            Frame::new()
                .fill(ORANGE.gamma_multiply(0.12))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.label(RichText::new("⚠ Esta reseña contiene spoilers").strong().color(ORANGE));
                    if ui.button("Mostrar de todas formas").clicked() {
                        app.revealed.insert(l.id);
                    }
                });
        }
        Some(text) => {
            ui.label(RichText::new(text).size(15.0).color(BRIGHT));
        }
        None => {
            ui.label(RichText::new("Sin texto.").color(DIM));
        }
    }

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.add(secondary_button("Editar")).clicked() {
            app.state.selected_log = Some(l.id);
            app.state.open_edit_selected_log();
        }
        if ui.add(secondary_button("Ver match")).clicked() {
            app.state.set_tab(Tab::Cartelera);
            app.state.filters.clear();
            app.state.filters_changed();
            app.state.select_match(l.match_id);
        }
        if ui.add(danger_button("Borrar")).clicked() {
            app.state.open_delete(DeleteTarget::Log(l.id));
        }
    });
}

// ---------------------------------------------------------------------------
// Listas
// ---------------------------------------------------------------------------

pub fn lists(ui: &mut Ui, app: &mut GuiApp) {
    egui::Panel::left("lists_panel")
        .resizable(true)
        .default_size(290.0)
        .min_size(220.0)
        .frame(side_frame())
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("LISTAS").size(12.5).strong().color(DIM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(primary_button("+ Nueva")).clicked() {
                        app.state.open_new_list();
                    }
                });
            });
            ui.add_space(8.0);
            let lists = app.state.lists.clone();
            if lists.is_empty() {
                empty_state(ui, "Sin listas todavía");
            }
            for (i, list) in lists.iter().enumerate() {
                let count = app.state.entries_of_list(list.id).len() as u32;
                let selected = app.state.selected_list == Some(list.id);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
                let painter = ui.painter_at(rect);
                if selected {
                    painter.rect_filled(rect, CornerRadius::same(6), HOVER);
                    painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, GREEN);
                } else if resp.hovered() {
                    painter.rect_filled(rect, CornerRadius::same(6), INPUT);
                }
                painter.text(
                    pos2(rect.left() + 12.0, rect.top() + 10.0),
                    Align2::LEFT_TOP,
                    truncate(&list.title, 32),
                    FontId::proportional(14.5),
                    BRIGHT,
                );
                let meta = format!(
                    "{}{}",
                    plural(count, "match", "matches"),
                    if list.is_ranked { " · ranking" } else { "" }
                );
                painter.text(pos2(rect.left() + 12.0, rect.top() + 32.0), Align2::LEFT_TOP, meta, FontId::proportional(12.0), DIM);
                if resp.clicked() {
                    app.state.set_cursor(i);
                }
            }
        });

    egui::CentralPanel::default().frame(central_frame()).show(ui, |ui| {
        let Some(list) = app.state.selected_list().cloned() else {
            empty_state(ui, "Selecciona o crea una lista");
            return;
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(&list.title).size(24.0).strong().color(BRIGHT));
            if list.is_ranked {
                ui.label(RichText::new(" RANKING ").size(11.0).strong().color(BG).background_color(GREEN));
            }
        });
        if let Some(desc) = &list.description {
            ui.label(RichText::new(desc).color(TEXT));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(primary_button("+ Añadir match")).clicked() {
                app.state.open_add_to_list();
            }
            if ui.add(secondary_button("Editar")).clicked() {
                app.state.open_edit_selected_list();
            }
            if ui.add(danger_button("Borrar lista")).clicked() {
                app.state.open_delete(DeleteTarget::List(list.id));
            }
        });
        ui.add_space(14.0);

        let entries = app.state.entries_of_list(list.id).to_vec();
        if entries.is_empty() {
            empty_state(ui, "Lista vacía: añade tu primer match");
            return;
        }
        let n = entries.len();
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(20.0, 20.0);
                for (i, e) in entries.iter().enumerate() {
                    ui.vertical(|ui| {
                        ui.set_width(130.0);
                        let (rect, resp) = ui.allocate_exact_size(vec2(130.0, 195.0), Sense::click());
                        paint_poster(ui, rect, &e.profile);
                        if list.is_ranked {
                            let badge = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(28.0, 22.0));
                            ui.painter().rect_filled(badge, CornerRadius::same(4), Color32::from_black_alpha(190));
                            ui.painter().text(
                                badge.center(),
                                Align2::CENTER_CENTER,
                                e.item.rank_order.to_string(),
                                FontId::proportional(14.0),
                                GREEN,
                            );
                        }
                        if resp.clicked() {
                            app.state.set_tab(Tab::Cartelera);
                            app.state.select_match(e.profile.id);
                        }
                        resp.on_hover_text("Clic para ver la ficha");
                        ui.add(egui::Label::new(RichText::new(&e.profile.alias).strong().color(BRIGHT)).truncate());
                        if let Some(card) = app.state.card(e.profile.id)
                            && card.log_count > 0
                        {
                            ui.label(RichText::new(card.avg_stars()).color(GREEN));
                        }
                        if let Some(note) = &e.item.item_note {
                            ui.label(RichText::new(note).small().color(TEXT));
                        }
                        ui.horizontal(|ui| {
                            let mid = e.profile.id;
                            if list.is_ranked {
                                if ui.add_enabled(i > 0, Button::new("◀").small()).on_hover_text("Subir").clicked() {
                                    app.act(move |st, db| st.move_list_item(db, mid, true));
                                }
                                if ui.add_enabled(i + 1 < n, Button::new("▶").small()).on_hover_text("Bajar").clicked() {
                                    app.act(move |st, db| st.move_list_item(db, mid, false));
                                }
                            }
                            if ui.add(Button::new(RichText::new("✕").color(RED)).small()).on_hover_text("Quitar de la lista").clicked() {
                                app.state.open_delete(DeleteTarget::ListItem { list_id: list.id, match_id: mid });
                            }
                        });
                    });
                }
            });
        });
    });
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

fn kpi(ui: &mut Ui, value: String, label: &str, color: Color32) {
    card_frame().show(ui, |ui| {
        ui.set_min_width(128.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(value).size(28.0).strong().color(color));
            ui.label(RichText::new(label).size(11.5).strong().color(DIM));
        });
    });
}

/// Fila de barra horizontal: etiqueta, barra proporcional y valor.
fn hbar_row(ui: &mut Ui, label: &str, label_color: Color32, value: u32, max: u32, bar_color: Color32, suffix: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let label_w = 160.0;
    let value_w = 90.0;
    painter.text(pos2(rect.left(), rect.center().y), Align2::LEFT_CENTER, truncate(label, 22), FontId::proportional(13.5), label_color);
    let track = Rect::from_min_max(
        pos2(rect.left() + label_w, rect.center().y - 7.0),
        pos2(rect.right() - value_w, rect.center().y + 7.0),
    );
    painter.rect_filled(track, CornerRadius::same(3), INPUT);
    if value > 0 && max > 0 {
        let w = track.width() * value as f32 / max as f32;
        painter.rect_filled(Rect::from_min_size(track.min, vec2(w.max(3.0), track.height())), CornerRadius::same(3), bar_color);
    }
    painter.text(
        pos2(track.right() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{value}{suffix}"),
        FontId::proportional(13.0),
        DIM,
    );
}

fn rating_chart(ui: &mut Ui, s: &StatsSummary) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 180.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let chart = Rect::from_min_max(rect.min + vec2(24.0, 16.0), rect.max - vec2(78.0, 22.0));
    let gap = 4.0;
    let bar_w = (chart.width() - gap * 9.0) / 10.0;
    let max = s.max_bucket().max(1) as f32;

    for (i, count) in s.rating_distribution.iter().enumerate() {
        let x = chart.left() + i as f32 * (bar_w + gap);
        let h = if *count > 0 { (chart.height() * *count as f32 / max).max(3.0) } else { 2.0 };
        let bar = Rect::from_min_max(pos2(x, chart.bottom() - h), pos2(x + bar_w, chart.bottom()));
        let column = Rect::from_min_max(pos2(x, chart.top()), pos2(x + bar_w, chart.bottom()));
        let resp = ui
            .interact(column, ui.scope_id().with(("rating_bar", i)), Sense::hover())
            .on_hover_text(format!("{} · {}", format_stars(i as u8 + 1), plural(*count, "reseña", "reseñas")));
        let hovered = resp.hovered();
        let color = if *count == 0 {
            INPUT
        } else if hovered {
            GREEN
        } else {
            GREEN.gamma_multiply(0.75)
        };
        painter.rect_filled(bar, CornerRadius::same(2), color);
        if *count > 0 {
            painter.text(bar.center_top() - vec2(0.0, 2.0), Align2::CENTER_BOTTOM, count.to_string(), FontId::proportional(11.5), DIM);
        }
        painter.text(
            pos2(bar.center().x, chart.bottom() + 5.0),
            Align2::CENTER_TOP,
            format_stars(i as u8 + 1),
            FontId::proportional(9.5),
            DIM,
        );
    }
    painter.text(pos2(rect.left() + 4.0, chart.bottom()), Align2::LEFT_BOTTOM, "½", FontId::proportional(16.0), GREEN);
    painter.text(pos2(rect.right(), chart.bottom()), Align2::RIGHT_BOTTOM, "★★★★★", FontId::proportional(14.0), GREEN);
}

fn funnel_chart(ui: &mut Ui, s: &StatsSummary) {
    let max = s.funnel.first().map(|f| f.count).unwrap_or(0);
    for step in &s.funnel {
        hbar_row(ui, &step.label, TEXT, step.count, max, BLUE, &format!("  ({:.0}%)", step.pct_of_total));
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Chat → Primera Cita").color(TEXT));
        ui.label(RichText::new(format!("{:.0}%", s.chat_to_date_pct.unwrap_or(0.0))).strong().size(18.0).color(GREEN));
        ui.add_space(20.0);
        ui.label(RichText::new("Chat → Ghosting").color(TEXT));
        ui.label(RichText::new(format!("{:.0}%", s.ghosting_pct.unwrap_or(0.0))).strong().size(18.0).color(RED));
    });
}

pub fn stats(ui: &mut Ui, app: &mut GuiApp) {
    egui::CentralPanel::default().frame(central_frame()).show(ui, |ui| {
        let s = app.state.stats.clone();
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                kpi(ui, s.total_matches.to_string(), "MATCHES", BRIGHT);
                kpi(ui, s.total_logs.to_string(), "RESEÑAS", BRIGHT);
                kpi(ui, s.liked_logs.to_string(), "CON ♥", ORANGE);
                kpi(
                    ui,
                    if s.average_rating.is_some() { s.average_stars() } else { "—".into() },
                    "PROMEDIO",
                    GREEN,
                );
                kpi(ui, format!("{:.0}%", s.chat_to_date_pct.unwrap_or(0.0)), "CHAT → CITA", GREEN);
                kpi(ui, format!("{:.0}%", s.ghosting_pct.unwrap_or(0.0)), "GHOSTING", RED);
            });
            ui.add_space(16.0);

            ui.columns(2, |cols| {
                card_frame().show(&mut cols[0], |ui| {
                    section_title(ui, "DISTRIBUCIÓN DE CALIFICACIONES");
                    rating_chart(ui, &s);
                });
                card_frame().show(&mut cols[1], |ui| {
                    section_title(ui, "EMBUDO DE SUPERVIVENCIA");
                    funnel_chart(ui, &s);
                });
            });
            ui.add_space(16.0);

            ui.columns(3, |cols| {
                card_frame().show(&mut cols[0], |ui| {
                    section_title(ui, "POR ESTADO");
                    let max = s.status_counts.iter().map(|(_, c)| *c).max().unwrap_or(0);
                    for (status, count) in &s.status_counts {
                        hbar_row(ui, status.as_str(), status_color(*status), *count, max, status_color(*status), "");
                    }
                });
                for (col, title, items, color) in [
                    (1, "TOP RED FLAGS", &s.top_red_flags, RED),
                    (2, "TOP GREEN FLAGS", &s.top_green_flags, GREEN),
                ] {
                    card_frame().show(&mut cols[col], |ui| {
                        section_title(ui, title);
                        if items.is_empty() {
                            ui.label(RichText::new("Sin datos").color(DIM));
                        }
                        let max = items.first().map(|t| t.count).unwrap_or(0);
                        for tc in items {
                            hbar_row(ui, &tc.tag.name, BRIGHT, tc.count, max, color, "");
                        }
                    });
                }
            });
        });
    });
}

// ---------------------------------------------------------------------------
// Modales
// ---------------------------------------------------------------------------

fn modal_title(state: &AppState, modal: &Modal) -> String {
    match modal {
        Modal::MatchForm(d) if d.editing_id.is_some() => "Editar match".into(),
        Modal::MatchForm(_) => "Nuevo match".into(),
        Modal::ReviewForm(d) if d.editing_id.is_some() => "Editar reseña".into(),
        Modal::ReviewForm(_) => "Registrar interacción".into(),
        Modal::ListForm(d) if d.editing_id.is_some() => "Editar lista".into(),
        Modal::ListForm(_) => "Nueva lista".into(),
        Modal::ListItemForm(d) => format!(
            "Añadir a «{}»",
            state.lists.iter().find(|l| l.id == d.list_id).map(|l| l.title.as_str()).unwrap_or("lista")
        ),
        Modal::TagForm(_) => "Nuevo flag".into(),
        Modal::ConfirmDelete(_) => "¿Seguro?".into(),
    }
}

pub fn modal(ctx: &egui::Context, app: &mut GuiApp) {
    let Some(current) = app.state.modal.as_ref() else { return };
    let title = modal_title(&app.state, current);
    let delete_text = match current {
        Modal::ConfirmDelete(target) => Some(app.state.describe_delete(*target)),
        _ => None,
    };

    let mut submit = false;
    let mut cancel = false;
    let frame = Frame::new()
        .fill(PANEL)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(22))
        .stroke(Stroke::new(1.0, BORDER));

    egui::Modal::new(egui::Id::unique("tinderboxd_modal")).frame(frame).show(ctx, |ui| {
        ui.set_width(if delete_text.is_some() { 420.0 } else { 540.0 });
        ui.label(RichText::new(&title).size(20.0).strong().color(BRIGHT));
        ui.add_space(12.0);

        let AppState { modal, cards, tags, status, .. } = &mut app.state;
        match modal {
            Some(Modal::MatchForm(d)) => match_form(ui, d, tags),
            Some(Modal::ReviewForm(d)) => review_form(ui, d, cards),
            Some(Modal::ListForm(d)) => list_form(ui, d),
            Some(Modal::ListItemForm(d)) => item_form(ui, d, cards),
            Some(Modal::TagForm(d)) => tag_form(ui, d),
            Some(Modal::ConfirmDelete(_)) => {
                ui.label(RichText::new(delete_text.as_deref().unwrap_or("")).size(15.0).color(TEXT));
            }
            None => {}
        }

        if let Some(msg) = status
            && msg.is_error
        {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("✖ {}", msg.text)).color(RED));
        }

        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let is_delete = delete_text.is_some();
                let ok = if is_delete {
                    Button::new(RichText::new("Borrar").strong().color(BRIGHT)).fill(RED.gamma_multiply(0.8))
                } else {
                    primary_button("Guardar")
                };
                if ui.add(ok).on_hover_text("Ctrl+Enter").clicked() {
                    submit = true;
                }
                if ui.add(secondary_button("Cancelar")).clicked() {
                    cancel = true;
                }
            });
        });

        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
            submit = true;
        }
    });

    // Sólo Esc o «Cancelar» cierran: un clic fuera no debe tirar una reseña a medio escribir.
    if cancel || ctx.input(|i| i.key_pressed(Key::Escape)) {
        app.state.close_modal();
        app.state.clear_status();
    } else if submit {
        app.act(|st, db| st.submit_modal(db));
    }
}

fn form_grid(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id).num_columns(2).spacing([16.0, 10.0]).show(ui, add);
}

fn match_form(ui: &mut Ui, d: &mut MatchDraft, tags: &[Tag]) {
    form_grid(ui, "match_form", |ui| {
        form_label(ui, "Alias *");
        ui.add(TextEdit::singleline(&mut d.alias).hint_text("La chica del perro gigante").desired_width(340.0));
        ui.end_row();
        form_label(ui, "Edad");
        ui.add(TextEdit::singleline(&mut d.age).hint_text("27").desired_width(70.0));
        ui.end_row();
        form_label(ui, "Plataforma");
        enum_combo(ui, "mf_platform", &mut d.platform, Platform::ALL, |p| p.as_str().to_string());
        ui.end_row();
        form_label(ui, "Estado");
        enum_combo(ui, "mf_status", &mut d.status, MatchStatus::ALL, |s| s.as_str().to_string());
        ui.end_row();
        form_label(ui, "Fecha de match");
        ui.add(TextEdit::singleline(&mut d.match_date).hint_text("AAAA-MM-DD").desired_width(120.0));
        ui.end_row();
        form_label(ui, "Frase de su bio");
        ui.add(TextEdit::singleline(&mut d.bio_highlight).hint_text("Si no te gusta mi perro…").desired_width(340.0));
        ui.end_row();
        form_label(ui, "Foto (ruta local)");
        ui.add(TextEdit::singleline(&mut d.avatar_path).hint_text(r"C:\fotos\match.jpg").desired_width(340.0));
        ui.end_row();
        form_label(ui, "Top 4");
        ui.checkbox(&mut d.is_favorite, RichText::new("◆ Mostrar en el perfil").color(BLUE));
        ui.end_row();
    });
    let path = d.avatar_path.trim();
    if !path.is_empty() && !Path::new(path).exists() {
        ui.label(RichText::new("No encuentro esa imagen; se usará un póster generado.").small().color(ORANGE));
    }

    ui.add_space(10.0);
    form_label(ui, "Flags");
    if tags.is_empty() {
        ui.label(RichText::new("Aún no hay flags. Créalos con «+ Flag».").color(DIM));
    }
    ui.horizontal_wrapped(|ui| {
        for tag in tags {
            if tag_pill(ui, tag, d.tag_ids.contains(&tag.id)).clicked() {
                d.toggle_tag(tag.id);
            }
        }
    });
}

fn review_form(ui: &mut Ui, d: &mut ReviewDraft, cards: &[MatchCard]) {
    form_grid(ui, "review_form", |ui| {
        form_label(ui, "Match *");
        match_combo(ui, "rf_match", &mut d.match_id, cards);
        ui.end_row();
        form_label(ui, "Interacción");
        enum_combo(ui, "rf_type", &mut d.interaction_type, crate::core::models::InteractionType::ALL, |t| {
            t.as_str().to_string()
        });
        ui.end_row();
        form_label(ui, "Fecha");
        ui.add(TextEdit::singleline(&mut d.date_logged).hint_text("AAAA-MM-DD").desired_width(120.0));
        ui.end_row();
        form_label(ui, "Calificación");
        ui.horizontal(|ui| {
            star_picker(ui, &mut d.rating);
            ui.label(RichText::new(format!("{:.1}/5", d.rating as f32 / 2.0)).color(DIM));
        });
        ui.end_row();
        for which in SubScore::ALL {
            form_label(ui, which.label());
            let new = dots_picker(ui, d.score(which));
            d.set_score(which, new);
            ui.end_row();
        }
        form_label(ui, "");
        ui.horizontal(|ui| {
            let heart = if d.liked {
                RichText::new("♥ Me gustó").strong().color(ORANGE)
            } else {
                RichText::new("♡ Me gustó").color(DIM)
            };
            if ui.add(Button::new(heart).fill(INPUT)).clicked() {
                d.liked = !d.liked;
            }
            ui.checkbox(&mut d.contains_spoilers, "⚠ Contiene spoilers");
        });
        ui.end_row();
    });
    ui.add_space(8.0);
    form_label(ui, "Reseña");
    ui.add(
        TextEdit::multiline(&mut d.review_text)
            .hint_text("¿Qué tal estuvo?")
            .desired_rows(5)
            .desired_width(f32::INFINITY),
    );
}

fn list_form(ui: &mut Ui, d: &mut ListDraft) {
    form_grid(ui, "list_form", |ui| {
        form_label(ui, "Título *");
        ui.add(TextEdit::singleline(&mut d.title).hint_text("Citas que debieron quedarse en el chat").desired_width(360.0));
        ui.end_row();
        form_label(ui, "Descripción");
        ui.add(TextEdit::multiline(&mut d.description).desired_rows(3).desired_width(360.0));
        ui.end_row();
        form_label(ui, "Ranking");
        ui.checkbox(&mut d.is_ranked, "Numerar los matches");
        ui.end_row();
    });
}

fn item_form(ui: &mut Ui, d: &mut ListItemDraft, cards: &[MatchCard]) {
    form_grid(ui, "item_form", |ui| {
        form_label(ui, "Match *");
        match_combo(ui, "if_match", &mut d.match_id, cards);
        ui.end_row();
        form_label(ui, "Nota");
        ui.add(TextEdit::singleline(&mut d.note).hint_text("¿Por qué está en esta lista?").desired_width(340.0));
        ui.end_row();
    });
}

fn tag_form(ui: &mut Ui, d: &mut TagDraft) {
    form_grid(ui, "tag_form", |ui| {
        form_label(ui, "Nombre *");
        ui.add(TextEdit::singleline(&mut d.name).hint_text("Foto con pez").desired_width(300.0));
        ui.end_row();
        form_label(ui, "Categoría");
        ComboBox::from_id_salt("tf_category")
            .selected_text(RichText::new(d.category.as_str()).color(tag_color(d.category)))
            .show_ui(ui, |ui| {
                for c in TagCategory::ALL {
                    ui.selectable_value(&mut d.category, *c, RichText::new(c.as_str()).color(tag_color(*c)));
                }
            });
        ui.end_row();
    });
}
