//! Importar/Exportar JSON para el modo "Co-op" con amigos.
//!
//! Un archivo exportado es un `ExportBundle` autocontenido: matches con sus
//! reseñas, flags y (opcionalmente) fotos en base64, más listas que referencian
//! a los matches por una clave local (`"m1"`, `"m2"`…). Los ids de SQLite nunca
//! salen del equipo, así que importar en otra base de datos es seguro.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};

use super::db::{Database, MAX_FAVORITES, photo_extension};
use super::models::{
    CustomList, InteractionLog, InteractionType, MatchProfile, MatchStatus, Platform, Rating,
    TagCategory,
};

/// Identificador del formato en el campo `format`.
pub const FORMAT: &str = "tinderboxd";
/// Versión del formato que escribe esta app (y la máxima que sabe leer).
pub const FORMAT_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Formato del archivo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportKind {
    /// Respaldo de toda la base de datos.
    Completo,
    /// Un solo match con sus reseñas.
    Match,
    /// Una lista con los matches que contiene.
    Lista,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportBundle {
    pub format: String,
    pub version: u32,
    pub kind: ExportKind,
    /// Fecha y hora de exportación (RFC 3339).
    pub exported_at: String,
    pub app_version: String,
    pub matches: Vec<ExportMatch>,
    #[serde(default)]
    pub lists: Vec<ExportList>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportMatch {
    /// Clave local para que las listas referencien al match.
    pub key: String,
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<u32>,
    pub platform: Platform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio_highlight: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_date: Option<NaiveDate>,
    pub status: MatchStatus,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub tags: Vec<ExportTag>,
    #[serde(default)]
    pub logs: Vec<ExportLog>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<ExportPhoto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportTag {
    pub name: String,
    pub category: TagCategory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportLog {
    pub interaction_type: InteractionType,
    pub date_logged: NaiveDate,
    pub rating: Rating,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_score: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cinematography_score: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction_score: Option<u8>,
    #[serde(default)]
    pub liked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_text: Option<String>,
    #[serde(default)]
    pub contains_spoilers: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportPhoto {
    /// `png`, `jpg`, `jpeg` o `webp`.
    pub extension: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportList {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub is_ranked: bool,
    pub items: Vec<ExportListItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportListItem {
    pub match_key: String,
    pub rank_order: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ExportBundle {
    pub fn log_count(&self) -> usize {
        self.matches.iter().map(|m| m.logs.len()).sum()
    }

    pub fn photo_count(&self) -> usize {
        self.matches.iter().filter(|m| m.photo.is_some()).count()
    }

    /// Resumen legible para previsualizar antes de importar.
    pub fn describe(&self) -> String {
        let kind = match self.kind {
            ExportKind::Completo => "Respaldo completo exportado",
            ExportKind::Match => "Un match exportado",
            ExportKind::Lista => "Una lista exportada",
        };
        let date = chrono::DateTime::parse_from_rfc3339(&self.exported_at)
            .map(|d| d.format("%d/%m/%Y %H:%M").to_string())
            .unwrap_or_else(|_| self.exported_at.clone());
        format!(
            "{kind} el {date}: {} · {} · {} · {}",
            plural(self.matches.len(), "match", "matches"),
            plural(self.log_count(), "reseña", "reseñas"),
            plural(self.lists.len(), "lista", "listas"),
            plural(self.photo_count(), "foto", "fotos"),
        )
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ---------------------------------------------------------------------------
// Exportar
// ---------------------------------------------------------------------------

/// Qué se exporta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportTarget {
    All,
    Match(i64),
    List(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportOptions {
    /// Incluir las fotos en base64 (archivo más grande, pero autocontenido).
    pub include_photos: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions { include_photos: true }
    }
}

pub fn export(db: &Database, target: ExportTarget, opts: ExportOptions) -> Result<ExportBundle> {
    let (kind, match_ids, list_ids): (ExportKind, Vec<i64>, Vec<i64>) = match target {
        ExportTarget::All => (
            ExportKind::Completo,
            db.list_matches()?.iter().map(|m| m.id).collect(),
            db.list_lists()?.iter().map(|l| l.id).collect(),
        ),
        ExportTarget::Match(id) => {
            db.get_match(id)?.ok_or_else(|| anyhow!("no existe el match con id {id}"))?;
            (ExportKind::Match, vec![id], vec![])
        }
        ExportTarget::List(id) => {
            db.get_list(id)?.ok_or_else(|| anyhow!("no existe la lista con id {id}"))?;
            let ids = db.list_entries(id)?.iter().map(|e| e.item.match_id).collect();
            (ExportKind::Lista, ids, vec![id])
        }
    };

    let keys: HashMap<i64, String> =
        match_ids.iter().enumerate().map(|(i, id)| (*id, format!("m{}", i + 1))).collect();

    let mut matches = Vec::with_capacity(match_ids.len());
    for id in &match_ids {
        let Some(p) = db.get_match(*id)? else { continue };
        let photo = if opts.include_photos { p.avatar_path.as_deref().and_then(read_photo) } else { None };
        matches.push(ExportMatch {
            key: keys[id].clone(),
            alias: p.alias,
            age: p.age,
            platform: p.platform,
            bio_highlight: p.bio_highlight,
            match_date: p.match_date,
            status: p.status,
            is_favorite: p.is_favorite,
            tags: db
                .tags_for_match(*id)?
                .into_iter()
                .map(|t| ExportTag { name: t.name, category: t.category })
                .collect(),
            logs: db.logs_for_match(*id)?.into_iter().map(export_log).collect(),
            photo,
        });
    }

    let mut lists = Vec::with_capacity(list_ids.len());
    for id in list_ids {
        let Some(list) = db.get_list(id)? else { continue };
        lists.push(ExportList {
            title: list.title,
            description: list.description,
            is_ranked: list.is_ranked,
            items: db
                .list_entries(id)?
                .into_iter()
                .filter_map(|e| {
                    Some(ExportListItem {
                        match_key: keys.get(&e.item.match_id)?.clone(),
                        rank_order: e.item.rank_order,
                        note: e.item.item_note,
                    })
                })
                .collect(),
        });
    }

    Ok(ExportBundle {
        format: FORMAT.to_string(),
        version: FORMAT_VERSION,
        kind,
        exported_at: Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        matches,
        lists,
    })
}

fn export_log(l: InteractionLog) -> ExportLog {
    ExportLog {
        interaction_type: l.interaction_type,
        date_logged: l.date_logged,
        rating: l.rating,
        script_score: l.script_score,
        cinematography_score: l.cinematography_score,
        direction_score: l.direction_score,
        liked: l.liked,
        review_text: l.review_text,
        contains_spoilers: l.contains_spoilers,
    }
}

/// Lee una foto para incrustarla. Una foto ilegible no debe impedir exportar el resto.
fn read_photo(path: &str) -> Option<ExportPhoto> {
    let extension = photo_extension(Path::new(path)).ok()?;
    let bytes = fs::read(path).ok()?;
    Some(ExportPhoto { extension, data_base64: BASE64.encode(bytes) })
}

pub fn to_json(bundle: &ExportBundle) -> Result<String> {
    Ok(serde_json::to_string_pretty(bundle)?)
}

/// Exporta y escribe el archivo. Devuelve el bundle para poder informar qué se guardó.
pub fn export_to_file(db: &Database, target: ExportTarget, opts: ExportOptions, path: &Path) -> Result<ExportBundle> {
    let bundle = export(db, target, opts)?;
    fs::write(path, to_json(&bundle)?).with_context(|| format!("no se pudo escribir {}", path.display()))?;
    Ok(bundle)
}

/// Nombre sugerido: `tinderboxd-lista-citas-que-debieron-20261017.json`.
pub fn suggested_file_name(db: &Database, target: ExportTarget) -> Result<String> {
    let label = match target {
        ExportTarget::All => "respaldo".to_string(),
        ExportTarget::Match(id) => {
            let m = db.get_match(id)?.ok_or_else(|| anyhow!("no existe el match con id {id}"))?;
            format!("match-{}", slug(&m.alias))
        }
        ExportTarget::List(id) => {
            let l = db.get_list(id)?.ok_or_else(|| anyhow!("no existe la lista con id {id}"))?;
            format!("lista-{}", slug(&l.title))
        }
    };
    Ok(format!("tinderboxd-{label}-{}.json", Local::now().format("%Y%m%d")))
}

/// Texto → fragmento de nombre de archivo seguro en cualquier sistema.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            c if c.is_ascii_alphanumeric() => c,
            _ => '-',
        };
        if c != '-' || !out.ends_with('-') {
            out.push(c);
        }
    }
    let out: String = out.trim_matches('-').chars().take(40).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "sin-nombre".into() } else { out }
}

// ---------------------------------------------------------------------------
// Leer y validar
// ---------------------------------------------------------------------------

pub fn from_json(json: &str) -> Result<ExportBundle> {
    // Primero comprobar que es nuestro formato, para dar un error útil.
    let value: serde_json::Value = serde_json::from_str(json).context("el archivo no es un JSON válido")?;
    if value.get("format").and_then(|f| f.as_str()) != Some(FORMAT) {
        bail!("no es un archivo exportado por Tinderboxd");
    }
    if let Some(v) = value.get("version").and_then(|v| v.as_u64())
        && v > FORMAT_VERSION as u64
    {
        bail!("el archivo usa la versión {v} del formato; actualiza Tinderboxd para abrirlo");
    }
    let bundle: ExportBundle = serde_json::from_value(value).context("el archivo de Tinderboxd está dañado")?;
    validate(&bundle)?;
    Ok(bundle)
}

pub fn read_file(path: &Path) -> Result<ExportBundle> {
    let json = fs::read_to_string(path).with_context(|| format!("no se pudo leer {}", path.display()))?;
    from_json(&json)
}

fn validate(bundle: &ExportBundle) -> Result<()> {
    let mut keys = HashSet::new();
    for m in &bundle.matches {
        if m.alias.trim().is_empty() {
            bail!("hay un match sin alias");
        }
        if !keys.insert(m.key.as_str()) {
            bail!("clave de match repetida: {}", m.key);
        }
    }
    for list in &bundle.lists {
        for item in &list.items {
            if !keys.contains(item.match_key.as_str()) {
                bail!("la lista «{}» apunta a un match inexistente ({})", list.title, item.match_key);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Importar
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportOptions {
    /// Prefijo para los alias (p. ej. el nombre de tu amigo): `"Ana · La chica del perro gigante"`.
    pub alias_prefix: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub matches_created: usize,
    /// Matches que ya existían (mismo alias, app y fecha de match): se reutilizan.
    pub matches_existing: usize,
    pub logs_created: usize,
    /// Reseñas que ya existían (misma interacción y fecha): no se duplican.
    pub logs_skipped: usize,
    pub tags_created: usize,
    pub lists_created: usize,
    /// Listas idénticas (mismo título y mismos matches) que ya existían.
    pub lists_existing: usize,
    pub photos_imported: usize,
    /// Favoritos que no cupieron en el Top 4.
    pub favorites_dropped: usize,
}

impl ImportReport {
    pub fn summary(&self) -> String {
        if self.matches_created + self.logs_created + self.lists_created == 0 {
            return "Nada nuevo que importar: todo lo del archivo ya estaba en tu Tinderboxd".to_string();
        }
        let mut parts = vec![
            format!("{} nuevos", plural(self.matches_created, "match", "matches")),
            plural(self.logs_created, "reseña", "reseñas"),
            plural(self.lists_created, "lista", "listas"),
        ];
        if self.photos_imported > 0 {
            parts.push(plural(self.photos_imported, "foto", "fotos"));
        }
        if self.matches_existing > 0 {
            parts.push(format!("{} ya existían", self.matches_existing));
        }
        if self.logs_skipped > 0 {
            parts.push(format!("{} reseñas repetidas omitidas", self.logs_skipped));
        }
        if self.favorites_dropped > 0 {
            parts.push(format!("{} favoritos no cupieron en el Top {MAX_FAVORITES}", self.favorites_dropped));
        }
        format!("Importado: {}", parts.join(" · "))
    }
}

/// Importa un bundle en una sola transacción: o entra todo o no entra nada.
pub fn import(db: &Database, bundle: &ExportBundle, opts: &ImportOptions) -> Result<ImportReport> {
    validate(bundle)?;
    let mut written_photos = Vec::new();
    let result = db.transaction(|db| import_inner(db, bundle, opts, &mut written_photos));
    if result.is_err() {
        // La BD hizo rollback; las fotos copiadas también deben desaparecer.
        for photo in &written_photos {
            db.remove_photo(photo);
        }
    }
    result
}

pub fn import_file(db: &Database, path: &Path, opts: &ImportOptions) -> Result<ImportReport> {
    import(db, &read_file(path)?, opts)
}

fn import_inner(
    db: &Database,
    bundle: &ExportBundle,
    opts: &ImportOptions,
    written_photos: &mut Vec<String>,
) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    let existing = db.list_matches()?;
    let mut favorites = db.favorites()?.len();
    let mut known_tags: HashSet<String> = db.list_tags()?.into_iter().map(|t| t.name).collect();
    let mut key_to_id: HashMap<&str, i64> = HashMap::new();
    let prefix = opts.alias_prefix.as_deref().map(str::trim).filter(|p| !p.is_empty());

    for m in &bundle.matches {
        let alias = match prefix {
            Some(p) => format!("{p} · {}", m.alias.trim()),
            None => m.alias.trim().to_string(),
        };
        let found = existing
            .iter()
            .find(|e| e.alias == alias && e.platform == m.platform && e.match_date == m.match_date)
            .map(|e| e.id);

        let match_id = match found {
            Some(id) => {
                report.matches_existing += 1;
                id
            }
            None => {
                let avatar_path = match &m.photo {
                    Some(photo) => {
                        let bytes = BASE64
                            .decode(&photo.data_base64)
                            .with_context(|| format!("la foto de «{}» está dañada", m.alias))?;
                        let path = db.import_photo_bytes(&bytes, &photo.extension)?;
                        written_photos.push(path.clone());
                        report.photos_imported += 1;
                        Some(path)
                    }
                    None => None,
                };
                let is_favorite = m.is_favorite && favorites < MAX_FAVORITES;
                if is_favorite {
                    favorites += 1;
                } else if m.is_favorite {
                    report.favorites_dropped += 1;
                }
                report.matches_created += 1;
                db.insert_match(&MatchProfile {
                    id: 0,
                    alias,
                    age: m.age,
                    platform: m.platform,
                    bio_highlight: m.bio_highlight.clone(),
                    avatar_path,
                    match_date: m.match_date,
                    status: m.status,
                    is_favorite,
                })?
            }
        };
        key_to_id.insert(m.key.as_str(), match_id);

        for tag in &m.tags {
            if known_tags.insert(tag.name.trim().to_string()) {
                report.tags_created += 1;
            }
            let tag_id = db.get_or_create_tag(&tag.name, tag.category)?;
            db.add_tag_to_match(match_id, tag_id)?;
        }

        let current_logs = db.logs_for_match(match_id)?;
        for l in &m.logs {
            let duplicate = current_logs
                .iter()
                .any(|c| c.interaction_type == l.interaction_type && c.date_logged == l.date_logged);
            if duplicate {
                report.logs_skipped += 1;
                continue;
            }
            db.insert_log(&InteractionLog {
                id: 0,
                match_id,
                interaction_type: l.interaction_type,
                date_logged: l.date_logged,
                rating: l.rating,
                script_score: l.script_score,
                cinematography_score: l.cinematography_score,
                direction_score: l.direction_score,
                liked: l.liked,
                review_text: l.review_text.clone(),
                contains_spoilers: l.contains_spoilers,
            })?;
            report.logs_created += 1;
        }
    }

    // Contenido actual de cada lista, para no duplicar una idéntica al reimportar.
    let mut existing_lists: Vec<(String, Vec<i64>)> = Vec::new();
    for l in db.list_lists()? {
        let ids = db.list_entries(l.id)?.iter().map(|e| e.item.match_id).collect();
        existing_lists.push((l.title, ids));
    }
    let mut titles: HashSet<String> = existing_lists.iter().map(|(t, _)| t.clone()).collect();

    for list in &bundle.lists {
        let mut items: Vec<&ExportListItem> = list.items.iter().collect();
        items.sort_by_key(|i| i.rank_order);
        let ids: Vec<i64> = items.iter().map(|i| key_to_id[i.match_key.as_str()]).collect();
        if existing_lists.iter().any(|(t, existing)| t == list.title.trim() && *existing == ids) {
            report.lists_existing += 1;
            continue;
        }

        let mut title = list.title.trim().to_string();
        let mut n = 1;
        while titles.contains(&title) {
            title = if n == 1 {
                format!("{} (importada)", list.title.trim())
            } else {
                format!("{} (importada {n})", list.title.trim())
            };
            n += 1;
        }
        titles.insert(title.clone());

        let list_id = db.create_list(&CustomList {
            id: 0,
            title,
            description: list.description.clone(),
            is_ranked: list.is_ranked,
        })?;
        for (item, match_id) in items.iter().zip(&ids) {
            db.add_list_item(list_id, *match_id, item.note.as_deref())?;
        }
        report.lists_created += 1;
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Búsqueda por texto (para la línea de comandos)
// ---------------------------------------------------------------------------

/// Encuentra un match por id o por parte del alias (debe ser único).
pub fn find_match(db: &Database, query: &str) -> Result<i64> {
    let matches = db.list_matches()?;
    if let Ok(id) = query.trim().parse::<i64>()
        && matches.iter().any(|m| m.id == id)
    {
        return Ok(id);
    }
    let q = query.trim().to_lowercase();
    let found: Vec<&MatchProfile> = matches.iter().filter(|m| m.alias.to_lowercase().contains(&q)).collect();
    match found.as_slice() {
        [one] => Ok(one.id),
        [] => bail!("ningún match coincide con «{query}»"),
        many => bail!(
            "«{query}» coincide con varios matches: {}",
            many.iter().map(|m| format!("[{}] {}", m.id, m.alias)).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Encuentra una lista por id o por parte del título (debe ser única).
pub fn find_list(db: &Database, query: &str) -> Result<i64> {
    let lists = db.list_lists()?;
    if let Ok(id) = query.trim().parse::<i64>()
        && lists.iter().any(|l| l.id == id)
    {
        return Ok(id);
    }
    let q = query.trim().to_lowercase();
    let found: Vec<&CustomList> = lists.iter().filter(|l| l.title.to_lowercase().contains(&q)).collect();
    match found.as_slice() {
        [one] => Ok(one.id),
        [] => bail!("ninguna lista coincide con «{query}»"),
        many => bail!(
            "«{query}» coincide con varias listas: {}",
            many.iter().map(|l| format!("[{}] {}", l.id, l.title)).collect::<Vec<_>>().join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> Database {
        let db = Database::open_in_memory().unwrap();
        db.seed_if_empty().unwrap();
        db
    }

    fn roundtrip(bundle: &ExportBundle) -> ExportBundle {
        from_json(&to_json(bundle).unwrap()).unwrap()
    }

    #[test]
    fn full_export_roundtrips_into_empty_db() {
        let src = seeded();
        let bundle = roundtrip(&export(&src, ExportTarget::All, ExportOptions::default()).unwrap());
        assert_eq!(bundle.kind, ExportKind::Completo);
        assert_eq!(bundle.matches.len(), 4);
        assert_eq!(bundle.log_count(), 7);

        let dst = Database::open_in_memory().unwrap();
        let report = import(&dst, &bundle, &ImportOptions::default()).unwrap();
        assert_eq!(report.matches_created, 4);
        assert_eq!(report.logs_created, 7);
        assert_eq!(report.tags_created, src.list_tags().unwrap().len());
        assert_eq!(report.lists_created, 1);

        let (a, b) = (src.stats().unwrap(), dst.stats().unwrap());
        assert_eq!(a.rating_distribution, b.rating_distribution);
        assert_eq!(a.funnel, b.funnel);
        assert_eq!(a.top_red_flags.len(), b.top_red_flags.len());
        let list = dst.list_lists().unwrap().remove(0);
        let entries = dst.list_entries(list.id).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].profile.alias, "Crypto Bro de Polanco");
        assert_eq!(dst.favorites().unwrap().len(), 2);
    }

    #[test]
    fn reimport_does_not_duplicate() {
        let db = seeded();
        let bundle = export(&db, ExportTarget::All, ExportOptions::default()).unwrap();
        let report = import(&db, &bundle, &ImportOptions::default()).unwrap();
        assert_eq!(report.matches_created, 0);
        assert_eq!(report.matches_existing, 4);
        assert_eq!(report.logs_skipped, 7);
        assert_eq!(db.list_matches().unwrap().len(), 4);
        assert_eq!(db.diary(None).unwrap().len(), 7);
        assert_eq!(report.lists_created, 0);
        assert_eq!(report.lists_existing, 1);
        assert_eq!(db.list_lists().unwrap().len(), 1);
        assert!(report.summary().starts_with("Nada nuevo"));
    }

    #[test]
    fn friend_prefix_creates_copies_and_respects_top4() {
        let db = seeded();
        let bundle = export(&db, ExportTarget::All, ExportOptions::default()).unwrap();
        let ana = import(&db, &bundle, &ImportOptions { alias_prefix: Some("Ana".into()) }).unwrap();
        assert_eq!(ana.matches_created, 4);
        assert_eq!(ana.favorites_dropped, 0); // 2 propios + 2 de Ana = 4
        assert!(db.list_matches().unwrap().iter().any(|m| m.alias == "Ana · Crypto Bro de Polanco"));
        // Mismo título pero con los matches de Ana: es otra lista.
        let titles: Vec<String> = db.list_lists().unwrap().into_iter().map(|l| l.title).collect();
        assert!(titles[1].ends_with("(importada)"), "{titles:?}");

        let beto = import(&db, &bundle, &ImportOptions { alias_prefix: Some("Beto".into()) }).unwrap();
        assert_eq!(beto.favorites_dropped, 2);
        assert_eq!(db.favorites().unwrap().len(), 4);
    }

    #[test]
    fn list_export_only_contains_its_matches() {
        let db = seeded();
        let list_id = find_list(&db, "chat").unwrap();
        let bundle = export(&db, ExportTarget::List(list_id), ExportOptions::default()).unwrap();
        assert_eq!(bundle.kind, ExportKind::Lista);
        assert_eq!(bundle.matches.len(), 3);
        assert_eq!(bundle.lists[0].items.len(), 3);
        assert!(!bundle.matches.iter().any(|m| m.alias.contains("perro")));

        let one = export(&db, ExportTarget::Match(find_match(&db, "perro").unwrap()), ExportOptions::default()).unwrap();
        assert_eq!(one.matches.len(), 1);
        assert_eq!(one.matches[0].logs.len(), 3);
        assert!(one.lists.is_empty());
    }

    #[test]
    fn photos_travel_inside_the_json() {
        let src = Database::open_in_memory().unwrap();
        let bytes = b"\x89PNG bytes de prueba".to_vec();
        let mut m = MatchProfile::new("Con foto", Platform::Hinge, MatchStatus::Activa);
        m.avatar_path = Some(src.import_photo_bytes(&bytes, "png").unwrap());
        src.insert_match(&m).unwrap();

        let with = roundtrip(&export(&src, ExportTarget::All, ExportOptions::default()).unwrap());
        assert_eq!(with.photo_count(), 1);
        let dst = Database::open_in_memory().unwrap();
        let report = import(&dst, &with, &ImportOptions::default()).unwrap();
        assert_eq!(report.photos_imported, 1);
        let path = dst.list_matches().unwrap()[0].avatar_path.clone().unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);

        let without = export(&src, ExportTarget::All, ExportOptions { include_photos: false }).unwrap();
        assert_eq!(without.photo_count(), 0);

        for p in [m.avatar_path.unwrap(), path] {
            src.remove_photo(&p);
        }
    }

    #[test]
    fn rejects_foreign_future_and_broken_files() {
        assert!(from_json("no es json").is_err());
        assert!(from_json(r#"{"format":"letterboxd"}"#).unwrap_err().to_string().contains("Tinderboxd"));
        let future = r#"{"format":"tinderboxd","version":99,"kind":"completo","exported_at":"","app_version":"","matches":[]}"#;
        assert!(from_json(future).unwrap_err().to_string().contains("versión 99"));

        let db = seeded();
        let mut bundle = export(&db, ExportTarget::All, ExportOptions::default()).unwrap();
        bundle.lists[0].items[0].match_key = "m999".into();
        assert!(from_json(&to_json(&bundle).unwrap()).is_err());

        // Un fallo a mitad de importación no deja nada a medias.
        let dst = Database::open_in_memory().unwrap();
        let mut bad = export(&db, ExportTarget::All, ExportOptions::default()).unwrap();
        bad.matches[3].logs[0].script_score = Some(9);
        assert!(import(&dst, &bad, &ImportOptions::default()).is_err());
        assert!(dst.list_matches().unwrap().is_empty());
    }

    #[test]
    fn find_and_slug_helpers() {
        let db = seeded();
        assert!(find_match(&db, "la").is_err()); // ambiguo
        assert!(find_match(&db, "zzz").is_err());
        assert_eq!(find_match(&db, "1").unwrap(), 1);
        assert_eq!(slug("Citas que debieron quedarse en el chat"), "citas-que-debieron-quedarse-en-el-chat");
        assert_eq!(slug("La astróloga de la Roma Norte!!"), "la-astrologa-de-la-roma-norte");
        assert_eq!(slug("¿?"), "sin-nombre");
    }
}
