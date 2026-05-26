//! `llimphi-module-symbol-outline` — outline navegable de símbolos.
//!
//! Equivalente al "Outline" panel de VS Code o "Structure" de JetBrains.
//! El host arma una lista plana de [`SymbolItem`] (funciones, structs,
//! métodos, con su posición en el buffer) y el módulo presenta un
//! overlay con input + lista rankeada por fuzzy. Cuando el user pica
//! uno, el módulo emite [`OutlineAction::GoTo`] y el host mueve el caret.
//!
//! El módulo es **agnóstico de la fuente de símbolos**. El host puede
//! poblarlo desde:
//!
//! - LSP (`textDocument/documentSymbol`) — fuente canónica.
//! - tree-sitter — sirve para archivos sin LSP.
//! - parser propio del lenguaje del host.
//! - una lista hardcodeada (en una app no-código que tenga "secciones").
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
pub const CAPABILITIES: &[&str] = &["editor.symbol-outline"];

pub const MAX_RESULTS: usize = 500;

const BAR_H: f32 = 320.0;
const ROW_H: f32 = 20.0;
const MAX_VISIBLE: usize = 12;

/// Un símbolo del documento. Los campos son convencionales:
///
/// - `name`: nombre visible (`foo`, `MyStruct`, `parse_line`).
/// - `kind`: etiqueta corta del tipo de símbolo (`fn`, `struct`, `method`,
///   `mod`, `const`, …). El módulo la pinta sin interpretar — el host
///   elige el vocabulario (LSP usa `SymbolKind` numérico; el host
///   convierte a string).
/// - `line`, `col`: posición 0-based en el buffer. El módulo no toca
///   coordenadas — sólo las devuelve en `GoTo`.
/// - `container`: nombre del símbolo padre (`Some("MyStruct")` para
///   un método). Visible en el render como anotación a la derecha;
///   también participa del fuzzy match para que tipear el nombre de
///   la clase filtre sus métodos.
/// - `depth`: profundidad jerárquica para indentación visual. El
///   módulo asume que la lista ya viene ordenada (parent antes que
///   children, en orden de aparición).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolItem {
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub col: usize,
    pub container: Option<String>,
    pub depth: u32,
}

/// Estado interno. `results` son índices al slice de symbols que pasa
/// el host: el módulo no copia, sólo guarda índices.
pub struct OutlineState {
    pub input: TextInputState,
    pub results: Vec<usize>,
    pub selected: usize,
}

impl Default for OutlineState {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl OutlineState {
    pub fn new_empty() -> Self {
        Self {
            input: TextInputState::new(),
            results: Vec::new(),
            selected: 0,
        }
    }

    /// Crea un outline poblado con todos los símbolos sin filtro.
    pub fn new(items: &[SymbolItem]) -> Self {
        let mut s = Self::new_empty();
        refilter(&mut s, items);
        s
    }
}

/// Vocabulario interno. El host lo wrapea en su Msg.
#[derive(Clone)]
pub enum OutlineMsg {
    /// Símbolo conveniente que el host emite al detectar el shortcut.
    /// El módulo no construye el state ni la lista él mismo.
    Open,
    Close,
    KeyInput(KeyEvent),
    Nav(i32),
    /// Enter: salta al símbolo seleccionado.
    Apply,
}

/// Efecto solicitado al host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutlineAction {
    None,
    /// El host debería remover el state del modelo.
    Close,
    /// El host debería mover el caret a esta posición del buffer activo.
    /// El módulo NO se cierra automáticamente — el host decide
    /// (típicamente sí, para que la navegación sea "salta y mira").
    GoTo { line: usize, col: usize },
}

/// Aplica un mensaje al estado.
pub fn apply(
    state: &mut OutlineState,
    msg: OutlineMsg,
    items: &[SymbolItem],
) -> OutlineAction {
    match msg {
        OutlineMsg::Open => OutlineAction::None,
        OutlineMsg::Close => OutlineAction::Close,
        OutlineMsg::KeyInput(ev) => {
            state.input.apply_key(&ev);
            refilter(state, items);
            OutlineAction::None
        }
        OutlineMsg::Nav(d) => {
            let n = state.results.len() as i32;
            if n > 0 {
                state.selected = (state.selected as i32 + d).rem_euclid(n) as usize;
            }
            OutlineAction::None
        }
        OutlineMsg::Apply => {
            let Some(&idx) = state.results.get(state.selected) else {
                return OutlineAction::None;
            };
            let Some(it) = items.get(idx) else {
                return OutlineAction::None;
            };
            OutlineAction::GoTo { line: it.line, col: it.col }
        }
    }
}

/// Routing de teclas cuando el outline está abierto.
pub fn on_key(_state: &OutlineState, event: &KeyEvent) -> Option<OutlineMsg> {
    if event.state != KeyState::Pressed {
        return None;
    }
    Some(match &event.key {
        Key::Named(NamedKey::Escape) => OutlineMsg::Close,
        Key::Named(NamedKey::Enter) => OutlineMsg::Apply,
        Key::Named(NamedKey::ArrowDown) => OutlineMsg::Nav(1),
        Key::Named(NamedKey::ArrowUp) => OutlineMsg::Nav(-1),
        _ => OutlineMsg::KeyInput(event.clone()),
    })
}

/// El atajo recomendado: **Ctrl+Shift+O**, igual que VS Code.
pub fn open_shortcut(event: &KeyEvent) -> bool {
    event.state == KeyState::Pressed
        && event.modifiers.ctrl
        && event.modifiers.shift
        && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("o"))
}

/// Recalcula `state.results` con fuzzy match sobre `"name kind container"`.
/// Query vacío = lista completa. Cap: [`MAX_RESULTS`].
pub fn refilter(state: &mut OutlineState, items: &[SymbolItem]) {
    let q = state.input.text();
    if q.trim().is_empty() {
        state.results = (0..items.len().min(MAX_RESULTS)).collect();
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
    for (i, it) in items.iter().enumerate() {
        let hay_str = match &it.container {
            Some(c) => format!("{} {} {c}", it.name, it.kind),
            None => format!("{} {}", it.name, it.kind),
        };
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
pub struct OutlinePalette {
    pub bg_panel: Color,
    pub bg_header: Color,
    pub bg_selected: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    theme: llimphi_theme::Theme,
}

impl OutlinePalette {
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

/// Render del overlay. `to_host` mapea cada `OutlineMsg` al `Msg` de la
/// app.
pub fn view<HostMsg, F>(
    state: &OutlineState,
    items: &[SymbolItem],
    palette: &OutlinePalette,
    to_host: F,
) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
    F: Fn(OutlineMsg) -> HostMsg + Copy + 'static,
{
    let header = if items.is_empty() {
        "outline · sin símbolos · Esc cierra".to_string()
    } else if state.results.is_empty() {
        format!("outline · sin matches · {} símbolos · Esc cierra", items.len())
    } else {
        format!(
            "outline · {} / {} · ↓↑ navega · Enter salta · Esc cierra",
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
        "filtro: nombre del símbolo o clase…",
        true,
        &tp,
        to_host(OutlineMsg::Open),
    )]);

    let visible_start = state.selected.saturating_sub(MAX_VISIBLE.saturating_sub(1));
    let visible_end = (visible_start + MAX_VISIBLE).min(state.results.len());
    let mut rows: Vec<View<HostMsg>> = Vec::with_capacity(MAX_VISIBLE);
    for i in visible_start..visible_end {
        let Some(&idx) = state.results.get(i) else { continue };
        let Some(it) = items.get(idx) else { continue };
        // Indentación visual por depth (sólo cuando no hay query — con
        // query el orden ya vino del ranking y la jerarquía se pierde).
        let indent = if state.input.text().trim().is_empty() {
            "  ".repeat(it.depth as usize)
        } else {
            String::new()
        };
        let container_tag = match &it.container {
            Some(c) if !c.is_empty() => format!("    in {c}"),
            _ => String::new(),
        };
        let label = format!(
            "{indent}{} {}    line {}{container_tag}",
            it.kind,
            it.name,
            it.line + 1,
        );
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
            .text_aligned(label, 11.0, fg, Alignment::Start),
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
