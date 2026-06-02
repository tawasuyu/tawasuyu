//! `llimphi-module-shuma-term` — terminal integrado al estilo Ctrl+` de
//! VS Code o "Terminal" de JetBrains, pero enchufable en cualquier app
//! Llimphi.
//!
//! Lo monta sobre dos piezas que ya existen:
//!
//! - [`shuma_exec::Exec::Pty`] aloja un pseudo-terminal cross-platform
//!   (`portable-pty`), lanza el shell con `TERM=xterm-256color`, y
//!   entrega los bytes crudos por un canal MPSC. El módulo no toca
//!   syscalls — sólo consume eventos.
//! - [`vt100::Parser`] convierte esos bytes en un buffer de pantalla
//!   ANSI: cursor, erase, OSC, scrollback. El módulo le pasa los bytes
//!   y al renderizar pide `screen().contents()`.
//!
//! Sigue el contrato Llimphi de `docs/MODULES.md`: `State + Msg +
//! Action + apply/on_key/open_shortcut/view + Palette`.
//!
//! ## Cómo lo enchufa una app (resumen)
//!
//! ```ignore
//! struct Model { term: Option<ShumaTermState>, … }
//! enum Msg { Term(ShumaTermMsg), Tick, … }
//!
//! // open: shuma_term::spawn("/home/user", 100, 30)?
//! // on_key: si term.is_some() y on_key devuelve Some(msg) → Msg::Term(msg)
//! //         si term.is_none() y open_shortcut(ev) → Msg::Term(Open)
//! // tick periódico: dispatch Msg::Term(Tick) para drenar PTY
//! // apply: match action { Close → model.term = None, SetStatus(s) → … }
//! // view:  si term.is_some() → push view(...)
//! ```

#![forbid(unsafe_code)]

use std::time::Instant;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use shuma_exec::{CommandSpec, Exec, Killer, RunEvent, RunHandle};

/// Capabilities que aporta este módulo al host. El host las puede
/// agregar a `provides` en su `card_core::Card` para que el broker
/// chasqui descubra que la instancia ofrece terminal integrado.
pub const CAPABILITIES: &[&str] = &["editor.terminal"];

/// Dimensiones por defecto del PTY. Cubren un panel inferior tipo
/// VS Code en una pantalla 1080p. Las apps pueden pasar otras a
/// [`spawn_with`].
pub const DEFAULT_COLS: u16 = 100;
pub const DEFAULT_ROWS: u16 = 24;

const SCROLLBACK: usize = 2000;

// =====================================================================
// State
// =====================================================================

/// Estado del panel terminal. Encapsula el `RunHandle` del shell y un
/// `vt100::Parser` que mantiene el buffer de pantalla. No es `Clone`
/// (los handles son únicos), y el host lo embebe como
/// `Option<ShumaTermState>`.
pub struct ShumaTermState {
    handle: RunHandle,
    killer: Killer,
    parser: vt100::Parser,
    cols: u16,
    rows: u16,
    /// Si el shell ya emitió `Exited(code)`. El panel se queda visible
    /// para que el usuario pueda leer la última salida antes de cerrar.
    exit_code: Option<i32>,
    /// CWD inicial — útil para el header sin tener que tocar /proc.
    cwd: String,
    started_at: Instant,
}

impl ShumaTermState {
    /// Bytes que el módulo ya consumió desde el PTY. Útil para tests y
    /// debug — no es parte del contrato Tier 1.
    pub fn screen_contents(&self) -> String {
        self.parser.screen().contents()
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }
    pub fn rows(&self) -> u16 {
        self.rows
    }
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
    pub fn cwd(&self) -> &str {
        &self.cwd
    }
}

impl Drop for ShumaTermState {
    fn drop(&mut self) {
        // Si el host descarta el state (panel cerrado), no dejamos al
        // shell huérfano consumiendo CPU. SIGTERM educado primero;
        // shuma-exec se encarga del SIGKILL si hace falta.
        self.killer.term();
    }
}

/// Lanza el shell por defecto (`$SHELL`, fallback `/bin/sh`) en `cwd`
/// con tamaño de PTY por defecto.
pub fn spawn(cwd: impl Into<String>) -> ShumaTermState {
    spawn_with(cwd, default_shell(), Vec::new(), DEFAULT_COLS, DEFAULT_ROWS)
}

/// Variante con control fino de programa, args y tamaño.
pub fn spawn_with(
    cwd: impl Into<String>,
    program: String,
    args: Vec<String>,
    cols: u16,
    rows: u16,
) -> ShumaTermState {
    let cwd = cwd.into();
    let spec = CommandSpec {
        exec: Exec::Pty { program, args, cols, rows },
        cwd: cwd.clone(),
        capture_limit: 0,
        spill_path: None,
        stdin_data: None,
        capture_stages: false,
    };
    let handle = shuma_exec::run(&spec);
    let killer = handle.killer();
    ShumaTermState {
        handle,
        killer,
        parser: vt100::Parser::new(rows, cols, SCROLLBACK),
        cols,
        rows,
        exit_code: None,
        cwd,
        started_at: Instant::now(),
    }
}

fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

// =====================================================================
// Msg / Action
// =====================================================================

/// Vocabulario interno. El host lo wrapea en su `Msg`.
#[derive(Debug, Clone)]
pub enum ShumaTermMsg {
    /// Símbolo conveniente para que el host dispatche al detectar el
    /// shortcut. El módulo no crea el state él mismo — el host lo crea
    /// con [`spawn`] porque conoce el cwd canónico de la app.
    Open,
    /// El usuario pidió cerrar el panel.
    Close,
    /// Tecla mientras el panel está enfocado. Se traduce a bytes y se
    /// reenvía al PTY.
    KeyInput(KeyEvent),
    /// Tick del host: drena eventos pendientes del PTY (bytes y exit).
    /// El host debe enviar este Msg de forma periódica (en cada frame,
    /// o cuando hay actividad). Sin Tick el terminal no avanza.
    Tick,
    /// Mata el shell (SIGTERM); el panel queda visible mostrando el
    /// estado final hasta que el host reciba `Close`.
    Terminate,
}

/// Efecto solicitado al host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShumaTermAction {
    None,
    /// El host debería remover el state del modelo.
    Close,
    /// El host debería actualizar su barra de estado.
    SetStatus(String),
}

// =====================================================================
// apply / on_key / open_shortcut
// =====================================================================

/// Aplica un mensaje al estado.
pub fn apply(state: &mut ShumaTermState, msg: ShumaTermMsg) -> ShumaTermAction {
    match msg {
        ShumaTermMsg::Open => ShumaTermAction::None,
        ShumaTermMsg::Close => ShumaTermAction::Close,
        ShumaTermMsg::Terminate => {
            state.killer.term();
            ShumaTermAction::SetStatus("shuma · SIGTERM".into())
        }
        ShumaTermMsg::Tick => drain(state),
        ShumaTermMsg::KeyInput(ev) => {
            // Interceptaciones del módulo (no llegan al PTY):
            //   Ctrl+Shift+W → cierra el panel.
            // Cualquier otra combinación se traduce a bytes y se envía.
            if ev.state == KeyState::Pressed
                && ev.modifiers.ctrl
                && ev.modifiers.shift
                && matches!(&ev.key, Key::Character(s) if s.eq_ignore_ascii_case("w"))
            {
                return ShumaTermAction::Close;
            }
            let bytes = key_to_bytes(&ev);
            if !bytes.is_empty() {
                state.handle.write_input(bytes);
            }
            ShumaTermAction::None
        }
    }
}

/// Routing de teclas cuando el panel está enfocado. Devuelve `Some` para
/// todo evento `Pressed` — el terminal **traga** las teclas; el host no
/// debe reusarlas para sus propios atajos mientras este panel esté
/// activo (la excepción es el atajo de apertura, que el host filtra
/// antes de delegar).
pub fn on_key(_state: &ShumaTermState, event: &KeyEvent) -> Option<ShumaTermMsg> {
    if event.state != KeyState::Pressed {
        return None;
    }
    Some(ShumaTermMsg::KeyInput(event.clone()))
}

/// El atajo recomendado para abrir: **Ctrl+`** (backtick), igual que
/// VS Code. Los hosts pueden ignorarlo y usar otro.
pub fn open_shortcut(event: &KeyEvent) -> bool {
    event.state == KeyState::Pressed
        && event.modifiers.ctrl
        && !event.modifiers.shift
        && matches!(&event.key, Key::Character(s) if s == "`")
}

// =====================================================================
// Drenado del PTY
// =====================================================================

fn drain(state: &mut ShumaTermState) -> ShumaTermAction {
    let mut bytes_in = 0usize;
    let mut final_action = ShumaTermAction::None;
    for ev in state.handle.try_events() {
        match ev {
            RunEvent::Bytes(b) => {
                bytes_in += b.len();
                state.parser.process(&b);
            }
            RunEvent::Exited(code) => {
                state.exit_code = Some(code);
                let elapsed = state.started_at.elapsed().as_secs_f64();
                final_action = ShumaTermAction::SetStatus(format!(
                    "shuma · exit {code} · {elapsed:.1}s"
                ));
            }
            RunEvent::Failed(err) => {
                state.exit_code = Some(-1);
                final_action =
                    ShumaTermAction::SetStatus(format!("shuma · falló: {err}"));
            }
            // Stdout/Stderr/Truncated/Spilled no aplican al modo Pty.
            _ => {}
        }
    }
    if matches!(final_action, ShumaTermAction::None) && bytes_in > 0 {
        // Nada que reportar — el repaint que el host hará por el frame
        // basta para mostrar lo nuevo.
        ShumaTermAction::None
    } else {
        final_action
    }
}

// =====================================================================
// Mapeo KeyEvent → bytes
// =====================================================================

/// Convierte un `KeyEvent` ya recibido en los bytes que un terminal
/// xterm espera. Cubre el subset usable (chars + control + flechas +
/// home/end/page + fn keys), suficiente para shells modernos, TUIs
/// (vim, htop, less) y CLIs interactivas (claude code, fzf).
pub fn key_to_bytes(ev: &KeyEvent) -> Vec<u8> {
    if ev.state != KeyState::Pressed {
        return Vec::new();
    }

    // Teclas con nombre primero: flechas, etc. Se mapean a CSI/SS3
    // estándar (xterm-256color).
    if let Key::Named(named) = &ev.key {
        return named_to_bytes(*named);
    }

    // Caracter: si hay Ctrl+letra → control byte (Ctrl+C = 0x03).
    if let Key::Character(s) = &ev.key {
        if ev.modifiers.ctrl && !ev.modifiers.alt {
            if let Some(b) = ctrl_byte(s) {
                return vec![b];
            }
        }
        // Alt+x → ESC + x (convención xterm meta-sends-escape).
        if ev.modifiers.alt {
            let mut out = vec![0x1b];
            out.extend_from_slice(s.as_bytes());
            return out;
        }
    }

    // Caso general: si el backend ya nos dio el texto resultante
    // (con shift/IME aplicados), eso es lo correcto para mandar.
    if let Some(text) = &ev.text {
        return text.as_bytes().to_vec();
    }
    Vec::new()
}

fn named_to_bytes(k: NamedKey) -> Vec<u8> {
    match k {
        // PTYs en modo raw esperan CR para Enter; el driver convierte a LF.
        NamedKey::Enter => b"\r".to_vec(),
        // Backspace moderno = DEL (0x7f). Los shells lo entienden mejor
        // que 0x08, que se reserva para ^H en TUIs viejos.
        NamedKey::Backspace => vec![0x7f],
        NamedKey::Tab => b"\t".to_vec(),
        NamedKey::Escape => vec![0x1b],
        NamedKey::ArrowUp => b"\x1b[A".to_vec(),
        NamedKey::ArrowDown => b"\x1b[B".to_vec(),
        NamedKey::ArrowRight => b"\x1b[C".to_vec(),
        NamedKey::ArrowLeft => b"\x1b[D".to_vec(),
        NamedKey::Home => b"\x1b[H".to_vec(),
        NamedKey::End => b"\x1b[F".to_vec(),
        NamedKey::PageUp => b"\x1b[5~".to_vec(),
        NamedKey::PageDown => b"\x1b[6~".to_vec(),
        NamedKey::Delete => b"\x1b[3~".to_vec(),
        NamedKey::Insert => b"\x1b[2~".to_vec(),
        NamedKey::F1 => b"\x1bOP".to_vec(),
        NamedKey::F2 => b"\x1bOQ".to_vec(),
        NamedKey::F3 => b"\x1bOR".to_vec(),
        NamedKey::F4 => b"\x1bOS".to_vec(),
        NamedKey::F5 => b"\x1b[15~".to_vec(),
        NamedKey::F6 => b"\x1b[17~".to_vec(),
        NamedKey::F7 => b"\x1b[18~".to_vec(),
        NamedKey::F8 => b"\x1b[19~".to_vec(),
        NamedKey::F9 => b"\x1b[20~".to_vec(),
        NamedKey::F10 => b"\x1b[21~".to_vec(),
        NamedKey::F11 => b"\x1b[23~".to_vec(),
        NamedKey::F12 => b"\x1b[24~".to_vec(),
        _ => Vec::new(),
    }
}

/// Ctrl+letter → byte de control ASCII (Ctrl+A=1, Ctrl+B=2, ..., Ctrl+Z=26).
/// Maneja también Ctrl+@ (NUL), Ctrl+[ (ESC), Ctrl+\\ (FS), Ctrl+] (GS),
/// Ctrl+^ (RS), Ctrl+_ (US), Ctrl+? (DEL).
fn ctrl_byte(s: &str) -> Option<u8> {
    let c = s.chars().next()?;
    match c {
        'a'..='z' => Some((c as u8) - b'a' + 1),
        'A'..='Z' => Some((c as u8) - b'A' + 1),
        '@' => Some(0),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        ' ' => Some(0), // Ctrl+Space = NUL, convención xterm
        _ => None,
    }
}

// =====================================================================
// View
// =====================================================================

/// Paleta visual del terminal. Monospace; fondo más oscuro que el
/// panel general para que el terminal "viva" visualmente.
#[derive(Debug, Clone)]
pub struct ShumaTermPalette {
    pub bg_panel: llimphi_ui::llimphi_raster::peniko::Color,
    pub bg_header: llimphi_ui::llimphi_raster::peniko::Color,
    pub fg_text: llimphi_ui::llimphi_raster::peniko::Color,
    pub fg_muted: llimphi_ui::llimphi_raster::peniko::Color,
}

impl ShumaTermPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_panel: t.bg_panel_alt,
            bg_header: t.bg_panel,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
        }
    }
}

const HEADER_H: f32 = 18.0;
const ROW_H: f32 = 14.0;
const CHAR_W: f32 = 7.5;

/// Render del panel. `to_host` mapea cada `ShumaTermMsg` al `Msg` del
/// host. `height_px` es la altura total del panel — el módulo divide
/// entre header + grid.
pub fn view<HostMsg, F>(
    state: &ShumaTermState,
    palette: &ShumaTermPalette,
    height_px: f32,
    to_host: F,
) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
    F: Fn(ShumaTermMsg) -> HostMsg + Copy + 'static,
{
    let _ = to_host; // v0 no monta eventos puntuales sobre el grid

    let header_text = match state.exit_code {
        Some(code) => format!(
            "shuma · {} · exit {code} · Ctrl+Shift+W cierra",
            state.cwd
        ),
        None => format!(
            "shuma · {} · {}×{} · Ctrl+Shift+W cierra · Esc envía al shell",
            state.cwd, state.cols, state.rows
        ),
    };

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(HEADER_H) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_header)
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start);

    let contents = state.parser.screen().contents();
    let grid_h = (height_px - HEADER_H).max(0.0);
    let max_rows = ((grid_h / ROW_H) as usize).max(1);

    // Tomamos las últimas `max_rows` líneas — preferimos mostrar el
    // tail (donde está el cursor / prompt) si el render no alcanza
    // para toda la pantalla.
    let all_lines: Vec<&str> = contents.split('\n').collect();
    let start = all_lines.len().saturating_sub(max_rows);
    let mut rows: Vec<View<HostMsg>> = Vec::with_capacity(max_rows);
    for line in &all_lines[start..] {
        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(ROW_H) },
                padding: Rect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                flex_shrink: 0.0,
                ..Default::default()
            })
            .fill(palette.bg_panel)
            .text_aligned((*line).to_string(), 11.0, palette.fg_text, Alignment::Start),
        );
    }
    // Si el render quedó corto, rellenamos con líneas vacías para que el
    // panel mantenga su altura visual.
    while rows.len() < max_rows {
        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(ROW_H) },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .fill(palette.bg_panel),
        );
    }

    let mut children: Vec<View<HostMsg>> = Vec::with_capacity(1 + rows.len());
    children.push(header);
    children.extend(rows);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(height_px) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .children(children)
}

/// Estimación heurística de cuántas columnas caben en `width_px` con la
/// fuente actual. Útil para que el host calcule el tamaño antes de
/// llamar a [`spawn_with`].
pub fn cols_for_width(width_px: f32) -> u16 {
    ((width_px / CHAR_W).floor() as u16).max(20)
}

/// Idem para filas a partir de la altura disponible del panel
/// (descontando el header).
pub fn rows_for_height(height_px: f32) -> u16 {
    (((height_px - HEADER_H) / ROW_H).floor() as u16).max(5)
}
