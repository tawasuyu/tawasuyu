//! `shuma-module` — contrato de los módulos enchufables a `shuma-shell-llimphi`.
//!
//! Un módulo aporta hasta tres cosas a la ventana del shell:
//!
//! 1. **Tab principal** — una vista propia, ocupando el panel central
//!    cuando su tab está activo.
//! 2. **Monitores** — curvas pequeñas que viven en el stack del panel
//!    derecho, junto a CPU/MEM.
//! 3. **Shortcuts** — botones de la toolbar de la app-header que disparan
//!    una acción del módulo o publican un comando al shell.
//!
//! El contrato es **estructural**, no un trait dinámico: cada módulo es
//! un crate que define su propio `State`/`Msg`/`update`/`view` y expone
//! una `pub fn make(host: ModuleHost) -> Box<...>`. El host (el binario
//! `shuma-shell-llimphi`) tiene un enum `ShellMsg` con una variante por
//! módulo conocido y los enlaza al compilar.
//!
//! Aquí sólo viven los **tipos compartidos**:
//!
//! - [`Source`] — local o remoto (con credenciales SSH).
//! - [`ModuleConfig`] — entrada de un `[[modules]]` del `shumarc.toml`.
//! - [`MonitorSpec`] — descriptor declarativo de un monitor (label,
//!   color, capacidad de historial, frecuencia de sampling).
//! - [`Sample`] — un punto de la curva.
//! - [`ShortcutSpec`] — descriptor declarativo de un botón de toolbar.
//! - [`ShortcutAction`] — qué hace al pulsarse.
//! - [`Placement`] — en qué slot del chasis vive el módulo (TopBar,
//!   Main, BottomBar, DrawerTab).
//! - [`DrawerTrigger`] — qué dispara la apertura del drawer Quake.
//!
//! El módulo no depende de `llimphi-ui` desde este crate; el host le
//! pasa el `Theme` y el módulo construye el `View` con un `lift`
//! (cierre que mapea su `Msg` propio al `ShellMsg`). El lift cierra la
//! brecha de "no hay `View::map`" sin pagar el costo de un trait
//! object con `Box<dyn Any>`.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Identificador único de un módulo dentro de una sesión. Se compara
/// case-sensitive contra los `id` de los `[[modules]]` del shumarc.
pub type ModuleId = &'static str;

/// Origen contra el cual opera un módulo: `Local` actúa sobre esta
/// máquina, `Daemon` lo hace vía `shuma-daemon` por Unix socket,
/// `DaemonTcp` igual pero por TCP autenticado (Noise XK), `Remote`
/// SSH para módulos que aún no hablan el protocolo del daemon (matilda).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Esta máquina, ejecución directa (`shuma-exec::run`).
    Local,
    /// Esta máquina pero los comandos van por el daemon — Unix socket.
    /// `socket` es opcional: cuando es `None` se usa `default_socket_path()`.
    Daemon {
        #[serde(default)]
        socket: Option<std::path::PathBuf>,
        #[serde(default)]
        label: Option<String>,
    },
    /// Daemon en otro host, conectado por TCP con handshake Noise XK.
    /// `server_pub_hex` es la pubkey del server (estilo `known_hosts`).
    DaemonTcp {
        addr: String,
        server_pub_hex: String,
        #[serde(default)]
        label: Option<String>,
    },
    /// Servidor remoto vía SSH. `host` y `user` son obligatorios; el
    /// método de autenticación se resuelve aparte (clave por defecto o
    /// password de un keystore — no se serializa aquí en claro).
    Remote {
        host: String,
        user: String,
        /// Puerto SSH; default 22.
        #[serde(default = "default_ssh_port")]
        port: u16,
        /// Etiqueta amigable para mostrar en la UI; default = `user@host`.
        #[serde(default)]
        label: Option<String>,
    },
}

fn default_ssh_port() -> u16 {
    22
}

impl Source {
    /// Etiqueta corta para la UI (tab, monitor, etc.).
    pub fn label(&self) -> String {
        match self {
            Source::Local => "local".into(),
            Source::Daemon { label: Some(l), .. } => l.clone(),
            Source::Daemon { socket: Some(p), .. } => format!("daemon:{}", p.display()),
            Source::Daemon { .. } => "daemon".into(),
            Source::DaemonTcp { label: Some(l), .. } => l.clone(),
            Source::DaemonTcp { addr, .. } => format!("daemon@{addr}"),
            Source::Remote { label: Some(l), .. } => l.clone(),
            Source::Remote { host, user, .. } => format!("{user}@{host}"),
        }
    }

    /// `true` si el origen es remoto (SSH o DaemonTcp).
    pub fn is_remote(&self) -> bool {
        matches!(self, Source::Remote { .. } | Source::DaemonTcp { .. })
    }
}

impl Default for Source {
    fn default() -> Self {
        Source::Local
    }
}

/// Configuración declarativa de **una instancia** de módulo, tal como
/// aparece en `shumarc.toml`:
///
/// ```toml
/// [[modules]]
/// id = "matilda"            # qué módulo activar (debe estar enlazado en el host)
/// source = { kind = "local" }
///
/// [[modules]]
/// id = "matilda"
/// source = { kind = "remote", host = "edge-1.example", user = "deploy" }
/// label = "edge-1"          # opcional, override del label del Source
/// options = { inventory = "/etc/matilda/edge-1.json" }
/// ```
///
/// `options` es un valor TOML opaco que cada módulo parsea a su gusto;
/// el host no lo interpreta. Si el módulo no enlistado en el host
/// aparece aquí, se ignora con un warning (no crash) — un shumarc no
/// debe romper el arranque del shell por un módulo desconocido.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleConfig {
    /// `id` que el host usa para enrutar (igual a `Module::id`).
    pub id: String,
    /// Origen contra el cual opera esta instancia.
    #[serde(default)]
    pub source: Source,
    /// Override del label de la tab/monitor. `None` = usa el default
    /// que decida el módulo (típicamente `Source::label`).
    #[serde(default)]
    pub label: Option<String>,
    /// Opciones específicas del módulo (parseo delegado al módulo).
    /// Se mantiene como string TOML para evitar acoplar este crate a
    /// `toml::Value` — el módulo decide cómo deserializar.
    #[serde(default)]
    pub options: Option<String>,
}

impl ModuleConfig {
    /// Construye una instancia con `id` + `source` y resto en defaults.
    /// Útil en tests y para registrar módulos sin shumarc (built-ins).
    pub fn new(id: impl Into<String>, source: Source) -> Self {
        Self {
            id: id.into(),
            source,
            label: None,
            options: None,
        }
    }

    /// Etiqueta efectiva: `label` si está, si no la del `Source`.
    pub fn effective_label(&self) -> String {
        self.label.clone().unwrap_or_else(|| self.source.label())
    }
}

/// En qué slot del chasis vive un módulo. El chasis dispone de cuatro
/// slots fijos que el shumarc puebla:
///
/// ```text
///  ┌─────────────────────────────────────────┐
///  │ TopBar         (1 módulo: launcher)     │
///  ├─────────────────────────────────────────┤
///  │                                         │
///  │  Main           (1 módulo focal —       │
///  │                  matilda, editor, etc.) │
///  │                                         │
///  ├─────────────────────────────────────────┤
///  │  ▲ Drawer Quake (overlay, oculto por    │
///  │     default; N módulos DrawerTab)       │
///  ├─────────────────────────────────────────┤
///  │ BottomBar      (1 módulo: command-bar)  │
///  └─────────────────────────────────────────┘
/// ```
///
/// Cada módulo declara su `Placement` *preferido*; el shumarc puede
/// sobreescribirlo. Un módulo puede ser válido en más de un slot
/// (p. ej. `shell` puede ir como `DrawerTab` o como `Main`) — esos
/// casos se modelan con instancias separadas en el shumarc, no con
/// "multi-placement" en el módulo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// Barra superior fija (launcher de apps / shortcuts).
    TopBar,
    /// Área principal de la ventana. **Único** por sesión: el módulo
    /// "foco". El drawer aparece encima como overlay sin reemplazarlo.
    #[default]
    Main,
    /// Barra inferior fija (command bar — input de doble modo
    /// launcher/shell). Auto-escondible según la `BarBehavior`.
    BottomBar,
    /// Tab del drawer Quake. **N** módulos pueden vivir aquí; el
    /// usuario navega entre ellos con la tira de tabs del drawer.
    DrawerTab,
}

impl Placement {
    /// `true` si el slot acepta múltiples instancias simultáneas.
    /// Sólo `DrawerTab` lo es; los demás son únicos.
    pub fn allows_multiple(self) -> bool {
        matches!(self, Placement::DrawerTab)
    }
}

/// Comportamiento de visibilidad de una barra (top o bottom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarBehavior {
    /// Siempre visible. Ocupa su alto en el layout permanentemente.
    #[default]
    Fixed,
    /// Visible al hover o foco; oculta cuando el cursor sale (con un
    /// delay corto). Cuando está oculta, no roba alto al `Main`.
    Autohide,
}

/// Qué dispara la apertura/cierre del drawer Quake. Múltiples triggers
/// se pueden activar simultáneamente (tecla + hover); cualquiera abre.
/// El cierre es por la inversa: salir del hover, soltar la tecla
/// (toggle) o pulsar `Esc`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawerTrigger {
    /// Tecla global que togglea el drawer. `None` = sin tecla. Formato
    /// libre (lo parsea el chasis): "F12", "Super+grave", etc.
    #[serde(default)]
    pub key: Option<String>,
    /// `true` = pasar el mouse sobre la command bar abre el drawer.
    #[serde(default)]
    pub hover: bool,
    /// Alto del drawer como fracción de la ventana (0.0..=1.0).
    /// `0.4` típico para drawer Quake.
    #[serde(default = "default_drawer_height")]
    pub height_fraction: f32,
}

fn default_drawer_height() -> f32 {
    0.4
}

impl Default for DrawerTrigger {
    /// Default razonable: tecla `F12` + hover off + 40% de alto.
    fn default() -> Self {
        Self {
            key: Some("F12".into()),
            hover: false,
            height_fraction: default_drawer_height(),
        }
    }
}

/// Una muestra puntual de un monitor — un valor numérico (porcentaje,
/// recuento, latencia, lo que sea) más un texto corto para mostrar
/// junto al label. El módulo decide la unidad y el formato.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sample {
    /// Valor numérico; típicamente `0.0..=100.0` para porcentajes pero
    /// el módulo puede usar cualquier rango. La curva escala al min/max
    /// de su buffer.
    pub value: f32,
    /// Texto secundario; típicamente "42%" o "3 pendientes". Vacío si
    /// el monitor sólo dibuja la curva sin valor numérico al lado.
    pub display: String,
}

impl Sample {
    pub fn new(value: f32, display: impl Into<String>) -> Self {
        Self {
            value,
            display: display.into(),
        }
    }
}

/// Color RGB en `0..=255` por canal. Lo deja como ints para no depender
/// de `peniko::Color` en este crate (el host lo convierte al pintar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Descriptor declarativo de **un monitor**. El host:
///
/// 1. Crea el slot en el panel derecho con el `label` + `accent`.
/// 2. Llama a `sampler()` cada `period` (típicamente 1s).
/// 3. Mantiene un historial de `history_capacity` muestras.
/// 4. Dibuja la curva (línea finita normalizada al min/max del buffer)
///    y al lado el `Sample::display` más reciente.
///
/// El módulo no toca el frame: sólo provee datos. Si `sampler()` es
/// caro, el módulo es libre de delegar a un hilo y devolver el último
/// snapshot cacheado — el host no impone política.
pub struct MonitorSpec {
    /// `id` único dentro del módulo (no global). El host antepone el id
    /// del módulo para evitar colisiones.
    pub id: &'static str,
    /// Texto que se muestra arriba de la curva ("docker", "drift", …).
    pub label: String,
    /// Color de la curva. `Rgb` para no depender de `peniko` aquí.
    pub accent: Rgb,
    /// Cuántas muestras guarda el ring buffer del historial.
    pub history_capacity: usize,
    /// Cada cuánto se muestrea (segundos). El host puede agregar
    /// jitter para evitar que todos los monitores caigan en el mismo
    /// tick.
    pub period_secs: f32,
    /// Closure que produce la muestra actual. Debe ser `Send + Sync`
    /// para que el host la pueda invocar desde un hilo de polling.
    pub sampler: Box<dyn Fn() -> Sample + Send + Sync>,
}

impl std::fmt::Debug for MonitorSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonitorSpec")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("accent", &self.accent)
            .field("history_capacity", &self.history_capacity)
            .field("period_secs", &self.period_secs)
            .field("sampler", &"<fn>")
            .finish()
    }
}

/// Qué hace un shortcut al pulsarse. La granularidad busca cubrir el
/// 80% sin exponer el `Msg` del módulo al host:
///
/// - `Command` — manda una línea al input del shell (como si el usuario
///   la hubiera tipeado y enter). Útil para integrar comandos arbitrarios.
/// - `FocusTab` — cambia la tab activa al módulo indicado.
/// - `ModuleAction` — opaco al host: el módulo lo recibe en su `update`
///   con esta `action_id` y decide. Es la vía para "Aplicar plan",
///   "Refrescar", etc. específicas del módulo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShortcutAction {
    /// Inyectar una línea en el input del shell.
    Command { line: String },
    /// Cambiar la tab activa al módulo `target` (su `ModuleId`).
    FocusTab { target: String },
    /// Acción opaca, enrutada al módulo emisor.
    ModuleAction { action_id: &'static str },
}

/// Descriptor declarativo de **un shortcut** de la toolbar. El host:
///
/// 1. Inserta un botón con el `label` en la app-header.
/// 2. Si `hint` está, lo muestra como tooltip.
/// 3. Al click, ejecuta el `action` según su variante.
#[derive(Debug, Clone, PartialEq)]
pub struct ShortcutSpec {
    /// Texto del botón ("Plan", "Apply", "Discover", "Logs", …).
    pub label: String,
    /// Tooltip / texto secundario. Opcional.
    pub hint: Option<String>,
    /// Qué hace al pulsarse.
    pub action: ShortcutAction,
}

impl ShortcutSpec {
    pub fn command(label: impl Into<String>, line: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            hint: None,
            action: ShortcutAction::Command { line: line.into() },
        }
    }

    pub fn module_action(label: impl Into<String>, action_id: &'static str) -> Self {
        Self {
            label: label.into(),
            hint: None,
            action: ShortcutAction::ModuleAction { action_id },
        }
    }

    pub fn focus_tab(label: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            hint: None,
            action: ShortcutAction::FocusTab { target: target.into() },
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// Catálogo de las contribuciones declarativas (sin View) de un módulo.
/// El módulo lo produce con su `State` actual y el host lo consume para
/// poblar el panel derecho y la toolbar. La vista del tab va aparte
/// porque depende del `ShellMsg` del host (no encaja como `dyn`).
#[derive(Debug)]
pub struct ModuleContributions {
    pub monitors: Vec<MonitorSpec>,
    pub shortcuts: Vec<ShortcutSpec>,
}

impl ModuleContributions {
    pub fn empty() -> Self {
        Self {
            monitors: Vec::new(),
            shortcuts: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_default_is_local() {
        assert_eq!(Source::default(), Source::Local);
        assert!(!Source::default().is_remote());
    }

    #[test]
    fn remote_source_label_falls_back_to_user_at_host() {
        let s = Source::Remote {
            host: "srv".into(),
            user: "ops".into(),
            port: 22,
            label: None,
        };
        assert_eq!(s.label(), "ops@srv");
        assert!(s.is_remote());
    }

    #[test]
    fn remote_source_label_uses_override_when_set() {
        let s = Source::Remote {
            host: "srv".into(),
            user: "ops".into(),
            port: 22,
            label: Some("edge".into()),
        };
        assert_eq!(s.label(), "edge");
    }

    #[test]
    fn module_config_effective_label_prefers_explicit() {
        let mut c = ModuleConfig::new("matilda", Source::Local);
        assert_eq!(c.effective_label(), "local");
        c.label = Some("Servidores".into());
        assert_eq!(c.effective_label(), "Servidores");
    }

    #[test]
    fn shortcut_constructors() {
        let cmd = ShortcutSpec::command("ls", "ls -la").with_hint("listar");
        assert_eq!(cmd.label, "ls");
        assert_eq!(cmd.hint.as_deref(), Some("listar"));
        match cmd.action {
            ShortcutAction::Command { line } => assert_eq!(line, "ls -la"),
            _ => panic!("expected Command"),
        }

        let act = ShortcutSpec::module_action("Apply", "matilda.apply");
        match act.action {
            ShortcutAction::ModuleAction { action_id } => assert_eq!(action_id, "matilda.apply"),
            _ => panic!("expected ModuleAction"),
        }

        let foc = ShortcutSpec::focus_tab("→ Matilda", "matilda");
        match foc.action {
            ShortcutAction::FocusTab { target } => assert_eq!(target, "matilda"),
            _ => panic!("expected FocusTab"),
        }
    }

    #[test]
    fn monitor_spec_holds_a_callable_sampler() {
        let m = MonitorSpec {
            id: "test",
            label: "Test".into(),
            accent: Rgb::new(255, 100, 0),
            history_capacity: 60,
            period_secs: 1.0,
            sampler: Box::new(|| Sample::new(42.0, "42%")),
        };
        let s = (m.sampler)();
        assert_eq!(s.value, 42.0);
        assert_eq!(s.display, "42%");
    }

    #[test]
    fn placement_default_is_main() {
        assert_eq!(Placement::default(), Placement::Main);
    }

    #[test]
    fn only_drawer_tab_allows_multiple_instances() {
        assert!(Placement::DrawerTab.allows_multiple());
        assert!(!Placement::TopBar.allows_multiple());
        assert!(!Placement::BottomBar.allows_multiple());
        assert!(!Placement::Main.allows_multiple());
    }

    #[test]
    fn placement_round_trips_snake_case_toml() {
        // Sanity check del rename_all snake_case en serde.
        let p: Placement = toml::from_str("v = \"top_bar\"\n")
            .ok()
            .and_then(|t: toml::Table| t.get("v").cloned())
            .and_then(|v| v.try_into().ok())
            .unwrap();
        assert_eq!(p, Placement::TopBar);
        let p: Placement = toml::from_str("v = \"drawer_tab\"\n")
            .ok()
            .and_then(|t: toml::Table| t.get("v").cloned())
            .and_then(|v| v.try_into().ok())
            .unwrap();
        assert_eq!(p, Placement::DrawerTab);
    }

    #[test]
    fn drawer_trigger_default_is_f12_no_hover() {
        let d = DrawerTrigger::default();
        assert_eq!(d.key.as_deref(), Some("F12"));
        assert!(!d.hover);
        assert!((d.height_fraction - 0.4).abs() < 1e-6);
    }

    #[test]
    fn bar_behavior_default_is_fixed() {
        assert_eq!(BarBehavior::default(), BarBehavior::Fixed);
    }

    #[test]
    fn module_config_round_trips_toml() {
        let c = ModuleConfig {
            id: "matilda".into(),
            source: Source::Remote {
                host: "srv".into(),
                user: "ops".into(),
                port: 2222,
                label: None,
            },
            label: Some("Edge 1".into()),
            options: Some("inventory = \"/etc/matilda/inv.json\"".into()),
        };
        // Round-trip por toml: el shumarc usa esto para serializar/parsear.
        let text = toml::to_string(&c).unwrap();
        let back: ModuleConfig = toml::from_str(&text).unwrap();
        assert_eq!(c, back);
    }
}
