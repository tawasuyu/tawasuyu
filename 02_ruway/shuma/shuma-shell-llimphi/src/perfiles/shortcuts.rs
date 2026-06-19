//! Perfiles de **atajos** — una biblioteca de keymaps del workspace conmutables
//! con un clic (globales en shuma).
//!
//! shuma no trae *un* keymap sino una **biblioteca**: presets de fábrica
//! (`shuma` nativo, `hyprland`, `tmux`, `zellij`, `vim`) más los que el usuario
//! cree. Sobre ella se puede **conmutar** el activo, **duplicar** y editar, etc.
//! — mismo patrón que `mirada-brain::profiles`.
//!
//! ## Modelo de acorde
//!
//! Un keymap es un `prefix` opcional + un mapa `acorde → acción`:
//!
//! - **directo** (`prefix: None`): el acorde dispara la acción al instante
//!   (estilo hyprland/dwm: `Super+…`, `Alt+…`). Para no tragarse texto normal,
//!   todos los binds directos llevan un modificador.
//! - **con prefijo** (`prefix: Some("Ctrl+b")`): primero se pulsa el prefijo
//!   (entra en estado "pendiente") y la siguiente tecla dispara la acción
//!   (estilo tmux/vim). Una tecla no ligada tras el prefijo lo cancela.
//!
//! El estado "pendiente" vive en `Model::pending_prefix` (transitorio, no se
//! persiste).
//!
//! Persistencia: `~/.config/shuma/shortcuts.ron`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::types::{Model, Msg};

/// Una acción del workspace tipo zellij que un atajo puede disparar. Se traduce
/// al `Msg` concreto con [`ShortcutAction::to_concrete`] (que necesita el modelo
/// para resolver "siguiente/anterior/ir-a tab N").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShortcutAction {
    /// Tab nueva (con un shell fresco).
    NewTab,
    /// Cierra la tab activa.
    CloseTab,
    /// Tab siguiente (con wrap).
    NextTab,
    /// Tab anterior (con wrap).
    PrevTab,
    /// Va a la tab N (1-based).
    GotoTab(u8),
    /// Parte el panel con foco lado a lado (Horizontal).
    SplitH,
    /// Parte el panel con foco apilado (Vertical).
    SplitV,
    /// Cierra el panel con foco.
    ClosePane,
    /// Cicla el foco al panel siguiente.
    CycleNext,
    /// Cicla el foco al panel anterior.
    CyclePrev,
    /// Enciende/apaga la capa de flotantes.
    FloatToggle,
    /// Agrega un panel flotante nuevo.
    FloatNew,
}

impl ShortcutAction {
    /// Traduce la acción al `Msg` concreto del chasis, usando el modelo para
    /// resolver las que dependen de la tab activa.
    pub(crate) fn to_concrete(self, model: &Model) -> Option<Msg> {
        use llimphi_widget_panes::Axis;
        let ws = model.active().map(|s| &s.workspace);
        let active_tab = ws.map(|w| w.active_tab).unwrap_or(0);
        let n_tabs = ws.map(|w| w.tabs.len().max(1)).unwrap_or(1);
        Some(match self {
            ShortcutAction::NewTab => Msg::TabNew,
            ShortcutAction::CloseTab => Msg::TabClose(active_tab),
            ShortcutAction::NextTab => Msg::TabSwitch((active_tab + 1) % n_tabs),
            ShortcutAction::PrevTab => Msg::TabSwitch((active_tab + n_tabs - 1) % n_tabs),
            ShortcutAction::GotoTab(n) => {
                if n >= 1 {
                    Msg::TabSwitch((n as usize) - 1)
                } else {
                    return None;
                }
            }
            ShortcutAction::SplitH => Msg::PaneSplit(Axis::Horizontal),
            ShortcutAction::SplitV => Msg::PaneSplit(Axis::Vertical),
            ShortcutAction::ClosePane => Msg::PaneClose,
            ShortcutAction::CycleNext => Msg::PaneCycle(true),
            ShortcutAction::CyclePrev => Msg::PaneCycle(false),
            ShortcutAction::FloatToggle => Msg::FloatToggle,
            ShortcutAction::FloatNew => Msg::FloatNew,
        })
    }
}

/// Un keymap: prefijo opcional + binds `acorde → acción`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keymap {
    /// Acorde de prefijo (estilo tmux/vim). `None` = binds directos.
    #[serde(default)]
    pub prefix: Option<String>,
    /// `acorde normalizado → acción`.
    pub binds: BTreeMap<String, ShortcutAction>,
}

impl Keymap {
    fn from_pairs(prefix: Option<&str>, pairs: &[(&str, ShortcutAction)]) -> Self {
        Keymap {
            prefix: prefix.map(|s| s.to_string()),
            binds: pairs
                .iter()
                .map(|(k, a)| ((*k).to_string(), *a))
                .collect(),
        }
    }

    /// Funde los binds de `fresh` que falten sin pisar los del usuario (espejo
    /// de `mirada-brain::Keymap::merge_from`): un preset de fábrica viejo en
    /// disco recibe los atajos nuevos. El prefijo del fresco gana si el guardado
    /// no tiene.
    fn merge_from(&mut self, fresh: &Keymap) {
        if self.prefix.is_none() {
            self.prefix = fresh.prefix.clone();
        }
        for (k, a) in &fresh.binds {
            self.binds.entry(k.clone()).or_insert(*a);
        }
    }
}

use ShortcutAction::*;

/// Los nombres de los presets de fábrica, en orden de presentación.
pub const PRESET_NAMES: &[&str] = &["shuma", "hyprland", "tmux", "zellij", "vim"];

/// `true` si `name` es un preset de fábrica (protegido contra borrado/renombre).
pub fn is_builtin(name: &str) -> bool {
    PRESET_NAMES.contains(&name)
}

/// El keymap de un preset de fábrica por nombre.
pub fn preset(name: &str) -> Option<Keymap> {
    Some(match name {
        // Nativo de shuma: directo, prefijo `Alt`. Es exactamente lo que tenía
        // `workspace_key` hardcoded antes de los perfiles.
        "shuma" => Keymap::from_pairs(
            None,
            &[
                ("Alt+t", NewTab),
                ("Alt+w", ClosePane),
                ("Alt+\\", SplitH),
                ("Alt+v", SplitH),
                ("Alt+-", SplitV),
                ("Alt+s", SplitV),
                ("Alt+f", FloatToggle),
                ("Alt+n", FloatNew),
                ("Alt+[", PrevTab),
                ("Alt+]", NextTab),
                ("Alt+Left", CyclePrev),
                ("Alt+Right", CycleNext),
                ("Alt+1", GotoTab(1)),
                ("Alt+2", GotoTab(2)),
                ("Alt+3", GotoTab(3)),
                ("Alt+4", GotoTab(4)),
                ("Alt+5", GotoTab(5)),
                ("Alt+6", GotoTab(6)),
                ("Alt+7", GotoTab(7)),
                ("Alt+8", GotoTab(8)),
                ("Alt+9", GotoTab(9)),
            ],
        ),
        // Hyprland: directo, prefijo `Super`.
        "hyprland" => Keymap::from_pairs(
            None,
            &[
                ("Super+Return", NewTab),
                ("Super+q", ClosePane),
                ("Super+v", FloatToggle),
                ("Super+s", SplitV),
                ("Super+\\", SplitH),
                ("Super+Left", CyclePrev),
                ("Super+Right", CycleNext),
                ("Super+1", GotoTab(1)),
                ("Super+2", GotoTab(2)),
                ("Super+3", GotoTab(3)),
                ("Super+4", GotoTab(4)),
                ("Super+5", GotoTab(5)),
                ("Super+6", GotoTab(6)),
                ("Super+7", GotoTab(7)),
                ("Super+8", GotoTab(8)),
                ("Super+9", GotoTab(9)),
            ],
        ),
        // tmux: prefijo `Ctrl+b`, luego una tecla. `%`=split lado-a-lado,
        // `"`=split apilado (igual que tmux real).
        "tmux" => Keymap::from_pairs(
            Some("Ctrl+b"),
            &[
                ("c", NewTab),
                ("&", CloseTab),
                ("x", ClosePane),
                ("%", SplitH),
                ("\"", SplitV),
                ("n", NextTab),
                ("p", PrevTab),
                ("o", CycleNext),
                ("z", FloatToggle),
                ("1", GotoTab(1)),
                ("2", GotoTab(2)),
                ("3", GotoTab(3)),
                ("4", GotoTab(4)),
                ("5", GotoTab(5)),
                ("6", GotoTab(6)),
                ("7", GotoTab(7)),
                ("8", GotoTab(8)),
                ("9", GotoTab(9)),
            ],
        ),
        // zellij (capa rápida tipo "locked"): directo, prefijo `Alt` —
        // aproxima los defaults alt-based de zellij.
        "zellij" => Keymap::from_pairs(
            None,
            &[
                ("Alt+n", SplitV),
                ("Alt+t", NewTab),
                ("Alt+w", ClosePane),
                ("Alt+f", FloatToggle),
                ("Alt+[", PrevTab),
                ("Alt+]", NextTab),
                ("Alt+h", CyclePrev),
                ("Alt+l", CycleNext),
                ("Alt+Left", CyclePrev),
                ("Alt+Right", CycleNext),
                ("Alt+1", GotoTab(1)),
                ("Alt+2", GotoTab(2)),
                ("Alt+3", GotoTab(3)),
                ("Alt+4", GotoTab(4)),
                ("Alt+5", GotoTab(5)),
            ],
        ),
        // vim: prefijo `Ctrl+w` (mando de ventanas de vim). `s`=split horizontal
        // (divisor horizontal → apilado → Vertical), `v`=vsplit (lado a lado →
        // Horizontal).
        "vim" => Keymap::from_pairs(
            Some("Ctrl+w"),
            &[
                ("s", SplitV),
                ("v", SplitH),
                ("c", ClosePane),
                ("q", ClosePane),
                ("w", CycleNext),
                ("h", CyclePrev),
                ("l", CycleNext),
                ("j", CycleNext),
                ("k", CyclePrev),
                ("t", NewTab),
                ("n", NewTab),
                ("1", GotoTab(1)),
                ("2", GotoTab(2)),
                ("3", GotoTab(3)),
                ("4", GotoTab(4)),
                ("5", GotoTab(5)),
            ],
        ),
        _ => return None,
    })
}

/// La biblioteca de perfiles de atajos: el activo + todos los keymaps por nombre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutProfiles {
    active: String,
    profiles: BTreeMap<String, Keymap>,
}

impl Default for ShortcutProfiles {
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        for name in PRESET_NAMES {
            if let Some(km) = preset(name) {
                profiles.insert((*name).to_string(), km);
            }
        }
        Self {
            active: "shuma".to_string(),
            profiles,
        }
    }
}

impl ShortcutProfiles {
    /// El nombre del perfil activo.
    pub fn active(&self) -> &str {
        &self.active
    }

    /// El keymap del perfil activo (fallback al nativo `shuma` si el activo no
    /// existe por edición a mano).
    pub fn active_keymap(&self) -> Keymap {
        self.profiles
            .get(&self.active)
            .cloned()
            .unwrap_or_else(|| preset("shuma").expect("preset de fábrica"))
    }

    /// Los nombres de todos los perfiles, en orden alfabético.
    pub fn names(&self) -> Vec<String> {
        self.profiles.keys().cloned().collect()
    }

    /// `true` si existe un perfil con ese nombre.
    pub fn contains(&self, name: &str) -> bool {
        self.profiles.contains_key(name)
    }

    /// Conmuta el perfil activo. Error si no existe.
    pub fn set_active(&mut self, name: &str) -> Result<(), ProfileError> {
        if self.profiles.contains_key(name) {
            self.active = name.to_string();
            Ok(())
        } else {
            Err(ProfileError::NotFound(name.to_string()))
        }
    }

    /// Crea un perfil nuevo con el keymap dado. Error si ya existe o el nombre
    /// es vacío.
    pub fn create(&mut self, name: &str, km: Keymap) -> Result<(), ProfileError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProfileError::EmptyName);
        }
        if self.profiles.contains_key(name) {
            return Err(ProfileError::AlreadyExists(name.to_string()));
        }
        self.profiles.insert(name.to_string(), km);
        Ok(())
    }

    /// Duplica un perfil existente con un nombre nuevo.
    pub fn duplicate(&mut self, src: &str, name: &str) -> Result<(), ProfileError> {
        let km = self
            .profiles
            .get(src)
            .cloned()
            .ok_or_else(|| ProfileError::NotFound(src.to_string()))?;
        self.create(name, km)
    }

    /// Borra un perfil. Los presets de fábrica no se pueden borrar. Si se borra
    /// el activo, cae a `shuma`.
    pub fn remove(&mut self, name: &str) -> Result<(), ProfileError> {
        if is_builtin(name) {
            return Err(ProfileError::BuiltinProtected(name.to_string()));
        }
        if self.profiles.remove(name).is_none() {
            return Err(ProfileError::NotFound(name.to_string()));
        }
        if self.active == name {
            self.active = "shuma".to_string();
        }
        Ok(())
    }

    /// Re-siembra los presets de fábrica que falten y funde los binds nuevos en
    /// los builtins guardados sin pisar los rebinds del usuario.
    fn ensure_builtins(&mut self) {
        for name in PRESET_NAMES {
            let fresh = preset(name).expect("preset de fábrica");
            match self.profiles.get_mut(*name) {
                Some(saved) => saved.merge_from(&fresh),
                None => {
                    self.profiles.insert((*name).to_string(), fresh);
                }
            }
        }
        if !self.profiles.contains_key(&self.active) {
            self.active = "shuma".to_string();
        }
    }

    // --- Disco --------------------------------------------------------

    /// La ruta canónica: `~/.config/shuma/shortcuts.ron`.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "shuma")
            .map(|d| d.config_dir().join("shortcuts.ron"))
    }

    fn to_ron(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .expect("ShortcutProfiles siempre serializa")
    }

    fn from_ron(text: &str) -> Result<ShortcutProfiles, String> {
        let mut me: ShortcutProfiles =
            ron::from_str(text).map_err(|e| format!("RON de atajos inválido: {e}"))?;
        me.ensure_builtins();
        Ok(me)
    }

    /// Escribe la biblioteca, creando el directorio padre si falta.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_ron())
    }

    /// Carga con fallback amable: si no existe lo crea con los presets; si está
    /// corrupto avisa por stderr y usa los de fábrica sin pisarlo.
    pub fn load_or_init(path: &Path) -> ShortcutProfiles {
        if path.exists() {
            match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| Self::from_ron(&t)) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("shuma · atajos «{}» inválidos ({e}); uso los de fábrica.", path.display());
                    ShortcutProfiles::default()
                }
            }
        } else {
            let p = ShortcutProfiles::default();
            if let Err(e) = p.save(path) {
                eprintln!("shuma · no pude escribir los atajos iniciales: {e}");
            }
            p
        }
    }
}

/// Un fallo al operar sobre la biblioteca de perfiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    NotFound(String),
    AlreadyExists(String),
    EmptyName,
    BuiltinProtected(String),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::NotFound(n) => write!(f, "no existe el perfil «{n}»"),
            ProfileError::AlreadyExists(n) => write!(f, "ya existe el perfil «{n}»"),
            ProfileError::EmptyName => f.write_str("el nombre del perfil no puede ser vacío"),
            ProfileError::BuiltinProtected(n) => {
                write!(f, "«{n}» es un preset de fábrica; duplicalo para editarlo")
            }
        }
    }
}

impl std::error::Error for ProfileError {}

// ─── Normalización de acordes ───────────────────────────────────────

/// Normaliza un `KeyEvent` a un acorde canónico (`"Alt+t"`, `"Ctrl+b"`,
/// `"Super+Return"`, `"%"`). Orden de modificadores: Ctrl, Alt, Shift, Super.
/// El Shift se omite para símbolos/dígitos (su glifo ya viene resuelto); se
/// mantiene para letras y teclas con nombre. Devuelve `None` para teclas que no
/// modelamos como acorde.
pub(crate) fn chord_of(e: &llimphi_ui::KeyEvent) -> Option<String> {
    use llimphi_ui::{Key, NamedKey};
    let base: String = match &e.key {
        Key::Character(c) => {
            let s = c.as_str();
            if s.is_empty() {
                return None;
            }
            s.to_lowercase()
        }
        Key::Named(nk) => match nk {
            NamedKey::ArrowLeft => "Left".to_string(),
            NamedKey::ArrowRight => "Right".to_string(),
            NamedKey::ArrowUp => "Up".to_string(),
            NamedKey::ArrowDown => "Down".to_string(),
            NamedKey::Enter => "Return".to_string(),
            NamedKey::Tab => "Tab".to_string(),
            NamedKey::Space => "Space".to_string(),
            _ => return None,
        },
        _ => return None,
    };
    // ¿el Shift es semánticamente relevante? Sí para letras y teclas con nombre;
    // no para símbolos/dígitos (cuyo glifo ya incorpora el shift).
    let is_named = matches!(&e.key, llimphi_ui::Key::Named(_));
    let is_letter = base.len() == 1 && base.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false);
    let keep_shift = is_named || is_letter;
    let m = &e.modifiers;
    let mut s = String::new();
    if m.ctrl {
        s.push_str("Ctrl+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    if m.shift && keep_shift {
        s.push_str("Shift+");
    }
    if m.meta {
        s.push_str("Super+");
    }
    s.push_str(&base);
    Some(s)
}

/// Resuelve una tecla contra el keymap activo. Devuelve el `Msg` a emitir:
/// `ShortcutFire` (acción directa o tras prefijo), `ShortcutEnterPrefix`
/// (entró al prefijo) o `ShortcutCancelPrefix` (tecla suelta tras prefijo).
/// `None` = la tecla no es un atajo, sigue su curso al shell.
pub(crate) fn resolve_key(model: &Model, e: &llimphi_ui::KeyEvent) -> Option<Msg> {
    // No actuar si la sesión activa está en el form de creación.
    if model
        .sessions
        .get(model.active_session)
        .map(|s| s.pending)
        .unwrap_or(true)
    {
        return None;
    }
    let km = model.shortcuts.active_keymap();
    let chord = chord_of(e)?;
    match &km.prefix {
        Some(prefix) => {
            if model.pending_prefix {
                if let Some(act) = km.binds.get(&chord) {
                    Some(Msg::ShortcutFire(*act))
                } else {
                    // Tecla no ligada tras el prefijo: cancela (la consumimos).
                    Some(Msg::ShortcutCancelPrefix)
                }
            } else if &chord == prefix {
                Some(Msg::ShortcutEnterPrefix)
            } else {
                None
            }
        }
        None => km.binds.get(&chord).map(|a| Msg::ShortcutFire(*a)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_trae_los_presets_con_shuma_activo() {
        let p = ShortcutProfiles::default();
        assert_eq!(p.active(), "shuma");
        for n in PRESET_NAMES {
            assert!(p.contains(n), "falta preset {n}");
        }
    }

    #[test]
    fn switch_a_inexistente_falla() {
        let mut p = ShortcutProfiles::default();
        assert!(p.set_active("nope").is_err());
        assert!(p.set_active("tmux").is_ok());
        assert_eq!(p.active(), "tmux");
    }

    #[test]
    fn no_se_borra_un_preset_pero_si_un_propio() {
        let mut p = ShortcutProfiles::default();
        assert!(matches!(p.remove("tmux"), Err(ProfileError::BuiltinProtected(_))));
        p.duplicate("tmux", "mío").unwrap();
        p.set_active("mío").unwrap();
        p.remove("mío").unwrap();
        assert!(!p.contains("mío"));
        assert_eq!(p.active(), "shuma"); // el activo cae al nativo
    }

    #[test]
    fn round_trip_por_ron_preserva_activo_y_perfiles() {
        let mut p = ShortcutProfiles::default();
        p.duplicate("vim", "custom").unwrap();
        p.set_active("custom").unwrap();
        let back = ShortcutProfiles::from_ron(&p.to_ron()).unwrap();
        assert_eq!(back.active(), "custom");
        assert_eq!(back, p);
    }

    #[test]
    fn from_ron_resiembra_presets_faltantes() {
        let ron = r#"(active: "shuma", profiles: { "solo": (prefix: None, binds: {}) })"#;
        let p = ShortcutProfiles::from_ron(ron).unwrap();
        for n in PRESET_NAMES {
            assert!(p.contains(n));
        }
        assert!(p.contains("solo"));
    }

    #[test]
    fn tmux_es_con_prefijo_y_hyprland_directo() {
        assert_eq!(preset("tmux").unwrap().prefix.as_deref(), Some("Ctrl+b"));
        assert_eq!(preset("vim").unwrap().prefix.as_deref(), Some("Ctrl+w"));
        assert!(preset("hyprland").unwrap().prefix.is_none());
        assert!(preset("shuma").unwrap().prefix.is_none());
    }

    fn key_char(c: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> llimphi_ui::KeyEvent {
        llimphi_ui::KeyEvent {
            key: llimphi_ui::Key::Character(c.into()),
            state: llimphi_ui::KeyState::Pressed,
            text: None,
            modifiers: llimphi_ui::Modifiers { ctrl, alt, shift, meta },
            repeat: false,
        }
    }

    #[test]
    fn chord_normaliza_modificadores_y_omite_shift_en_simbolos() {
        assert_eq!(chord_of(&key_char("t", false, true, false, false)).as_deref(), Some("Alt+t"));
        assert_eq!(chord_of(&key_char("b", true, false, false, false)).as_deref(), Some("Ctrl+b"));
        // Shift se mantiene en letras…
        assert_eq!(chord_of(&key_char("a", false, true, true, false)).as_deref(), Some("Alt+Shift+a"));
        // …pero se omite en símbolos (el glifo ya viene shifteado).
        assert_eq!(chord_of(&key_char("%", false, false, true, false)).as_deref(), Some("%"));
        // Super (meta).
        assert_eq!(chord_of(&key_char("q", false, false, false, true)).as_deref(), Some("Super+q"));
    }
}
