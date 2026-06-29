//! `rimay-localize` — i18n del escritorio tawasuyu sobre Fluent.
//!
//! Disciplina (PLAN.md §6.3):
//!
//! - Los `*-core` agnósticos **no contienen strings de UI**. Emiten
//!   identificadores (`MsgId`) que las apps Llimphi resuelven a texto
//!   localizado aquí, al renderizar.
//! - Un único catálogo `.ftl` por idioma vive en `locales/{lang}.ftl`,
//!   embebido en el binario vía [`include_str!`]. El embebido es el
//!   **fallback garantizado**: nunca se puede romper la app borrando un
//!   archivo de disco.
//! - Sobre el embebido se pueden **superponer catálogos en runtime**
//!   ([`register_override`]) — para traducir o corregir strings, o añadir
//!   un idioma nuevo, **sin recompilar**. La primitiva toma el *contenido*
//!   `.ftl`, no una ruta: en el host el helper [`load_overrides_from_dir`]
//!   lo lee de disco (`~/.config/wawa/locales/`, `/etc/wawa/locales/`),
//!   pero en wawa (sin filesystem POSIX) lo que corra ahí lee los bytes de
//!   su almacén direccionado por contenido y los inyecta con la misma
//!   primitiva. La capa de override gana sobre el embebido; usuario gana
//!   sobre sistema.
//! - El locale activo es un singleton de proceso. Cambiarlo recarga el
//!   bundle pero **no** retransla automáticamente la vista — las apps
//!   Llimphi vuelven a llamar a [`t`] en el próximo `view()`.
//!
//! ## API mínima
//!
//! ```no_run
//! use rimay_localize as l10n;
//!
//! // Una vez al inicio de la app: detecta `LANG`/sistema, fallback es-PE.
//! l10n::init();
//!
//! // En cualquier `view()`:
//! let label = l10n::t("save");
//!
//! // Con argumentos posicionales tipo Fluent `{ $name }`:
//! let greet = l10n::t_args("welcome-user", &[("name", "Sergio".into())]);
//! ```
//!
//! ## Por qué Fluent (y no gettext / embeddings)
//!
//! - **Fluent** trae plurales/género/contextos declarativos en el `.ftl`,
//!   sin macros de código. Esencial para idiomas aglutinantes (quechua)
//!   donde la pluralización no es la binaria one/other del inglés.
//! - **Embeddings** son la herramienta correcta para *búsqueda semántica*
//!   (command palette, intent → acción) — ver [`rimay-verbo`]. **No** para
//!   strings deterministas de UI.
//!
//! ## Alcance fuera de wawa
//!
//! Este crate requiere `std` y `alloc` (Fluent tira de ambos). El kernel
//! `wawa` es `no_std` y no se localiza: emite **códigos** de error que
//! las apps WASM por encima traducen consultando este catálogo.

#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use thiserror::Error;
use tracing::warn;
use unic_langid::LanguageIdentifier;

// =====================================================================
// Catálogos embebidos
// =====================================================================

/// Lista de catálogos compilada al binario. Para añadir un idioma:
/// 1. Crear `locales/{lang}.ftl`.
/// 2. Añadir la tupla `("{lang}", include_str!(...))` aquí.
///
/// **Orden = prioridad declarada del proyecto**: español primero (es el
/// fallback y la lengua de trabajo), quechua segundo (lengua de la
/// arquitectura del monorepo), inglés tercero (uso técnico). Cambios
/// futuros conservan este orden por convención.
const CATALOGS: &[(&str, &str)] = &[
    ("es-PE", include_str!("../locales/es.ftl")),
    ("qu-PE", include_str!("../locales/qu.ftl")),
    ("en-US", include_str!("../locales/en.ftl")),
];

/// Locale por defecto cuando la detección del sistema falla o pide algo
/// que no tenemos catalogado.
pub const FALLBACK_LOCALE: &str = "es-PE";

// =====================================================================
// Errores
// =====================================================================

#[derive(Debug, Error)]
pub enum LocalizeError {
    #[error("locale '{0}' no está catalogado")]
    UnknownLocale(String),
    #[error("parseando catálogo de '{0}': {1}")]
    CatalogParse(String, String),
    #[error("identificador de locale inválido '{0}': {1}")]
    InvalidLangId(String, String),
}

// =====================================================================
// Estado global
// =====================================================================

struct State {
    /// Locale activo (clave de [`CATALOGS`] o de un override).
    active: String,
    /// Un bundle por catálogo en uso. Se construyen perezosamente la
    /// primera vez que un locale entra en uso y se cachean. Se invalidan
    /// (se vacía el mapa) al registrar un override.
    bundles: HashMap<String, FluentBundle<FluentResource>>,
    /// Catálogos `.ftl` superpuestos en runtime, por clave de locale ya
    /// canonicalizada (ver [`State::canonical_key`]). Cada entrada es la
    /// lista de fuentes en orden de registro: la última gana
    /// (`add_resource_overriding`). Vacío por defecto.
    overrides: HashMap<String, Vec<String>>,
}

impl State {
    fn new() -> Self {
        Self {
            active: FALLBACK_LOCALE.to_string(),
            bundles: HashMap::new(),
            overrides: HashMap::new(),
        }
    }

    /// Canonicaliza una clave de override a la del catálogo embebido que
    /// comparte lengua base, para que `es.ftl` / `es-AR.ftl` superpongan
    /// sobre `es-PE` (el embebido) en vez de crear un locale huérfano. Un
    /// idioma sin embebido (`de`) se conserva tal cual → locale nuevo.
    fn canonical_key(locale: &str) -> String {
        if CATALOGS.iter().any(|(l, _)| *l == locale) {
            return locale.to_string();
        }
        let base = locale.split(['-', '_']).next().unwrap_or(locale);
        CATALOGS
            .iter()
            .find(|(l, _)| l.split('-').next() == Some(base))
            .map(|(l, _)| l.to_string())
            .unwrap_or_else(|| locale.to_string())
    }

    /// Locales conocidos: embebidos ∪ overrides, en orden (embebidos
    /// primero, extras después). Sin duplicados.
    fn known_locales(&self) -> Vec<String> {
        let mut out: Vec<String> = CATALOGS.iter().map(|(l, _)| l.to_string()).collect();
        for k in self.overrides.keys() {
            if !out.contains(k) {
                out.push(k.clone());
            }
        }
        out
    }

    fn ensure_bundle(&mut self, locale: &str) -> Result<(), LocalizeError> {
        if self.bundles.contains_key(locale) {
            return Ok(());
        }
        let embedded = CATALOGS.iter().find(|(l, _)| *l == locale).map(|(_, s)| *s);
        let ov = self.overrides.get(locale);
        if embedded.is_none() && ov.map_or(true, |v| v.is_empty()) {
            return Err(LocalizeError::UnknownLocale(locale.to_string()));
        }
        let langid: LanguageIdentifier = locale
            .parse()
            .map_err(|e: unic_langid::LanguageIdentifierError| {
                LocalizeError::InvalidLangId(locale.to_string(), e.to_string())
            })?;
        let mut bundle = FluentBundle::new_concurrent(vec![langid]);
        // Fluent inserta caracteres bidi U+2068/U+2069 alrededor de los
        // placeables. En una UI de escritorio que no soporta BIDI complejo
        // (Llimphi no lo hace todavía) se ven como ◌. Los desactivamos:
        // los catálogos no mezclan RTL/LTR por ahora.
        bundle.set_use_isolating(false);
        // 1) Capa embebida (fallback garantizado).
        if let Some(src) = embedded {
            let res = FluentResource::try_new(src.to_string()).map_err(|(_, errs)| {
                LocalizeError::CatalogParse(
                    locale.to_string(),
                    errs.into_iter()
                        .map(|e| e.to_string())
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            })?;
            if let Err(errs) = bundle.add_resource(res) {
                warn!(target: "rimay-localize", ?errs, "errores al añadir recurso embebido a bundle '{locale}'");
            }
        }
        // 2) Capas de override, en orden de registro: la última pisa
        //    claves duplicadas (y el embebido). Una fuente que no parsea no
        //    tumba el bundle — se omite con un warn.
        if let Some(srcs) = self.overrides.get(locale) {
            for src in srcs {
                match FluentResource::try_new(src.clone()) {
                    Ok(res) => bundle.add_resource_overriding(res),
                    Err((_, errs)) => warn!(
                        target: "rimay-localize",
                        ?errs,
                        "override de '{locale}' no parsea — se omite"
                    ),
                }
            }
        }
        self.bundles.insert(locale.to_string(), bundle);
        Ok(())
    }
}

static STATE: Lazy<RwLock<State>> = Lazy::new(|| RwLock::new(State::new()));

// =====================================================================
// API pública
// =====================================================================

/// Inicializa el localizador detectando el locale del sistema (env
/// `LANG`/`LC_ALL` vía [`sys_locale`]) y eligiendo el más cercano de los
/// catálogos disponibles. Si no hay match, cae en [`FALLBACK_LOCALE`].
///
/// Idempotente — invocaciones sucesivas resetean el locale activo según
/// el sistema actual. Si la app quiere fijar el locale a mano, usar
/// [`set_locale`] después.
pub fn init() {
    // Cargar overrides de disco ANTES de detectar, así un idioma que solo
    // existe como `.ftl` en disco (sin embebido) puede ganar el match.
    load_system_overrides();
    let detected = sys_locale::get_locale().unwrap_or_else(|| FALLBACK_LOCALE.to_string());
    let chosen = best_match(&detected).unwrap_or_else(|| FALLBACK_LOCALE.to_string());
    let _ = set_locale(&chosen);
}

/// Cambia el locale activo. Compila el catálogo correspondiente si aún
/// no estaba cargado.
pub fn set_locale(locale: &str) -> Result<(), LocalizeError> {
    let mut state = STATE.write();
    state.ensure_bundle(locale)?;
    state.active = locale.to_string();
    Ok(())
}

/// Devuelve el locale activo (clave de [`CATALOGS`]).
pub fn current_locale() -> String {
    STATE.read().active.clone()
}

/// Lista de locales disponibles: catálogos embebidos ∪ overrides
/// registrados en runtime (embebidos primero). Devuelve `String` porque
/// los overrides son dinámicos, no `'static`.
pub fn available_locales() -> Vec<String> {
    STATE.read().known_locales()
}

// =====================================================================
// Overrides en runtime (traducción sin recompilar)
// =====================================================================

/// Superpone un catálogo `.ftl` sobre el embebido del mismo idioma — o
/// registra un idioma nuevo si no hay embebido que comparta lengua base.
///
/// Esta es la **primitiva agnóstica de fuente**: recibe el *contenido*
/// `.ftl`, no una ruta. En el host, [`load_overrides_from_dir`] la
/// alimenta desde disco; en wawa (sin filesystem POSIX) lo que corra ahí
/// lee los bytes de su almacén direccionado por contenido y llama aquí.
///
/// Las claves del override pisan las del embebido; varios overrides del
/// mismo locale se aplican en orden de registro (el último gana). Una
/// fuente que no parsea como Fluent se omite al construir el bundle (con
/// un warn), no tumba la app.
///
/// `locale` debe ser un identificador BCP-47 válido (`de`, `de-DE`,
/// `es-PE`); si no, devuelve [`LocalizeError::InvalidLangId`]. Registrar
/// invalida los bundles cacheados para que el próximo [`t`] los reconstruya.
pub fn register_override(locale: &str, ftl: &str) -> Result<(), LocalizeError> {
    // Validar que el locale parsea (falla temprano, no al resolver).
    let _: LanguageIdentifier = locale
        .parse()
        .map_err(|e: unic_langid::LanguageIdentifierError| {
            LocalizeError::InvalidLangId(locale.to_string(), e.to_string())
        })?;
    let key = State::canonical_key(locale);
    let mut state = STATE.write();
    state.overrides.entry(key).or_default().push(ftl.to_string());
    // Invalidar todo lo cacheado: la reconstrucción es perezosa y barata.
    state.bundles.clear();
    Ok(())
}

/// Directorio de overrides **de usuario**: `~/.config/wawa/locales/` en
/// Linux (vía `directories`, misma raíz `wawa` que `wawa-config`).
/// `None` si la plataforma no expone config dir.
pub fn user_locales_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "wawa").map(|d| d.config_dir().join("locales"))
}

/// Directorio de overrides **de sistema**: `/etc/wawa/locales/` en Linux;
/// `None` en otras plataformas (no hay convención equivalente).
pub fn system_locales_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        Some(PathBuf::from("/etc/wawa/locales"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Escanea un directorio de `.ftl` y registra cada uno como override. El
/// **nombre del archivo sin extensión es la clave de locale** (`de.ftl` →
/// `de`, `es-PE.ftl` → `es-PE`). Devuelve cuántos cargó. Errores de IO o
/// archivos que no parsean se omiten con un warn (no propaga). Helper
/// host-only: usa `std::fs`.
pub fn load_overrides_from_dir(dir: &Path) -> usize {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0, // directorio inexistente = sin overrides, normal
    };
    let mut n = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ftl") {
            continue;
        }
        let Some(locale) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        match std::fs::read_to_string(&path) {
            Ok(src) => match register_override(locale, &src) {
                Ok(()) => n += 1,
                Err(e) => {
                    warn!(target: "rimay-localize", %e, "override '{}' inválido", path.display())
                }
            },
            Err(e) => {
                warn!(target: "rimay-localize", %e, "no se pudo leer override '{}'", path.display())
            }
        }
    }
    n
}

/// Carga overrides de las capas estándar del host: **sistema primero,
/// usuario después** (así el usuario gana sobre el sistema, y ambos sobre
/// el embebido). Idempotente en la práctica salvo que se editen los
/// archivos. Devuelve el total cargado. La invoca [`init`].
pub fn load_system_overrides() -> usize {
    let mut n = 0;
    if let Some(d) = system_locales_dir() {
        n += load_overrides_from_dir(&d);
    }
    if let Some(d) = user_locales_dir() {
        n += load_overrides_from_dir(&d);
    }
    n
}

/// Resuelve un mensaje sin argumentos. Si el ID no existe en el catálogo
/// activo, devuelve el propio ID — facilita ver qué falta traducir sin
/// crashear la UI.
pub fn t(id: &str) -> String {
    resolve(id, None)
}

/// Resuelve un mensaje con argumentos posicionales tipo Fluent
/// (`{ $name }`). Los valores se convierten a [`FluentValue`] vía la
/// impl `From<Cow<str>>` — números pásalos pre-formateados como string
/// si querés controlar la presentación.
pub fn t_args(id: &str, args: &[(&str, Cow<'_, str>)]) -> String {
    let mut fa = FluentArgs::new();
    for (k, v) in args {
        fa.set(*k, FluentValue::from(v.clone().into_owned()));
    }
    resolve(id, Some(&fa))
}

// =====================================================================
// Internos
// =====================================================================

fn resolve(id: &str, args: Option<&FluentArgs>) -> String {
    // Auto-init lazy: si `t()` se llama sin `init()` previo (típico desde
    // librerías como `cosmos-modules` que no ven el `main`), cargamos el
    // fallback en ese momento. Es un costo amortizado de una sola vez.
    if STATE.read().bundles.is_empty() {
        let mut w = STATE.write();
        if w.bundles.is_empty() {
            let _ = w.ensure_bundle(FALLBACK_LOCALE);
            w.active = FALLBACK_LOCALE.to_string();
        }
    }
    let state = STATE.read();
    let bundle = match state.bundles.get(&state.active) {
        Some(b) => b,
        None => return id.to_string(),
    };
    let Some(msg) = bundle.get_message(id) else {
        return id.to_string();
    };
    let Some(pattern) = msg.value() else {
        return id.to_string();
    };
    let mut errors = vec![];
    let s = bundle.format_pattern(pattern, args, &mut errors);
    if !errors.is_empty() {
        warn!(target: "rimay-localize", ?errors, "errores formateando '{id}' en locale '{}'", state.active);
    }
    s.into_owned()
}

/// Mejor match entre un locale solicitado y los disponibles (embebidos ∪
/// overrides registrados).
///
/// 1. Match exacto (`qu-PE` → `qu-PE`).
/// 2. Match por lengua base ignorando región (`es-AR` → `es-PE`,
///    `qu-BO` → `qu-PE`, `de-AT` → `de` si hay override `de`).
/// 3. Sin match → `None`.
fn best_match(requested: &str) -> Option<String> {
    let known = STATE.read().known_locales();
    if known.iter().any(|l| l == requested) {
        return Some(requested.to_string());
    }
    let base = requested.split(['-', '_']).next()?;
    known
        .iter()
        .find(|l| l.split('-').next() == Some(base))
        .cloned()
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Los tests comparten estado global → serialización manual.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn fallback_locale_resolves() {
        let _g = SERIAL.lock().unwrap();
        set_locale("es-PE").unwrap();
        let s = t("save");
        assert_eq!(s, "Guardar");
    }

    #[test]
    fn switches_to_english() {
        let _g = SERIAL.lock().unwrap();
        set_locale("en-US").unwrap();
        assert_eq!(t("save"), "Save");
        assert_eq!(t("cancel"), "Cancel");
    }

    #[test]
    fn quechua_loads() {
        let _g = SERIAL.lock().unwrap();
        set_locale("qu-PE").unwrap();
        // Solo verificamos que devuelva *algo distinto* del id, no la
        // traducción literal — para no acoplar el test al fraseo exacto
        // del catálogo (que el revisor de quechua va a ajustar).
        let s = t("save");
        assert_ne!(s, "save", "qu-PE no resolvió 'save'");
    }

    #[test]
    fn unknown_id_returns_id_as_degradation() {
        let _g = SERIAL.lock().unwrap();
        set_locale("es-PE").unwrap();
        assert_eq!(t("__id_que_no_existe__"), "__id_que_no_existe__");
    }

    #[test]
    fn args_interpolate() {
        let _g = SERIAL.lock().unwrap();
        set_locale("es-PE").unwrap();
        let s = t_args("welcome-user", &[("name", "Sergio".into())]);
        assert!(s.contains("Sergio"), "no interpoló name: {s}");
    }

    #[test]
    fn best_match_region_fallback() {
        assert_eq!(best_match("es-AR"), Some("es-PE".to_string()));
        assert_eq!(best_match("en-GB"), Some("en-US".to_string()));
        assert_eq!(best_match("qu-BO"), Some("qu-PE".to_string()));
        assert_eq!(best_match("ja-JP"), None);
    }

    #[test]
    fn unknown_locale_errors() {
        let err = set_locale("xx-YY").unwrap_err();
        assert!(matches!(err, LocalizeError::UnknownLocale(_)));
    }

    #[test]
    fn available_locales_lists_all() {
        let v = available_locales();
        assert!(v.iter().any(|l| l == "es-PE"));
        assert!(v.iter().any(|l| l == "en-US"));
        assert!(v.iter().any(|l| l == "qu-PE"));
    }

    #[test]
    fn override_pisa_clave_embebida() {
        let _g = SERIAL.lock().unwrap();
        // `es.ftl` con UNA clave → canonicaliza a es-PE y pisa solo esa.
        register_override("es", "save = Resguardar\n").unwrap();
        set_locale("es-PE").unwrap();
        assert_eq!(t("save"), "Resguardar", "el override no pisó 'save'");
        // Una clave no incluida en el override sigue viniendo del embebido.
        assert_eq!(t("cancel"), "Cancelar", "el override borró otras claves");
        // Limpieza: re-registrar el valor embebido para no contaminar otros tests.
        register_override("es", "save = Guardar\n").unwrap();
        STATE.write().bundles.clear();
    }

    #[test]
    fn override_introduce_locale_nuevo() {
        let _g = SERIAL.lock().unwrap();
        // Idioma sin embebido: queda como locale propio y es matcheable.
        register_override("de", "save = Speichern\n").unwrap();
        assert!(available_locales().iter().any(|l| l == "de"));
        assert_eq!(best_match("de-AT"), Some("de".to_string()));
        set_locale("de").unwrap();
        assert_eq!(t("save"), "Speichern");
    }

    #[test]
    fn load_overrides_from_dir_lee_ftl() {
        let _g = SERIAL.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fr.ftl"), "save = Enregistrer\n").unwrap();
        std::fs::write(dir.path().join("ignorar.txt"), "no soy ftl\n").unwrap();
        let n = load_overrides_from_dir(dir.path());
        assert_eq!(n, 1, "solo el .ftl debe contar");
        set_locale("fr").unwrap();
        assert_eq!(t("save"), "Enregistrer");
    }
}
