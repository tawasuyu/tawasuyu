//! El keymap configurable — atajos del escritorio en RON, recargables en
//! caliente.
//!
//! # Dónde vive el keymap
//!
//! Sólo en el Cerebro. El Cuerpo (`mirada-compositor`) **nunca** ve este
//! mapa: lo único que recibe es la lista de cadenas a interceptar
//! ([`grab_list`](Keymap::grab_list)) dentro de un
//! [`BrainCommand::GrabKeys`](mirada_protocol::BrainCommand::GrabKeys). El
//! Cuerpo hace un `Vec::contains` ciego y devuelve la combinación pulsada
//! como [`BodyEvent::Keybind`](mirada_protocol::BodyEvent::Keybind); es el
//! [`Desktop`](crate::Desktop) quien la traduce a una
//! [`DesktopAction`]. Esa separación —*qué* interceptar vs. *qué
//! significa*— es la que hace innecesario cualquier candado o `Arc`:
//! el mapa es monohilo aquí y la lista viaja de golpe en un solo mensaje.
//!
//! # Persistencia
//!
//! En disco es RON de texto (`~/.config/mirada/keymap.ron`), editable a
//! mano y versionable. El cable sólo lleva la lista de cadenas; no hay
//! format binario de configuración. Hay un único ejecutable que hace de
//! "configurador": la app `mirada`, que carga este archivo al arrancar.
//!
//! # Recarga en caliente
//!
//! [`Keymap::watch`] devuelve un [`KeymapWatch`] que vigila el archivo;
//! cuando cambia, el dueño del [`Desktop`](crate::Desktop) recarga el
//! keymap, llama a [`Desktop::set_keymap`](crate::Desktop::set_keymap) y
//! reenvía el `GrabKeys` resultante. Sin reiniciar nada.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::action::{default_keymap, DesktopAction};
use crate::watch::FileWatch;

/// Atajos del escritorio: combinación canónica → acción.
///
/// La combinación es la cadena que canoniza el Cuerpo (`"Super+Shift+j"`,
/// `"Super+space"`…). El keymap es lo único que la traduce a una
/// [`DesktopAction`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    bindings: BTreeMap<String, DesktopAction>,
}

impl Default for Keymap {
    /// El keymap por defecto, estilo *tiling WM* (ver [`default_keymap`]).
    fn default() -> Self {
        Self {
            bindings: default_keymap().into_iter().collect(),
        }
    }
}

impl Keymap {
    /// Construye un keymap a partir de pares `(combinación, acción)`.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, DesktopAction)>) -> Self {
        Self {
            bindings: pairs.into_iter().collect(),
        }
    }

    /// La acción asociada a una combinación, si la hay.
    pub fn lookup(&self, combo: &str) -> Option<DesktopAction> {
        self.bindings.get(combo).cloned()
    }

    /// Las combinaciones a interceptar — el contenido de un `GrabKeys`.
    pub fn grab_list(&self) -> Vec<String> {
        self.bindings.keys().cloned().collect()
    }

    /// Todos los atajos, en orden de combinación.
    pub fn bindings(&self) -> &BTreeMap<String, DesktopAction> {
        &self.bindings
    }

    /// Cuántos atajos hay.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// `true` si no hay ningún atajo.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    // --- RON ----------------------------------------------------------

    /// Parsea un keymap desde el texto RON de un archivo de configuración.
    pub fn from_ron(text: &str) -> Result<Keymap, KeymapError> {
        let file: KeymapFile = ron::from_str(text)
            .map_err(|e| KeymapError::Parse(format!("RON inválido: {e}")))?;
        let mut bindings = BTreeMap::new();
        for (combo, action) in file.bindings {
            let parsed = action
                .parse::<DesktopAction>()
                .map_err(|e| KeymapError::Parse(format!("atajo \"{combo}\": {e}")))?;
            bindings.insert(combo, parsed);
        }
        Ok(Keymap { bindings })
    }

    /// Serializa el keymap a RON (sin la cabecera de documentación).
    pub fn to_ron(&self) -> String {
        let file = KeymapFile {
            bindings: self
                .bindings
                .iter()
                .map(|(k, v)| (k.clone(), v.to_string()))
                .collect(),
        };
        ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())
            .expect("un KeymapFile de cadenas siempre serializa")
    }

    // --- Edición por tabla (panel de control) -------------------------

    /// Las filas `[combinación, acción]` para editar el keymap como tabla, en
    /// orden de combinación. La acción se muestra con su `Display` (la misma
    /// forma que el RON, p. ej. `FocusNext` o `Spawn("kitty")`).
    pub fn to_rows(&self) -> Vec<Vec<String>> {
        self.bindings
            .iter()
            .map(|(combo, action)| vec![combo.clone(), action.to_string()])
            .collect()
    }

    /// Reconstruye un keymap desde filas `[combinación, acción]` de la tabla.
    /// Las filas con combinación vacía o acción que no parsea a [`DesktopAction`]
    /// se **omiten** (el panel conserva el texto crudo aparte para que no se
    /// pierdan mientras se editan; acá sólo entran las válidas).
    pub fn from_rows(rows: &[Vec<String>]) -> Keymap {
        let mut bindings = BTreeMap::new();
        for r in rows {
            let combo = r.first().map(String::as_str).unwrap_or("").trim();
            let action = r.get(1).map(String::as_str).unwrap_or("").trim();
            if combo.is_empty() {
                continue;
            }
            if let Ok(parsed) = action.parse::<DesktopAction>() {
                bindings.insert(combo.to_string(), parsed);
            }
        }
        Keymap { bindings }
    }

    // --- Disco --------------------------------------------------------

    /// La ruta canónica del keymap del usuario: `~/.config/mirada/keymap.ron`.
    /// `None` si no se puede determinar el directorio de configuración.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "mirada")
            .map(|d| d.config_dir().join("keymap.ron"))
    }

    /// Carga un keymap desde un archivo RON.
    pub fn load(path: &Path) -> Result<Keymap, KeymapError> {
        let text = std::fs::read_to_string(path)?;
        Keymap::from_ron(&text)
    }

    /// El keymap como RON con la cabecera de documentación — exactamente
    /// lo que [`save`](Keymap::save) escribe en disco.
    pub fn documented_ron(&self) -> String {
        format!("{KEYMAP_HEADER}\n{}", self.to_ron())
    }

    /// Escribe el keymap a `path` como RON documentado (con cabecera de
    /// comentarios), creando el directorio padre si falta.
    pub fn save(&self, path: &Path) -> Result<(), KeymapError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.documented_ron())?;
        Ok(())
    }

    /// Carga el keymap del usuario con un fallback amable:
    ///
    /// - si el archivo no existe, escribe uno por defecto documentado y lo
    ///   devuelve (así el usuario lo descubre y lo puede editar);
    /// - si existe pero está corrupto, avisa por `stderr` y devuelve el
    ///   keymap por defecto **sin tocar el archivo** (no se pierde el
    ///   trabajo del usuario por un error de sintaxis).
    pub fn load_or_init(path: &Path) -> Keymap {
        if path.exists() {
            match Keymap::load(path) {
                Ok(km) => km,
                Err(e) => {
                    eprintln!(
                        "mirada · keymap «{}» inválido ({e}); uso el de por defecto.",
                        path.display()
                    );
                    Keymap::default()
                }
            }
        } else {
            let km = Keymap::default();
            match km.save(path) {
                Ok(()) => eprintln!("mirada · keymap inicial escrito en {}", path.display()),
                Err(e) => eprintln!("mirada · no pude escribir el keymap inicial: {e}"),
            }
            km
        }
    }

    /// Vigila el archivo del keymap para recargarlo en caliente — un
    /// [`FileWatch`] genérico, igual que la config y las reglas.
    pub fn watch(path: &Path) -> notify::Result<KeymapWatch> {
        FileWatch::new(path)
    }
}

/// Vigía del archivo de keymap para la recarga en caliente. Hoy es un
/// alias del [`FileWatch`] genérico — se conserva el nombre por
/// compatibilidad con quien lo nombra (`mirada-compositor`).
pub type KeymapWatch = FileWatch;

/// La forma en disco del keymap — un mapa de cadenas. Las acciones van
/// como texto (`"layout:grid"`) y no como enum, para que el RON sea
/// trivial y los errores se reporten atajo a atajo.
#[derive(Serialize, Deserialize)]
struct KeymapFile {
    bindings: BTreeMap<String, String>,
}

/// La cabecera de comentarios del archivo que escribe [`Keymap::save`].
const KEYMAP_HEADER: &str = "\
// keymap de mirada — atajos del escritorio (carmen).
//
// Formato:  \"Combinación\": \"acción\"
// La combinación la canoniza el compositor: Super, Ctrl, Shift, Alt y la
// tecla, en ese orden (p. ej. \"Super+Shift+j\", \"Super+space\").
//
// Acciones:
//   focus-next / focus-prev          mueve el foco (cíclico)
//   focus-left/right/up/down         mueve el foco espacial (Super+flechas)
//   move-forward / move-backward     reordena la ventana enfocada (orden)
//   move-left/right/up/down          mueve la ventana por geometría (Super+Shift+flechas)
//   close-focused                    cierra la enfocada
//   toggle-float                     alterna flotante / teselada (una)
//   toggle-tiling                    alterna todo el escritorio teselado/flotante
//   toggle-fullscreen                alterna pantalla completa
//   send-to-scratchpad               guarda la enfocada en el scratchpad
//   toggle-scratchpad                invoca / oculta la del scratchpad
//   toggle-dropterm                  baja / sube la terminal dropdown (quake)
//   cycle-layout                     siguiente modo de teselado
//   layout:<modo>                    master-stack | centered-master | spiral
//                                    grid | columns | rows | monocle
//   grow-master / shrink-master      redimensiona el área maestra
//   inc-master / dec-master          nº de ventanas maestras (nmaster)
//   promote-to-master                la enfocada al puesto maestro (rota)
//   swap-master                      intercambia la enfocada con la maestra (sólo esas dos)
//   resize-float-left/right/up/down   redimensiona la flotante enfocada
//   focus-output-next                pasa el foco al siguiente monitor
//   focus-output-left/right/up/down   foco al monitor vecino (por geometría)
//   send-to-output-left/right/up/down manda la enfocada al monitor vecino
//   workspace:N                      activa el escritorio N (1..9)
//   send-to-workspace:N              manda la enfocada al escritorio N (sin saltar)
//   move-to-workspace:N              manda la enfocada al escritorio N y salta allí
//   spawn:<comando>                  lanza un programa (p. ej. spawn:foot)
//   quit                             apaga el compositor
//
// Edita y guarda: mirada recarga el keymap en caliente, sin reiniciar.";

/// Un fallo al cargar o guardar un keymap.
#[derive(Debug)]
pub enum KeymapError {
    /// El RON no parsea, o una acción no se reconoce. El mensaje ya está
    /// formateado para mostrarse al usuario.
    Parse(String),
    /// Fallo de E/S al leer o escribir el archivo.
    Io(io::Error),
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeymapError::Parse(msg) => f.write_str(msg),
            KeymapError::Io(e) => write!(f, "E/S: {e}"),
        }
    }
}

impl std::error::Error for KeymapError {}

impl From<io::Error> for KeymapError {
    fn from(e: io::Error) -> Self {
        KeymapError::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirada_layout::LayoutMode;

    #[test]
    fn the_default_keymap_round_trips_through_ron() {
        let km = Keymap::default();
        let back = Keymap::from_ron(&km.to_ron()).unwrap();
        assert_eq!(km, back);
    }

    #[test]
    fn to_rows_y_from_rows_round_trip() {
        let km = Keymap::default();
        let rows = km.to_rows();
        assert_eq!(rows.len(), km.len());
        // Cada fila es [combo, accion].
        assert!(rows.iter().all(|r| r.len() == 2));
        assert_eq!(Keymap::from_rows(&rows), km);
    }

    #[test]
    fn from_rows_omite_invalidas_y_vacias() {
        let rows = vec![
            vec!["Super+q".into(), "close-focused".into()], // válida (slug kebab)
            vec!["".into(), "focus-next".into()],           // combo vacío → omitida
            vec!["Super+x".into(), "acción-inventada".into()], // acción inválida → omitida
        ];
        let km = Keymap::from_rows(&rows);
        assert_eq!(km.len(), 1);
        assert_eq!(km.lookup("Super+q"), Some(DesktopAction::CloseFocused));
        assert_eq!(km.lookup("Super+x"), None);
    }

    #[test]
    fn the_saved_file_carries_the_documentation_header() {
        let km = Keymap::default();
        let written = km.documented_ron();
        // La cabecera son comentarios — RON los ignora al reparsear.
        assert!(written.starts_with("// keymap de mirada"));
        assert_eq!(Keymap::from_ron(&written).unwrap(), km);
    }

    #[test]
    fn grab_list_is_exactly_the_set_of_bound_combos() {
        let km = Keymap::default();
        let grabs = km.grab_list();
        assert_eq!(grabs.len(), km.len());
        assert!(grabs.contains(&"Super+j".to_string()));
        assert!(grabs.contains(&"Super+Shift+e".to_string()));
    }

    #[test]
    fn lookup_resolves_a_default_binding() {
        let km = Keymap::default();
        assert_eq!(km.lookup("Super+q"), Some(DesktopAction::CloseFocused));
        assert_eq!(km.lookup("Super+t"), Some(DesktopAction::SetLayout(LayoutMode::MasterStack)));
        assert_eq!(km.lookup("Super+sin-asignar"), None);
    }

    #[test]
    fn a_custom_keymap_parses_from_ron() {
        let ron = r#"(
            bindings: {
                "Alt+Return": "cycle-layout",
                "Alt+x": "close-focused",
                "Alt+3": "workspace:3",
            },
        )"#;
        let km = Keymap::from_ron(ron).unwrap();
        assert_eq!(km.len(), 3);
        assert_eq!(km.lookup("Alt+Return"), Some(DesktopAction::CycleLayout));
        assert_eq!(km.lookup("Alt+3"), Some(DesktopAction::SwitchWorkspace(2)));
    }

    #[test]
    fn an_unknown_action_names_the_offending_binding() {
        let ron = r#"( bindings: { "Super+z": "fly-away" } )"#;
        let err = Keymap::from_ron(ron).unwrap_err().to_string();
        assert!(err.contains("Super+z"), "el error debe nombrar el atajo: {err}");
    }

    #[test]
    fn malformed_ron_is_rejected() {
        assert!(Keymap::from_ron("esto no es ron").is_err());
    }
}
