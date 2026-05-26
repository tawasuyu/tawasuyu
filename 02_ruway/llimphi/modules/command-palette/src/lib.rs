//! `llimphi-module-command-palette` — paleta de comandos reutilizable.
//!
//! Equivalente a Ctrl+Shift+P de VS Code: el host declara una lista
//! plana de [`Command`]s (id opaco + título visible + grupo + hint del
//! atajo) y el módulo presenta un overlay con input + resultados
//! rankeados por fuzzy match. Cuando el user pica uno, el módulo emite
//! [`PaletteAction::Invoke`] con el `id` — el host hace match y
//! dispatcha lo que corresponda en su propio Msg.
//!
//! El módulo no sabe **qué** hacen los comandos. Eso es deliberado:
//! mantiene al palette agnóstico de la app, y permite que aplicaciones
//! muy distintas (un editor, un explorador de grafos, un viewer de
//! imágenes) lo enchufen con sus respectivas listas sin acoplarse.
//!
//! Sigue el contrato Llimphi de `docs/MODULES.md`:
//! `State + Msg + Action + apply/on_key/open_shortcut/view + Palette`.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

/// Capabilities que aporta este módulo al host.
pub const CAPABILITIES: &[&str] = &["editor.command-palette"];

/// Tope de resultados rankeados visibles.
pub const MAX_RESULTS: usize = 200;

const BAR_H: f32 = 280.0;
const ROW_H: f32 = 22.0;
const MAX_VISIBLE: usize = 10;

/// Una entrada del catálogo de comandos que el host arma.
///
/// Los campos son convencionales:
/// - `id`: identificador opaco, único dentro del catálogo del host.
///   El host lo recibe en [`PaletteAction::Invoke`] y hace match a su
///   propio Msg. Por convención, formato `"namespace.action"` (ej.
///   `"editor.save"`, `"terminal.open"`).
/// - `title`: lo que el user lee. Idealmente en lengua de la app.
/// - `group`: categoría visible a la derecha de la fila (ej. `"Editor"`,
///   `"Terminal"`, `"LSP"`). Sirve para escanear visualmente.
/// - `shortcut`: hint textual del atajo nativo del comando, si existe
///   (ej. `"Ctrl+S"`). Sólo decorativo — el módulo no captura nada
///   distinto a Enter/Esc/↑↓.
#[derive(Debug, Clone)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub group: String,
    pub shortcut: Option<String>,
}

impl Command {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        group: impl Into<String>,
    ) -> Self {
        Self { id: id.into(), title: title.into(), group: group.into(), shortcut: None }
    }

    pub fn with_shortcut(mut self, s: impl Into<String>) -> Self {
        self.shortcut = Some(s.into());
        self
    }
}

/// Estado interno. `results` son índices al slice de commands que pasa
/// el host: el módulo no copia, sólo guarda índices.
pub struct PaletteState {
    pub input: TextInputState,
    pub results: Vec<usize>,
    pub selected: usize,
}

impl Default for PaletteState {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl PaletteState {
    pub fn new_empty() -> Self {
        Self {
            input: TextInputState::new(),
            results: Vec::new(),
            selected: 0,
        }
    }

    /// Crea un palette pre-poblado con todos los comandos sin filtro,
    /// listo para mostrar después del shortcut de apertura.
    pub fn new(commands: &[Command]) -> Self {
        let mut s = Self::new_empty();
        refilter(&mut s, commands);
        s
    }
}

/// Vocabulario interno. El host lo wrapea en su Msg.
#[derive(Clone)]
pub enum PaletteMsg {
    /// Símbolo conveniente para que el host dispatche al detectar el
    /// shortcut. El módulo no construye el state él mismo — eso lo hace
    /// el host con la lista canónica de commands.
    Open,
    Close,
    KeyInput(KeyEvent),
    Nav(i32),
    /// Enter: invoca el comando seleccionado.
    Apply,
}

/// Efecto solicitado al host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction {
    None,
    /// El host debería remover el state del modelo.
    Close,
    /// El host debería ejecutar el comando con este `id`. El módulo NO
    /// se cierra automáticamente — el host decide (típicamente sí, igual
    /// que un menú).
    Invoke(String),
}

/// Aplica un mensaje al estado.
pub fn apply(
    state: &mut PaletteState,
    msg: PaletteMsg,
    commands: &[Command],
) -> PaletteAction {
    match msg {
        PaletteMsg::Open => PaletteAction::None,
        PaletteMsg::Close => PaletteAction::Close,
        PaletteMsg::KeyInput(ev) => {
            state.input.apply_key(&ev);
            refilter(state, commands);
            PaletteAction::None
        }
        PaletteMsg::Nav(d) => {
            let n = state.results.len() as i32;
            if n > 0 {
                state.selected = (state.selected as i32 + d).rem_euclid(n) as usize;
            }
            PaletteAction::None
        }
        PaletteMsg::Apply => {
            let Some(&cmd_idx) = state.results.get(state.selected) else {
                return PaletteAction::None;
            };
            let Some(cmd) = commands.get(cmd_idx) else {
                return PaletteAction::None;
            };
            PaletteAction::Invoke(cmd.id.clone())
        }
    }
}

/// Routing de teclas cuando el palette está abierto.
pub fn on_key(_state: &PaletteState, event: &KeyEvent) -> Option<PaletteMsg> {
    if event.state != KeyState::Pressed {
        return None;
    }
    Some(match &event.key {
        Key::Named(NamedKey::Escape) => PaletteMsg::Close,
        Key::Named(NamedKey::Enter) => PaletteMsg::Apply,
        Key::Named(NamedKey::ArrowDown) => PaletteMsg::Nav(1),
        Key::Named(NamedKey::ArrowUp) => PaletteMsg::Nav(-1),
        _ => PaletteMsg::KeyInput(event.clone()),
    })
}

/// El atajo recomendado: **Ctrl+Shift+P**, igual que VS Code.
pub fn open_shortcut(event: &KeyEvent) -> bool {
    event.state == KeyState::Pressed
        && event.modifiers.ctrl
        && event.modifiers.shift
        && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("p"))
}

/// Recalcula `state.results` según el query del input. Fuzzy match con
/// `nucleo-matcher` sobre `"title · group"` (mismo string para que el
/// usuario pueda buscar por grupo: "term" matchea "Open Terminal · Editor").
/// Query vacío = lista completa ordenada como vino del host.
/// Cap: [`MAX_RESULTS`].
pub fn refilter(state: &mut PaletteState, commands: &[Command]) {
    let q = state.input.text();
    if q.trim().is_empty() {
        state.results = (0..commands.len().min(MAX_RESULTS)).collect();
        state.selected = 0;
        return;
    }
    use nucleo_matcher::{
        pattern::{CaseMatching, Normalization, Pattern},
        Config, Matcher, Utf32Str,
    };
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pat = Pattern::parse(&q, CaseMatching::Smart, Normalization::Smart);
    let mut scored: Vec<(u32, usize)> = Vec::new();
    let mut buf = Vec::new();
    for (i, cmd) in commands.iter().enumerate() {
        let hay_str = format!("{} {}", cmd.title, cmd.group);
        buf.clear();
        let hay = Utf32Str::new(&hay_str, &mut buf);
        if let Some(score) = pat.score(hay, &mut matcher) {
            scored.push((score, i));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.truncate(MAX_RESULTS);
    state.results = scored.into_iter().map(|(_, i)| i).collect();
    state.selected = 0;
}

/// Paleta visual.
#[derive(Debug, Clone)]
pub struct PalettePalette {
    pub bg_panel: Color,
    pub bg_header: Color,
    pub bg_selected: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    theme: llimphi_theme::Theme,
}

impl PalettePalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_panel: t.bg_panel,
            bg_header: t.bg_panel_alt,
            bg_selected: t.bg_selected,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            theme: t.clone(),
        }
    }
}

/// Render del overlay. `to_host` mapea cada `PaletteMsg` interno al
/// `Msg` de la app.
pub fn view<HostMsg, F>(
    state: &PaletteState,
    commands: &[Command],
    palette: &PalettePalette,
    to_host: F,
) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
    F: Fn(PaletteMsg) -> HostMsg + Copy + 'static,
{
    let header = if state.results.is_empty() {
        format!("command palette · sin matches · {} comandos · Esc cierra", commands.len())
    } else {
        format!(
            "command palette · {} / {} · ↓↑ navega · Enter ejecuta · Esc cierra",
            state.selected + 1,
            state.results.len(),
        )
    };

    let header_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
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
    .text_aligned(header, 10.0, palette.fg_muted, Alignment::Start);

    let tp = TextInputPalette::from_theme(&palette.theme);
    let input_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .children(vec![text_input_view(
        &state.input,
        "filtro: nombre del comando…",
        true,
        &tp,
        to_host(PaletteMsg::Open),
    )]);

    let visible_start = state.selected.saturating_sub(MAX_VISIBLE.saturating_sub(1));
    let visible_end = (visible_start + MAX_VISIBLE).min(state.results.len());
    let mut rows: Vec<View<HostMsg>> = Vec::with_capacity(MAX_VISIBLE);
    for i in visible_start..visible_end {
        let Some(&cmd_idx) = state.results.get(i) else { continue };
        let Some(cmd) = commands.get(cmd_idx) else { continue };
        let label = match (&cmd.shortcut, cmd.group.as_str()) {
            (Some(sc), grp) if !grp.is_empty() => {
                format!("{}    {}    [{sc}]", cmd.title, cmd.group)
            }
            (Some(sc), _) => format!("{}    [{sc}]", cmd.title),
            (None, grp) if !grp.is_empty() => format!("{}    {}", cmd.title, cmd.group),
            (None, _) => cmd.title.clone(),
        };
        let selected = i == state.selected;
        let bg = if selected { palette.bg_selected } else { palette.bg_panel };
        let fg = if selected { palette.fg_text } else { palette.fg_muted };
        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(ROW_H) },
                padding: Rect {
                    left: length(10.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                flex_shrink: 0.0,
                ..Default::default()
            })
            .fill(bg)
            .text_aligned(label, 12.0, fg, Alignment::Start),
        );
    }

    let mut children: Vec<View<HostMsg>> = Vec::with_capacity(2 + rows.len());
    children.push(header_view);
    children.push(input_view);
    children.extend(rows);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(BAR_H) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .children(children)
}
