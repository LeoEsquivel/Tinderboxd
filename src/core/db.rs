//! Capa SQLite de Tinderboxd: esquema (Punto 4), CRUD, estadísticas (Punto 5) y semilla.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Local, NaiveDate};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{Connection, OptionalExtension, Row, ToSql, params};

use super::models::*;

/// Nombre por defecto del archivo de base de datos.
pub const DB_FILE_NAME: &str = "tinderboxd.db";

/// Ubicación por defecto de la base de datos: la carpeta de datos del usuario
/// (`%APPDATA%\tinderboxd` en Windows, `~/.local/share/tinderboxd` en Linux,
/// `~/Library/Application Support/tinderboxd` en macOS). Así da igual desde qué
/// carpeta se ejecute el comando.
pub fn default_db_path() -> PathBuf {
    match dirs::data_dir() {
        Some(dir) => dir.join("tinderboxd").join(DB_FILE_NAME),
        None => PathBuf::from(DB_FILE_NAME),
    }
}

/// Tamaño máximo del "Top 4" del perfil.
pub const MAX_FAVORITES: usize = 4;

/// Carpeta (junto a la base de datos) donde se copian las fotos de los matches.
pub const PHOTOS_DIR: &str = "tinderboxd_fotos";

/// Extensiones de imagen aceptadas para las fotos.
pub const PHOTO_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

const SCHEMA_VERSION: i64 = 2;

/// v1: esquema del Punto 4 de AGENT.md.
const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS matches (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    alias         TEXT    NOT NULL,
    age           INTEGER,
    platform      TEXT    NOT NULL CHECK (platform IN ('Tinder','Bumble','Hinge','IG','IRL')),
    bio_highlight TEXT,
    avatar_path   TEXT,
    match_date    TEXT,
    status        TEXT    NOT NULL CHECK (status IN ('Watchlist','Activa','Ghosting Mutuo','Ghosteado','Unmatched','Solo Amigos')),
    is_favorite   BOOLEAN NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS logs (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    match_id             INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    interaction_type     TEXT    NOT NULL CHECK (interaction_type IN ('Chat','Llamada','Primera Cita','Cita Casual')),
    date_logged          TEXT    NOT NULL,
    rating               INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 10),
    script_score         INTEGER CHECK (script_score BETWEEN 1 AND 5),
    cinematography_score INTEGER CHECK (cinematography_score BETWEEN 1 AND 5),
    direction_score      INTEGER CHECK (direction_score BETWEEN 1 AND 5),
    liked                BOOLEAN NOT NULL DEFAULT 0,
    review_text          TEXT,
    contains_spoilers    BOOLEAN NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_logs_match ON logs(match_id);
CREATE INDEX IF NOT EXISTS idx_logs_date  ON logs(date_logged);

CREATE TABLE IF NOT EXISTS tags (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    name     TEXT NOT NULL UNIQUE,
    category TEXT NOT NULL CHECK (category IN ('Red Flag','Green Flag','Neutral'))
);

CREATE TABLE IF NOT EXISTS match_tags (
    match_id INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    tag_id   INTEGER NOT NULL REFERENCES tags(id)    ON DELETE CASCADE,
    PRIMARY KEY (match_id, tag_id)
);

CREATE TABLE IF NOT EXISTS lists (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    title       TEXT    NOT NULL,
    description TEXT,
    is_ranked   BOOLEAN NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS list_items (
    list_id    INTEGER NOT NULL REFERENCES lists(id)   ON DELETE CASCADE,
    match_id   INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    rank_order INTEGER NOT NULL,
    item_note  TEXT,
    PRIMARY KEY (list_id, match_id)
);
"#;

/// v2: dueños de datos importados. `owner_id = NULL` son tus datos; borrar un
/// dueño borra en cascada sus matches (con sus logs) y sus listas.
const MIGRATION_V2: &str = r#"
CREATE TABLE IF NOT EXISTS owners (
    id   INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
);
ALTER TABLE matches ADD COLUMN owner_id INTEGER REFERENCES owners(id) ON DELETE CASCADE;
ALTER TABLE lists   ADD COLUMN owner_id INTEGER REFERENCES owners(id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS idx_matches_owner ON matches(owner_id);
CREATE INDEX IF NOT EXISTS idx_lists_owner   ON lists(owner_id);
"#;

/// Condición SQL de un `OwnerFilter` sobre la columna `col`. El id es un entero
/// nuestro, así que interpolarlo es seguro.
fn owner_sql(col: &str, filter: OwnerFilter) -> String {
    match filter {
        OwnerFilter::Mine => format!("{col} IS NULL"),
        OwnerFilter::Owner(id) => format!("{col} = {id}"),
        OwnerFilter::All => "1 = 1".to_string(),
    }
}

// ---------------------------------------------------------------------------
// ToSql / FromSql para los tipos de dominio
// ---------------------------------------------------------------------------

macro_rules! sql_text_enum {
    ($($t:ty),+) => {$(
        impl ToSql for $t {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::from(self.as_str()))
            }
        }

        impl FromSql for $t {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                value
                    .as_str()?
                    .parse()
                    .map_err(|e: ParseEnumError| FromSqlError::Other(Box::new(e)))
            }
        }
    )+};
}

sql_text_enum!(Platform, MatchStatus, InteractionType, TagCategory);

impl ToSql for Rating {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.value() as i64))
    }
}

impl FromSql for Rating {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let v = value.as_i64()?;
        u8::try_from(v)
            .ok()
            .and_then(|v| Rating::new(v).ok())
            .ok_or(FromSqlError::OutOfRange(v))
    }
}

// ---------------------------------------------------------------------------
// Mapeo de filas
// ---------------------------------------------------------------------------

const MATCH_COLS: &str = "m.id, m.alias, m.age, m.platform, m.bio_highlight, m.avatar_path, m.match_date, \
     m.status, m.is_favorite, m.owner_id";

/// Número de columnas de `MATCH_COLS` (para leer lo que viene detrás).
const MATCH_NCOLS: usize = 10;

const LIST_COLS: &str = "l.id, l.title, l.description, l.is_ranked, l.owner_id";

const LOG_COLS: &str = "l.id, l.match_id, l.interaction_type, l.date_logged, l.rating, l.script_score, \
     l.cinematography_score, l.direction_score, l.liked, l.review_text, l.contains_spoilers";

/// Lee un `MatchProfile` empezando en la columna `o` (orden de `MATCH_COLS`).
fn match_from_row(row: &Row<'_>, o: usize) -> rusqlite::Result<MatchProfile> {
    Ok(MatchProfile {
        id: row.get(o)?,
        alias: row.get(o + 1)?,
        age: row.get(o + 2)?,
        platform: row.get(o + 3)?,
        bio_highlight: row.get(o + 4)?,
        avatar_path: row.get(o + 5)?,
        match_date: row.get(o + 6)?,
        status: row.get(o + 7)?,
        is_favorite: row.get(o + 8)?,
        owner_id: row.get(o + 9)?,
    })
}

/// Lee un `InteractionLog` empezando en la columna `o` (orden de `LOG_COLS`).
fn log_from_row(row: &Row<'_>, o: usize) -> rusqlite::Result<InteractionLog> {
    Ok(InteractionLog {
        id: row.get(o)?,
        match_id: row.get(o + 1)?,
        interaction_type: row.get(o + 2)?,
        date_logged: row.get(o + 3)?,
        rating: row.get(o + 4)?,
        script_score: row.get(o + 5)?,
        cinematography_score: row.get(o + 6)?,
        direction_score: row.get(o + 7)?,
        liked: row.get(o + 8)?,
        review_text: row.get(o + 9)?,
        contains_spoilers: row.get(o + 10)?,
    })
}

fn tag_from_row(row: &Row<'_>, o: usize) -> rusqlite::Result<Tag> {
    Ok(Tag { id: row.get(o)?, name: row.get(o + 1)?, category: row.get(o + 2)? })
}

fn list_from_row(row: &Row<'_>) -> rusqlite::Result<CustomList> {
    Ok(CustomList {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        is_ranked: row.get(3)?,
        owner_id: row.get(4)?,
    })
}

/// Extensión (en minúsculas) de una foto, si es un formato soportado.
pub fn photo_extension(path: &Path) -> Result<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .filter(|e| PHOTO_EXTENSIONS.contains(&e.as_str()))
        .ok_or_else(|| anyhow!("Formato de imagen no soportado: usa PNG, JPG o WEBP"))
}

fn validate_sub_score(name: &str, score: Option<u8>) -> Result<()> {
    if let Some(s) = score
        && !(1..=5).contains(&s)
    {
        bail!("la sub-métrica {name} debe estar entre 1 y 5 (recibido {s})");
    }
    Ok(())
}

fn validate_log(log: &InteractionLog) -> Result<()> {
    validate_sub_score("Conversación", log.script_score)?;
    validate_sub_score("Fotos vs. realidad", log.cinematography_score)?;
    validate_sub_score("Química", log.direction_score)
}

fn pct(part: u32, total: u32) -> Option<f64> {
    (total > 0).then(|| part as f64 * 100.0 / total as f64)
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

pub struct Database {
    conn: Connection,
    /// Carpeta de fotos gestionadas por la app.
    photos_dir: PathBuf,
}

impl Database {
    /// Abre (o crea) la base de datos en `path` y aplica el esquema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).with_context(|| format!("no se pudo crear {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("no se pudo abrir la base de datos {}", path.display()))?;
        let base = std::path::absolute(path)?.parent().map(Path::to_path_buf).unwrap_or_default();
        Self::from_connection(conn, base.join(PHOTOS_DIR))
    }

    /// Base de datos efímera para los tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let photos = std::env::temp_dir().join(format!("{PHOTOS_DIR}_{}", std::process::id()));
        Self::from_connection(Connection::open_in_memory()?, photos)
    }

    fn from_connection(conn: Connection, photos_dir: PathBuf) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Database { conn, photos_dir };
        db.migrate()?;
        Ok(db)
    }

    /// Ejecuta `f` dentro de una transacción: si devuelve error, no queda nada a medias.
    pub fn transaction<T>(&self, f: impl FnOnce(&Database) -> Result<T>) -> Result<T> {
        let tx = self.conn.unchecked_transaction()?;
        let out = f(self)?;
        tx.commit()?;
        Ok(out)
    }

    /// Carpeta donde vive la base de datos (y la carpeta de fotos).
    pub fn data_dir(&self) -> &Path {
        self.photos_dir.parent().unwrap_or(&self.photos_dir)
    }

    // ------------------------------------------------------------------- fotos
    //
    // En SQLite las fotos gestionadas se guardan como nombre de archivo relativo a
    // `photos_dir`; hacia fuera `MatchProfile::avatar_path` siempre es absoluta.

    /// `true` si la ruta apunta a una foto ya copiada en la carpeta de la app.
    pub fn is_managed_photo(&self, path: &str) -> bool {
        Path::new(path).starts_with(&self.photos_dir)
    }

    /// Copia una imagen a la carpeta de fotos y devuelve su nueva ruta absoluta.
    pub fn import_photo(&self, src: &Path) -> Result<String> {
        if !src.is_file() {
            bail!("No encuentro la imagen {}", src.display());
        }
        let ext = photo_extension(src)?;
        let bytes = fs::read(src).with_context(|| format!("no se pudo leer {}", src.display()))?;
        self.import_photo_bytes(&bytes, &ext)
    }

    /// Guarda una foto (p. ej. decodificada de un JSON importado) en la carpeta de fotos.
    pub fn import_photo_bytes(&self, bytes: &[u8], ext: &str) -> Result<String> {
        let ext = ext.to_lowercase();
        if !PHOTO_EXTENSIONS.contains(&ext.as_str()) {
            bail!("Formato de imagen no soportado ({ext}): usa PNG, JPG o WEBP");
        }
        fs::create_dir_all(&self.photos_dir)
            .with_context(|| format!("no se pudo crear {}", self.photos_dir.display()))?;

        static SEQ: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "{}-{}.{ext}",
            Local::now().format("%Y%m%d-%H%M%S%3f"),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let dest = self.photos_dir.join(name);
        fs::write(&dest, bytes).with_context(|| format!("no se pudo escribir {}", dest.display()))?;
        Ok(dest.to_string_lossy().into_owned())
    }

    /// Borra una foto gestionada (las rutas externas nunca se tocan).
    pub fn remove_photo(&self, path: &str) {
        if self.is_managed_photo(path) {
            let _ = fs::remove_file(path);
        }
    }

    /// Ruta absoluta → valor a guardar en la BD.
    fn stored_photo(&self, path: &Option<String>) -> Option<String> {
        path.as_ref().map(|p| match Path::new(p).strip_prefix(&self.photos_dir) {
            Ok(rel) => rel.to_string_lossy().into_owned(),
            Err(_) => p.clone(),
        })
    }

    /// Lee un `MatchProfile` resolviendo la foto a ruta absoluta.
    fn match_row(&self, row: &Row<'_>, o: usize) -> rusqlite::Result<MatchProfile> {
        let mut p = match_from_row(row, o)?;
        p.avatar_path = p.avatar_path.map(|s| {
            if Path::new(&s).is_relative() { self.photos_dir.join(s).to_string_lossy().into_owned() } else { s }
        });
        Ok(p)
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }
        self.transaction(|db| {
            if version < 1 {
                db.conn.execute_batch(SCHEMA_V1).context("error creando el esquema")?;
            }
            if version < 2 {
                db.conn.execute_batch(MIGRATION_V2).context("error migrando a la versión 2")?;
            }
            db.conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
            Ok(())
        })
    }

    // ------------------------------------------------------------------ dueños

    pub fn list_owners(&self) -> Result<Vec<Owner>> {
        let mut stmt = self.conn.prepare("SELECT id, name FROM owners ORDER BY name COLLATE NOCASE")?;
        let rows = stmt.query_map([], |r| Ok(Owner { id: r.get(0)?, name: r.get(1)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Id del dueño con ese nombre (sin distinguir mayúsculas), creándolo si no existe.
    pub fn get_or_create_owner(&self, name: &str) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            bail!("el perfil necesita un nombre");
        }
        let existing: Option<i64> = self
            .conn
            .query_row("SELECT id FROM owners WHERE name = ?1", [name], |r| r.get(0))
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        self.conn.execute("INSERT INTO owners (name) VALUES (?1)", [name])?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Borra un dueño y, en cascada, todo lo importado de él (también sus fotos).
    pub fn delete_owner(&self, id: i64) -> Result<()> {
        let photos: Vec<String> =
            self.list_matches(OwnerFilter::Owner(id))?.into_iter().filter_map(|m| m.avatar_path).collect();
        self.conn.execute("DELETE FROM owners WHERE id = ?1", [id])?;
        for photo in photos {
            self.remove_photo(&photo);
        }
        Ok(())
    }

    // ----------------------------------------------------------------- matches

    pub fn insert_match(&self, m: &MatchProfile) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO matches (alias, age, platform, bio_highlight, avatar_path, match_date, status,
                                  is_favorite, owner_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                m.alias,
                m.age,
                m.platform,
                m.bio_highlight,
                self.stored_photo(&m.avatar_path),
                m.match_date,
                m.status,
                m.is_favorite,
                m.owner_id
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_match(&self, m: &MatchProfile) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE matches SET alias = ?2, age = ?3, platform = ?4, bio_highlight = ?5, avatar_path = ?6,
                    match_date = ?7, status = ?8, is_favorite = ?9
             WHERE id = ?1",
            params![
                m.id,
                m.alias,
                m.age,
                m.platform,
                m.bio_highlight,
                self.stored_photo(&m.avatar_path),
                m.match_date,
                m.status,
                m.is_favorite
            ],
        )?;
        if n == 0 {
            bail!("no existe el match con id {}", m.id);
        }
        Ok(())
    }

    /// Borra el match y, en cascada, sus logs, tags asociados e items de listas.
    pub fn delete_match(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM matches WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn get_match(&self, id: i64) -> Result<Option<MatchProfile>> {
        let sql = format!("SELECT {MATCH_COLS} FROM matches m WHERE m.id = ?1");
        Ok(self.conn.query_row(&sql, [id], |r| self.match_row(r, 0)).optional()?)
    }

    /// Matches del filtro, los más recientes primero.
    pub fn list_matches(&self, filter: OwnerFilter) -> Result<Vec<MatchProfile>> {
        let sql = format!(
            "SELECT {MATCH_COLS} FROM matches m WHERE {}
             ORDER BY m.match_date IS NULL, m.match_date DESC, m.id DESC",
            owner_sql("m.owner_id", filter)
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| self.match_row(r, 0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// El "Top 4" del perfil (cada dueño tiene el suyo).
    pub fn favorites(&self, filter: OwnerFilter) -> Result<Vec<MatchProfile>> {
        let sql = format!(
            "SELECT {MATCH_COLS} FROM matches m WHERE m.is_favorite = 1 AND {}
             ORDER BY m.id LIMIT {MAX_FAVORITES}",
            owner_sql("m.owner_id", filter)
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| self.match_row(r, 0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Marca/desmarca favorito respetando el límite del Top 4 de su dueño.
    pub fn set_favorite(&self, match_id: i64, favorite: bool) -> Result<()> {
        if favorite {
            // `IS` compara también NULL = NULL (tus propios matches).
            let (already, count): (bool, i64) = self.conn.query_row(
                "SELECT COALESCE((SELECT is_favorite FROM matches WHERE id = ?1), 0),
                        (SELECT COUNT(*) FROM matches WHERE is_favorite = 1
                           AND owner_id IS (SELECT owner_id FROM matches WHERE id = ?1))",
                [match_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if !already && count as usize >= MAX_FAVORITES {
                bail!("el Top {MAX_FAVORITES} está lleno: quita un favorito antes de añadir otro");
            }
        }
        let n = self
            .conn
            .execute("UPDATE matches SET is_favorite = ?2 WHERE id = ?1", params![match_id, favorite])?;
        if n == 0 {
            bail!("no existe el match con id {match_id}");
        }
        Ok(())
    }

    pub fn set_status(&self, match_id: i64, status: MatchStatus) -> Result<()> {
        self.conn
            .execute("UPDATE matches SET status = ?2 WHERE id = ?1", params![match_id, status])?;
        Ok(())
    }

    /// Tarjetas de matches con agregados y tags, ordenadas por fecha de match.
    pub fn match_cards(&self, filter: OwnerFilter) -> Result<Vec<MatchCard>> {
        let sql = format!(
            "SELECT {MATCH_COLS},
                    COUNT(l.id),
                    AVG(l.rating),
                    COALESCE(MAX(l.liked), 0),
                    MAX(l.date_logged),
                    (SELECT l2.rating FROM logs l2 WHERE l2.match_id = m.id
                      ORDER BY l2.date_logged DESC, l2.id DESC LIMIT 1)
             FROM matches m
             LEFT JOIN logs l ON l.match_id = m.id
             WHERE {}
             GROUP BY m.id
             ORDER BY m.match_date IS NULL, m.match_date DESC, m.id DESC",
            owner_sql("m.owner_id", filter)
        );
        let mut tags_by_match = self.tags_by_match()?;
        let mut stmt = self.conn.prepare(&sql)?;
        let o = MATCH_NCOLS;
        let rows = stmt.query_map([], |r| {
            Ok(MatchCard {
                profile: self.match_row(r, 0)?,
                log_count: r.get(o)?,
                avg_rating: r.get(o + 1)?,
                liked: r.get(o + 2)?,
                last_logged: r.get(o + 3)?,
                last_rating: r.get(o + 4)?,
                tags: Vec::new(),
            })
        })?;
        let mut cards = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        for card in &mut cards {
            card.tags = tags_by_match.remove(&card.profile.id).unwrap_or_default();
        }
        Ok(cards)
    }

    // -------------------------------------------------------------------- logs

    pub fn insert_log(&self, log: &InteractionLog) -> Result<i64> {
        validate_log(log)?;
        self.conn.execute(
            "INSERT INTO logs (match_id, interaction_type, date_logged, rating, script_score,
                               cinematography_score, direction_score, liked, review_text, contains_spoilers)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                log.match_id, log.interaction_type, log.date_logged, log.rating, log.script_score,
                log.cinematography_score, log.direction_score, log.liked, log.review_text,
                log.contains_spoilers
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_log(&self, log: &InteractionLog) -> Result<()> {
        validate_log(log)?;
        let n = self.conn.execute(
            "UPDATE logs SET match_id = ?2, interaction_type = ?3, date_logged = ?4, rating = ?5,
                    script_score = ?6, cinematography_score = ?7, direction_score = ?8, liked = ?9,
                    review_text = ?10, contains_spoilers = ?11
             WHERE id = ?1",
            params![
                log.id, log.match_id, log.interaction_type, log.date_logged, log.rating,
                log.script_score, log.cinematography_score, log.direction_score, log.liked,
                log.review_text, log.contains_spoilers
            ],
        )?;
        if n == 0 {
            bail!("no existe el log con id {}", log.id);
        }
        Ok(())
    }

    pub fn delete_log(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM logs WHERE id = ?1", [id])?;
        Ok(())
    }

    #[cfg(test)]
    pub fn get_log(&self, id: i64) -> Result<Option<InteractionLog>> {
        let sql = format!("SELECT {LOG_COLS} FROM logs l WHERE l.id = ?1");
        Ok(self.conn.query_row(&sql, [id], |r| log_from_row(r, 0)).optional()?)
    }

    /// Logs de un match en orden cronológico.
    pub fn logs_for_match(&self, match_id: i64) -> Result<Vec<InteractionLog>> {
        let sql =
            format!("SELECT {LOG_COLS} FROM logs l WHERE l.match_id = ?1 ORDER BY l.date_logged, l.id");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([match_id], |r| log_from_row(r, 0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Diary: interacciones del filtro, las más recientes primero. `limit = None` → todas.
    pub fn diary(&self, limit: Option<u32>, filter: OwnerFilter) -> Result<Vec<DiaryEntry>> {
        let sql = format!(
            "SELECT {LOG_COLS}, m.alias, m.platform, m.owner_id
             FROM logs l JOIN matches m ON m.id = l.match_id
             WHERE {}
             ORDER BY l.date_logged DESC, l.id DESC
             LIMIT ?1",
            owner_sql("m.owner_id", filter)
        );
        let limit = limit.map(i64::from).unwrap_or(-1);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([limit], |r| {
            Ok(DiaryEntry {
                log: log_from_row(r, 0)?,
                match_alias: r.get(11)?,
                platform: r.get(12)?,
                owner_id: r.get(13)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // -------------------------------------------------------------------- tags

    pub fn create_tag(&self, name: &str, category: TagCategory) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            bail!("el tag no puede estar vacío");
        }
        self.conn
            .execute("INSERT INTO tags (name, category) VALUES (?1, ?2)", params![name, category])
            .with_context(|| format!("no se pudo crear el tag '{name}' (¿ya existe?)"))?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Devuelve el id del tag con ese nombre, creándolo si no existe.
    pub fn get_or_create_tag(&self, name: &str, category: TagCategory) -> Result<i64> {
        let existing: Option<i64> = self
            .conn
            .query_row("SELECT id FROM tags WHERE name = ?1", [name.trim()], |r| r.get(0))
            .optional()?;
        match existing {
            Some(id) => Ok(id),
            None => self.create_tag(name, category),
        }
    }

    pub fn list_tags(&self) -> Result<Vec<Tag>> {
        let mut stmt =
            self.conn.prepare("SELECT id, name, category FROM tags ORDER BY category, name")?;
        let rows = stmt.query_map([], |r| tag_from_row(r, 0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn tags_for_match(&self, match_id: i64) -> Result<Vec<Tag>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.name, t.category FROM tags t
             JOIN match_tags mt ON mt.tag_id = t.id
             WHERE mt.match_id = ?1 ORDER BY t.category, t.name",
        )?;
        let rows = stmt.query_map([match_id], |r| tag_from_row(r, 0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn tags_by_match(&self) -> Result<HashMap<i64, Vec<Tag>>> {
        let mut stmt = self.conn.prepare(
            "SELECT mt.match_id, t.id, t.name, t.category FROM match_tags mt
             JOIN tags t ON t.id = mt.tag_id ORDER BY t.category, t.name",
        )?;
        let mut map: HashMap<i64, Vec<Tag>> = HashMap::new();
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, tag_from_row(r, 1)?)))?;
        for row in rows {
            let (match_id, tag) = row?;
            map.entry(match_id).or_default().push(tag);
        }
        Ok(map)
    }

    pub fn add_tag_to_match(&self, match_id: i64, tag_id: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO match_tags (match_id, tag_id) VALUES (?1, ?2)",
            params![match_id, tag_id],
        )?;
        Ok(())
    }

    pub fn remove_tag_from_match(&self, match_id: i64, tag_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM match_tags WHERE match_id = ?1 AND tag_id = ?2",
            params![match_id, tag_id],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------ listas

    pub fn create_list(&self, list: &CustomList) -> Result<i64> {
        if list.title.trim().is_empty() {
            bail!("la lista necesita un título");
        }
        self.conn.execute(
            "INSERT INTO lists (title, description, is_ranked, owner_id) VALUES (?1, ?2, ?3, ?4)",
            params![list.title.trim(), list.description, list.is_ranked, list.owner_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_list(&self, list: &CustomList) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE lists SET title = ?2, description = ?3, is_ranked = ?4 WHERE id = ?1",
            params![list.id, list.title.trim(), list.description, list.is_ranked],
        )?;
        if n == 0 {
            bail!("no existe la lista con id {}", list.id);
        }
        Ok(())
    }

    pub fn delete_list(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM lists WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn get_list(&self, id: i64) -> Result<Option<CustomList>> {
        let sql = format!("SELECT {LIST_COLS} FROM lists l WHERE l.id = ?1");
        Ok(self.conn.query_row(&sql, [id], list_from_row).optional()?)
    }

    pub fn list_lists(&self, filter: OwnerFilter) -> Result<Vec<CustomList>> {
        let sql =
            format!("SELECT {LIST_COLS} FROM lists l WHERE {} ORDER BY l.id", owner_sql("l.owner_id", filter));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], list_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Items de una lista con su match, en orden de ranking.
    pub fn list_entries(&self, list_id: i64) -> Result<Vec<ListEntry>> {
        let sql = format!(
            "SELECT li.list_id, li.match_id, li.rank_order, li.item_note, {MATCH_COLS}
             FROM list_items li JOIN matches m ON m.id = li.match_id
             WHERE li.list_id = ?1 ORDER BY li.rank_order, m.alias"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([list_id], |r| {
            Ok(ListEntry {
                item: ListItem {
                    list_id: r.get(0)?,
                    match_id: r.get(1)?,
                    rank_order: r.get(2)?,
                    item_note: r.get(3)?,
                },
                profile: self.match_row(r, 4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Añade (o actualiza la nota de) un match en una lista; siempre al final del ranking si es nuevo.
    pub fn add_list_item(&self, list_id: i64, match_id: i64, note: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO list_items (list_id, match_id, rank_order, item_note)
             VALUES (?1, ?2, (SELECT COALESCE(MAX(rank_order), 0) + 1 FROM list_items WHERE list_id = ?1), ?3)
             ON CONFLICT (list_id, match_id) DO UPDATE SET item_note = excluded.item_note",
            params![list_id, match_id, note],
        )?;
        Ok(())
    }

    pub fn remove_list_item(&self, list_id: i64, match_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM list_items WHERE list_id = ?1 AND match_id = ?2",
            params![list_id, match_id],
        )?;
        self.renumber_list(list_id)
    }

    /// Sube (`up = true`) o baja un item una posición en el ranking.
    pub fn move_list_item(&self, list_id: i64, match_id: i64, up: bool) -> Result<()> {
        let entries = self.list_entries(list_id)?;
        let Some(pos) = entries.iter().position(|e| e.item.match_id == match_id) else {
            bail!("el match {match_id} no está en la lista {list_id}");
        };
        let target = if up { pos.checked_sub(1) } else { Some(pos + 1).filter(|&t| t < entries.len()) };
        let Some(target) = target else { return Ok(()) };

        let mut order: Vec<i64> = entries.iter().map(|e| e.item.match_id).collect();
        order.swap(pos, target);
        self.write_list_order(list_id, &order)
    }

    /// Deja el ranking como 1..=n sin huecos tras un borrado.
    fn renumber_list(&self, list_id: i64) -> Result<()> {
        let order: Vec<i64> =
            self.list_entries(list_id)?.iter().map(|e| e.item.match_id).collect();
        self.write_list_order(list_id, &order)
    }

    fn write_list_order(&self, list_id: i64, order: &[i64]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (i, match_id) in order.iter().enumerate() {
            tx.execute(
                "UPDATE list_items SET rank_order = ?3 WHERE list_id = ?1 AND match_id = ?2",
                params![list_id, match_id, i as i64 + 1],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------------------------- stats

    /// Estadísticas sólo de los datos del filtro: nunca se mezclan perfiles salvo en `All`.
    pub fn stats(&self, filter: OwnerFilter) -> Result<StatsSummary> {
        let mut s = StatsSummary::default();
        let own = owner_sql("m.owner_id", filter);

        // Totales.
        let (total_matches, total_logs, liked_logs, avg): (u32, u32, u32, Option<f64>) =
            self.conn.query_row(
                &format!(
                    "SELECT (SELECT COUNT(*) FROM matches m WHERE {own}),
                            COUNT(*),
                            COALESCE(SUM(l.liked), 0),
                            AVG(l.rating)
                     FROM logs l JOIN matches m ON m.id = l.match_id
                     WHERE {own}"
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        s.total_matches = total_matches;
        s.total_logs = total_logs;
        s.liked_logs = liked_logs;
        s.average_rating = avg;

        // Distribución de estrellas (1..=10 medias estrellas).
        let mut stmt = self.conn.prepare(&format!(
            "SELECT l.rating, COUNT(*) FROM logs l JOIN matches m ON m.id = l.match_id
             WHERE {own} GROUP BY l.rating"
        ))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, Rating>(0)?, r.get::<_, u32>(1)?)))?;
        for row in rows {
            let (rating, count) = row?;
            s.rating_distribution[(rating.value() - 1) as usize] = count;
        }

        // Conteo por estado.
        let mut by_status: HashMap<MatchStatus, u32> = HashMap::new();
        let mut stmt =
            self.conn.prepare(&format!("SELECT m.status, COUNT(*) FROM matches m WHERE {own} GROUP BY m.status"))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, MatchStatus>(0)?, r.get::<_, u32>(1)?)))?;
        for row in rows {
            let (status, count) = row?;
            by_status.insert(status, count);
        }
        s.status_counts =
            MatchStatus::ALL.iter().map(|st| (*st, by_status.get(st).copied().unwrap_or(0))).collect();

        // Embudo: etapa máxima alcanzada por cada match según sus interacciones.
        let (total, chat, call, first_date, casual, ghosted_after_chat): (u32, u32, u32, u32, u32, u32) =
            self.conn.query_row(
                &format!(
                    "WITH reach AS (
                         SELECT m.id, m.status,
                                COALESCE(MAX(CASE l.interaction_type
                                    WHEN 'Chat'         THEN 1
                                    WHEN 'Llamada'      THEN 2
                                    WHEN 'Primera Cita' THEN 3
                                    WHEN 'Cita Casual'  THEN 4 END), 0) AS stage
                         FROM matches m LEFT JOIN logs l ON l.match_id = m.id
                         WHERE {own}
                         GROUP BY m.id
                     )
                     SELECT COUNT(*),
                            COALESCE(SUM(stage >= 1), 0),
                            COALESCE(SUM(stage >= 2), 0),
                            COALESCE(SUM(stage >= 3), 0),
                            COALESCE(SUM(stage >= 4), 0),
                            COALESCE(SUM(stage >= 1 AND status IN ('Ghosteado', 'Ghosting Mutuo')), 0)
                     FROM reach"
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )?;
        s.funnel = [
            ("Match", total),
            ("Chat", chat),
            ("Llamada", call),
            ("Primera Cita", first_date),
            ("Cita Casual", casual),
        ]
        .into_iter()
        .map(|(label, count)| FunnelStep {
            label: label.to_string(),
            count,
            pct_of_total: pct(count, total).unwrap_or(0.0),
        })
        .collect();
        s.chat_to_date_pct = pct(first_date, chat);
        s.ghosting_pct = pct(ghosted_after_chat, chat);

        // Top flags.
        s.top_red_flags = self.top_tags(TagCategory::RedFlag, 5, filter)?;
        s.top_green_flags = self.top_tags(TagCategory::GreenFlag, 5, filter)?;

        Ok(s)
    }

    /// Tags de una categoría ordenados por número de matches (del filtro) que los tienen.
    pub fn top_tags(&self, category: TagCategory, limit: u32, filter: OwnerFilter) -> Result<Vec<TagCount>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT t.id, t.name, t.category, COUNT(mt.match_id) AS c
             FROM tags t
             JOIN match_tags mt ON mt.tag_id = t.id
             JOIN matches m ON m.id = mt.match_id
             WHERE t.category = ?1 AND {}
             GROUP BY t.id
             ORDER BY c DESC, t.name
             LIMIT ?2",
            owner_sql("m.owner_id", filter)
        ))?;
        let rows = stmt.query_map(params![category, limit], |r| {
            Ok(TagCount { tag: tag_from_row(r, 0)?, count: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ------------------------------------------------------------------ semilla

    pub fn is_empty(&self) -> Result<bool> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM matches", [], |r| r.get(0))?;
        Ok(n == 0)
    }

    /// Inserta datos de ejemplo si no hay ningún match. Devuelve `true` si sembró.
    pub fn seed_if_empty(&self) -> Result<bool> {
        if !self.is_empty()? {
            return Ok(false);
        }
        self.transaction(seed)?;
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Datos semilla
// ---------------------------------------------------------------------------

struct SeedLog {
    kind: InteractionType,
    date: &'static str,
    rating: u8,
    scores: (u8, u8, u8),
    liked: bool,
    spoilers: bool,
    review: &'static str,
}

struct SeedMatch {
    alias: &'static str,
    age: u32,
    platform: Platform,
    bio: &'static str,
    date: &'static str,
    status: MatchStatus,
    favorite: bool,
    tags: &'static [(&'static str, TagCategory)],
    logs: &'static [SeedLog],
}

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("fecha semilla válida")
}

const SEED: &[SeedMatch] = &[
    SeedMatch {
        alias: "La chica del perro gigante",
        age: 27,
        platform: Platform::Tinder,
        bio: "Si Bruno (mi gran danés de 70 kg) no te aprueba, no hay segunda cita.",
        date: "2026-08-02",
        status: MatchStatus::Activa,
        favorite: true,
        tags: &[
            ("Ama a los animales", TagCategory::GreenFlag),
            ("Puntual", TagCategory::GreenFlag),
            ("Responde rápido", TagCategory::GreenFlag),
            ("Tiene un perro más grande que tu depa", TagCategory::Neutral),
        ],
        logs: &[
            SeedLog {
                kind: InteractionType::Chat,
                date: "2026-08-03",
                rating: 8,
                scores: (4, 4, 4),
                liked: true,
                spoilers: false,
                review: "Abrió con un meme de un gran danés en una silla de oficina. Respuestas \
                         rápidas, humor afinado y cero '¿hola, cómo estás?'. Muy buena señal.",
            },
            SeedLog {
                kind: InteractionType::Llamada,
                date: "2026-08-07",
                rating: 7,
                scores: (4, 3, 4),
                liked: false,
                spoilers: false,
                review: "40 minutos de llamada, 15 de ellos con ladridos de fondo. Se oía fatal, \
                         pero la plática fluyó sola.",
            },
            SeedLog {
                kind: InteractionType::PrimeraCita,
                date: "2026-08-15",
                rating: 9,
                scores: (5, 5, 4),
                liked: true,
                spoilers: true,
                review: "Llevó a Bruno al parque. Bruno me tiró al lodo en el minuto 3 y ella se rió \
                         tanto que le salió agua de coco por la nariz. Lo importante: Bruno me aprobó. \
                         Ya hay segunda cita agendada.",
            },
        ],
    },
    SeedMatch {
        alias: "El sommelier de mezcal",
        age: 31,
        platform: Platform::Hinge,
        bio: "Te puedo explicar 45 minutos la diferencia entre un espadín y un tobalá.",
        date: "2026-07-11",
        status: MatchStatus::Ghosteado,
        favorite: false,
        tags: &[
            ("Monólogo de 2 horas", TagCategory::RedFlag),
            ("Ghosting post-cita", TagCategory::RedFlag),
            ("Buen gusto musical", TagCategory::GreenFlag),
            ("Puntual", TagCategory::GreenFlag),
        ],
        logs: &[
            SeedLog {
                kind: InteractionType::Chat,
                date: "2026-07-12",
                rating: 7,
                scores: (4, 4, 3),
                liked: true,
                spoilers: false,
                review: "Mandó una playlist de son jarocho con jazz. Buen gusto, buena ortografía \
                         y preguntas interesantes. Prometía muchísimo.",
            },
            SeedLog {
                kind: InteractionType::PrimeraCita,
                date: "2026-07-20",
                rating: 4,
                scores: (2, 4, 1),
                liked: false,
                spoilers: true,
                review: "Llegó puntual, eso sí. Luego: dos horas de clase sobre el agave sin dejarme \
                         hablar. Pidió seis catas y propuso pagar a la mitad. No volvió a escribir. \
                         Sus fotos eran reales; su interés en mí, no.",
            },
        ],
    },
    SeedMatch {
        alias: "Crypto Bro de Polanco",
        age: 29,
        platform: Platform::Bumble,
        bio: "CEO de 3 startups (pre-revenue). Si no tienes wallet, no tienes futuro. WAGMI 🚀",
        date: "2026-09-01",
        status: MatchStatus::Unmatched,
        favorite: false,
        tags: &[
            ("Habla de crypto", TagCategory::RedFlag),
            ("Foto con pez", TagCategory::RedFlag),
            ("Monólogo de 2 horas", TagCategory::RedFlag),
            ("Foto con lentes de sol en interiores", TagCategory::Neutral),
        ],
        logs: &[SeedLog {
            kind: InteractionType::Chat,
            date: "2026-09-02",
            rating: 2,
            scores: (1, 2, 2),
            liked: false,
            spoilers: false,
            review: "Me ofreció invertir en su memecoin ($BROCOIN) antes de preguntarme el nombre. \
                     La foto con el pez era, de hecho, un NFT del pez. Unmatch a los diez mensajes.",
        }],
    },
    SeedMatch {
        alias: "La astróloga de la Roma Norte",
        age: 28,
        platform: Platform::Instagram,
        bio: "Sol en Leo, Luna en Escorpio, ascendente en 'no me escribas en Mercurio retrógrado'.",
        date: "2026-10-05",
        status: MatchStatus::Watchlist,
        favorite: true,
        tags: &[
            ("Signo zodiacal en la bio", TagCategory::Neutral),
            ("Memes de calidad", TagCategory::GreenFlag),
            ("Responde rápido", TagCategory::GreenFlag),
            ("Ghosting post-cita", TagCategory::RedFlag),
        ],
        logs: &[SeedLog {
            kind: InteractionType::Chat,
            date: "2026-10-06",
            rating: 6,
            scores: (4, 3, 3),
            liked: true,
            spoilers: false,
            review: "Me pidió mi hora exacta de nacimiento antes que mi nombre. Hizo mi carta astral \
                     y concluyó que 'tenemos potencial pero mi Venus da miedo'. La primera cita queda \
                     pendiente hasta que Mercurio deje de estar retrógrado.",
        }],
    },
];

fn seed(db: &Database) -> Result<()> {
    let mut match_ids: HashMap<&str, i64> = HashMap::new();

    for sm in SEED {
        let profile = MatchProfile {
            id: 0,
            alias: sm.alias.to_string(),
            age: Some(sm.age),
            platform: sm.platform,
            bio_highlight: Some(sm.bio.to_string()),
            avatar_path: None,
            match_date: Some(date(sm.date)),
            status: sm.status,
            is_favorite: sm.favorite,
            owner_id: None,
        };
        let match_id = db.insert_match(&profile)?;
        match_ids.insert(sm.alias, match_id);

        for (name, category) in sm.tags {
            let tag_id = db.get_or_create_tag(name, *category)?;
            db.add_tag_to_match(match_id, tag_id)?;
        }

        for sl in sm.logs {
            let review = sl.review.split_whitespace().collect::<Vec<_>>().join(" ");
            db.insert_log(&InteractionLog {
                id: 0,
                match_id,
                interaction_type: sl.kind,
                date_logged: date(sl.date),
                rating: Rating::new(sl.rating).map_err(anyhow::Error::msg)?,
                script_score: Some(sl.scores.0),
                cinematography_score: Some(sl.scores.1),
                direction_score: Some(sl.scores.2),
                liked: sl.liked,
                review_text: Some(review),
                contains_spoilers: sl.spoilers,
            })?;
        }
    }

    let list_id = db.create_list(&CustomList {
        id: 0,
        title: "Citas que debieron quedarse en el chat".to_string(),
        description: Some(
            "Ranking de mis peores experiencias en apps. Leer con precaución y poca fe en el amor."
                .to_string(),
        ),
        is_ranked: true,
        owner_id: None,
    })?;
    for (alias, note) in [
        ("Crypto Bro de Polanco", "Me mandó su pitch deck en lugar de su Instagram."),
        ("El sommelier de mezcal", "Dos horas de monólogo sobre agave y aun así pagamos a la mitad."),
        ("La astróloga de la Roma Norte", "Sigo sin cita: Mercurio continúa retrógrado."),
    ] {
        db.add_list_item(list_id, match_ids[alias], Some(note))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> Database {
        let db = Database::open_in_memory().unwrap();
        assert!(db.seed_if_empty().unwrap());
        db
    }

    #[test]
    fn seed_is_idempotent() {
        let db = seeded();
        assert!(!db.seed_if_empty().unwrap());
        assert_eq!(db.list_matches(OwnerFilter::Mine).unwrap().len(), 4);
        assert_eq!(db.diary(None, OwnerFilter::Mine).unwrap().len(), 7);
        assert_eq!(db.list_lists(OwnerFilter::Mine).unwrap().len(), 1);
    }

    #[test]
    fn crud_match_and_cascade() {
        let db = seeded();
        let mut m = MatchProfile::new("El del gym a las 5am", Platform::Irl, MatchStatus::Watchlist);
        m.age = Some(30);
        let id = db.insert_match(&m).unwrap();
        m.id = id;
        m.status = MatchStatus::SoloAmigos;
        db.update_match(&m).unwrap();
        assert_eq!(db.get_match(id).unwrap().unwrap().status, MatchStatus::SoloAmigos);

        let log_id = db
            .insert_log(&InteractionLog {
                id: 0,
                match_id: id,
                interaction_type: InteractionType::CitaCasual,
                date_logged: date("2026-10-10"),
                rating: Rating::new(5).unwrap(),
                script_score: Some(3),
                cinematography_score: None,
                direction_score: Some(2),
                liked: false,
                review_text: None,
                contains_spoilers: false,
            })
            .unwrap();
        let tag = db.get_or_create_tag("Leg day eterno", TagCategory::Neutral).unwrap();
        db.add_tag_to_match(id, tag).unwrap();

        db.delete_match(id).unwrap();
        assert!(db.get_match(id).unwrap().is_none());
        assert!(db.get_log(log_id).unwrap().is_none(), "los logs deben borrarse en cascada");
    }

    #[test]
    fn photos_are_copied_stored_relative_and_removed() {
        let db = seeded();
        let src = std::env::temp_dir().join(format!("tinderboxd_test_src_{}.png", std::process::id()));
        fs::write(&src, b"no es un png real, pero sirve para copiar").unwrap();

        let imported = db.import_photo(&src).unwrap();
        assert!(db.is_managed_photo(&imported));
        assert!(Path::new(&imported).is_file());

        let mut m = MatchProfile::new("Con foto", Platform::Tinder, MatchStatus::Activa);
        m.avatar_path = Some(imported.clone());
        let id = db.insert_match(&m).unwrap();
        let raw: String =
            db.conn.query_row("SELECT avatar_path FROM matches WHERE id = ?1", [id], |r| r.get(0)).unwrap();
        assert!(Path::new(&raw).is_relative(), "en la BD se guarda relativa: {raw}");
        assert_eq!(db.get_match(id).unwrap().unwrap().avatar_path.as_deref(), Some(imported.as_str()));

        db.remove_photo(&imported);
        assert!(!Path::new(&imported).exists());
        db.remove_photo(src.to_str().unwrap()); // externa: no se toca
        assert!(src.exists());
        let _ = fs::remove_file(&src);

        let bad = std::env::temp_dir().join("tinderboxd_test.gif");
        fs::write(&bad, b"gif").unwrap();
        assert!(db.import_photo(&bad).is_err());
        let _ = fs::remove_file(&bad);
    }

    #[test]
    fn owners_keep_data_and_stats_separate() {
        let db = seeded();
        let ana = db.get_or_create_owner("Ana").unwrap();
        assert_eq!(db.get_or_create_owner("  ana ").unwrap(), ana, "sin distinguir mayúsculas");

        let mut m = MatchProfile::new("El de Ana", Platform::Bumble, MatchStatus::Activa);
        m.owner_id = Some(ana);
        m.is_favorite = true;
        let id = db.insert_match(&m).unwrap();
        db.insert_log(&InteractionLog {
            id: 0,
            match_id: id,
            interaction_type: InteractionType::PrimeraCita,
            date_logged: date("2026-10-01"),
            rating: Rating::new(10).unwrap(),
            script_score: None,
            cinematography_score: None,
            direction_score: None,
            liked: true,
            review_text: None,
            contains_spoilers: false,
        })
        .unwrap();
        let tag = db.get_or_create_tag("Puntual", TagCategory::GreenFlag).unwrap();
        db.add_tag_to_match(id, tag).unwrap();

        // Tus estadísticas no cambian; las de Ana sólo ven lo suyo.
        let mine = db.stats(OwnerFilter::Mine).unwrap();
        assert_eq!((mine.total_matches, mine.total_logs), (4, 7));
        assert_eq!(mine.rating_distribution[9], 0);
        let hers = db.stats(OwnerFilter::Owner(ana)).unwrap();
        assert_eq!((hers.total_matches, hers.total_logs), (1, 1));
        assert_eq!(hers.rating_distribution[9], 1);
        assert_eq!(hers.top_green_flags[0].count, 1);
        assert_eq!(db.stats(OwnerFilter::All).unwrap().total_matches, 5);

        // Cada dueño tiene su Top 4.
        assert_eq!(db.favorites(OwnerFilter::Mine).unwrap().len(), 2);
        assert_eq!(db.favorites(OwnerFilter::Owner(ana)).unwrap().len(), 1);
        assert_eq!(db.diary(None, OwnerFilter::Owner(ana)).unwrap()[0].owner_id, Some(ana));

        // Borrar a Ana borra lo suyo en cascada y nada tuyo.
        db.delete_owner(ana).unwrap();
        assert!(db.get_match(id).unwrap().is_none());
        assert_eq!(db.list_matches(OwnerFilter::All).unwrap().len(), 4);
        assert!(db.list_owners().unwrap().is_empty());
    }

    #[test]
    fn migrates_v1_database() {
        // Una BD creada con el esquema v1 (sin owners) se actualiza al abrirla.
        let path = std::env::temp_dir().join(format!("tinderboxd_v1_{}.db", std::process::id()));
        let _ = fs::remove_file(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA_V1).unwrap();
            conn.execute_batch(
                "PRAGMA user_version = 1;
                 INSERT INTO matches (alias, platform, status) VALUES ('Antiguo', 'Tinder', 'Activa');",
            )
            .unwrap();
        }
        let db = Database::open(&path).unwrap();
        let all = db.list_matches(OwnerFilter::Mine).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].owner_id, None, "lo que ya había es tuyo");
        db.get_or_create_owner("Ana").unwrap();
        drop(db);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn invalid_sub_score_is_rejected() {
        let db = seeded();
        let mut log = db.diary(Some(1), OwnerFilter::Mine).unwrap().remove(0).log;
        log.script_score = Some(9);
        assert!(db.update_log(&log).is_err());
    }

    #[test]
    fn favorites_capped_at_four() {
        let db = seeded();
        let ids: Vec<i64> = db.list_matches(OwnerFilter::Mine).unwrap().iter().map(|m| m.id).collect();
        for id in &ids {
            db.set_favorite(*id, true).unwrap();
        }
        let extra = db.insert_match(&MatchProfile::new("Quinto", Platform::Tinder, MatchStatus::Activa)).unwrap();
        assert!(db.set_favorite(extra, true).is_err());
        assert_eq!(db.favorites(OwnerFilter::Mine).unwrap().len(), 4);
    }

    #[test]
    fn stats_from_seed() {
        let db = seeded();
        let s = db.stats(OwnerFilter::Mine).unwrap();
        assert_eq!(s.total_matches, 4);
        assert_eq!(s.total_logs, 7);
        assert_eq!(s.rating_distribution.iter().sum::<u32>(), 7);
        assert_eq!(s.rating_distribution[6], 2); // dos logs de 3.5★
        assert_eq!(s.funnel[1].count, 4); // todos chatearon
        assert_eq!(s.funnel[3].count, 2); // dos llegaron a Primera Cita
        assert_eq!(s.chat_to_date_pct, Some(50.0));
        assert_eq!(s.ghosting_pct, Some(25.0));
        assert_eq!(s.top_red_flags[0].count, 2);
        assert_eq!(s.top_green_flags.len(), 5);
    }

    #[test]
    fn list_reordering() {
        let db = seeded();
        let list = db.list_lists(OwnerFilter::Mine).unwrap().remove(0);
        let before = db.list_entries(list.id).unwrap();
        let second = before[1].item.match_id;
        db.move_list_item(list.id, second, true).unwrap();
        let after = db.list_entries(list.id).unwrap();
        assert_eq!(after[0].item.match_id, second);
        db.remove_list_item(list.id, second).unwrap();
        let ranks: Vec<i64> = db.list_entries(list.id).unwrap().iter().map(|e| e.item.rank_order).collect();
        assert_eq!(ranks, vec![1, 2]);
    }

    #[test]
    fn cards_aggregate_logs_and_tags() {
        let db = seeded();
        let cards = db.match_cards(OwnerFilter::Mine).unwrap();
        let perro = cards.iter().find(|c| c.profile.alias.contains("perro")).unwrap();
        assert_eq!(perro.log_count, 3);
        assert!(perro.liked);
        assert_eq!(perro.last_rating, Some(Rating::new(9).unwrap()));
        assert_eq!(perro.tags.len(), 4);
    }
}
