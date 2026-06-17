//! Biblioteca de **conjuntos de animación** — el comportamiento animado del
//! escritorio (transición de Win+Tab, duración del slide, vuelo de cámara del
//! Prezi) como un set reusable, perpendicular a los perfiles. Mismo patrón que
//! [`crate::themes`]: un [`crate::perfiles::DesktopProfile`] referencia un
//! conjunto por nombre (`animation_set`) y lo aplica al activarse.
//!
//! Vive en `~/.config/mirada/animaciones.ron`; se siembra la primera vez con
//! unos presets de fábrica.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mirada_brain::{Config as MiradaConfig, WorkspaceSwitchMode};
use serde::{Deserialize, Serialize};

/// El comportamiento animado reusable que un conjunto «absorbe» del escritorio.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Animation {
    /// Modo de transición de Win+Tab entre escritorios.
    #[serde(default)]
    pub switch_mode: WorkspaceSwitchMode,
    /// Duración del slide entre escritorios (ms). `0` = salto seco.
    #[serde(default)]
    pub slide_ms: u32,
    /// Vuelo de cámara al abrir/aterrizar la vista espacial (Prezi), en ms.
    #[serde(default)]
    pub overview_anim_ms: u32,
}

impl Animation {
    /// Extrae el comportamiento animado de una config mirada.
    pub fn from_config(m: &MiradaConfig) -> Self {
        Self {
            switch_mode: m.workspace_switch_mode,
            slide_ms: m.slide_ms,
            overview_anim_ms: m.overview_anim_ms,
        }
    }

    /// Vuelca el conjunto sobre una config mirada.
    pub fn apply_to(&self, m: &mut MiradaConfig) {
        m.workspace_switch_mode = self.switch_mode;
        m.slide_ms = self.slide_ms;
        m.overview_anim_ms = self.overview_anim_ms;
    }
}

/// La biblioteca de conjuntos de animación, serializable a RON.
#[derive(Default, Serialize, Deserialize)]
pub struct Animations {
    pub animations: BTreeMap<String, Animation>,
    /// Nombre del conjunto activo (el que se edita/aplica).
    #[serde(default)]
    pub active: String,
}

impl Animations {
    /// `~/.config/mirada/animaciones.ron`.
    pub fn path() -> Option<PathBuf> {
        let cfg = MiradaConfig::default_path()?;
        cfg.parent().map(|d| d.join("animaciones.ron"))
    }

    /// Carga la biblioteca; si no existe (o está vacía) la siembra con presets.
    pub fn load_or_seed() -> Self {
        if let Some(p) = Self::path() {
            if let Ok(txt) = std::fs::read_to_string(&p) {
                if let Ok(mut lib) = ron::from_str::<Animations>(&txt) {
                    if !lib.animations.is_empty() {
                        if lib.active.is_empty() || !lib.animations.contains_key(&lib.active) {
                            lib.active = lib.animations.keys().next().cloned().unwrap_or_default();
                        }
                        return lib;
                    }
                }
            }
        }
        let mut lib = Animations::default();
        let h = WorkspaceSwitchMode::Hyprland;
        lib.animations.insert("fluido".into(), Animation { switch_mode: h, slide_ms: 220, overview_anim_ms: 260 });
        lib.animations.insert("rápido".into(), Animation { switch_mode: h, slide_ms: 120, overview_anim_ms: 140 });
        lib.animations.insert(
            "sin animación".into(),
            Animation { switch_mode: WorkspaceSwitchMode::Direct, slide_ms: 0, overview_anim_ms: 0 },
        );
        lib.animations.insert(
            "prezi".into(),
            Animation { switch_mode: WorkspaceSwitchMode::Prezi, slide_ms: 220, overview_anim_ms: 320 },
        );
        lib.active = "fluido".into();
        let _ = lib.save();
        lib
    }

    pub fn active(&self) -> &str {
        &self.active
    }

    /// El conjunto activo, o el primero, o uno por defecto.
    pub fn active_animation(&self) -> Animation {
        self.animations
            .get(&self.active)
            .or_else(|| self.animations.values().next())
            .cloned()
            .unwrap_or_else(|| Animation::from_config(&MiradaConfig::default()))
    }

    pub fn get(&self, name: &str) -> Option<&Animation> {
        self.animations.get(name)
    }

    pub fn names(&self) -> Vec<String> {
        self.animations.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.animations.len()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.animations.contains_key(name)
    }

    pub fn set_active(&mut self, name: &str) -> bool {
        if self.animations.contains_key(name) {
            self.active = name.to_string();
            true
        } else {
            false
        }
    }

    pub fn set(&mut self, name: &str, anim: Animation) {
        self.animations.insert(name.to_string(), anim);
    }

    /// Crea un conjunto con nombre único desde `hint`. Devuelve el nombre.
    pub fn create(&mut self, base: Animation, hint: &str) -> String {
        let name = self.unique_name(hint);
        self.animations.insert(name.clone(), base);
        name
    }

    pub fn duplicate(&mut self, src: &str) -> Option<String> {
        let a = self.animations.get(src)?.clone();
        let name = self.unique_name(&format!("{src} copia"));
        self.animations.insert(name.clone(), a);
        Some(name)
    }

    /// Renombra un conjunto. `true` si existía y no chocó el destino.
    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        let to = to.trim();
        if to.is_empty() || self.animations.contains_key(to) || !self.animations.contains_key(from) {
            return false;
        }
        if let Some(a) = self.animations.remove(from) {
            self.animations.insert(to.to_string(), a);
            if self.active == from {
                self.active = to.to_string();
            }
            return true;
        }
        false
    }

    pub fn remove(&mut self, name: &str) {
        self.animations.remove(name);
        if self.active == name {
            self.active = self.animations.keys().next().cloned().unwrap_or_default();
        }
    }

    fn unique_name(&self, hint: &str) -> String {
        let base = if hint.trim().is_empty() { "animación" } else { hint.trim() };
        if !self.animations.contains_key(base) {
            return base.to_string();
        }
        (2..).map(|n| format!("{base} {n}")).find(|c| !self.animations.contains_key(c)).unwrap()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = Self::path() else {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "sin HOME"));
        };
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let cfg = ron::ser::PrettyConfig::new().depth_limit(4);
        let txt = ron::ser::to_string_pretty(self, cfg)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(&p, txt)
    }
}
