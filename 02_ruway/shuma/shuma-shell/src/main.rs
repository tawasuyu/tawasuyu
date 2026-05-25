//! `shuma-shell` — el shell de brahman, ejecutando de verdad.
//!
//! El shell trabaja *dentro de una sesión* ([`shuma_session::WorkSession`]):
//! un directorio actual —que es además el identificador de aislamiento—,
//! el historial de comandos ejecutados y los grupos reutilizables.
//!
//! ```text
//!   ┌─ estado · cwd · aislamiento ──────────────────────┐
//!   │ [RUN]   │   comandos ejecutados + su salida   │ [SENS] │
//!   │ grupos  │   (streaming en vivo)               │ monit. │
//!   └─ prompt inteligente ─────────────────────────────┘
//! ```
//!
//! El input se analiza con `shuma-line` (resaltado + autocompletado);
//! al ejecutar, `shuma-exec` lanza el comando y transmite su salida
//! línea a línea, que se vuelca en el panel central. El cerebro vive en
//! crates agnósticos — este binario sólo es el frontend GPUI.

use std::panic;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use std::collections::HashMap;

use gpui::{
    div, point, prelude::*, px, App, Application, Bounds, Context, CursorStyle, Element, ElementId,
    FocusHandle, GlobalElementId, Hsla, InspectorElementId, IntoElement, KeyDownEvent, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Render,
    ScrollHandle, SharedString, Style, Window, WindowBounds, WindowOptions,
};
use mirada_brain::ctl::{default_socket_path, send_request};
use mirada_brain::{CtlReply, CtlRequest, DesktopAction, WindowLine};
use nahual_launcher::launch_app;
use nahual_theme::Theme;
use shuma_config::{Config, DedupPolicy as CfgDedup};
use shuma_exec::{run as exec_run, CommandSpec, Exec, RunEvent, RunHandle, StageSpec};
use shuma_remote_exec::{run as remote_exec_run, RemoteRunHandle};
use shuma_history::{DedupPolicy as HistDedup, Entry as HistoryEntry, History, Nav as HistoryNav};
use shuma_line::{CompletionKind, CompletionSource, LineState, TokenKind};
use shuma_session::{CommandRun, RunId, RunStatus, Stream, WorkSession};
use shuma_sysmon::{Snapshot, SystemSampler};

/// Cuántas muestras guarda la curva de cada monitor.
const HISTORY: usize = 80;
/// Alto de la barra del modo launcher, en píxeles.
const LAUNCHER_BAR_H: f32 = 40.0;
/// Alto del cajón de resultados del modo launcher cuando se despliega.
const LAUNCHER_DRAWER_H: f32 = 320.0;
/// Archivos/directorios que delatan la estructura de un proyecto.
const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "go.mod",
    "Makefile",
    "pyproject.toml",
    "pom.xml",
    "build.gradle",
];

/// Marcadores de proyecto presentes en `dir`.
fn markers_in(dir: &str) -> Vec<String> {
    let base = std::path::Path::new(dir);
    PROJECT_MARKERS
        .iter()
        .filter(|m| base.join(m).exists())
        .map(|m| m.to_string())
        .collect()
}

/// Segundo Unix actual.
fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Rama de git activa para `cwd` — `None` si no estamos en un repo (o si
/// HEAD está detached y no podemos resolverlo trivialmente). Implementación
/// minimalista por archivo: sube por los padres buscando `.git`, lee
/// `HEAD` y extrae `refs/heads/<rama>`. No usa libgit2 ni shells.
fn git_branch(cwd: &str) -> Option<String> {
    let mut dir = std::path::PathBuf::from(cwd);
    let git_dir = loop {
        let candidate = dir.join(".git");
        if candidate.exists() {
            break candidate;
        }
        if !dir.pop() {
            return None;
        }
    };
    // `.git` puede ser un archivo (worktrees/submódulos) con `gitdir: …`,
    // o un directorio con `HEAD` dentro.
    let head_path = if git_dir.is_file() {
        let s = std::fs::read_to_string(&git_dir).ok()?;
        let target = s.strip_prefix("gitdir:")?.trim();
        std::path::PathBuf::from(target).join("HEAD")
    } else {
        git_dir.join("HEAD")
    };
    let head = std::fs::read_to_string(head_path).ok()?;
    let head = head.trim();
    head.strip_prefix("ref: refs/heads/").map(|b| b.to_string())
}

/// Índice de grupo (0-based) de una tecla de función `f1`..`f8`.
fn fkey_index(key: &str) -> Option<usize> {
    let n: usize = key.strip_prefix('f')?.parse().ok()?;
    (1..=8).contains(&n).then_some(n - 1)
}

/// Pregunta a carmen, por su socket de control, la lista de ventanas
/// abiertas. `None` si el compositor no está o respondió otra cosa.
fn poll_ctl_windows() -> Option<Vec<WindowLine>> {
    match send_request(&default_socket_path(), &CtlRequest::ListWindows) {
        Ok(CtlReply::Windows(w)) => Some(w),
        _ => None,
    }
}

/// Pide a carmen que enfoque una ventana del escritorio.
fn focus_window(id: u64) {
    let _ = send_request(
        &default_socket_path(),
        &CtlRequest::Do(DesktopAction::FocusWindow(id)),
    );
}

/// Quita las comillas exteriores de un argumento (`"hola"` → `hola`).
fn unquote(arg: &str) -> String {
    let b = arg.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        let inner = &arg[1..arg.len() - 1];
        if b[0] == b'"' {
            inner.replace("\\\"", "\"").replace("\\\\", "\\")
        } else {
            inner.to_string()
        }
    } else {
        arg.to_string()
    }
}

/// Decide cómo ejecutar una línea. Si es un pipe «simple» —sólo comandos,
/// argumentos y `|`, sin `$`, redirecciones, operadores ni globs— brahman
/// la ejecuta **directo**, conectando los procesos él mismo. Si tiene
/// sintaxis que el modo directo aún no absorbe, cae a `bash -c`: bash
/// queda como un parser de sintaxis, no como el ejecutor por defecto.
fn plan_exec(line: &str) -> Exec {
    use shuma_line::TokenKind::*;
    let tokens = shuma_line::tokenize(line, shuma_line::Dialect::Bash);
    let simple = !tokens.is_empty()
        && tokens.iter().all(|t| {
            matches!(t.kind, Command | Argument | Flag | StringLit | Pipe | Whitespace)
                && !t.text.contains(['*', '?', '[', ']', '{', '}'])
                && !t.text.starts_with('~')
        });
    if simple {
        let pipeline = shuma_line::split_pipeline(&tokens);
        let mut stages = Vec::new();
        for st in &pipeline.stages {
            match &st.command {
                Some(cmd) => stages.push(StageSpec {
                    program: cmd.clone(),
                    args: st.args.iter().map(|a| unquote(a)).collect(),
                }),
                // Una etapa sin comando (línea incompleta) → al shell.
                None => return Exec::Shell { line: line.into(), program: "bash".into() },
            }
        }
        if !stages.is_empty() {
            return Exec::Direct { stages };
        }
    }
    Exec::Shell { line: line.into(), program: "bash".into() }
}

// =====================================================================
// Fuente de autocompletado.
// =====================================================================

struct ShellCompletionSource {
    commands: Vec<String>,
    cwd: String,
    /// Flags por comando cargadas de `~/.config/shuma/completions/*.toml`.
    /// El método `flags()` las suma a la tabla estática del motor.
    flag_db: HashMap<String, Vec<String>>,
}

impl ShellCompletionSource {
    fn scan(cwd: String) -> Self {
        let mut commands = Vec::new();
        if let Ok(path) = std::env::var("PATH") {
            for dir in path.split(':') {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for e in entries.flatten() {
                        if let Some(name) = e.file_name().to_str() {
                            commands.push(name.to_string());
                        }
                    }
                }
            }
        }
        commands.sort();
        commands.dedup();
        // Si el usuario tiene un dir de completions, lo cargamos una vez
        // al arrancar — son archivos pequeños y leerlos por demanda
        // añadiría latencia visible al primer Tab.
        let flag_db = shuma_config::Config::completions_dir()
            .map(|dir| {
                shuma_config::CommandCompletion::load_all(&dir)
                    .into_iter()
                    .map(|(cmd, c)| (cmd, c.flags))
                    .collect()
            })
            .unwrap_or_default();
        Self { commands, cwd, flag_db }
    }
}

impl CompletionSource for ShellCompletionSource {
    fn commands(&self) -> Vec<String> {
        self.commands.clone()
    }

    fn paths(&self, prefix: &str) -> Vec<String> {
        let (dir, partial) = match prefix.rfind('/') {
            Some(i) => (&prefix[..=i], &prefix[i + 1..]),
            None => ("", prefix),
        };
        // Una ruta relativa se resuelve contra el cwd de la sesión.
        let base = if dir.starts_with('/') {
            dir.to_string()
        } else if dir.is_empty() {
            self.cwd.clone()
        } else {
            format!("{}/{}", self.cwd, dir)
        };
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&base) {
            for e in entries.flatten() {
                if let Some(name) = e.file_name().to_str() {
                    if name.starts_with(partial) {
                        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                        let slash = if is_dir { "/" } else { "" };
                        out.push(format!("{dir}{name}{slash}"));
                    }
                }
            }
        }
        out.sort();
        out
    }

    fn flags(&self, command: &str) -> Vec<String> {
        // Mergeo: el catálogo estático del motor + lo declarado por el
        // usuario en `~/.config/shuma/completions/<cmd>.toml`. Si hay
        // duplicados, `complete()` los dedup-ea al final.
        let mut out: Vec<String> = shuma_line::flag_hints(command)
            .iter()
            .map(|s| s.to_string())
            .collect();
        if let Some(user) = self.flag_db.get(command) {
            out.extend(user.iter().cloned());
        }
        out
    }
}

// =====================================================================
// CurveElement — la curva de un monitor.
// =====================================================================

struct CurveElement {
    values: Vec<f32>,
    color: Hsla,
}

impl CurveElement {
    fn new(values: Vec<f32>, color: Hsla) -> Self {
        Self { values, color }
    }
}

impl IntoElement for CurveElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CurveElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        style.size.height = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _layout: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) {
        let n = self.values.len();
        if n < 2 {
            return;
        }
        let ox: f32 = bounds.origin.x.into();
        let oy: f32 = bounds.origin.y.into();
        let bw: f32 = bounds.size.width.into();
        let bh: f32 = bounds.size.height.into();
        let mut pb = PathBuilder::stroke(px(1.6));
        for (i, v) in self.values.iter().enumerate() {
            let x = ox + bw * (i as f32 / (n - 1) as f32);
            let y = oy + bh - (v.clamp(0.0, 100.0) / 100.0) * bh;
            let p = point(px(x), px(y));
            if i == 0 {
                pb.move_to(p);
            } else {
                pb.line_to(p);
            }
        }
        if let Ok(path) = pb.build() {
            window.paint_path(path, self.color);
        }
    }
}

// =====================================================================
// El shell.
// =====================================================================

/// Qué panel lateral está redimensionando un drag activo.
#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

/// Estado de un arrastre de divisor en curso.
struct Drag {
    side: Side,
    /// Posición X del cursor al iniciar el arrastre.
    start_x: f32,
    /// Ancho del panel al iniciar el arrastre.
    start_w: f32,
}

/// Estado de presentación de la tarjeta de un comando.
#[derive(Default, Clone, Copy)]
struct RunUi {
    /// Acordeón cerrado — sólo se ve la cabecera.
    collapsed: bool,
    /// El filtro muestra stderr en vez de stdout.
    show_stderr: bool,
    /// El usuario tocó el acordeón a mano — ya no se autocolapsa.
    user_touched: bool,
    /// El usuario lanzó el comando con `&` — la tarjeta queda "pinned"
    /// aunque entren otros runs detrás. En este shell todo corre en
    /// background, así que el `&` es sobre todo una pista visual para
    /// no perder el run de vista en sesiones largas.
    background_explicit: bool,
}

/// Las cuatro señales que el shell expone como builtins (`:kill`,
/// `:term`, `:stop`, `:cont`). Mantengo el enum interno aquí para no
/// filtrar `nix::sys::signal` hasta el caller.
#[derive(Debug, Clone, Copy)]
enum Sig {
    Kill,
    Term,
    Stop,
    Cont,
}

impl Sig {
    fn label(self) -> &'static str {
        match self {
            Sig::Kill => "kill",
            Sig::Term => "term",
            Sig::Stop => "stop",
            Sig::Cont => "cont",
        }
    }
}

/// Asa unificada sobre dos backends de ejecución: local (`shuma-exec`)
/// y remoto (`shuma-remote-exec` por Unix socket al daemon). El resto
/// del shell sólo ve esta variante — try_events / kill / is_finished
/// tienen la misma forma para los dos.
enum AnyRunHandle {
    Local(RunHandle),
    Remote(RemoteRunHandle),
    /// Eventos pre-poblados — sólo se usa para reportar fallos upfront
    /// del backend remoto (p. ej. el daemon no responde) sin caer el
    /// shell ni hacer fallback silencioso al backend local.
    Synthetic { pending: Vec<RunEvent>, finished: bool },
}

impl AnyRunHandle {
    fn try_events(&mut self) -> Vec<RunEvent> {
        match self {
            Self::Local(h) => h.try_events(),
            Self::Remote(h) => h.try_events(),
            Self::Synthetic { pending, finished } => {
                *finished = true;
                std::mem::take(pending)
            }
        }
    }
    fn kill(&self) {
        match self {
            Self::Local(h) => h.kill(),
            Self::Remote(h) => h.kill(),
            Self::Synthetic { .. } => {}
        }
    }
    fn is_finished(&self) -> bool {
        match self {
            Self::Local(h) => h.is_finished(),
            Self::Remote(h) => h.is_finished(),
            Self::Synthetic { finished, .. } => *finished,
        }
    }
    /// SIGTERM. `false` si el backend no lo soporta (remote: el daemon
    /// sólo expone close-as-kill por ahora; sintético: no aplica).
    fn term(&self) -> bool {
        match self {
            Self::Local(h) => h.killer().term(),
            Self::Remote(_) | Self::Synthetic { .. } => false,
        }
    }
    /// SIGSTOP. Mismas limitaciones que `term`.
    fn stop(&self) -> bool {
        match self {
            Self::Local(h) => h.killer().stop(),
            Self::Remote(_) | Self::Synthetic { .. } => false,
        }
    }
    /// SIGCONT.
    fn cont(&self) -> bool {
        match self {
            Self::Local(h) => h.killer().cont(),
            Self::Remote(_) | Self::Synthetic { .. } => false,
        }
    }
    /// PIDs visibles del backend (vacío para Remote/Synthetic).
    fn pids(&self) -> Vec<u32> {
        match self {
            Self::Local(h) => h.killer().pids(),
            Self::Remote(_) | Self::Synthetic { .. } => Vec::new(),
        }
    }
    /// `true` si este backend soporta señales no-letales (stop/cont/term).
    /// El shell lo usa para informar cuando un usuario pide algo que la
    /// implementación remota no atiende aún.
    fn supports_signals(&self) -> bool {
        matches!(self, Self::Local(_))
    }
    /// Reenvía bytes al stdin del proceso (sólo aplica a runs PTY
    /// locales). Devuelve `false` si no hay listener — el caller puede
    /// reportarlo o silenciarlo según el contexto.
    fn write_input(&self, bytes: Vec<u8>) -> bool {
        match self {
            Self::Local(h) => h.write_input(bytes),
            Self::Remote(_) | Self::Synthetic { .. } => false,
        }
    }
}

/// Asa sintética que el `drain_exec` verá como un comando que falló
/// inmediatamente con `Failed(msg)` + `Exited(-1)`. Útil cuando el
/// backend remoto no se pudo iniciar.
fn synthetic_failed_handle(msg: String) -> AnyRunHandle {
    AnyRunHandle::Synthetic {
        pending: vec![RunEvent::Failed(msg), RunEvent::Exited(-1)],
        finished: false,
    }
}

/// Transporte hacia el daemon. `Local` ejecuta in-process via
/// `shuma-exec`. `UnixSocket` delega al daemon local (postcard sobre
/// Unix socket). `Tcp` delega a un daemon **remoto** con Noise XK
/// (autenticación mutua, cifrado ChaCha20-Poly1305).
enum RemoteTransport {
    Local,
    UnixSocket(std::path::PathBuf),
    Tcp {
        addr: String,
        keypair: shuma_link::Keypair,
        server_pub: shuma_link::PublicKey,
    },
}

impl RemoteTransport {
    /// Etiqueta corta para el prompt — el power user ve de un vistazo
    /// dónde está corriendo cada comando.
    fn badge(&self) -> Option<&'static str> {
        match self {
            Self::Local => None,
            Self::UnixSocket(_) => Some("⇄ unix"),
            Self::Tcp { .. } => Some("🔒 tcp"),
        }
    }
}

/// Decide el transporte hacia el daemon según las env vars del proceso:
///
/// - `SHUMA_REMOTE_TCP_ADDR=host:port` + `SHUMA_REMOTE_TCP_PUB=<hex>`
///   activan el modo TCP autenticado. La keypair propia se carga (o
///   genera al primer uso) de `SHUMA_REMOTE_TCP_KEY` o, por defecto,
///   `~/.config/shuma/keys/identity.x25519`. Si falla algo, cae a
///   Local con un warning.
/// - `SHUMA_REMOTE_SOCKET=/path` (path explícito) o `SHUMA_REMOTE=1`
///   (path canónico) activan el modo Unix socket — comportamiento de
///   antes.
/// - Sin ninguna env, `Local` (ejecución in-process).
fn detect_remote_transport() -> RemoteTransport {
    // TCP autenticado tiene prioridad — la combinación más expresiva
    // (host:port + pubkey del server) es la que el usuario fijó.
    if let (Ok(addr), Ok(pub_hex)) = (
        std::env::var("SHUMA_REMOTE_TCP_ADDR"),
        std::env::var("SHUMA_REMOTE_TCP_PUB"),
    ) {
        if !addr.is_empty() && !pub_hex.is_empty() {
            let key_path = std::env::var("SHUMA_REMOTE_TCP_KEY")
                .ok()
                .map(std::path::PathBuf::from)
                .or_else(shuma_link::Keypair::default_path);
            match (key_path, shuma_link::PublicKey::from_hex(&pub_hex)) {
                (Some(kp_path), Ok(server_pub)) => {
                    match shuma_link::Keypair::load_or_generate(&kp_path) {
                        Ok(kp) => return RemoteTransport::Tcp { addr, keypair: kp, server_pub },
                        Err(e) => {
                            eprintln!("shuma-shell: no se pudo cargar/generar la keypair {}: {e} — cayendo a local", kp_path.display());
                        }
                    }
                }
                (None, _) => {
                    eprintln!("shuma-shell: SHUMA_REMOTE_TCP_KEY sin path y XDG no disponible — cayendo a local");
                }
                (_, Err(e)) => {
                    eprintln!("shuma-shell: SHUMA_REMOTE_TCP_PUB inválido ({e}) — cayendo a local");
                }
            }
        }
    }
    if let Ok(p) = std::env::var("SHUMA_REMOTE_SOCKET") {
        if !p.is_empty() {
            return RemoteTransport::UnixSocket(std::path::PathBuf::from(p));
        }
    }
    if std::env::var("SHUMA_REMOTE").as_deref() == Ok("1") {
        return RemoteTransport::UnixSocket(shuma_protocol::default_socket_path());
    }
    RemoteTransport::Local
}

/// Picker fuzzy del historial durable — el overlay que aparece con Ctrl-R.
struct HistoryPicker {
    /// Consulta vigente; query vacía muestra las entradas más recientes.
    query: String,
    /// Índice de la línea resaltada dentro de los resultados visibles.
    selected: usize,
}

impl HistoryPicker {
    /// Cuántas filas se muestran a la vez.
    const VISIBLE: usize = 10;

    fn new() -> Self {
        Self { query: String::new(), selected: 0 }
    }

    /// Resultados ordenados por relevancia, limitados a `VISIBLE`.
    fn results<'a>(&self, history: &'a History) -> Vec<&'a HistoryEntry> {
        history.fuzzy_search(&self.query, Self::VISIBLE)
    }
}

/// Tabla de binarios que necesitan PTY para funcionar (fullscreen TUIs
/// o herramientas que detectan `isatty()` y rehúsan correr con pipes).
/// Coincide por el nombre exacto del comando (sin path).
fn is_tui_command(cmd: &str) -> bool {
    matches!(
        cmd,
        // Editores
        "vim" | "vi" | "nvim" | "neovim" | "nano" | "emacs" | "hx" | "helix" | "kak"
        // Paginadores
        | "less" | "more"
        // Monitores
        | "top" | "htop" | "btop" | "btm" | "atop" | "iotop" | "iftop"
        // File managers
        | "ranger" | "nnn" | "lf" | "mc" | "ncdu"
        // Network / cluster TUIs
        | "k9s" | "lazydocker" | "lazygit" | "gitui" | "tig" | "tui"
        // Otros TUIs comunes
        | "tmux" | "screen" | "byobu" | "claude" | "gptcli"
    )
}

/// Traduce un keystroke GPUI a los bytes que un PTY xterm-compatible
/// espera de su stdin. Cubre los casos comunes: caracteres imprimibles,
/// `Enter` → `\r`, flechas, Ctrl-letras, Esc, Tab, Backspace. Devuelve
/// `None` para teclas que no traducimos todavía.
fn key_to_pty_bytes(ks: &gpui::Keystroke) -> Option<Vec<u8>> {
    let ctrl = ks.modifiers.control;
    let alt = ks.modifiers.alt;
    let key = ks.key.as_str();
    // Ctrl+letra → byte de control (A=0x01, B=0x02, ...).
    if ctrl && key.len() == 1 {
        if let Some(c) = key.chars().next() {
            if c.is_ascii_alphabetic() {
                let upper = c.to_ascii_uppercase() as u8;
                return Some(vec![upper - b'A' + 1]);
            }
        }
    }
    // Alt+letra → ESC + letra.
    if alt && !ctrl {
        if let Some(s) = ks.key_char.as_deref() {
            let mut out = vec![0x1b];
            out.extend_from_slice(s.as_bytes());
            return Some(out);
        }
    }
    match key {
        "enter" => Some(b"\r".to_vec()),
        "tab" => Some(b"\t".to_vec()),
        "escape" => Some(b"\x1b".to_vec()),
        "backspace" => Some(b"\x7f".to_vec()),
        "delete" => Some(b"\x1b[3~".to_vec()),
        "up" => Some(b"\x1b[A".to_vec()),
        "down" => Some(b"\x1b[B".to_vec()),
        "right" => Some(b"\x1b[C".to_vec()),
        "left" => Some(b"\x1b[D".to_vec()),
        "home" => Some(b"\x1b[H".to_vec()),
        "end" => Some(b"\x1b[F".to_vec()),
        "pageup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" => Some(b"\x1b[6~".to_vec()),
        _ => {
            // Carácter imprimible — usamos lo que GPUI ya nos da en
            // `key_char` (respeta shift, layouts, etc.).
            ks.key_char.as_deref().map(|s| s.as_bytes().to_vec())
        }
    }
}

/// Detecta el `&` final aislado (no `&&`) y devuelve la línea sin él
/// junto a `true` si lo había. `cmd && otra` deja la línea intacta —
/// el `&&` queda como operador para el ejecutor.
fn strip_trailing_background(line: &str) -> (String, bool) {
    let trimmed = line.trim_end();
    if !trimmed.ends_with('&') {
        return (line.to_string(), false);
    }
    let bytes = trimmed.as_bytes();
    // El `&` final es el último char; comprobamos el penúltimo.
    if bytes.len() >= 2 && bytes[bytes.len() - 2] == b'&' {
        // Es `&&` (o `&&&` etc.) — operador, no background.
        return (line.to_string(), false);
    }
    let without = &trimmed[..trimmed.len() - 1];
    (without.trim_end().to_string(), true)
}

/// Limpia un pegado del portapapeles para insertarlo en la línea actual
/// (que es de un solo renglón). Equivalente a "bracketed paste" en un
/// terminal: nada se ejecuta automáticamente, las nuevas líneas se
/// coalescen a `; ` y los controles se descartan.
fn sanitize_paste(s: &str) -> String {
    // Trabajamos por líneas no vacías y unimos con "; ". Así las
    // múltiples \n consecutivas y los \r\n caen al mismo separador,
    // sin generar `; ;` ni colas vacías.
    let normalized = s.replace("\r\n", "\n").replace('\r', "\n");
    let pieces: Vec<String> = normalized
        .split('\n')
        .map(|seg| {
            seg.chars()
                .map(|c| match c {
                    '\t' => ' ',
                    c if c.is_control() => '\0',
                    c => c,
                })
                .filter(|c| *c != '\0')
                .collect::<String>()
        })
        .filter(|seg| !seg.is_empty())
        .collect();
    pieces.join("; ")
}

struct Shell {
    line: LineState,
    /// La sesión de trabajo: cwd, historial y grupos.
    session: WorkSession,
    /// Comandos en curso, con su canal de salida. El backend puede ser
    /// local (`shuma-exec`) o remoto (al daemon vía `shuma-remote-exec`)
    /// — el shell sólo ve el asa unificada.
    active: Vec<(RunId, AnyRunHandle)>,
    /// Por qué transporte ejecutar cada comando: in-process (Local),
    /// daemon local (UnixSocket) o daemon remoto autenticado (Tcp).
    /// Lo decide [`detect_remote_transport`] al arrancar; el modo no
    /// cambia durante la vida del proceso (re-exportar env var requiere
    /// relanzar el shell).
    transport: RemoteTransport,
    completion: Option<shuma_line::Completion>,
    completion_index: usize,
    show_completion: bool,
    source: ShellCompletionSource,
    sampler: SystemSampler,
    snapshot: Snapshot,
    left_collapsed: bool,
    right_collapsed: bool,
    /// Anchos de los paneles laterales (los divisores los ajustan).
    left_width: f32,
    right_width: f32,
    /// Arrastre de divisor en curso, si lo hay.
    drag: Option<Drag>,
    /// Estado de presentación por comando (acordeón, filtro stderr).
    run_ui: HashMap<RunId, RunUi>,
    /// Largo del historial en el último `:save` — define qué comandos
    /// entran al próximo grupo guardado.
    group_anchor: usize,
    /// Patrones detectados por el motor de inferencia (cache).
    patterns: Vec<shuma_infer::EmergingPattern>,
    /// Si está activo, el próximo comando reprocesa la salida de este run.
    reprocess_source: Option<RunId>,
    /// Scroll del feed central — sigue al comando más reciente.
    scroll: ScrollHandle,
    focus: FocusHandle,
    focused_once: bool,
    /// `true` cuando el shell corre como **modo launcher**: una barra
    /// compacta acoplada al pie de carmen, en vez del panel completo.
    launcher: bool,
    /// Las ventanas abiertas del escritorio, según el socket de control
    /// de carmen — la barra de tareas del modo launcher.
    windows_bar: Vec<WindowLine>,
    /// `true` cuando el cajón de resultados del modo launcher está
    /// desplegado (la ventana crece hacia arriba sobre el escritorio).
    drawer_open: bool,
    /// Configuración personal cargada de `~/.config/shuma/shumarc.toml`.
    /// Si falta o no parsea se usa [`Config::default`].
    config: Config,
    /// Historial durable entre sesiones — persistido a `~/.local/share/shuma/history.jsonl`.
    /// `None` si el SO no expone un directorio de datos o si la apertura
    /// falló (el shell sigue funcionando, sólo sin durabilidad).
    history: Option<History>,
    /// Posición en el historial durable mientras se navega con flechas;
    /// `None` = el cursor está "debajo" de la última entrada (líneanueva).
    history_cursor: Option<usize>,
    /// La línea que se estaba editando cuando comenzó la navegación por
    /// historial; se restaura al salir del historial por abajo.
    history_draft: Option<String>,
    /// Picker fuzzy del historial (Ctrl-R) — `Some` mientras está abierto.
    picker: Option<HistoryPicker>,
    /// Estados de emulación vt100 por run-PTY. Cada `RunEvent::Bytes` se
    /// procesa contra el parser correspondiente, que mantiene un buffer
    /// de pantalla renderizable como grid.
    tui_states: HashMap<RunId, vt100::Parser>,
    /// Run PTY actualmente en "modo raw input": las teclas del usuario
    /// se traducen a bytes y se reenvían al stdin del proceso en vez
    /// de editar la `LineState`. Esc fuera del PTY o `:detach` cierran
    /// el modo (el proceso sigue corriendo, sólo perdemos foco).
    focused_pty: Option<RunId>,
}

impl Shell {
    fn new(cx: &mut Context<Self>) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "/".to_string());

        // Configuración personal: fallback silencioso a default si falta
        // o no parsea. Aplica las env vars del .shumarc al proceso del
        // shell antes de cualquier subproceso — los hijos las heredan.
        let config = Config::load_default().unwrap_or_default();
        config.apply_env();

        let mut session = WorkSession::new("sesión", &cwd);
        // Aplica la política de captura del rc al arrancar la sesión.
        session.set_capture_limit(config.capture.limit_mb * 1024 * 1024);
        session.set_spill(config.capture.spill);
        // Grupos de ejemplo — recetas reutilizables.
        session.save_group("estado git", vec!["git status --short".into()]);
        session.save_group("build", vec!["cargo build --release".into()]);

        let shell = Self {
            line: LineState::new(),
            session,
            active: Vec::new(),
            transport: detect_remote_transport(),
            completion: None,
            completion_index: 0,
            show_completion: false,
            source: ShellCompletionSource::scan(cwd),
            sampler: SystemSampler::new(HISTORY),
            snapshot: Snapshot {
                cpu_percent: 0.0,
                mem_percent: 0.0,
                mem_used_mb: 0,
                mem_total_mb: 0,
                valid: false,
            },
            left_collapsed: false,
            right_collapsed: false,
            left_width: 176.0,
            right_width: 188.0,
            drag: None,
            run_ui: HashMap::new(),
            group_anchor: 0,
            patterns: Vec::new(),
            reprocess_source: None,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            focused_once: false,
            launcher: false,
            windows_bar: Vec::new(),
            drawer_open: false,
            history: History::default_path().and_then(|p| {
                let mut h = History::open(p).ok()?;
                h.set_dedup(match config.history.dedup {
                    CfgDedup::None => HistDedup::None,
                    CfgDedup::IgnoreConsecutive => HistDedup::IgnoreConsecutive,
                    CfgDedup::EraseDups => HistDedup::EraseDups,
                });
                Some(h)
            }),
            history_cursor: None,
            history_draft: None,
            picker: None,
            tui_states: HashMap::new(),
            focused_pty: None,
            config,
        };
        shell.start_loop(cx);
        shell
    }

    /// Bucle de fondo: drena la salida de los comandos (~9/s) y refresca
    /// los monitores (~1/s).
    fn start_loop(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut tick: u32 = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(110)).await;
                tick += 1;
                let sysmon = tick % 10 == 0;
                // Cada ~1 s pregunta a carmen por sus ventanas. La llamada
                // bloquea un instante sobre un socket Unix local — aquí,
                // en el executor de fondo, no en el hilo de la UI.
                let windows = (tick % 9 == 0).then(poll_ctl_windows).flatten();
                let alive = this.update(cx, |shell, cx| {
                    let mut changed = shell.drain_exec();
                    if sysmon {
                        shell.snapshot = shell.sampler.sample();
                        changed = true;
                    }
                    if let Some(w) = windows {
                        if w != shell.windows_bar {
                            shell.windows_bar = w;
                            changed = true;
                        }
                    }
                    if changed {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Vuelca la salida disponible de los comandos en curso al historial.
    fn drain_exec(&mut self) -> bool {
        let now = unix_now();
        let mut changed = false;
        for (id, handle) in &mut self.active {
            for ev in handle.try_events() {
                changed = true;
                match ev {
                    RunEvent::Stdout(l) => {
                        self.session.append_output(*id, Stream::Stdout, l)
                    }
                    RunEvent::Stderr(l) => {
                        self.session.append_output(*id, Stream::Stderr, l)
                    }
                    RunEvent::Truncated => self.session.mark_truncated(*id),
                    RunEvent::Spilled(path) => {
                        self.session.mark_truncated(*id);
                        self.session.append_output(
                            *id,
                            Stream::Stdout,
                            format!("↡ salida excedente volcada a {path}"),
                        );
                    }
                    RunEvent::Exited(code) => self.session.finish_run(*id, code, now),
                    RunEvent::Failed(msg) => {
                        self.session.append_output(
                            *id,
                            Stream::Stderr,
                            format!("✗ no se pudo lanzar: {msg}"),
                        );
                        self.session.finish_run(*id, -1, now);
                    }
                    // Bytes crudos del PTY: van al emulador vt100 del
                    // run correspondiente, que mantiene un buffer de
                    // pantalla renderizable por el render_run.
                    RunEvent::Bytes(bytes) => {
                        let parser = self
                            .tui_states
                            .entry(*id)
                            .or_insert_with(|| vt100::Parser::new(24, 80, 0));
                        parser.process(&bytes);
                    }
                }
            }
        }
        let finished = self.active.iter().any(|(_, h)| h.is_finished());
        self.active.retain(|(_, h)| !h.is_finished());
        if finished {
            // Al cerrarse un comando, el motor de inferencia revisa si
            // emergió un patrón repetido y lo promueve a un grupo.
            self.infer_patterns();
        }
        if changed {
            self.scroll.scroll_to_bottom();
        }
        changed
    }

    /// Comandos del historial reducidos a registros de inferencia.
    fn infer_records(&self) -> Vec<shuma_infer::CommandRecord> {
        self.session
            .history()
            .iter()
            .map(|r| {
                shuma_infer::CommandRecord::parse(&r.line, &r.cwd, r.status == RunStatus::Ok)
            })
            .collect()
    }

    /// Corre el motor de inferencia, cachea los patrones y promueve el
    /// más fuerte a un grupo reutilizable (rehidratación).
    fn infer_patterns(&mut self) {
        let records = self.infer_records();
        self.patterns =
            shuma_infer::detect_patterns(&records, &shuma_infer::InferConfig::default());
        if let Some(top) = self.patterns.first() {
            let name = format!("✨ {}", top.suggested_name());
            if self.session.group(&name).is_none() {
                self.session.save_group(name, top.example.clone());
            }
        }
    }

    /// Condición de disparo de un patrón: los marcadores de proyecto
    /// comunes a todos los directorios donde corrió.
    fn pattern_trigger(&self, p: &shuma_infer::EmergingPattern) -> Vec<String> {
        let mut dirs = p.directories.iter();
        let Some(first) = dirs.next() else {
            return Vec::new();
        };
        let mut common = markers_in(first);
        for d in dirs {
            let here = markers_in(d);
            common.retain(|m| here.contains(m));
        }
        common
    }

    /// La secuencia que el motor predice como continuación, si la hay.
    fn predicted_sequence(&self) -> Option<String> {
        if self.patterns.is_empty() {
            return None;
        }
        let records = self.infer_records();
        let tail = &records[records.len().saturating_sub(6)..];
        let (pi, next) = shuma_infer::predict_next(tail, &self.patterns)?;
        if next.is_empty() {
            return None;
        }
        // Disparo por estructura: no anticipar un patrón en un directorio
        // que no comparte su forma (no sugerir `cargo` sin `Cargo.toml`).
        let trigger = self.pattern_trigger(&self.patterns[pi]);
        if !trigger.is_empty() {
            let here = markers_in(self.session.cwd());
            if !trigger.iter().all(|m| here.contains(m)) {
                return None;
            }
        }
        Some(next.join(" && "))
    }

    /// Calcula el sufijo fantasma del prompt: el resto de la línea que el
    /// shell predice. Sólo con el cursor al final.
    fn compute_ghost(&self) -> Option<String> {
        let line = self.line.text();
        if line.is_empty() || self.line.cursor() != line.len() {
            return None;
        }
        // Corpus por prioridad: secuencia predicha, luego historial
        // reciente.
        let mut corpus: Vec<String> = Vec::new();
        if let Some(seq) = self.predicted_sequence() {
            corpus.push(seq);
        }
        for r in self.session.history().iter().rev() {
            corpus.push(r.line.clone());
        }
        shuma_line::ghost_suggestion(line, &corpus)
    }

    /// Resuelve el destino de un `cd` contra el cwd de la sesión.
    fn resolve_cd(&self, arg: &str) -> Result<String, String> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let target = if arg.is_empty() || arg == "~" {
            home
        } else if let Some(rest) = arg.strip_prefix("~/") {
            format!("{home}/{rest}")
        } else if arg.starts_with('/') {
            arg.to_string()
        } else {
            format!("{}/{}", self.session.cwd(), arg)
        };
        match std::fs::canonicalize(&target) {
            Ok(p) if p.is_dir() => Ok(p.to_string_lossy().into_owned()),
            Ok(_) => Err(format!("cd: no es un directorio: {target}")),
            Err(e) => Err(format!("cd: {target}: {e}")),
        }
    }

    /// Ejecuta una línea: `cd` se maneja internamente (cambia el cwd y,
    /// con él, el aislamiento); el resto se lanza con `shuma-exec` y su
    /// salida se transmite al panel central.
    /// Guarda como grupo los comandos ejecutados desde el último `:save`.
    fn save_group(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let n = self.session.history().len().saturating_sub(self.group_anchor);
        if n > 0 {
            self.session.save_recent_as_group(name, n);
            self.group_anchor = self.session.history().len();
        }
    }

    fn run_command(&mut self, line: String) {
        let line = line.trim().to_string();
        if line.is_empty() {
            return;
        }
        let now = unix_now();

        // Meta-comandos del shell — configuran la sesión, no se ejecutan.
        // Se chequean ANTES de la expansión de aliases para que el rc no
        // pueda secuestrar `:save`, `:limit`, etc.
        if let Some(name) = line.strip_prefix(":save ") {
            self.save_group(name);
            return;
        }
        if let Some(arg) = line.strip_prefix(":limit ") {
            // `:limit <MB>` — tope de captura de la sesión; 0 = sin tope.
            if let Ok(mb) = arg.trim().parse::<usize>() {
                self.session.set_capture_limit(mb * 1024 * 1024);
            }
            return;
        }
        if let Some(arg) = line.strip_prefix(":spill ") {
            // `:spill on|off` — volcar a disco la salida excedente.
            self.session
                .set_spill(matches!(arg.trim(), "on" | "si" | "sí" | "1" | "true"));
            return;
        }
        if let Some(args) = line.strip_prefix(":matilda ") {
            // Herramienta matilda embebida — administración de servidores.
            self.matilda_command(args);
            return;
        }
        // `:tui <cmd> [args...]` — fuerza el modo PTY (Exec::Pty) para
        // un comando concreto. Útil cuando el auto-detect no acierta
        // (binarios poco usuales) o cuando el usuario quiere un PTY
        // para algo que normalmente correría plano.
        if let Some(rest) = line.strip_prefix(":tui ") {
            self.run_tui(rest.trim().to_string());
            return;
        }
        // `:detach` — cierra el modo raw-input sin matar al proceso PTY.
        // El proceso sigue corriendo en background; las teclas vuelven al
        // editor de línea. `:attach <N>` (futuro) lo recoloca en foco.
        if line == ":detach" {
            self.focused_pty = None;
            return;
        }
        // Auto-detección: si el primer token coincide con la lista de
        // TUIs conocidas, promovemos a PTY automáticamente.
        if let Some(first) = line.split_whitespace().next() {
            if is_tui_command(first) {
                self.run_tui(line);
                return;
            }
        }
        // --- Job control ---
        // `:jobs` lista los runs activos numerados [1..]. Convención
        // estable mientras no termine ninguno: si uno termina, los
        // índices se renumeran al ejecutar `:jobs` de nuevo. Es la
        // misma semántica que bash.
        if line == ":jobs" {
            self.builtin_jobs();
            return;
        }
        if let Some(arg) = line.strip_prefix(":kill ") {
            self.builtin_signal(arg.trim(), Sig::Kill);
            return;
        }
        if let Some(arg) = line.strip_prefix(":term ") {
            self.builtin_signal(arg.trim(), Sig::Term);
            return;
        }
        if let Some(arg) = line.strip_prefix(":stop ") {
            self.builtin_signal(arg.trim(), Sig::Stop);
            return;
        }
        if let Some(arg) = line.strip_prefix(":cont ") {
            self.builtin_signal(arg.trim(), Sig::Cont);
            return;
        }

        // El `&` final marca un run como "background explícito": la
        // tarjeta se mantiene visible aunque entre otra detrás (los
        // demás runs sin user_touched igual se autocolapsan). En este
        // shell todo es "background" en realidad, así que el `&` es
        // sobre todo una pista de UX para destacar el run.
        // OJO: `cmd && otra` también termina en `&` cuando se mira por
        // sufijo; sólo lo aceptamos si la `&` no viene precedida de otra
        // (es decir, no es la cola de un `&&` operador).
        let (line, background_explicit) = strip_trailing_background(&line);

        // Expansión de aliases: la primera palabra se reemplaza si está
        // declarada en el `.shumarc`. Tras esta línea, `line` es lo que
        // realmente se ejecuta y se persiste — coherente con `history` en
        // bash/zsh, donde el alias resuelto es lo que queda registrado.
        let line = self.config.expand_aliases(&line).into_owned();

        // Los comandos anteriores que el usuario no fijó se autocolapsan
        // al aparecer uno nuevo abajo — orden de terminal tradicional.
        for ui in self.run_ui.values_mut() {
            if !ui.user_touched {
                ui.collapsed = true;
            }
        }

        // Persistir en el historial durable. La política de dedup descarta
        // duplicados consecutivos; cualquier error de I/O se ignora — el
        // historial durable es una mejora, no un requisito para ejecutar.
        if let Some(h) = self.history.as_mut() {
            let _ = h.append(HistoryEntry::new(&line, self.session.cwd(), now));
        }

        // `cd` interno — un subproceso no podría cambiar nuestro cwd.
        if line == "cd" || line.starts_with("cd ") {
            let arg = line.strip_prefix("cd").unwrap_or("").trim();
            let id = self.session.begin_run(&line, now);
            self.run_ui.insert(id, RunUi::default());
            match self.resolve_cd(arg) {
                Ok(new_cwd) => {
                    self.session.set_cwd(new_cwd.clone());
                    self.source.cwd = new_cwd.clone();
                    self.session
                        .append_output(id, Stream::Stdout, format!("→ {new_cwd}"));
                    self.session.finish_run(id, 0, now);
                }
                Err(e) => {
                    self.session.append_output(id, Stream::Stderr, e);
                    self.session.finish_run(id, 1, now);
                }
            }
            self.scroll.scroll_to_bottom();
            return;
        }

        let id = self.session.begin_run(&line, now);
        let mut ui = RunUi::default();
        if background_explicit {
            // El run "pinned" no se autocolapsa al entrar otro nuevo.
            ui.user_touched = true;
            ui.background_explicit = true;
        }
        self.run_ui.insert(id, ui);
        let spec = self.build_spec(&line, None, id);
        self.active.push((id, self.spawn(&spec)));
        self.scroll.scroll_to_bottom();
    }

    /// `:tui <cmd> [args...]` — lanza `cmd` bajo PTY y le da foco
    /// (modo raw-input). El usuario puede usar el comando como en un
    /// terminal real; `:detach` (o cerrar el run) vuelve al editor.
    fn run_tui(&mut self, line: String) {
        let line = line.trim().to_string();
        if line.is_empty() {
            self.synthetic_run(
                ":tui",
                vec!["uso: :tui <comando> [args...]".into()],
                false,
            );
            return;
        }
        // Parse muy simple: el primer token es el programa, el resto
        // son args. Sin shell expansion, sin pipes (un PTY siempre es
        // un único proceso). Las comillas se descartan literalmente
        // para no obligar al usuario a escaparlas, pero no procesamos
        // sustituciones.
        let mut parts = line.split_whitespace();
        let program = parts.next().expect("non-empty").to_string();
        let args: Vec<String> = parts.map(|s| unquote(s)).collect();
        let now = unix_now();
        for ui in self.run_ui.values_mut() {
            if !ui.user_touched {
                ui.collapsed = true;
            }
        }
        if let Some(h) = self.history.as_mut() {
            let _ = h.append(HistoryEntry::new(&line, self.session.cwd(), now));
        }
        let id = self.session.begin_run(&line, now);
        let mut ui = RunUi::default();
        ui.user_touched = true; // PTY runs no se autocolapsan
        ui.background_explicit = true;
        self.run_ui.insert(id, ui);
        let policy = self.session.capture();
        // PTY runs no usan capture_limit — la captura va a vt100, no a
        // ring buffer textual — pero respetamos spill_path por si en el
        // futuro alguien quiere grabar la sesión.
        let _ = policy;
        // Cols/rows iniciales conservadores; un futuro PTY-resize los
        // ajustará al tamaño real del panel.
        let spec = CommandSpec {
            exec: Exec::Pty { program, args, cols: 100, rows: 28 },
            cwd: self.session.cwd().to_string(),
            capture_limit: 0,
            spill_path: None,
            stdin_data: None,
        };
        // PTY no se puede delegar al daemon remoto todavía — siempre
        // local. Si SHUMA_REMOTE_TCP está activo, igual va por exec_run.
        self.active.push((id, AnyRunHandle::Local(exec_run(&spec))));
        self.tui_states.insert(id, vt100::Parser::new(28, 100, 200));
        self.focused_pty = Some(id);
        self.scroll.scroll_to_bottom();
    }

    /// `:jobs` — lista los runs activos numerados desde 1 con PID(s)
    /// y la línea original. Convención bash: la numeración es estable
    /// dentro de una invocación; si un job termina entre dos `:jobs`
    /// consecutivos, los demás se renumeran.
    fn builtin_jobs(&mut self) {
        let mut lines: Vec<String> = Vec::new();
        for (idx, (id, handle)) in self.active.iter().enumerate() {
            let pids = handle.pids();
            let pid_label = if pids.is_empty() {
                "—".to_string()
            } else {
                pids.iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let cmd = self
                .session
                .run(*id)
                .map(|r| r.line.as_str())
                .unwrap_or("<?>");
            let backend = match handle {
                AnyRunHandle::Local(_) => "local",
                AnyRunHandle::Remote(_) => "remote",
                AnyRunHandle::Synthetic { .. } => "synth",
            };
            lines.push(format!("[{}] {} {:<6}  {}", idx + 1, backend, pid_label, cmd));
        }
        if lines.is_empty() {
            lines.push("(sin jobs activos)".into());
        }
        self.synthetic_run(":jobs", lines, true);
    }

    /// `:kill N`, `:term N`, `:stop N`, `:cont N` — aplica la señal al
    /// job N de la lista actual. Si N no existe, lo dice; si el backend
    /// no soporta la señal, también. Las señales letales (`kill`,
    /// `term`) drenan el run del `active` a la siguiente iteración del
    /// reaper; `stop`/`cont` no cambian el active.
    fn builtin_signal(&mut self, arg: &str, sig: Sig) {
        let n: usize = match arg.parse() {
            Ok(n) if n >= 1 => n,
            _ => {
                self.synthetic_run(
                    &format!(":{}", sig.label()),
                    vec![format!("uso: :{} <N>  (ver :jobs)", sig.label())],
                    false,
                );
                return;
            }
        };
        let Some((_, handle)) = self.active.get(n - 1) else {
            self.synthetic_run(
                &format!(":{}", sig.label()),
                vec![format!("job {n} no existe (ver :jobs)")],
                false,
            );
            return;
        };
        let supports = handle.supports_signals();
        let delivered = match sig {
            Sig::Kill => {
                handle.kill();
                true
            }
            Sig::Term => handle.term(),
            Sig::Stop => handle.stop(),
            Sig::Cont => handle.cont(),
        };
        let msg = if !supports && !matches!(sig, Sig::Kill) {
            format!(
                "[{n}] {} no soportado para este backend (sólo kill funciona en remoto)",
                sig.label()
            )
        } else if delivered {
            format!("[{n}] {} entregado", sig.label())
        } else {
            format!("[{n}] {} no llegó (proceso terminado?)", sig.label())
        };
        self.synthetic_run(&format!(":{}", sig.label()), vec![msg], delivered);
    }

    /// Arma la `CommandSpec` de una línea: decide directo vs shell y
    /// aplica la política de captura de la sesión.
    fn build_spec(&self, line: &str, stdin: Option<String>, run_id: RunId) -> CommandSpec {
        self.build_spec_exec(plan_exec(line), stdin, run_id)
    }

    /// Lanza `spec` por el transporte activo (Local / UnixSocket /
    /// Tcp). Si el transporte remoto no se puede iniciar (daemon caído,
    /// pubkey errónea, etc.) se reporta como `RunEvent::Failed`
    /// sintético — el comando se ve como "✗ no se pudo lanzar" pero el
    /// shell sigue vivo.
    fn spawn(&self, spec: &CommandSpec) -> AnyRunHandle {
        match &self.transport {
            RemoteTransport::Local => AnyRunHandle::Local(exec_run(spec)),
            RemoteTransport::UnixSocket(sock) => match remote_exec_run(spec, sock) {
                Ok(h) => AnyRunHandle::Remote(h),
                Err(e) => synthetic_failed_handle(format!(
                    "daemon en {}: {e}",
                    sock.display()
                )),
            },
            RemoteTransport::Tcp { addr, keypair, server_pub } => {
                match shuma_remote_exec::run_tcp(spec, addr, keypair.clone(), *server_pub) {
                    Ok(h) => AnyRunHandle::Remote(h),
                    Err(e) => synthetic_failed_handle(format!("daemon TCP {addr}: {e}")),
                }
            }
        }
    }

    /// `build_spec` con el modo de ejecución ya decidido (lo usa la
    /// herramienta matilda, que ejecuta un script de shell completo).
    fn build_spec_exec(&self, exec: Exec, stdin: Option<String>, run_id: RunId) -> CommandSpec {
        let policy = self.session.capture();
        let spill_path = (policy.spill && policy.limit_bytes > 0).then(|| {
            std::env::temp_dir()
                .join(format!("shuma-spill-{}-{run_id}.log", std::process::id()))
        });
        CommandSpec {
            exec,
            cwd: self.session.cwd().to_string(),
            capture_limit: policy.limit_bytes,
            spill_path,
            stdin_data: stdin,
        }
    }

    /// Registra un comando "sintético" ya terminado — su salida la
    /// produce el shell mismo, no un proceso (la usa `:matilda plan`).
    fn synthetic_run(&mut self, label: &str, output: Vec<String>, ok: bool) {
        for ui in self.run_ui.values_mut() {
            if !ui.user_touched {
                ui.collapsed = true;
            }
        }
        let now = unix_now();
        let id = self.session.begin_run(label, now);
        self.run_ui.insert(id, RunUi::default());
        for line in output {
            self.session.append_output(id, Stream::Stdout, line);
        }
        self.session.finish_run(id, if ok { 0 } else { 1 }, now);
        self.scroll.scroll_to_bottom();
    }

    /// Ejecuta `exec_line` mostrando `label` en la tarjeta — el comando
    /// real puede diferir de lo que se ve (matilda corre un script).
    fn spawn_labeled(&mut self, label: String, exec_line: String) {
        for ui in self.run_ui.values_mut() {
            if !ui.user_touched {
                ui.collapsed = true;
            }
        }
        let now = unix_now();
        let id = self.session.begin_run(&label, now);
        self.run_ui.insert(id, RunUi::default());
        let exec = Exec::Shell { line: exec_line, program: "bash".into() };
        let spec = self.build_spec_exec(exec, None, id);
        self.active.push((id, self.spawn(&spec)));
        self.scroll.scroll_to_bottom();
    }

    /// Carga un inventario JSON, resolviendo la ruta contra el cwd.
    fn load_inventory(&self, file: &str) -> Result<matilda_core::Inventory, String> {
        let path = if file.starts_with('/') {
            file.to_string()
        } else {
            format!("{}/{}", self.session.cwd(), file)
        };
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("no se pudo leer {path}: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("JSON inválido: {e}"))
    }

    /// La herramienta matilda, embebida: `:matilda plan|script|apply
    /// <inventario.json>`. Reconcilia contra el estado real de la
    /// máquina y vuelca el resultado al feed del shell.
    fn matilda_command(&mut self, args: &str) {
        let label = format!(":matilda {args}");
        let parts: Vec<&str> = args.split_whitespace().collect();
        let (sub, file) = match parts.as_slice() {
            [s, f] => (*s, *f),
            _ => {
                self.synthetic_run(
                    &label,
                    vec!["uso: :matilda plan|script|apply <inventario.json>".into()],
                    false,
                );
                return;
            }
        };
        let desired = match self.load_inventory(file) {
            Ok(d) => d,
            Err(e) => {
                self.synthetic_run(&label, vec![e], false);
                return;
            }
        };
        // Reconcilia contra el estado real de esta máquina — con
        // detección de drift vía `docker inspect`.
        let current = matilda_discover::discover_inventory(&desired);
        let p = matilda_plan::plan(&current, &desired);

        match sub {
            "plan" => {
                let lines: Vec<String> = if p.is_empty() {
                    vec!["sin cambios: el servidor ya está al día".into()]
                } else {
                    p.actions
                        .iter()
                        .enumerate()
                        .map(|(i, a)| format!("{:>2}. {}", i + 1, a.describe()))
                        .collect()
                };
                self.synthetic_run(&label, lines, true);
            }
            "script" => {
                let script = matilda_apply::steps_to_script(&matilda_apply::plan_to_steps(
                    &p, &desired,
                ));
                self.synthetic_run(&label, script.lines().map(String::from).collect(), true);
            }
            "apply" => {
                let steps = matilda_apply::plan_to_steps(&p, &desired);
                if steps.is_empty() {
                    self.synthetic_run(&label, vec!["sin cambios: nada que aplicar".into()], true);
                } else {
                    // El script se ejecuta de verdad — fluye al feed.
                    self.spawn_labeled(label, matilda_apply::steps_to_script(&steps));
                }
            }
            other => {
                self.synthetic_run(&label, vec![format!("subcomando desconocido: {other}")], false)
            }
        }
    }

    /// Reprocesa la salida capturada del comando `source`: ejecuta `line`
    /// alimentándole esa salida por stdin, sin volver a correr el
    /// original. Así un resultado se filtra con distintas herramientas.
    fn run_reprocess(&mut self, line: String, source: RunId) {
        let line = line.trim().to_string();
        if line.is_empty() {
            return;
        }
        let data: String = self
            .session
            .run(source)
            .map(|r| r.lines_of(Stream::Stdout).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        for ui in self.run_ui.values_mut() {
            if !ui.user_touched {
                ui.collapsed = true;
            }
        }
        let now = unix_now();
        let id = self.session.begin_run(&line, now);
        self.run_ui.insert(id, RunUi::default());
        let spec = self.build_spec(&line, Some(data), id);
        self.active.push((id, self.spawn(&spec)));
        self.scroll.scroll_to_bottom();
    }

    /// Mata el proceso de un comando en curso.
    fn kill_run(&self, id: RunId) {
        if let Some((_, handle)) = self.active.iter().find(|(rid, _)| *rid == id) {
            handle.kill();
        }
    }

    fn refresh_completion(&mut self) {
        let comp = self.line.complete(&self.source);
        self.show_completion =
            !comp.candidates.is_empty() && comp.replace_end > comp.replace_start;
        self.completion_index = 0;
        self.completion = Some(comp);
    }

    fn on_tab(&mut self) {
        let comp = self.line.complete(&self.source);
        if comp.candidates.is_empty() {
            return;
        }
        if self.show_completion {
            let idx = self.completion_index.min(comp.candidates.len() - 1);
            let candidate = comp.candidates[idx].clone();
            self.line.apply_completion(&comp, &candidate);
            self.show_completion = false;
            self.completion = None;
        } else {
            self.completion_index = 0;
            self.completion = Some(comp);
            self.show_completion = true;
        }
    }

    fn cycle_completion(&mut self, delta: i32) {
        if !self.show_completion {
            return;
        }
        if let Some(comp) = &self.completion {
            let n = comp.candidates.len();
            if n > 0 {
                let i = self.completion_index as i32 + delta;
                self.completion_index = i.rem_euclid(n as i32) as usize;
            }
        }
    }

    /// Enter — ejecuta el contenido del input, o reprocesa una salida
    /// previa si hay un origen de reproceso activo.
    fn submit(&mut self) {
        let line = self.line.text().to_string();
        self.line.clear();
        self.completion = None;
        self.show_completion = false;
        if let Some(source) = self.reprocess_source.take() {
            self.run_reprocess(line, source);
        } else {
            self.run_command(line);
        }
    }

    fn handle_key(&mut self, event: &KeyDownEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let key = ks.key.as_str();
        let ctrl = ks.modifiers.control;
        let mut changed = false;

        // Picker fuzzy del historial — atajos propios mientras está abierto.
        if self.picker.is_some() {
            self.handle_picker_key(ks, cx);
            return;
        }

        // Modo raw-input para PTY enfocado. Todas las teclas se reenvían
        // al stdin del proceso. Sólo dos "escape hatches":
        //   - Ctrl-\\: detach (el proceso sigue, perdemos foco)
        //   - Ctrl-q como override por si Ctrl-\\ no se traduce
        if let Some(pty_id) = self.focused_pty {
            if ctrl && (key == "\\" || key == "q") {
                self.focused_pty = None;
                cx.notify();
                return;
            }
            // Si el run ya terminó, salimos automáticamente del modo.
            let alive = self.active.iter().any(|(rid, h)| *rid == pty_id && !h.is_finished());
            if !alive {
                self.focused_pty = None;
            } else if let Some(bytes) = key_to_pty_bytes(ks) {
                if let Some((_, handle)) = self.active.iter().find(|(rid, _)| *rid == pty_id) {
                    handle.write_input(bytes);
                    cx.notify();
                    return;
                }
            }
        }

        match key {
            "enter" => {
                // Multi-línea: si el input tiene una construcción abierta
                // (comilla, paréntesis, heredoc, `\` al final, operador
                // pendiente), Enter inserta `\n` en lugar de ejecutar.
                // Shift+Enter / Alt+Enter fuerzan newline aunque el input
                // esté completo (escape para construir multi-línea
                // explícita). Ctrl+Enter fuerza submit aunque parezca
                // abierto (escape para forzar la ejecución).
                let force_newline = ks.modifiers.shift || ks.modifiers.alt;
                let force_submit = ctrl;
                if !force_submit
                    && (force_newline || shuma_line::needs_continuation(self.line.text()))
                {
                    self.line.insert("\n");
                    self.history_cursor = None;
                    self.history_draft = None;
                    self.refresh_completion();
                } else {
                    self.submit();
                }
                cx.notify();
                return;
            }
            "escape" => {
                self.show_completion = false;
                self.reprocess_source = None;
                self.history_cursor = None;
                self.history_draft = None;
                cx.notify();
                return;
            }
            "tab" => {
                self.on_tab();
                cx.notify();
                return;
            }
            "up" => {
                // Prioridad: si el popup de autocompletado está abierto, las
                // flechas lo recorren; si no, navegan el historial durable.
                if self.show_completion {
                    self.cycle_completion(-1);
                } else {
                    self.history_step(HistoryNav::Older);
                }
                cx.notify();
                return;
            }
            "down" => {
                if self.show_completion {
                    self.cycle_completion(1);
                } else {
                    self.history_step(HistoryNav::Newer);
                }
                cx.notify();
                return;
            }
            "backspace" => {
                self.line.backspace();
                changed = true;
            }
            "delete" => {
                self.line.delete();
                changed = true;
            }
            "left" => {
                if ctrl {
                    self.line.move_word_left();
                } else {
                    self.line.move_left();
                }
            }
            "right" => {
                if ctrl {
                    self.line.move_word_right();
                } else if self.line.cursor() == self.line.text().len() {
                    // En el extremo, la flecha derecha acepta el fantasma.
                    if let Some(g) = self.compute_ghost() {
                        self.line.insert(&g);
                        changed = true;
                    }
                } else {
                    self.line.move_right();
                }
            }
            "home" => self.line.move_home(),
            "end" => self.line.move_end(),
            "a" if ctrl => self.line.move_home(),
            "e" if ctrl => self.line.move_end(),
            "u" if ctrl => {
                self.line.clear();
                changed = true;
            }
            "r" if ctrl => {
                // Abre el picker fuzzy del historial — overlay tipo Ctrl-R de bash.
                self.open_history_picker();
                cx.notify();
                return;
            }
            "v" if ctrl => {
                // Pegado del portapapeles — equivalente a "bracketed paste".
                if let Some(item) = cx.read_from_clipboard() {
                    if let Some(text) = item.text() {
                        let sanitized = sanitize_paste(&text);
                        if !sanitized.is_empty() {
                            self.line.insert(&sanitized);
                            changed = true;
                        }
                    }
                }
            }
            "space" if ctrl => {
                // Ctrl+Space también acepta el fantasma predicho.
                if let Some(g) = self.compute_ghost() {
                    self.line.insert(&g);
                    changed = true;
                }
            }
            _ if fkey_index(key).is_some() => {
                // F1..F8 ejecutan el grupo de esa posición en [RUN].
                let idx = fkey_index(key).unwrap();
                let joined =
                    self.session.groups().get(idx).map(|g| g.lines.join(" && "));
                if let Some(j) = joined {
                    self.run_command(j);
                }
                cx.notify();
                return;
            }
            _ => {
                if !ctrl {
                    if let Some(ch) = ks.key_char.as_deref() {
                        if !ch.chars().any(|c| c.is_control()) {
                            self.line.insert(ch);
                            changed = true;
                        }
                    }
                }
            }
        }

        if changed {
            // Cualquier edición del texto invalida la navegación por historial.
            self.history_cursor = None;
            self.history_draft = None;
            self.refresh_completion();
        }
        cx.notify();
    }

    /// Mueve el cursor del historial durable y refresca la línea con la
    /// entrada apuntada. Si se sale por abajo, restaura el borrador
    /// original que el usuario estaba escribiendo.
    fn history_step(&mut self, dir: HistoryNav) {
        let Some(h) = self.history.as_ref() else { return };
        if h.is_empty() {
            return;
        }
        if matches!(dir, HistoryNav::Older) && self.history_cursor.is_none() {
            // Primer paso hacia atrás: recordamos lo que había escrito.
            self.history_draft = Some(self.line.text().to_string());
        }
        match h.navigate(self.history_cursor, dir) {
            Some((idx, entry)) => {
                self.history_cursor = Some(idx);
                let text = entry.line.clone();
                self.line.clear();
                self.line.insert(&text);
            }
            None => {
                // Sólo cuando intentamos avanzar más allá de la última entrada
                // (sentido Newer) restauramos el borrador. Ir más atrás del
                // primer comando es un no-op.
                if matches!(dir, HistoryNav::Newer) {
                    self.history_cursor = None;
                    self.line.clear();
                    if let Some(draft) = self.history_draft.take() {
                        self.line.insert(&draft);
                    }
                }
            }
        }
        // El historial no debe filtrar el popup de autocompletado.
        self.show_completion = false;
    }

    /// Abre el picker fuzzy del historial. Si el historial está vacío o
    /// no se pudo cargar, no hace nada.
    fn open_history_picker(&mut self) {
        if self.history.as_ref().is_some_and(|h| !h.is_empty()) {
            self.picker = Some(HistoryPicker::new());
            self.show_completion = false;
        }
    }

    /// Manejo de teclas mientras el picker fuzzy del historial está abierto.
    fn handle_picker_key(&mut self, ks: &gpui::Keystroke, cx: &mut Context<Self>) {
        let key = ks.key.as_str();
        let ctrl = ks.modifiers.control;
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        let Some(history) = self.history.as_ref() else {
            self.picker = None;
            cx.notify();
            return;
        };

        match key {
            "escape" => {
                self.picker = None;
            }
            "enter" => {
                let results = picker.results(history);
                if let Some(entry) = results.get(picker.selected).copied() {
                    let text = entry.line.clone();
                    self.line.clear();
                    self.line.insert(&text);
                }
                self.picker = None;
            }
            "up" => {
                if picker.selected > 0 {
                    picker.selected -= 1;
                }
            }
            "down" => {
                let n = picker.results(history).len();
                if n > 0 && picker.selected + 1 < n {
                    picker.selected += 1;
                }
            }
            "r" if ctrl => {
                // Ctrl-R repetido avanza al siguiente match — convención bash.
                let n = picker.results(history).len();
                if n > 0 && picker.selected + 1 < n {
                    picker.selected += 1;
                }
            }
            "backspace" => {
                picker.query.pop();
                picker.selected = 0;
            }
            _ => {
                if !ctrl {
                    if let Some(ch) = ks.key_char.as_deref() {
                        if !ch.chars().any(|c| c.is_control()) {
                            picker.query.push_str(ch);
                            picker.selected = 0;
                        }
                    }
                }
            }
        }
        cx.notify();
    }

    /// Construye las **filas** del input — una por cada línea (cuando hay
    /// `\n` interno tras Enter sobre input incompleto). Cada fila tiene
    /// los tokens coloreados, el caret en su sitio si cae en esa línea,
    /// y el sufijo fantasma (sólo en la última fila). Sin prefijo del
    /// prompt — lo pone quien la usa.
    fn input_lines(&self, theme: &Theme) -> Vec<Vec<gpui::Div>> {
        let accent = gpui::hsla(190.0 / 360.0, 0.70, 0.62, 1.0);
        let dim = theme.fg_muted;
        let cursor = self.line.cursor();
        let text = self.line.text();
        let tokens = self.line.tokens();
        let caret = || div().w(px(2.)).h(px(19.)).bg(accent);
        let mut rows: Vec<Vec<gpui::Div>> = vec![Vec::new()];

        if text.is_empty() {
            rows[0].push(caret());
            rows[0].push(
                div()
                    .text_color(dim)
                    .child("escribe un comando…  (Tab autocompleta · Enter ejecuta · Shift+Enter newline)"),
            );
            return rows;
        }

        let mut caret_done = false;
        for t in &tokens {
            let color = token_color(t.kind, theme);
            let token_text = &text[t.start..t.end];
            let pieces: Vec<&str> = token_text.split('\n').collect();
            let n_pieces = pieces.len();
            let mut local_byte = 0usize;
            for (idx, piece) in pieces.iter().enumerate() {
                let abs_start = t.start + local_byte;
                let abs_end = abs_start + piece.len();
                // El caret cae en este fragmento si el cursor está en
                // `[abs_start, abs_end]`. Lo emitimos partiendo el
                // string en el offset local exacto.
                if !caret_done && cursor >= abs_start && cursor <= abs_end {
                    let cur_local = cursor - abs_start;
                    let (left, right) = piece.split_at(cur_local);
                    if !left.is_empty() {
                        rows.last_mut().unwrap().push(
                            div().flex_none().text_color(color).child(left.to_string()),
                        );
                    }
                    rows.last_mut().unwrap().push(caret());
                    if !right.is_empty() {
                        rows.last_mut().unwrap().push(
                            div().flex_none().text_color(color).child(right.to_string()),
                        );
                    }
                    caret_done = true;
                } else if !piece.is_empty() {
                    rows.last_mut().unwrap().push(
                        div().flex_none().text_color(color).child(piece.to_string()),
                    );
                }
                // Tras cada pieza excepto la última, vino un `\n` —
                // empezamos una nueva fila. El `+1` salta el byte del \n.
                local_byte += piece.len() + 1;
                if idx + 1 < n_pieces {
                    rows.push(Vec::new());
                }
            }
        }
        if !caret_done {
            rows.last_mut().unwrap().push(caret());
        }
        // El sufijo fantasma (predicción) sólo aparece en la fila final,
        // donde está el cursor al final del input.
        if let Some(ghost) = self.compute_ghost() {
            rows.last_mut().unwrap().push(
                div()
                    .flex_none()
                    .text_color(theme.fg_disabled)
                    .child(SharedString::from(ghost)),
            );
        }
        rows
    }

    /// Construye la fila del input: los tokens coloreados, el caret en su
    /// sitio y el sufijo fantasma. Sin el prefijo del prompt — lo pone
    /// quien la usa. La comparten el panel completo y el modo launcher.
    fn input_row(&self, theme: &Theme) -> Vec<gpui::Div> {
        let accent = gpui::hsla(190.0 / 360.0, 0.70, 0.62, 1.0);
        let dim = theme.fg_muted;
        let mut row: Vec<gpui::Div> = Vec::new();
        let cursor = self.line.cursor();
        let tokens = self.line.tokens();
        let caret = || div().w(px(2.)).h(px(19.)).bg(accent);
        if tokens.is_empty() {
            row.push(caret());
            row.push(
                div()
                    .text_color(dim)
                    .child("escribe un comando…  (Tab autocompleta · Enter ejecuta)"),
            );
        } else {
            let mut caret_done = false;
            for t in &tokens {
                let color = token_color(t.kind, theme);
                if !caret_done && cursor >= t.start && cursor < t.end {
                    let local = cursor - t.start;
                    let (left_s, right_s) = t.text.split_at(local);
                    if !left_s.is_empty() {
                        row.push(div().flex_none().text_color(color).child(left_s.to_string()));
                    }
                    row.push(caret());
                    row.push(div().flex_none().text_color(color).child(right_s.to_string()));
                    caret_done = true;
                } else {
                    row.push(div().flex_none().text_color(color).child(t.text.clone()));
                }
            }
            if !caret_done {
                row.push(caret());
            }
        }
        if let Some(ghost) = self.compute_ghost() {
            row.push(
                div()
                    .flex_none()
                    .text_color(theme.fg_disabled)
                    .child(SharedString::from(ghost)),
            );
        }
        row
    }

    /// Último código de salida del historial vivo — alimenta el segmento
    /// `exit` del prompt.
    fn last_exit_code(&self) -> Option<i32> {
        self.session.history().iter().rev().find_map(|r| r.exit_code)
    }

    /// Construye la fila de prefijo del prompt a partir de
    /// `config.prompt.segments`. Tokens soportados: `cwd`, `git`, `exit`,
    /// `time`; cualquier otro valor se trata como literal de texto. El
    /// glifo `›` y un espacio se añaden siempre al final como separador.
    fn prompt_prefix_row(&self, theme: &Theme) -> Vec<gpui::Div> {
        let accent = gpui::hsla(190.0 / 360.0, 0.70, 0.62, 1.0);
        let red = gpui::hsla(0.0, 0.65, 0.62, 1.0);
        let dim = theme.fg_muted;
        let mut row: Vec<gpui::Div> = Vec::new();
        // Badge de transporte — sólo cuando el shell delega al daemon.
        // En modo Local no aparece nada, para no añadir ruido al prompt
        // de quien usa la shell tal cual.
        if let Some(badge) = self.transport.badge() {
            row.push(
                div()
                    .flex_none()
                    .text_color(accent)
                    .child(SharedString::from(badge.to_string())),
            );
        }
        for seg in &self.config.prompt.segments {
            match seg.as_str() {
                "cwd" => row.push(
                    div()
                        .flex_none()
                        .text_color(accent)
                        .child(SharedString::from(format!(
                            "📁 {}",
                            pretty_cwd(self.session.cwd())
                        ))),
                ),
                "git" => {
                    if let Some(branch) = git_branch(self.session.cwd()) {
                        row.push(
                            div()
                                .flex_none()
                                .text_color(dim)
                                .child(SharedString::from(format!(" {branch}"))),
                        );
                    }
                }
                "exit" => {
                    if let Some(code) = self.last_exit_code() {
                        if code != 0 {
                            row.push(
                                div()
                                    .flex_none()
                                    .text_color(red)
                                    .child(SharedString::from(format!("✗{code}"))),
                            );
                        }
                    }
                }
                "time" => {
                    let secs = unix_now() % 86_400;
                    let h = secs / 3600;
                    let m = (secs / 60) % 60;
                    let s = secs % 60;
                    row.push(
                        div()
                            .flex_none()
                            .text_color(dim)
                            .child(SharedString::from(format!("{h:02}:{m:02}:{s:02}"))),
                    );
                }
                literal => row.push(
                    div()
                        .flex_none()
                        .text_color(dim)
                        .child(SharedString::from(literal.to_string())),
                ),
            }
        }
        // Flecha — separador entre prefijo del prompt e input.
        row.push(div().flex_none().text_color(accent).child(" ›  "));
        row
    }

    /// El modo launcher: una barra compacta —glifo, input, barra de
    /// ventanas, estado del último comando— para la franja que carmen
    /// reserva al pie.
    fn render_launcher(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let theme = Theme::global(cx).clone();
        let panel = gpui::hsla(220.0 / 360.0, 0.16, 0.11, 1.0);
        let node_bg = gpui::hsla(220.0 / 360.0, 0.14, 0.16, 1.0);
        let accent = gpui::hsla(190.0 / 360.0, 0.70, 0.62, 1.0);
        let dim = theme.fg_muted;
        let text = theme.fg_text;

        // Barra de tareas: una cajita por ventana abierta, la enfocada
        // resaltada. Un clic se la pide a carmen por el socket de control.
        let chips: Vec<_> = self
            .windows_bar
            .iter()
            .map(|w| {
                let id = w.id;
                let raw = if !w.title.is_empty() { &w.title } else { &w.app_id };
                let label = if raw.chars().count() > 18 {
                    format!("{}…", raw.chars().take(18).collect::<String>())
                } else {
                    raw.clone()
                };
                let focused = w.focused;
                div()
                    .id(SharedString::from(format!("win-{id}")))
                    .flex_none()
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(focused, |d| {
                        d.bg(accent).text_color(gpui::hsla(0.0, 0.0, 0.12, 1.0))
                    })
                    .when(!focused, |d| d.bg(node_bg).text_color(dim))
                    .child(SharedString::from(label))
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        focus_window(id);
                        // Eco inmediato — el sondeo confirma en ~1 s.
                        for w in &mut shell.windows_bar {
                            w.focused = w.id == id;
                        }
                        cx.notify();
                    }))
            })
            .collect();
        let taskbar = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .flex_none()
            .overflow_hidden()
            .children(chips);

        // Estado a la derecha: nº en curso, o el último comando. Un clic
        // despliega o repliega el cajón de resultados.
        let (status_text, status_color) = if !self.active.is_empty() {
            (format!("▷ {} en curso", self.active.len()), accent)
        } else if let Some(last) = self.session.history().last() {
            let (glyph, color) = match last.status {
                RunStatus::Running => ("▷", accent),
                RunStatus::Ok => ("✓", gpui::hsla(140.0 / 360.0, 0.48, 0.55, 1.0)),
                RunStatus::Failed => ("✗", gpui::hsla(2.0 / 360.0, 0.68, 0.60, 1.0)),
            };
            let mut line = last.line.clone();
            if line.chars().count() > 30 {
                line = format!("{}…", line.chars().take(30).collect::<String>());
            }
            (format!("{glyph} {line}"), color)
        } else {
            ("sin comandos".to_string(), dim)
        };
        let caret = if self.drawer_open { "▾" } else { "▴" };
        let status = div()
            .id("drawer-toggle")
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .px(px(6.))
            .rounded(px(4.))
            .text_size(px(12.))
            .cursor_pointer()
            .hover(|s| s.bg(node_bg))
            .child(div().text_color(dim).child(caret))
            .child(div().text_color(status_color).child(SharedString::from(status_text)))
            .on_click(cx.listener(|shell, _, window, cx| {
                shell.drawer_open = !shell.drawer_open;
                // La ventana crece o se encoge; carmen la ancla al pie.
                let w = window.bounds().size.width;
                let h = if shell.drawer_open {
                    LAUNCHER_BAR_H + LAUNCHER_DRAWER_H
                } else {
                    LAUNCHER_BAR_H
                };
                window.resize(gpui::size(w, px(h)));
                cx.notify();
            }));

        // La barra propiamente dicha — glifo, input, ventanas, estado.
        let bar = div()
            .h(px(LAUNCHER_BAR_H))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .overflow_hidden()
            .child(div().flex_none().text_color(accent).child("⟫"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_1()
                    .overflow_hidden()
                    .children(self.input_row(&theme)),
            )
            .child(taskbar)
            .child(status);

        // El cajón de resultados — los últimos comandos y su salida.
        let drawer = self.drawer_open.then(|| {
            let hist = self.session.history();
            let start = hist.len().saturating_sub(8);
            let runs: Vec<_> = hist[start..]
                .iter()
                .map(|r| {
                    let ui = self.run_ui.get(&r.id).copied().unwrap_or_default();
                    let tui = self.tui_states.get(&r.id);
                    render_run(r, ui, &theme, node_bg, cx, tui)
                })
                .collect();
            let empty = runs.is_empty();
            div()
                .id("launcher-drawer")
                .flex_1()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(8.))
                .bg(theme.bg_app)
                .when(empty, |d| {
                    d.child(div().text_color(dim).child("sin comandos todavía"))
                })
                .children(runs)
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(panel)
            .text_color(text)
            .text_size(px(13.))
            .track_focus(&self.focus)
            .key_context("ShumaShell")
            .on_key_down(cx.listener(Self::handle_key))
            .children(drawer)
            .child(bar)
    }
}

/// Color de resaltado de cada clase de token.
fn token_color(kind: TokenKind, theme: &Theme) -> Hsla {
    match kind {
        TokenKind::Command => gpui::hsla(190.0 / 360.0, 0.65, 0.62, 1.0),
        TokenKind::Argument => theme.fg_text,
        TokenKind::Flag => gpui::hsla(38.0 / 360.0, 0.80, 0.62, 1.0),
        TokenKind::StringLit => gpui::hsla(95.0 / 360.0, 0.42, 0.60, 1.0),
        TokenKind::Variable => gpui::hsla(280.0 / 360.0, 0.55, 0.72, 1.0),
        TokenKind::Pipe => gpui::hsla(190.0 / 360.0, 0.90, 0.72, 1.0),
        TokenKind::Redirect => gpui::hsla(20.0 / 360.0, 0.78, 0.62, 1.0),
        TokenKind::Operator => gpui::hsla(0.0, 0.66, 0.66, 1.0),
        TokenKind::Comment => theme.fg_muted,
        TokenKind::Whitespace => theme.fg_text,
        TokenKind::Unknown => gpui::hsla(0.0, 0.70, 0.60, 1.0),
    }
}

/// Resalta un texto estático (la línea de un comando del historial).
fn highlight(text: &str, theme: &Theme) -> Vec<gpui::Div> {
    shuma_line::tokenize(text, shuma_line::Dialect::Bash)
        .into_iter()
        .map(|t| {
            div()
                .flex_none()
                .text_color(token_color(t.kind, theme))
                .child(t.text)
        })
        .collect()
}

/// Acorta el cwd: el `$HOME` se muestra como `~`.
fn pretty_cwd(cwd: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if cwd == home => "~".to_string(),
        Ok(home) if cwd.starts_with(&format!("{home}/")) => format!("~{}", &cwd[home.len()..]),
        _ => cwd.to_string(),
    }
}

/// Mapea un `vt100::Color` (el que devuelve la celda del emulador
/// vt100 del run PTY) a un `Hsla` del frontend. Es un puente más rico
/// que el SGR plano: cubre Default + 256-color indexed + 24-bit RGB.
fn vt100_color_to_hsla(c: vt100::Color, default: Hsla) -> Hsla {
    use vt100::Color::*;
    match c {
        Default => default,
        Idx(i) => {
            // 16 colores ANSI (0..15) — reutilizamos la paleta del shell.
            if i < 16 {
                let mapped = match i {
                    0 => shuma_line::AnsiColor::Black,
                    1 => shuma_line::AnsiColor::Red,
                    2 => shuma_line::AnsiColor::Green,
                    3 => shuma_line::AnsiColor::Yellow,
                    4 => shuma_line::AnsiColor::Blue,
                    5 => shuma_line::AnsiColor::Magenta,
                    6 => shuma_line::AnsiColor::Cyan,
                    7 => shuma_line::AnsiColor::White,
                    8 => shuma_line::AnsiColor::BrightBlack,
                    9 => shuma_line::AnsiColor::BrightRed,
                    10 => shuma_line::AnsiColor::BrightGreen,
                    11 => shuma_line::AnsiColor::BrightYellow,
                    12 => shuma_line::AnsiColor::BrightBlue,
                    13 => shuma_line::AnsiColor::BrightMagenta,
                    14 => shuma_line::AnsiColor::BrightCyan,
                    _ => shuma_line::AnsiColor::BrightWhite,
                };
                ansi_color_to_hsla(Some(mapped), false, default)
            } else {
                // Paleta 256 (16..231 = cubo 6x6x6, 232..255 = grises).
                // Aproximación: convertimos a RGB lineal y luego a HSL.
                let (r, g, b) = if i < 232 {
                    let c = i - 16;
                    let r = (c / 36) % 6;
                    let g = (c / 6) % 6;
                    let b = c % 6;
                    let step = |v: u8| if v == 0 { 0 } else { 55 + 40 * v as u32 };
                    (step(r), step(g), step(b))
                } else {
                    let v = 8 + 10 * (i - 232) as u32;
                    (v, v, v)
                };
                rgb_to_hsla(r as u8, g as u8, b as u8)
            }
        }
        Rgb(r, g, b) => rgb_to_hsla(r, g, b),
    }
}

/// Conversión RGB → HSL para los pocos lugares (vt100 24-bit, paleta
/// 256) donde el color viene en sRGB directo en vez de via la paleta
/// ANSI nombrada. Devuelve un `Hsla` opaco.
fn rgb_to_hsla(r: u8, g: u8, b: u8) -> Hsla {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let (h, s) = if d < f32::EPSILON {
        (0.0, 0.0)
    } else {
        let s = if l < 0.5 { d / (max + min) } else { d / (2.0 - max - min) };
        let h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h / 6.0, s)
    };
    gpui::hsla(h, s, l, 1.0)
}

/// Mapea un [`shuma_line::AnsiColor`] a un `Hsla` del frontend. La
/// paleta es la "VS Code Dark+" — buena legibilidad sobre fondos
/// oscuros (panel del shell). `None` con `bold=true` levanta apenas el
/// fallback para que el `bold` siga siendo distinguible.
fn ansi_color_to_hsla(
    color: Option<shuma_line::AnsiColor>,
    bold: bool,
    fallback: Hsla,
) -> Hsla {
    use shuma_line::AnsiColor::*;
    let c = match color {
        None => {
            if bold {
                // bold sin color = un blanco brillante.
                return gpui::hsla(0.0, 0.0, 0.96, 1.0);
            }
            return fallback;
        }
        Some(c) => c,
    };
    // Las parejas (hue, sat, light). Bright sube light a ~0.72; normal a
    // ~0.56. Black/White se manejan aparte.
    let (h, s, l) = match c {
        Black => (0.0, 0.0, 0.20),
        Red => (0.0, 0.65, 0.58),
        Green => (135.0, 0.50, 0.55),
        Yellow => (48.0, 0.70, 0.62),
        Blue => (210.0, 0.65, 0.62),
        Magenta => (300.0, 0.55, 0.66),
        Cyan => (185.0, 0.60, 0.60),
        White => (0.0, 0.0, 0.85),
        BrightBlack => (0.0, 0.0, 0.45),
        BrightRed => (0.0, 0.75, 0.68),
        BrightGreen => (135.0, 0.60, 0.65),
        BrightYellow => (48.0, 0.85, 0.72),
        BrightBlue => (210.0, 0.75, 0.72),
        BrightMagenta => (300.0, 0.65, 0.74),
        BrightCyan => (185.0, 0.75, 0.72),
        BrightWhite => (0.0, 0.0, 0.96),
    };
    gpui::hsla(h / 360.0, s, l, 1.0)
}

/// Renderiza la tarjeta de un comando ejecutado: cabecera-acordeón +
/// filtro stdout/stderr + cuerpo de salida.
fn render_run(
    r: &CommandRun,
    ui: RunUi,
    theme: &Theme,
    node_bg: Hsla,
    cx: &mut Context<Shell>,
    tui: Option<&vt100::Parser>,
) -> impl IntoElement {
    let id = r.id;
    let dim = theme.fg_muted;
    let (glyph, gcolor) = match r.status {
        RunStatus::Running => ("▷", gpui::hsla(45.0 / 360.0, 0.75, 0.60, 1.0)),
        RunStatus::Ok => ("✓", gpui::hsla(140.0 / 360.0, 0.48, 0.55, 1.0)),
        RunStatus::Failed => ("✗", gpui::hsla(2.0 / 360.0, 0.68, 0.60, 1.0)),
    };
    let stderr_color = gpui::hsla(8.0 / 360.0, 0.62, 0.66, 1.0);
    let accent = gpui::hsla(190.0 / 360.0, 0.60, 0.62, 1.0);

    // Nota a la derecha: salida no-cero, truncado, y conteo si colapsada.
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = r.exit_code {
        if c != 0 {
            parts.push(format!("salió {c}"));
        }
    }
    if r.truncated {
        parts.push("⚠ truncado".to_string());
    }
    if ui.collapsed {
        let n = r.count_of(Stream::Stdout);
        if n > 0 {
            parts.push(format!("{n} líneas"));
        }
    }
    let note = parts.join(" · ");

    // Cabecera-acordeón: un clic colapsa/expande.
    let caret = if ui.collapsed { "▸" } else { "▾" };
    let header_left = div()
        .id(SharedString::from(format!("hdr-{id}")))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .flex_1()
        .cursor_pointer()
        .child(div().flex_none().text_color(dim).child(caret))
        .child(div().flex_none().text_color(gcolor).child(glyph))
        .children(highlight(&r.line, theme))
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(dim)
                .child(SharedString::from(note)),
        )
        .on_click(cx.listener(move |shell, _, _, cx| {
            let e = shell.run_ui.entry(id).or_default();
            e.collapsed = !e.collapsed;
            e.user_touched = true;
            cx.notify();
        }));

    // Filtro de stderr — sólo aparece si el comando emitió errores.
    let stderr_chip = if r.has_stderr() {
        let n = r.count_of(Stream::Stderr);
        Some(
            div()
                .id(SharedString::from(format!("err-{id}")))
                .flex_none()
                .px(px(6.))
                .py(px(1.))
                .rounded(px(3.))
                .text_size(px(11.))
                .cursor_pointer()
                .when(ui.show_stderr, |d| {
                    d.bg(stderr_color).text_color(gpui::hsla(0.0, 0.0, 0.12, 1.0))
                })
                .when(!ui.show_stderr, |d| d.text_color(stderr_color))
                .child(SharedString::from(format!("⚠ {n}")))
                .on_click(cx.listener(move |shell, _, _, cx| {
                    let e = shell.run_ui.entry(id).or_default();
                    e.show_stderr = !e.show_stderr;
                    cx.notify();
                })),
        )
    } else {
        None
    };

    // Botón de matar — sólo mientras el comando sigue corriendo.
    let kill_chip = if r.status == RunStatus::Running {
        Some(
            div()
                .id(SharedString::from(format!("kill-{id}")))
                .flex_none()
                .px(px(6.))
                .py(px(1.))
                .rounded(px(3.))
                .text_size(px(11.))
                .text_color(gpui::hsla(2.0 / 360.0, 0.66, 0.64, 1.0))
                .cursor_pointer()
                .hover(|s| s.bg(gpui::hsla(2.0 / 360.0, 0.55, 0.28, 1.0)))
                .child("✕ matar")
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.kill_run(id);
                    cx.notify();
                })),
        )
    } else {
        None
    };

    // Reprocesar — sólo si el comando dejó algo en stdout que filtrar.
    let reprocess_chip = if r.count_of(Stream::Stdout) > 0 {
        Some(
            div()
                .id(SharedString::from(format!("repro-{id}")))
                .flex_none()
                .px(px(6.))
                .py(px(1.))
                .rounded(px(3.))
                .text_size(px(11.))
                .text_color(accent)
                .cursor_pointer()
                .hover(|s| s.text_color(gpui::hsla(0.0, 0.0, 0.95, 1.0)))
                .child("⤳ reprocesar")
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.reprocess_source = Some(id);
                    cx.notify();
                })),
        )
    } else {
        None
    };

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .child(header_left)
        .children(stderr_chip)
        .children(reprocess_chip)
        .children(kill_chip);

    // Cuerpo: sólo con el acordeón abierto. El filtro elige el flujo.
    let mut body: Vec<gpui::Div> = Vec::new();
    if !ui.collapsed {
        // Etapas del pipe: un clic re-ejecuta la línea hasta esa etapa,
        // como un comando nuevo — así se inspeccionan los intermedios.
        if r.line.contains('|') {
            let toks = shuma_line::tokenize(&r.line, shuma_line::Dialect::Bash);
            let pipe = shuma_line::split_pipeline(&toks);
            if pipe.stages.len() >= 2 {
                let chip_bg = gpui::hsla(220.0 / 360.0, 0.18, 0.24, 1.0);
                let accent = gpui::hsla(190.0 / 360.0, 0.62, 0.62, 1.0);
                let mut chips: Vec<gpui::AnyElement> = vec![div()
                    .flex_none()
                    .text_size(px(10.))
                    .text_color(dim)
                    .child("⇢ etapas")
                    .into_any_element()];
                for (i, st) in pipe.stages.iter().enumerate() {
                    let end = st.tokens.last().map(|t| t.end).unwrap_or(r.line.len());
                    let prefix = r.line[..end].trim().to_string();
                    let name =
                        st.command.clone().unwrap_or_else(|| format!("{}", i + 1));
                    chips.push(
                        div()
                            .id(SharedString::from(format!("stage-{id}-{i}")))
                            .flex_none()
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(3.))
                            .bg(chip_bg)
                            .text_size(px(11.))
                            .text_color(accent)
                            .cursor_pointer()
                            .hover(|s| s.text_color(gpui::hsla(0.0, 0.0, 0.95, 1.0)))
                            .child(SharedString::from(name))
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                shell.run_command(prefix.clone());
                                cx.notify();
                            }))
                            .into_any_element(),
                    );
                }
                body.push(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(4.))
                        .items_center()
                        .children(chips),
                );
            }
        }

        // Si el run trae estado vt100 (modo PTY), el cuerpo es un grid
        // de celdas en vez de líneas planas — el render dibuja la
        // pantalla del proceso TUI con su cursor y colores.
        if let Some(parser) = tui {
            let screen = parser.screen();
            let (rows, cols) = screen.size();
            let mut grid_rows: Vec<gpui::Div> = Vec::with_capacity(rows as usize);
            for row in 0..rows {
                let mut spans: Vec<gpui::Div> = Vec::new();
                let mut current_text = String::new();
                let mut current_fg: Option<vt100::Color> = None;
                let flush = |spans: &mut Vec<gpui::Div>, text: &mut String, fg: vt100::Color, theme: &Theme| {
                    if text.is_empty() {
                        return;
                    }
                    let color = vt100_color_to_hsla(fg, theme.fg_text);
                    spans.push(
                        div().flex_none().text_color(color).child(SharedString::from(std::mem::take(text))),
                    );
                };
                for col in 0..cols {
                    if let Some(cell) = screen.cell(row, col) {
                        let fg = cell.fgcolor();
                        if current_fg != Some(fg) {
                            if let Some(prev) = current_fg {
                                flush(&mut spans, &mut current_text, prev, theme);
                            }
                            current_fg = Some(fg);
                        }
                        let c = cell.contents();
                        if c.is_empty() {
                            current_text.push(' ');
                        } else {
                            current_text.push_str(c);
                        }
                    } else {
                        current_text.push(' ');
                    }
                }
                if let Some(fg) = current_fg {
                    flush(&mut spans, &mut current_text, fg, theme);
                } else if !current_text.is_empty() {
                    spans.push(
                        div()
                            .flex_none()
                            .text_color(theme.fg_text)
                            .child(SharedString::from(std::mem::take(&mut current_text))),
                    );
                }
                grid_rows.push(div().flex().flex_row().children(spans));
            }
            body.push(
                div()
                    .flex()
                    .flex_col()
                    .font_family("Iosevka")
                    .text_size(px(12.))
                    .children(grid_rows),
            );
            // En modo TUI no mostramos el filtro stdout/stderr; salimos.
            return div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .p(px(8.))
                .bg(node_bg)
                .border_l_2()
                .border_color(gcolor)
                .rounded(px(5.))
                .child(header)
                .children(body);
        }

        let stream = if ui.show_stderr { Stream::Stderr } else { Stream::Stdout };
        let lines: Vec<&str> = r.lines_of(stream).collect();
        let default_color = if ui.show_stderr { stderr_color } else { theme.fg_text };
        if lines.is_empty() {
            body.push(div().text_size(px(11.)).text_color(dim).child(
                if ui.show_stderr { "sin errores" } else { "sin salida" },
            ));
        } else {
            // Cada línea se renderiza como una fila de spans coloreados
            // según las secuencias ANSI (`\x1b[…m`) y los `\r` (overwrite)
            // que viene emitiendo el subproceso. Si no hay ANSI, sale un
            // único span con el color por defecto del stream.
            for l in &lines {
                let spans = shuma_line::parse_ansi_line(l);
                let mut row = div().flex().flex_row().flex_wrap().text_size(px(12.));
                if spans.is_empty() {
                    row = row.text_color(default_color);
                } else {
                    for sp in spans {
                        let mut cell = div().flex_none();
                        cell = cell.text_color(ansi_color_to_hsla(
                            sp.style.fg,
                            sp.style.bold,
                            default_color,
                        ));
                        if let Some(bg) = sp.style.bg {
                            cell = cell.bg(ansi_color_to_hsla(Some(bg), false, default_color));
                        }
                        if sp.style.underline {
                            cell = cell.text_decoration_1();
                        }
                        row = row.child(cell.child(SharedString::from(sp.text)));
                    }
                }
                body.push(row);
            }
        }
    }

    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .p(px(8.))
        .bg(node_bg)
        .border_l_2()
        .border_color(gcolor)
        .rounded(px(5.))
        .child(header)
        .children(body)
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            window.focus(&self.focus);
            self.focused_once = true;
        }
        // Modo launcher: una barra compacta, no el panel de 3 columnas.
        if self.launcher {
            return self.render_launcher(cx);
        }
        let theme = Theme::global(cx).clone();
        let bg = theme.bg_app;
        let panel = gpui::hsla(220.0 / 360.0, 0.16, 0.11, 1.0);
        let node_bg = gpui::hsla(220.0 / 360.0, 0.14, 0.16, 1.0);
        let accent = gpui::hsla(190.0 / 360.0, 0.70, 0.62, 1.0);
        let text = theme.fg_text;
        let dim = theme.fg_muted;

        let pipeline = self.line.pipeline();
        let piped = pipeline.stages.iter().filter(|s| s.command.is_some()).count();

        // --- Barra de estado: cwd + identificador de aislamiento ---
        let status = div()
            .h(px(32.))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(14.))
            .bg(panel)
            .text_color(text)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(10.))
                    .items_baseline()
                    .child("● shuma")
                    .child(div().text_color(accent).child(SharedString::from(format!(
                        "📁 {}",
                        pretty_cwd(self.session.cwd())
                    ))))
                    .child(div().text_color(dim).text_size(px(11.)).child(SharedString::from(
                        format!("aisl:{}", self.session.isolation_id()),
                    ))),
            )
            .child(
                div().text_color(dim).text_size(px(12.)).child(SharedString::from({
                    // Política de captura de la sesión: tope + volcado.
                    let pol = self.session.capture();
                    let cap = if pol.limit_bytes == 0 {
                        "cap ∞".to_string()
                    } else {
                        format!(
                            "cap {}M{}",
                            pol.limit_bytes / (1024 * 1024),
                            if pol.spill { "↡" } else { "" }
                        )
                    };
                    let running = if piped > 1 {
                        format!("⇄ {piped} etapas · {} en curso", self.active.len())
                    } else {
                        format!("{} en curso", self.active.len())
                    };
                    format!("{cap}  ·  {running}")
                })),
            );

        // --- Panel izquierdo: grupos reutilizables [RUN] ---
        let left = if self.left_collapsed {
            div()
                .id("expand-left")
                .w(px(26.))
                .flex()
                .flex_col()
                .items_center()
                .pt(px(8.))
                .bg(panel)
                .text_color(dim)
                .cursor_pointer()
                .hover(|s| s.bg(node_bg))
                .child("»")
                .on_click(cx.listener(|s, _, _, cx| {
                    s.left_collapsed = false;
                    cx.notify();
                }))
        } else {
            let groups: Vec<_> = self
                .session
                .groups()
                .iter()
                .enumerate()
                .map(|(idx, g)| {
                    let joined = g.lines.join(" && ");
                    let count = g.lines.len();
                    // Los 8 primeros grupos llevan atajo dinámico F1..F8.
                    let label = if idx < 8 {
                        format!("F{}  ▸ {}  ·{count}", idx + 1, g.name)
                    } else {
                        format!("▸ {}  ·{count}", g.name)
                    };
                    div()
                        .id(SharedString::from(format!("group-{}", g.name)))
                        .px(px(8.))
                        .py(px(6.))
                        .bg(node_bg)
                        .rounded(px(4.))
                        .text_color(text)
                        .text_size(px(13.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.bg_row_hover))
                        .child(SharedString::from(label))
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.run_command(joined.clone());
                            cx.notify();
                        }))
                })
                .collect();
            div()
                .id("run-panel")
                .w(px(self.left_width))
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(10.))
                .bg(panel)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(div().text_color(dim).text_size(px(12.)).child("[RUN] grupos"))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(4.))
                                .items_center()
                                .child(
                                    div()
                                        .id("save-group")
                                        .px(px(5.))
                                        .text_color(dim)
                                        .cursor_pointer()
                                        .hover(|s| s.text_color(accent))
                                        .child("＋")
                                        .on_click(cx.listener(|s, _, _, cx| {
                                            s.line.set_text(":save ");
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    div()
                                        .id("collapse-left")
                                        .px(px(5.))
                                        .text_color(dim)
                                        .cursor_pointer()
                                        .hover(|s| s.text_color(accent))
                                        .child("«")
                                        .on_click(cx.listener(|s, _, _, cx| {
                                            s.left_collapsed = true;
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
                .children(groups)
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(dim)
                        .child("clic ejecuta · ＋ guarda lo último"),
                )
                .child(div().h(px(1.)).bg(theme.border))
                .child(div().text_color(dim).text_size(px(12.)).child("[tools]"))
                .child(
                    div()
                        .id("tool-matilda")
                        .px(px(8.))
                        .py(px(6.))
                        .bg(node_bg)
                        .rounded(px(4.))
                        .text_color(text)
                        .text_size(px(13.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.bg_row_hover))
                        .child("⚙ matilda")
                        .on_click(cx.listener(|shell, _, _, cx| {
                            // Precarga el comando para que el usuario nombre el inventario.
                            shell.line.set_text(":matilda plan ");
                            cx.notify();
                        })),
                )
        };

        // --- Lienzo central: comandos ejecutados + su salida ---
        // Orden de terminal: los más viejos arriba, los nuevos abajo.
        let hist = self.session.history();
        let start = hist.len().saturating_sub(40);
        let runs: Vec<_> = hist[start..]
            .iter()
            .map(|r| {
                let ui = self.run_ui.get(&r.id).copied().unwrap_or_default();
                let tui = self.tui_states.get(&r.id);
                render_run(r, ui, &theme, node_bg, cx, tui)
            })
            .collect();
        let runs_empty = runs.is_empty();
        let canvas = div()
            .id("runs")
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(10.))
            .bg(bg)
            .when(runs_empty, |d| {
                d.child(div().text_color(dim).child(
                    "Escribe un comando abajo y presiona Enter — su salida aparece aquí.",
                ))
            })
            .children(runs);

        // --- Panel derecho: monitores [SENS] ---
        let right = if self.right_collapsed {
            div()
                .id("expand-right")
                .w(px(26.))
                .flex()
                .flex_col()
                .items_center()
                .pt(px(8.))
                .bg(panel)
                .text_color(dim)
                .cursor_pointer()
                .hover(|s| s.bg(node_bg))
                .child("«")
                .on_click(cx.listener(|s, _, _, cx| {
                    s.right_collapsed = false;
                    cx.notify();
                }))
        } else {
            let cpu = self.snapshot.cpu_percent;
            let cpu_curve = self.sampler.cpu_history().values();
            let mem_curve = self.sampler.mem_history().values();
            let cpu_color = gpui::hsla(190.0 / 360.0, 0.72, 0.62, 1.0);
            let mem_color = gpui::hsla(265.0 / 360.0, 0.55, 0.70, 1.0);

            let monitor = |title: &str, value: String, curve: Vec<f32>, color: Hsla| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(8.))
                    .bg(node_bg)
                    .rounded(px(5.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .items_baseline()
                            .child(div().text_color(dim).text_size(px(11.)).child(title.to_string()))
                            .child(div().text_color(color).child(SharedString::from(value))),
                    )
                    .child(div().h(px(44.)).child(CurveElement::new(curve, color)))
            };

            div()
                .id("sens-panel")
                .w(px(self.right_width))
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(10.))
                .bg(panel)
                .text_color(text)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(div().text_color(dim).text_size(px(12.)).child("[SENS]"))
                        .child(
                            div()
                                .id("collapse-right")
                                .px(px(5.))
                                .text_color(dim)
                                .cursor_pointer()
                                .hover(|s| s.text_color(accent))
                                .child("»")
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.right_collapsed = true;
                                    cx.notify();
                                })),
                        ),
                )
                .child(monitor("CPU", format!("{cpu:.0} %"), cpu_curve, cpu_color))
                .child(monitor(
                    "MEM",
                    if self.snapshot.valid {
                        format!(
                            "{:.1}/{:.0} GB",
                            self.snapshot.mem_used_mb as f32 / 1024.0,
                            self.snapshot.mem_total_mb as f32 / 1024.0
                        )
                    } else {
                        "— GB".to_string()
                    },
                    mem_curve,
                    mem_color,
                ))
        };

        // --- Zona prompt: el input inteligente (multi-línea) ---
        // El prefijo lo arma `prompt_prefix_row` desde la config
        // (`config.prompt.segments`); cada línea del input es una fila
        // horizontal y se apilan en vertical. Sólo la primera lleva el
        // prompt prefix; las continuaciones llevan un sangrado de `… `
        // para señalar que es la misma orden.
        let lines = self.input_lines(&theme);
        let n_lines = lines.len();
        let input_lines_stack: Vec<gpui::Div> = lines
            .into_iter()
            .enumerate()
            .map(|(i, row_children)| {
                let mut row = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(14.));
                if i == 0 {
                    for cell in self.prompt_prefix_row(&theme) {
                        row = row.child(cell);
                    }
                } else {
                    row = row.child(
                        div()
                            .flex_none()
                            .text_color(theme.fg_muted)
                            .child("…  "),
                    );
                }
                for cell in row_children {
                    row = row.child(cell);
                }
                row
            })
            .collect();
        let input_bar = div()
            .when(n_lines == 1, |d| d.h(px(46.)))
            .flex()
            .flex_col()
            .justify_center()
            .text_color(text)
            .text_size(px(14.))
            .py(px(if n_lines == 1 { 0.0 } else { 6.0 }))
            .children(input_lines_stack);
        // Banner del modo reproceso — escribí un filtro para la salida.
        let banner = self.reprocess_source.map(|src| {
            div()
                .px(px(14.))
                .py(px(3.))
                .bg(gpui::hsla(190.0 / 360.0, 0.30, 0.22, 1.0))
                .text_size(px(11.))
                .text_color(accent)
                .child(SharedString::from(format!(
                    "⤳ reprocesando la salida de #{src} — escribí un filtro · Esc cancela"
                )))
        });
        let prompt = div()
            .flex()
            .flex_col()
            .bg(panel)
            .children(banner)
            .child(input_bar);

        // --- Popup de autocompletado ---
        let mut popup_layer: Vec<gpui::Div> = Vec::new();

        // --- Overlay del picker fuzzy del historial (Ctrl-R) ---
        // Se dibuja primero para que, si está abierto, los demás popups no
        // compitan visualmente. Tiene su propia cabecera con la query.
        if let (Some(picker), Some(history)) = (self.picker.as_ref(), self.history.as_ref()) {
            let results = picker.results(history);
            let query_display = if picker.query.is_empty() {
                SharedString::from("(escribe para filtrar · ↑↓ navega · Enter elige · Esc cierra)")
            } else {
                SharedString::from(format!("› {}", picker.query))
            };
            let total = history.len();
            let header = div()
                .px(px(10.))
                .py(px(5.))
                .text_color(accent)
                .text_size(px(13.))
                .child(query_display);
            let stats = div()
                .px(px(10.))
                .py(px(3.))
                .text_color(dim)
                .text_size(px(11.))
                .child(SharedString::from(format!(
                    "{} / {} · Ctrl-R próximo",
                    results.len(),
                    total
                )));
            let rows: Vec<_> = results
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    let selected = i == picker.selected;
                    let line_color = if selected { node_bg } else { text };
                    let cwd_color = if selected { node_bg } else { dim };
                    div()
                        .px(px(10.))
                        .py(px(3.))
                        .flex()
                        .flex_row()
                        .gap(px(8.))
                        .when(selected, |d| d.bg(accent))
                        .child(
                            div()
                                .flex_none()
                                .text_color(line_color)
                                .child(SharedString::from(entry.line.clone())),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_color(cwd_color)
                                .text_size(px(11.))
                                .child(SharedString::from(entry.cwd.clone())),
                        )
                })
                .collect();
            popup_layer.push(
                div()
                    .absolute()
                    .left(px(28.))
                    .bottom(px(52.))
                    .w(px(640.))
                    .flex()
                    .flex_col()
                    .bg(node_bg)
                    .border_1()
                    .border_color(accent)
                    .rounded(px(5.))
                    .text_size(px(13.))
                    .child(header)
                    .child(stats)
                    .children(rows),
            );
        }

        if self.show_completion {
            if let Some(comp) = &self.completion {
                if !comp.candidates.is_empty() {
                    let kind_label = match comp.kind {
                        CompletionKind::Command => "comando",
                        CompletionKind::Flag => "flag",
                        CompletionKind::Path => "ruta",
                    };
                    let total = comp.candidates.len();
                    let start = self
                        .completion_index
                        .saturating_sub(3)
                        .min(total.saturating_sub(8));
                    let rows: Vec<_> = comp
                        .candidates
                        .iter()
                        .enumerate()
                        .skip(start)
                        .take(8)
                        .map(|(i, cand)| {
                            let selected = i == self.completion_index;
                            div()
                                .px(px(8.))
                                .py(px(3.))
                                .when(selected, |d| {
                                    d.bg(accent).text_color(gpui::hsla(0.0, 0.0, 0.1, 1.0))
                                })
                                .when(!selected, |d| d.text_color(text))
                                .child(SharedString::from(cand.clone()))
                        })
                        .collect();
                    popup_layer.push(
                        div()
                            .absolute()
                            .left(px(28.))
                            .bottom(px(52.))
                            .w(px(320.))
                            .flex()
                            .flex_col()
                            .bg(node_bg)
                            .border_1()
                            .border_color(accent)
                            .rounded(px(5.))
                            .text_size(px(13.))
                            .child(
                                div()
                                    .px(px(8.))
                                    .py(px(3.))
                                    .text_color(dim)
                                    .text_size(px(11.))
                                    .child(SharedString::from(format!(
                                        "{kind_label} · {total} · ↑↓ Tab"
                                    ))),
                            )
                            .children(rows),
                    );
                }
            }
        }

        // --- Divisores arrastrables ---
        let divider = |side: Side, cx: &mut Context<Self>| {
            div()
                .w(px(5.))
                .bg(node_bg)
                .cursor(CursorStyle::ResizeLeftRight)
                .hover(|s| s.bg(accent))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |shell, ev: &MouseDownEvent, _w, cx| {
                        let start_w = match side {
                            Side::Left => shell.left_width,
                            Side::Right => shell.right_width,
                        };
                        shell.drag = Some(Drag {
                            side,
                            start_x: ev.position.x.into(),
                            start_w,
                        });
                        cx.notify();
                    }),
                )
        };

        let mut middle = div().flex().flex_row().flex_1().overflow_hidden().child(left);
        if !self.left_collapsed {
            middle = middle.child(divider(Side::Left, cx));
        }
        middle = middle.child(canvas);
        if !self.right_collapsed {
            middle = middle.child(divider(Side::Right, cx));
        }
        middle = middle.child(right);

        // --- Composición ---
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(bg)
            .track_focus(&self.focus)
            .key_context("ShumaShell")
            .on_key_down(cx.listener(Self::handle_key))
            .on_mouse_move(cx.listener(|shell, ev: &MouseMoveEvent, _w, cx| {
                if let Some(drag) = &shell.drag {
                    let cur: f32 = ev.position.x.into();
                    let delta = cur - drag.start_x;
                    match drag.side {
                        // El panel izquierdo crece al arrastrar a la derecha.
                        Side::Left => {
                            shell.left_width = (drag.start_w + delta).clamp(130.0, 420.0)
                        }
                        // El derecho crece al arrastrar a la izquierda.
                        Side::Right => {
                            shell.right_width = (drag.start_w - delta).clamp(130.0, 420.0)
                        }
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|shell, _ev: &MouseUpEvent, _w, cx| {
                    if shell.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(status)
            .child(middle)
            .child(prompt)
            .children(popup_layer)
    }
}

impl Drop for Shell {
    /// Al cerrar la sesión, limpia sus archivos de volcado temporales.
    fn drop(&mut self) {
        let prefix = format!("shuma-spill-{}-", std::process::id());
        if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

/// Levanta el shell en **modo launcher**: una ventana sin barra de
/// título y con `app_id` `carmen.shell`, para que el compositor la
/// reconozca y la acople a la franja del pie.
fn run_launcher() {
    Application::new().run(|cx: &mut App| {
        Theme::install_default(cx);
        let bounds = Bounds::centered(None, gpui::size(px(1280.), px(40.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                app_id: Some("carmen.shell".into()),
                ..Default::default()
            },
            |_w, cx| {
                cx.new(|cx| {
                    let mut shell = Shell::new(cx);
                    shell.launcher = true;
                    shell
                })
            },
        )
        .expect("open window");
        cx.activate(true);
    });
}

fn main() {
    // Modo launcher: barra acoplada a carmen. Lo activan el argumento
    // `--launcher` o la variable de entorno `MIRADA_SHELL`.
    let launcher = std::env::args().any(|a| a == "--launcher")
        || std::env::var_os("MIRADA_SHELL").is_some();
    if launcher {
        run_launcher();
    } else {
        launch_app("brahman · shuma shell", (1100., 700.), Shell::new);
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_paste;

    #[test]
    fn paste_drops_trailing_newline() {
        // Caso típico: pegar "ls -la\n" no debe ejecutar nada por sí solo.
        assert_eq!(sanitize_paste("ls -la\n"), "ls -la");
    }

    #[test]
    fn paste_joins_multiline_with_semicolon() {
        // Varios comandos pegados se coalescen como secuencia, sin ejecutar.
        assert_eq!(sanitize_paste("ls\npwd\n"), "ls; pwd");
    }

    #[test]
    fn paste_normalizes_crlf() {
        assert_eq!(sanitize_paste("a\r\nb"), "a; b");
        assert_eq!(sanitize_paste("a\rb"), "a; b");
    }

    #[test]
    fn paste_strips_other_controls() {
        // ESC (\x1b) y BEL (\x07) se descartan; tab → espacio.
        assert_eq!(sanitize_paste("ls\t-la\x1b[X\x07"), "ls -la[X");
    }

    #[test]
    fn paste_collapses_repeated_separators() {
        // Dos saltos seguidos no producen "; ;".
        assert_eq!(sanitize_paste("a\n\nb"), "a; b");
    }

    #[test]
    fn paste_keeps_plain_text() {
        assert_eq!(sanitize_paste("echo hola mundo"), "echo hola mundo");
    }

    #[test]
    fn background_ampersand_is_stripped_and_flagged() {
        use super::strip_trailing_background;
        assert_eq!(strip_trailing_background("sleep 30 &"), ("sleep 30".into(), true));
        assert_eq!(strip_trailing_background("sleep 30&"), ("sleep 30".into(), true));
        assert_eq!(strip_trailing_background("sleep 30"), ("sleep 30".into(), false));
        // `cmd_a && cmd_b` no termina en `&`, queda intacto.
        assert_eq!(
            strip_trailing_background("cmd_a && cmd_b"),
            ("cmd_a && cmd_b".into(), false)
        );
        // `cmd &&` SÍ termina en `&`, pero la `&` previa lo identifica
        // como operador — no se trata como background.
        assert_eq!(strip_trailing_background("cmd &&"), ("cmd &&".into(), false));
    }
}
