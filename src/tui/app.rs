//! Event loop de la TUI: traduce teclas a llamadas sobre `AppState` / `Database`.

use std::cell::{Cell, RefCell, RefMut};
use std::collections::HashMap;

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

use crate::core::db::Database;
use crate::core::models::{ListEntry, MatchCard};
use crate::core::state::{AppState, DeleteTarget, Modal, SubScore, Tab};

use super::views;

// ---------------------------------------------------------------------------
// Campos de formulario
// ---------------------------------------------------------------------------

/// Campo editable de un formulario modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    // Match
    Alias,
    Age,
    Platform,
    Status,
    MatchDate,
    Bio,
    Avatar,
    Favorite,
    Tags,
    // Reseña
    ReviewMatch,
    Interaction,
    LogDate,
    Rating,
    Score(SubScore),
    Liked,
    Spoilers,
    ReviewText,
    // Lista
    ListTitle,
    ListDescription,
    ListRanked,
    // Item de lista
    ItemMatch,
    ItemNote,
    // Tag
    TagName,
    TagKind,
    // Importar
    ImportPath,
    ImportPrefix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// Texto libre: se escribe directamente.
    Text,
    /// Valor que se cambia con ←/→.
    Choice,
    /// Casilla que se alterna con Espacio.
    Toggle,
    /// Selector múltiple de tags.
    Tags,
}

const MATCH_FIELDS: &[Field] = &[
    Field::Alias,
    Field::Age,
    Field::Platform,
    Field::Status,
    Field::MatchDate,
    Field::Bio,
    Field::Avatar,
    Field::Favorite,
    Field::Tags,
];

const REVIEW_FIELDS: &[Field] = &[
    Field::ReviewMatch,
    Field::Interaction,
    Field::LogDate,
    Field::Rating,
    Field::Score(SubScore::Script),
    Field::Score(SubScore::Cinematography),
    Field::Score(SubScore::Direction),
    Field::Liked,
    Field::Spoilers,
    Field::ReviewText,
];

const LIST_FIELDS: &[Field] = &[Field::ListTitle, Field::ListDescription, Field::ListRanked];
const ITEM_FIELDS: &[Field] = &[Field::ItemMatch, Field::ItemNote];
const TAG_FIELDS: &[Field] = &[Field::TagName, Field::TagKind];
const IMPORT_FIELDS: &[Field] = &[Field::ImportPath, Field::ImportPrefix];

pub fn form_fields(modal: &Modal) -> &'static [Field] {
    match modal {
        Modal::MatchForm(_) => MATCH_FIELDS,
        Modal::ReviewForm(_) => REVIEW_FIELDS,
        Modal::ListForm(_) => LIST_FIELDS,
        Modal::ListItemForm(_) => ITEM_FIELDS,
        Modal::TagForm(_) => TAG_FIELDS,
        Modal::ImportForm(_) => IMPORT_FIELDS,
        Modal::ConfirmDelete(_) => &[],
    }
}

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Field::Alias => "Alias",
            Field::Age => "Edad",
            Field::Platform => "Plataforma",
            Field::Status => "Estado",
            Field::MatchDate => "Fecha de match",
            Field::Bio => "Frase de su bio",
            Field::Avatar => "Ruta de su foto",
            Field::Favorite => "Top 4",
            Field::Tags => "Flags",
            Field::ReviewMatch | Field::ItemMatch => "Match",
            Field::Interaction => "Interacción",
            Field::LogDate => "Fecha",
            Field::Rating => "Calificación",
            Field::Score(s) => s.label(),
            Field::Liked => "Me gustó",
            Field::Spoilers => "Spoilers",
            Field::ReviewText => "Reseña",
            Field::ListTitle => "Título",
            Field::ListDescription => "Descripción",
            Field::ListRanked => "Con ranking",
            Field::ItemNote => "Nota",
            Field::TagName => "Nombre",
            Field::TagKind => "Categoría",
            Field::ImportPath => "Archivo JSON",
            Field::ImportPrefix => "Perfil (de quién es)",
        }
    }

    pub fn kind(self) -> FieldKind {
        match self {
            Field::Alias
            | Field::Age
            | Field::MatchDate
            | Field::Bio
            | Field::Avatar
            | Field::LogDate
            | Field::ReviewText
            | Field::ListTitle
            | Field::ListDescription
            | Field::ItemNote
            | Field::TagName
            | Field::ImportPath
            | Field::ImportPrefix => FieldKind::Text,
            Field::Platform
            | Field::Status
            | Field::ReviewMatch
            | Field::Interaction
            | Field::Rating
            | Field::Score(_)
            | Field::ItemMatch
            | Field::TagKind => FieldKind::Choice,
            Field::Favorite | Field::Liked | Field::Spoilers | Field::ListRanked => FieldKind::Toggle,
            Field::Tags => FieldKind::Tags,
        }
    }
}

/// Texto editable del campo, si es de texto.
fn text_mut(modal: &mut Modal, field: Field) -> Option<&mut String> {
    match (modal, field) {
        (Modal::MatchForm(d), Field::Alias) => Some(&mut d.alias),
        (Modal::MatchForm(d), Field::Age) => Some(&mut d.age),
        (Modal::MatchForm(d), Field::MatchDate) => Some(&mut d.match_date),
        (Modal::MatchForm(d), Field::Bio) => Some(&mut d.bio_highlight),
        (Modal::MatchForm(d), Field::Avatar) => Some(&mut d.avatar_path),
        (Modal::ReviewForm(d), Field::LogDate) => Some(&mut d.date_logged),
        (Modal::ReviewForm(d), Field::ReviewText) => Some(&mut d.review_text),
        (Modal::ListForm(d), Field::ListTitle) => Some(&mut d.title),
        (Modal::ListForm(d), Field::ListDescription) => Some(&mut d.description),
        (Modal::ListItemForm(d), Field::ItemNote) => Some(&mut d.note),
        (Modal::TagForm(d), Field::TagName) => Some(&mut d.name),
        (Modal::ImportForm(d), Field::ImportPath) => Some(&mut d.path),
        (Modal::ImportForm(d), Field::ImportPrefix) => Some(&mut d.owner_name),
        _ => None,
    }
}

/// Cicla un id dentro de la lista de matches (para los selectores de match).
fn cycle_match(current: Option<i64>, cards: &[MatchCard], delta: i32) -> Option<i64> {
    if cards.is_empty() {
        return None;
    }
    let n = cards.len() as i32;
    let next = match current.and_then(|id| cards.iter().position(|c| c.profile.id == id)) {
        Some(i) => (i as i32 + delta).rem_euclid(n),
        None if delta >= 0 => 0,
        None => n - 1,
    };
    Some(cards[next as usize].profile.id)
}

/// Cambia el valor de un campo de selección (o alterna una casilla).
fn adjust(modal: &mut Modal, field: Field, delta: i32, cards: &[MatchCard]) {
    let fwd = delta > 0;
    match (modal, field) {
        (Modal::MatchForm(d), Field::Platform) => {
            d.platform = if fwd { d.platform.next() } else { d.platform.prev() }
        }
        (Modal::MatchForm(d), Field::Status) => {
            d.status = if fwd { d.status.next() } else { d.status.prev() }
        }
        (Modal::MatchForm(d), Field::Favorite) => d.is_favorite = !d.is_favorite,
        (Modal::ReviewForm(d), Field::ReviewMatch) => d.match_id = cycle_match(d.match_id, cards, delta),
        (Modal::ReviewForm(d), Field::Interaction) => {
            d.interaction_type =
                if fwd { d.interaction_type.next() } else { d.interaction_type.prev() }
        }
        (Modal::ReviewForm(d), Field::Rating) => d.adjust_rating(delta),
        (Modal::ReviewForm(d), Field::Score(s)) => d.adjust_score(s, delta),
        (Modal::ReviewForm(d), Field::Liked) => d.liked = !d.liked,
        (Modal::ReviewForm(d), Field::Spoilers) => d.contains_spoilers = !d.contains_spoilers,
        (Modal::ListForm(d), Field::ListRanked) => d.is_ranked = !d.is_ranked,
        (Modal::ListItemForm(d), Field::ItemMatch) => d.match_id = cycle_match(d.match_id, cards, delta),
        (Modal::TagForm(d), Field::TagKind) => {
            d.category = if fwd { d.category.next() } else { d.category.prev() }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

/// Panel con foco dentro de la pestaña Listas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListPane {
    Lists,
    Entries,
}

pub struct TuiApp {
    pub db: Database,
    pub state: AppState,
    /// Escribiendo en la barra de búsqueda (`/`).
    pub searching: bool,
    pub show_help: bool,
    /// Muestra las reseñas marcadas con spoilers.
    pub reveal_spoilers: bool,
    /// Campo con foco en el formulario abierto.
    pub form_focus: usize,
    /// Tag resaltado dentro del selector de flags.
    pub tag_cursor: usize,
    pub list_pane: ListPane,
    pub entry_cursor: usize,
    /// Columnas de la cuadrícula de Matches en el último frame (para navegar con j/k).
    pub grid_cols: Cell<usize>,
    /// Mostrar la foto en la ficha (`P`).
    pub show_photos: bool,
    /// Protocolo gráfico de la terminal (Sixel, Kitty, iTerm2 o bloques Unicode).
    pub picker: Picker,
    /// Fotos ya preparadas por ruta; `None` si no se pudo leer (no se reintenta).
    photos: RefCell<HashMap<String, Option<StatefulProtocol>>>,
}

/// Lado máximo (px) al que se reduce una foto antes de enviarla a la terminal:
/// codificar en Sixel una foto de 12 MP en cada redibujado sería muy lento.
const PHOTO_MAX_PX: u32 = 600;

fn load_photo(picker: &Picker, path: &str) -> Option<StatefulProtocol> {
    let img = image::open(path).ok()?;
    let img = if img.width().max(img.height()) > PHOTO_MAX_PX {
        img.thumbnail(PHOTO_MAX_PX, PHOTO_MAX_PX)
    } else {
        img
    };
    Some(picker.new_resize_protocol(img))
}

impl TuiApp {
    /// Con `Picker::halfblocks()`: funciona sin consultar la terminal.
    #[cfg(test)]
    pub fn new(db: Database, state: AppState) -> Self {
        Self::with_picker(db, state, Picker::halfblocks())
    }

    pub fn with_picker(db: Database, state: AppState, picker: Picker) -> Self {
        TuiApp {
            db,
            state,
            searching: false,
            show_help: false,
            reveal_spoilers: false,
            form_focus: 0,
            tag_cursor: 0,
            list_pane: ListPane::Lists,
            entry_cursor: 0,
            grid_cols: Cell::new(1),
            show_photos: true,
            picker,
            photos: RefCell::new(HashMap::new()),
        }
    }

    /// Foto lista para dibujar (se decodifica una vez y queda en caché).
    pub fn photo(&self, path: &str) -> Option<RefMut<'_, StatefulProtocol>> {
        let cache = self.photos.borrow_mut();
        RefMut::filter_map(cache, |c| {
            c.entry(path.to_string()).or_insert_with(|| load_photo(&self.picker, path)).as_mut()
        })
        .ok()
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.state.should_quit {
            terminal.draw(|frame| views::render(frame, &self))?;
            // En Windows llegan también eventos de soltar tecla: sólo nos interesan las pulsaciones.
            if let Event::Key(key) = event::read()?
                && key.kind != KeyEventKind::Release
            {
                self.on_key(key);
            }
        }
        Ok(())
    }

    /// Items de la lista seleccionada.
    pub fn current_entries(&self) -> &[ListEntry] {
        self.state.selected_list.map(|id| self.state.entries_of_list(id)).unwrap_or(&[])
    }

    /// Campo con foco del formulario abierto.
    pub fn focused_field(&self) -> Option<Field> {
        let fields = form_fields(self.state.modal.as_ref()?);
        fields.get(self.form_focus.min(fields.len().saturating_sub(1))).copied()
    }

    fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.state.should_quit = true;
            return;
        }
        let had_modal = self.state.modal.is_some();

        let result = if had_modal {
            self.on_modal_key(key)
        } else if self.show_help {
            self.show_help = false;
            Ok(())
        } else if self.searching {
            self.on_search_key(key);
            Ok(())
        } else {
            self.on_normal_key(key)
        };
        if let Err(e) = result {
            self.state.error(format!("{e:#}"));
        }

        // Formulario recién abierto: foco al primer campo.
        if !had_modal && self.state.modal.is_some() {
            self.form_focus = 0;
            self.tag_cursor = 0;
        }
        let len = self.current_entries().len();
        self.entry_cursor = self.entry_cursor.min(len.saturating_sub(1));
        if len == 0 {
            self.list_pane = ListPane::Lists;
        }
    }

    // ------------------------------------------------------------ modo normal

    fn on_normal_key(&mut self, key: KeyEvent) -> Result<()> {
        self.state.clear_status();
        let tab = self.state.active_tab;

        if tab == Tab::Lists && self.list_pane == ListPane::Entries && self.on_entries_key(key)? {
            return Ok(());
        }
        if matches!(tab, Tab::Cartelera | Tab::Watchlist) && self.on_filter_key(key) {
            return Ok(());
        }

        match key.code {
            KeyCode::Char('q') => self.state.should_quit = true,
            KeyCode::Char('?') => self.show_help = true,

            // Pestañas
            KeyCode::Tab => self.switch_tab(tab.next()),
            KeyCode::BackTab => self.switch_tab(tab.prev()),
            KeyCode::Char(c @ '1'..='6') => self.switch_tab(Tab::ALL[c as usize - '1' as usize]),

            // Movimiento
            KeyCode::Char('j') | KeyCode::Down => self.move_vertical(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_vertical(-1),
            KeyCode::Char('h') | KeyCode::Left => {
                if tab == Tab::Cartelera {
                    self.state.move_cursor(-1)
                } else {
                    self.switch_tab(tab.prev())
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if tab == Tab::Cartelera {
                    self.state.move_cursor(1)
                } else {
                    self.switch_tab(tab.next())
                }
            }
            KeyCode::Char('g') | KeyCode::Home => self.state.cursor_to_start(),
            KeyCode::Char('G') | KeyCode::End => self.state.cursor_to_end(),
            KeyCode::PageDown => self.state.move_cursor(10),
            KeyCode::PageUp => self.state.move_cursor(-10),

            // Acciones
            KeyCode::Char('n') => {
                if tab == Tab::Lists {
                    self.state.open_new_list()
                } else {
                    self.state.open_new_match()
                }
            }
            KeyCode::Char('r') => self.state.open_new_review(),
            KeyCode::Char('e') => match tab {
                Tab::Dashboard | Tab::Diary => self.state.open_edit_selected_log(),
                Tab::Cartelera | Tab::Watchlist => self.state.open_edit_selected_match(),
                Tab::Lists => self.state.open_edit_selected_list(),
                Tab::Stats => {}
            },
            KeyCode::Char('d') | KeyCode::Delete => self.state.open_delete_selected(),
            KeyCode::Char('f') => self.state.toggle_favorite_selected(&self.db)?,
            KeyCode::Char('s') => self.state.cycle_status_selected(&self.db)?,
            KeyCode::Char('t') => self.state.open_new_tag(),
            KeyCode::Char('P') => {
                self.show_photos = !self.show_photos;
                self.state.info(if self.show_photos { "Fotos visibles" } else { "Fotos ocultas" });
            }
            KeyCode::Char('u') => self.state.cycle_owner_filter(&self.db)?,
            KeyCode::Char('X') => self.state.open_delete_owner(),
            KeyCode::Char('E') => self.state.quick_export(&self.db)?,
            KeyCode::Char('I') => self.state.open_import(None),
            KeyCode::Char('v') => {
                self.reveal_spoilers = !self.reveal_spoilers;
                self.state.info(if self.reveal_spoilers {
                    "Spoilers visibles"
                } else {
                    "Spoilers ocultos"
                });
            }
            KeyCode::Char('a') if tab == Tab::Lists => self.state.open_add_to_list(),
            KeyCode::Enter if tab == Tab::Lists => {
                if self.current_entries().is_empty() {
                    self.state.info("Lista vacía: pulsa a para añadir un match");
                } else {
                    self.list_pane = ListPane::Entries;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn switch_tab(&mut self, tab: Tab) {
        self.list_pane = ListPane::Lists;
        self.state.set_tab(tab);
    }

    fn move_vertical(&mut self, delta: isize) {
        let tab = self.state.active_tab;
        if tab != Tab::Cartelera {
            let before = self.state.selected_list;
            self.state.move_cursor(delta);
            if self.state.selected_list != before {
                self.entry_cursor = 0;
            }
            return;
        }
        // En la cuadrícula j/k saltan una fila completa.
        let cols = self.grid_cols.get().max(1);
        let cursor = self.state.cursor();
        let len = self.state.item_count(tab);
        if delta < 0 && cursor < cols {
            return;
        }
        if delta > 0 && cursor / cols == len.saturating_sub(1) / cols {
            return;
        }
        self.state.move_cursor(delta * cols as isize);
    }

    /// Filtros de Matches / Watchlist. Devuelve `true` si consumió la tecla.
    fn on_filter_key(&mut self, key: KeyEvent) -> bool {
        let f = &mut self.state.filters;
        match key.code {
            KeyCode::Char('/') => {
                self.searching = true;
                return true;
            }
            KeyCode::Char('p') => f.cycle_platform(),
            KeyCode::Char('c') if self.state.active_tab == Tab::Cartelera => f.cycle_status(),
            KeyCode::Char('o') => f.sort = f.sort.next(),
            KeyCode::Char('L') => f.only_liked = !f.only_liked,
            KeyCode::Char('*') => f.only_favorites = !f.only_favorites,
            KeyCode::Char('x') => f.clear(),
            _ => return false,
        }
        self.state.filters_changed();
        true
    }

    fn on_search_key(&mut self, key: KeyEvent) {
        let search = &mut self.state.filters.search;
        match key.code {
            KeyCode::Enter => self.searching = false,
            KeyCode::Esc => {
                search.clear();
                self.searching = false;
            }
            KeyCode::Backspace => {
                search.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => search.clear(),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => search.push(c),
            _ => return,
        }
        self.state.filters_changed();
    }

    /// Teclas del panel de items de una lista. Devuelve `true` si consumió la tecla.
    fn on_entries_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(list_id) = self.state.selected_list else { return Ok(false) };
        let entries = self.current_entries();
        let len = entries.len();
        let Some(match_id) = entries.get(self.entry_cursor).map(|e| e.item.match_id) else {
            return Ok(false);
        };

        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left | KeyCode::Enter => {
                self.list_pane = ListPane::Lists
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.entry_cursor = (self.entry_cursor + 1).min(len - 1)
            }
            KeyCode::Char('k') | KeyCode::Up => self.entry_cursor = self.entry_cursor.saturating_sub(1),
            KeyCode::Char('J') => {
                self.state.move_list_item(&self.db, match_id, false)?;
                self.entry_cursor = (self.entry_cursor + 1).min(len - 1);
            }
            KeyCode::Char('K') => {
                self.state.move_list_item(&self.db, match_id, true)?;
                self.entry_cursor = self.entry_cursor.saturating_sub(1);
            }
            KeyCode::Char('x') | KeyCode::Char('d') | KeyCode::Delete => {
                self.state.open_delete(DeleteTarget::ListItem { list_id, match_id })
            }
            KeyCode::Char('r') => {
                self.state.selected_match = Some(match_id);
                self.state.open_new_review();
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    // ---------------------------------------------------------------- modales

    fn on_modal_key(&mut self, key: KeyEvent) -> Result<()> {
        if let Some(Modal::ConfirmDelete(_)) = self.state.modal {
            match key.code {
                KeyCode::Char('s' | 'S' | 'y' | 'Y') | KeyCode::Enter => {
                    self.state.submit_modal(&self.db)?
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => self.state.close_modal(),
                _ => {}
            }
            return Ok(());
        }

        let Some(field) = self.focused_field() else { return Ok(()) };
        let n = self.state.modal.as_ref().map(|m| form_fields(m).len()).unwrap_or(1);
        let kind = field.kind();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => {
                self.state.close_modal();
                self.state.clear_status();
            }
            KeyCode::Enter => self.state.submit_modal(&self.db)?,
            KeyCode::Char('s') if ctrl => self.state.submit_modal(&self.db)?,
            KeyCode::Tab | KeyCode::Down => self.focus_field((self.form_focus + 1) % n),
            KeyCode::BackTab | KeyCode::Up => self.focus_field((self.form_focus + n - 1) % n),
            // En campos que no son de texto también vale moverse con j/k y cambiar con h/l.
            KeyCode::Char('j') if kind != FieldKind::Text => self.focus_field((self.form_focus + 1) % n),
            KeyCode::Char('k') if kind != FieldKind::Text => {
                self.focus_field((self.form_focus + n - 1) % n)
            }
            KeyCode::Left => self.change_field(field, -1),
            KeyCode::Right => self.change_field(field, 1),
            KeyCode::Char('h') if kind != FieldKind::Text => self.change_field(field, -1),
            KeyCode::Char('l') if kind != FieldKind::Text => self.change_field(field, 1),
            KeyCode::Char(' ') if kind != FieldKind::Text => self.toggle_field(field),
            KeyCode::Backspace => {
                if let Some(text) = self.state.modal.as_mut().and_then(|m| text_mut(m, field)) {
                    text.pop();
                }
            }
            KeyCode::Char('u') if ctrl => {
                if let Some(text) = self.state.modal.as_mut().and_then(|m| text_mut(m, field)) {
                    text.clear();
                }
            }
            KeyCode::Char(c) if !ctrl => {
                if let Some(text) = self.state.modal.as_mut().and_then(|m| text_mut(m, field)) {
                    text.push(c);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn focus_field(&mut self, index: usize) {
        self.form_focus = index;
        self.tag_cursor = 0;
        // Al salir del campo de ruta, leer el archivo para mostrar qué contiene.
        if let Some(Modal::ImportForm(d)) = &mut self.state.modal {
            d.refresh_preview();
        }
    }

    fn change_field(&mut self, field: Field, delta: i32) {
        match field.kind() {
            FieldKind::Text => {}
            FieldKind::Tags => {
                let n = self.state.tags.len();
                if n > 0 {
                    self.tag_cursor = (self.tag_cursor as i32 + delta).rem_euclid(n as i32) as usize;
                }
            }
            FieldKind::Choice | FieldKind::Toggle => {
                // Sólo se reseñan o listan matches propios.
                let own = self.state.own_cards();
                if let Some(modal) = &mut self.state.modal {
                    adjust(modal, field, delta, &own);
                }
            }
        }
    }

    fn toggle_field(&mut self, field: Field) {
        match field.kind() {
            FieldKind::Tags => {
                let AppState { modal, tags, .. } = &mut self.state;
                if let (Some(Modal::MatchForm(d)), Some(tag)) = (modal, tags.get(self.tag_cursor)) {
                    d.toggle_tag(tag.id);
                }
            }
            _ => self.change_field(field, 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn app() -> TuiApp {
        let db = Database::open_in_memory().unwrap();
        db.seed_if_empty().unwrap();
        let state = AppState::load(&db).unwrap();
        TuiApp::new(db, state)
    }

    fn press(app: &mut TuiApp, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn type_text(app: &mut TuiApp, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn draw(app: &TuiApp, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| views::render(f, app)).unwrap();
        let buf = terminal.backend().buffer();
        buf.content()
            .chunks(w as usize)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_every_tab_and_modal_at_any_size() {
        let mut app = app();
        for (w, h) in [(140, 45), (80, 24), (30, 10)] {
            for tab in Tab::ALL {
                app.state.set_tab(tab);
                draw(&app, w, h);
            }
            for key in ['n', 'r', 't', '?'] {
                app.state.set_tab(Tab::Cartelera);
                press(&mut app, KeyCode::Char(key));
                draw(&app, w, h);
                press(&mut app, KeyCode::Esc);
            }
            app.state.set_tab(Tab::Lists);
            for key in ['n', 'a', 'd'] {
                press(&mut app, KeyCode::Char(key));
                draw(&app, w, h);
                press(&mut app, KeyCode::Esc);
            }
        }
        let screen = draw(&app, 140, 45);
        assert!(screen.contains("TINDERBOXD"));
    }

    #[test]
    fn grid_navigation_moves_by_rows() {
        let mut app = app();
        app.state.set_tab(Tab::Cartelera);
        draw(&app, 100, 40); // calcula columnas de la cuadrícula
        let cols = app.grid_cols.get();
        assert!(cols >= 2);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.state.cursor(), 1);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.state.cursor(), (1 + cols).min(app.state.cards.len() - 1));
    }

    #[test]
    fn new_match_then_review_with_keyboard() {
        let mut app = app();
        app.state.set_tab(Tab::Cartelera);

        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "El DJ de bodas");
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "34");
        press(&mut app, KeyCode::Tab); // plataforma
        press(&mut app, KeyCode::Right); // Bumble
        // Flags es el último campo: marcar el primero
        for _ in 0..6 {
            press(&mut app, KeyCode::Tab);
        }
        assert_eq!(app.focused_field(), Some(Field::Tags));
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Enter);
        assert!(app.state.modal.is_none(), "{:?}", app.state.status);
        let card = app.state.selected_card().unwrap();
        assert_eq!(card.profile.alias, "El DJ de bodas");
        assert_eq!(card.profile.platform.as_str(), "Bumble");
        assert_eq!(card.tags.len(), 1);

        press(&mut app, KeyCode::Char('r'));
        for _ in 0..3 {
            press(&mut app, KeyCode::Tab); // → Calificación
        }
        assert_eq!(app.focused_field(), Some(Field::Rating));
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Right);
        for _ in 0..4 {
            press(&mut app, KeyCode::Up); // da la vuelta hasta Reseña (último campo)
        }
        assert_eq!(app.focused_field(), Some(Field::ReviewText));
        type_text(&mut app, "Puso Despacito tres veces.");
        press(&mut app, KeyCode::Enter);
        assert!(app.state.modal.is_none(), "{:?}", app.state.status);
        let entry = app.state.selected_entry().unwrap();
        assert_eq!(entry.match_alias, "El DJ de bodas");
        assert_eq!(entry.log.rating.value(), 8);
        assert_eq!(entry.log.review_text.as_deref(), Some("Puso Despacito tres veces."));
    }

    #[test]
    fn invalid_form_keeps_modal_and_shows_error() {
        let mut app = app();
        press(&mut app, KeyCode::Char('n'));
        press(&mut app, KeyCode::Enter); // alias vacío
        assert!(app.state.modal.is_some());
        assert!(draw(&app, 120, 40).contains("El alias es obligatorio"));
    }

    #[test]
    fn list_entries_reorder_and_delete() {
        let mut app = app();
        app.state.set_tab(Tab::Lists);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.list_pane, ListPane::Entries);
        let first = app.current_entries()[0].item.match_id;
        press(&mut app, KeyCode::Char('J'));
        assert_eq!(app.current_entries()[1].item.match_id, first);
        press(&mut app, KeyCode::Char('x'));
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.current_entries().len(), 2);
    }

    #[test]
    fn search_filters_matches() {
        let mut app = app();
        app.state.set_tab(Tab::Cartelera);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "crypto");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.state.visible_cards().len(), 1);
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.state.visible_cards().len(), 4);
    }

    #[test]
    fn photo_is_drawn_in_the_detail_panel() {
        let mut app = app();
        // PNG real de 40x60: mitad superior naranja, inferior azul.
        let img = image::RgbImage::from_fn(40, 60, |_, y| {
            if y < 30 { image::Rgb([255, 128, 0]) } else { image::Rgb([64, 188, 244]) }
        });
        let path = std::env::temp_dir().join(format!("tinderboxd_tui_photo_{}.png", std::process::id()));
        img.save(&path).unwrap();
        let stored = app.db.import_photo(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        let id = app.state.cards[0].profile.id;
        let mut profile = app.state.cards[0].profile.clone();
        profile.avatar_path = Some(stored.clone());
        app.db.update_match(&profile).unwrap();
        app.state.refresh(&app.db).unwrap();
        app.state.set_tab(Tab::Cartelera);
        app.state.select_match(id);

        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        terminal.draw(|f| views::render(f, &app)).unwrap();
        let buf = terminal.backend().buffer();
        let orange = ratatui::style::Color::Rgb(255, 128, 0);
        let painted = buf.content().iter().filter(|c| c.fg == orange || c.bg == orange).count();
        assert!(painted > 20, "la foto debería pintarse con bloques de color ({painted} celdas)");

        // Con P se oculta y aparece la ruta en su lugar.
        press(&mut app, KeyCode::Char('P'));
        assert!(draw(&app, 140, 40).contains("P para mostrarla"));
        app.db.remove_photo(&stored);
    }

    #[test]
    fn export_and_import_with_keyboard() {
        let mut app = app();
        app.state.set_tab(Tab::Stats);
        press(&mut app, KeyCode::Char('E'));
        let msg = app.state.status.clone().unwrap();
        assert!(!msg.is_error, "{}", msg.text);
        let path = msg.text.split(" a ").last().unwrap().to_string();
        assert!(std::path::Path::new(&path).is_file(), "{path}");

        press(&mut app, KeyCode::Char('I'));
        type_text(&mut app, &path);
        press(&mut app, KeyCode::Tab); // al salir del campo se lee el archivo
        type_text(&mut app, "Ana");
        assert!(draw(&app, 120, 40).contains("Respaldo completo"));
        press(&mut app, KeyCode::Enter);
        assert!(app.state.modal.is_none(), "{:?}", app.state.status);
        let _ = std::fs::remove_file(&path);

        // Queda en su propio perfil (sin prefijo en los alias) y la cabecera lo indica.
        assert_eq!(app.state.filter_label(), "Ana");
        assert_eq!(app.state.cards.len(), 4);
        assert!(app.state.cards.iter().any(|c| c.profile.alias == "Crypto Bro de Polanco"));
        let screen = draw(&app, 140, 40);
        assert!(screen.contains("Perfil: Ana"), "{screen}");
        assert!(screen.contains("sólo lectura"));

        // u cicla: Ana → Todos → Tú.
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.state.cards.len(), 8);
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.state.filter_label(), "Tú");
        assert_eq!(app.state.cards.len(), 4);
    }

    /// `cargo test preview -- --ignored --nocapture` imprime las pantallas.
    #[test]
    #[ignore]
    fn preview() {
        let mut app = app();
        for tab in Tab::ALL {
            app.state.set_tab(tab);
            println!("{}\n", draw(&app, 120, 34));
        }
        app.state.set_tab(Tab::Diary);
        press(&mut app, KeyCode::Char('e'));
        println!("{}\n", draw(&app, 120, 34));
    }
}
