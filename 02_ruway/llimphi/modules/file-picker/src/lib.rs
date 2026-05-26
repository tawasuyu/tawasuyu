//! `llimphi-module-file-picker` — fuzzy file picker reutilizable.
//!
//! Equivalente a Ctrl+P de VS Code / "Go to file" de JetBrains: el host
//! mantiene una lista de paths candidatos (típicamente walk del workspace
//! cacheado al arrancar) y el módulo presenta un overlay con input +
//! resultados rankeados. Cuando el user pica uno, el módulo emite
//! [`PickerAction::Open`] y el host decide cómo abrir (tab nuevo, split,
//! etc.).
//!
//! Sigue el contrato Llimphi de [`docs/MODULES.md`]:
//! `State + Msg + Action + apply/on_key/open_shortcut/view + Palette`.
//!
//! ## Cómo lo enchufa una app
//!
//! ```ignore
//! use llimphi_module_file_picker::{self as picker, PickerAction, PickerMsg,
//!     PickerPalette, PickerState};
//!
//! struct Model { all_files: Vec<PathBuf>, picker: Option<PickerState>, … }
//! enum Msg { Picker(PickerMsg), … }
//!
//! // update:
//! Msg::Picker(pm) => {
//!     let mut m = model;
//!     if matches!(pm, PickerMsg::Open) && m.picker.is_none() {
//!         m.picker = Some(PickerState::new(&m.all_files, &m.root));
//!         return m;
//!     }
//!     let action = match m.picker.as_mut() {
//!         Some(s) => picker::apply(s, pm, &m.all_files, &m.root),
//!         None => return m,
//!     };
//!     match action {
//!         PickerAction::Close => m.picker = None,
//!         PickerAction::Open(path) => {
//!             m.picker = None;
//!             m = open_path_in_app(m, path);
//!         }
//!         PickerAction::None => {}
//!     }
//!     m
//! }
//!
//! // on_key:
//! if let Some(state) = model.picker.as_ref() {
//!     if let Some(pm) = picker::on_key(state, event) {
//!         return Some(Msg::Picker(pm));
//!     }
//! }
//! if picker::open_shortcut(event) {
//!     return Some(Msg::Picker(PickerMsg::Open));
//! }
//!
//! // view:
//! if let Some(state) = model.picker.as_ref() {
//!     let panel = picker::view(
//!         state, &model.all_files, &model.root,
//!         &PickerPalette::from_theme(&theme),
//!         Msg::Picker,
//!     );
//!     children.push(panel);
//! }
//! ```

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

/// Capabilities que este módulo aporta al host. El host (cuando construye
/// su `card_core::Card`) puede agregar esto a `provides` para anunciar
/// vía broker que ofrece file-picker al ecosistema.
pub const CAPABILITIES: &[&str] = &["editor.file-picker"];

/// Máximo de resultados rankeados que entran al popup.
pub const MAX_RESULTS: usize = 200;

const BAR_H: f32 = 220.0;
const ROW_H: f32 = 20.0;
const MAX_VISIBLE: usize = 9;

/// Estado interno. Los `results` son índices al slice de paths que pasa
/// el host: el módulo no copia paths, sólo guarda índices.
pub struct PickerState {
    pub input: TextInputState,
    pub results: Vec<usize>,
    pub selected: usize,
}

impl Default for PickerState {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl PickerState {
    /// Crea un picker vacío. Si querés pre-filtrar con los paths que ya
    /// tenés, llamá [`PickerState::new`] en su lugar.
    pub fn new_empty() -> Self {
        Self {
            input: TextInputState::new(),
            results: Vec::new(),
            selected: 0,
        }
    }

    /// Crea un picker con todos los `paths` como resultados iniciales
    /// (sin filtrar). Conveniente para el ack visual del Ctrl+P recién
    /// disparado.
    pub fn new(paths: &[PathBuf], root: &Path) -> Self {
        let mut s = Self::new_empty();
        refilter(&mut s, paths, root);
        s
    }
}

/// Vocabulario interno. El host lo wrapea en su Msg.
#[derive(Clone)]
pub enum PickerMsg {
    /// Símbolo conveniente para que el host dispatche al detectar el
    /// shortcut. El módulo no maneja Open él mismo — la creación del
    /// state corre por cuenta del host (porque típicamente quiere pasar
    /// la lista canónica de paths).
    Open,
    Close,
    KeyInput(KeyEvent),
    Nav(i32),
    /// Enter: abre el match seleccionado.
    Apply,
}

/// Efecto solicitado al host.
#[derive(Debug, Clone)]
pub enum PickerAction {
    None,
    /// El host debería remover el state del modelo.
    Close,
    /// El host debería abrir este `path`. El módulo NO se cierra
    /// automáticamente — el host decide si ocultar el picker tras abrir.
    Open(PathBuf),
}

/// Aplica un mensaje al estado.
pub fn apply(
    state: &mut PickerState,
    msg: PickerMsg,
    paths: &[PathBuf],
    root: &Path,
) -> PickerAction {
    match msg {
        PickerMsg::Open => PickerAction::None,
        PickerMsg::Close => PickerAction::Close,
        PickerMsg::KeyInput(ev) => {
            state.input.apply_key(&ev);
            refilter(state, paths, root);
            PickerAction::None
        }
        PickerMsg::Nav(d) => {
            let n = state.results.len() as i32;
            if n > 0 {
                state.selected = (state.selected as i32 + d).rem_euclid(n) as usize;
            }
            PickerAction::None
        }
        PickerMsg::Apply => {
            let Some(&file_idx) = state.results.get(state.selected) else {
                return PickerAction::None;
            };
            let Some(path) = paths.get(file_idx).cloned() else {
                return PickerAction::None;
            };
            PickerAction::Open(path)
        }
    }
}

/// Routing de teclas cuando el panel está abierto.
pub fn on_key(_state: &PickerState, event: &KeyEvent) -> Option<PickerMsg> {
    if event.state != KeyState::Pressed {
        return None;
    }
    Some(match &event.key {
        Key::Named(NamedKey::Escape) => PickerMsg::Close,
        Key::Named(NamedKey::Enter) => PickerMsg::Apply,
        Key::Named(NamedKey::ArrowDown) => PickerMsg::Nav(1),
        Key::Named(NamedKey::ArrowUp) => PickerMsg::Nav(-1),
        _ => PickerMsg::KeyInput(event.clone()),
    })
}

/// Chequea si el evento es el atajo recomendado: **Ctrl+P**.
pub fn open_shortcut(event: &KeyEvent) -> bool {
    event.state == KeyState::Pressed
        && event.modifiers.ctrl
        && !event.modifiers.shift
        && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("p"))
}

/// Recalcula `state.results` según el query del input. Match case-insensitive
/// sobre el path relativo. Score penaliza paths largos y premia hits en el
/// basename. Query vacío = todos los paths ordenados por longitud asc.
/// Cap: [`MAX_RESULTS`].
pub fn refilter(state: &mut PickerState, paths: &[PathBuf], root: &Path) {
    let q = state.input.text();
    let q_lc = q.to_lowercase();
    let mut scored: Vec<(i64, usize)> = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let rel = relative_to(root, path);
        if q_lc.is_empty() {
            scored.push((rel.len() as i64, i));
            continue;
        }
        let rel_lc = rel.to_lowercase();
        let Some(rel_hit) = rel_lc.find(&q_lc) else { continue };
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        let name_hit = name.find(&q_lc);
        let score = match name_hit {
            Some(pos) => pos as i64 * 4 + rel.len() as i64,
            None => 10_000 + rel_hit as i64 + rel.len() as i64,
        };
        scored.push((score, i));
    }
    scored.sort_by_key(|(s, _)| *s);
    scored.truncate(MAX_RESULTS);
    state.results = scored.into_iter().map(|(_, i)| i).collect();
    state.selected = 0;
}

/// Paleta visual.
#[derive(Debug, Clone)]
pub struct PickerPalette {
    pub bg_panel: Color,
    pub bg_header: Color,
    pub bg_selected: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    theme: llimphi_theme::Theme,
}

impl PickerPalette {
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

/// Render del panel. `to_host` mapea cada `PickerMsg` interno al `Msg`
/// de la app.
pub fn view<HostMsg, F>(
    state: &PickerState,
    paths: &[PathBuf],
    root: &Path,
    palette: &PickerPalette,
    to_host: F,
) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
    F: Fn(PickerMsg) -> HostMsg + Copy + 'static,
{
    let header = if state.results.is_empty() {
        format!("file picker · sin matches · {} archivos · Esc cierra", paths.len())
    } else {
        format!(
            "file picker · {} / {} · ↓↑ navega · Enter abre · Esc cierra",
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
        "filtro: nombre o ruta…",
        true,
        &tp,
        to_host(PickerMsg::Open),
    )]);

    let visible_start = state.selected.saturating_sub(MAX_VISIBLE.saturating_sub(1));
    let visible_end = (visible_start + MAX_VISIBLE).min(state.results.len());
    let mut rows: Vec<View<HostMsg>> = Vec::with_capacity(MAX_VISIBLE);
    for i in visible_start..visible_end {
        let Some(&file_idx) = state.results.get(i) else { continue };
        let Some(path) = paths.get(file_idx) else { continue };
        let rel = relative_to(root, path);
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
        let dir = rel.strip_suffix(name).unwrap_or("");
        let label = if dir.is_empty() {
            name.to_string()
        } else {
            format!("{name}    {}", dir.trim_end_matches('/'))
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

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}
