//! `shuma-module-launcher` — barra superior fija con apps/shortcuts.
//!
//! Vive en el slot [`Placement::TopBar`] del chasis: una tira corta
//! con accesos directos. Las entries se leen del filesystem:
//!
//! ```text
//! $XDG_CONFIG_HOME/shuma/apps/*.toml
//! ```
//!
//! Cada `.toml` declara una entry:
//!
//! ```toml
//! label = "Pluma"
//! exec = "pluma-app"          # opcional; si está, click → spawn detached
//! action_id = "focus:pluma"   # opcional; si no hay exec, el chasis lo dispatchea
//! ```
//!
//! Si `~/.config/shuma/apps/` no existe (o está vacío), el launcher
//! cae al `State::demo()` con tres entries fijas (Files/Shell/Matilda)
//! para que el chasis sea exploratorio desde el día uno.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::Theme;
use shuma_module::{ModuleContributions, Placement, ShortcutSpec};

/// `id` canónico del módulo.
pub const ID: &str = "launcher";

/// `Placement` por defecto del módulo. El shumarc puede overrideearlo
/// (p. ej. ponerlo como `DrawerTab` para tenerlo dentro del overlay
/// Quake), pero su lugar natural es la barra superior.
pub const DEFAULT_PLACEMENT: Placement = Placement::TopBar;

/// Estado del módulo. En el placeholder lleva un buffer mínimo: la
/// app que se está hovereando, si hay. Cuando llegue la integración
/// real, aquí vivirán los `[apps]` cargados del shumarc.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// Lista de entradas del launcher (label + acción al click).
    pub entries: Vec<LauncherEntry>,
}

/// Una entrada del launcher: label, opcional `exec` para spawn
/// detached al click, opcional `action_id` que el chasis dispatchea.
/// Al menos uno de los dos debe estar (el loader rechaza el manifest
/// si los dos están vacíos).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct LauncherEntry {
    pub label: String,
    /// Programa a ejecutar (parseo simple por whitespace: el primer
    /// token es el binario, el resto son args). Si está, el click hace
    /// spawn detached vía `process_group(0)`.
    #[serde(default)]
    pub exec: Option<String>,
    /// Acción opaca al chasis (focus:shell, open:files, etc.). Si no
    /// hay `exec`, el chasis decide qué hacer (focus a un tab, abrir
    /// un módulo, etc.). Default `""` cuando hay `exec`.
    #[serde(default)]
    pub action_id: String,
}

impl LauncherEntry {
    pub fn new(label: impl Into<String>, action_id: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            exec: None,
            action_id: action_id.into(),
        }
    }

    pub fn with_exec(label: impl Into<String>, exec: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            exec: Some(exec.into()),
            action_id: String::new(),
        }
    }
}

impl State {
    /// State de demo con entries fijas: Files / Shell / Matilda. El
    /// loader real las reemplaza si encuentra manifests en disco.
    pub fn demo() -> Self {
        Self {
            entries: vec![
                LauncherEntry::new("Files", "open:files"),
                LauncherEntry::new("Shell", "focus:shell"),
                LauncherEntry::new("Matilda", "focus:matilda"),
            ],
        }
    }

    /// Lee `$XDG_CONFIG_HOME/shuma/apps/*.toml` (orden alfabético) y
    /// arma las entries. Si el dir no existe o no hay manifests
    /// válidos, devuelve `State::demo()` — el chasis arranca usable.
    pub fn from_apps_dir() -> Self {
        let Some(dir) = apps_dir() else {
            return Self::demo();
        };
        let entries = load_entries_from_dir(&dir);
        if entries.is_empty() {
            Self::demo()
        } else {
            Self { entries }
        }
    }
}

fn apps_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("shuma").join("apps"))
}

/// Lee `.toml` del dir como `LauncherEntry` y los devuelve ordenados.
/// Manifests inválidos se omiten silenciosamente (un launcher no debe
/// fallar el shell por un toml roto).
pub fn load_entries_from_dir(dir: &std::path::Path) -> Vec<LauncherEntry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<std::path::PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("toml"))
        .collect();
    paths.sort();
    let mut out: Vec<LauncherEntry> = Vec::new();
    for p in paths {
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(entry) = toml::from_str::<LauncherEntry>(&text) else {
            continue;
        };
        if entry.exec.is_none() && entry.action_id.is_empty() {
            continue;
        }
        out.push(entry);
    }
    out
}

/// Mensajes del módulo.
#[derive(Debug, Clone)]
pub enum Msg {
    /// Click en una entry; lleva el `action_id` para que el chasis lo
    /// resuelva (típicamente buscando un módulo con ese id, o lanzando
    /// el comando si es `cmd:...`). Sólo se emite cuando la entry NO
    /// tiene `exec` propio — si tenía, el launcher ya lo spawneó y el
    /// chasis no necesita hacer nada.
    EntryClicked(String),
}

pub fn update(state: State, _msg: Msg) -> State {
    state
}

/// Spawnea el `exec` de una entry detached del shell. Parseo simple
/// por whitespace; quoting avanzado no soportado (un launcher quiere
/// invocar binarios, no scripts).
pub fn spawn_exec(exec_line: &str) {
    use std::os::unix::process::CommandExt;
    let mut parts = exec_line.split_whitespace();
    let Some(program) = parts.next() else {
        return;
    };
    let args: Vec<&str> = parts.collect();
    let _ = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
}

/// Mapea `action_id` a `Msg`. El launcher expone `launcher.toggle` como
/// acción global que el chasis consume directamente (toggle de la
/// TopBar autohide); ningún `action_id` produce un `Msg` propio del
/// launcher todavía.
pub fn dispatch(_action_id: &str) -> Option<Msg> {
    None
}

/// Renderiza la barra superior: el label "shuma" a la izquierda y los
/// botones de entries a la derecha (compactos, alto fijo). Aplica el
/// alto de la app-header global (40 px) para que cuadre con el resto
/// de las apps Llimphi.
pub fn view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + 'static + Clone,
) -> View<HostMsg> {
    let brand = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned("shuma".to_string(), 13.0, theme.fg_text, Alignment::Start);

    let mut children: Vec<View<HostMsg>> = vec![brand];
    for entry in &state.entries {
        let lift = lift.clone();
        let action_id = entry.action_id.clone();
        let exec = entry.exec.clone();
        children.push(entry_button(entry.label.clone(), theme, move || {
            // Si la entry tiene `exec`, lanzamos detached y devolvemos
            // un msg neutral (el chasis lo ve como "no hagas nada").
            // Si no, emitimos EntryClicked para que el chasis lo
            // resuelva via su tabla de dispatch.
            if let Some(line) = exec.clone() {
                spawn_exec(&line);
                lift(Msg::EntryClicked(String::new()))
            } else {
                lift(Msg::EntryClicked(action_id.clone()))
            }
        }));
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(children)
}

fn entry_button<HostMsg: Clone + 'static>(
    label: String,
    theme: &Theme,
    on_click: impl FnOnce() -> HostMsg,
) -> View<HostMsg> {
    let msg = on_click();
    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_button)
    .hover_fill(theme.bg_button_hover)
    .radius(4.0)
    .text_aligned(label, 12.0, theme.fg_text, Alignment::Center)
    .on_click(msg)
}

/// Por consistencia con `Color::accent`. No usado en el placeholder
/// pero referenciado para que pase clippy si el bloque siguiente lo
/// llama desde un panel de "recent apps" o similar.
#[allow(dead_code)]
fn _accent_unused(theme: &Theme) -> Color {
    theme.accent
}

/// Contribuciones: el launcher mismo aporta un shortcut al toolbar
/// general ("Apps") que es redundante con la TopBar pero útil cuando
/// el launcher está oculto (TopBar autohide).
pub fn contributions(_state: &State) -> ModuleContributions {
    ModuleContributions {
        monitors: Vec::new(),
        shortcuts: vec![ShortcutSpec::module_action("Apps", "launcher.toggle")
            .with_hint("Abrir el launcher de apps")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable() {
        assert_eq!(ID, "launcher");
    }

    #[test]
    fn default_placement_is_topbar() {
        assert_eq!(DEFAULT_PLACEMENT, Placement::TopBar);
    }

    #[test]
    fn demo_state_has_three_entries() {
        let s = State::demo();
        assert_eq!(s.entries.len(), 3);
        assert_eq!(s.entries[0].label, "Files");
        assert_eq!(s.entries[1].action_id, "focus:shell");
    }

    #[test]
    fn contributions_expose_apps_shortcut() {
        let s = State::default();
        let c = contributions(&s);
        assert_eq!(c.shortcuts.len(), 1);
        assert_eq!(c.shortcuts[0].label, "Apps");
    }

    #[test]
    fn entry_clicked_message_carries_action_id() {
        let m = Msg::EntryClicked("focus:matilda".into());
        match m {
            Msg::EntryClicked(id) => assert_eq!(id, "focus:matilda"),
        }
    }

    #[test]
    fn load_entries_from_dir_reads_toml_manifests() {
        let dir = std::env::temp_dir().join(format!(
            "shuma-launcher-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("01-pluma.toml"),
            "label = \"Pluma\"\nexec = \"pluma-app\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("02-shell.toml"),
            "label = \"Shell\"\naction_id = \"focus:shell\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("03-invalida.toml"), "label = \"X\"\n").unwrap();
        let entries = load_entries_from_dir(&dir);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label, "Pluma");
        assert_eq!(entries[0].exec.as_deref(), Some("pluma-app"));
        assert_eq!(entries[1].action_id, "focus:shell");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
