//! Perfiles de atajos — una biblioteca de keymaps conmutables.
//!
//! mirada no trae **un** keymap sino una **biblioteca**: presets de fábrica
//! (`dwm`, `i3`, `hyprland`) más los que el usuario cree. Sobre ella se puede
//!
//! - **conmutar** el activo ([`set_active`](KeymapProfiles::set_active)),
//! - **crear** uno nuevo desde un preset ([`create_from_preset`](KeymapProfiles::create_from_preset)),
//! - **duplicar** uno existente ([`duplicate`](KeymapProfiles::duplicate)),
//! - **borrar** uno ([`remove`](KeymapProfiles::remove)) — salvo los de fábrica.
//!
//! # Cómo llega al compositor
//!
//! El Cuerpo y el `Desktop` siguen consumiendo **un solo** keymap
//! (`~/.config/mirada/keymap.ron`, vigilado y recargado en caliente — ver
//! [`crate::keymap`]). La biblioteca es una capa por encima: vive en
//! `~/.config/mirada/profiles.ron` y, cada vez que cambia el activo, **escribe
//! el keymap del perfil activo en `keymap.ron`**. El `FileWatch` existente lo
//! recarga y reenvía el `GrabKeys` — sin tocar el compositor ni el protocolo.
//!
//! # Presets de fábrica
//!
//! Los tres presets siempre están disponibles: si faltan en el archivo (porque
//! es nuevo o el usuario los borró a mano), se vuelven a sembrar al cargar. No
//! se pueden borrar por la API ([`remove`](KeymapProfiles::remove) los rechaza);
//! el flujo para tunearlos es **duplicar y editar la copia**.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::keymap::{Keymap, KeymapError};

/// La biblioteca de perfiles de atajos: el activo + todos los keymaps por nombre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapProfiles {
    active: String,
    profiles: BTreeMap<String, Keymap>,
}

impl Default for KeymapProfiles {
    /// Todos los presets de fábrica, con `mirada` (el nativo) activo.
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        for name in Keymap::PRESET_NAMES {
            if let Some(km) = Keymap::preset(name) {
                profiles.insert(name.to_string(), km);
            }
        }
        Self {
            active: "mirada".to_string(),
            profiles,
        }
    }
}

impl KeymapProfiles {
    // --- Lectura ------------------------------------------------------

    /// El nombre del perfil activo.
    pub fn active(&self) -> &str {
        &self.active
    }

    /// El keymap del perfil activo. Si por alguna razón el activo no existe
    /// (archivo corrupto editado a mano), cae al preset `dwm`.
    pub fn active_keymap(&self) -> Keymap {
        // El fallback (activo inexistente por edición a mano) es el keymap
        // nativo: `Keymap::default()` == el preset `mirada`.
        self.profiles
            .get(&self.active)
            .cloned()
            .unwrap_or_default()
    }

    /// Los nombres de todos los perfiles, en orden alfabético.
    pub fn names(&self) -> Vec<String> {
        self.profiles.keys().cloned().collect()
    }

    /// Cuántos perfiles hay.
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// `true` si no hay ningún perfil (no debería pasar: siempre hay presets).
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// El keymap de un perfil por nombre.
    pub fn get(&self, name: &str) -> Option<&Keymap> {
        self.profiles.get(name)
    }

    /// `true` si existe un perfil con ese nombre.
    pub fn contains(&self, name: &str) -> bool {
        self.profiles.contains_key(name)
    }

    // --- Edición ------------------------------------------------------

    /// Conmuta el perfil activo. Error si el perfil no existe.
    pub fn set_active(&mut self, name: &str) -> Result<(), ProfileError> {
        if self.profiles.contains_key(name) {
            self.active = name.to_string();
            Ok(())
        } else {
            Err(ProfileError::NotFound(name.to_string()))
        }
    }

    /// Reemplaza el keymap de un perfil existente (lo que guarda el editor del
    /// panel). Error si el perfil no existe.
    pub fn set_keymap(&mut self, name: &str, keymap: Keymap) -> Result<(), ProfileError> {
        if let Some(slot) = self.profiles.get_mut(name) {
            *slot = keymap;
            Ok(())
        } else {
            Err(ProfileError::NotFound(name.to_string()))
        }
    }

    /// Crea un perfil nuevo `name` con el keymap dado. Error si ya existe o el
    /// nombre es vacío.
    pub fn create(&mut self, name: &str, keymap: Keymap) -> Result<(), ProfileError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProfileError::EmptyName);
        }
        if self.profiles.contains_key(name) {
            return Err(ProfileError::AlreadyExists(name.to_string()));
        }
        self.profiles.insert(name.to_string(), keymap);
        Ok(())
    }

    /// Crea un perfil nuevo `name` clonando un **preset de fábrica** (`from`).
    /// Error si `from` no es un preset, si `name` ya existe o es vacío.
    pub fn create_from_preset(&mut self, name: &str, from: &str) -> Result<(), ProfileError> {
        let km = Keymap::preset(from).ok_or_else(|| ProfileError::UnknownPreset(from.to_string()))?;
        self.create(name, km)
    }

    /// Duplica un perfil existente `src` con el nuevo nombre `name`. Error si
    /// `src` no existe, si `name` ya existe o es vacío.
    pub fn duplicate(&mut self, src: &str, name: &str) -> Result<(), ProfileError> {
        let km = self
            .profiles
            .get(src)
            .cloned()
            .ok_or_else(|| ProfileError::NotFound(src.to_string()))?;
        self.create(name, km)
    }

    /// Renombra un perfil. No se pueden renombrar los presets de fábrica. Si se
    /// renombra el activo, el activo pasa al nombre nuevo.
    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), ProfileError> {
        let to = to.trim();
        if to.is_empty() {
            return Err(ProfileError::EmptyName);
        }
        if Keymap::is_builtin_name(from) {
            return Err(ProfileError::BuiltinProtected(from.to_string()));
        }
        if !self.profiles.contains_key(from) {
            return Err(ProfileError::NotFound(from.to_string()));
        }
        if self.profiles.contains_key(to) {
            return Err(ProfileError::AlreadyExists(to.to_string()));
        }
        let km = self.profiles.remove(from).expect("recién comprobado");
        self.profiles.insert(to.to_string(), km);
        if self.active == from {
            self.active = to.to_string();
        }
        Ok(())
    }

    /// Borra un perfil. Los presets de fábrica no se pueden borrar (duplicá y
    /// editá la copia). Si se borra el activo, el activo cae a `dwm`.
    pub fn remove(&mut self, name: &str) -> Result<(), ProfileError> {
        if Keymap::is_builtin_name(name) {
            return Err(ProfileError::BuiltinProtected(name.to_string()));
        }
        if self.profiles.remove(name).is_none() {
            return Err(ProfileError::NotFound(name.to_string()));
        }
        if self.active == name {
            self.active = "mirada".to_string();
        }
        Ok(())
    }

    /// Re-siembra los presets de fábrica que falten — así nunca se pierden,
    /// aunque el archivo se haya editado a mano. Se llama al cargar.
    fn ensure_builtins(&mut self) {
        for name in Keymap::PRESET_NAMES {
            let fresh = Keymap::preset(name).expect("preset de fábrica");
            match self.profiles.get_mut(name) {
                // Preset builtin ya guardado: le fundimos los binds nuevos del
                // preset de fábrica sin pisar los rebinds del usuario. Sin esto
                // un perfil viejo quedaba congelado y no recibía atajos nuevos
                // (el riesgo gemelo del bug de keymap.ron). Los perfiles custom
                // (no-builtin) no se tocan: son del usuario.
                Some(saved) => {
                    saved.merge_from(&fresh);
                }
                None => {
                    self.profiles.insert(name.to_string(), fresh);
                }
            }
        }
        if !self.profiles.contains_key(&self.active) {
            self.active = "mirada".to_string();
        }
    }

    // --- RON ----------------------------------------------------------

    /// Parsea la biblioteca desde el texto RON del archivo de perfiles.
    pub fn from_ron(text: &str) -> Result<KeymapProfiles, KeymapError> {
        let file: ProfilesFile = ron::from_str(text)
            .map_err(|e| KeymapError::Parse(format!("RON de perfiles inválido: {e}")))?;
        let mut profiles = BTreeMap::new();
        for (name, map) in file.profiles {
            profiles.insert(name, Keymap::from_string_map(&map));
        }
        let mut me = KeymapProfiles {
            active: file.active,
            profiles,
        };
        me.ensure_builtins();
        Ok(me)
    }

    /// Serializa la biblioteca a RON (sin la cabecera de documentación).
    pub fn to_ron(&self) -> String {
        let file = ProfilesFile {
            active: self.active.clone(),
            profiles: self
                .profiles
                .iter()
                .map(|(name, km)| (name.clone(), km.to_string_map()))
                .collect(),
        };
        ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())
            .expect("un ProfilesFile de cadenas siempre serializa")
    }

    /// La biblioteca como RON con cabecera de documentación — lo que escribe
    /// [`save`](KeymapProfiles::save).
    pub fn documented_ron(&self) -> String {
        format!("{PROFILES_HEADER}\n{}", self.to_ron())
    }

    // --- Disco --------------------------------------------------------

    /// La ruta canónica de la biblioteca: `~/.config/mirada/profiles.ron`.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "mirada")
            .map(|d| d.config_dir().join("profiles.ron"))
    }

    /// La ruta canónica del keymap activo: `~/.config/mirada/keymap.ron`.
    pub fn keymap_path() -> Option<PathBuf> {
        Keymap::default_path()
    }

    /// Carga la biblioteca desde un archivo RON.
    pub fn load(path: &Path) -> Result<KeymapProfiles, KeymapError> {
        let text = std::fs::read_to_string(path)?;
        KeymapProfiles::from_ron(&text)
    }

    /// Escribe la biblioteca a `path` como RON documentado, creando el
    /// directorio padre si falta.
    pub fn save(&self, path: &Path) -> Result<(), KeymapError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.documented_ron())?;
        Ok(())
    }

    /// Escribe el keymap del perfil **activo** en `keymap.ron` — el archivo que
    /// el compositor vigila y recarga en caliente. Llamar tras conmutar o editar
    /// el perfil activo. Crea el directorio padre si falta.
    pub fn write_active_keymap(&self, keymap_path: &Path) -> Result<(), KeymapError> {
        self.active_keymap().save(keymap_path)
    }

    /// Carga la biblioteca del usuario con un fallback amable, espejo de
    /// [`Keymap::load_or_init`]: si el archivo no existe lo crea con los presets;
    /// si está corrupto avisa por `stderr` y usa los de por defecto sin pisarlo.
    pub fn load_or_init(path: &Path) -> KeymapProfiles {
        if path.exists() {
            match KeymapProfiles::load(path) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!(
                        "mirada · perfiles «{}» inválidos ({e}); uso los de fábrica.",
                        path.display()
                    );
                    KeymapProfiles::default()
                }
            }
        } else {
            let p = KeymapProfiles::default();
            match p.save(path) {
                Ok(()) => eprintln!("mirada · biblioteca de perfiles inicial en {}", path.display()),
                Err(e) => eprintln!("mirada · no pude escribir los perfiles iniciales: {e}"),
            }
            p
        }
    }
}

/// La forma en disco de la biblioteca — el activo y, por perfil, su mapa de
/// cadenas `combinación → acción` (las acciones van como texto, igual que en
/// [`crate::keymap`], para un RON trivial y errores tolerados entrada a entrada).
#[derive(Serialize, Deserialize)]
struct ProfilesFile {
    active: String,
    profiles: BTreeMap<String, BTreeMap<String, String>>,
}

/// La cabecera de comentarios del archivo de perfiles.
const PROFILES_HEADER: &str = "\
// perfiles de atajos de mirada — la biblioteca de keymaps conmutables.
//
//   active:    el perfil en uso (su keymap se vuelca a keymap.ron).
//   profiles:  nombre → { \"Combinación\": \"acción\" }.
//
// Presets de fábrica (dwm · i3 · hyprland): siempre presentes, no se borran;
// para tunearlos, duplicalos y editá la copia. Gestión por CLI:
//
//   mirada-ctl profile list             lista los perfiles (* = activo)
//   mirada-ctl profile use <nombre>     conmuta el activo (recarga en caliente)
//   mirada-ctl profile new <nombre> [from <preset>]   crea desde un preset
//   mirada-ctl profile dup <origen> <nombre>          duplica uno existente
//   mirada-ctl profile rm <nombre>      borra un perfil propio
//
// El vocabulario de acciones es el mismo del keymap: mirada-ctl actions.";

/// Un fallo al operar sobre la biblioteca de perfiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// No existe un perfil con ese nombre.
    NotFound(String),
    /// Ya existe un perfil con ese nombre.
    AlreadyExists(String),
    /// El nombre del perfil está vacío.
    EmptyName,
    /// El preset de fábrica pedido no existe (`dwm` · `i3` · `hyprland`).
    UnknownPreset(String),
    /// Operación no permitida sobre un preset de fábrica (borrar/renombrar).
    BuiltinProtected(String),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::NotFound(n) => write!(f, "no existe el perfil «{n}»"),
            ProfileError::AlreadyExists(n) => write!(f, "ya existe el perfil «{n}»"),
            ProfileError::EmptyName => f.write_str("el nombre del perfil no puede ser vacío"),
            ProfileError::UnknownPreset(n) => write!(
                f,
                "preset desconocido «{n}» (de fábrica: dwm · i3 · hyprland)"
            ),
            ProfileError::BuiltinProtected(n) => {
                write!(f, "«{n}» es un preset de fábrica; duplicalo para editarlo")
            }
        }
    }
}

impl std::error::Error for ProfileError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_trae_los_presets_con_mirada_activo() {
        let p = KeymapProfiles::default();
        assert_eq!(p.active(), "mirada"); // el nativo es el default
        // BTreeMap → alfabético; están todos los de fábrica.
        assert_eq!(
            p.names(),
            vec!["dwm", "hyprland", "i3", "mac", "mirada", "windows"]
        );
        assert!(p.get("hyprland").is_some());
    }

    #[test]
    fn switch_a_un_perfil_inexistente_falla() {
        let mut p = KeymapProfiles::default();
        assert!(p.set_active("noexiste").is_err());
        assert!(p.set_active("i3").is_ok());
        assert_eq!(p.active(), "i3");
    }

    #[test]
    fn duplicar_crea_una_copia_editable_independiente() {
        let mut p = KeymapProfiles::default();
        p.duplicate("i3", "mío").unwrap();
        assert!(p.contains("mío"));
        // Editar la copia no toca el original.
        let mut km = p.get("mío").unwrap().clone();
        km = Keymap::from_pairs(km.bindings().iter().map(|(k, v)| (k.clone(), v.clone())));
        p.set_keymap("mío", km).unwrap();
        assert_eq!(p.get("i3"), Some(&Keymap::i3()));
    }

    #[test]
    fn crear_desde_preset_y_nombre_repetido_falla() {
        let mut p = KeymapProfiles::default();
        p.create_from_preset("trabajo", "hyprland").unwrap();
        assert_eq!(p.get("trabajo"), Some(&Keymap::hyprland()));
        assert!(p.create_from_preset("trabajo", "i3").is_err()); // ya existe
        assert!(p.create_from_preset("otro", "noexiste").is_err()); // preset malo
        assert!(p.create("", Keymap::dwm()).is_err()); // nombre vacío
    }

    #[test]
    fn preset_builtin_guardado_recibe_binds_nuevos_sin_pisar() {
        // Simula un preset i3 «viejo» guardado en disco: le falta un bind de
        // fábrica (como si lo hubiéramos agregado después) y trae un combo
        // propio del usuario. El round-trip por RON dispara ensure_builtins.
        let fresco = Keymap::i3();
        let full: Vec<_> = fresco
            .bindings()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        assert!(full.len() >= 2, "i3 debería tener varios binds");
        let (combo_perdido, accion) = full[0].clone();
        let mut recortado: Vec<_> = full[1..].to_vec();
        recortado.push(("Ctrl+Alt+Super+Q".to_string(), accion)); // combo propio
        let viejo = Keymap::from_pairs(recortado);

        let mut p = KeymapProfiles::default();
        p.set_keymap("i3", viejo).unwrap();

        let reparsed = KeymapProfiles::from_ron(&p.to_ron()).unwrap();
        let i3 = reparsed.get("i3").unwrap();
        // recuperó el bind de fábrica que faltaba…
        assert!(
            i3.bindings().contains_key(&combo_perdido),
            "el merge debe recuperar el bind de fábrica nuevo"
        );
        // …sin borrar el combo propio del usuario.
        assert!(
            i3.bindings().contains_key("Ctrl+Alt+Super+Q"),
            "el merge no debe pisar/borrar los rebinds del usuario"
        );
    }

    #[test]
    fn no_se_puede_borrar_un_preset_de_fabrica() {
        let mut p = KeymapProfiles::default();
        assert!(matches!(
            p.remove("i3"),
            Err(ProfileError::BuiltinProtected(_))
        ));
        assert!(p.contains("i3"));
    }

    #[test]
    fn borrar_el_activo_cae_a_mirada() {
        let mut p = KeymapProfiles::default();
        p.duplicate("hyprland", "noche").unwrap();
        p.set_active("noche").unwrap();
        p.remove("noche").unwrap();
        assert!(!p.contains("noche"));
        assert_eq!(p.active(), "mirada");
    }

    #[test]
    fn renombrar_respeta_los_presets_y_sigue_al_activo() {
        let mut p = KeymapProfiles::default();
        assert!(p.rename("dwm", "x").is_err()); // de fábrica
        p.duplicate("i3", "a").unwrap();
        p.set_active("a").unwrap();
        p.rename("a", "b").unwrap();
        assert!(p.contains("b") && !p.contains("a"));
        assert_eq!(p.active(), "b");
    }

    #[test]
    fn round_trip_por_ron_preserva_activo_y_perfiles() {
        let mut p = KeymapProfiles::default();
        p.duplicate("hyprland", "custom").unwrap();
        p.set_active("custom").unwrap();
        let back = KeymapProfiles::from_ron(&p.to_ron()).unwrap();
        assert_eq!(back.active(), "custom");
        assert_eq!(back, p);
    }

    #[test]
    fn from_ron_resiembra_presets_faltantes() {
        // Un archivo que sólo trae un perfil propio: los de fábrica reaparecen.
        let ron = r#"(active: "solo", profiles: { "solo": { "Super+q": "close-focused" } })"#;
        let p = KeymapProfiles::from_ron(ron).unwrap();
        assert!(p.contains("dwm") && p.contains("i3") && p.contains("hyprland"));
        assert!(p.contains("solo"));
        assert_eq!(p.active(), "solo");
    }

    #[test]
    fn from_ron_con_activo_invalido_cae_a_mirada() {
        let ron = r#"(active: "fantasma", profiles: {})"#;
        let p = KeymapProfiles::from_ron(ron).unwrap();
        assert_eq!(p.active(), "mirada");
    }

    #[test]
    fn write_active_keymap_vuelca_el_perfil_activo() {
        let dir = std::env::temp_dir().join(format!("mirada-prof-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let kpath = dir.join("keymap.ron");
        let mut p = KeymapProfiles::default();
        p.set_active("hyprland").unwrap();
        p.write_active_keymap(&kpath).unwrap();
        let km = Keymap::load(&kpath).unwrap();
        assert_eq!(km, Keymap::hyprland());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
