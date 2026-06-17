//! Biblioteca de **themes** — el *look* reusable del escritorio: apariencia
//! (variante + acento) + teselado + decoración. Es **perpendicular** a los
//! perfiles: un [`crate::perfiles::DesktopProfile`] REFERENCIA un theme por
//! nombre (no guarda su propio teselado/decoración), y el theme define cómo se
//! ve. Así un mismo look se reusa entre perfiles y editarlo afecta a todos los
//! que lo referencian.
//!
//! Vive en `~/.config/mirada/themes.ron`; se siembra la primera vez con un theme
//! por vista de fábrica (el óptimo para su perfil homónimo).

use std::collections::BTreeMap;
use std::path::PathBuf;

use mirada_brain::{Config as MiradaConfig, LayoutMode};
use serde::{Deserialize, Serialize};

/// El look reusable: lo que un theme «absorbe» del escritorio.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Theme {
    // --- Apariencia (toolkit / WawaConfig) ---
    #[serde(default)]
    pub theme_variant: String,
    #[serde(default)]
    pub accent: String,
    // --- Teselado (mirada) ---
    // `LayoutMode` no es `Default`; siempre lo escribimos, así que no lleva
    // `serde(default)` (un themes.ron sin `layout` se rechaza, no debería pasar).
    pub layout: LayoutMode,
    #[serde(default)]
    pub gap: i32,
    #[serde(default)]
    pub master_ratio: f32,
    #[serde(default)]
    pub master_count: usize,
    #[serde(default)]
    pub master_step: f32,
    // --- Decoración (mirada) ---
    #[serde(default)]
    pub border_width: i32,
    #[serde(default)]
    pub border_focus: [u8; 4],
    #[serde(default)]
    pub border_normal: [u8; 4],
    #[serde(default)]
    pub titlebar_height: i32,
}

impl Theme {
    /// Extrae el look de una config mirada + la apariencia (variante/acento).
    pub fn from_config(m: &MiradaConfig, variant: &str, accent: &str) -> Self {
        Self {
            theme_variant: variant.to_string(),
            accent: accent.to_string(),
            layout: m.layout.clone(),
            gap: m.gap,
            master_ratio: m.master_ratio,
            master_count: m.master_count,
            master_step: m.master_step,
            border_width: m.border_width,
            border_focus: m.border_focus,
            border_normal: m.border_normal,
            titlebar_height: m.titlebar_height,
        }
    }

    /// Vuelca el **teselado + decoración** del theme sobre una config mirada
    /// (la apariencia variante/acento va aparte, a WawaConfig).
    pub fn apply_to(&self, m: &mut MiradaConfig) {
        m.layout = self.layout.clone();
        m.gap = self.gap;
        m.master_ratio = self.master_ratio;
        m.master_count = self.master_count;
        m.master_step = self.master_step;
        m.border_width = self.border_width;
        m.border_focus = self.border_focus;
        m.border_normal = self.border_normal;
        m.titlebar_height = self.titlebar_height;
    }
}

/// La biblioteca de themes, serializable a RON.
#[derive(Default, Serialize, Deserialize)]
pub struct Themes {
    pub themes: BTreeMap<String, Theme>,
}

impl Themes {
    /// `~/.config/mirada/themes.ron`.
    pub fn path() -> Option<PathBuf> {
        let cfg = MiradaConfig::default_path()?;
        cfg.parent().map(|d| d.join("themes.ron"))
    }

    /// Carga la biblioteca; si no existe (o está vacía) la siembra con un theme
    /// por vista de fábrica — el look óptimo para su perfil homónimo.
    pub fn load_or_seed(variant: &str, accent: &str) -> Self {
        if let Some(p) = Self::path() {
            if let Ok(txt) = std::fs::read_to_string(&p) {
                if let Ok(lib) = ron::from_str::<Themes>(&txt) {
                    if !lib.themes.is_empty() {
                        return lib;
                    }
                }
            }
        }
        let mut lib = Themes::default();
        for name in mirada_brain::VISTA_NAMES {
            if let Some(v) = mirada_brain::Vista::by_name(name) {
                lib.themes
                    .insert(name.to_string(), Theme::from_config(&v.config, variant, accent));
            }
        }
        let _ = lib.save();
        lib
    }

    /// El theme `name`, o el nativo «mirada», o el primero.
    pub fn get_or_default(&self, name: &str) -> Option<&Theme> {
        self.themes
            .get(name)
            .or_else(|| self.themes.get("mirada"))
            .or_else(|| self.themes.values().next())
    }

    pub fn get(&self, name: &str) -> Option<&Theme> {
        self.themes.get(name)
    }

    pub fn names(&self) -> Vec<String> {
        self.themes.keys().cloned().collect()
    }

    pub fn set(&mut self, name: &str, theme: Theme) {
        self.themes.insert(name.to_string(), theme);
    }

    /// Crea un theme con nombre único desde `hint`.
    pub fn create(&mut self, base: Theme, hint: &str) -> String {
        let name = self.unique_name(hint);
        self.themes.insert(name.clone(), base);
        name
    }

    pub fn duplicate(&mut self, src: &str) -> Option<String> {
        let t = self.themes.get(src)?.clone();
        let name = self.unique_name(&format!("{src} copia"));
        self.themes.insert(name.clone(), t);
        Some(name)
    }

    /// Renombra un theme. Devuelve `true` si existía y no chocó el destino.
    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        let to = to.trim();
        if to.is_empty() || self.themes.contains_key(to) || !self.themes.contains_key(from) {
            return false;
        }
        if let Some(t) = self.themes.remove(from) {
            self.themes.insert(to.to_string(), t);
            return true;
        }
        false
    }

    pub fn remove(&mut self, name: &str) {
        self.themes.remove(name);
    }

    fn unique_name(&self, hint: &str) -> String {
        let base = if hint.trim().is_empty() { "theme" } else { hint.trim() };
        if !self.themes.contains_key(base) {
            return base.to_string();
        }
        for n in 2.. {
            let cand = format!("{base} {n}");
            if !self.themes.contains_key(&cand) {
                return cand;
            }
        }
        base.to_string()
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
