//! `llimphi-module-mini-map` — minimap del buffer activo.
//!
//! Equivalente al "Minimap" de VS Code / "thumbnail" de Sublime: un
//! panel angosto pegado al editor que pinta una linea horizontal por
//! cada linea del buffer (ancho ~= len_chars, cap a `usable_w`),
//! resalta el viewport visible como rect translucido y marca el caret.
//! Click sobre el minimap salta esa linea al editor.
//!
//! El modulo es agnostico del editor: el host pasa un slice con la
//! cantidad de chars por linea, el rango visible y la linea del
//! caret. No depende de `llimphi-widget-text-editor` — cualquier
//! buffer (rope, vec<String>, archivo memmaped) sirve.
//!
//! Sigue el contrato Llimphi de `docs/MODULES.md`:
//! `State + Msg + Action + apply/on_key/open_shortcut/view + Palette`.

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KRect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::{Key, KeyEvent, KeyState, View};

/// Capabilities que aporta este modulo al host.
pub const CAPABILITIES: &[&str] = &["editor.mini-map"];

/// Ancho del panel en pixeles (estilo VS Code).
pub const PANEL_W: f32 = 120.0;
/// Altura maxima por linea del buffer dentro del minimap (cap).
pub const LINE_PX: f32 = 2.0;
/// Escala chars->pixels para el ancho de cada slab. ~75 chars caben
/// completos en `PANEL_W - PAD * 2`; lo demas se trunca.
pub const CHAR_PX: f32 = 1.4;
/// Padding lateral del panel (los slabs no tocan los bordes).
pub const PAD: f32 = 6.0;

/// Estado interno. Hoy efectivamente vacio — la informacion del buffer
/// la pasa el host en cada frame via [`view`] — pero existe como
/// `Option<MiniMapState>` en el host para representar abierto/cerrado
/// y para futuras extensiones (scrubbing, fold-aware, syntax per slab).
#[derive(Debug, Default, Clone)]
pub struct MiniMapState {
    /// Reservado para drag-scrub: la y inicial en pixeles dentro del
    /// panel cuando el usuario empieza a arrastrar. `None` = sin drag
    /// activo. Hoy no se consume (click es suficiente); declarado para
    /// que el contrato del state no cambie cuando se agregue.
    pub drag_anchor_y: Option<f32>,
}

impl MiniMapState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Vocabulario interno. El host lo wrapea en su Msg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MiniMapMsg {
    /// Convencional: el host abre el panel guardando un `MiniMapState`
    /// en el modelo. El modulo no construye state global.
    Open,
    Close,
    /// El usuario clickeo o arrastro: salta a la linea indicada.
    Jump(usize),
}

/// Efecto solicitado al host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MiniMapAction {
    None,
    /// El host deberia remover el state del modelo.
    Close,
    /// El host deberia centrar el viewport en esta linea del buffer
    /// activo. El modulo NO se cierra — el minimap es persistente.
    JumpTo(usize),
}

/// Snapshot del buffer que el host pasa en cada frame. El modulo no
/// copia, solo lee. La cantidad de chars por linea es lo unico que
/// necesita para dibujar; viewport + caret se overlayean encima.
pub struct Snapshot<'a> {
    /// `lines[i]` = numero de chars (no bytes) en la linea `i`.
    pub lines: &'a [usize],
    /// Rango visible en el editor: `[start, end)`.
    pub viewport_start: usize,
    pub viewport_end: usize,
    /// Linea del caret (0-based). Se pinta como marker accent.
    pub caret_line: usize,
}

/// Aplica un mensaje al estado.
pub fn apply(state: &mut MiniMapState, msg: MiniMapMsg) -> MiniMapAction {
    match msg {
        MiniMapMsg::Open => MiniMapAction::None,
        MiniMapMsg::Close => MiniMapAction::Close,
        MiniMapMsg::Jump(line) => {
            state.drag_anchor_y = None;
            MiniMapAction::JumpTo(line)
        }
    }
}

/// Routing de teclas. El minimap NO captura teclas (es un viewer
/// pasivo). Devolvemos `None`; el host sigue su routing normal.
pub fn on_key(_state: &MiniMapState, _event: &KeyEvent) -> Option<MiniMapMsg> {
    None
}

/// Atajo recomendado: **Ctrl+Shift+M** (mnemonic M = Minimap).
pub fn open_shortcut(event: &KeyEvent) -> bool {
    event.state == KeyState::Pressed
        && event.modifiers.ctrl
        && event.modifiers.shift
        && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("m"))
}

/// Convierte una posicion-y dentro del panel a indice de linea. La
/// conversion es proporcional al total de lineas; clamping en ambos
/// extremos.
pub fn y_to_line(y: f32, panel_h: f32, total_lines: usize) -> usize {
    if total_lines == 0 || panel_h <= 0.0 {
        return 0;
    }
    let t = (y / panel_h).clamp(0.0, 1.0);
    let line = (t * total_lines as f32) as usize;
    line.min(total_lines.saturating_sub(1))
}

/// Paleta visual derivable del theme.
#[derive(Debug, Clone)]
pub struct MiniMapPalette {
    /// Fondo del panel del minimap.
    pub bg_panel: Color,
    /// Color de los slabs (uno por linea de buffer).
    pub fg_slab: Color,
    /// Color del rect translucido que marca el viewport visible.
    pub bg_viewport: Color,
    /// Borde del rect del viewport.
    pub border_viewport: Color,
    /// Color del marker del caret.
    pub fg_caret: Color,
}

impl MiniMapPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_panel: t.bg_panel_alt,
            fg_slab: t.fg_muted,
            bg_viewport: with_alpha(t.bg_selected, 0.35),
            border_viewport: t.border_focus,
            fg_caret: t.accent,
        }
    }
}

fn with_alpha(c: Color, alpha: f32) -> Color {
    let rgba = c.to_rgba8();
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, a)
}

/// Render del panel. `to_host` mapea cada `MiniMapMsg` al `Msg` de la app.
/// `snapshot` es la vista del buffer en este frame (sin copia).
///
/// Layout: columna fija de `PANEL_W` px que ocupa todo el alto del
/// contenedor padre. El host la mete en el `Row` del editor
/// (tipicamente al final, al estilo VS Code).
pub fn view<HostMsg, F>(
    _state: &MiniMapState,
    snapshot: &Snapshot,
    palette: &MiniMapPalette,
    to_host: F,
) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
    F: Fn(MiniMapMsg) -> HostMsg + Copy + Send + Sync + 'static,
{
    // Capturamos por valor porque el painter es Arc<dyn Fn>: 'static + Send + Sync.
    let lines: Vec<usize> = snapshot.lines.to_vec();
    let viewport_start = snapshot.viewport_start;
    let viewport_end = snapshot.viewport_end;
    let caret_line = snapshot.caret_line;
    let pal = palette.clone();

    let total_lines = lines.len();
    let click_host = to_host;
    let on_click: Arc<dyn Fn(f32, f32, f32, f32) -> Option<HostMsg> + Send + Sync> = Arc::new(move |_x: f32, y: f32, _w: f32, h: f32| {
        let line = y_to_line(y, h, total_lines);
        Some(click_host(MiniMapMsg::Jump(line)))
    });

    let mut view = View::new(Style {
        size: Size { width: length(PANEL_W), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(pal.bg_panel)
    .clip(true)
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 0.0 || rect.h <= 0.0 || lines.is_empty() {
            return;
        }
        let n = lines.len() as f32;
        let line_h = (rect.h / n).min(LINE_PX);
        let usable_w = (rect.w - PAD * 2.0).max(1.0);

        // 1) Viewport overlay debajo de los slabs.
        if viewport_end > viewport_start {
            let y0 = rect.y + (viewport_start as f32 / n) * rect.h;
            let y1 = rect.y + (viewport_end as f32 / n) * rect.h;
            let vp = KRect::new(
                rect.x as f64,
                y0 as f64,
                (rect.x + rect.w) as f64,
                y1.max(y0 + 2.0) as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, pal.bg_viewport, None, &vp);
        }

        // 2) Slabs: uno por linea de buffer.
        for (i, &chars) in lines.iter().enumerate() {
            if chars == 0 {
                continue;
            }
            let w = (chars as f32 * CHAR_PX).min(usable_w);
            let y = rect.y + (i as f32 / n) * rect.h;
            let slab_h = line_h.max(1.0);
            let r = KRect::new(
                (rect.x + PAD) as f64,
                y as f64,
                (rect.x + PAD + w) as f64,
                (y + slab_h) as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, pal.fg_slab, None, &r);
        }

        // 3) Borde del viewport encima de los slabs.
        if viewport_end > viewport_start {
            let y0 = rect.y + (viewport_start as f32 / n) * rect.h;
            let y1 = (rect.y + (viewport_end as f32 / n) * rect.h).max(y0 + 2.0);
            let top = KRect::new(
                rect.x as f64,
                y0 as f64,
                (rect.x + rect.w) as f64,
                (y0 + 1.0) as f64,
            );
            let bot = KRect::new(
                rect.x as f64,
                (y1 - 1.0) as f64,
                (rect.x + rect.w) as f64,
                y1 as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, pal.border_viewport, None, &top);
            scene.fill(Fill::NonZero, Affine::IDENTITY, pal.border_viewport, None, &bot);
        }

        // 4) Marker del caret: barra horizontal accent.
        if caret_line < lines.len() {
            let y = rect.y + (caret_line as f32 / n) * rect.h;
            let r = KRect::new(
                rect.x as f64,
                y as f64,
                (rect.x + rect.w) as f64,
                (y + 2.0) as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, pal.fg_caret, None, &r);
        }
    });
    view.on_click_at = Some(on_click);
    view
}
