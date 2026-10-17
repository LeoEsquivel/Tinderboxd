//! Modelos de dominio de Tinderboxd (Punto 4 de AGENT.md).
//!
//! Los enums se guardan en SQLite como el texto exacto que define el esquema
//! (`'Primera Cita'`, `'Ghosting Mutuo'`, ...). La conversión vive aquí
//! (`as_str` / `FromStr`) y `db.rs` la reutiliza para `ToSql` / `FromSql`.

use std::fmt;
use std::str::FromStr;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Error de parseo compartido por todos los enums de texto
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseEnumError {
    pub kind: &'static str,
    pub value: String,
}

impl fmt::Display for ParseEnumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "valor de {} desconocido: '{}'", self.kind, self.value)
    }
}

impl std::error::Error for ParseEnumError {}

/// Genera `as_str`, `ALL`, `Display` y `FromStr` para un enum cuyo
/// representante en SQLite es un texto fijo.
macro_rules! text_enum {
    ($name:ident, $kind:literal, { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(&self) -> &'static str {
                match self {
                    $($name::$variant => $text),+
                }
            }

            /// Siguiente valor (cíclico); útil para selectores en los formularios.
            pub fn next(&self) -> Self {
                let i = Self::ALL.iter().position(|v| v == self).unwrap_or(0);
                Self::ALL[(i + 1) % Self::ALL.len()]
            }

            /// Valor anterior (cíclico).
            pub fn prev(&self) -> Self {
                let i = Self::ALL.iter().position(|v| v == self).unwrap_or(0);
                Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ParseEnumError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($text => Ok($name::$variant),)+
                    other => Err(ParseEnumError { kind: $kind, value: other.to_string() }),
                }
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Enums del esquema
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Platform {
    Tinder,
    Bumble,
    Hinge,
    #[serde(rename = "IG")]
    Instagram,
    #[serde(rename = "IRL")]
    Irl,
}

text_enum!(Platform, "plataforma", {
    Tinder => "Tinder",
    Bumble => "Bumble",
    Hinge => "Hinge",
    Instagram => "IG",
    Irl => "IRL",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MatchStatus {
    Watchlist,
    Activa,
    #[serde(rename = "Ghosting Mutuo")]
    GhostingMutuo,
    Ghosteado,
    Unmatched,
    #[serde(rename = "Solo Amigos")]
    SoloAmigos,
}

text_enum!(MatchStatus, "estado", {
    Watchlist => "Watchlist",
    Activa => "Activa",
    GhostingMutuo => "Ghosting Mutuo",
    Ghosteado => "Ghosteado",
    Unmatched => "Unmatched",
    SoloAmigos => "Solo Amigos",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InteractionType {
    Chat,
    Llamada,
    #[serde(rename = "Primera Cita")]
    PrimeraCita,
    #[serde(rename = "Cita Casual")]
    CitaCasual,
}

text_enum!(InteractionType, "tipo de interacción", {
    Chat => "Chat",
    Llamada => "Llamada",
    PrimeraCita => "Primera Cita",
    CitaCasual => "Cita Casual",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TagCategory {
    #[serde(rename = "Red Flag")]
    RedFlag,
    #[serde(rename = "Green Flag")]
    GreenFlag,
    Neutral,
}

text_enum!(TagCategory, "categoría de tag", {
    RedFlag => "Red Flag",
    GreenFlag => "Green Flag",
    Neutral => "Neutral",
});

impl TagCategory {
    pub fn emoji(&self) -> &'static str {
        match self {
            TagCategory::RedFlag => "🚩",
            TagCategory::GreenFlag => "🟢",
            TagCategory::Neutral => "⚪",
        }
    }
}

// ---------------------------------------------------------------------------
// Rating (1..=10 → 0.5..=5.0 estrellas) y sub-métricas (1..=5)
// ---------------------------------------------------------------------------

/// Calificación estilo Letterboxd: 1..=10 medias estrellas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Rating(u8);

impl Rating {
    pub const MIN: u8 = 1;
    pub const MAX: u8 = 10;

    pub fn new(value: u8) -> Result<Self, String> {
        if (Self::MIN..=Self::MAX).contains(&value) {
            Ok(Rating(value))
        } else {
            Err(format!("el rating debe estar entre 1 y 10 (recibido {value})"))
        }
    }

    /// Construye un rating forzando el valor al rango válido.
    pub fn clamped(value: i32) -> Self {
        Rating(value.clamp(Self::MIN as i32, Self::MAX as i32) as u8)
    }

    pub fn value(&self) -> u8 {
        self.0
    }

    /// Número de estrellas (0.5..=5.0).
    pub fn as_stars(&self) -> f32 {
        self.0 as f32 / 2.0
    }

    /// Estrellas tipo Letterboxd: `7` → `"★★★½"`.
    pub fn stars(&self) -> String {
        format_stars(self.0)
    }

    /// Estrellas rellenadas con `☆` hasta 5, para alinear columnas: `7` → `"★★★½☆"`.
    pub fn stars_padded(&self) -> String {
        let full = (self.0 / 2) as usize;
        let half = self.0 % 2 == 1;
        let empty = 5 - full - half as usize;
        format!("{}{}{}", "★".repeat(full), if half { "½" } else { "" }, "☆".repeat(empty))
    }
}

impl TryFrom<u8> for Rating {
    type Error = String;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Rating::new(value)
    }
}

impl From<Rating> for u8 {
    fn from(r: Rating) -> u8 {
        r.0
    }
}

impl fmt::Display for Rating {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.stars())
    }
}

/// Formatea medias estrellas (1..=10) como `★★★½`. Valores fuera de rango se recortan.
pub fn format_stars(half_stars: u8) -> String {
    let v = half_stars.min(Rating::MAX);
    let mut s = "★".repeat((v / 2) as usize);
    if v % 2 == 1 {
        s.push('½');
    }
    s
}

/// Formatea un promedio (en escala 1..=10) redondeado a la media estrella más cercana.
pub fn format_avg_stars(avg_half_stars: f64) -> String {
    format_stars(avg_half_stars.round().clamp(1.0, 10.0) as u8)
}

/// Sub-métrica 1..=5 como puntos: `3` → `"●●●○○"`.
pub fn format_score_dots(score: u8) -> String {
    let s = score.min(5) as usize;
    format!("{}{}", "●".repeat(s), "○".repeat(5 - s))
}

// ---------------------------------------------------------------------------
// Entidades (tablas)
// ---------------------------------------------------------------------------

/// Tabla `matches`: cada persona con la que hiciste match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchProfile {
    pub id: i64,
    pub alias: String,
    pub age: Option<u32>,
    pub platform: Platform,
    pub bio_highlight: Option<String>,
    pub avatar_path: Option<String>,
    pub match_date: Option<NaiveDate>,
    pub status: MatchStatus,
    pub is_favorite: bool,
}

impl MatchProfile {
    /// Perfil nuevo (aún sin persistir, `id = 0`).
    pub fn new(alias: impl Into<String>, platform: Platform, status: MatchStatus) -> Self {
        MatchProfile {
            id: 0,
            alias: alias.into(),
            age: None,
            platform,
            bio_highlight: None,
            avatar_path: None,
            match_date: None,
            status,
            is_favorite: false,
        }
    }

    /// Línea corta de presentación: `La chica del perro gigante (27) · Tinder`.
    pub fn headline(&self) -> String {
        match self.age {
            Some(age) => format!("{} ({}) · {}", self.alias, age, self.platform),
            None => format!("{} · {}", self.alias, self.platform),
        }
    }
}

/// Tabla `logs`: una entrada del Diary / reseña.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionLog {
    pub id: i64,
    pub match_id: i64,
    pub interaction_type: InteractionType,
    pub date_logged: NaiveDate,
    pub rating: Rating,
    /// Conversación (1..=5).
    pub script_score: Option<u8>,
    /// Fotos vs. realidad (1..=5).
    pub cinematography_score: Option<u8>,
    /// Química e iniciativa (1..=5).
    pub direction_score: Option<u8>,
    pub liked: bool,
    pub review_text: Option<String>,
    pub contains_spoilers: bool,
}

impl InteractionLog {
    pub fn heart(&self) -> &'static str {
        if self.liked { "♥" } else { "" }
    }

    /// Resumen de sub-métricas: `Conversación ●●●●○ · Fotos vs. realidad ●●●○○ · Química ●●○○○`.
    pub fn sub_scores_line(&self) -> String {
        [
            ("Conversación", self.script_score),
            ("Fotos vs. realidad", self.cinematography_score),
            ("Química", self.direction_score),
        ]
        .iter()
        .filter_map(|(label, s)| s.map(|v| format!("{label} {}", format_score_dots(v))))
        .collect::<Vec<_>>()
        .join(" · ")
    }
}

/// Tabla `tags`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub category: TagCategory,
}

impl Tag {
    pub fn label(&self) -> String {
        format!("{} {}", self.category.emoji(), self.name)
    }
}

/// Tabla `lists`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomList {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub is_ranked: bool,
}

/// Tabla `list_items`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListItem {
    pub list_id: i64,
    pub match_id: i64,
    pub rank_order: i64,
    pub item_note: Option<String>,
}

// ---------------------------------------------------------------------------
// Vistas agregadas (lo que consumen los frontends)
// ---------------------------------------------------------------------------

/// Tarjeta de un match (vista Cartelera/Matches): perfil + agregados de sus logs + tags.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchCard {
    pub profile: MatchProfile,
    pub log_count: u32,
    /// Promedio de ratings en escala 1..=10.
    pub avg_rating: Option<f64>,
    /// Rating del log más reciente.
    pub last_rating: Option<Rating>,
    /// `true` si algún log tiene corazón.
    pub liked: bool,
    pub last_logged: Option<NaiveDate>,
    pub tags: Vec<Tag>,
}

impl MatchCard {
    pub fn avg_stars(&self) -> String {
        self.avg_rating.map(format_avg_stars).unwrap_or_default()
    }
}

/// Entrada del Diary: log + datos mínimos del match para mostrarlo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiaryEntry {
    pub log: InteractionLog,
    pub match_alias: String,
    pub platform: Platform,
}

/// Item de una lista con su match resuelto.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListEntry {
    pub item: ListItem,
    pub profile: MatchProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagCount {
    pub tag: Tag,
    pub count: u32,
}

/// Paso del embudo de supervivencia.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunnelStep {
    pub label: String,
    pub count: u32,
    /// Porcentaje respecto al total de matches (0..=100).
    pub pct_of_total: f64,
}

/// Estadísticas de la vista Stats (Punto 5).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StatsSummary {
    pub total_matches: u32,
    pub total_logs: u32,
    pub liked_logs: u32,
    pub average_rating: Option<f64>,
    /// `rating_distribution[i]` = número de logs con rating `i + 1` (0.5★ ... 5★).
    pub rating_distribution: [u32; 10],
    /// Conteo por estado (todos los estados, incluidos los de 0).
    pub status_counts: Vec<(MatchStatus, u32)>,
    /// Matches que alcanzaron al menos cada etapa: Match → Chat → Llamada → Primera Cita → Cita Casual.
    pub funnel: Vec<FunnelStep>,
    /// % de matches con Chat que llegaron a Primera Cita.
    pub chat_to_date_pct: Option<f64>,
    /// % de matches con Chat que terminaron en Ghosteado / Ghosting Mutuo.
    pub ghosting_pct: Option<f64>,
    pub top_red_flags: Vec<TagCount>,
    pub top_green_flags: Vec<TagCount>,
}

impl StatsSummary {
    pub fn average_stars(&self) -> String {
        self.average_rating.map(format_avg_stars).unwrap_or_default()
    }

    /// Máximo de la distribución (para escalar barras).
    pub fn max_bucket(&self) -> u32 {
        self.rating_distribution.iter().copied().max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_letterboxd() {
        assert_eq!(format_stars(1), "½");
        assert_eq!(format_stars(2), "★");
        assert_eq!(format_stars(7), "★★★½");
        assert_eq!(format_stars(10), "★★★★★");
        assert_eq!(Rating::new(7).unwrap().stars_padded(), "★★★½☆");
        assert_eq!(Rating::new(1).unwrap().stars_padded(), "½☆☆☆☆");
        assert!(Rating::new(0).is_err());
        assert!(Rating::new(11).is_err());
        assert_eq!(format_avg_stars(6.6), "★★★½");
    }

    #[test]
    fn enums_roundtrip() {
        for s in MatchStatus::ALL {
            assert_eq!(s.as_str().parse::<MatchStatus>().unwrap(), *s);
        }
        assert_eq!("Primera Cita".parse::<InteractionType>().unwrap(), InteractionType::PrimeraCita);
        assert!("Netflix".parse::<Platform>().is_err());
        assert_eq!(Platform::Irl.next(), Platform::Tinder);
        assert_eq!(
            serde_json::to_string(&MatchStatus::GhostingMutuo).unwrap(),
            "\"Ghosting Mutuo\""
        );
    }
}
