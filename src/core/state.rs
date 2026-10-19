//! Estado de la aplicación compartido por la TUI y la GUI.
//!
//! `AppState` no conoce `egui` ni `ratatui`: guarda una caché de los datos de
//! `Database`, la pestaña activa, la selección, los filtros y los borradores de
//! formularios. Los frontends sólo traducen eventos a llamadas de estos métodos
//! y pintan lo que exponen.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use anyhow::Result;
use chrono::{Local, NaiveDate};

use super::db::{Database, MAX_FAVORITES};
use super::export::{self, ExportOptions, ExportTarget, ImportOptions};
use super::models::*;

// ---------------------------------------------------------------------------
// Pestañas
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tab {
    #[default]
    Dashboard,
    Cartelera,
    Watchlist,
    Diary,
    Lists,
    Stats,
}

impl Tab {
    pub const ALL: [Tab; 6] =
        [Tab::Dashboard, Tab::Cartelera, Tab::Watchlist, Tab::Diary, Tab::Lists, Tab::Stats];

    pub fn title(&self) -> &'static str {
        match self {
            Tab::Dashboard => "Perfil",
            Tab::Cartelera => "Matches",
            Tab::Watchlist => "Watchlist",
            Tab::Diary => "Diary",
            Tab::Lists => "Listas",
            Tab::Stats => "Stats",
        }
    }

    pub fn index(&self) -> usize {
        Tab::ALL.iter().position(|t| t == self).unwrap_or(0)
    }

    pub fn next(&self) -> Tab {
        Tab::ALL[(self.index() + 1) % Tab::ALL.len()]
    }

    pub fn prev(&self) -> Tab {
        Tab::ALL[(self.index() + Tab::ALL.len() - 1) % Tab::ALL.len()]
    }
}

// ---------------------------------------------------------------------------
// Filtros de la Cartelera / Watchlist
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    #[default]
    MatchDate,
    Rating,
    LastLogged,
    Alias,
}

impl SortBy {
    pub const ALL: [SortBy; 4] = [SortBy::MatchDate, SortBy::Rating, SortBy::LastLogged, SortBy::Alias];

    pub fn label(&self) -> &'static str {
        match self {
            SortBy::MatchDate => "Fecha de match",
            SortBy::Rating => "Rating promedio",
            SortBy::LastLogged => "Última interacción",
            SortBy::Alias => "Alias (A-Z)",
        }
    }

    pub fn next(&self) -> SortBy {
        let i = SortBy::ALL.iter().position(|s| s == self).unwrap_or(0);
        SortBy::ALL[(i + 1) % SortBy::ALL.len()]
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filters {
    /// Búsqueda libre sobre alias, bio y tags (sin distinguir mayúsculas).
    pub search: String,
    pub platform: Option<Platform>,
    pub status: Option<MatchStatus>,
    pub tag_id: Option<i64>,
    pub only_liked: bool,
    pub only_favorites: bool,
    pub sort: SortBy,
}

impl Filters {
    pub fn is_active(&self) -> bool {
        !self.search.trim().is_empty()
            || self.platform.is_some()
            || self.status.is_some()
            || self.tag_id.is_some()
            || self.only_liked
            || self.only_favorites
    }

    /// Limpia los filtros conservando el orden elegido.
    pub fn clear(&mut self) {
        *self = Filters { sort: self.sort, ..Filters::default() };
    }

    /// Cicla el filtro de plataforma: Todas → Tinder → ... → IRL → Todas.
    pub fn cycle_platform(&mut self) {
        self.platform = cycle_option(self.platform, Platform::ALL);
    }

    /// Cicla el filtro de estado: Todos → Watchlist → ... → Solo Amigos → Todos.
    pub fn cycle_status(&mut self) {
        self.status = cycle_option(self.status, MatchStatus::ALL);
    }

    pub fn matches(&self, card: &MatchCard) -> bool {
        let p = &card.profile;
        let query = self.search.trim().to_lowercase();
        let text_ok = query.is_empty()
            || p.alias.to_lowercase().contains(&query)
            || p.bio_highlight.as_deref().unwrap_or("").to_lowercase().contains(&query)
            || card.tags.iter().any(|t| t.name.to_lowercase().contains(&query));

        text_ok
            && self.platform.is_none_or(|pl| p.platform == pl)
            && self.status.is_none_or(|st| p.status == st)
            && self.tag_id.is_none_or(|id| card.tags.iter().any(|t| t.id == id))
            && (!self.only_liked || card.liked)
            && (!self.only_favorites || p.is_favorite)
    }

    /// Filtra y ordena las tarjetas.
    pub fn apply<'a>(&self, cards: &'a [MatchCard]) -> Vec<&'a MatchCard> {
        let mut out: Vec<&MatchCard> = cards.iter().filter(|c| self.matches(c)).collect();
        match self.sort {
            // `cards` ya viene ordenado por fecha de match desde la BD.
            SortBy::MatchDate => {}
            SortBy::Rating => out.sort_by(|a, b| {
                b.avg_rating.unwrap_or(0.0).total_cmp(&a.avg_rating.unwrap_or(0.0))
            }),
            SortBy::LastLogged => out.sort_by_key(|c| std::cmp::Reverse(c.last_logged)),
            SortBy::Alias => {
                out.sort_by_key(|c| c.profile.alias.to_lowercase());
            }
        }
        out
    }
}

fn cycle_option<T: Copy + PartialEq>(current: Option<T>, all: &[T]) -> Option<T> {
    match current {
        None => all.first().copied(),
        Some(v) => {
            let i = all.iter().position(|x| *x == v).unwrap_or(0);
            all.get(i + 1).copied()
        }
    }
}

// ---------------------------------------------------------------------------
// Borradores de formularios
// ---------------------------------------------------------------------------

fn parse_optional_date(field: &str, raw: &str) -> Result<Option<NaiveDate>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map(Some)
        .map_err(|_| format!("{field}: usa el formato AAAA-MM-DD (recibido '{raw}')"))
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Borrador para crear o editar un match.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchDraft {
    pub editing_id: Option<i64>,
    pub alias: String,
    pub age: String,
    pub platform: Platform,
    pub bio_highlight: String,
    pub avatar_path: String,
    pub match_date: String,
    pub status: MatchStatus,
    pub is_favorite: bool,
    pub tag_ids: BTreeSet<i64>,
}

impl Default for MatchDraft {
    fn default() -> Self {
        MatchDraft {
            editing_id: None,
            alias: String::new(),
            age: String::new(),
            platform: Platform::Tinder,
            bio_highlight: String::new(),
            avatar_path: String::new(),
            match_date: today().format("%Y-%m-%d").to_string(),
            status: MatchStatus::Watchlist,
            is_favorite: false,
            tag_ids: BTreeSet::new(),
        }
    }
}

impl MatchDraft {
    pub fn from_card(card: &MatchCard) -> Self {
        let p = &card.profile;
        MatchDraft {
            editing_id: Some(p.id),
            alias: p.alias.clone(),
            age: p.age.map(|a| a.to_string()).unwrap_or_default(),
            platform: p.platform,
            bio_highlight: p.bio_highlight.clone().unwrap_or_default(),
            avatar_path: p.avatar_path.clone().unwrap_or_default(),
            match_date: p.match_date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default(),
            status: p.status,
            is_favorite: p.is_favorite,
            tag_ids: card.tags.iter().map(|t| t.id).collect(),
        }
    }

    pub fn toggle_tag(&mut self, tag_id: i64) {
        if !self.tag_ids.remove(&tag_id) {
            self.tag_ids.insert(tag_id);
        }
    }

    pub fn to_profile(&self) -> Result<MatchProfile, String> {
        let alias = non_empty(&self.alias).ok_or("El alias es obligatorio")?;
        let age = match self.age.trim() {
            "" => None,
            raw => {
                let age: u32 = raw.parse().map_err(|_| format!("Edad inválida: '{raw}'"))?;
                if !(18..=120).contains(&age) {
                    return Err(format!("La edad debe estar entre 18 y 120 (recibido {age})"));
                }
                Some(age)
            }
        };
        Ok(MatchProfile {
            id: self.editing_id.unwrap_or(0),
            alias,
            age,
            platform: self.platform,
            bio_highlight: non_empty(&self.bio_highlight),
            avatar_path: non_empty(&self.avatar_path),
            match_date: parse_optional_date("Fecha de match", &self.match_date)?,
            status: self.status,
            is_favorite: self.is_favorite,
            // Sólo se crean/editan matches propios; `update_match` nunca cambia el dueño.
            owner_id: None,
        })
    }
}

/// Sub-métricas de una reseña.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubScore {
    Script,
    Cinematography,
    Direction,
}

impl SubScore {
    pub const ALL: [SubScore; 3] = [SubScore::Script, SubScore::Cinematography, SubScore::Direction];

    pub fn label(&self) -> &'static str {
        match self {
            SubScore::Script => "Conversación",
            SubScore::Cinematography => "Fotos vs. realidad",
            SubScore::Direction => "Química e iniciativa",
        }
    }
}

/// Borrador para crear o editar una reseña (log).
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewDraft {
    pub editing_id: Option<i64>,
    pub match_id: Option<i64>,
    pub interaction_type: InteractionType,
    pub date_logged: String,
    /// 1..=10 medias estrellas.
    pub rating: u8,
    pub script_score: Option<u8>,
    pub cinematography_score: Option<u8>,
    pub direction_score: Option<u8>,
    pub liked: bool,
    pub review_text: String,
    pub contains_spoilers: bool,
}

impl Default for ReviewDraft {
    fn default() -> Self {
        ReviewDraft {
            editing_id: None,
            match_id: None,
            interaction_type: InteractionType::Chat,
            date_logged: today().format("%Y-%m-%d").to_string(),
            rating: 6,
            script_score: None,
            cinematography_score: None,
            direction_score: None,
            liked: false,
            review_text: String::new(),
            contains_spoilers: false,
        }
    }
}

impl ReviewDraft {
    pub fn for_match(match_id: i64) -> Self {
        ReviewDraft { match_id: Some(match_id), ..ReviewDraft::default() }
    }

    pub fn from_log(log: &InteractionLog) -> Self {
        ReviewDraft {
            editing_id: Some(log.id),
            match_id: Some(log.match_id),
            interaction_type: log.interaction_type,
            date_logged: log.date_logged.format("%Y-%m-%d").to_string(),
            rating: log.rating.value(),
            script_score: log.script_score,
            cinematography_score: log.cinematography_score,
            direction_score: log.direction_score,
            liked: log.liked,
            review_text: log.review_text.clone().unwrap_or_default(),
            contains_spoilers: log.contains_spoilers,
        }
    }

    /// Sube o baja el rating en medias estrellas, dentro de 1..=10.
    pub fn adjust_rating(&mut self, delta: i32) {
        self.rating = Rating::clamped(self.rating as i32 + delta).value();
    }

    pub fn rating_stars(&self) -> String {
        format_stars(self.rating)
    }

    pub fn score(&self, which: SubScore) -> Option<u8> {
        match which {
            SubScore::Script => self.script_score,
            SubScore::Cinematography => self.cinematography_score,
            SubScore::Direction => self.direction_score,
        }
    }

    pub fn set_score(&mut self, which: SubScore, value: Option<u8>) {
        let value = value.map(|v| v.clamp(1, 5));
        match which {
            SubScore::Script => self.script_score = value,
            SubScore::Cinematography => self.cinematography_score = value,
            SubScore::Direction => self.direction_score = value,
        }
    }

    /// Ajusta una sub-métrica: bajar de 1 la deja vacía, subir desde vacía empieza en 1.
    pub fn adjust_score(&mut self, which: SubScore, delta: i32) {
        let next = match self.score(which) {
            None if delta > 0 => Some(1),
            None => None,
            Some(v) => {
                let n = v as i32 + delta;
                (n >= 1).then(|| n.min(5) as u8)
            }
        };
        self.set_score(which, next);
    }

    pub fn to_log(&self) -> Result<InteractionLog, String> {
        let match_id = self.match_id.ok_or("Elige el match al que pertenece la reseña")?;
        let date_logged = parse_optional_date("Fecha", &self.date_logged)?
            .ok_or("La fecha de la interacción es obligatoria")?;
        Ok(InteractionLog {
            id: self.editing_id.unwrap_or(0),
            match_id,
            interaction_type: self.interaction_type,
            date_logged,
            rating: Rating::new(self.rating)?,
            script_score: self.script_score,
            cinematography_score: self.cinematography_score,
            direction_score: self.direction_score,
            liked: self.liked,
            review_text: non_empty(&self.review_text),
            contains_spoilers: self.contains_spoilers,
        })
    }
}

/// Borrador para crear o editar una lista temática.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListDraft {
    pub editing_id: Option<i64>,
    pub title: String,
    pub description: String,
    pub is_ranked: bool,
}

impl ListDraft {
    pub fn from_list(list: &CustomList) -> Self {
        ListDraft {
            editing_id: Some(list.id),
            title: list.title.clone(),
            description: list.description.clone().unwrap_or_default(),
            is_ranked: list.is_ranked,
        }
    }

    pub fn to_list(&self) -> Result<CustomList, String> {
        Ok(CustomList {
            id: self.editing_id.unwrap_or(0),
            title: non_empty(&self.title).ok_or("La lista necesita un título")?,
            description: non_empty(&self.description),
            is_ranked: self.is_ranked,
            owner_id: None,
        })
    }
}

/// Borrador para crear un tag (flag).
#[derive(Debug, Clone, PartialEq)]
pub struct TagDraft {
    pub name: String,
    pub category: TagCategory,
}

impl Default for TagDraft {
    fn default() -> Self {
        TagDraft { name: String::new(), category: TagCategory::RedFlag }
    }
}

/// Borrador para añadir un match a una lista.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListItemDraft {
    pub list_id: i64,
    pub match_id: Option<i64>,
    pub note: String,
}

/// Borrador para importar un archivo JSON de Tinderboxd.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportDraft {
    pub path: String,
    /// Perfil donde se guarda (p. ej. el nombre de quien te lo pasó). Vacío = tus datos.
    pub owner_name: String,
    /// Resumen del archivo, o el error al leerlo (tras `refresh_preview`).
    pub preview: Option<Result<String, String>>,
}

impl ImportDraft {
    pub fn refresh_preview(&mut self) {
        let path = self.path.trim();
        self.preview = (!path.is_empty()).then(|| {
            export::read_file(Path::new(path)).map(|b| b.describe()).map_err(|e| format!("{e:#}"))
        });
    }

    fn options(&self) -> ImportOptions {
        ImportOptions { owner: non_empty(&self.owner_name) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteTarget {
    Match(i64),
    Log(i64),
    List(i64),
    ListItem { list_id: i64, match_id: i64 },
    /// Un perfil importado con todo lo suyo.
    Owner(i64),
}

/// Ventana modal abierta (formularios y confirmaciones).
#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    MatchForm(MatchDraft),
    ReviewForm(ReviewDraft),
    ListForm(ListDraft),
    ListItemForm(ListItemDraft),
    TagForm(TagDraft),
    ImportForm(ImportDraft),
    ConfirmDelete(DeleteTarget),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusMessage {
    pub text: String,
    pub is_error: bool,
}

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub active_tab: Tab,
    pub filters: Filters,
    /// Datos que se ven: los tuyos, los de un perfil importado o todos.
    pub owner_filter: OwnerFilter,
    /// Perfiles importados.
    pub owners: Vec<Owner>,

    // Caché de datos (se recarga con `refresh`).
    pub cards: Vec<MatchCard>,
    pub diary: Vec<DiaryEntry>,
    pub lists: Vec<CustomList>,
    pub list_entries: HashMap<i64, Vec<ListEntry>>,
    pub tags: Vec<Tag>,
    pub stats: StatsSummary,

    // Selección.
    pub selected_match: Option<i64>,
    pub selected_log: Option<i64>,
    pub selected_list: Option<i64>,
    /// Cursor de cada pestaña (índice en su lista visible).
    pub cursors: [usize; Tab::ALL.len()],

    pub modal: Option<Modal>,
    pub status: Option<StatusMessage>,
    pub should_quit: bool,
}

/// Número de logs recientes que muestra el Dashboard.
pub const RECENT_LOGS: usize = 5;

impl AppState {
    pub fn load(db: &Database) -> Result<Self> {
        let mut state = AppState::default();
        state.refresh(db)?;
        Ok(state)
    }

    /// Recarga la caché desde la base de datos conservando la selección cuando sigue existiendo.
    pub fn refresh(&mut self, db: &Database) -> Result<()> {
        self.owners = db.list_owners()?;
        if let OwnerFilter::Owner(id) = self.owner_filter
            && !self.owners.iter().any(|o| o.id == id)
        {
            self.owner_filter = OwnerFilter::Mine;
        }
        let filter = self.owner_filter;
        self.cards = db.match_cards(filter)?;
        self.diary = db.diary(None, filter)?;
        self.lists = db.list_lists(filter)?;
        self.list_entries = self
            .lists
            .iter()
            .map(|l| Ok((l.id, db.list_entries(l.id)?)))
            .collect::<Result<_>>()?;
        self.tags = db.list_tags()?;
        self.stats = db.stats(filter)?;

        if self.selected_match.is_some_and(|id| self.card(id).is_none()) {
            self.selected_match = None;
        }
        if self.selected_log.is_some_and(|id| self.log(id).is_none()) {
            self.selected_log = None;
        }
        if self.selected_list.is_some_and(|id| !self.lists.iter().any(|l| l.id == id)) {
            self.selected_list = None;
        }
        for tab in Tab::ALL {
            self.clamp_cursor(tab);
        }
        self.align_cursor_to_selection();
        Ok(())
    }

    /// Tras recargar, lleva el cursor de la pestaña activa al elemento seleccionado.
    /// Si esa pestaña aún no tiene selección, la toma del cursor.
    fn align_cursor_to_selection(&mut self) {
        let (tracked, pos) = match self.active_tab {
            Tab::Cartelera => (
                self.selected_match,
                self.visible_cards().iter().position(|c| Some(c.profile.id) == self.selected_match),
            ),
            Tab::Watchlist => (
                self.selected_match,
                self.watchlist_cards().iter().position(|c| Some(c.profile.id) == self.selected_match),
            ),
            Tab::Dashboard => (
                self.selected_log,
                self.recent_logs().iter().position(|e| Some(e.log.id) == self.selected_log),
            ),
            Tab::Diary => (
                self.selected_log,
                self.diary.iter().position(|e| Some(e.log.id) == self.selected_log),
            ),
            Tab::Lists => (
                self.selected_list,
                self.lists.iter().position(|l| Some(l.id) == self.selected_list),
            ),
            Tab::Stats => (None, None),
        };
        match (tracked, pos) {
            (_, Some(pos)) => self.cursors[self.active_tab.index()] = pos,
            (None, None) => self.sync_selection(),
            // Selección válida pero fuera de esta pestaña (p. ej. un match recién creado
            // que no aparece en el Dashboard): se respeta.
            (Some(_), None) => {}
        }
    }

    // -------------------------------------------------------------- consultas

    pub fn card(&self, match_id: i64) -> Option<&MatchCard> {
        self.cards.iter().find(|c| c.profile.id == match_id)
    }

    pub fn log(&self, log_id: i64) -> Option<&DiaryEntry> {
        self.diary.iter().find(|e| e.log.id == log_id)
    }

    pub fn selected_card(&self) -> Option<&MatchCard> {
        self.selected_match.and_then(|id| self.card(id))
    }

    pub fn selected_entry(&self) -> Option<&DiaryEntry> {
        self.selected_log.and_then(|id| self.log(id))
    }

    pub fn selected_list(&self) -> Option<&CustomList> {
        self.selected_list.and_then(|id| self.lists.iter().find(|l| l.id == id))
    }

    pub fn entries_of_list(&self, list_id: i64) -> &[ListEntry] {
        self.list_entries.get(&list_id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Logs de un match en orden cronológico (desde la caché del Diary).
    pub fn logs_for(&self, match_id: i64) -> Vec<&InteractionLog> {
        let mut logs: Vec<&InteractionLog> =
            self.diary.iter().filter(|e| e.log.match_id == match_id).map(|e| &e.log).collect();
        logs.reverse();
        logs
    }

    /// El "Top 4" del perfil.
    pub fn favorites(&self) -> Vec<&MatchCard> {
        self.cards.iter().filter(|c| c.profile.is_favorite).take(MAX_FAVORITES).collect()
    }

    pub fn recent_logs(&self) -> &[DiaryEntry] {
        &self.diary[..self.diary.len().min(RECENT_LOGS)]
    }

    /// Cartelera: tarjetas con los filtros aplicados.
    pub fn visible_cards(&self) -> Vec<&MatchCard> {
        self.filters.apply(&self.cards)
    }

    /// Watchlist: matches en sala de espera (aplica búsqueda/plataforma/tags pero no el filtro de estado).
    pub fn watchlist_cards(&self) -> Vec<&MatchCard> {
        let filters = Filters { status: Some(MatchStatus::Watchlist), ..self.filters.clone() };
        filters.apply(&self.cards)
    }

    pub fn cursor(&self) -> usize {
        self.cursors[self.active_tab.index()]
    }

    /// Número de elementos navegables en una pestaña.
    pub fn item_count(&self, tab: Tab) -> usize {
        match tab {
            Tab::Dashboard => self.recent_logs().len(),
            Tab::Cartelera => self.visible_cards().len(),
            Tab::Watchlist => self.watchlist_cards().len(),
            Tab::Diary => self.diary.len(),
            Tab::Lists => self.lists.len(),
            Tab::Stats => 0,
        }
    }

    // ------------------------------------------------------------- navegación

    pub fn set_tab(&mut self, tab: Tab) {
        self.active_tab = tab;
        self.clamp_cursor(tab);
        self.sync_selection();
    }

    /// Mueve el cursor de la pestaña activa (negativo = arriba) y actualiza la selección.
    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.item_count(self.active_tab);
        if len == 0 {
            return;
        }
        let i = self.active_tab.index();
        self.cursors[i] = self.cursors[i].saturating_add_signed(delta).min(len - 1);
        self.sync_selection();
    }

    pub fn cursor_to_start(&mut self) {
        self.cursors[self.active_tab.index()] = 0;
        self.sync_selection();
    }

    pub fn cursor_to_end(&mut self) {
        let len = self.item_count(self.active_tab);
        self.cursors[self.active_tab.index()] = len.saturating_sub(1);
        self.sync_selection();
    }

    /// Coloca el cursor sobre un elemento concreto (p. ej. un clic en la GUI).
    pub fn set_cursor(&mut self, index: usize) {
        self.cursors[self.active_tab.index()] = index;
        self.clamp_cursor(self.active_tab);
        self.sync_selection();
    }

    /// Selecciona un match por id y, si es visible en la pestaña activa, mueve el cursor hacia él.
    pub fn select_match(&mut self, match_id: i64) {
        self.selected_match = Some(match_id);
        let pos = match self.active_tab {
            Tab::Cartelera => self.visible_cards().iter().position(|c| c.profile.id == match_id),
            Tab::Watchlist => self.watchlist_cards().iter().position(|c| c.profile.id == match_id),
            _ => None,
        };
        if let Some(pos) = pos {
            self.cursors[self.active_tab.index()] = pos;
        }
    }

    /// Llamar tras cambiar `filters` para que el cursor siga siendo válido.
    pub fn filters_changed(&mut self) {
        self.clamp_cursor(Tab::Cartelera);
        self.clamp_cursor(Tab::Watchlist);
        self.sync_selection();
    }

    fn clamp_cursor(&mut self, tab: Tab) {
        let len = self.item_count(tab);
        let c = &mut self.cursors[tab.index()];
        *c = (*c).min(len.saturating_sub(1));
    }

    /// Deriva `selected_*` del cursor de la pestaña activa.
    fn sync_selection(&mut self) {
        let c = self.cursor();
        match self.active_tab {
            Tab::Cartelera => {
                if let Some(id) = self.visible_cards().get(c).map(|card| card.profile.id) {
                    self.selected_match = Some(id);
                }
            }
            Tab::Watchlist => {
                if let Some(id) = self.watchlist_cards().get(c).map(|card| card.profile.id) {
                    self.selected_match = Some(id);
                }
            }
            Tab::Dashboard | Tab::Diary => {
                let entry = if self.active_tab == Tab::Dashboard {
                    self.recent_logs().get(c)
                } else {
                    self.diary.get(c)
                };
                if let Some((log_id, match_id)) = entry.map(|e| (e.log.id, e.log.match_id)) {
                    self.selected_log = Some(log_id);
                    self.selected_match = Some(match_id);
                }
            }
            Tab::Lists => {
                if let Some(id) = self.lists.get(c).map(|l| l.id) {
                    self.selected_list = Some(id);
                }
            }
            Tab::Stats => {}
        }
    }

    // ---------------------------------------------------------------- mensajes

    pub fn info(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMessage { text: text.into(), is_error: false });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMessage { text: text.into(), is_error: true });
    }

    pub fn clear_status(&mut self) {
        self.status = None;
    }

    // ----------------------------------------------------------------- perfiles

    /// Nombre de un dueño: "Tú" para tus datos.
    pub fn owner_name(&self, owner_id: Option<i64>) -> String {
        match owner_id {
            None => "Tú".to_string(),
            Some(id) => self.owners.iter().find(|o| o.id == id).map(|o| o.name.clone()).unwrap_or_else(|| "?".into()),
        }
    }

    /// Etiqueta del filtro activo: "Tú", el nombre del perfil o "Todos".
    pub fn filter_label(&self) -> String {
        match self.owner_filter {
            OwnerFilter::Mine => "Tú".to_string(),
            OwnerFilter::Owner(id) => self.owner_name(Some(id)),
            OwnerFilter::All => "Todos".to_string(),
        }
    }

    /// Filtros disponibles en orden: Tú, cada perfil importado, Todos.
    pub fn owner_filters(&self) -> Vec<OwnerFilter> {
        let mut out = vec![OwnerFilter::Mine];
        out.extend(self.owners.iter().map(|o| OwnerFilter::Owner(o.id)));
        if !self.owners.is_empty() {
            out.push(OwnerFilter::All);
        }
        out
    }

    /// Cambia los datos visibles; la selección y los cursores vuelven al inicio.
    pub fn set_owner_filter(&mut self, db: &Database, filter: OwnerFilter) -> Result<()> {
        if filter == self.owner_filter {
            return Ok(());
        }
        self.owner_filter = filter;
        self.selected_match = None;
        self.selected_log = None;
        self.selected_list = None;
        self.cursors = Default::default();
        self.refresh(db)?;
        let label = self.filter_label();
        self.info(match filter {
            OwnerFilter::Owner(_) => format!("Viendo el perfil de {label} (sólo lectura)"),
            _ => format!("Viendo: {label}"),
        });
        Ok(())
    }

    /// Pasa al siguiente filtro (Tú → perfiles → Todos → Tú).
    pub fn cycle_owner_filter(&mut self, db: &Database) -> Result<()> {
        let filters = self.owner_filters();
        if filters.len() < 2 {
            self.info("No hay perfiles importados: importa un JSON de un amigo para compararlo");
            return Ok(());
        }
        let i = filters.iter().position(|f| *f == self.owner_filter).unwrap_or(0);
        self.set_owner_filter(db, filters[(i + 1) % filters.len()])
    }

    /// `true` si se está viendo un perfil importado (todo es de sólo lectura).
    pub fn is_read_only_view(&self) -> bool {
        matches!(self.owner_filter, OwnerFilter::Owner(_))
    }

    /// Matches propios (para los selectores de reseñas e items de lista).
    pub fn own_cards(&self) -> Vec<MatchCard> {
        self.cards.iter().filter(|c| c.profile.owner_id.is_none()).cloned().collect()
    }

    /// Si el dato es de un perfil importado, avisa y devuelve `true` (no se puede tocar).
    fn deny_foreign(&mut self, owner_id: Option<i64>) -> bool {
        if owner_id.is_none() {
            return false;
        }
        let name = self.owner_name(owner_id);
        self.error(format!("Los datos de {name} son de sólo lectura: sirven para comparar"));
        true
    }

    /// En la vista de un perfil importado no se crea nada nuevo.
    fn deny_in_read_only_view(&mut self) -> bool {
        if !self.is_read_only_view() {
            return false;
        }
        let name = self.filter_label();
        self.error(format!("Estás viendo el perfil de {name}: cambia a «Tú» para añadir cosas"));
        true
    }

    fn list_owner(&self, list_id: i64) -> Option<i64> {
        self.lists.iter().find(|l| l.id == list_id).and_then(|l| l.owner_id)
    }

    /// Dueño de lo que se quiere borrar (para no tocar datos importados).
    fn target_owner(&self, target: DeleteTarget) -> Option<i64> {
        match target {
            DeleteTarget::Match(id) => self.card(id).and_then(|c| c.profile.owner_id),
            DeleteTarget::Log(id) => self.log(id).and_then(|e| e.owner_id),
            DeleteTarget::List(id) | DeleteTarget::ListItem { list_id: id, .. } => self.list_owner(id),
            DeleteTarget::Owner(_) => None,
        }
    }

    /// Confirmación para borrar el perfil que se está viendo (con todo lo importado).
    pub fn open_delete_owner(&mut self) {
        match self.owner_filter {
            OwnerFilter::Owner(id) => self.modal = Some(Modal::ConfirmDelete(DeleteTarget::Owner(id))),
            _ => self.error("Elige primero el perfil importado que quieres borrar"),
        }
    }

    // ----------------------------------------------------------------- modales

    pub fn open_new_match(&mut self) {
        if self.deny_in_read_only_view() {
            return;
        }
        self.modal = Some(Modal::MatchForm(MatchDraft::default()));
    }

    pub fn open_edit_selected_match(&mut self) {
        let Some(card) = self.selected_card().cloned() else {
            self.error("Selecciona un match para editarlo");
            return;
        };
        if !self.deny_foreign(card.profile.owner_id) {
            self.modal = Some(Modal::MatchForm(MatchDraft::from_card(&card)));
        }
    }

    /// Nueva reseña para el match seleccionado (o sin match, para elegirlo en el formulario).
    pub fn open_new_review(&mut self) {
        if self.deny_in_read_only_view() {
            return;
        }
        let owner = self.selected_card().and_then(|c| c.profile.owner_id);
        if self.deny_foreign(owner) {
            return;
        }
        let draft = match self.selected_match {
            Some(id) => ReviewDraft::for_match(id),
            None => ReviewDraft::default(),
        };
        self.modal = Some(Modal::ReviewForm(draft));
    }

    pub fn open_edit_selected_log(&mut self) {
        let Some(entry) = self.selected_entry().cloned() else {
            self.error("Selecciona una entrada del Diary para editarla");
            return;
        };
        if !self.deny_foreign(entry.owner_id) {
            self.modal = Some(Modal::ReviewForm(ReviewDraft::from_log(&entry.log)));
        }
    }

    pub fn open_new_list(&mut self) {
        if self.deny_in_read_only_view() {
            return;
        }
        self.modal = Some(Modal::ListForm(ListDraft::default()));
    }

    pub fn open_edit_selected_list(&mut self) {
        let Some(list) = self.selected_list().cloned() else {
            self.error("Selecciona una lista para editarla");
            return;
        };
        if !self.deny_foreign(list.owner_id) {
            self.modal = Some(Modal::ListForm(ListDraft::from_list(&list)));
        }
    }

    pub fn open_add_to_list(&mut self) {
        let Some(list_id) = self.selected_list else {
            self.error("Selecciona una lista primero");
            return;
        };
        if self.deny_foreign(self.list_owner(list_id)) {
            return;
        }
        let match_id = self.selected_card().filter(|c| c.profile.owner_id.is_none()).map(|c| c.profile.id);
        self.modal = Some(Modal::ListItemForm(ListItemDraft { list_id, match_id, note: String::new() }));
    }

    pub fn open_new_tag(&mut self) {
        self.modal = Some(Modal::TagForm(TagDraft::default()));
    }

    /// Pide confirmación para borrar lo seleccionado en la pestaña activa.
    pub fn open_delete_selected(&mut self) {
        let target = match self.active_tab {
            Tab::Dashboard | Tab::Diary => self.selected_log.map(DeleteTarget::Log),
            Tab::Cartelera | Tab::Watchlist => self.selected_match.map(DeleteTarget::Match),
            Tab::Lists => self.selected_list.map(DeleteTarget::List),
            Tab::Stats => None,
        };
        match target {
            Some(t) => self.open_delete(t),
            None => self.error("No hay nada seleccionado para borrar"),
        }
    }

    pub fn open_delete(&mut self, target: DeleteTarget) {
        if !self.deny_foreign(self.target_owner(target)) {
            self.modal = Some(Modal::ConfirmDelete(target));
        }
    }

    pub fn close_modal(&mut self) {
        self.modal = None;
    }

    /// Texto de confirmación para un borrado.
    pub fn describe_delete(&self, target: DeleteTarget) -> String {
        match target {
            DeleteTarget::Match(id) => format!(
                "¿Borrar a «{}» y todas sus reseñas?",
                self.card(id).map(|c| c.profile.alias.as_str()).unwrap_or("?")
            ),
            DeleteTarget::Log(id) => match self.log(id) {
                Some(e) => format!(
                    "¿Borrar la reseña de «{}» ({}, {})?",
                    e.match_alias, e.log.interaction_type, e.log.date_logged
                ),
                None => "¿Borrar la reseña?".to_string(),
            },
            DeleteTarget::List(id) => format!(
                "¿Borrar la lista «{}»?",
                self.lists.iter().find(|l| l.id == id).map(|l| l.title.as_str()).unwrap_or("?")
            ),
            DeleteTarget::ListItem { match_id, .. } => format!(
                "¿Quitar a «{}» de la lista?",
                self.card(match_id).map(|c| c.profile.alias.as_str()).unwrap_or("?")
            ),
            DeleteTarget::Owner(id) => format!(
                "¿Borrar el perfil «{}» con todo lo importado (matches, reseñas, listas y fotos)?",
                self.owner_name(Some(id))
            ),
        }
    }

    /// Guarda el formulario abierto (o ejecuta el borrado confirmado).
    ///
    /// Si la validación falla, el modal sigue abierto y el error queda en `status`.
    /// Devuelve `Err` sólo ante fallos de la base de datos.
    pub fn submit_modal(&mut self, db: &Database) -> Result<()> {
        let Some(modal) = self.modal.take() else { return Ok(()) };
        match self.apply_modal(db, &modal) {
            Ok(Ok(msg)) => {
                self.refresh(db)?;
                self.info(msg);
            }
            Ok(Err(validation)) => {
                self.error(validation);
                self.modal = Some(modal);
            }
            Err(e) => {
                self.error(format!("{e:#}"));
                self.modal = Some(modal);
            }
        }
        Ok(())
    }

    /// `Ok(Ok(mensaje))` = guardado, `Ok(Err(texto))` = validación, `Err` = BD.
    fn apply_modal(&mut self, db: &Database, modal: &Modal) -> Result<Result<String, String>> {
        Ok(Ok(match modal {
            Modal::MatchForm(draft) => {
                let mut profile = match draft.to_profile() {
                    Ok(p) => p,
                    Err(e) => return Ok(Err(e)),
                };
                let previous = draft.editing_id.and_then(|id| self.card(id)).map(|c| c.profile.clone());
                let was_favorite = previous.as_ref().is_some_and(|p| p.is_favorite);
                let own_favorites =
                    self.cards.iter().filter(|c| c.profile.owner_id.is_none() && c.profile.is_favorite).count();
                if profile.is_favorite && !was_favorite && own_favorites >= MAX_FAVORITES {
                    return Ok(Err(format!("El Top {MAX_FAVORITES} está lleno")));
                }
                // Una foto elegida fuera de la carpeta de la app se copia dentro.
                if let Some(path) = profile.avatar_path.clone()
                    && !db.is_managed_photo(&path)
                {
                    match db.import_photo(Path::new(&path)) {
                        Ok(stored) => profile.avatar_path = Some(stored),
                        Err(e) => return Ok(Err(format!("{e:#}"))),
                    }
                }
                let id = match draft.editing_id {
                    Some(_) => {
                        db.update_match(&profile)?;
                        profile.id
                    }
                    None => db.insert_match(&profile)?,
                };
                // Sólo tras guardar con éxito: la foto reemplazada ya no se usa.
                if let Some(old) = previous.and_then(|p| p.avatar_path)
                    && profile.avatar_path.as_deref() != Some(old.as_str())
                {
                    db.remove_photo(&old);
                }
                let current: BTreeSet<i64> =
                    self.card(id).map(|c| c.tags.iter().map(|t| t.id).collect()).unwrap_or_default();
                for tag_id in draft.tag_ids.difference(&current) {
                    db.add_tag_to_match(id, *tag_id)?;
                }
                for tag_id in current.difference(&draft.tag_ids) {
                    db.remove_tag_from_match(id, *tag_id)?;
                }
                self.selected_match = Some(id);
                format!("«{}» guardado en tus matches", profile.alias)
            }
            Modal::ReviewForm(draft) => {
                let log = match draft.to_log() {
                    Ok(l) => l,
                    Err(e) => return Ok(Err(e)),
                };
                if self.card(log.match_id).is_some_and(|c| c.profile.owner_id.is_some()) {
                    return Ok(Err("Sólo puedes reseñar tus propios matches".to_string()));
                }
                let id = match draft.editing_id {
                    Some(_) => {
                        db.update_log(&log)?;
                        log.id
                    }
                    None => db.insert_log(&log)?,
                };
                self.selected_log = Some(id);
                self.selected_match = Some(log.match_id);
                format!("Reseña guardada en el Diary: {} {}", log.rating.stars(), log.heart())
            }
            Modal::ListForm(draft) => {
                let list = match draft.to_list() {
                    Ok(l) => l,
                    Err(e) => return Ok(Err(e)),
                };
                let id = match draft.editing_id {
                    Some(_) => {
                        db.update_list(&list)?;
                        list.id
                    }
                    None => db.create_list(&list)?,
                };
                self.selected_list = Some(id);
                format!("Lista «{}» guardada", list.title)
            }
            Modal::ListItemForm(draft) => {
                let Some(match_id) = draft.match_id else {
                    return Ok(Err("Elige un match para añadir a la lista".to_string()));
                };
                if self.card(match_id).is_some_and(|c| c.profile.owner_id.is_some()) {
                    return Ok(Err("Sólo puedes añadir tus propios matches a tus listas".to_string()));
                }
                let note = non_empty(&draft.note);
                db.add_list_item(draft.list_id, match_id, note.as_deref())?;
                "Match añadido a la lista".to_string()
            }
            Modal::TagForm(draft) => {
                let Some(name) = non_empty(&draft.name) else {
                    return Ok(Err("El tag necesita un nombre".to_string()));
                };
                if self.tags.iter().any(|t| t.name.eq_ignore_ascii_case(&name)) {
                    return Ok(Err(format!("El tag «{name}» ya existe")));
                }
                db.create_tag(&name, draft.category)?;
                format!("Tag {} {name} creado", draft.category.emoji())
            }
            Modal::ImportForm(draft) => {
                let path = draft.path.trim();
                if path.is_empty() {
                    return Ok(Err("Indica el archivo a importar".to_string()));
                }
                // Un archivo inválido es un error de validación: el formulario sigue abierto.
                match export::import_file(db, Path::new(path), &draft.options()) {
                    Ok(report) => {
                        // Mostrar directamente el perfil importado, sin mezclarlo con lo tuyo.
                        if let Some(owner) = &report.owner {
                            self.show_owner_after_change(OwnerFilter::Owner(owner.id));
                        }
                        report.summary()
                    }
                    Err(e) => return Ok(Err(format!("{e:#}"))),
                }
            }
            Modal::ConfirmDelete(target) => {
                let msg = self.describe_delete(*target);
                match *target {
                    DeleteTarget::Match(id) => {
                        let photo = self.card(id).and_then(|c| c.profile.avatar_path.clone());
                        db.delete_match(id)?;
                        if let Some(photo) = photo {
                            db.remove_photo(&photo);
                        }
                    }
                    DeleteTarget::Log(id) => db.delete_log(id)?,
                    DeleteTarget::List(id) => db.delete_list(id)?,
                    DeleteTarget::ListItem { list_id, match_id } => {
                        db.remove_list_item(list_id, match_id)?
                    }
                    DeleteTarget::Owner(id) => {
                        db.delete_owner(id)?;
                        self.show_owner_after_change(OwnerFilter::Mine);
                    }
                }
                format!("Hecho: {}", msg.trim_start_matches('¿').trim_end_matches('?'))
            }
        }))
    }

    /// Cambia el filtro antes del `refresh` que sigue a un modal (importar, borrar perfil).
    fn show_owner_after_change(&mut self, filter: OwnerFilter) {
        self.owner_filter = filter;
        self.selected_match = None;
        self.selected_log = None;
        self.selected_list = None;
        self.cursors = Default::default();
    }

    // ------------------------------------------------------ exportar / importar

    /// Qué exportar según la pestaña: el match o la lista seleccionados; si no, todo
    /// lo del perfil que se está viendo.
    pub fn export_target(&self) -> ExportTarget {
        let all = ExportTarget::All(self.owner_filter);
        match self.active_tab {
            Tab::Cartelera | Tab::Watchlist => self.selected_match.map(ExportTarget::Match).unwrap_or(all),
            Tab::Lists => self.selected_list.map(ExportTarget::List).unwrap_or(all),
            _ => all,
        }
    }

    /// Exporta a `path` y deja un mensaje con lo que se guardó.
    pub fn export_to(&mut self, db: &Database, target: ExportTarget, path: &Path, opts: ExportOptions) -> Result<()> {
        let bundle = export::export_to_file(db, target, opts, path)?;
        self.info(format!(
            "Exportado {} {} a {}",
            bundle.matches.len(),
            if bundle.matches.len() == 1 { "match" } else { "matches" },
            path.display()
        ));
        Ok(())
    }

    /// Exporta lo seleccionado junto a la base de datos con un nombre sugerido.
    pub fn quick_export(&mut self, db: &Database) -> Result<()> {
        let target = self.export_target();
        let path = db.data_dir().join(export::suggested_file_name(db, target)?);
        self.export_to(db, target, &path, ExportOptions::default())
    }

    pub fn open_import(&mut self, path: Option<String>) {
        let mut draft = ImportDraft { path: path.unwrap_or_default(), ..ImportDraft::default() };
        draft.refresh_preview();
        self.modal = Some(Modal::ImportForm(draft));
    }

    // --------------------------------------------------------- acciones rápidas

    /// Alterna el favorito (Top 4) del match seleccionado.
    pub fn toggle_favorite_selected(&mut self, db: &Database) -> Result<()> {
        let Some(card) = self.selected_card() else {
            self.error("Selecciona un match");
            return Ok(());
        };
        let (id, fav, alias, owner) =
            (card.profile.id, !card.profile.is_favorite, card.profile.alias.clone(), card.profile.owner_id);
        if self.deny_foreign(owner) {
            return Ok(());
        }
        match db.set_favorite(id, fav) {
            Ok(()) => {
                self.refresh(db)?;
                self.info(if fav {
                    format!("«{alias}» entra al Top 4")
                } else {
                    format!("«{alias}» sale del Top 4")
                });
            }
            Err(e) => self.error(format!("{e:#}")),
        }
        Ok(())
    }

    /// Avanza el estado del match seleccionado al siguiente (cíclico).
    pub fn cycle_status_selected(&mut self, db: &Database) -> Result<()> {
        let Some(card) = self.selected_card() else {
            self.error("Selecciona un match");
            return Ok(());
        };
        let (id, next, owner) = (card.profile.id, card.profile.status.next(), card.profile.owner_id);
        if self.deny_foreign(owner) {
            return Ok(());
        }
        db.set_status(id, next)?;
        self.refresh(db)?;
        self.info(format!("Nuevo estado: {next}"));
        Ok(())
    }

    /// Mueve el item del match seleccionado dentro de la lista seleccionada.
    pub fn move_list_item(&mut self, db: &Database, match_id: i64, up: bool) -> Result<()> {
        let Some(list_id) = self.selected_list else { return Ok(()) };
        if self.deny_foreign(self.list_owner(list_id)) {
            return Ok(());
        }
        db.move_list_item(list_id, match_id, up)?;
        self.refresh(db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (Database, AppState) {
        let db = Database::open_in_memory().unwrap();
        db.seed_if_empty().unwrap();
        let st = AppState::load(&db).unwrap();
        (db, st)
    }

    #[test]
    fn navigation_updates_selection() {
        let (_db, mut st) = state();
        st.set_tab(Tab::Cartelera);
        let first = st.selected_match;
        assert!(first.is_some());
        st.move_cursor(1);
        assert_ne!(st.selected_match, first);
        st.move_cursor(100);
        assert_eq!(st.cursor(), st.item_count(Tab::Cartelera) - 1);
        st.set_tab(Tab::Watchlist);
        assert_eq!(st.watchlist_cards().len(), 1);
        assert_eq!(st.selected_card().unwrap().profile.status, MatchStatus::Watchlist);
    }

    #[test]
    fn filters_work() {
        let (_db, mut st) = state();
        st.filters.search = "pez".into(); // coincide por tag
        assert_eq!(st.visible_cards().len(), 1);
        st.filters.clear();
        st.filters.only_favorites = true;
        assert_eq!(st.visible_cards().len(), 2);
        st.filters.clear();
        st.filters.sort = SortBy::Rating;
        assert!(st.visible_cards()[0].profile.alias.contains("perro"));
    }

    #[test]
    fn review_form_validation_and_save() {
        let (db, mut st) = state();
        st.set_tab(Tab::Watchlist);
        st.open_new_review();
        if let Some(Modal::ReviewForm(d)) = &mut st.modal {
            d.date_logged = "ayer".into();
        }
        st.submit_modal(&db).unwrap();
        assert!(st.modal.is_some(), "la validación debe mantener el modal abierto");
        assert!(st.status.as_ref().unwrap().is_error);

        if let Some(Modal::ReviewForm(d)) = &mut st.modal {
            d.date_logged = "2026-10-16".into();
            d.interaction_type = InteractionType::PrimeraCita;
            d.adjust_rating(4);
            d.adjust_score(SubScore::Direction, 1);
            d.liked = true;
        }
        st.submit_modal(&db).unwrap();
        assert!(st.modal.is_none());
        assert_eq!(st.diary.len(), 8);
        assert_eq!(st.selected_entry().unwrap().log.rating.value(), 10);
    }

    #[test]
    fn match_form_creates_with_tags_and_delete() {
        let (db, mut st) = state();
        st.open_new_match();
        let tag_id = st.tags[0].id;
        if let Some(Modal::MatchForm(d)) = &mut st.modal {
            d.alias = "El DJ de bodas que se cree Avicii".into();
            d.age = "34".into();
            d.platform = Platform::Irl;
            d.toggle_tag(tag_id);
        }
        st.submit_modal(&db).unwrap();
        let card = st.selected_card().unwrap();
        assert_eq!(card.profile.alias, "El DJ de bodas que se cree Avicii");
        assert_eq!(card.tags.len(), 1);

        let id = card.profile.id;
        st.open_delete(DeleteTarget::Match(id));
        st.submit_modal(&db).unwrap();
        assert!(st.card(id).is_none());
    }

    #[test]
    fn export_selection_and_import_form() {
        let (db, mut st) = state();
        st.set_tab(Tab::Lists);
        assert!(matches!(st.export_target(), ExportTarget::List(_)));
        st.set_tab(Tab::Stats);
        assert_eq!(st.export_target(), ExportTarget::All(OwnerFilter::Mine));

        let path = std::env::temp_dir().join(format!("tinderboxd_state_export_{}.json", std::process::id()));
        st.export_to(&db, ExportTarget::All(OwnerFilter::Mine), &path, ExportOptions::default()).unwrap();
        assert!(st.status.as_ref().unwrap().text.contains("4 matches"));

        let dst = Database::open_in_memory().unwrap();
        let mut other = AppState::load(&dst).unwrap();
        other.open_import(Some(path.display().to_string()));
        if let Some(Modal::ImportForm(d)) = &other.modal {
            assert!(d.preview.as_ref().unwrap().as_ref().unwrap().contains("4 matches"));
        }
        other.submit_modal(&dst).unwrap();
        assert!(other.modal.is_none(), "{:?}", other.status);
        assert_eq!(other.cards.len(), 4);

        // Archivo inexistente: error y el formulario sigue abierto.
        other.open_import(Some("no-existe.json".into()));
        other.submit_modal(&dst).unwrap();
        assert!(other.modal.is_some());
        assert!(other.status.as_ref().unwrap().is_error);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn imported_profiles_are_separate_and_read_only() {
        let (db, mut st) = state();
        let path = std::env::temp_dir().join(format!("tinderboxd_state_owner_{}.json", std::process::id()));
        st.export_to(&db, ExportTarget::All(OwnerFilter::Mine), &path, ExportOptions::default()).unwrap();

        // Importar en el perfil «Ana» lleva directamente a verlo, con sus propias stats.
        st.open_import(Some(path.display().to_string()));
        if let Some(Modal::ImportForm(d)) = &mut st.modal {
            d.owner_name = "Ana".into();
        }
        st.submit_modal(&db).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(st.modal.is_none(), "{:?}", st.status);
        assert!(st.is_read_only_view());
        assert_eq!(st.filter_label(), "Ana");
        assert_eq!(st.cards.len(), 4);
        assert_eq!(st.stats.total_logs, 7);

        // Nada de Ana se puede crear, editar, marcar ni borrar.
        st.set_tab(Tab::Cartelera);
        for action in [AppState::open_new_match, AppState::open_edit_selected_match, AppState::open_new_review] {
            action(&mut st);
            assert!(st.modal.is_none());
            assert!(st.status.as_ref().unwrap().is_error);
        }
        st.open_delete_selected();
        assert!(st.modal.is_none());
        st.toggle_favorite_selected(&db).unwrap();
        assert!(st.status.as_ref().unwrap().text.contains("sólo lectura"));

        // Ciclo de perfiles: Ana → Todos → Tú.
        st.cycle_owner_filter(&db).unwrap();
        assert_eq!(st.owner_filter, OwnerFilter::All);
        assert_eq!(st.cards.len(), 8);
        st.cycle_owner_filter(&db).unwrap();
        assert_eq!(st.owner_filter, OwnerFilter::Mine);
        assert_eq!(st.stats.total_matches, 4, "tus estadísticas no incluyen a Ana");

        // Borrar el perfil quita todo lo suyo y vuelve a «Tú».
        let ana = st.owners[0].id;
        st.set_owner_filter(&db, OwnerFilter::Owner(ana)).unwrap();
        st.open_delete_owner();
        st.submit_modal(&db).unwrap();
        assert!(st.owners.is_empty());
        assert_eq!(st.owner_filter, OwnerFilter::Mine);
        assert_eq!(db.list_matches(OwnerFilter::All).unwrap().len(), 4);
    }

    #[test]
    fn match_form_imports_photo_and_cleans_up() {
        let (db, mut st) = state();
        let src = std::env::temp_dir().join(format!("tinderboxd_state_src_{}.jpg", std::process::id()));
        std::fs::write(&src, b"jpg").unwrap();

        st.open_new_match();
        if let Some(Modal::MatchForm(d)) = &mut st.modal {
            d.alias = "Con foto".into();
            d.avatar_path = src.display().to_string();
        }
        st.submit_modal(&db).unwrap();
        assert!(st.modal.is_none(), "{:?}", st.status);
        let first = st.selected_card().unwrap().profile.avatar_path.clone().unwrap();
        assert!(db.is_managed_photo(&first));

        // Cambiar la foto borra la copia anterior.
        st.open_edit_selected_match();
        if let Some(Modal::MatchForm(d)) = &mut st.modal {
            d.avatar_path = src.display().to_string();
        }
        st.submit_modal(&db).unwrap();
        let second = st.selected_card().unwrap().profile.avatar_path.clone().unwrap();
        assert_ne!(first, second);
        assert!(!Path::new(&first).exists());

        // Borrar el match borra su foto.
        let id = st.selected_card().unwrap().profile.id;
        st.open_delete(DeleteTarget::Match(id));
        st.submit_modal(&db).unwrap();
        assert!(!Path::new(&second).exists());
        assert!(src.exists(), "el archivo original nunca se toca");
        let _ = std::fs::remove_file(&src);
    }
}
