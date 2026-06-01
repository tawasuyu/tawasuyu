//! Registro de visores — el "open-with universal", dirigido por datos y
//! abierto al descubrimiento.
//!
//! Toma el [`Discernment`] que `shuma-discern` produce sobre una muestra
//! del archivo (detección por **contenido**, no por extensión) y lo
//! despacha al [`ViewerKind`] que sabe pintar esa naturaleza de dato.
//!
//! ## La tabla es una Card, no un `match`
//!
//! Cada visor se describe como un [`ViewerCard`]: qué `lens` acepta, qué
//! `mime` (prefijo o exacto) cubre, y con qué [`Priority`] compite. La
//! decisión NO vive en ramas de control sino en [`registry`], una tabla de
//! datos. El `lens` es el mismo `presentation_hint` que `card_core::DataFacet`
//! comparte con chasqui (`dominant_lens`) y shuma-discern, y `Priority` es el
//! mismo tiebreaker que usa `chasqui-broker` para rankear productores. Esto
//! es la "Capa 2" de Brahman a nivel de UI (ver `/BRAHMAN.md`, Fase 2a).
//!
//! ## La costura hacia el AppBus — ya no es estática
//!
//! [`registry`] ya **no** devuelve una tabla cerrada. Se ensambla en runtime
//! (una sola vez, cacheada) como `built-ins + Cards descubiertas`:
//!
//! - **Built-ins** ([`builtin_registry`]): el piso — los visores que el shell
//!   linkea en proceso. Cada uno con su `(lens, mime, priority)`.
//! - **Descubiertas** ([`discover_viewer_cards`]): `card_core::Card`s leídas
//!   de `$NAHUAL_VIEWERS_DIR` (por defecto `~/.config/nahual/viewers.d`), el
//!   mismo formato JSON/TOML que el broker anuncia y que `card-discovery`
//!   escanea. Una Card que extiende el ruteo de un visor **ya montado**
//!   (p. ej. "ruteá `image/heic` al visor de imágenes con prioridad alta")
//!   funciona end-to-end: no necesita IPC porque reusa el constructor
//!   in-process. Las Cards cuyo `viewer_kind` el shell no sabe montar se
//!   ignoran (serían visores fuera de proceso, pendientes del render-IPC).
//!
//! Cuando exista el AppBus vivo, [`discover_viewer_cards`] cambia su origen
//! de "directorio en disco" a "broker / `card-discovery`" sin tocar el
//! algoritmo de ranking: el contrato (una `Card` con `lens`/`mime`/`priority`)
//! ya es el mismo.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use card_core::Priority;
use serde_json::Value as JsonValue;
use shuma_discern::Discernment;

/// Qué visor del shell pinta el panel derecho.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerKind {
    /// Visor de imágenes (`nahual-image-viewer-llimphi`).
    Image,
    /// Reproductor de video (`nahual-video-viewer-llimphi`); abre
    /// WebM/MKV (AV1) e IVF con el decoder nativo puro-Rust.
    Video,
    /// Reproductor de audio (`nahual-audio-viewer-llimphi`); WAV/MP3/
    /// FLAC/Opus/Vorbis por cpal, con espectro en vivo.
    Audio,
    /// Visor estructurado de Cards (`nahual-card-viewer-llimphi`); pinta
    /// los campos de una `shared/card` en vez del JSON crudo.
    Card,
    /// Visor de árbol JSON/TOML (`nahual-tree-viewer-llimphi`); indenta
    /// la estructura, legible aun para JSON minificado.
    Tree,
    /// Volcado hex/ASCII (`nahual-hex-viewer-llimphi`) para binarios que
    /// shuma reconoce pero no tienen visor propio (ELF/wasm/gzip/zip).
    Hex,
    /// Tabla CSV/TSV (`nahual-table-viewer-llimphi`); columnas alineadas.
    Table,
    /// Markdown renderizado (`nahual-markdown-viewer-llimphi`); encabezados,
    /// listas, código y citas con estilo en vez de la sintaxis cruda.
    Markdown,
    /// Listado de un archivo comprimido (`nahual-archive-viewer-llimphi`);
    /// muestra las entradas (nombre/tamaño/ratio) en vez del volcado hex.
    /// Cubre ZIP (y .jar/.apk/.epub/OOXML), tar y tar.gz.
    Archive,
    /// Visor de fuentes (`nahual-font-viewer-llimphi`); metadatos + una
    /// muestra dibujada con los contornos de la propia fuente (TTF/OTF).
    Font,
    /// Visor de texto (`nahual-text-viewer-llimphi`); degrada a "binario"
    /// si el contenido no es UTF-8. Es el fallback universal.
    Text,
    /// Página web (HTML): el "open-with" la entrega a **puriy**, el
    /// navegador de la suite. El panel muestra el fuente; abrir el archivo
    /// (Enter) lanza puriy sobre `file://<path>`. Es la costura nahual↔puriy
    /// — el escritorio sabe que el HTML es asunto del navegador.
    Web,
}

impl ViewerKind {
    /// Etiqueta estable del visor — el `nahual.viewer_kind` que una Card
    /// descubierta declara para mapearse a un constructor in-process. NO
    /// cambiar sin migrar las Cards en disco.
    pub fn as_tag(self) -> &'static str {
        match self {
            ViewerKind::Image => "image",
            ViewerKind::Video => "video",
            ViewerKind::Audio => "audio",
            ViewerKind::Card => "card",
            ViewerKind::Tree => "tree",
            ViewerKind::Hex => "hex",
            ViewerKind::Table => "table",
            ViewerKind::Markdown => "markdown",
            ViewerKind::Archive => "archive",
            ViewerKind::Font => "font",
            ViewerKind::Text => "text",
            ViewerKind::Web => "web",
        }
    }

    /// Inverso de [`as_tag`](Self::as_tag). `None` si la etiqueta no
    /// corresponde a ningún visor que el shell sepa montar.
    pub fn from_tag(tag: &str) -> Option<Self> {
        Some(match tag {
            "image" => ViewerKind::Image,
            "video" => ViewerKind::Video,
            "audio" => ViewerKind::Audio,
            "card" => ViewerKind::Card,
            "tree" => ViewerKind::Tree,
            "hex" => ViewerKind::Hex,
            "table" => ViewerKind::Table,
            "markdown" => ViewerKind::Markdown,
            "archive" => ViewerKind::Archive,
            "font" => ViewerKind::Font,
            "text" => ViewerKind::Text,
            "web" => ViewerKind::Web,
            _ => return None,
        })
    }
}

/// Descripción declarativa de un visor: qué naturaleza de dato cubre y con
/// qué prioridad compite. Es el equivalente UI de una `card_core::Card` de
/// tipo `Data` cuyo `presentation_hint` (lens) y `mime` declaran qué sabe
/// pintar. Los built-ins nacen de [`builtin_registry`]; los descubiertos, de
/// una `Card` real vía [`viewer_card_from_card`].
#[derive(Debug, Clone)]
pub struct ViewerCard {
    /// Visor concreto que se monta si esta fila gana.
    pub kind: ViewerKind,
    /// Lenses (`presentation_hint`) que el visor reclama. Match exacto.
    pub lenses: Vec<String>,
    /// Prefijos de `mime` que cubre (p. ej. `"image/"`).
    pub mime_prefixes: Vec<String>,
    /// `mime` exactos que cubre (p. ej. `"application/zip"`).
    pub mime_exact: Vec<String>,
    /// Prioridad de desempate cuando más de un visor matchea con la misma
    /// especificidad. Mismo orden que usa el broker (`Low<Normal<High<Critical`).
    pub priority: Priority,
}

impl ViewerCard {
    /// Constructor desde slices estáticos — azúcar para [`builtin_registry`].
    fn builtin(
        kind: ViewerKind,
        lenses: &[&str],
        mime_prefixes: &[&str],
        mime_exact: &[&str],
        priority: Priority,
    ) -> Self {
        let own = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        ViewerCard {
            kind,
            lenses: own(lenses),
            mime_prefixes: own(mime_prefixes),
            mime_exact: own(mime_exact),
            priority,
        }
    }
}

/// Los visores que el shell linkea en proceso: el piso del registro.
///
/// El orden importa sólo como desempate final (especificidad y `Priority`
/// mandan primero). `Text` NO está acá: es el fallback explícito de [`pick`]
/// cuando ninguna fila matchea.
pub fn builtin_registry() -> Vec<ViewerCard> {
    use Priority::Normal;
    vec![
        ViewerCard::builtin(ViewerKind::Image, &["gallery"], &["image/"], &[], Normal),
        ViewerCard::builtin(ViewerKind::Video, &["video"], &["video/"], &[], Normal),
        ViewerCard::builtin(ViewerKind::Audio, &["audio"], &["audio/"], &[], Normal),
        ViewerCard::builtin(ViewerKind::Card, &["card"], &[], &[], Normal),
        ViewerCard::builtin(ViewerKind::Tree, &["tree"], &[], &[], Normal),
        ViewerCard::builtin(ViewerKind::Table, &["table"], &[], &[], Normal),
        ViewerCard::builtin(ViewerKind::Markdown, &["markdown"], &[], &[], Normal),
        ViewerCard::builtin(ViewerKind::Font, &["font"], &[], &[], Normal),
        // HTML → puriy (el navegador de la suite). Cubre el lens `web`/`html`
        // que pueda emitir shuma-discern y los mime canónicos del HTML/XHTML.
        ViewerCard::builtin(
            ViewerKind::Web,
            &["web", "html"],
            &[],
            &["text/html", "application/xhtml+xml"],
            Normal,
        ),
        // Contenedores: un comprimido se lista (entradas) en vez de volcarse.
        // ZIP cubre .jar/.apk/.epub/OOXML; gzip se asume envolviendo un tar.
        ViewerCard::builtin(
            ViewerKind::Archive,
            &[],
            &[],
            &["application/zip", "application/x-tar", "application/gzip"],
            Normal,
        ),
        // Binarios que shuma detecta por magic-bytes sin lens y que ningún
        // visor rico cubre: un dump hex es mejor que "(binario — sin preview)".
        ViewerCard::builtin(
            ViewerKind::Hex,
            &[],
            &[],
            &["application/x-executable", "application/wasm"],
            Normal,
        ),
    ]
}

/// Tabla de ruteo efectiva: built-ins + Cards descubiertas, ensamblada una
/// sola vez y cacheada. Las descubiertas se concatenan después de las
/// built-ins, así que en empate de `(especificidad, Priority)` ganan ellas
/// (último máximo) — lo que permite a una Card en disco **sobrescribir** o
/// **extender** el ruteo de un visor montado.
pub fn registry() -> &'static [ViewerCard] {
    static REGISTRY: OnceLock<Vec<ViewerCard>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut rows = builtin_registry();
        rows.extend(discover_viewer_cards());
        rows
    })
}

/// Directorio del que se leen las Cards de visores. `$NAHUAL_VIEWERS_DIR`
/// manda; si no, `$XDG_CONFIG_HOME/nahual/viewers.d` o `~/.config/nahual/viewers.d`.
fn viewers_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("NAHUAL_VIEWERS_DIR") {
        return Some(PathBuf::from(dir));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("nahual").join("viewers.d"))
}

/// Lee las `Card`s de visores del directorio de descubrimiento y las mapea a
/// [`ViewerCard`]. Tolerante: un directorio inexistente o una Card inválida
/// se ignoran en silencio (el shell debe arrancar igual sin ninguna).
pub fn discover_viewer_cards() -> Vec<ViewerCard> {
    match viewers_dir() {
        Some(dir) => discover_in(&dir),
        None => Vec::new(),
    }
}

/// Núcleo de [`discover_viewer_cards`] sobre un directorio concreto —
/// separado para testearlo sin depender de variables de entorno globales.
fn discover_in(dir: &std::path::Path) -> Vec<ViewerCard> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("json") | Some("toml") => {}
            _ => continue,
        }
        // `Card::from_path` auto-detecta formato y valida; descartamos
        // silenciosamente lo que no parsee.
        let Ok(card) = card_core::Card::from_path(&path) else {
            continue;
        };
        if let Some(vc) = viewer_card_from_card(&card) {
            out.push(vc);
        }
    }
    out
}

/// Mapea una `card_core::Card` a un [`ViewerCard`] del shell.
///
/// La Card declara su visor en `extensions["nahual.viewer_kind"]` (la
/// etiqueta estable de [`ViewerKind::as_tag`]). Si falta o el shell no sabe
/// montar ese visor, devuelve `None` (sería un visor fuera de proceso, aún
/// sin render-IPC). Los `lens` salen de `data.presentation_hint` más
/// `extensions["nahual.lenses"]`; los `mime`, de `extensions["nahual.mime_prefixes"]`
/// y `["nahual.mime_exact"]`. La `priority` es la de la propia Card.
pub fn viewer_card_from_card(card: &card_core::Card) -> Option<ViewerCard> {
    let tag = card.extensions.get("nahual.viewer_kind")?.as_str()?;
    let kind = ViewerKind::from_tag(tag)?;

    let mut lenses = json_str_array(&card.extensions, "nahual.lenses");
    if let Some(data) = &card.data {
        if !data.presentation_hint.is_empty() && !lenses.contains(&data.presentation_hint) {
            lenses.push(data.presentation_hint.clone());
        }
    }

    Some(ViewerCard {
        kind,
        lenses,
        mime_prefixes: json_str_array(&card.extensions, "nahual.mime_prefixes"),
        mime_exact: json_str_array(&card.extensions, "nahual.mime_exact"),
        priority: card.priority,
    })
}

/// Lee un array JSON de strings de `extensions[key]`; `[]` si falta o no es
/// un array de strings.
fn json_str_array(extensions: &BTreeMap<String, JsonValue>, key: &str) -> Vec<String> {
    extensions
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// Especificidad de un match: a mayor número, más concreta la coincidencia.
/// El orden replica el de la versión hardcoded previa: `lens` manda sobre
/// `mime` exacto, y éste sobre el prefijo de `mime`.
const SCORE_LENS: u8 = 3;
const SCORE_MIME_EXACT: u8 = 2;
const SCORE_MIME_PREFIX: u8 = 1;
const SCORE_NONE: u8 = 0;

fn score(card: &ViewerCard, d: &Discernment) -> u8 {
    if let Some(lens) = d.lens.as_deref() {
        if card.lenses.iter().any(|l| l == lens) {
            return SCORE_LENS;
        }
    }
    if let Some(mime) = d.mime.as_deref() {
        if card.mime_exact.iter().any(|m| m == mime) {
            return SCORE_MIME_EXACT;
        }
        if card.mime_prefixes.iter().any(|p| mime.starts_with(p.as_str())) {
            return SCORE_MIME_PREFIX;
        }
    }
    SCORE_NONE
}

/// Núcleo de [`pick`] sobre una tabla arbitraria — separado para testearlo
/// sin tocar el [`registry`] global (que toca el filesystem).
fn pick_in(rows: &[ViewerCard], discernment: Option<&Discernment>) -> ViewerKind {
    let Some(d) = discernment else {
        return ViewerKind::Text;
    };
    if d.mime.as_deref() == Some("image/gif") {
        return ViewerKind::Video;
    }
    rows.iter()
        .map(|card| (score(card, d), card))
        .filter(|(s, _)| *s > SCORE_NONE)
        // mayor especificidad, luego mayor Priority; el orden de la tabla
        // queda como desempate estable (las descubiertas, al final, ganan).
        .max_by_key(|(s, card)| (*s, card.priority))
        .map(|(_, card)| card.kind)
        .unwrap_or(ViewerKind::Text)
}

/// Elige el visor para un discernimiento consultando [`registry`].
///
/// Reglas, en orden:
/// 1. Caso especial GIF: shuma lo marca `gallery` (es imagen), pero un GIF
///    animado se ve mejor reproducido. Va al video viewer (que acepta su
///    `FrameSource` y lo anima en loop; un GIF de un frame se ve estático).
/// 2. Match por la tabla: gana la fila con mayor especificidad
///    (`lens` > `mime` exacto > prefijo de `mime`), desempatada por
///    `Priority` y, en última instancia, por orden en la tabla.
/// 3. Fallback a [`ViewerKind::Text`] — el visor que nunca falla feo.
///
/// Un `None` (no se pudo discernir, p.ej. archivo ilegible) cae a texto.
pub fn pick(discernment: Option<&Discernment>) -> ViewerKind {
    pick_in(registry(), discernment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use card_core::{Card, CardKind, DataFacet, TypeRef};

    fn disc(lens: Option<&str>, mime: Option<&str>) -> Discernment {
        Discernment {
            ty: TypeRef::Primitive { name: "x".into() },
            confidence: 0.9,
            mime: mime.map(String::from),
            lens: lens.map(String::from),
        }
    }

    // `pick` sobre la tabla built-in pura — independiente del filesystem.
    fn pick_builtin(lens: Option<&str>, mime: Option<&str>) -> ViewerKind {
        pick_in(&builtin_registry(), Some(&disc(lens, mime)))
    }

    #[test]
    fn gallery_lens_va_a_imagen() {
        assert_eq!(pick_builtin(Some("gallery"), Some("image/png")), ViewerKind::Image);
    }

    #[test]
    fn mime_image_sin_lens_va_a_imagen() {
        assert_eq!(pick_builtin(None, Some("image/webp")), ViewerKind::Image);
    }

    #[test]
    fn gif_va_a_video_aunque_sea_gallery() {
        // shuma marca el GIF como gallery; igual lo anima el video viewer.
        assert_eq!(pick_builtin(Some("gallery"), Some("image/gif")), ViewerKind::Video);
    }

    #[test]
    fn video_lens_va_a_video() {
        assert_eq!(pick_builtin(Some("video"), Some("video/webm")), ViewerKind::Video);
    }

    #[test]
    fn mime_video_sin_lens_va_a_video() {
        assert_eq!(pick_builtin(None, Some("video/x-ivf")), ViewerKind::Video);
    }

    #[test]
    fn audio_lens_y_mime_van_a_audio() {
        assert_eq!(pick_builtin(Some("audio"), Some("audio/wav")), ViewerKind::Audio);
        assert_eq!(pick_builtin(None, Some("audio/mpeg")), ViewerKind::Audio);
    }

    #[test]
    fn card_lens_va_a_card() {
        assert_eq!(pick_builtin(Some("card"), Some("application/json")), ViewerKind::Card);
    }

    #[test]
    fn tree_lens_va_a_tree() {
        assert_eq!(pick_builtin(Some("tree"), Some("application/json")), ViewerKind::Tree);
        assert_eq!(pick_builtin(Some("tree"), Some("application/toml")), ViewerKind::Tree);
    }

    #[test]
    fn table_lens_va_a_table() {
        assert_eq!(pick_builtin(Some("table"), Some("text/csv")), ViewerKind::Table);
    }

    #[test]
    fn binarios_van_a_hex() {
        assert_eq!(pick_builtin(None, Some("application/x-executable")), ViewerKind::Hex);
        assert_eq!(pick_builtin(None, Some("application/wasm")), ViewerKind::Hex);
    }

    #[test]
    fn comprimidos_van_a_archive() {
        assert_eq!(pick_builtin(None, Some("application/zip")), ViewerKind::Archive);
        assert_eq!(pick_builtin(None, Some("application/x-tar")), ViewerKind::Archive);
        assert_eq!(pick_builtin(None, Some("application/gzip")), ViewerKind::Archive);
    }

    #[test]
    fn markdown_va_a_markdown() {
        assert_eq!(pick_builtin(Some("markdown"), Some("text/plain")), ViewerKind::Markdown);
    }

    #[test]
    fn font_lens_va_a_font() {
        assert_eq!(pick_builtin(Some("font"), Some("font/sfnt")), ViewerKind::Font);
    }

    #[test]
    fn code_va_a_texto() {
        assert_eq!(pick_builtin(Some("code"), Some("text/plain")), ViewerKind::Text);
    }

    #[test]
    fn html_va_a_web() {
        // Por mime (sin lens) y por lens — ambos rutean a puriy.
        assert_eq!(pick_builtin(None, Some("text/html")), ViewerKind::Web);
        assert_eq!(pick_builtin(None, Some("application/xhtml+xml")), ViewerKind::Web);
        assert_eq!(pick_builtin(Some("web"), Some("text/plain")), ViewerKind::Web);
        assert_eq!(pick_builtin(Some("html"), None), ViewerKind::Web);
    }

    #[test]
    fn sin_discernimiento_cae_a_texto() {
        assert_eq!(pick_in(&builtin_registry(), None), ViewerKind::Text);
    }

    #[test]
    fn lens_gana_sobre_mime_prefijo() {
        // lens explícito (especificidad 3) debe ganar aunque el mime también
        // matchee un prefijo de otro visor distinto.
        assert_eq!(pick_builtin(Some("tree"), Some("image/png")), ViewerKind::Tree);
    }

    #[test]
    fn tag_round_trip() {
        for card in builtin_registry() {
            assert_eq!(ViewerKind::from_tag(card.kind.as_tag()), Some(card.kind));
        }
        assert_eq!(ViewerKind::from_tag("inexistente"), None);
    }

    #[test]
    fn cada_kind_built_in_es_alcanzable_por_su_lens() {
        // garantía de cobertura: ninguna fila built-in queda muerta.
        let rows = builtin_registry();
        for card in &rows {
            if let Some(lens) = card.lenses.first() {
                assert_eq!(pick_in(&rows, Some(&disc(Some(lens), None))), card.kind);
            } else if let Some(mime) = card.mime_exact.first() {
                assert_eq!(pick_in(&rows, Some(&disc(None, Some(mime)))), card.kind);
            }
        }
    }

    // --- Descubrimiento: Card en disco → ViewerCard ---

    fn viewer_card(tag: &str, hint: &str, priority: Priority, exts: JsonValue) -> Card {
        let extensions = exts
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut ext = extensions;
        ext.insert("nahual.viewer_kind".into(), JsonValue::String(tag.into()));
        Card {
            kind: CardKind::Data,
            data: Some(DataFacet {
                presentation_hint: hint.into(),
                ..Default::default()
            }),
            priority,
            extensions: ext,
            ..Card::new("test.viewer")
        }
    }

    #[test]
    fn card_sin_tag_no_mapea() {
        let card = Card {
            kind: CardKind::Data,
            ..Card::new("test.sin-tag")
        };
        assert!(viewer_card_from_card(&card).is_none());
    }

    #[test]
    fn card_con_tag_desconocido_no_mapea() {
        let card = viewer_card("visor-marciano", "", Priority::Normal, serde_json::json!({}));
        assert!(viewer_card_from_card(&card).is_none());
    }

    #[test]
    fn card_mapea_lens_mime_y_priority() {
        let card = viewer_card(
            "image",
            "gallery",
            Priority::High,
            serde_json::json!({
                "nahual.mime_exact": ["image/heic"],
                "nahual.mime_prefixes": ["image/x-"],
                "nahual.lenses": ["fotos"]
            }),
        );
        let vc = viewer_card_from_card(&card).expect("debe mapear");
        assert_eq!(vc.kind, ViewerKind::Image);
        assert_eq!(vc.priority, Priority::High);
        assert!(vc.lenses.contains(&"fotos".to_string()));
        assert!(vc.lenses.contains(&"gallery".to_string())); // del presentation_hint
        assert!(vc.mime_exact.contains(&"image/heic".to_string()));
        assert!(vc.mime_prefixes.contains(&"image/x-".to_string()));
    }

    #[test]
    fn card_descubierta_extiende_ruteo_de_visor_montado() {
        // Una Card que enseña a nahual a abrir PSD con el visor de imágenes:
        // funciona end-to-end porque reusa el constructor in-process. (`image/`
        // no aplica acá: el mime de Photoshop es `application/...`, que ningún
        // built-in cubre, así que sin la Card cae a texto.)
        let card = viewer_card(
            "image",
            "",
            Priority::Normal,
            serde_json::json!({ "nahual.mime_exact": ["application/vnd.adobe.photoshop"] }),
        );
        let vc = viewer_card_from_card(&card).unwrap();
        let mut rows = builtin_registry();
        let psd = disc(None, Some("application/vnd.adobe.photoshop"));
        // Sin la Card, PSD no matchea nada rico → cae a texto.
        assert_eq!(pick_in(&rows, Some(&psd)), ViewerKind::Text);
        rows.push(vc);
        // Con la Card, va al visor de imágenes.
        assert_eq!(pick_in(&rows, Some(&psd)), ViewerKind::Image);
    }

    #[test]
    #[ignore = "helper: emite el JSON de ejemplo, no es una aserción"]
    fn emite_ejemplo_json() {
        let card = viewer_card(
            "image",
            "gallery",
            Priority::High,
            serde_json::json!({
                "nahual.mime_exact": ["application/vnd.adobe.photoshop"],
                "nahual.mime_prefixes": []
            }),
        );
        println!("{}", card.to_json_pretty().unwrap());
    }

    #[test]
    fn directorio_de_ejemplo_carga_y_rutea() {
        // El JSON de `viewers.d.example/` debe parsear, validar y producir un
        // ViewerCard que rutea PSD al visor de imágenes.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("viewers.d.example");
        let discovered = discover_in(&dir);
        assert!(
            discovered.iter().any(|vc| vc.kind == ViewerKind::Image
                && vc.mime_exact.iter().any(|m| m == "application/vnd.adobe.photoshop")),
            "el ejemplo debe descubrirse como visor de imágenes para PSD"
        );
        let mut rows = builtin_registry();
        rows.extend(discovered);
        let psd = disc(None, Some("application/vnd.adobe.photoshop"));
        assert_eq!(pick_in(&rows, Some(&psd)), ViewerKind::Image);
    }

    #[test]
    fn card_descubierta_gana_empate_por_orden() {
        // Una Card que sobrescribe el lens "gallery" para mandarlo a Hex
        // (caso artificial): al ir al final, gana el empate de especificidad.
        let card = viewer_card(
            "hex",
            "gallery",
            Priority::Normal,
            serde_json::json!({}),
        );
        let vc = viewer_card_from_card(&card).unwrap();
        let mut rows = builtin_registry();
        assert_eq!(pick_in(&rows, Some(&disc(Some("gallery"), None))), ViewerKind::Image);
        rows.push(vc);
        assert_eq!(pick_in(&rows, Some(&disc(Some("gallery"), None))), ViewerKind::Hex);
    }
}
