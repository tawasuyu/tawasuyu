//! `app-bus` — el cimiento del menú de aplicaciones de tawasuyu.
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
//!    Se descubre de `~/.config/tawasuyu/apps/*.toml` (feature `std`).
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

/// Cómo se enciende una app. Los tres mundos de tawasuyu:
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
    /// `true` si la app declara saber abrir `mime`. Un handle que termina en
    /// `/` (p.ej. `"image/"`) actúa como **prefijo**: matchea cualquier mime
    /// que arranque con él (`image/png`, `image/webp`…). El resto es match
    /// exacto. Así una app declara una familia entera sin enumerar cada mime.
    pub fn handles_mime(&self, mime: &str) -> bool {
        self.handles
            .iter()
            .any(|h| h == mime || (h.ends_with('/') && mime.starts_with(h.as_str())))
    }
}

/// El catálogo **por defecto** de la suite: las apps con su `Launch::Exec` y
/// los mimes que abren (`handles`), para que el open-with universal funcione
/// sin que el usuario tenga que sembrar config. `AppRegistry::with_defaults`
/// las fusiona con lo descubierto en disco. Es `no_std + alloc`.
pub fn default_entries() -> Vec<AppEntry> {
    // (id, label, icono, exec, categoría, handles)
    const D: &[(&str, &str, &str, &str, &str, &[&str])] = &[
        ("nada", "Nada", "≡", "nada", "ruway",
            &["text/", "application/json", "application/toml", "inode/x-empty"]),
        ("pluma", "Pluma", "¶", "pluma-editor-llimphi", "unanchay",
            &["text/markdown", "text/plain"]),
        ("pluma-notebook", "Pluma Notebook", "▣", "pluma-notebook-llimphi", "unanchay",
            &["application/x-pluma-notebook"]),
        ("tullpu", "Tullpu", "✦", "tullpu-app-llimphi", "ruway", &["image/"]),
        ("takiy", "Takiy", "♪", "takiy-app-llimphi", "ruway", &["audio/"]),
        ("media", "Media", "▶", "media-app", "ruway", &["video/", "audio/"]),
        ("media-tube", "Media Tube", "▷", "media-tube", "ruway", &[]),
        ("cosmos", "Cosmos", "✶", "cosmos-app-llimphi", "yachay",
            &["application/x-cosmos-chart"]),
        ("dominium", "Dominium", "◉", "dominium-app-llimphi", "yachay",
            &["application/x-dominium"]),
        ("tinkuy", "Tinkuy", "⚛", "tinkuy-llimphi", "yachay", &["application/x-tinkuy"]),
        ("chaka", "Chaka", "◫", "chaka-app-llimphi", "unanchay", &["application/x-chaka"]),
        ("nakui", "Nakui", "Σ", "nakui-sheet-llimphi", "yachay",
            &["text/csv", "application/x-nakui"]),
        ("puriy", "Puriy", "◎", "puriy", "unanchay", &["text/html"]),
        ("raymi", "Raymi", "◷", "raymi-app", "ruway", &["text/calendar"]),
        ("supay", "Supay", "✷", "supay-app-llimphi", "ruway", &[]),
        ("sandokan-monitor", "Monitor", "❤", "sandokan-monitor", "ukupacha", &[]),
        ("nahual", "Nahual", "❖", "nahual-shell-llimphi", "ruway", &["inode/directory"]),
        ("mirada-panel", "Mirada Panel", "▭", "mirada-llimphi", "ukupacha", &[]),
        ("panel-control", "Panel de control", "⚙", "wawa-panel", "ukupacha", &[]),
    ];
    D.iter()
        .map(|(id, label, icon, exec, cat, handles)| AppEntry {
            id: String::from(*id),
            label: String::from(*label),
            icon: Some(String::from(*icon)),
            category: Some(String::from(*cat)),
            launch: Launch::Exec { program: String::from(*exec), args: Vec::new() },
            handles: handles.iter().map(|h| String::from(*h)).collect(),
        })
        .collect()
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

    /// **Open-with out-of-process**: abre `target` con esta app. Para `Exec`,
    /// spawnea el binario sustituyendo el placeholder `%f`/`%u` en los args
    /// por `target`; si ningún arg lo trae, agrega `target` como último
    /// argumento (semántica estilo freedesktop `Exec=app %f`). `Action`/`Wasm`
    /// devuelven `Ok(None)`: el target lo despacha el host (chasis a una vista
    /// in-process, o kernel de wawa a una app WASM), no un proceso del SO.
    pub fn open(&self, target: &str) -> std::io::Result<Option<std::process::Child>> {
        match &self.launch {
            Launch::Exec { program, args } => std::process::Command::new(program)
                .args(expand_target(args, target))
                .spawn()
                .map(Some),
            Launch::Action(_) | Launch::Wasm { .. } => Ok(None),
        }
    }
}

/// Sustituye los placeholders `%f`/`%u` por `target` en `args`. Si ninguno
/// aparece, agrega `target` como argumento final — la convención de
/// freedesktop (`Exec=app %f`) que entiende cualquier "abrir con".
#[cfg(feature = "std")]
pub fn expand_target(args: &[String], target: &str) -> Vec<String> {
    let mut sustituido = false;
    let mut out: Vec<String> = args
        .iter()
        .map(|a| {
            if a.contains("%f") || a.contains("%u") {
                sustituido = true;
                a.replace("%f", target).replace("%u", target)
            } else {
                a.clone()
            }
        })
        .collect();
    if !sustituido {
        out.push(target.to_string());
    }
    out
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

/// Directorio canónico del registro: `~/.config/tawasuyu/apps/`.
#[cfg(feature = "std")]
pub fn apps_dir() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|b| b.config_dir().join("tawasuyu").join("apps"))
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

    /// Como [`discover`](Self::discover) pero **fusionando** las apps de la suite
    /// tawasuyu (`~/.config/tawasuyu/apps/*.toml`) con las `.desktop` del sistema
    /// (XDG). Dedup por label en minúsculas: la entrada tawasuyu gana sobre la del
    /// sistema con el mismo nombre (así «Media» de tawasuyu tapa un `media.desktop`
    /// del sistema). Orden alfabético por label. Es lo que un launcher "normal"
    /// (rofi/wofi) descubre, más la suite propia.
    pub fn discover_merged() -> Self {
        use std::collections::HashSet;
        let mut entries = Self::discover().entries;
        let mut labels: HashSet<String> =
            entries.iter().map(|e| e.label.to_lowercase()).collect();
        for e in discover_desktop_entries() {
            if labels.insert(e.label.to_lowercase()) {
                entries.push(e);
            }
        }
        Self::new(entries)
    }

    /// El registro **con el catálogo por defecto de la suite** ([`default_entries`])
    /// fusionado con lo descubierto en disco (`~/.config/tawasuyu/apps/*.toml`) y
    /// las `.desktop` del sistema. Para apps presentes en ambos lados: el
    /// `launch`/`label` del usuario gana, pero los `handles` se **unen** (no se
    /// pierden los mimes builtin). Es lo que un front quiere por defecto: el
    /// open-with funciona sin sembrar nada.
    pub fn with_defaults() -> Self {
        use std::collections::{BTreeMap, HashSet};
        let mut by_id: BTreeMap<String, AppEntry> =
            default_entries().into_iter().map(|e| (e.id.clone(), e)).collect();
        for d in Self::discover().entries {
            match by_id.get_mut(&d.id) {
                Some(base) => {
                    for h in &d.handles {
                        if !base.handles.contains(h) {
                            base.handles.push(h.clone());
                        }
                    }
                    base.launch = d.launch;
                    base.label = d.label;
                    if d.icon.is_some() {
                        base.icon = d.icon;
                    }
                    if d.category.is_some() {
                        base.category = d.category;
                    }
                }
                None => {
                    by_id.insert(d.id.clone(), d);
                }
            }
        }
        let mut entries: Vec<AppEntry> = by_id.into_values().collect();
        let mut labels: HashSet<String> = entries.iter().map(|e| e.label.to_lowercase()).collect();
        for e in discover_desktop_entries() {
            if labels.insert(e.label.to_lowercase()) {
                entries.push(e);
            }
        }
        Self::new(entries)
    }

    /// **Open-with universal**: elige el primer handler de `mime` (orden de
    /// label) y le abre `target` out-of-process vía [`AppEntry::open`].
    /// Devuelve el `AppEntry` elegido y su `Child` (o `None` en el child si
    /// la app es `Action`/`Wasm`, que despacha el host). `Ok(None)` si ninguna
    /// app registrada declara abrir ese mime — el caller cae a su visor
    /// in-process por defecto (p.ej. el `viewer_registry` de nahual-shell).
    pub fn open_with(
        &self,
        mime: &str,
        target: &str,
    ) -> std::io::Result<Option<(&AppEntry, Option<std::process::Child>)>> {
        match self.handlers_for(mime).into_iter().next() {
            Some(entry) => Ok(Some((entry, entry.open(target)?))),
            None => Ok(None),
        }
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

// ----- descubrimiento de .desktop del sistema (XDG) -----

/// Los directorios `applications/` del estándar XDG, en orden de prioridad
/// (usuario primero, sistema después). Un `.desktop` de mayor prioridad tapa a
/// otro con el mismo nombre de archivo.
#[cfg(feature = "std")]
fn xdg_application_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let mut dirs = Vec::new();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    if let Some(home) = data_home {
        dirs.push(home.join("applications"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for d in data_dirs.split(':').filter(|s| !s.is_empty()) {
        dirs.push(PathBuf::from(d).join("applications"));
    }
    dirs
}

/// Descubre las aplicaciones `.desktop` instaladas en el sistema (XDG) como
/// [`AppEntry`] con [`Launch::Exec`]. Dedup por id XDG (ruta relativa con
/// `/`→`-`): un dir de más prioridad tapa al de menos. Ignora las marcadas
/// `NoDisplay`/`Hidden` y las que no son `Type=Application`. `MimeType` alimenta
/// `handles` (open-with universal).
#[cfg(feature = "std")]
pub fn discover_desktop_entries() -> Vec<AppEntry> {
    use std::collections::HashSet;
    let mut entries = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for root in xdg_application_dirs() {
        collect_desktop_dir(&root, &root, &mut seen, &mut entries);
    }
    entries
}

#[cfg(feature = "std")]
fn collect_desktop_dir(
    root: &std::path::Path,
    dir: &std::path::Path,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<AppEntry>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let path = e.path();
        if path.is_dir() {
            collect_desktop_dir(root, &path, seen, out);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
            continue;
        }
        let id = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('/', "-");
        if !seen.insert(id.clone()) {
            continue; // ya lo tapó un directorio de más prioridad
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(entry) = parse_desktop_entry(&text, &id) {
                out.push(entry);
            }
        }
    }
}

/// Extrae un [`AppEntry`] del texto de un `.desktop`. `None` si no es lanzable
/// o está oculta. El `Exec` se parte en programa + args quitando los códigos de
/// campo (`%f`/`%U`/…).
#[cfg(feature = "std")]
fn parse_desktop_entry(text: &str, id: &str) -> Option<AppEntry> {
    let mut in_entry = false;
    let mut name = None::<String>;
    let mut exec = None::<String>;
    let mut kind = None::<String>;
    let mut icon = None::<String>;
    let mut mimes: Vec<String> = Vec::new();
    let mut categories = String::new();
    let mut no_display = false;
    let mut hidden = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            // El `Name` plano (no las variantes `Name[es]`, que traen `[`).
            "Name" if name.is_none() => name = Some(value.into()),
            "Exec" => exec = Some(value.into()),
            "Type" => kind = Some(value.into()),
            "Icon" => icon = Some(value.into()),
            "MimeType" => {
                mimes = value
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            }
            "Categories" => categories = value.to_string(),
            "NoDisplay" => no_display = value == "true",
            "Hidden" => hidden = value == "true",
            _ => {}
        }
    }
    if no_display || hidden || kind.as_deref() != Some("Application") {
        return None;
    }
    let label = name?;
    let (program, args) = split_exec(&exec?)?;
    if label.is_empty() || program.is_empty() {
        return None;
    }
    Some(AppEntry {
        id: String::from(id),
        label,
        // Nombre de ícono freedesktop (no ruta); el launcher decide cómo
        // pintarlo. pata cae a un glyph genérico si no sabe resolverlo.
        icon,
        category: Some(categoria_primaria(&categories)),
        launch: Launch::Exec { program, args },
        handles: mimes,
    })
}

/// Mapea el campo `Categories=` de un `.desktop` (lista `;`-separada, estándar
/// freedesktop) a UNA categoría primaria legible en español, para agrupar el
/// menú. Toma la primera categoría principal que reconoce; si ninguna, "Otros".
#[cfg(feature = "std")]
fn categoria_primaria(categories: &str) -> String {
    // (token freedesktop, etiqueta) en orden de prioridad.
    const MAIN: &[(&str, &str)] = &[
        ("AudioVideo", "Multimedia"),
        ("Audio", "Multimedia"),
        ("Video", "Multimedia"),
        ("Development", "Desarrollo"),
        ("Education", "Educación"),
        ("Game", "Juegos"),
        ("Graphics", "Gráficos"),
        ("Network", "Internet"),
        ("Office", "Oficina"),
        ("Science", "Ciencia"),
        ("Settings", "Configuración"),
        ("System", "Sistema"),
        ("Utility", "Accesorios"),
    ];
    let toks: Vec<&str> = categories.split(';').filter(|s| !s.is_empty()).collect();
    for (tok, label) in MAIN {
        if toks.iter().any(|t| t.eq_ignore_ascii_case(tok)) {
            return label.to_string();
        }
    }
    "Otros".to_string()
}

/// Parte el `Exec` de un `.desktop` en (programa, args), quitando los códigos de
/// campo (`%f`, `%U`, `%i`, …; `%%`→`%`). División por espacios (sin honrar
/// comillas: alcanza para lanzar — los args entrecomillados con espacios en
/// `Exec` son raros y se lanzan igual por el shell del usuario si hiciera falta).
#[cfg(feature = "std")]
fn split_exec(exec: &str) -> Option<(String, Vec<String>)> {
    let mut tokens: Vec<String> = Vec::new();
    for raw in exec.split_whitespace() {
        // Un token que es exactamente un código de campo (`%f`, `%U`, …) se
        // descarta entero; `%%` es un `%` literal.
        if raw.len() == 2 && raw.starts_with('%') && raw != "%%" {
            continue;
        }
        tokens.push(raw.replace("%%", "%"));
    }
    let mut it = tokens.into_iter();
    let program = it.next()?;
    Some((program, it.collect()))
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

/// **Reveal in nahual** — el recíproco del open-with: abre el front universal
/// `nahual` en el directorio que contiene `path` (o en `path` si ya es un dir),
/// para que cualquier app pueda "volver al explorador". Spawnea
/// `nahual-shell-llimphi <dir>`; `$NAHUAL_BIN` lo override (útil en dev). Un
/// fallo al spawnear se propaga para que el caller lo reporte.
#[cfg(feature = "std")]
pub fn reveal(path: impl AsRef<std::path::Path>) -> std::io::Result<std::process::Child> {
    let path = path.as_ref();
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    let bin = std::env::var("NAHUAL_BIN").unwrap_or_else(|_| "nahual-shell-llimphi".into());
    std::process::Command::new(bin).arg(dir).spawn()
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
    fn handles_mime_exacto_y_prefijo() {
        let img = AppEntry {
            id: "tullpu".into(),
            label: "Tullpu".into(),
            icon: None,
            category: None,
            launch: Launch::Exec { program: "tullpu-app-llimphi".into(), args: vec![] },
            handles: vec!["image/".into(), "text/x-rust".into()],
        };
        // Prefijo: cualquier image/* matchea.
        assert!(img.handles_mime("image/png"));
        assert!(img.handles_mime("image/webp"));
        // Exacto: sólo el mime declarado.
        assert!(img.handles_mime("text/x-rust"));
        assert!(!img.handles_mime("text/plain"));
        assert!(!img.handles_mime("audio/mp3"));
    }

    #[test]
    fn registry_de_defaults_rutea_por_mime() {
        let reg = AppRegistry::new(default_entries());
        // image/png → tullpu (editor pixel).
        let png = reg.handlers_for("image/png");
        assert!(png.iter().any(|e| e.id == "tullpu"));
        // audio/mp3 → takiy y media.
        let mp3: Vec<&str> = reg.handlers_for("audio/mpeg").iter().map(|e| e.id.as_str()).collect();
        assert!(mp3.contains(&"takiy"));
        assert!(mp3.contains(&"media"));
        // text/x-rust → nada (prefijo text/).
        assert!(reg.handlers_for("text/x-rust").iter().any(|e| e.id == "nada"));
        // text/html → puriy.
        assert!(reg.handlers_for("text/html").iter().any(|e| e.id == "puriy"));
        // Un mime sin handler no rompe.
        assert!(reg.handlers_for("application/x-desconocido").is_empty());
    }

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
    fn manifiestos_de_ejemplo_parsean_y_resuelven_handlers() {
        // Los manifiestos de `assets/apps/` (las apps reales de la suite que se
        // copian a ~/.config/tawasuyu/apps/) deben parsear y declarar sus mimes.
        // Canario del formato: si cambia el esquema, esto avisa.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/apps");
        let reg = AppRegistry::from_dir(&dir);
        assert_eq!(reg.len(), 2, "media + nada");
        // media abre video/audio; nada, texto/código.
        assert_eq!(reg.handlers_for("video/mp4")[0].id, "media");
        assert_eq!(reg.handlers_for("text/x-rust")[0].id, "nada");
        // El exec lleva el placeholder freedesktop.
        let media = reg.get("media").unwrap();
        assert_eq!(
            media.launch,
            Launch::Exec {
                program: "media-app".into(),
                args: vec!["%f".into()],
            }
        );
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

    // ===== open-with out-of-process =====

    #[test]
    fn expand_target_sustituye_placeholder() {
        let args = vec!["--open".to_string(), "%f".to_string()];
        assert_eq!(
            expand_target(&args, "/tmp/x.png"),
            vec!["--open".to_string(), "/tmp/x.png".to_string()]
        );
        // `%u` también; y substitución embebida en un arg compuesto.
        let args = vec!["url=%u".to_string()];
        assert_eq!(expand_target(&args, "http://a"), vec!["url=http://a".to_string()]);
    }

    #[test]
    fn expand_target_agrega_si_no_hay_placeholder() {
        let args = vec!["--flag".to_string()];
        assert_eq!(
            expand_target(&args, "/tmp/x.png"),
            vec!["--flag".to_string(), "/tmp/x.png".to_string()]
        );
    }

    #[test]
    fn open_with_sin_handler_devuelve_none() {
        let reg = AppRegistry::new(vec![]);
        assert!(reg.open_with("image/png", "/tmp/x.png").unwrap().is_none());
    }

    #[test]
    fn open_with_spawnea_handler_y_le_pasa_el_target() {
        use std::io::Read;
        // Archivo donde el "handler" escribirá el target que recibió.
        let out =
            std::env::temp_dir().join(format!("app-bus-openwith-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&out);

        // Handler = sh que escribe $1 (el target expandido en %f) al archivo.
        let entry = AppEntry {
            id: "writer".into(),
            label: "Writer".into(),
            icon: None,
            category: None,
            launch: Launch::Exec {
                program: "sh".into(),
                args: vec![
                    "-c".into(),
                    format!("printf '%s' \"$1\" > {}", out.display()),
                    "_".into(),
                    "%f".into(),
                ],
            },
            handles: vec!["image/png".into()],
        };
        let reg = AppRegistry::new(vec![entry]);

        let (chosen, child) = reg
            .open_with("image/png", "TARGET-123")
            .unwrap()
            .expect("debe haber handler para image/png");
        assert_eq!(chosen.id, "writer");
        child.expect("Exec debe spawnear un Child").wait().unwrap();

        let mut s = String::new();
        std::fs::File::open(&out)
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        assert_eq!(s, "TARGET-123", "el handler recibió el target en %f");
        let _ = std::fs::remove_file(&out);
    }
}
