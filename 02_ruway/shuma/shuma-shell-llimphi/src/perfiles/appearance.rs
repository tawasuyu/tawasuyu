//! Perfiles de **apariencia** — coloración estilo konsole conmutable.
//!
//! Cada perfil es una foto de aspecto: tema base (preset de `llimphi-theme`),
//! override de acento, zoom de fuente, opacidad de fondo (transparencia) y
//! wallpaper opcional. Se aplican **por ventana de shuma**: hay un activo
//! global (el default de toda ventana nueva) y cada **sesión** puede fijar el
//! suyo ([`crate::types::SessionConfig::appearance`]), que gana cuando esa
//! sesión está activa.
//!
//! El perfil especial **`Sistema`** sigue el tema de `wawa-config` (el
//! comportamiento histórico): mientras esté activo, los cambios de tema del
//! sistema se propagan a shuma. Cualquier otro perfil fija el aspecto y deja de
//! seguir a wawa.
//!
//! Persistencia: `~/.config/shuma/appearance.ron`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use llimphi_theme::{Color, Theme};
use serde::{Deserialize, Serialize};

/// El nombre del perfil que sigue el tema del sistema (wawa).
pub const SYSTEM_NAME: &str = "Sistema";

/// Una foto de apariencia.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    /// Nombre del preset de `llimphi-theme` (`Theme::by_name`). Para `Sistema`
    /// se ignora (se usa el tema de wawa).
    pub theme: String,
    /// Override de acento RGBA; `None` deja el del tema.
    #[serde(default)]
    pub accent: Option<[u8; 4]>,
    /// Zoom de fuente por defecto de los shells de esta apariencia.
    #[serde(default = "one")]
    pub font_zoom: f32,
    /// Opacidad del fondo de ventana (0.0 transparente … 1.0 opaco).
    #[serde(default = "one")]
    pub opacity: f32,
    /// Ruta a una imagen de wallpaper, opcional.
    #[serde(default)]
    pub wallpaper: Option<String>,
}

fn one() -> f32 {
    1.0
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: "Dark".to_string(),
            accent: None,
            font_zoom: 1.0,
            opacity: 1.0,
            wallpaper: None,
        }
    }
}

impl Appearance {
    /// Resuelve esta apariencia a un [`Theme`] concreto: parte del preset
    /// nombrado, aplica el acento y la opacidad de fondo. Para `Sistema` use
    /// [`super::apply_active_appearance`] (necesita wawa); acá cae a `Dark`.
    pub fn resolve(&self) -> Theme {
        let mut t = Theme::by_name(&self.theme).unwrap_or_else(Theme::dark);
        if let Some([r, g, b, a]) = self.accent {
            t.accent = Color::from_rgba8(r, g, b, a);
            t.border_focus = Color::from_rgba8(r, g, b, a);
        }
        // Con wallpaper, el fondo debe dejarlo ver: si el perfil quedó opaco,
        // forzamos una translucidez mínima para que la imagen asome.
        let has_wp = self
            .wallpaper
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let eff_op = if has_wp { self.opacity.min(0.7) } else { self.opacity };
        if eff_op < 1.0 {
            t.bg_app = with_opacity(t.bg_app, eff_op);
            t.bg_panel = with_opacity(t.bg_panel, eff_op);
            t.bg_panel_alt = with_opacity(t.bg_panel_alt, eff_op);
        }
        t
    }
}

/// Devuelve `c` con la opacidad pedida (sustituye el canal alfa).
fn with_opacity(c: Color, op: f32) -> Color {
    let k = c.components;
    Color::from_rgba8(
        (k[0] * 255.0).round() as u8,
        (k[1] * 255.0).round() as u8,
        (k[2] * 255.0).round() as u8,
        (op.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

/// Los nombres de los presets de fábrica, en orden de presentación.
pub const PRESET_NAMES: &[&str] = &[
    SYSTEM_NAME,
    "Oscuro",
    "Claro",
    "Tawa",
    "Aurora",
    "Atardecer",
    "Translúcido",
];

/// `true` si `name` es un preset de fábrica.
pub fn is_builtin(name: &str) -> bool {
    PRESET_NAMES.contains(&name)
}

/// La apariencia de un preset de fábrica por nombre. `Sistema` devuelve un
/// placeholder (su tema lo provee wawa); el resto mapea a presets de
/// `llimphi-theme` con la opacidad indicada.
pub fn preset(name: &str) -> Option<Appearance> {
    let ap = |theme: &str, opacity: f32| Appearance {
        theme: theme.to_string(),
        accent: None,
        font_zoom: 1.0,
        opacity,
        wallpaper: None,
    };
    Some(match name {
        SYSTEM_NAME => Appearance {
            theme: "Dark".to_string(),
            ..Appearance::default()
        },
        "Oscuro" => ap("Dark", 1.0),
        "Claro" => ap("Light", 1.0),
        "Tawa" => ap("Tawa", 1.0),
        "Aurora" => ap("Aurora", 1.0),
        "Atardecer" => ap("Sunset", 1.0),
        "Translúcido" => ap("Dark", 0.85),
        _ => return None,
    })
}

/// La biblioteca de perfiles de apariencia: el activo (default global) + todos
/// por nombre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppearanceProfiles {
    active: String,
    profiles: BTreeMap<String, Appearance>,
}

impl Default for AppearanceProfiles {
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        for name in PRESET_NAMES {
            if let Some(a) = preset(name) {
                profiles.insert((*name).to_string(), a);
            }
        }
        Self {
            active: SYSTEM_NAME.to_string(),
            profiles,
        }
    }
}

impl AppearanceProfiles {
    /// El nombre del perfil activo (default global).
    pub fn active(&self) -> &str {
        &self.active
    }

    /// La apariencia de un perfil por nombre.
    pub fn get(&self, name: &str) -> Option<&Appearance> {
        self.profiles.get(name)
    }

    /// La apariencia del activo.
    pub fn active_appearance(&self) -> Appearance {
        self.profiles
            .get(&self.active)
            .cloned()
            .unwrap_or_default()
    }

    /// Los nombres de todos los perfiles, en orden alfabético.
    pub fn names(&self) -> Vec<String> {
        self.profiles.keys().cloned().collect()
    }

    /// `true` si existe un perfil con ese nombre.
    pub fn contains(&self, name: &str) -> bool {
        self.profiles.contains_key(name)
    }

    /// Conmuta el perfil activo (default global). Error si no existe.
    pub fn set_active(&mut self, name: &str) -> Result<(), super::shortcuts::ProfileError> {
        if self.profiles.contains_key(name) {
            self.active = name.to_string();
            Ok(())
        } else {
            Err(super::shortcuts::ProfileError::NotFound(name.to_string()))
        }
    }

    /// Crea un perfil nuevo. Error si ya existe o el nombre es vacío.
    pub fn create(&mut self, name: &str, ap: Appearance) -> Result<(), super::shortcuts::ProfileError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(super::shortcuts::ProfileError::EmptyName);
        }
        if self.profiles.contains_key(name) {
            return Err(super::shortcuts::ProfileError::AlreadyExists(name.to_string()));
        }
        self.profiles.insert(name.to_string(), ap);
        Ok(())
    }

    /// Fija (o quita, con `None`) el wallpaper de un perfil. Funciona también
    /// sobre presets (queda un override en disco; `ensure_builtins` no lo pisa).
    pub fn set_wallpaper(&mut self, name: &str, path: Option<String>) -> Result<(), super::shortcuts::ProfileError> {
        match self.profiles.get_mut(name) {
            Some(ap) => {
                ap.wallpaper = path.filter(|s| !s.trim().is_empty());
                Ok(())
            }
            None => Err(super::shortcuts::ProfileError::NotFound(name.to_string())),
        }
    }

    /// El wallpaper del perfil activo (si lo tiene).
    pub fn active_wallpaper(&self) -> Option<String> {
        self.profiles.get(&self.active).and_then(|ap| ap.wallpaper.clone())
    }

    /// Duplica un perfil existente con un nombre nuevo.
    pub fn duplicate(&mut self, src: &str, name: &str) -> Result<(), super::shortcuts::ProfileError> {
        let ap = self
            .profiles
            .get(src)
            .cloned()
            .ok_or_else(|| super::shortcuts::ProfileError::NotFound(src.to_string()))?;
        self.create(name, ap)
    }

    /// Renombra un perfil propio (los presets no se renombran). Si se renombra
    /// el activo, el activo sigue al nombre nuevo.
    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), super::shortcuts::ProfileError> {
        use super::shortcuts::ProfileError;
        let to = to.trim();
        if to.is_empty() {
            return Err(ProfileError::EmptyName);
        }
        if is_builtin(from) {
            return Err(ProfileError::BuiltinProtected(from.to_string()));
        }
        if !self.profiles.contains_key(from) {
            return Err(ProfileError::NotFound(from.to_string()));
        }
        if self.profiles.contains_key(to) {
            return Err(ProfileError::AlreadyExists(to.to_string()));
        }
        let ap = self.profiles.remove(from).expect("recién comprobado");
        self.profiles.insert(to.to_string(), ap);
        if self.active == from {
            self.active = to.to_string();
        }
        Ok(())
    }

    /// Borra un perfil. Los presets de fábrica no se pueden borrar; si se borra
    /// el activo, cae a `Sistema`.
    pub fn remove(&mut self, name: &str) -> Result<(), super::shortcuts::ProfileError> {
        if is_builtin(name) {
            return Err(super::shortcuts::ProfileError::BuiltinProtected(name.to_string()));
        }
        if self.profiles.remove(name).is_none() {
            return Err(super::shortcuts::ProfileError::NotFound(name.to_string()));
        }
        if self.active == name {
            self.active = SYSTEM_NAME.to_string();
        }
        Ok(())
    }

    fn ensure_builtins(&mut self) {
        for name in PRESET_NAMES {
            self.profiles
                .entry((*name).to_string())
                .or_insert_with(|| preset(name).expect("preset de fábrica"));
        }
        if !self.profiles.contains_key(&self.active) {
            self.active = SYSTEM_NAME.to_string();
        }
    }

    // --- Disco --------------------------------------------------------

    /// La ruta canónica: `~/.config/shuma/appearance.ron`.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "shuma")
            .map(|d| d.config_dir().join("appearance.ron"))
    }

    fn to_ron(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .expect("AppearanceProfiles siempre serializa")
    }

    fn from_ron(text: &str) -> Result<AppearanceProfiles, String> {
        let mut me: AppearanceProfiles =
            ron::from_str(text).map_err(|e| format!("RON de apariencia inválido: {e}"))?;
        me.ensure_builtins();
        Ok(me)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_ron())
    }

    pub fn load_or_init(path: &Path) -> AppearanceProfiles {
        if path.exists() {
            match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| Self::from_ron(&t)) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("shuma · apariencia «{}» inválida ({e}); uso la de fábrica.", path.display());
                    AppearanceProfiles::default()
                }
            }
        } else {
            let p = AppearanceProfiles::default();
            if let Err(e) = p.save(path) {
                eprintln!("shuma · no pude escribir la apariencia inicial: {e}");
            }
            p
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_trae_los_presets_con_sistema_activo() {
        let p = AppearanceProfiles::default();
        assert_eq!(p.active(), SYSTEM_NAME);
        for n in PRESET_NAMES {
            assert!(p.contains(n), "falta preset {n}");
        }
    }

    #[test]
    fn translucido_aplica_opacidad_al_fondo() {
        let a = preset("Translúcido").unwrap();
        assert!(a.opacity < 1.0);
        let t = a.resolve();
        // bg_app debe quedar con alfa < 255.
        let alpha = (t.bg_app.components[3] * 255.0).round() as u8;
        assert!(alpha < 255, "el fondo translúcido debe tener alfa parcial");
    }

    #[test]
    fn resolve_aplica_acento_override() {
        let mut a = preset("Oscuro").unwrap();
        a.accent = Some([200, 50, 50, 255]);
        let t = a.resolve();
        assert_eq!(t.accent, Color::from_rgba8(200, 50, 50, 255));
    }

    #[test]
    fn round_trip_por_ron() {
        let mut p = AppearanceProfiles::default();
        p.duplicate("Oscuro", "Mío").unwrap();
        p.set_active("Mío").unwrap();
        let back = AppearanceProfiles::from_ron(&p.to_ron()).unwrap();
        assert_eq!(back.active(), "Mío");
        assert_eq!(back, p);
    }

    #[test]
    fn renombrar_respeta_presets() {
        let mut p = AppearanceProfiles::default();
        assert!(p.rename("Oscuro", "x").is_err()); // de fábrica
        p.duplicate("Oscuro", "Mío").unwrap();
        p.rename("Mío", "Tuyo").unwrap();
        assert!(p.contains("Tuyo") && !p.contains("Mío"));
    }

    #[test]
    fn no_se_borra_un_preset() {
        let mut p = AppearanceProfiles::default();
        assert!(p.remove("Oscuro").is_err());
        assert!(p.contains("Oscuro"));
    }

    #[test]
    fn set_y_clear_wallpaper_en_el_activo() {
        let mut p = AppearanceProfiles::default();
        p.duplicate("Oscuro", "Foto").unwrap();
        p.set_active("Foto").unwrap();
        assert_eq!(p.active_wallpaper(), None);
        p.set_wallpaper("Foto", Some("/img/x.jpg".to_string())).unwrap();
        assert_eq!(p.active_wallpaper().as_deref(), Some("/img/x.jpg"));
        // vacío cuenta como quitar.
        p.set_wallpaper("Foto", Some("   ".to_string())).unwrap();
        assert_eq!(p.active_wallpaper(), None);
        // perfil inexistente falla.
        assert!(p.set_wallpaper("nope", Some("/y".to_string())).is_err());
    }

    #[test]
    fn con_wallpaper_el_fondo_se_vuelve_translucido_aunque_opacity_sea_1() {
        let mut a = preset("Oscuro").unwrap();
        assert_eq!(a.opacity, 1.0);
        a.wallpaper = Some("/img/x.jpg".to_string());
        let t = a.resolve();
        let alpha = (t.bg_app.components[3] * 255.0).round() as u8;
        assert!(alpha < 255, "con wallpaper el fondo debe dejar ver la imagen");
    }
}
