//! Reglas de ventana — config declarativa que decide, al abrirse una
//! ventana, a qué escritorio va y si flota.
//!
//! Mismo patrón que [`crate::keymap`]: RON de texto en
//! `~/.config/mirada/rules.ron`, que el [`Desktop`](crate::Desktop)
//! consulta en cada `WindowOpened` — el evento ya trae `app_id` y
//! `title`. Una regla casa por subcadena (sin distinguir mayúsculas);
//! gana la primera que case.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Una regla: criterio de coincidencia + qué aplicar.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rule {
    /// Subcadena que debe contener el `app_id`; vacía = casa con cualquiera.
    #[serde(default)]
    pub app_id: String,
    /// Subcadena que debe contener el título; vacía = cualquiera.
    #[serde(default)]
    pub title: String,
    /// Escritorio de destino (1-based); `0` = no moverla.
    #[serde(default)]
    pub workspace: usize,
    /// Abrir la ventana flotando.
    #[serde(default)]
    pub floating: bool,
    /// Abrir la ventana en pantalla completa (estilo Hyprland `fullscreen`).
    #[serde(default)]
    pub fullscreen: bool,
    /// Tamaño inicial `(ancho, alto)` en px si flota (Hyprland `size`). Implica
    /// `floating`. `(0, 0)` (o ausente) = tamaño por defecto (centrado).
    #[serde(default)]
    pub size: (i32, i32),
}

impl Rule {
    /// `true` si la regla casa con una ventana de este `app_id`/`title`.
    fn matches(&self, app_id: &str, title: &str) -> bool {
        let app_ok = self.app_id.is_empty() || contains_ci(app_id, &self.app_id);
        let title_ok = self.title.is_empty() || contains_ci(title, &self.title);
        app_ok && title_ok
    }
}

/// `true` si `haystack` contiene `needle`, sin distinguir mayúsculas.
fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Qué hacer con una ventana recién abierta, según las reglas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RuleOutcome {
    /// Escritorio de destino, ya como índice 0-based. `None` = el activo.
    pub workspace: Option<usize>,
    /// Abrir flotando.
    pub floating: bool,
    /// Abrir en pantalla completa.
    pub fullscreen: bool,
    /// Tamaño inicial `(ancho, alto)` si flota. `None` = por defecto.
    pub size: Option<(i32, i32)>,
}

/// El conjunto de reglas de ventana del usuario.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    #[serde(default)]
    rules: Vec<Rule>,
}

impl Rules {
    /// Construye un conjunto de reglas a partir de una lista.
    pub fn new(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    /// Resuelve qué hacer con una ventana — gana la primera regla que case.
    pub fn resolve(&self, app_id: &str, title: &str) -> RuleOutcome {
        for r in &self.rules {
            if r.matches(app_id, title) {
                let has_size = r.size.0 > 0 && r.size.1 > 0;
                return RuleOutcome {
                    workspace: (r.workspace >= 1).then(|| r.workspace - 1),
                    // `size` implica flotar (no tiene sentido teselada).
                    floating: r.floating || has_size,
                    fullscreen: r.fullscreen,
                    size: has_size.then_some(r.size),
                };
            }
        }
        RuleOutcome::default()
    }

    /// Cuántas reglas hay.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// `true` si no hay ninguna regla.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Parsea las reglas desde el texto RON de un archivo de config.
    pub fn from_ron(text: &str) -> Result<Rules, String> {
        ron::from_str(text).map_err(|e| format!("RON inválido: {e}"))
    }

    /// La ruta canónica de las reglas: `~/.config/mirada/rules.ron`.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "mirada")
            .map(|d| d.config_dir().join("rules.ron"))
    }

    /// Carga las reglas de un archivo RON.
    pub fn load(path: &Path) -> Result<Rules, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("E/S: {e}"))?;
        Rules::from_ron(&text)
    }

    /// Vigila el archivo de reglas para recargarlo en caliente.
    pub fn watch(path: &Path) -> notify::Result<crate::watch::FileWatch> {
        crate::watch::FileWatch::new(path)
    }

    /// Carga las reglas del usuario con un fallback amable: si el archivo
    /// no existe, escribe una plantilla documentada y devuelve un
    /// conjunto vacío; si está corrupto, avisa y devuelve vacío.
    pub fn load_or_default(path: &Path) -> Rules {
        if path.exists() {
            match Rules::load(path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!(
                        "mirada · reglas «{}» inválidas ({e}); las ignoro.",
                        path.display()
                    );
                    Rules::default()
                }
            }
        } else {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match std::fs::write(path, RULES_TEMPLATE) {
                Ok(()) => eprintln!("mirada · plantilla de reglas escrita en {}", path.display()),
                Err(e) => eprintln!("mirada · no pude escribir la plantilla de reglas: {e}"),
            }
            Rules::default()
        }
    }
}

/// La plantilla que se escribe la primera vez — sin reglas, con ejemplos
/// comentados para que el usuario los descubra.
const RULES_TEMPLATE: &str = "\
// Reglas de ventana de mirada — qué hacer con una ventana al abrirse.
//
// Cada regla casa por subcadena de `app_id` (la «clase») y/o `title` (sin
// distinguir mayúsculas; cadena vacía = cualquiera) y aplica:
//   workspace:  1..9 (0 = no mover)        ·  floating:   abre flotando
//   fullscreen: abre a pantalla completa   ·  size:       (ancho, alto) px (flota)
// Gana la primera regla que case.
//
// Descomenta y edita los ejemplos:
(
    rules: [
        // (app_id: \"pavucontrol\", floating: true),
        // (app_id: \"firefox\", workspace: 2),
        // (title: \"Picture-in-Picture\", floating: true, size: (480, 270)),
        // (app_id: \"mpv\", fullscreen: true),
    ],
)
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_parses_to_an_empty_rule_set() {
        assert!(Rules::from_ron(RULES_TEMPLATE).unwrap().is_empty());
    }

    #[test]
    fn rules_parse_from_ron_with_omitted_fields() {
        let ron = r#"(
            rules: [
                (app_id: "pavucontrol", floating: true),
                (app_id: "firefox", workspace: 2),
            ],
        )"#;
        assert_eq!(Rules::from_ron(ron).unwrap().len(), 2);
    }

    #[test]
    fn resolve_sends_a_match_to_its_workspace() {
        let r = Rules::from_ron(r#"( rules: [ (app_id: "firefox", workspace: 3) ] )"#).unwrap();
        let out = r.resolve("org.mozilla.firefox", "");
        assert_eq!(out.workspace, Some(2)); // 3 (1-based) -> índice 2
        assert!(!out.floating);
    }

    #[test]
    fn resolve_matches_app_id_case_insensitively_by_substring() {
        let r = Rules::from_ron(r#"( rules: [ (app_id: "FIREFOX", floating: true) ] )"#).unwrap();
        assert!(r.resolve("org.mozilla.firefox", "").floating);
    }

    #[test]
    fn resolve_matches_by_title() {
        let r =
            Rules::from_ron(r#"( rules: [ (title: "Picture-in-Picture", floating: true) ] )"#)
                .unwrap();
        assert!(r.resolve("cualquiera", "YouTube — Picture-in-Picture").floating);
        assert!(!r.resolve("cualquiera", "ventana normal").floating);
    }

    #[test]
    fn the_first_matching_rule_wins() {
        let r = Rules::from_ron(
            r#"( rules: [ (app_id: "term", workspace: 1), (app_id: "term", workspace: 5) ] )"#,
        )
        .unwrap();
        assert_eq!(r.resolve("term", "").workspace, Some(0));
    }

    #[test]
    fn resolve_carries_fullscreen_and_size() {
        let r = Rules::from_ron(
            r#"( rules: [ (app_id: "mpv", fullscreen: true), (title: "PiP", size: (480, 270)) ] )"#,
        )
        .unwrap();
        let a = r.resolve("mpv", "");
        assert!(a.fullscreen);
        let b = r.resolve("x", "YouTube PiP");
        assert_eq!(b.size, Some((480, 270)));
        assert!(b.floating, "una regla con `size` implica flotar");
    }

    #[test]
    fn no_match_yields_the_default_outcome() {
        let r = Rules::from_ron(r#"( rules: [ (app_id: "firefox", workspace: 2) ] )"#).unwrap();
        assert_eq!(r.resolve("xterm", ""), RuleOutcome::default());
    }
}
