//! Biblioteca de **perfiles de escritorio**.
//!
//! Un perfil no es sólo un keymap ni sólo una vista de fábrica: es una *foto
//! completa* del escritorio — la config de mirada (teselado, decoración,
//! fondo, zonas…), el keymap (filas de atajos) y la barra `pata`. La biblioteca
//! vive en un único RON (`~/.config/mirada/perfiles-escritorio.ron`) y se
//! siembra la primera vez con las 8 vistas de fábrica, ya **editables**.
//!
//! - **Activar** un perfil escribe su contenido a las rutas vivas
//!   (`config.ron` / `keymap.ron` / `launcher.toml`) → el compositor y la barra
//!   recargan en caliente.
//! - **Editar** el perfil activo (desde las pestañas mirada/pata/atajos) guarda
//!   de vuelta en su entrada de la biblioteca, así cada perfil conserva lo suyo.
//! - **Crear** parte de la config viva; **duplicar** copia un perfil existente.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mirada_brain::{Config as MiradaConfig, Keymap};
use pata_core::Config as PataConfig;
use serde::{Deserialize, Serialize};

/// Una foto completa del escritorio.
#[derive(Clone, Serialize, Deserialize)]
pub struct DesktopProfile {
    pub mirada: MiradaConfig,
    /// Keymap como filas `[combo, accion, args…]` (lo que consume la tabla).
    pub keymap: Vec<Vec<String>>,
    pub pata: PataConfig,
    /// **Theme referenciado** (nombre en la biblioteca de themes). El perfil ya
    /// NO es dueño de su teselado/decoración: los toma del theme al activarse.
    /// Vacío = el theme nativo. `serde(default)` para cargar RON viejos.
    #[serde(default)]
    pub theme: String,
    /// **Conjunto de atajos referenciado** (nombre en la biblioteca de keymaps,
    /// `profiles.ron`). Mismo patrón que [`Self::theme`]: el perfil USA un
    /// conjunto reusable en vez de tener el suyo embebido. Vacío = usa el campo
    /// `keymap` embebido (RON viejo) o el conjunto activo.
    #[serde(default)]
    pub keymap_set: String,
    /// **Conjunto de animación referenciado** (nombre en `animaciones.ron`).
    /// Mismo patrón que [`Self::theme`]/[`Self::keymap_set`]. Vacío = usa lo que
    /// haya en la config (sin conjunto).
    #[serde(default)]
    pub animation_set: String,
}

impl DesktopProfile {
    /// Construye un perfil desde una vista de fábrica. Su theme por defecto es el
    /// homónimo de la vista (sembrado en la biblioteca de themes).
    fn from_vista(name: &str) -> Option<Self> {
        let v = mirada_brain::Vista::by_name(name)?;
        let keymap = mirada_brain::preset_keymap(v.keymap)
            .map(|pairs| Keymap::from_pairs(pairs).to_rows())
            .unwrap_or_default();
        let pata = PataConfig::vista_preset(name).unwrap_or_default();
        Some(Self {
            mirada: v.config.clone(),
            keymap,
            pata,
            theme: name.to_string(),
            keymap_set: String::new(),
            animation_set: String::new(),
        })
    }
}

/// La biblioteca completa, serializable a un RON.
#[derive(Default, Serialize, Deserialize)]
pub struct DesktopProfiles {
    /// Nombre del perfil activo (vacío si ninguno).
    pub active: String,
    pub profiles: BTreeMap<String, DesktopProfile>,
}

impl DesktopProfiles {
    /// `~/.config/mirada/perfiles-escritorio.ron`, derivado del directorio
    /// donde mirada guarda su `config.ron`.
    pub fn path() -> Option<PathBuf> {
        let cfg = MiradaConfig::default_path()?;
        cfg.parent().map(|d| d.join("perfiles-escritorio.ron"))
    }

    /// Carga la biblioteca; si no existe (o está vacía) la siembra con las 8
    /// vistas de fábrica y marca como activa la que coincide con la config viva.
    pub fn load_or_seed(live: &MiradaConfig) -> Self {
        if let Some(p) = Self::path() {
            if let Ok(txt) = std::fs::read_to_string(&p) {
                if let Ok(mut lib) = ron::from_str::<DesktopProfiles>(&txt) {
                    if !lib.profiles.is_empty() {
                        lib.repair_active();
                        // Re-derivar el activo de la config VIVA: la vista cuya
                        // config coincide con la del compositor es la que está
                        // realmente aplicada. Si se cambió de vista por otra vía
                        // (mirada-ctl, menú de mirada) que no tocó este RON, el
                        // `active` guardado quedaba colgado y el panel mostraba
                        // «mirada» aunque estuvieras en otra. Si ninguna coincide
                        // (config personalizada), se respeta el activo guardado.
                        if let Some(n) = lib
                            .profiles
                            .iter()
                            .find(|(_, p)| &p.mirada == live)
                            .map(|(n, _)| n.clone())
                        {
                            if lib.active != n {
                                lib.active = n;
                                let _ = lib.save();
                            }
                        }
                        return lib;
                    }
                }
            }
        }
        let mut lib = DesktopProfiles::default();
        for name in mirada_brain::VISTA_NAMES {
            if let Some(prof) = DesktopProfile::from_vista(name) {
                lib.profiles.insert(name.to_string(), prof);
            }
        }
        // Activo = la vista cuya config coincide con la viva; si ninguna
        // coincide, el **nativo** («mirada»). OJO: NO el primero del BTreeMap,
        // que por orden alfabético es «dwm» — esa era la causa de que el
        // escritorio arrancara en dwm sin que nadie lo pidiera.
        lib.active = lib
            .profiles
            .iter()
            .find(|(_, p)| &p.mirada == live)
            .map(|(n, _)| n.clone())
            .or_else(|| lib.default_name())
            .unwrap_or_default();
        let _ = lib.save();
        lib
    }

    /// El perfil por defecto: el nativo «mirada» si existe; si no, el primero
    /// disponible. Nunca el alfabético a ciegas (sería «dwm»).
    fn default_name(&self) -> Option<String> {
        if self.profiles.contains_key("mirada") {
            Some("mirada".to_string())
        } else {
            self.profiles.keys().next().cloned()
        }
    }

    /// Si el activo apunta a un perfil inexistente, lo reapunta al nativo.
    fn repair_active(&mut self) {
        if !self.profiles.contains_key(&self.active) {
            self.active = self.default_name().unwrap_or_default();
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = Self::path() else {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "sin HOME"));
        };
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let cfg = ron::ser::PrettyConfig::new().depth_limit(6);
        let txt = ron::ser::to_string_pretty(self, cfg)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(&p, txt)
    }

    /// Nombres ordenados (BTreeMap ya viene ordenado).
    pub fn names(&self) -> Vec<String> {
        self.profiles.keys().cloned().collect()
    }

    pub fn get(&self, name: &str) -> Option<&DesktopProfile> {
        self.profiles.get(name)
    }

    /// Sobrescribe (o crea) la entrada de un perfil con la foto dada.
    pub fn set(&mut self, name: &str, prof: DesktopProfile) {
        self.profiles.insert(name.to_string(), prof);
    }

    /// Renombra un perfil; mueve el «activo» si era ése. `false` si el origen no
    /// existe o el destino está vacío/ocupado.
    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        let to = to.trim();
        if to.is_empty() || self.profiles.contains_key(to) || !self.profiles.contains_key(from) {
            return false;
        }
        if let Some(p) = self.profiles.remove(from) {
            self.profiles.insert(to.to_string(), p);
            if self.active == from {
                self.active = to.to_string();
            }
            return true;
        }
        false
    }

    /// Re-siembra los perfiles de fábrica (vistas) que falten — por si se
    /// renombraron o borraron. Devuelve cuántos agregó.
    pub fn ensure_defaults(&mut self) -> usize {
        let mut added = 0;
        for name in mirada_brain::VISTA_NAMES {
            if !self.profiles.contains_key(name) {
                if let Some(prof) = DesktopProfile::from_vista(name) {
                    self.profiles.insert(name.to_string(), prof);
                    added += 1;
                }
            }
        }
        added
    }

    /// Crea un perfil nuevo desde una foto base; nombre único si choca.
    pub fn create(&mut self, base: DesktopProfile, hint: &str) -> String {
        let name = self.unique_name(hint);
        self.profiles.insert(name.clone(), base);
        name
    }

    /// Duplica un perfil existente bajo un nombre nuevo. Devuelve el nombre.
    pub fn duplicate(&mut self, src: &str) -> Option<String> {
        let prof = self.profiles.get(src)?.clone();
        let name = self.unique_name(&format!("{src} copia"));
        self.profiles.insert(name.clone(), prof);
        Some(name)
    }

    pub fn remove(&mut self, name: &str) {
        self.profiles.remove(name);
        self.repair_active();
    }

    fn unique_name(&self, hint: &str) -> String {
        let base = if hint.trim().is_empty() { "perfil" } else { hint.trim() };
        if !self.profiles.contains_key(base) {
            return base.to_string();
        }
        for i in 2.. {
            let cand = format!("{base} {i}");
            if !self.profiles.contains_key(&cand) {
                return cand;
            }
        }
        base.to_string()
    }
}
