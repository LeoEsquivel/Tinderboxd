# AGENT.md - Especificación del Proyecto "Tinderboxd"

## 1. Visión General
**Tinderboxd** es una aplicación local nativa en Rust inspirada en *Letterboxd*, diseñada para registrar, calificar y analizar "matches" y citas de apps de ligue (Tinder, Bumble, Hinge, etc.) a modo de crítica cinematográfica.
El proyecto cuenta con una arquitectura de **Cerebro Único** conectada a **SQLite**, capaz de ejecutarse tanto en **TUI (Terminal con `ratatui`)** como en **GUI (Ventana nativa con `egui`/`eframe`)** compartiendo el mismo estado y base de datos.

---

## 2. Stack Tecnológico y Crates
- **Lenguaje:** Rust (Edición 2021 / 2024)
- **Base de Datos:** SQLite local mediante `rusqlite` (con feature `bundled` para no depender del sistema).
- **CLI & Argumentos:** `clap` (con `derive`) para elegir entre `--tui`, `--gui`, o exportar/importar JSON.
- **TUI (Terminal):** `ratatui` + `crossterm` (+ `ratatui-image` opcional para avatares en consola).
- **GUI (Escritorio):** `eframe` / `egui` + `egui_extras` (para carga de imágenes/pósters).
- **Serialización:** `serde` + `serde_json` (para compartir perfiles/listas con amigos).
- **Fechas y Manejo de Errores:** `chrono` (ISO8601) y `anyhow` / `thiserror`.

---

## 3. Arquitectura del Proyecto (Separación Estricta)
El código DEBE mantener la lógica de negocio 100% desacoplada de las interfaces gráficas:

    src/
    ├── main.rs            # Entrypoint: parsea CLI (clap) y lanza TUI o GUI
    ├── core/              # CEREBRO (Cero dependencias de egui o ratatui)
    │   ├── mod.rs
    │   ├── models.rs      # Structs: MatchProfile, InteractionLog, Tag, CustomList, Stats
    │   ├── db.rs          # Inicialización SQLite, migraciones y queries CRUD
    │   ├── state.rs       # AppState compartido (filtros, vista activa, borradores)
    │   └── export.rs      # Importar/Exportar JSON para modo "Co-op" con amigos
    ├── tui/               # FRENTE TERMINAL (Ratatui)
    │   ├── mod.rs
    │   ├── app.rs         # Event loop de terminal y atajos tipo Vim (h,j,k,l)
    │   └── views.rs       # Renderizado de tablas, barras Unicode y fichas
    └── gui/               # FRENTE ESCRITORIO (egui)
        ├── mod.rs
        ├── app.rs         # Implementación de eframe::App
        └── views.rs       # Grid de pósters, modales, tema colores Letterboxd

**Regla de Oro para el Agente:** Ningún archivo dentro de `src/core/` puede importar tipos de `egui` ni de `ratatui`. Ambos frontends consumen `AppState` y llaman a los métodos de `Database`.

---

## 4. Esquema de Base de Datos (SQLite)

1. **`matches` (Equivalente a Películas):**
   - `id` (INTEGER PK AUTOINCREMENT)
   - `alias` (TEXT NOT NULL) - Ej. "La chica del perro gigante"
   - `age` (INTEGER)
   - `platform` (TEXT NOT NULL) - 'Tinder', 'Bumble', 'Hinge', 'IG', 'IRL'
   - `bio_highlight` (TEXT) - Frase célebre de su bio
   - `avatar_path` (TEXT) - Ruta local a la imagen/captura
   - `match_date` (TEXT) - Formato YYYY-MM-DD
   - `status` (TEXT NOT NULL) - 'Watchlist', 'Activa', 'Ghosting Mutuo', 'Ghosteado', 'Unmatched', 'Solo Amigos'
   - `is_favorite` (BOOLEAN DEFAULT 0) - Para el "Top 4" del perfil

2. **`logs` (Equivalente al Diary / Reviews):**
   - `id` (INTEGER PK AUTOINCREMENT)
   - `match_id` (INTEGER FK -> matches.id ON DELETE CASCADE)
   - `interaction_type` (TEXT NOT NULL) - 'Chat', 'Llamada', 'Primera Cita', 'Cita Casual'
   - `date_logged` (TEXT NOT NULL)
   - `rating` (INTEGER NOT NULL) - Escala del 1 al 10 (equivale a 0.5 a 5.0 estrellas)
   - `script_score` (INTEGER) - Sub-métrica: Guion/Conversación (1-5)
   - `cinematography_score` (INTEGER) - Sub-métrica: Fotografía/Fidelidad a sus fotos (1-5)
   - `direction_score` (INTEGER) - Sub-métrica: Dirección/Química e iniciativa (1-5)
   - `liked` (BOOLEAN NOT NULL DEFAULT 0) - Corazón naranja de Letterboxd
   - `review_text` (TEXT)
   - `contains_spoilers` (BOOLEAN NOT NULL DEFAULT 0)

3. **`tags` y `match_tags` (Flags y Etiquetas):**
   - `tags`: `id` (PK), `name` (TEXT UNIQUE), `category` ('Red Flag', 'Green Flag', 'Neutral')
   - `match_tags`: `match_id` (FK), `tag_id` (FK), PRIMARY KEY (`match_id`, `tag_id`)

4. **`lists` y `list_items` (Listas Temáticas):**
   - `lists`: `id` (PK), `title` (TEXT), `description` (TEXT), `is_ranked` (BOOLEAN)
   - `list_items`: `list_id` (FK), `match_id` (FK), `rank_order` (INTEGER), `item_note` (TEXT)

---

## 5. Vistas y Funcionalidades Clave
- **Dashboard / Perfil:** Muestra los "4 Favoritos" en la parte superior y los logs recientes.
- **Cartelera (Matches Grid):** Vista de pósters (con foto en `egui` o tarjeta compacta en `ratatui`), estrellas y corazón.
- **Watchlist (Sala de Espera):** Matches con estado `'Watchlist'` pendientes de primera cita o primer mensaje.
- **Diary (Historial cronológico):** Lista de todas las interacciones calificadas ordenadas por fecha.
- **Stats (Estadísticas Letterboxd):**
  - Distribución de calificaciones (0.5 a 5.0 estrellas).
  - Embudo de supervivencia (% que pasa de Chat a Primera Cita vs Ghosting).
  - Top Red Flags y Green Flags más frecuentes.
- **Estética:** Paleta inspirada en Letterboxd (Fondo oscuro `#14181c`, texto `#9ab`, verde `#00e054` para ratings/éxito, naranja `#ff8000` para likes, azul `#40bcf4`).