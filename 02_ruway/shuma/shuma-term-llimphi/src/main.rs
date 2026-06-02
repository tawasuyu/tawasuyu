//! `shuma-term` — terminal interactivo Llimphi sobre el plano de sandokan.
//!
//! Al arrancar abre un **ambiente aislado** (shell en un PTH, vía
//! `sandokan-local::LocalEngine::run_interactive`), se conecta a su socket
//! canónico `<card_id>.sock` (out-of-band, como hará shuma), y:
//!   - los bytes del PTY → `shuma_term::Terminal::feed` → grid → se pinta.
//!   - las teclas → se traducen a bytes y se escriben al socket.
//!
//! Una ventana = un ambiente. La multiventana (varios ambientes a la vez)
//! es el siguiente paso, una vez que el ruteo de teclado por ventana exista.

use std::thread;
use std::time::Duration;

use card_core::{Card, NamespaceSet, Payload};
use sandokan_core::{InteractiveEngine, Intent, PtySize};
use sandokan_local::LocalEngine;
use shuma_term::{Cell, Terminal};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::llimphi_text::{self as text, Alignment, TextBlock, Typesetter};
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, PaintRect, View};

const COLS: usize = 100;
const ROWS: usize = 30;
const FONT_PX: f32 = 15.0;

fn main() {
    llimphi_ui::run::<TermApp>();
}

struct TermApp;

struct Model {
    term: Terminal,
    cell_w: f64,
    cell_h: f64,
    keys: mpsc::UnboundedSender<Vec<u8>>,
    status: String,
}

#[derive(Clone)]
enum Msg {
    /// Bytes crudos del PTY (vienen del socket).
    Bytes(Vec<u8>),
    /// Bytes de input (teclas ya traducidas) a escribir al socket.
    Input(Vec<u8>),
    /// Línea de estado (id del ambiente, sesión terminada, error).
    Status(String),
}

impl App for TermApp {
    type Model = Model;
    type Msg = Msg;

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        // Medir una celda monoespaciada para mapear (col,row) → píxeles.
        let mut ts = Typesetter::new();
        let m = text::measure(&mut ts, &mono_block("M", Color::WHITE, (0.0, 0.0)));
        let cell_w = m.width.max(1.0) as f64;
        let cell_h = m.height.max(1.0) as f64;

        let (keys_tx, keys_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        // Hilo de IO: corre su propio runtime tokio, abre el ambiente, se
        // conecta al <card_id>.sock y bombea bytes en ambos sentidos.
        let h = handle.clone();
        thread::Builder::new()
            .name("shuma-term-io".into())
            .spawn(move || io_loop(h, keys_rx))
            .expect("spawn io thread");

        Model {
            term: Terminal::new(COLS, ROWS),
            cell_w,
            cell_h,
            keys: keys_tx,
            status: "abriendo ambiente…".into(),
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _h: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Bytes(b) => model.term.feed(&b),
            Msg::Input(b) => {
                let _ = model.keys.send(b);
            }
            Msg::Status(s) => model.status = s,
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let snap = GridSnap::from_term(&model.term);
        let (cw, ch) = (model.cell_w, model.cell_h);
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .fill(Color::from_rgba8(0x12, 0x12, 0x16, 0xff))
        .clip(true)
        .paint_with(move |scene, ts, rect: PaintRect| {
            paint_terminal(scene, ts, rect, &snap, cw, ch);
        })
    }

    fn on_key(_model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        let bytes = translate_key(e)?;
        if bytes.is_empty() {
            return None;
        }
        Some(Msg::Input(bytes))
    }

    fn title() -> &'static str {
        "shuma — terminal"
    }

    fn window_title(model: &Self::Model) -> Option<String> {
        Some(format!("shuma — {}", model.status))
    }

    fn initial_size() -> (u32, u32) {
        (920, 560)
    }
}

/// Bucle de IO: abre el ambiente, conecta el socket, y bombea bytes.
fn io_loop(h: Handle<Msg>, keys_rx: mpsc::UnboundedReceiver<Vec<u8>>) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            h.dispatch(Msg::Bytes(format!("runtime tokio: {e}\r\n").into_bytes()));
            return;
        }
    };
    rt.block_on(async move {
        let run_dir = std::env::temp_dir().join(format!("shuma-term-{}", std::process::id()));
        let engine = LocalEngine::in_dir(run_dir);

        let started = match engine
            .run_interactive(
                Intent::new(shell_card()),
                PtySize {
                    rows: ROWS as u16,
                    cols: COLS as u16,
                },
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                h.dispatch(Msg::Bytes(
                    format!("no se pudo abrir el ambiente: {e}\r\n").into_bytes(),
                ));
                return;
            }
        };

        // El front se conecta SIEMPRE al socket canónico de la sesión.
        let sock = engine.session_socket_path(started.card_id);
        for _ in 0..150 {
            if sock.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let stream = match UnixStream::connect(&sock).await {
            Ok(s) => s,
            Err(e) => {
                h.dispatch(Msg::Bytes(format!("socket: {e}\r\n").into_bytes()));
                return;
            }
        };
        h.dispatch(Msg::Status(format!("{}", started.card_id)));

        let (mut rd, mut wr) = stream.into_split();

        // Lector: socket → UI.
        let hr = h.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 8192];
            loop {
                match rd.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => hr.dispatch(Msg::Bytes(buf[..n].to_vec())),
                }
            }
            hr.dispatch(Msg::Status("sesión terminada".into()));
        });

        // Escritor: teclas → socket. Termina cuando la UI cierra el canal
        // (app saliendo). `engine` vive hasta acá → la sesión se mantiene.
        let mut keys_rx = keys_rx;
        while let Some(b) = keys_rx.recv().await {
            if wr.write_all(&b).await.is_err() {
                break;
            }
        }
        drop(engine);
    });
}

/// Card del shell aislado (namespaces user+pid+mount+uts+ipc, misma rootfs).
fn shell_card() -> Card {
    let mut c = Card::new("shuma.terminal");
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
    // Heredamos el env del proceso + forzamos un TERM razonable.
    let mut envp: Vec<(String, String)> = std::env::vars().filter(|(k, _)| k != "TERM").collect();
    envp.push(("TERM".into(), "xterm-256color".into()));
    c.payload = Payload::Native {
        exec: shell,
        argv: vec!["-i".into()],
        envp,
    };
    c.soma.namespaces = NamespaceSet {
        user: true,
        pid: true,
        mount: true,
        uts: true,
        ipc: true,
        net: false,
        cgroup: false,
    };
    c
}

/// Snapshot del grid para mover al closure de `paint_with` ('static).
struct GridSnap {
    rows: Vec<Vec<Cell>>,
    cursor: (usize, usize),
}

impl GridSnap {
    fn from_term(t: &Terminal) -> Self {
        Self {
            rows: (0..t.rows()).map(|y| t.row(y).to_vec()).collect(),
            cursor: t.cursor(),
        }
    }
}

fn paint_terminal(scene: &mut Scene, ts: &mut Typesetter, rect: PaintRect, snap: &GridSnap, cw: f64, ch: f64) {
    let ox = rect.x as f64;
    let oy = rect.y as f64;

    for (y, row) in snap.rows.iter().enumerate() {
        let ry = oy + y as f64 * ch;

        // Fondos: corridas de celdas con el mismo bg (no-default).
        let mut x = 0;
        while x < row.len() {
            let bg = row[x].bg;
            let start = x;
            while x < row.len() && row[x].bg == bg {
                x += 1;
            }
            if !bg.is_default() {
                let (r, g, b) = ansi_rgb(bg.0);
                let r2 = Rect::new(ox + start as f64 * cw, ry, ox + x as f64 * cw, ry + ch);
                scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgba8(r, g, b, 0xff), None, &r2);
            }
        }

        // Texto: corridas con el mismo fg, una llamada de layout por corrida.
        let mut x = 0;
        while x < row.len() {
            let fg = row[x].fg;
            let start = x;
            let mut s = String::new();
            while x < row.len() && row[x].fg == fg {
                s.push(row[x].ch);
                x += 1;
            }
            if s.chars().all(|c| c == ' ') {
                continue;
            }
            let (r, g, b) = if fg.is_default() {
                (0xcc, 0xcc, 0xd0)
            } else {
                ansi_rgb(fg.0)
            };
            let color = Color::from_rgba8(r, g, b, 0xff);
            let origin = (ox + start as f64 * cw, ry);
            let layout = text::layout_block(ts, &mono_block(&s, color, origin));
            text::draw_layout(scene, &layout, color, origin);
        }
    }

    // Cursor: bloque translúcido sobre su celda.
    let (cx, cy) = snap.cursor;
    let cur = Rect::new(
        ox + cx as f64 * cw,
        oy + cy as f64 * ch,
        ox + (cx as f64 + 1.0) * cw,
        oy + (cy as f64 + 1.0) * ch,
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgba8(0xcc, 0xcc, 0xd0, 0x88),
        None,
        &cur,
    );
}

fn mono_block(s: &str, color: Color, origin: (f64, f64)) -> TextBlock<'_> {
    TextBlock {
        text: s,
        size_px: FONT_PX,
        color,
        origin,
        max_width: None,
        alignment: Alignment::Start,
        line_height: 1.0,
        italic: false,
        font_family: Some("monospace".to_string()),
    }
}

/// Paleta ANSI 16 colores (estilo xterm).
fn ansi_rgb(i: u8) -> (u8, u8, u8) {
    match i {
        0 => (0, 0, 0),
        1 => (205, 0, 0),
        2 => (0, 205, 0),
        3 => (205, 205, 0),
        4 => (0, 0, 238),
        5 => (205, 0, 205),
        6 => (0, 205, 205),
        7 => (229, 229, 229),
        8 => (127, 127, 127),
        9 => (255, 0, 0),
        10 => (0, 255, 0),
        11 => (255, 255, 0),
        12 => (92, 92, 255),
        13 => (255, 0, 255),
        14 => (0, 255, 255),
        15 => (255, 255, 255),
        _ => (0xcc, 0xcc, 0xd0),
    }
}

/// Traduce una tecla a los bytes que el shell espera por el PTY.
fn translate_key(e: &KeyEvent) -> Option<Vec<u8>> {
    match &e.key {
        Key::Named(n) => Some(match n {
            NamedKey::Enter => vec![b'\r'],
            NamedKey::Backspace => vec![0x7f],
            NamedKey::Tab => vec![b'\t'],
            NamedKey::Escape => vec![0x1b],
            NamedKey::ArrowUp => b"\x1b[A".to_vec(),
            NamedKey::ArrowDown => b"\x1b[B".to_vec(),
            NamedKey::ArrowRight => b"\x1b[C".to_vec(),
            NamedKey::ArrowLeft => b"\x1b[D".to_vec(),
            NamedKey::Home => b"\x1b[H".to_vec(),
            NamedKey::End => b"\x1b[F".to_vec(),
            NamedKey::Delete => b"\x1b[3~".to_vec(),
            NamedKey::PageUp => b"\x1b[5~".to_vec(),
            NamedKey::PageDown => b"\x1b[6~".to_vec(),
            _ => return None,
        }),
        Key::Character(s) => {
            let c = s.chars().next()?;
            // Ctrl+letra → byte de control (Ctrl-C = 0x03, etc.).
            if e.modifiers.ctrl && c.is_ascii() {
                return Some(vec![(c.to_ascii_uppercase() as u8) & 0x1f]);
            }
            // Texto normal (respeta shift/mayúsculas/símbolos).
            if let Some(t) = &e.text {
                if !t.is_empty() {
                    return Some(t.as_bytes().to_vec());
                }
            }
            Some(s.as_bytes().to_vec())
        }
        // Unidentified / Dead: sin bytes que mandar.
        _ => None,
    }
}
