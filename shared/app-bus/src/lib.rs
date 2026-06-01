//! `app-bus` — el cimiento del menú de aplicaciones de gioser.
//!
//! Hoy hay tres lanzadores que no comparten nada: `mirada-launcher`
//! (TOML propio, `std::process`), `shuma-module-launcher` (otro TOML,
//! `process_group`) y el launcher in-kernel de wawa (Manifiesto, WASM).
//! Cada uno reimplementa "qué apps existen y cómo se lanzan". Este crate
//! es la tabla única que todos consultan.
//!
//! Cuatro piezas, en capas:
//!
//! 1. **Registro** ([`AppRegistry`] + [`AppEntry`]): qué apps hay, cómo se
//!    lanzan ([`Launch`]) y qué mimes/lentes saben abrir (open-with).
//!    Se descubre de `~/.config/gioser/apps/*.toml` (feature `std`).
//! 2. **Menú global** ([`AppMenu`]/[`Menu`]/[`MenuItem`]): el clásico
//!    Archivo/Editar/Ayuda que la app *declara*. Cuando hay una barra de
//!    launcher presente, ésta lo *adopta* y la app deja de pintarlo en su
//!    ventana — el comportamiento "menú global" de macOS.
//! 3. **Launcher** ([`Launcher`] trait + [`LaunchError`]): la *instrucción
//!    de ejecución* abstracta. El host implementa con `std::process`
//!    ([`ProcessLauncher`]), wawa con instanciación WASM, shuma
//!    despachando `action`. El motor de launcher llama al trait y no se
//!    entera de en qué entorno corre.
//! 4. **Bus** ([`Bus`] + [`BusEvent`]): pub/sub in-process de foco /
//!    cambio de menú / pedido de lanzamiento / comando. La versión
//!    cross-proceso montará sobre el broker de brahman más adelante.
//!
//! Los **datos** ([`AppEntry`], [`Launch`], [`AppMenu`]…) y el **trait
//! [`Launcher`]** son `no_std + alloc`. El descubrimiento por filesystem,
//! el spawn de procesos y el [`Bus`] viven detrás del feature `std`.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

// =====================================================================
// Registro de apps
// =====================================================================

/// Cómo se enciende una app. Los tres mundos de gioser:
/// `Exec` (binario del host), `Action` (acción interna del chasis que la
/// hospeda — p.ej. `focus:shell`) y `Wasm` (módulo en el almacén de wawa,
/// direccionado por hash de bytecode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// Spawnear un comando/binario del host.
    Exec { program: String, args: Vec<String> },
    /// Acción interna a despachar por el host (no spawnea proceso).
    Action(String),
    /// App WASM de wawa, por hash de bytecode (hex) en el almacén.
    Wasm { bytecode_hex: String },
}

/// Una app registrada — la fila de la tabla que ven los launchers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    pub id: String,
    pub label: String,
    /// Glyph/emoji o ruta de ícono. Sin imponer formato — el launcher
    /// decide cómo pintarlo (texto en el dock MVP).
    pub icon: Option<String>,
    /// Agrupador opcional para la grilla/spotlight (p.ej. cuadrante).
    pub category: Option<String>,
    pub launch: Launch,
    /// Mimes/lentes que esta app sabe abrir (open-with). Vacío = no es
    /// visor; el registro de visores de nahual-shell se alimenta de acá.
    pub handles: Vec<String>,
}

impl AppEntry {
    /// `true` si la app declara saber abrir `mime`.
    pub fn handles_mime(&self, mime: &str) -> bool {
        self.handles.iter().any(|m| m == mime)
    }
}

#[cfg(feature = "std")]
impl AppEntry {
    /// Enciende la app vía `std::process`. Sólo `Exec` spawnea; `Action`/
    /// `Wasm` devuelven `Ok(None)` — los despacha el host (chasis/kernel).
    pub fn spawn(&self) -> std::io::Result<Option<std::process::Child>> {
        match &self.launch {
            Launch::Exec { program, args } => std::process::Command::new(program)
                .args(args)
                .spawn()
                .map(Some),
            Launch::Action(_) | Launch::Wasm { .. } => Ok(None),
        }
    }
}

// ----- forma en disco (TOML) -----

/// Espejo serde del archivo `<id>.toml`. La `[launch]` es una tabla con
/// campos opcionales en vez de un enum etiquetado — toml 0.8 trata los
/// enums internamente etiquetados de forma quisquillosa, así que
/// resolvemos a mano a [`Launch`]. Sólo se usa al parsear TOML (`std`).
#[cfg(feature = "std")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppFile {
    id: String,
    label: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    handles: Vec<String>,
    launch: LaunchFile,
}

#[cfg(feature = "std")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LaunchFile {
    #[serde(default)]
    exec: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    wasm: Option<String>,
}

#[cfg(feature = "std")]
impl LaunchFile {
    fn resolve(self) -> Option<Launch> {
        if let Some(program) = self.exec {
            Some(Launch::Exec {
                program,
                args: self.args,
            })
        } else if let Some(action) = self.action {
            Some(Launch::Action(action))
        } else {
            self.wasm.map(|bytecode_hex| Launch::Wasm { bytecode_hex })
        }
    }
}

#[cfg(feature = "std")]
impl AppFile {
    fn into_entry(self) -> Option<AppEntry> {
        Some(AppEntry {
            id: self.id,
            label: self.label,
            icon: self.icon,
            category: self.category,
            handles: self.handles,
            launch: self.launch.resolve()?,
        })
    }
}

/// Parsea una entrada de app desde texto TOML. Devuelve `None` si no
/// parsea o si la `[launch]` no nombra ningún modo (`exec`/`action`/`wasm`).
#[cfg(feature = "std")]
pub fn parse_entry(toml_src: &str) -> Option<AppEntry> {
    toml::from_str::<AppFile>(toml_src)
        .ok()
        .and_then(AppFile::into_entry)
}

/// Directorio canónico del registro: `~/.config/gioser/apps/`.
#[cfg(feature = "std")]
pub fn apps_dir() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|b| b.config_dir().join("gioser").join("apps"))
}

/// La tabla de apps. Inmutable tras descubrir — recargar = volver a
/// `discover`. Barato: son pocos archivos y no es hot-path.
#[derive(Debug, Clone, Default)]
pub struct AppRegistry {
    entries: Vec<AppEntry>,
}

impl AppRegistry {
    pub fn new(mut entries: Vec<AppEntry>) -> Self {
        // sort_unstable_by para no exigir alloc extra (vive en core).
        entries.sort_unstable_by(|a, b| a.label.cmp(&b.label));
        Self { entries }
    }

    pub fn all(&self) -> &[AppEntry] {
        &self.entries
    }

    pub fn get(&self, id: &str) -> Option<&AppEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Apps que declaran abrir `mime` — para el open-with universal.
    pub fn handlers_for(&self, mime: &str) -> Vec<&AppEntry> {
        self.entries.iter().filter(|e| e.handles_mime(mime)).collect()
    }

    /// Apps de una categoría, en orden de label (para grilla/spotlight).
    pub fn in_category(&self, category: &str) -> Vec<&AppEntry> {
        self.entries
            .iter()
            .filter(|e| e.category.as_deref() == Some(category))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(feature = "std")]
impl AppRegistry {
    /// Descubre del dir canónico. Vacío si no hay config dir o el dir no
    /// existe — la app sigue, sólo sin entradas.
    pub fn discover() -> Self {
        apps_dir().map(Self::from_dir).unwrap_or_default()
    }

    /// Escanea `<dir>/*.toml`. Ignora en silencio los que no parsean
    /// (con una nota a stderr), igual que el resto de los loaders del repo.
    pub fn from_dir(dir: impl AsRef<std::path::Path>) -> Self {
        let dir = dir.as_ref();
        let mut entries = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) != Some("toml") {
                    continue;
                }
                match std::fs::read_to_string(&p) {
                    Ok(src) => match parse_entry(&src) {
                        Some(entry) => entries.push(entry),
                        None => eprintln!("app-bus: {p:?} no declara una app válida"),
                    },
                    Err(err) => eprintln!("app-bus: no se pudo leer {p:?}: {err}"),
                }
            }
        }
        Self::new(entries)
    }
}

/// Siembra manifests por defecto en [`apps_dir`] si todavía no hay
/// ninguno, para que [`AppRegistry::discover`] devuelva las apps del repo
/// en una máquina recién instalada. No pisa nada si ya existe algún
/// `*.toml`. Devuelve cuántos manifests escribió.
#[cfg(feature = "std")]
pub fn seed_default_apps() -> std::io::Result<usize> {
    let Some(dir) = apps_dir() else {
        return Ok(0);
    };
    // Si ya hay manifests, respetar la config del usuario y no tocar nada.
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let ya_hay = rd.flatten().any(|e| {
            e.path().extension().and_then(|s| s.to_str()) == Some("toml")
        });
        if ya_hay {
            return Ok(0);
        }
    }
    std::fs::create_dir_all(&dir)?;

    // (id, label, icono, binario, cuadrante). Los binarios son los nombres
    // de crate ejecutables del workspace; el cuadrante alimenta la grilla.
    const DEFAULTS: &[(&str, &str, &str, &str, &str)] = &[
        ("cosmos", "Cosmos", "✶", "cosmos-app-llimphi", "yachay"),
        ("nada", "Nada", "✎", "nada", "ruway"),
        ("pluma", "Pluma", "✒", "pluma-editor-llimphi", "unanchay"),
        ("nahual", "Nahual", "❖", "nahual-shell-llimphi", "ruway"),
        ("dominium", "Dominium", "◉", "dominium-app-llimphi", "yachay"),
        ("tinkuy", "Tinkuy", "⚛", "tinkuy-llimphi", "yachay"),
        ("takiy", "Takiy", "♪", "takiy-app-llimphi", "ruway"),
        ("media", "Media", "▶", "media-app", "ruway"),
        ("tullpu", "Tullpu", "✦", "tullpu-app-llimphi", "ruway"),
        ("supay", "Supay", "✷", "supay-app-llimphi", "ruway"),
        ("sandokan-monitor", "Monitor", "❤", "sandokan-monitor", "ukupacha"),
    ];

    let mut escritos = 0;
    for (id, label, icon, exec, cat) in DEFAULTS {
        let toml = alloc::format!(
            "id = \"{id}\"\nlabel = \"{label}\"\nicon = \"{icon}\"\ncategory = \"{cat}\"\n\n[launch]\nexec = \"{exec}\"\n"
        );
        std::fs::write(dir.join(alloc::format!("{id}.toml")), toml)?;
        escritos += 1;
    }
    Ok(escritos)
}

// =====================================================================
// Menú global (Archivo / Editar / Ayuda …)
// =====================================================================

/// Un ítem de menú. `command` es el id que la app entiende: la barra lo
/// re-emite por el [`Bus`] como [`BusEvent::Command`] y la app focuseada
/// lo ejecuta. `shortcut` es sólo para pintar (la app sigue dueña del
/// binding real).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuItem {
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub shortcut: Option<String>,
    /// Glifo (unicode) opcional para el gutter de íconos del dropdown.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Dibujar un separador *antes* de este ítem.
    #[serde(default)]
    pub separator_before: bool,
}

fn yes() -> bool {
    true
}

impl MenuItem {
    pub fn new(label: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            command: command.into(),
            shortcut: None,
            icon: None,
            enabled: true,
            separator_before: false,
        }
    }

    pub fn shortcut(mut self, s: impl Into<String>) -> Self {
        self.shortcut = Some(s.into());
        self
    }

    /// Glifo del gutter izquierdo (unicode).
    pub fn icon(mut self, glyph: impl Into<String>) -> Self {
        self.icon = Some(glyph.into());
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn separated(mut self) -> Self {
        self.separator_before = true;
        self
    }
}

/// Un menú raíz (Archivo, Editar, Ayuda…) con sus ítems.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Menu {
    pub label: String,
    pub items: Vec<MenuItem>,
}

impl Menu {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            items: Vec::new(),
        }
    }

    pub fn item(mut self, it: MenuItem) -> Self {
        self.items.push(it);
        self
    }
}

/// El menú global completo de una app. La app lo declara; la barra de
/// launcher lo adopta (y entonces la app no lo pinta en su ventana).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppMenu {
    pub menus: Vec<Menu>,
}

impl AppMenu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn menu(mut self, m: Menu) -> Self {
        self.menus.push(m);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.menus.is_empty()
    }

    /// Esqueleto estándar Archivo/Editar/Ayuda — punto de partida para que
    /// toda app tenga un menú base coherente sin reinventarlo. Los
    /// `command` siguen la convención `menu.<verbo>`; la app mapea los que
    /// le sirven y deja `disabled` los que no.
    pub fn standard() -> Self {
        Self::new()
            .menu(
                Menu::new("Archivo")
                    .item(MenuItem::new("Nuevo", "file.new").shortcut("Ctrl+N"))
                    .item(MenuItem::new("Abrir…", "file.open").shortcut("Ctrl+O"))
                    .item(MenuItem::new("Guardar", "file.save").shortcut("Ctrl+S"))
                    .item(MenuItem::new("Cerrar", "file.close").shortcut("Ctrl+W").separated()),
            )
            .menu(
                Menu::new("Editar")
                    .item(MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z"))
                    .item(MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Y"))
                    .item(MenuItem::new("Cortar", "edit.cut").shortcut("Ctrl+X").separated())
                    .item(MenuItem::new("Copiar", "edit.copy").shortcut("Ctrl+C"))
                    .item(MenuItem::new("Pegar", "edit.paste").shortcut("Ctrl+V")),
            )
            .menu(
                Menu::new("Ayuda")
                    .item(MenuItem::new("Atajos", "help.shortcuts").shortcut("F1"))
                    .item(MenuItem::new("Acerca de", "help.about")),
            )
    }
}

// =====================================================================
// Launcher — la instrucción de ejecución abstracta
// =====================================================================

/// Por qué no se pudo lanzar una app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    /// Este `Launcher` no maneja el modo de la app (p.ej. un host que no
    /// instancia WASM, o wawa que no spawnea procesos del host).
    Unsupported,
    /// El lanzamiento falló; mensaje libre.
    Failed(String),
}

/// La *instrucción de ejecución* abstracta. El motor de launcher
/// (`launcher-core`/`launcher-llimphi`) llama a `launch` y no sabe en qué
/// entorno corre — host, shuma o wawa cada uno trae su impl.
pub trait Launcher {
    fn launch(&self, app: &AppEntry) -> Result<(), LaunchError>;
}

/// Launcher del host: spawnea binarios vía `std::process`. No maneja
/// `Action`/`Wasm` (devuelve `Unsupported` — esos los resuelve el chasis).
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessLauncher;

#[cfg(feature = "std")]
impl Launcher for ProcessLauncher {
    fn launch(&self, app: &AppEntry) -> Result<(), LaunchError> {
        match app.spawn() {
            Ok(Some(_child)) => Ok(()),
            Ok(None) => Err(LaunchError::Unsupported),
            Err(e) => Err(LaunchError::Failed(alloc::string::ToString::to_string(&e))),
        }
    }
}

// =====================================================================
// Bus de eventos (pub/sub in-process) — sólo `std`
// =====================================================================

/// Lo que viaja por el bus. El flujo del menú global: una app toma foco
/// → `AppFocused` + `MenuChanged` → la barra adopta el menú → el usuario
/// clickea un ítem → la barra emite `Command` → la app focuseada lo
/// ejecuta. El dock/spotlight emiten `LaunchRequested` y el shell lo
/// resuelve contra el [`AppRegistry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusEvent {
    /// Una app tomó foco — la barra debería adoptar su menú.
    AppFocused { app_id: String },
    /// El menú de una app cambió (ítems habilitados/labels dinámicos).
    MenuChanged { app_id: String, menu: AppMenu },
    /// Dock/spotlight pidieron lanzar una app por id.
    LaunchRequested { app_id: String },
    /// La barra global disparó un comando del menú hacia la app focuseada.
    Command { app_id: String, command: String },
}

/// Bus pub/sub mínimo y `Send + Sync`: fan-out a todos los suscriptores.
/// Un suscriptor caído (receiver dropeado) se poda en el próximo publish.
/// Clonar el `Bus` comparte el mismo conjunto de suscriptores.
#[cfg(feature = "std")]
#[derive(Clone, Default)]
pub struct Bus {
    subs: std::sync::Arc<std::sync::Mutex<Vec<std::sync::mpsc::Sender<BusEvent>>>>,
}

#[cfg(feature = "std")]
impl Bus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Crea un canal y devuelve su extremo de recepción. El emisor queda
    /// registrado para recibir cada `publish` futuro.
    pub fn subscribe(&self) -> std::sync::mpsc::Receiver<BusEvent> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.subs.lock().unwrap().push(tx);
        rx
    }

    /// Emite a todos los suscriptores vivos. Devuelve cuántos lo recibieron.
    pub fn publish(&self, ev: BusEvent) -> usize {
        let mut subs = self.subs.lock().unwrap();
        subs.retain(|tx| tx.send(ev.clone()).is_ok());
        subs.len()
    }
}

// =====================================================================
// Tests (corren con default features = std)
// =====================================================================

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn parse_exec_entry() {
        let src = r#"
            id = "cosmos"
            label = "Cosmos"
            icon = "✶"
            category = "yachay"
            handles = ["application/x-cosmos-chart"]
            [launch]
            exec = "cosmos-app-llimphi"
            args = ["--release"]
        "#;
        let e = parse_entry(src).expect("parsea");
        assert_eq!(e.id, "cosmos");
        assert_eq!(e.icon.as_deref(), Some("✶"));
        assert!(e.handles_mime("application/x-cosmos-chart"));
        assert_eq!(
            e.launch,
            Launch::Exec {
                program: "cosmos-app-llimphi".into(),
                args: vec!["--release".into()],
            }
        );
    }

    #[test]
    fn parse_action_and_wasm() {
        let a = parse_entry("id='s'\nlabel='Shell'\n[launch]\naction='focus:shell'").unwrap();
        assert_eq!(a.launch, Launch::Action("focus:shell".into()));
        let w = parse_entry("id='h'\nlabel='Hola'\n[launch]\nwasm='deadbeef'").unwrap();
        assert_eq!(w.launch, Launch::Wasm { bytecode_hex: "deadbeef".into() });
    }

    #[test]
    fn launch_sin_modo_es_none() {
        assert!(parse_entry("id='x'\nlabel='X'\n[launch]").is_none());
    }

    #[test]
    fn registry_from_dir_y_consultas() {
        let dir =
            std::env::temp_dir().join(format!("app-bus-test-{}-{}", std::process::id(), "reg"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("cosmos.toml"),
            "id='cosmos'\nlabel='Cosmos'\ncategory='yachay'\nhandles=['x/chart']\n[launch]\nexec='cosmos-app-llimphi'",
        )
        .unwrap();
        std::fs::write(
            dir.join("nada.toml"),
            "id='nada'\nlabel='Nada'\ncategory='ruway'\n[launch]\nexec='nada'",
        )
        .unwrap();
        std::fs::write(dir.join("roto.toml"), "no es toml válido = =").unwrap();

        let reg = AppRegistry::from_dir(&dir);
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.all()[0].id, "cosmos");
        assert_eq!(reg.get("nada").unwrap().label, "Nada");
        assert_eq!(reg.handlers_for("x/chart").len(), 1);
        assert_eq!(reg.in_category("yachay").len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn menu_estandar_y_builder() {
        let m = AppMenu::standard();
        assert_eq!(m.menus.len(), 3);
        assert_eq!(m.menus[0].label, "Archivo");
        let custom = AppMenu::new().menu(
            Menu::new("Carta").item(MenuItem::new("Duplicar", "carta.dup").shortcut("Ctrl+D")),
        );
        assert_eq!(custom.menus[0].items[0].command, "carta.dup");
    }

    #[test]
    fn menu_roundtrip_serde() {
        let m = AppMenu::standard();
        let json = serde_json::to_string(&m).unwrap();
        let back: AppMenu = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn process_launcher_unsupported_para_action() {
        // Action no es del host → Unsupported (no intenta spawnear).
        let app = AppEntry {
            id: "s".into(),
            label: "Shell".into(),
            icon: None,
            category: None,
            launch: Launch::Action("focus:shell".into()),
            handles: Vec::new(),
        };
        assert_eq!(ProcessLauncher.launch(&app), Err(LaunchError::Unsupported));
    }

    #[test]
    fn bus_fanout_y_poda() {
        let bus = Bus::new();
        let a = bus.subscribe();
        let b = bus.subscribe();
        let n = bus.publish(BusEvent::LaunchRequested {
            app_id: "cosmos".into(),
        });
        assert_eq!(n, 2);
        assert!(matches!(a.recv().unwrap(), BusEvent::LaunchRequested { .. }));
        assert!(matches!(b.recv().unwrap(), BusEvent::LaunchRequested { .. }));
        drop(a);
        let n = bus.publish(BusEvent::AppFocused {
            app_id: "nada".into(),
        });
        assert_eq!(n, 1);
    }
}
