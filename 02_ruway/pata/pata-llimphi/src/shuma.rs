//! El `shuma_input` y su despliegue **Quake**.
//!
//! La frontera del SDD §5: el marco (`pata`) provee el borde; `shuma` provee el
//! contenido. `shuma_input` es el cabezal que vive en una barra; al activarlo
//! (click o hotkey) el frontend **despliega un drawer** estilo Quake sobre el
//! escritorio, con un input que captura el teclado. Repliega al cerrar.
//!
//! La ejecución del comando es, estrictamente, trabajo de `shuma` (no de
//! `pata`). El puente del SDD §5 ya existe: [`ejecutar`] corre el comando por
//! el **ejecutor real de shuma** (`shuma-exec`) —captura acotada, eventos en
//! streaming— en vez de un `sh -c` pelado. El mecanismo del drawer no cambia.

use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style},
    Rect as TaffyRect,
};
use llimphi_ui::View;

use pata_core::WidgetSpec;
use shuma_line::{split_pipeline, tokenize, Dialect};

use crate::Msg;

/// Alto máximo del drawer, como fracción de la pantalla.
const DRAWER_FRAC: f32 = 0.45;

/// Una línea de salida con su naturaleza, para colorearla en la card.
#[derive(Clone, Debug)]
pub struct OutLine {
    /// `true` si vino por stderr (se pinta en color de error).
    pub err: bool,
    pub text: String,
}

/// Una *card* del drawer: un comando ejecutado con sus etapas de pipe, su
/// salida y su código de salida. Es el modelo de paridad con el shell de
/// shuma (cards + etapas clickeables), que el render del marco pinta.
#[derive(Clone, Debug)]
pub struct DrawerBlock {
    /// La línea tal cual se tecleó.
    pub cmd: String,
    /// Etiquetas de cada etapa del pipe (de `shuma-line`) — chips clickeables
    /// que re-ejecutan la línea truncada hasta esa etapa.
    pub stages: Vec<String>,
    /// Líneas de salida (stdout/stderr entremezcladas en orden de llegada).
    pub lines: Vec<OutLine>,
    /// Código de salida; `None` mientras el comando sigue corriendo.
    pub exit: Option<i32>,
    /// `true` si la card está plegada (sólo se ve el encabezado).
    pub collapsed: bool,
}

/// El resultado estructurado de una corrida — lo que un hilo de fondo manda de
/// vuelta para rellenar la card pendiente.
#[derive(Clone, Debug)]
pub struct RunResult {
    pub lines: Vec<OutLine>,
    pub exit: Option<i32>,
}

/// Las etiquetas de las etapas de pipe de una línea (vía `shuma-line`): el
/// `comando` de cada etapa, o el texto crudo si no se reconoció. Vacío si la
/// línea no tiene pipe (una sola etapa no amerita chips).
pub fn stage_labels(cmd: &str) -> Vec<String> {
    let pipeline = split_pipeline(&tokenize(cmd, Dialect::Bash));
    if pipeline.stages.len() < 2 {
        return Vec::new();
    }
    pipeline
        .stages
        .iter()
        .map(|s| {
            s.command.clone().unwrap_or_else(|| {
                // Sin comando reconocido: el primer argumento, o «·».
                s.args.first().cloned().unwrap_or_else(|| "·".into())
            })
        })
        .collect()
}

/// La línea truncada hasta la etapa `upto` inclusive — lo que re-ejecuta el
/// clic en una chip (`a | b | c`, clic en `b` → `a | b`). Reconstruye cada
/// etapa desde su comando+args.
pub fn truncated_line(cmd: &str, upto: usize) -> String {
    let pipeline = split_pipeline(&tokenize(cmd, Dialect::Bash));
    pipeline
        .stages
        .iter()
        .take(upto + 1)
        .map(|s| {
            let mut parts = Vec::new();
            if let Some(c) = &s.command {
                parts.push(c.clone());
            }
            parts.extend(s.args.iter().cloned());
            parts.join(" ")
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// El estado del cabezal del shell y su drawer. Vive en el `Model` del frontend
/// —es interacción, no modelo de dominio—, no en `pata-core`.
pub struct ShumaState {
    /// `true` cuando el drawer está desplegado.
    pub open: bool,
    /// El comando que se está escribiendo.
    pub buffer: String,
    /// Historial de comandos del drawer, uno por *card* — paridad con el
    /// shell de shuma (cada `$ cmd` con sus etapas de pipe, su salida y su
    /// código). El más reciente va al final.
    pub blocks: Vec<DrawerBlock>,
    /// `true` mientras el comando corre en segundo plano.
    pub pending: bool,
    /// Hotkey que abre/cierra el drawer (de la prop `hotkey`), o `None`.
    pub hotkey: Option<String>,
    /// Prompt al frente del input (`›`, `$`, …).
    pub prompt: String,
    /// Placeholder cuando el buffer está vacío.
    pub placeholder: String,
    /// Animación de despliegue `0..1` (0 = replegado, 1 = desplegado).
    pub anim: Tween<f32>,
    /// `true` si el config declaró algún `shuma_input` (si no, no hay cabezal
    /// ni drawer).
    pub present: bool,
}

impl ShumaState {
    /// Tope de cards en el historial — más allá, las viejas se descartan.
    const MAX_BLOCKS: usize = 12;

    /// Empuja una card nueva en estado «corriendo» para `cmd` (con sus etapas
    /// de pipe ya resueltas) y marca el drawer como pendiente. Acota el
    /// historial a [`MAX_BLOCKS`]. Lo usan ambos backends al lanzar.
    pub fn push_pending(&mut self, cmd: String) {
        let stages = stage_labels(&cmd);
        self.blocks.push(DrawerBlock {
            cmd,
            stages,
            lines: Vec::new(),
            exit: None,
            collapsed: false,
        });
        if self.blocks.len() > Self::MAX_BLOCKS {
            let drop = self.blocks.len() - Self::MAX_BLOCKS;
            self.blocks.drain(0..drop);
        }
        self.pending = true;
    }

    /// Rellena la última card (la pendiente) con el resultado de la corrida.
    pub fn finish_last(&mut self, res: RunResult) {
        self.pending = false;
        if let Some(b) = self.blocks.last_mut() {
            b.lines = res.lines;
            b.exit = res.exit;
        }
    }
}

impl Default for ShumaState {
    fn default() -> Self {
        Self {
            open: false,
            buffer: String::new(),
            blocks: Vec::new(),
            pending: false,
            hotkey: None,
            prompt: "›".into(),
            placeholder: "shuma".into(),
            anim: Tween::idle(0.0),
            present: false,
        }
    }
}

impl ShumaState {
    /// Construye el estado desde la spec del `shuma_input` (prompt/placeholder/
    /// hotkey). Marca `present = true`.
    pub fn from_spec(spec: &WidgetSpec) -> Self {
        let hotkey = spec.str_prop("hotkey", "");
        Self {
            prompt: spec.str_prop("prompt", "›").to_string(),
            placeholder: spec.str_prop("placeholder", "shuma").to_string(),
            hotkey: if hotkey.is_empty() {
                None
            } else {
                Some(hotkey.to_string())
            },
            present: true,
            ..Self::default()
        }
    }

    /// `true` si el drawer debe pintarse (abierto o aún animando el cierre).
    pub fn visible(&self) -> bool {
        self.open || self.anim.value() > 0.01
    }
}

/// El cabezal clicable que va en la barra: prompt + buffer/placeholder. Un click
/// despliega el drawer.
pub fn headline_view(state: &ShumaState, theme: &Theme) -> View<Msg> {
    let texto = if state.buffer.is_empty() {
        state.placeholder.clone()
    } else {
        state.buffer.clone()
    };
    let color = if state.buffer.is_empty() {
        theme.fg_muted
    } else {
        theme.fg_text
    };

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: length(220.0_f32),
            height: length(24.0_f32),
        },
        padding: TaffyRect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(6.0)
    .hover_fill(theme.bg_button_hover)
    .on_click(Msg::ShumaToggle)
    .children(vec![
        chip_text(&state.prompt, 13.0, theme.accent),
        chip_text(&texto, 13.0, color),
    ])
}

/// El drawer desplegado: scrim que cierra al click + panel inferior con el input
/// y la salida. `None` si no hay nada que mostrar.
pub fn drawer_overlay(state: &ShumaState, screen: (i32, i32), theme: &Theme) -> Option<View<Msg>> {
    if !state.visible() {
        return None;
    }
    let t = state.anim.value().clamp(0.0, 1.0);
    let (_sw, sh) = screen;
    let alto = (sh as f32 * DRAWER_FRAC * t).max(1.0);

    // Línea del input: prompt + buffer + cursor.
    let linea = {
        let mut s = state.buffer.clone();
        s.push('▏'); // cursor
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(8.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![
            chip_text(&state.prompt, 16.0, theme.accent),
            chip_text(&s, 16.0, theme.fg_text),
        ])
    };

    // Cuerpo: las cards del historial (paridad con el shell de shuma).
    let cuerpo = blocks_view(state, theme);

    let panel = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: auto(),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: length(alto),
        },
        flex_direction: FlexDirection::Column,
        padding: TaffyRect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(16.0_f32),
            bottom: length(16.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(12.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![linea, cuerpo]);

    // Scrim a pantalla completa: oscurece el fondo y cierra al click.
    let scrim = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .alpha(0.55 * t)
    .on_click(Msg::ShumaToggle)
    .children(vec![panel]);

    Some(scrim)
}

/// El **cuerpo** del drawer (sin scrim ni posición absoluta), pensado para
/// llenar el contenedor que le da el backend `wlr-layer-shell`: ahí la propia
/// layer surface ya *es* el panel del Quake (la barra crece hacia arriba), así
/// que no hace falta scrim ni animación. Línea de input (prompt + buffer +
/// cursor) arriba, salida del último comando debajo.
pub fn drawer_body_view(state: &ShumaState, theme: &Theme) -> View<Msg> {
    let mut buf = state.buffer.clone();
    buf.push('▏'); // cursor
    let linea = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        chip_text(&state.prompt, 16.0, theme.accent),
        chip_text(&buf, 16.0, theme.fg_text),
    ]);

    let cuerpo = blocks_view(state, theme);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: TaffyRect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(16.0_f32),
            bottom: length(16.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(12.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![linea, cuerpo])
}

/// El cuerpo del drawer: las cards del historial (paridad con el shell de
/// shuma), o la pista de uso si no hay ninguna. Lo comparten los dos backends
/// (winit con scrim y layer-shell). Cada card es un `$ cmd` con sus etapas de
/// pipe clickeables, su salida coloreada por stdout/stderr y su código.
fn blocks_view(state: &ShumaState, theme: &Theme) -> View<Msg> {
    let col = Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: auto() },
        // Pegado abajo: lo más nuevo queda a la vista, lo viejo clipa arriba.
        justify_content: Some(JustifyContent::FlexEnd),
        gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
        ..Default::default()
    };
    if state.blocks.is_empty() {
        return View::new(col).text(
            "shuma se despliega aquí — escribí un comando y Enter (Esc cierra)".to_string(),
            13.0,
            theme.fg_muted,
        );
    }
    let cards = state
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| card_view(i, b, theme))
        .collect();
    View::new(col).children(cards)
}

/// Una card: encabezado (`$` plegable + etapas de pipe clickeables) y, si no
/// está plegada, las líneas de salida y el código de salida.
fn card_view(idx: usize, b: &DrawerBlock, theme: &Theme) -> View<Msg> {
    // Encabezado: el `$` pliega/despliega; las etapas re-ejecutan la línea
    // truncada hasta esa etapa (o, sin pipe, la línea entera).
    let mut head: Vec<View<Msg>> = vec![chip_text("$", 14.0, theme.accent)
        .on_click(Msg::ShumaCollapse(idx))];
    if b.stages.is_empty() {
        head.push(chip_text(&b.cmd, 14.0, theme.fg_text).on_click(Msg::ShumaRunLine(b.cmd.clone())));
    } else {
        for (si, label) in b.stages.iter().enumerate() {
            if si > 0 {
                head.push(chip_text("|", 14.0, theme.fg_muted));
            }
            head.push(
                chip_text(label, 14.0, theme.accent)
                    .on_click(Msg::ShumaRunLine(truncated_line(&b.cmd, si))),
            );
        }
    }
    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(head);

    let mut col: Vec<View<Msg>> = vec![header];
    if !b.collapsed {
        if b.exit.is_none() {
            col.push(out_line("…", theme.fg_muted));
        }
        for l in &b.lines {
            let c = if l.err { theme.fg_destructive } else { theme.fg_text };
            col.push(out_line(&l.text, c));
        }
        if let Some(code) = b.exit {
            if code != 0 {
                col.push(out_line(&format!("exit {code}"), theme.fg_destructive));
            }
        }
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: auto() },
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .children(col)
}

/// Una línea de salida a ancho completo.
fn out_line(t: &str, color: llimphi_theme::Color) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: auto() },
        ..Default::default()
    })
    .text(t.to_string(), 13.0, color)
}

/// Un texto suelto, centrado verticalmente.
fn chip_text(t: &str, size: f32, color: llimphi_theme::Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: auto(),
            height: length(size + 6.0),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(t.to_string(), size, color)
}

/// El puente pata→shuma (SDD §5): ejecuta `cmd` por el **ejecutor real de
/// shuma** (`shuma-exec`) y devuelve su stdout, o el stderr/código como error.
/// Reúne los eventos en streaming hasta el final; la captura está acotada a
/// [`CAPTURE_CAP`] (lo que excede se marca como truncado). Bloqueante: se
/// llama desde un hilo de fondo (`Handle::spawn` o `std::thread`).
pub fn ejecutar(cmd: &str) -> RunResult {
    use shuma_exec::{run, CommandSpec, RunEvent};

    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/".to_string());
    let spec = CommandSpec::shell(cmd, cwd).with_limit(CAPTURE_CAP);

    let mut lines: Vec<OutLine> = Vec::new();
    let mut exit: Option<i32> = None;
    for ev in run(&spec).wait_all() {
        match ev {
            RunEvent::Stdout(t) => lines.push(OutLine { err: false, text: t }),
            RunEvent::Stderr(t) => lines.push(OutLine { err: true, text: t }),
            RunEvent::Exited(c) => exit = Some(c),
            RunEvent::Failed(m) => lines.push(OutLine {
                err: true,
                text: format!("no pude lanzar: {m}"),
            }),
            RunEvent::Truncated => lines.push(OutLine {
                err: true,
                text: format!("… (salida truncada a {CAPTURE_CAP} bytes)"),
            }),
            RunEvent::Spilled(p) => lines.push(OutLine {
                err: false,
                text: format!("… (salida volcada a {p})"),
            }),
            // Sólo aparece en modo PTY; el drawer no lo usa.
            RunEvent::Bytes(_) => {}
        }
    }
    RunResult { lines, exit }
}

/// Tope de captura del drawer, en bytes — un comando charlatán no infla la RAM.
const CAPTURE_CAP: usize = 256 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_linea_sin_pipe_no_tiene_chips() {
        assert!(stage_labels("ls -la").is_empty());
        assert!(stage_labels("").is_empty());
    }

    #[test]
    fn las_etapas_de_un_pipe_son_los_comandos() {
        assert_eq!(
            stage_labels("ls -la | grep foo | sort"),
            vec!["ls".to_string(), "grep".to_string(), "sort".to_string()]
        );
    }

    #[test]
    fn la_linea_truncada_corta_en_la_etapa_clickeada() {
        let cmd = "ls -la | grep foo | sort";
        assert_eq!(truncated_line(cmd, 0), "ls -la");
        assert_eq!(truncated_line(cmd, 1), "ls -la | grep foo");
        assert_eq!(truncated_line(cmd, 2), "ls -la | grep foo | sort");
    }

    #[test]
    fn push_pending_resuelve_etapas_y_acota_el_historial() {
        let mut s = ShumaState::default();
        s.push_pending("a | b".into());
        let last = s.blocks.last().unwrap();
        assert_eq!(last.stages, vec!["a".to_string(), "b".to_string()]);
        assert!(last.exit.is_none() && s.pending);
        // Más allá del tope, las viejas se descartan.
        for _ in 0..ShumaState::MAX_BLOCKS + 5 {
            s.push_pending("x".into());
        }
        assert_eq!(s.blocks.len(), ShumaState::MAX_BLOCKS);
    }

    #[test]
    fn finish_last_rellena_la_card_pendiente() {
        let mut s = ShumaState::default();
        s.push_pending("echo hi".into());
        s.finish_last(RunResult {
            lines: vec![OutLine { err: false, text: "hi".into() }],
            exit: Some(0),
        });
        let last = s.blocks.last().unwrap();
        assert_eq!(last.exit, Some(0));
        assert_eq!(last.lines.len(), 1);
        assert!(!s.pending);
    }
}
