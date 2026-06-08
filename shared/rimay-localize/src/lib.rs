//! `rimay-localize` — i18n del escritorio tawasuyu sobre Fluent.
//!
//! Disciplina (PLAN.md §6.3):
//!
//! - Los `*-core` agnósticos **no contienen strings de UI**. Emiten
//!   identificadores (`MsgId`) que las apps Llimphi resuelven a texto
//!   localizado aquí, al renderizar.
//! - Un único catálogo `.ftl` por idioma vive en `locales/{lang}.ftl`,
//!   embebido en el binario vía [`include_str!`].
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
    /// Locale activo (clave de [`CATALOGS`]).
    active: String,
    /// Un bundle por catálogo embebido. Se construyen perezosamente la
    /// primera vez que un locale entra en uso y se cachean.
    bundles: HashMap<String, FluentBundle<FluentResource>>,
}

impl State {
    fn new() -> Self {
        Self {
            active: FALLBACK_LOCALE.to_string(),
            bundles: HashMap::new(),
        }
    }

    fn ensure_bundle(&mut self, locale: &str) -> Result<(), LocalizeError> {
        if self.bundles.contains_key(locale) {
            return Ok(());
        }
        let src = CATALOGS
            .iter()
            .find(|(l, _)| *l == locale)
            .map(|(_, s)| *s)
            .ok_or_else(|| LocalizeError::UnknownLocale(locale.to_string()))?;
        let langid: LanguageIdentifier = locale
            .parse()
            .map_err(|e: unic_langid::LanguageIdentifierError| {
                LocalizeError::InvalidLangId(locale.to_string(), e.to_string())
            })?;
        let res = FluentResource::try_new(src.to_string()).map_err(|(_, errs)| {
            LocalizeError::CatalogParse(
                locale.to_string(),
                errs.into_iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        let mut bundle = FluentBundle::new_concurrent(vec![langid]);
        // Fluent inserta caracteres bidi U+2068/U+2069 alrededor de los
        // placeables. En una UI de escritorio que no soporta BIDI complejo
        // (Llimphi no lo hace todavía) se ven como ◌. Los desactivamos:
        // los catálogos no mezclan RTL/LTR por ahora.
        bundle.set_use_isolating(false);
        if let Err(errs) = bundle.add_resource(res) {
            warn!(target: "rimay-localize", ?errs, "errores al añadir recurso a bundle '{locale}'");
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

/// Lista de locales disponibles (claves de [`CATALOGS`]).
pub fn available_locales() -> Vec<&'static str> {
    CATALOGS.iter().map(|(l, _)| *l).collect()
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

/// Mejor match entre un locale solicitado y los catalogados.
///
/// 1. Match exacto (`qu-PE` → `qu-PE`).
/// 2. Match por lengua base ignorando región (`es-AR` → `es-PE`,
///    `qu-BO` → `qu-PE`).
/// 3. Sin match → `None`.
fn best_match(requested: &str) -> Option<String> {
    if CATALOGS.iter().any(|(l, _)| *l == requested) {
        return Some(requested.to_string());
    }
    let base = requested.split(['-', '_']).next()?;
    CATALOGS
        .iter()
        .find(|(l, _)| l.split('-').next() == Some(base))
        .map(|(l, _)| l.to_string())
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
        assert!(v.contains(&"es-PE"));
        assert!(v.contains(&"en-US"));
        assert!(v.contains(&"qu-PE"));
    }
}
