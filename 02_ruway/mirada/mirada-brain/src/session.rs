//! Persistencia de sesión del escritorio — el agujero #1 de Wayland.
//!
//! Los entornos Wayland casi no recuerdan nada entre arranques. Como el
//! Cerebro ([`Desktop`](crate::Desktop)) es una state-machine pura, capturar y
//! restaurar su *forma* es casi gratis: este módulo define la cara serializable
//! de ese estado ([`DesktopState`]) y su ida y vuelta al disco en RON.
//!
//! Lo que se persiste es la **forma** del escritorio —los parámetros de
//! teselado de cada escritorio virtual, qué escritorio mostraba cada salida y
//! cuál tenía el foco—, **no** las ventanas vivas: sus [`WindowId`] son
//! efímeros (los clientes Wayland se reconectan con otros), así que la geometría
//! por-ventana no sobrevive a un reinicio. Restaurar ventanas concretas
//! (respawn por `app_id`) es un paso aparte que reusará las reglas de ventana.
//!
//! [`WindowId`]: mirada_layout::WindowId

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mirada_layout::LayoutParams;

/// Un nodo de la **forma** de una agrupación, anclado por `app_id` en vez de por
/// [`WindowId`] (efímero). Espeja [`mirada_layout::LayoutNode`] cambiando la hoja
/// de ventana por su `app_id`, lo único estable entre arranques.
///
/// [`WindowId`]: mirada_layout::WindowId
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeShape {
    /// Una ventana, identificada por el `app_id` de su cliente.
    Leaf(String),
    /// Un sub-espacio anidado.
    Space(SpaceShape),
}

/// La forma de un sub-espacio: sus parámetros de teselado + sus hijos por
/// `app_id`. Es la cara persistible de [`mirada_layout::SpaceNode`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceShape {
    pub params: LayoutParams,
    pub children: Vec<NodeShape>,
}

/// Versión del formato en disco de [`DesktopState`]. Se sube cuando el formato
/// cambia de forma incompatible; una sesión de otra versión se ignora sin
/// romper el arranque (ver [`DesktopState::from_ron`]).
pub const SESSION_VERSION: u32 = 1;

/// El estado persistible del escritorio — lo que sobrevive a un reinicio del
/// Cerebro. Lo produce [`Desktop::snapshot`](crate::Desktop::snapshot) y lo
/// consume [`Desktop::restore`](crate::Desktop::restore).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopState {
    /// Versión del formato — para migrar/ignorar sin romper.
    pub version: u32,
    /// Parámetros de teselado de cada escritorio virtual, en orden (0-based).
    pub workspaces: Vec<LayoutParams>,
    /// Qué escritorio mostraba cada salida, en su orden de aparición. Se
    /// re-aplica a medida que las salidas se reconectan.
    pub output_workspaces: Vec<usize>,
    /// Índice de la salida que tenía el foco.
    pub focused_output: usize,
    /// Para cada `app_id`, el escritorio (índice 0-based) donde vivía su
    /// ventana — para re-ubicar las que **reaparezcan** (al reabrir la app o al
    /// reconectar el Cuerpo). **No** respawnea nada: sólo enruta lo que vuelve a
    /// abrirse. Mapa plano: si una app tenía ventanas en varios escritorios,
    /// gana el de índice mayor. `#[serde(default)]` para que las sesiones
    /// viejas (sin este campo) sigan cargando.
    #[serde(default)]
    pub window_homes: Vec<(String, usize)>,
    /// La **forma** de la agrupación (árbol fractal del zoom-Z) de cada
    /// escritorio que estaba agrupado, anclada por `app_id`: `(índice de
    /// escritorio, forma)`. Sólo aparecen los agrupados. Al restaurar se queda
    /// pendiente y se **rematerializa** cuando todas las apps miembro reabren en
    /// ese escritorio (los `WindowId` nuevos se mapean por `app_id`).
    /// `#[serde(default)]` para que las sesiones viejas sigan cargando.
    #[serde(default)]
    pub groupings: Vec<(usize, SpaceShape)>,
}

impl DesktopState {
    /// El estado como RON con sangría — lo que [`save`](DesktopState::save)
    /// escribe en disco.
    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }

    /// Parsea un estado desde RON. Rechaza una versión de formato distinta de
    /// [`SESSION_VERSION`] (mejor empezar de cero que restaurar basura).
    pub fn from_ron(text: &str) -> Result<DesktopState, String> {
        let state: DesktopState = ron::from_str(text).map_err(|e| e.to_string())?;
        if state.version != SESSION_VERSION {
            return Err(format!(
                "versión de sesión {} ≠ {SESSION_VERSION} soportada",
                state.version
            ));
        }
        Ok(state)
    }

    /// La ruta canónica de la sesión: `~/.local/share/mirada/session.ron`. Es
    /// estado de ejecución (no config que el usuario edite a mano), por eso va
    /// en el directorio de datos y no en el de configuración.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "mirada")
            .map(|d| d.data_dir().join("session.ron"))
    }

    /// Persiste el estado al archivo RON. Escribe atómicamente (tmp + rename) y
    /// crea el directorio si falta.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = self
            .to_ron()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("ron.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Carga el estado de un archivo RON.
    pub fn load(path: &Path) -> Result<DesktopState, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("E/S: {e}"))?;
        DesktopState::from_ron(&text)
    }

    /// Carga la sesión si el archivo existe y es válido; `None` si no hay
    /// ninguna o está corrupta/obsoleta. Un error se avisa por `stderr` pero
    /// **no** rompe el arranque: el escritorio empieza con su forma por defecto.
    pub fn load_if_present(path: &Path) -> Option<DesktopState> {
        if !path.exists() {
            return None;
        }
        match DesktopState::load(path) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("mirada · sesión «{}» ignorada ({e}).", path.display());
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirada_layout::LayoutMode;

    fn sample() -> DesktopState {
        DesktopState {
            version: SESSION_VERSION,
            workspaces: vec![
                LayoutParams { mode: LayoutMode::Grid, master_ratio: 0.4, master_count: 2, gap: 12 },
                LayoutParams::default(),
            ],
            output_workspaces: vec![3, 0],
            focused_output: 1,
            window_homes: vec![("org.foo.bar".into(), 3), ("zed".into(), 0)],
            groupings: vec![(
                0,
                SpaceShape {
                    params: LayoutParams::default(),
                    children: vec![
                        NodeShape::Leaf("org.foo.bar".into()),
                        NodeShape::Space(SpaceShape {
                            params: LayoutParams { mode: LayoutMode::Rows, ..LayoutParams::default() },
                            children: vec![NodeShape::Leaf("zed".into()), NodeShape::Leaf("zed".into())],
                        }),
                    ],
                },
            )],
        }
    }

    #[test]
    fn ron_round_trips_the_state() {
        let s = sample();
        let back = DesktopState::from_ron(&s.to_ron().unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn a_different_version_is_rejected() {
        let mut s = sample();
        s.version = SESSION_VERSION + 1;
        let ron = s.to_ron().unwrap();
        assert!(DesktopState::from_ron(&ron).is_err());
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(DesktopState::from_ron("esto no es ron").is_err());
    }

    #[test]
    fn save_then_load_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("mirada-session-test-{}", std::process::id()));
        let path = dir.join("session.ron");
        let s = sample();
        s.save(&path).unwrap();
        assert_eq!(DesktopState::load_if_present(&path), Some(s));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_session_without_homes_still_loads() {
        // Una sesión escrita antes de añadir `window_homes`: debe cargar con la
        // lista vacía (gracias a `#[serde(default)]`), no fallar.
        let ron = "(version: 1, workspaces: [], output_workspaces: [], focused_output: 0)";
        let s = DesktopState::from_ron(ron).unwrap();
        assert!(s.window_homes.is_empty());
    }

    #[test]
    fn a_missing_file_is_none() {
        let path = std::env::temp_dir().join("mirada-no-such-session-xyz.ron");
        let _ = std::fs::remove_file(&path);
        assert_eq!(DesktopState::load_if_present(&path), None);
    }
}
