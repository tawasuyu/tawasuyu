//! `cosmobiologia-canvas` — el widget GPUI del lienzo astrológico.
//!
//! Modela el cielo como un lienzo de **geometría reactiva**: un estado
//! unificado [`CanvasState`] guarda offsets de rotación, flags de
//! visibilidad y la lista de `Layer`s a pintar. Las interacciones
//! (drag, hotkeys, toggles) mutan el estado; el render lee la última
//! `RenderModel` y la deriva al frame.
//!
//! ## Convención de rotación
//!
//! El Ascendente cae a las 9 del reloj (lado izquierdo). Las casas
//! crecen contrarreloj visualmente. Para una longitud eclíptica `L` y
//! un ascendente `asc`:
//!
//! ```text
//!   screen_angle_rad = π - (L - asc + view_rotation) · π/180
//!   point = (cx + r·cos(θ), cy + r·sin(θ))
//! ```
//!
//! ## Interacciones (fase 4)
//!
//! - **Drag en el aro exterior** (jog-dial perimetral): rota la rueda
//!   visualmente mientras dura el drag y, al soltar, traduce el delta
//!   angular a minutos (1° ≈ 4 min) y emite
//!   [`CanvasEvent::TimeOffsetChanged`]. El host (la app) recomputa la
//!   carta para el instante desplazado.
//! - **Hotkeys**: `D`/`H`/`X`/`P` togglean SignDial/Houses/Aspects/
//!   Bodies. Click sobre el wheel le da focus al widget.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::collections::HashMap;
use std::f32::consts::PI;

use gpui::{
    Bounds, Context, EventEmitter, FocusHandle, Focusable, Hsla, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PathBuilder, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, SharedString, Styled,
    Window, canvas, div, hsla, point, prelude::*, px,
};

use cosmobiologia_engine::{
    Geometry, GrTrigger, Layer, LayerKind, Rectificacion, RenderModel, UranianGroup,
    OUTER_RING_MODULES,
};
use cosmobiologia_model::{ChartId, ContactId, GroupId};
use cosmobiologia_theme::{AspectKind as TAspectKind, AstroPalette, Element, Planet};
use nahual_theme::Theme;

// =====================================================================
// Eventos
// =====================================================================

#[derive(Clone, Debug)]
pub enum CanvasEvent {
    /// Doble click sobre un thumbnail.
    ChartRequested(ChartId),
    /// Drag terminado: el offset acumulado de tiempo (en minutos)
    /// cambió. El host debe recomputar el chart con este offset.
    TimeOffsetChanged(i64),
    /// El usuario togggleó una capa via hotkey — el panel debería
    /// reflejarlo si quisiera mantenerse en sync.
    LayerVisibilityChanged { kind: LayerKind, visible: bool },
    /// El usuario togggleó los coord labels via hotkey C. El panel
    /// debe sincronizar el toggle "show_coords" del NatalModule.
    ShowCoordsChanged(bool),
    /// El usuario pidió exportar el render actual como SVG. El shell
    /// se encarga de escribir el archivo (la engine genera el string).
    ExportSvgRequested,
    /// En modo GR (direcciones primarias activas) el jog-dial scrubea
    /// la edad en vez del tiempo. Lleva el delta de edad en años; el
    /// host lo acumula sobre `target_age_years` y recompone en vivo.
    GrAgeDelta(f64),
    /// El usuario hizo clic en una barra del espectro armónico. Lleva
    /// el orden de armónica elegido; el host fija el slider `harmonic`
    /// del módulo natal y recompone.
    HarmonicSelected(u32),
}

// =====================================================================
// Estado
// =====================================================================

#[derive(Clone, Debug, Default)]
pub enum CanvasMode {
    #[default]
    Empty,
    Wheel { render: Box<RenderModel> },
    Thumbnails {
        scope: ThumbnailScope,
        items: Vec<ThumbnailItem>,
    },
}

#[derive(Clone, Debug)]
pub enum ThumbnailScope {
    Group(GroupId),
    Contact(ContactId),
}

#[derive(Clone, Debug)]
pub struct ThumbnailItem {
    pub chart_id: ChartId,
    pub label: SharedString,
    pub subtitle: Option<SharedString>,
    pub preview: Option<RenderModel>,
}

/// Estado de un drag activo del jog-dial. `last_screen_angle_deg` se
/// actualiza en cada `MouseMoveEvent`; `accumulated_delta_deg` lleva la
/// rotación total desde que arrancó el drag (puede pasar de ±360°).
#[derive(Clone, Debug)]
struct JogDragState {
    last_screen_angle_deg: f32,
    accumulated_delta_deg: f32,
}

/// Drag activo de pan (MMB o LMB con Space). Captura el pan inicial al
/// hacer mousedown; el move agrega delta_pos a esos valores.
#[derive(Clone, Debug)]
struct PanDragState {
    start_pos: Point<Pixels>,
    pan_x_start: f32,
    pan_y_start: f32,
}

#[derive(Clone, Debug)]
pub struct CanvasState {
    pub mode: CanvasMode,
    /// Rotación visual transitoria durante un drag. Se resetea a `0` al
    /// soltar — el render nuevo trae el `ascendant_deg` ya rotado.
    pub view_rotation_deg: f32,
    /// Offset acumulado en minutos. Persiste entre drags hasta que el
    /// host lo resetee.
    pub time_offset_minutes: i64,
    /// Factor de zoom multiplicativo aplicado al wheel. `1.0` = tamaño
    /// nominal. Clampeado a [VIEW_SCALE_MIN, VIEW_SCALE_MAX].
    pub view_scale: f32,
    /// Pan horizontal en px (positivo = desplaza el wheel a la derecha
    /// desde el centro). Se aplica como margin shift sobre el centrado
    /// natural del flex parent.
    pub view_pan_x: f32,
    /// Pan vertical en px (positivo = abajo).
    pub view_pan_y: f32,
    /// Por-LayerKind: `true` = visible. Default = todo visible.
    pub layer_visibility: HashMap<LayerKind, bool>,
    /// Indicadores de grado al lado de cada planeta y cusp de casa.
    /// Default `true` — el usuario los espera ver para leer la
    /// carta. Se togglean con `C` (Coords) o desde el panel.
    pub show_coords: bool,
    /// Planeta hovered actualmente (para tooltip). `None` cuando el
    /// mouse no está sobre ningún cuerpo.
    pub hover: Option<HoverInfo>,
    /// Último resultado del rectificador automático, si se corrió uno.
    /// El canvas dibuja su perfil como una curva en el footer; el valle
    /// marca la hora de nacimiento que mejor explica los eventos.
    pub rectificacion: Option<Rectificacion>,
    drag_jog: Option<JogDragState>,
    drag_pan: Option<PanDragState>,
}

/// Límites del zoom — bajo 0.5 los glyphs se vuelven ilegibles; sobre
/// 3.0 el wheel desborda incluso pantallas grandes.
pub const VIEW_SCALE_MIN: f32 = 0.5;
pub const VIEW_SCALE_MAX: f32 = 3.0;

/// Info del elemento bajo el cursor — usado por el render para mostrar
/// un tooltip flotante con detalles. Cubre body glyphs, cusps de casa,
/// y líneas de aspectos.
#[derive(Clone, Debug)]
pub enum HoverInfo {
    Body {
        module_id: String,
        symbol: String,
        deg: f32,
        house: Option<u8>,
        retrograde: bool,
        dignity_marker: Option<String>,
        annotation: Option<String>,
        local_x: f32,
        local_y: f32,
    },
    HouseCusp {
        house_number: u8,
        deg: f32,
        local_x: f32,
        local_y: f32,
    },
    /// Hover sobre una línea de aspecto. `from_body`/`to_body` y `kind`
    /// vienen de la LineSeg; `orb_deg` también. Los coords son el
    /// punto medio del segmento donde se muestra el tooltip.
    Aspect {
        module_id: String,
        from_body: String,
        to_body: String,
        kind: String,
        orb_deg: f32,
        local_x: f32,
        local_y: f32,
    },
}

impl HoverInfo {
    fn local(&self) -> (f32, f32) {
        match self {
            HoverInfo::Body { local_x, local_y, .. } => (*local_x, *local_y),
            HoverInfo::HouseCusp { local_x, local_y, .. } => (*local_x, *local_y),
            HoverInfo::Aspect { local_x, local_y, .. } => (*local_x, *local_y),
        }
    }

    fn key(&self) -> String {
        match self {
            HoverInfo::Body {
                module_id, symbol, ..
            } => format!("body:{}:{}", module_id, symbol),
            HoverInfo::HouseCusp { house_number, .. } => format!("cusp:{}", house_number),
            HoverInfo::Aspect {
                module_id,
                from_body,
                to_body,
                kind,
                ..
            } => format!("aspect:{}:{}-{}-{}", module_id, from_body, kind, to_body),
        }
    }
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            mode: CanvasMode::default(),
            view_rotation_deg: 0.0,
            time_offset_minutes: 0,
            view_scale: 1.0,
            view_pan_x: 0.0,
            view_pan_y: 0.0,
            layer_visibility: HashMap::new(),
            show_coords: true,
            hover: None,
            rectificacion: None,
            drag_jog: None,
            drag_pan: None,
        }
    }
}

impl CanvasState {
    pub fn is_layer_visible(&self, kind: LayerKind) -> bool {
        self.layer_visibility.get(&kind).copied().unwrap_or(true)
    }

    /// `true` cuando hay un overlay de direcciones primarias activo.
    /// En ese modo el jog-dial scrubea la edad GR en vez del tiempo.
    fn gr_active(&self) -> bool {
        matches!(
            &self.mode,
            CanvasMode::Wheel { render }
                if render.layers.iter().any(|l| l.module_id == "pd_direct")
        )
    }
}

/// Sensibilidad del scrubbing GR: años de edad por grado de jog. A
/// 0.1, una vuelta completa del dial barre 36 años — fino para
/// explorar contactos sin perder rango.
const GR_AGE_PER_DEG: f32 = 0.1;

// =====================================================================
// Widget
// =====================================================================

pub struct AstrologyCanvas {
    state: CanvasState,
    focus_handle: FocusHandle,
}

impl EventEmitter<CanvasEvent> for AstrologyCanvas {}

impl Focusable for AstrologyCanvas {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl AstrologyCanvas {
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        Self {
            state: CanvasState::default(),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn state(&self) -> &CanvasState {
        &self.state
    }

    pub fn set_mode(&mut self, mode: CanvasMode, cx: &mut Context<'_, Self>) {
        self.state.mode = mode;
        cx.notify();
    }

    pub fn set_layer_visible(&mut self, kind: LayerKind, visible: bool, cx: &mut Context<'_, Self>) {
        self.state.layer_visibility.insert(kind, visible);
        cx.notify();
    }

    pub fn toggle_layer(&mut self, kind: LayerKind, cx: &mut Context<'_, Self>) {
        let current = self.state.is_layer_visible(kind);
        self.set_layer_visible(kind, !current, cx);
        cx.emit(CanvasEvent::LayerVisibilityChanged {
            kind,
            visible: !current,
        });
    }

    pub fn reset_time_offset(&mut self, cx: &mut Context<'_, Self>) {
        if self.state.time_offset_minutes != 0 || self.state.view_rotation_deg != 0.0 {
            self.state.time_offset_minutes = 0;
            self.state.view_rotation_deg = 0.0;
            cx.emit(CanvasEvent::TimeOffsetChanged(0));
            cx.notify();
        }
    }

    pub fn set_view_rotation(&mut self, deg: f32, cx: &mut Context<'_, Self>) {
        self.state.view_rotation_deg = deg.rem_euclid(360.0);
        cx.notify();
    }

    pub fn toggle_coords(&mut self, cx: &mut Context<'_, Self>) {
        let new_val = !self.state.show_coords;
        self.set_show_coords(new_val, cx);
        cx.emit(CanvasEvent::ShowCoordsChanged(new_val));
    }

    /// Setter idempotente — el shell lo usa para reflejar cambios del
    /// panel sin disparar el `ShowCoordsChanged` (que iría en el otro
    /// sentido y crearía un loop).
    pub fn set_show_coords(&mut self, value: bool, cx: &mut Context<'_, Self>) {
        if self.state.show_coords != value {
            self.state.show_coords = value;
            cx.notify();
        }
    }

    /// Publica el resultado de un barrido de rectificación: el canvas
    /// dibuja su perfil como una curva en el footer. `None` lo borra.
    pub fn set_rectificacion(
        &mut self,
        rectificacion: Option<Rectificacion>,
        cx: &mut Context<'_, Self>,
    ) {
        self.state.rectificacion = rectificacion;
        cx.notify();
    }

    /// Resetea zoom y pan a sus defaults (1.0 y 0,0). No toca rotation
    /// ni time offset — esos son ortogonales y tienen su propio reset.
    pub fn reset_view(&mut self, cx: &mut Context<'_, Self>) {
        if self.state.view_scale != 1.0
            || self.state.view_pan_x != 0.0
            || self.state.view_pan_y != 0.0
        {
            self.state.view_scale = 1.0;
            self.state.view_pan_x = 0.0;
            self.state.view_pan_y = 0.0;
            cx.notify();
        }
    }

    /// Zoom multiplicativo. El nuevo scale es `current * factor`, clamp
    /// al rango permitido. El zoom es centrado (no rastrea el cursor) —
    /// para mover el foco después del zoom, el usuario paneja con MMB.
    fn zoom_by(&mut self, factor: f32, cx: &mut Context<'_, Self>) {
        let new_scale =
            (self.state.view_scale * factor).clamp(VIEW_SCALE_MIN, VIEW_SCALE_MAX);
        if (new_scale - self.state.view_scale).abs() < 1e-4 {
            return;
        }
        // Mantener el centro del wheel anclado al centro de pantalla:
        // como el pan está en coords de la pantalla y el zoom es desde
        // el centro del wheel, el pan se escala proporcional al ratio.
        let ratio = new_scale / self.state.view_scale;
        self.state.view_pan_x *= ratio;
        self.state.view_pan_y *= ratio;
        self.state.view_scale = new_scale;
        cx.notify();
    }

    #[allow(dead_code)]
    fn pan_by(&mut self, dx: f32, dy: f32, cx: &mut Context<'_, Self>) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        self.state.view_pan_x += dx;
        self.state.view_pan_y += dy;
        cx.notify();
    }

    // ----- Internos: handlers de jog-dial -----

    /// Despacha el LMB down entre jog-dial y pan. El jog-dial es un
    /// control "fuerte" (mueve el tiempo de la carta), así que se
    /// activa SOLO con modifier Ctrl/Cmd + click sobre el anillo de
    /// signos — sin modifier es siempre pan, incluso sobre el anillo,
    /// para que no haya rotaciones accidentales al manipular la
    /// rueda.
    fn on_primary_down(
        &mut self,
        position: Point<Pixels>,
        modifiers: gpui::Modifiers,
        bounds: Bounds<Pixels>,
        cx: &mut Context<'_, Self>,
    ) {
        // Sin modifier: pan, sin importar dónde caiga el click.
        if !(modifiers.control || modifiers.platform) {
            self.on_pan_down(position, cx);
            return;
        }
        let (cx_px, cy_px) = bounds_center(bounds);
        let mx: f32 = position.x.into();
        let my: f32 = position.y.into();
        let dx = mx - cx_px;
        let dy = my - cy_px;
        let dist = (dx * dx + dy * dy).sqrt();
        let r_outer = effective_r_outer(bounds);
        let radii = Radii::from_outer(r_outer);
        let on_dial = dist >= radii.sign_inner * 0.95 && dist <= radii.sign_outer * 1.10;
        if on_dial {
            let angle = dy.atan2(dx).to_degrees();
            self.state.drag_jog = Some(JogDragState {
                last_screen_angle_deg: angle,
                accumulated_delta_deg: 0.0,
            });
        } else {
            // Ctrl+click fuera del anillo: pan también — el modifier
            // habilita el jog-dial pero no impide la navegación.
            self.on_pan_down(position, cx);
        }
    }

    fn on_jog_move(
        &mut self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        cx: &mut Context<'_, Self>,
    ) {
        let gr = self.state.gr_active();
        let Some(jog) = self.state.drag_jog.as_mut() else {
            return;
        };
        let (cx_px, cy_px) = bounds_center(bounds);
        let mx: f32 = position.x.into();
        let my: f32 = position.y.into();
        let dx = mx - cx_px;
        let dy = my - cy_px;
        let angle = dy.atan2(dx).to_degrees();
        let mut delta = angle - jog.last_screen_angle_deg;
        // Normalizar a (-180, 180] para cruzar el wrap sin saltar.
        if delta > 180.0 {
            delta -= 360.0;
        } else if delta < -180.0 {
            delta += 360.0;
        }
        jog.accumulated_delta_deg += delta;
        jog.last_screen_angle_deg = angle;
        let accumulated = jog.accumulated_delta_deg;
        if gr {
            // Modo GR: el jog scrubea la edad. No rota el wheel — el
            // feedback es el movimiento de los glifos dirigidos cuando
            // el shell recompone con la edad nueva.
            cx.emit(CanvasEvent::GrAgeDelta((-delta * GR_AGE_PER_DEG) as f64));
        } else {
            // Reflejo visual durante el drag (sin recomputar).
            self.state.view_rotation_deg = accumulated;
            cx.notify();
        }
    }

    /// Hit-test sobre body glyphs + house cusps. Para bodies: distancia
    /// al centro del glyph dentro de threshold. Para cusps: el mouse
    /// debe estar cerca del ring de casas Y angularmente cerca del
    /// cusp (proximidad a la línea radial).
    fn on_hover_check(
        &mut self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        cx: &mut Context<'_, Self>,
    ) {
        let CanvasMode::Wheel { render } = &self.state.mode else {
            if self.state.hover.take().is_some() {
                cx.notify();
            }
            return;
        };
        let (cx_px, cy_px) = bounds_center(bounds);
        let mx: f32 = position.x.into();
        let my: f32 = position.y.into();
        let ox: f32 = bounds.origin.x.into();
        let oy: f32 = bounds.origin.y.into();
        let r_outer = effective_r_outer(bounds);
        let radii = Radii::from_outer(r_outer);
        let asc = render.ascendant_deg;
        let rot = self.state.view_rotation_deg;
        let body_threshold = 14.0_f32;

        let mut best: Option<(f32, HoverInfo)> = None;

        // 1) Body glyphs (incluye natal, overlays, midpoints).
        //
        // Importante: el hit-test debe usar `display_deg` (post-spread)
        // y no `g.deg` (raw) — el spread mueve los discos para evitar
        // solapes y si el hover sigue al raw, el usuario tendría que
        // apuntar a una zona vacía para activarlo. Calculamos los
        // displays con la misma función que render_wheel.
        let view_scale = self.state.view_scale;
        for layer in &render.layers {
            let ring = match layer.kind {
                LayerKind::Bodies => radii.body_ring(&layer.module_id),
                LayerKind::Midpoints => radii.midpoints,
                LayerKind::Outer if OUTER_RING_MODULES.contains(&layer.module_id.as_str()) => {
                    radii.transits
                }
                _ => continue,
            };
            let disk_base = body_disk_base(&layer.module_id, layer.kind, view_scale);
            let raw_degs: Vec<f32> = layer.glyphs.iter().map(|g| g.deg).collect();
            let disk_angular = (disk_base / (std::f32::consts::TAU * ring)) * 360.0;
            let (display_degs, _) =
                spread_angles(&raw_degs, disk_angular, disk_angular);
            for (i, g) in layer.glyphs.iter().enumerate() {
                let (gx, gy) = polar_to_screen(display_degs[i], asc, rot, ring);
                let dx = mx - (cx_px + gx);
                let dy = my - (cy_px + gy);
                let dist = (dx * dx + dy * dy).sqrt();
                if dist > body_threshold {
                    continue;
                }
                if best.as_ref().map(|(d, _)| dist < *d).unwrap_or(true) {
                    best = Some((
                        dist,
                        HoverInfo::Body {
                            module_id: layer.module_id.clone(),
                            symbol: g.symbol.clone(),
                            deg: g.deg,
                            house: g.house,
                            retrograde: g.retrograde,
                            dignity_marker: g.dignity_marker.clone(),
                            annotation: g.annotation.clone(),
                            local_x: cx_px + gx - ox,
                            local_y: cy_px + gy - oy,
                        },
                    ));
                }
            }
        }

        // 2) Aspect lines (segundo: las líneas son más "frágiles" que
        // los planetas; si un body matcheó arriba ya tomó precedencia).
        // Computa distancia punto-segmento del mouse al line.
        if best.is_none() {
            for layer in &render.layers {
                if !matches!(layer.kind, LayerKind::Aspects) {
                    continue;
                }
                let (r_from, r_to) = radii.aspect_endpoints(&layer.module_id);
                if let Geometry::Lines(segs) = &layer.geometry {
                    for seg in segs {
                        if seg.from_body.is_empty() || seg.to_body.is_empty() {
                            continue;
                        }
                        let (ax, ay) = polar_to_screen(seg.from_deg, asc, rot, r_from);
                        let (bx, by) = polar_to_screen(seg.to_deg, asc, rot, r_to);
                        let px_a = cx_px + ax;
                        let py_a = cy_px + ay;
                        let px_b = cx_px + bx;
                        let py_b = cy_px + by;
                        let dist = dist_point_segment(mx, my, px_a, py_a, px_b, py_b);
                        if dist > 4.0 {
                            continue;
                        }
                        if best.as_ref().map(|(d, _)| dist < *d).unwrap_or(true) {
                            let mid_x = (px_a + px_b) / 2.0;
                            let mid_y = (py_a + py_b) / 2.0;
                            best = Some((
                                dist,
                                HoverInfo::Aspect {
                                    module_id: layer.module_id.clone(),
                                    from_body: seg.from_body.clone(),
                                    to_body: seg.to_body.clone(),
                                    kind: seg.kind.clone(),
                                    orb_deg: seg.orb_deg,
                                    local_x: mid_x - ox,
                                    local_y: mid_y - oy,
                                },
                            ));
                        }
                    }
                }
            }
        }

        // 3) House cusps — solo si el mouse está cerca del anillo de
        // casas (radio entre houses_inner y houses_outer + margen) y
        // ningún body ganó. Las cusps son líneas radiales — la
        // distancia angular al cusp más cercano determina el hit.
        if best.is_none() {
            let dx = mx - cx_px;
            let dy = my - cy_px;
            let mouse_r = (dx * dx + dy * dy).sqrt();
            let r_in = radii.houses_inner - 6.0;
            let r_out = radii.houses_outer + 6.0;
            if mouse_r >= r_in && mouse_r <= r_out {
                // Calcular la longitud zodiacal que corresponde a este
                // ángulo de pantalla (inversa de polar_to_screen).
                let screen_angle_deg = dy.atan2(dx).to_degrees(); // (-180, 180]
                // polar_to_screen: deg = 180 - (lon - asc + rot)
                // → lon = asc + 180 - screen_angle_deg - rot
                let lon = ((asc + 180.0 - screen_angle_deg - rot) as f32).rem_euclid(360.0);
                // Buscar cusp más cercano (con wraparound).
                for layer in &render.layers {
                    if matches!(layer.kind, LayerKind::Houses) {
                        if let Geometry::Ring { cusps_deg } = &layer.geometry {
                            for (i, c) in cusps_deg.iter().enumerate() {
                                let mut diff = (lon - c).abs();
                                if diff > 180.0 {
                                    diff = 360.0 - diff;
                                }
                                if diff < 2.5 {
                                    // Mouse cerca de ESTE cusp.
                                    let (gx, gy) = polar_to_screen(
                                        *c,
                                        asc,
                                        rot,
                                        (radii.houses_inner + radii.houses_outer) / 2.0,
                                    );
                                    best = Some((
                                        diff,
                                        HoverInfo::HouseCusp {
                                            house_number: (i as u8) + 1,
                                            deg: *c,
                                            local_x: cx_px + gx - ox,
                                            local_y: cy_px + gy - oy,
                                        },
                                    ));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        let new_hover = best.map(|(_, h)| h);
        let changed = match (&self.state.hover, &new_hover) {
            (Some(a), Some(b)) => a.key() != b.key(),
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.state.hover = new_hover;
            cx.notify();
        }
    }

    // ----- Internos: pan drag (MMB) -----

    fn on_pan_down(&mut self, position: Point<Pixels>, _cx: &mut Context<'_, Self>) {
        self.state.drag_pan = Some(PanDragState {
            start_pos: position,
            pan_x_start: self.state.view_pan_x,
            pan_y_start: self.state.view_pan_y,
        });
    }

    fn on_pan_move(&mut self, position: Point<Pixels>, cx: &mut Context<'_, Self>) {
        let Some(pan) = self.state.drag_pan.as_ref() else {
            return;
        };
        let dx: f32 = (position.x - pan.start_pos.x).into();
        let dy: f32 = (position.y - pan.start_pos.y).into();
        self.state.view_pan_x = pan.pan_x_start + dx;
        self.state.view_pan_y = pan.pan_y_start + dy;
        cx.notify();
    }

    fn on_pan_up(&mut self, cx: &mut Context<'_, Self>) {
        if self.state.drag_pan.take().is_some() {
            cx.notify();
        }
    }

    fn on_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let (_dx_px, dy_px) = match event.delta {
            ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
            ScrollDelta::Lines(p) => (p.x * 16.0, p.y * 16.0),
        };
        // Wheel = zoom puro, sin modifier. Pan se hace con drag (LMB
        // fuera del anillo, o MMB). 100px de scroll ≈ ±20% zoom.
        let factor = (dy_px * 0.002).exp();
        self.zoom_by(factor, cx);
    }

    fn on_jog_up(&mut self, cx: &mut Context<'_, Self>) {
        let gr = self.state.gr_active();
        let Some(jog) = self.state.drag_jog.take() else {
            return;
        };
        if gr {
            // El scrub GR se aplicó en vivo durante el drag; al soltar
            // no queda nada que confirmar.
            return;
        }
        // 1° de arco ≈ 4 minutos de tiempo sideral (15°/hora).
        // CW visual (delta negativa en nuestra convención) → tiempo
        // hacia adelante.
        let delta_minutes = (-jog.accumulated_delta_deg * 4.0) as i64;
        if delta_minutes != 0 {
            self.state.time_offset_minutes =
                self.state.time_offset_minutes.saturating_add(delta_minutes);
            // Snap visual: el shell recomputa con el nuevo offset y el
            // render trae el ascendant rotado.
            self.state.view_rotation_deg = 0.0;
            cx.emit(CanvasEvent::TimeOffsetChanged(self.state.time_offset_minutes));
            cx.notify();
        } else {
            self.state.view_rotation_deg = 0.0;
            cx.notify();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _w: &mut Window, cx: &mut Context<'_, Self>) {
        let key = event.keystroke.key.as_str();
        let kind = match key {
            "d" | "D" => LayerKind::SignDial,
            "h" | "H" => LayerKind::Houses,
            "x" | "X" => LayerKind::Aspects,
            "p" | "P" => LayerKind::Bodies,
            "t" | "T" => LayerKind::Outer,
            "r" | "R" => {
                self.reset_time_offset(cx);
                return;
            }
            "0" => {
                self.reset_view(cx);
                return;
            }
            "c" | "C" => {
                self.toggle_coords(cx);
                return;
            }
            "s" | "S" => {
                cx.emit(CanvasEvent::ExportSvgRequested);
                return;
            }
            _ => return,
        };
        self.toggle_layer(kind, cx);
    }
}

// =====================================================================
// Geometría de pantalla
// =====================================================================

const WHEEL_SIZE: f32 = 580.0;
const WHEEL_MARGIN: f32 = 28.0;

/// Pinta un gradiente radial de profundidad sobre el background del
/// canvas — efecto vignette. Se aproxima al gradient radial (no
/// soportado nativamente por gpui en `.bg()`) pintando ~28 anillos
/// concéntricos del centro hacia afuera, con alpha creciente hacia el
/// borde. El centro queda claro y los extremos se oscurecen, dando
/// sensación de "el wheel emerge desde la profundidad".
///
/// Solo activo en themes dark — sobre papel (light / print) el panel
/// queda plano: una viñeta sobre fondo claro tiñe el papel y rompe
/// la metáfora "impresión".
fn paint_depth_field(bounds: Bounds<Pixels>, window: &mut Window, theme: &Theme) {
    if !theme.is_dark {
        return;
    }
    let ox: f32 = bounds.origin.x.into();
    let oy: f32 = bounds.origin.y.into();
    let bw: f32 = bounds.size.width.into();
    let bh: f32 = bounds.size.height.into();
    if bw <= 0.0 || bh <= 0.0 {
        return;
    }
    let cx = ox + bw / 2.0;
    let cy = oy + bh / 2.0;
    // El gradient se extiende hasta la diagonal del rectángulo para
    // que las esquinas estén dentro del último anillo (sin "halo"
    // visible donde se corta).
    let r_max = ((bw * bw + bh * bh).sqrt()) / 2.0 * 1.05;
    let steps = 28;
    // Color: casi-negro con tinte ligero del panel (el panel es dark).
    let deep = hsla(230.0 / 360.0, 0.30, 0.04, 1.0);
    // Stroke de cada anillo: el ancho cubre 1/steps del radio para
    // que no queden gaps entre anillos.
    let stroke_w = (r_max / steps as f32) * 1.15;
    for i in 0..steps {
        let t = i as f32 / (steps - 1) as f32;
        let r = r_max * t;
        // Curva ease-in: alpha crece de 0 (centro) a ~0.55 (borde),
        // con la mayor parte del cambio en la mitad exterior. t² da
        // ese "fondo profundo en el perímetro sin opacar el centro".
        let alpha = 0.55 * (t * t);
        stroke_circle(window, cx, cy, r, stroke_w, with_alpha(deep, alpha));
    }
}

fn bounds_center(bounds: Bounds<Pixels>) -> (f32, f32) {
    let ox: f32 = bounds.origin.x.into();
    let oy: f32 = bounds.origin.y.into();
    let bw: f32 = bounds.size.width.into();
    let bh: f32 = bounds.size.height.into();
    (ox + bw / 2.0, oy + bh / 2.0)
}

/// Radio del anillo exterior derivado del width *actual* del canvas
/// (que ya está escalado por view_scale). Mantiene la proporción del
/// margen contra `WHEEL_SIZE` original, así el hit-test del jog-dial y
/// las cusps se adapta automáticamente al zoom sin que cada caller
/// recalcule `view_scale`.
fn effective_r_outer(bounds: Bounds<Pixels>) -> f32 {
    let bw: f32 = bounds.size.width.into();
    let scale = if WHEEL_SIZE > 0.0 { bw / WHEEL_SIZE } else { 1.0 };
    (bw - WHEEL_MARGIN * scale * 2.0) / 2.0
}

// =====================================================================
// Render
// =====================================================================

impl Render for AstrologyCanvas {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let palette = AstroPalette::for_theme(&theme);
        let entity = cx.entity();
        let focus = self.focus_handle.clone();

        let body = match &self.state.mode {
            CanvasMode::Empty => render_empty(&theme),
            CanvasMode::Wheel { render } => render_wheel(
                &theme,
                &palette,
                render,
                self.state.view_rotation_deg,
                self.state.time_offset_minutes,
                self.state.view_scale,
                self.state.view_pan_x,
                self.state.view_pan_y,
                &self.state.layer_visibility,
                self.state.show_coords,
                self.state.hover.as_ref(),
                self.state.rectificacion.as_ref(),
                entity,
            ),
            CanvasMode::Thumbnails { items, .. } => render_thumbnails(&theme, items),
        };

        // Depth field: capa absoluta detrás del body, ocupa todo el
        // canvas. Vignette radial — el centro queda claro y los
        // bordes se oscurecen, dando profundidad sin "ruido" de
        // puntos. Solo en themes dark (en papel rompería la
        // metáfora).
        let theme_for_depth = theme.clone();
        let depth_field = canvas(
            |_b, _w, _cx| (),
            move |bounds, _, window, _| paint_depth_field(bounds, window, &theme_for_depth),
        )
        .absolute()
        .size_full();

        div()
            .id("astrology-canvas-root")
            .track_focus(&focus)
            .key_context("AstrologyCanvas")
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, w, _cx| {
                    w.focus(&this.focus_handle);
                }),
            )
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .size_full()
            .bg(theme.bg_panel.clone())
            .relative()
            .overflow_hidden()
            .child(depth_field)
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .child(body),
            )
    }
}

// =====================================================================
// Modos: empty / thumbnails / wheel
// =====================================================================

fn render_empty(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(12.0))
        .child(
            div()
                .text_size(px(13.0))
                .text_color(theme.fg_muted)
                .child("Tahuantinsuyu"),
        )
        .child(
            div()
                .text_size(px(11.0))
                .text_color(theme.fg_disabled)
                .child("Seleccioná una carta en el árbol para empezar."),
        )
}

fn render_thumbnails(theme: &Theme, items: &[ThumbnailItem]) -> gpui::Div {
    if items.is_empty() {
        return div()
            .text_size(px(12.0))
            .text_color(theme.fg_muted)
            .child("Sin cartas en este grupo todavía.");
    }
    let mut row = div().flex().flex_row().flex_wrap().gap(px(12.0));
    for it in items {
        row = row.child(
            div()
                .w(px(140.0))
                .h(px(160.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.bg_panel_alt.clone())
                .flex()
                .flex_col()
                .items_center()
                .justify_end()
                .pb(px(8.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(theme.fg_text)
                        .child(it.label.clone()),
                ),
        );
    }
    row
}

// =====================================================================
// Wheel
// =====================================================================

#[allow(clippy::too_many_arguments)]
fn render_wheel(
    theme: &Theme,
    palette: &AstroPalette,
    render: &RenderModel,
    view_rotation_deg: f32,
    time_offset_minutes: i64,
    view_scale: f32,
    view_pan_x: f32,
    view_pan_y: f32,
    layer_visibility: &HashMap<LayerKind, bool>,
    show_coords: bool,
    hover: Option<&HoverInfo>,
    rectificacion: Option<&Rectificacion>,
    entity: gpui::Entity<AstrologyCanvas>,
) -> gpui::Div {
    let asc = render.ascendant_deg;
    let rot_offset = view_rotation_deg;
    // Todo el wheel escala uniforme: el cuadro contenedor y los anillos
    // crecen con view_scale, así que glifos, líneas y márgenes mantienen
    // sus proporciones. cx/cy_center vive en coords locales del wheel,
    // donde el wheel tiene tamaño `wheel_size` (no WHEEL_SIZE).
    let wheel_size = WHEEL_SIZE * view_scale;
    let wheel_margin = WHEEL_MARGIN * view_scale;
    let cx_center = wheel_size / 2.0;
    let cy_center = wheel_size / 2.0;
    let r_outer = (wheel_size - wheel_margin * 2.0) / 2.0;
    let radii = Radii::from_outer(r_outer);

    let visible = layer_visibility.clone();

    // --- Canvas element con todo el trazo + jog-dial drag ---
    let palette_paint = palette.clone();
    let theme_paint = theme.clone();
    let layers_paint: Vec<Layer> = render.layers.clone();
    let gr_triggers_paint: Vec<GrTrigger> = render.gr_triggers.clone();
    let asc_for_paint = asc;
    let mc_for_paint = render.midheaven_deg;
    let visibility_for_paint = visible.clone();
    let entity_for_canvas = entity.clone();
    // Hover focus para el highlight de aspectos — solo cuando el hover
    // es un Body (sobre un planeta), no sobre cusps ni aspectos.
    let hover_focus_paint: Option<String> = match hover {
        Some(HoverInfo::Body { symbol, .. }) => Some(symbol.clone()),
        _ => None,
    };
    let canvas_element = canvas(
        move |_b: Bounds<Pixels>, _w, _cx| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            // Painting de la rueda.
            paint_wheel(
                bounds,
                window,
                &theme_paint,
                &palette_paint,
                &layers_paint,
                asc_for_paint,
                mc_for_paint,
                rot_offset,
                radii,
                &visibility_for_paint,
                hover_focus_paint.as_deref(),
                &gr_triggers_paint,
            );

            // Handlers de mouse — se registran cada frame contra el
            // window; GPUI los reemplaza al re-renderear. LMB despacha
            // entre jog-dial (sobre el anillo) y pan (afuera). MMB es
            // pan secundario para usuarios con scroll-mouse.
            let entity_d = entity_for_canvas.clone();
            window.on_mouse_event(move |ev: &MouseDownEvent, _, _w, cx| {
                if !bounds.contains(&ev.position) {
                    return;
                }
                match ev.button {
                    MouseButton::Left => {
                        let mods = ev.modifiers;
                        entity_d.update(cx, |this, cx| {
                            this.on_primary_down(ev.position, mods, bounds, cx)
                        });
                    }
                    MouseButton::Middle => {
                        entity_d.update(cx, |this, cx| this.on_pan_down(ev.position, cx));
                    }
                    _ => {}
                }
            });
            let entity_m = entity_for_canvas.clone();
            window.on_mouse_event(move |ev: &MouseMoveEvent, _, _w, cx| {
                if ev.dragging() {
                    entity_m.update(cx, |this, cx| {
                        if this.state.drag_pan.is_some() {
                            this.on_pan_move(ev.position, cx);
                        } else {
                            this.on_jog_move(ev.position, bounds, cx);
                        }
                    });
                } else if bounds.contains(&ev.position) {
                    // Mouse hover sin drag: hit-test sobre los body
                    // glyphs para el tooltip.
                    entity_m.update(cx, |this, cx| this.on_hover_check(ev.position, bounds, cx));
                } else {
                    entity_m.update(cx, |this, cx| {
                        if this.state.hover.take().is_some() {
                            cx.notify();
                        }
                    });
                }
            });
            let entity_u = entity_for_canvas.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, _, _w, cx| {
                entity_u.update(cx, |this, cx| {
                    this.on_pan_up(cx);
                    this.on_jog_up(cx);
                });
            });
        },
    )
    .absolute()
    .w(px(wheel_size))
    .h(px(wheel_size));

    // El wheel ya no tiene bg propio — antes era un cuadrado con
    // gradient que cortaba contra el fondo del panel; ahora el panel
    // (con su starfield encima en `render`) fluye continuo a través
    // del área del wheel, dando el efecto de "rueda flotando en el
    // universo" en lugar de "rueda sobre placa cuadrada".
    let mut wheel = div()
        .relative()
        .w(px(wheel_size))
        .h(px(wheel_size))
        .ml(px(view_pan_x))
        .mt(px(view_pan_y))
        .child(canvas_element);

    // Factor de escala para los glyphs DOM. Los radii ya están
    // escalados (vienen de wheel_size = WHEEL_SIZE * view_scale), pero
    // los tamaños de fuente y disco están hardcoded — los multiplico
    // por view_scale para que el zoom afecte uniformemente todo el
    // contenido visual del wheel, no solo la geometría del canvas.
    let s = view_scale;
    // Color del halo para los discos detrás de glyphs y pills — se
    // calcula una sola vez, lo usan planetas, casas, ASC/MC y los
    // coord labels.
    let halo_bg = glyph_halo(theme);
    // Sign glyphs.
    if visible.get(&LayerKind::SignDial).copied().unwrap_or(true) {
        let sign_ring_mid = (radii.sign_outer + radii.sign_inner) / 2.0;
        for layer in &render.layers {
            if matches!(layer.kind, LayerKind::SignDial) {
                for g in &layer.glyphs {
                    let (x, y) = polar_to_screen(g.deg, asc, rot_offset, sign_ring_mid);
                    let color = element_color_for_sign(palette, &g.symbol);
                    wheel = wheel.child(centered_glyph(
                        cx_center + x,
                        cy_center + y,
                        20.0 * s,
                        18.0 * s,
                        sign_unicode(&g.symbol).into(),
                        color,
                    ));
                }
            }
        }
    }

    // House numbers + (opcional) coord del cusp.
    //
    // El layer `natal` usa Zona CD (entre aros C y D); `topocentric`
    // usa Zona BC (entre aros B y C). Los house numbers se posan al
    // centro de la zona; las coord pills se posan adyacentes al aro
    // interior de la propia zona, así no se sale del bloque.
    if visible.get(&LayerKind::Houses).copied().unwrap_or(true) {
        let house_label_color = house_ring_color(palette);
        for layer in &render.layers {
            if matches!(layer.kind, LayerKind::Houses) {
                let is_topo = layer.module_id == "topocentric";
                let (r_out, r_in) = if is_topo {
                    (radii.topo_houses_outer, radii.topo_houses_inner)
                } else {
                    (radii.houses_outer, radii.houses_inner)
                };
                let label_r = (r_out + r_in) / 2.0;
                let coord_r = r_in + (r_out - r_in) * 0.18;
                for g in &layer.glyphs {
                    let (x, y) = polar_to_screen(g.deg, asc, rot_offset, label_r);
                    if let Some(h) = g.house {
                        wheel = wheel.child(centered_glyph(
                            cx_center + x,
                            cy_center + y,
                            16.0 * s,
                            11.0 * s,
                            format!("{}", h).into(),
                            house_label_color,
                        ));
                        if show_coords {
                            let coord = format_coord_compact(g.deg);
                            let (lx, ly) =
                                polar_to_screen(g.deg, asc, rot_offset, coord_r);
                            wheel = wheel.child(coord_label(
                                cx_center + lx,
                                cy_center + ly,
                                coord.into(),
                                theme.fg_muted,
                                halo_bg,
                                8.5 * s,
                            ));
                        }
                    }
                }
            }
        }
    }

    // Planet glyphs: natal en `bodies` + overlays (progression,
    // solar_arc) en sus rings, ambos con disco-halo para legibilidad
    // contra cualquier fondo. El natal lleva un tamaño un poco mayor
    // que los overlays para que se lea como "el cuerpo principal".
    if visible.get(&LayerKind::Bodies).copied().unwrap_or(true) {
        for layer in &render.layers {
            if matches!(layer.kind, LayerKind::Bodies) {
                let is_natal = layer.module_id == "natal";
                let is_topo = layer.module_id == "topocentric";
                let is_pd_direct = layer.module_id == "pd_direct";
                let is_pd_converse = layer.module_id == "pd_converse";
                let is_pd = is_pd_direct || is_pd_converse;
                let ring = radii.body_ring(&layer.module_id);
                let alpha = if is_natal {
                    1.0
                } else if is_topo {
                    0.75
                } else if is_pd {
                    0.80
                } else {
                    0.88
                };
                let font_size = (if is_natal {
                    18.0
                } else if is_topo {
                    15.0
                } else if is_pd {
                    13.0
                } else {
                    14.0
                }) * s;
                let disk_size_base = (if is_natal {
                    26.0
                } else if is_topo {
                    22.0
                } else if is_pd {
                    20.0
                } else {
                    22.0
                }) * s;

                // Anti-solapamiento: spread directo sobre TODOS los
                // glyphs con `min_sep = disk_angular` (tangencial: los
                // discos se rozan sin pisarse) y `max_shift = disk_angular`
                // (cap fuerte: ningún planeta puede alejarse más de
                // un diámetro de disco de su grado real). El cap evita
                // que un cluster denso "empuje" a planetas lejanos.
                //
                // En paralelo, `find_clusters` con threshold = ancho
                // del disco × 1.2 detecta pares/tríos cercanos para
                // que compartan label. Sin esto, dos planetas en
                // conjunción a 5° real se ven con sus discos
                // separados a 10° y CADA UNO con su pill — dos labels
                // que dicen casi lo mismo, exactamente lo que el
                // usuario reporta como "se repiten en vez de
                // reutilizarse".
                let raw_degs: Vec<f32> = layer.glyphs.iter().map(|g| g.deg).collect();
                let disk_angular_deg =
                    (disk_size_base / (std::f32::consts::TAU * ring)) * 360.0;
                let max_shift = disk_angular_deg;
                let (display_degs, residual) =
                    spread_angles(&raw_degs, disk_angular_deg, max_shift);
                let cluster_thresh = disk_angular_deg * 1.2;
                let clusters = find_clusters(&raw_degs, cluster_thresh);

                let cluster_centroids: Vec<f32> = clusters
                    .iter()
                    .map(|c| {
                        let mut sx = 0.0_f32;
                        let mut sy = 0.0_f32;
                        for &idx in c {
                            let a = raw_degs[idx].to_radians();
                            sx += a.cos();
                            sy += a.sin();
                        }
                        sy.atan2(sx).to_degrees().rem_euclid(360.0)
                    })
                    .collect();
                let display_centroids: Vec<f32> = clusters
                    .iter()
                    .map(|c| {
                        let mut sx = 0.0_f32;
                        let mut sy = 0.0_f32;
                        for &idx in c {
                            let a = display_degs[idx].to_radians();
                            sx += a.cos();
                            sy += a.sin();
                        }
                        sy.atan2(sx).to_degrees().rem_euclid(360.0)
                    })
                    .collect();
                let mut cluster_of = vec![0usize; layer.glyphs.len()];
                for (ci, c) in clusters.iter().enumerate() {
                    for &idx in c {
                        cluster_of[idx] = ci;
                    }
                }

                let shrink_residual = (1.0 - residual * 0.30).clamp(0.60, 1.0);

                // El hovered glyph y su cluster reciben tratamiento
                // especial: lo postponemos para pintarlo al FINAL del
                // árbol (queda por encima del resto = z-order), y le
                // damos un border más fuerte. Su label cluster también
                // se destaca (color fg_text en lugar de fg_muted, font
                // un punto más grande).
                let hovered_sym: Option<&str> = match hover {
                    Some(HoverInfo::Body { symbol, .. }) => Some(symbol.as_str()),
                    _ => None,
                };
                let hovered_idx: Option<usize> = hovered_sym.and_then(|sym| {
                    layer.glyphs.iter().position(|g| g.symbol == sym)
                });
                let hovered_cluster: Option<usize> = hovered_idx.map(|i| cluster_of[i]);

                for (i, g) in layer.glyphs.iter().enumerate() {
                    if Some(i) == hovered_idx {
                        continue; // se pinta al final
                    }
                    // Achicar discos cuando el glyph está en cluster
                    // (≥2 miembros) — al estar pegados se ven mejor
                    // un poco más pequeños.
                    let cluster_size = clusters[cluster_of[i]].len();
                    let in_cluster_shrink = if cluster_size >= 2 { 0.86 } else { 1.0 };
                    let effective_shrink = shrink_residual * in_cluster_shrink;
                    let disk_size = disk_size_base * effective_shrink;
                    let font_size_eff = (font_size * effective_shrink).max(11.0);

                    let display_deg = display_degs[i];
                    let (x, y) = polar_to_screen(display_deg, asc, rot_offset, ring);
                    let color = with_alpha(planet_color(palette, &g.symbol), alpha);
                    let mut glyph_text = planet_unicode(&g.symbol).to_string();
                    if g.retrograde {
                        glyph_text.push('ᴿ');
                    }
                    if let Some(marker) = &g.dignity_marker {
                        glyph_text.push_str(marker);
                    }
                    wheel = wheel.child(planet_glyph(
                        cx_center + x,
                        cy_center + y,
                        disk_size,
                        font_size_eff,
                        glyph_text.into(),
                        color,
                        halo_bg,
                        with_alpha(color, 0.85),
                    ));

                    // Coord label individual: solo cuando el glyph
                    // está SOLO en su cluster (≥2 ⇒ label compartido).
                    if show_coords && (is_natal || is_topo) && cluster_size == 1 {
                        let coord = format_coord_compact(g.deg);
                        let label_r = ring - disk_size * 1.3;
                        let (lx, ly) =
                            polar_to_screen(display_deg, asc, rot_offset, label_r);
                        wheel = wheel.child(coord_label(
                            cx_center + lx,
                            cy_center + ly,
                            coord.into(),
                            theme.fg_muted,
                            halo_bg,
                            8.5 * s,
                        ));
                    }
                }

                // Label compartido para CADA cluster con ≥2 miembros.
                // El del cluster hovereado se destaca: color fg_text
                // (vs fg_muted) y font un punto más grande.
                if show_coords && (is_natal || is_topo) {
                    let disk_size_typical = disk_size_base * shrink_residual * 0.86;
                    for (ci, c) in clusters.iter().enumerate() {
                        if c.len() < 2 {
                            continue;
                        }
                        let highlighted = Some(ci) == hovered_cluster;
                        let center_display_deg = display_centroids[ci];
                        let center_real_deg = cluster_centroids[ci];
                        let symbols: String = c
                            .iter()
                            .map(|&idx| planet_unicode(&layer.glyphs[idx].symbol))
                            .collect::<Vec<_>>()
                            .join(" ");
                        let coord = format_coord_compact(center_real_deg);
                        let text = format!("{}  {}", symbols, coord);
                        let label_r = ring - disk_size_typical * 1.5;
                        let (lx, ly) = polar_to_screen(
                            center_display_deg,
                            asc,
                            rot_offset,
                            label_r,
                        );
                        let (fg, font_sz) = if highlighted {
                            (theme.fg_text, 10.0 * s)
                        } else {
                            (theme.fg_muted, 9.0 * s)
                        };
                        wheel = wheel.child(coord_label(
                            cx_center + lx,
                            cy_center + ly,
                            text.into(),
                            fg,
                            halo_bg,
                            font_sz,
                        ));
                    }
                }

                // Render del glyph hovered al FINAL: queda encima del
                // resto en z-order. Disco un poco más grande y border
                // más prominente para destacar.
                if let Some(hi) = hovered_idx {
                    let g = &layer.glyphs[hi];
                    let display_deg = display_degs[hi];
                    let (x, y) = polar_to_screen(display_deg, asc, rot_offset, ring);
                    let color = with_alpha(planet_color(palette, &g.symbol), alpha);
                    let mut glyph_text = planet_unicode(&g.symbol).to_string();
                    if g.retrograde {
                        glyph_text.push('ᴿ');
                    }
                    if let Some(marker) = &g.dignity_marker {
                        glyph_text.push_str(marker);
                    }
                    let disk_size = disk_size_base * shrink_residual * 1.18;
                    let font_size_eff = font_size * shrink_residual * 1.12;
                    wheel = wheel.child(planet_glyph(
                        cx_center + x,
                        cy_center + y,
                        disk_size,
                        font_size_eff,
                        glyph_text.into(),
                        color,
                        halo_bg,
                        color, // border al color pleno (no .85) — destaca
                    ));
                    // Si el hovered no está en cluster compartido,
                    // pintamos su coord individual destacada acá.
                    let cluster_size = clusters[cluster_of[hi]].len();
                    if show_coords && (is_natal || is_topo) && cluster_size == 1 {
                        let coord = format_coord_compact(g.deg);
                        let label_r = ring - disk_size * 1.3;
                        let (lx, ly) =
                            polar_to_screen(display_deg, asc, rot_offset, label_r);
                        wheel = wheel.child(coord_label(
                            cx_center + lx,
                            cy_center + ly,
                            coord.into(),
                            theme.fg_text,
                            halo_bg,
                            10.0 * s,
                        ));
                    }
                }
            }
        }
    }

    // Planet glyphs en el outer ring — transit o synastry (slot
    // compartido, mutuamente excluyentes a nivel de Shell). Disco un
    // poco más chico que el natal — el outer es "secundario".
    if visible.get(&LayerKind::Outer).copied().unwrap_or(true) {
        for layer in &render.layers {
            if matches!(layer.kind, LayerKind::Outer)
                && (OUTER_RING_MODULES.contains(&layer.module_id.as_str()))
            {
                let disk_base = 20.0 * s;
                let raw_degs: Vec<f32> = layer.glyphs.iter().map(|g| g.deg).collect();
                let disk_angular =
                    (disk_base / (std::f32::consts::TAU * radii.transits)) * 360.0;
                let (display_degs, residual) =
                    spread_angles(&raw_degs, disk_angular, disk_angular);
                let shrink = (1.0 - residual * 0.30).clamp(0.60, 1.0);
                for (i, g) in layer.glyphs.iter().enumerate() {
                    let display_deg = display_degs[i];
                    let (x, y) = polar_to_screen(display_deg, asc, rot_offset, radii.transits);
                    let color = with_alpha(planet_color(palette, &g.symbol), 0.92);
                    let glyph_text = if g.retrograde {
                        format!("{}ᴿ", planet_unicode(&g.symbol))
                    } else {
                        planet_unicode(&g.symbol).into()
                    };
                    wheel = wheel.child(planet_glyph(
                        cx_center + x,
                        cy_center + y,
                        20.0 * s * shrink,
                        13.0 * s * shrink,
                        glyph_text.into(),
                        color,
                        halo_bg,
                        with_alpha(color, 0.75),
                    ));
                }
            }
        }
    }

    // Tooltip absoluto sobre el elemento hovered (cuerpo o cusp).
    if let Some(hov) = hover {
        let text = match hov {
            HoverInfo::Body {
                module_id,
                symbol,
                deg,
                house,
                retrograde,
                dignity_marker,
                annotation,
                ..
            } => {
                let sign_idx = ((deg / 30.0).floor() as usize) % 12;
                let sign_name = SIGN_NAMES_ES[sign_idx];
                let deg_in_sign = deg - (sign_idx as f32) * 30.0;
                let display_symbol = if module_id == "midpoints" {
                    // El symbol del midpoint es "a/b" — para el header
                    // del tooltip usamos los unicodes individuales.
                    if let Some((a, b)) = symbol.split_once('/') {
                        format!("{}/{}", planet_unicode(a), planet_unicode(b))
                    } else {
                        symbol.clone()
                    }
                } else {
                    planet_unicode(symbol).to_string()
                };
                let mut t = format!("{} {} · {:.1}°", display_symbol, sign_name, deg_in_sign);
                if let Some(h) = house {
                    t.push_str(&format!(" · Casa {}", h));
                }
                if *retrograde {
                    t.push_str(" · ℞");
                }
                if let Some(m) = dignity_marker {
                    t.push_str(&format!(" · {}", m));
                }
                if module_id == "midpoints" {
                    if let Some(a) = annotation {
                        t.push_str(&format!(" · {}", a));
                    }
                } else if module_id != "natal" {
                    t.push_str(&format!(" · {}", module_id));
                }
                t
            }
            HoverInfo::HouseCusp {
                house_number, deg, ..
            } => {
                let sign_idx = ((deg / 30.0).floor() as usize) % 12;
                let sign_name = SIGN_NAMES_ES[sign_idx];
                let deg_in_sign = deg - (sign_idx as f32) * 30.0;
                format!(
                    "Cusp Casa {} · {} {:.1}°",
                    house_number, sign_name, deg_in_sign
                )
            }
            HoverInfo::Aspect {
                module_id,
                from_body,
                to_body,
                kind,
                orb_deg,
                ..
            } => {
                let mut t = format!(
                    "{} {} {}  ·  orb {:.1}°",
                    planet_unicode(from_body),
                    aspect_unicode(kind),
                    planet_unicode(to_body),
                    orb_deg
                );
                if module_id != "natal" {
                    t.push_str(&format!(" · {}", module_id));
                }
                t
            }
        };
        let (lx, ly) = hov.local();
        let tip_x = (lx + 14.0).min(WHEEL_SIZE - 220.0).max(8.0);
        let tip_y = (ly - 28.0).max(8.0);
        wheel = wheel.child(
            div()
                .absolute()
                .left(px(tip_x))
                .top(px(tip_y))
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(6.0))
                .bg(theme.bg_panel_alt.clone())
                .border_1()
                .border_color(palette.angle_highlight)
                .text_size(px(11.0))
                .text_color(theme.fg_text)
                .child(SharedString::from(text)),
        );
    }

    // Labels ASC/MC/DESC/IC como pills en el perímetro — bg del halo
    // + border y texto en `angle_highlight`. Más legibles que el
    // centered_glyph plano del fase anterior, en especial sobre
    // fondos claros donde el ámbar/oro de angle_highlight se diluye.
    let angle_labels = [
        (asc, "ASC"),
        (render.midheaven_deg, "MC"),
        (render.descendant_deg, "DESC"),
        (render.imum_coeli_deg, "IC"),
    ];
    let label_r = r_outer * 1.08;
    for (deg, label) in angle_labels {
        let (x, y) = polar_to_screen(deg, asc, rot_offset, label_r);
        let pill_w = (if label.len() > 2 { 38.0 } else { 30.0 }) * s;
        let pill_h = 18.0 * s;
        wheel = wheel.child(
            div()
                .absolute()
                .left(px(cx_center + x - pill_w / 2.0))
                .top(px(cy_center + y - pill_h / 2.0))
                .w(px(pill_w))
                .h(px(pill_h))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(9.0 * s))
                .bg(halo_bg)
                .border_1()
                .border_color(with_alpha(palette.angle_highlight, 0.85))
                .text_size(px(11.0 * s))
                .text_color(palette.angle_highlight)
                .child(SharedString::from(label)),
        );
    }

    // --- Header + footer + indicador de tiempo ---
    let header = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(2.0))
        .child(
            div()
                .text_size(px(16.0))
                .text_color(theme.fg_text)
                .child(SharedString::from(render.title.clone())),
        );
    let header = if let Some(sub) = &render.subtitle {
        header.child(
            div()
                .text_size(px(11.0))
                .text_color(theme.fg_muted)
                .child(SharedString::from(sub.clone())),
        )
    } else {
        header
    };
    // Botón export SVG — pequeño, alineado a la derecha del title.
    let export_btn = div()
        .id("tts-canvas-export-svg")
        .px(px(10.0))
        .py(px(3.0))
        .rounded(px(4.0))
        .bg(theme.bg_button())
        .hover(|s| s.bg(theme.bg_button_hover()))
        .border_1()
        .border_color(theme.border)
        .text_size(px(10.0))
        .text_color(theme.fg_text)
        .child("⬇ SVG")
        .on_click({
            let entity_e = entity.clone();
            move |_: &gpui::ClickEvent, _w, cx: &mut gpui::App| {
                entity_e.update(cx, |_this, cx| {
                    cx.emit(CanvasEvent::ExportSvgRequested);
                });
            }
        });
    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .child(header)
        .child(export_btn);

    let offset_label = format_offset(time_offset_minutes);
    let offset_color = if time_offset_minutes == 0 {
        theme.fg_disabled
    } else {
        palette.angle_highlight
    };
    let info_row = div()
        .flex()
        .flex_row()
        .gap(px(10.0))
        .child(
            div()
                .text_size(px(10.0))
                .text_color(theme.fg_disabled)
                .child(SharedString::from(format!(
                    "Asc {:.1}°  ·  MC {:.1}°  ·  {} ms",
                    render.ascendant_deg, render.midheaven_deg, render.compute_ms,
                ))),
        )
        .child(
            div()
                .text_size(px(10.0))
                .text_color(offset_color)
                .child(SharedString::from(offset_label)),
        )
        .child(
            div()
                .text_size(px(10.0))
                .text_color(theme.fg_disabled)
                .child(
                    "[D]ial [H]ouses as[X]pects [P]lanets [T]ransits [C]oords  ·  Ctrl+drag = tiempo/edad GR  ·  [0] reset zoom  ·  [R] reset tiempo  ·  [S]vg",
                ),
        );

    // Badges de overlays activos. Cada uno se pinta como pill con
    // background sutil y border tenue. Solo aparecen cuando hay
    // overlays — la carta natal pura ve solo el info_row.
    let badges_row = if render.overlays.is_empty() {
        None
    } else {
        let mut row = div().flex().flex_row().flex_wrap().gap(px(6.0));
        // Badge "natal" base, siempre presente cuando hay overlays —
        // ayuda al usuario a leer la pila de izquierda a derecha.
        row = row.child(badge(theme, palette, "natal", "Natal", true));
        for ov in &render.overlays {
            row = row.child(badge(theme, palette, &ov.module_id, &ov.label, false));
        }
        Some(row)
    };

    let mut footer = div().flex().flex_col().items_center().gap(px(4.0)).child(info_row);
    if let Some(b) = badges_row {
        footer = footer.child(b);
    }

    // Dial uraniano de 90°. Aparece cuando el módulo Uranian está
    // activo: una proyección geométrica de los cuerpos sobre el eje
    // 0-90° + la lista de fórmulas (cuerpos en el mismo grado dial).
    if render.overlays.iter().any(|o| o.module_id == "uranian") {
        let mut section = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(theme.fg_muted)
                    .child("Dial 90° (uraniano)"),
            )
            .child(render_uranian_dial(
                theme,
                palette,
                &render.layers,
                &render.uranian_groups,
            ));
        // Pills de fórmulas, sólo si se detectó algún eje.
        if !render.uranian_groups.is_empty() {
            let mut row = div()
                .flex()
                .flex_row()
                .flex_wrap()
                .justify_center()
                .gap(px(6.0));
            for group in &render.uranian_groups {
                let bodies_text: String = group
                    .bodies
                    .iter()
                    .map(|b| planet_unicode(b))
                    .collect::<Vec<_>>()
                    .join(" ");
                row = row.child(
                    div()
                        .px(px(8.0))
                        .py(px(2.0))
                        .rounded(px(10.0))
                        .bg(theme.bg_panel_alt.clone())
                        .border_1()
                        .border_color(with_alpha(palette.angle_highlight, 0.6))
                        .text_size(px(11.0))
                        .text_color(theme.fg_text)
                        .child(SharedString::from(format!(
                            "{}  ·  {:.1}°",
                            bodies_text, group.mod90_deg
                        ))),
                );
            }
            section = section.child(row);
        }
        footer = footer.child(section);
    }

    // Espectro de fuerza armónica — histograma clicable. Aparece sólo
    // en modo armónico (harmonic > 1) y guía qué armónico mirar.
    if !render.harmonic_spectrum.is_empty() {
        footer = footer.child(render_harmonic_spectrum(
            theme,
            palette,
            &render.harmonic_spectrum,
            render.harmonic,
            entity.clone(),
        ));
    }

    // Perfil del rectificador automático — la curva del barrido de horas
    // candidatas. Aparece tras correr una rectificación; su valle marca
    // la hora de nacimiento que mejor explica los eventos conocidos.
    if let Some(r) = rectificacion {
        if !r.perfil.is_empty() {
            footer = footer.child(render_rectify_profile(theme, palette, r));
        }
    }

    // Lista textual de aspectos (top 12 por orb). Compacta, en grid
    // de 3 columnas, fonts pequeños. Solo aparece cuando hay aspectos
    // computados.
    if !render.aspect_summary.is_empty() {
        let mut grid = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(10.0))
            .max_w(px(WHEEL_SIZE + 80.0))
            .justify_center();
        for ap in render.aspect_summary.iter().take(12) {
            let kind_sym = aspect_unicode(&ap.kind);
            let line = format!(
                "{} {} {}  ·  {:.1}°{}",
                planet_unicode(&ap.from_body),
                kind_sym,
                planet_unicode(&ap.to_body),
                ap.orb_deg,
                match ap.applying {
                    Some(true) => " A",
                    Some(false) => " S",
                    None => "",
                }
            );
            let prefix = if ap.module_id == "natal" {
                String::new()
            } else {
                format!("[{}] ", ap.module_id)
            };
            grid = grid.child(
                div()
                    .px(px(6.0))
                    .py(px(2.0))
                    .text_size(px(11.0))
                    .text_color(aspect_color(palette, &ap.kind))
                    .child(SharedString::from(format!("{}{}", prefix, line))),
            );
        }
        footer = footer.child(grid);
    }

    // El wheel va solo, salvo en modo GR: ahí lo acompaña el HUD
    // lateral de triggers de rectificación, anclado a su derecha.
    let body = if render.gr_triggers.is_empty() {
        div().child(wheel)
    } else {
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(14.0))
            .child(wheel)
            .child(render_gr_hud(theme, &render.gr_triggers))
    };

    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.0))
        .child(header)
        .child(body)
        .child(footer)
}

/// HUD lateral de rectificación GR: lista los triggers de direcciones
/// primarias ordenados por orbe (los más cerrados arriba). El color va
/// de rojo (orbe apretado) a gris (orbe ancho); las convergencias
/// directo+converso llevan un marcador ✦ y un fondo resaltado.
fn render_gr_hud(theme: &Theme, triggers: &[GrTrigger]) -> gpui::Div {
    const SHOWN: usize = 20;
    let event_count = triggers.iter().filter(|t| t.event).count();

    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .w(px(238.0))
        .p(px(10.0))
        .rounded(px(8.0))
        .bg(theme.bg_panel_alt.clone())
        .border_1()
        .border_color(theme.border);

    col = col.child(
        div()
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme.fg_text)
                    .child("Triggers GR"),
            )
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(theme.fg_muted)
                    .child(SharedString::from(format!(
                        "{} · {} conv.",
                        triggers.len(),
                        event_count
                    ))),
            ),
    );
    col = col.child(
        div()
            .text_size(px(9.0))
            .text_color(theme.fg_disabled)
            .mb(px(4.0))
            .child("rectificación · orbe ascendente"),
    );

    for t in triggers.iter().take(SHOWN) {
        let color = if t.event {
            hsla(0.0, 0.88, 0.64, 1.0)
        } else {
            gr_orb_color(t.orb_deg)
        };
        let marker = if t.event { "✦" } else { "·" };
        let line = format!(
            "{} {}{} → {}  {}",
            marker,
            planet_unicode(&t.promissor),
            t.direction.short(),
            gr_target_glyph(&t.natal_target),
            format_orb(t.orb_deg),
        );
        let mut row = div()
            .px(px(5.0))
            .py(px(2.0))
            .rounded(px(3.0))
            .text_size(px(11.0))
            .text_color(color)
            .child(SharedString::from(line));
        if t.event {
            row = row.bg(with_alpha(hsla(0.0, 0.80, 0.50, 1.0), 0.16));
        }
        col = col.child(row);
    }
    if triggers.len() > SHOWN {
        col = col.child(
            div()
                .text_size(px(9.0))
                .text_color(theme.fg_disabled)
                .mt(px(3.0))
                .child(SharedString::from(format!(
                    "+{} más",
                    triggers.len() - SHOWN
                ))),
        );
    }
    col
}

/// Histograma del espectro de fuerza armónica. Cada barra es clicable:
/// un clic salta el slider de armónico a esa armónica. La barra de la
/// armónica activa va resaltada.
fn render_harmonic_spectrum(
    theme: &Theme,
    palette: &AstroPalette,
    spectrum: &[f32],
    current: u32,
    entity: gpui::Entity<AstrologyCanvas>,
) -> gpui::Div {
    const BAR_AREA_H: f32 = 46.0;
    let max = spectrum.iter().copied().fold(0.0_f32, f32::max).max(1e-3);

    let mut bars = div().flex().flex_row().items_end().gap(px(2.0));
    for (i, &strength) in spectrum.iter().enumerate() {
        let h = (i as u32) + 1;
        let norm = (strength / max).clamp(0.0, 1.0);
        let bar_h = (norm * BAR_AREA_H).max(2.0);
        let is_current = h == current;
        let color = if is_current {
            palette.angle_highlight
        } else {
            with_alpha(palette.angle_highlight, 0.28 + norm * 0.45)
        };
        // Etiqueta cada 4 armónicas (+ la primera y la activa) para no
        // saturar la tira.
        let label = if h == current || h == 1 || h % 4 == 0 {
            format!("{h}")
        } else {
            String::new()
        };
        let column = div()
            .id(SharedString::from(format!("tts-harmonic-bar-{h}")))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.0))
            .cursor_pointer()
            .child(
                div()
                    .h(px(BAR_AREA_H))
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(div().w(px(11.0)).h(px(bar_h)).rounded(px(1.5)).bg(color)),
            )
            .child(
                div()
                    .text_size(px(7.0))
                    .text_color(if is_current {
                        palette.angle_highlight
                    } else {
                        theme.fg_disabled
                    })
                    .child(SharedString::from(label)),
            )
            .on_click({
                let entity = entity.clone();
                move |_: &gpui::ClickEvent, _w, cx: &mut gpui::App| {
                    entity.update(cx, |_this, cx| {
                        cx.emit(CanvasEvent::HarmonicSelected(h));
                    });
                }
            });
        bars = bars.child(column);
    }

    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(3.0))
        .child(
            div()
                .text_size(px(10.0))
                .text_color(theme.fg_muted)
                .child(SharedString::from(format!(
                    "Espectro armónico · H{current} activo · clic para saltar"
                ))),
        )
        .child(bars)
}

/// Curva del barrido del rectificador automático. Cada barra es una hora
/// de nacimiento candidata; su altura crece cuanto MEJOR explica los
/// eventos conocidos (menor puntaje de convergencia). La barra más alta
/// —el valle del puntaje— es la hora rectificada, y va resaltada.
fn render_rectify_profile(
    theme: &Theme,
    palette: &AstroPalette,
    r: &Rectificacion,
) -> gpui::Div {
    const BAR_AREA_H: f32 = 46.0;

    let (min_p, max_p) = r.perfil.iter().fold(
        (f32::INFINITY, f32::NEG_INFINITY),
        |(lo, hi), &(_, p)| (lo.min(p), hi.max(p)),
    );
    let rango = (max_p - min_p).max(1e-3);
    let primero = r.perfil.first().map(|&(o, _)| o).unwrap_or(0);
    let ultimo = r.perfil.last().map(|&(o, _)| o).unwrap_or(0);

    let mut bars = div().flex().flex_row().items_end().gap(px(2.0));
    for &(offset, puntaje) in &r.perfil {
        // Fitness: el mejor candidato (puntaje mínimo) → barra más alta.
        let fitness = ((max_p - puntaje) / rango).clamp(0.0, 1.0);
        let bar_h = (fitness * BAR_AREA_H).max(2.0);
        let es_mejor = offset == r.mejor_offset_minutos;
        let color = if es_mejor {
            palette.angle_highlight
        } else {
            with_alpha(palette.angle_highlight, 0.25 + fitness * 0.45)
        };
        // Etiquetar sólo los hitos: el mejor, el 0 y los dos extremos.
        let label = if es_mejor || offset == 0 || offset == primero || offset == ultimo {
            if offset == 0 {
                "0".to_string()
            } else {
                format!("{offset:+}")
            }
        } else {
            String::new()
        };
        let column = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.0))
            .child(
                div()
                    .h(px(BAR_AREA_H))
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(div().w(px(9.0)).h(px(bar_h)).rounded(px(1.5)).bg(color)),
            )
            .child(
                div()
                    .text_size(px(7.0))
                    .text_color(if es_mejor {
                        palette.angle_highlight
                    } else {
                        theme.fg_disabled
                    })
                    .child(SharedString::from(label)),
            );
        bars = bars.child(column);
    }

    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(3.0))
        .child(
            div()
                .text_size(px(10.0))
                .text_color(theme.fg_muted)
                .child(SharedString::from(format!(
                    "Rectificación · hora {:+} min · puntaje {:.2} · el valle es la hora",
                    r.mejor_offset_minutos, r.mejor_puntaje
                ))),
        )
        .child(bars)
}

/// Dial uraniano de 90°: proyección geométrica de los cuerpos natales
/// sobre un eje horizontal 0-90° (longitud mod 90). Los cuerpos que
/// forman una fórmula uraniana (mismo grado dial) caen agrupados y se
/// resaltan; clusters densos se escalonan en filas para legibilidad.
fn render_uranian_dial(
    theme: &Theme,
    palette: &AstroPalette,
    layers: &[Layer],
    groups: &[UranianGroup],
) -> gpui::Div {
    const DIAL_W: f32 = 560.0;
    const ROW_H: f32 = 18.0;
    const MAX_ROWS: usize = 4;
    const AXIS_Y: f32 = ROW_H * MAX_ROWS as f32;
    const MIN_GAP: f32 = 17.0;

    let Some(layer) = layers
        .iter()
        .find(|l| l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies))
    else {
        return div();
    };

    // `(símbolo, x, agrupado)` ordenados por posición en el dial.
    let mut marks: Vec<(String, f32, bool)> = layer
        .glyphs
        .iter()
        .map(|g| {
            let x = g.deg.rem_euclid(90.0) / 90.0 * DIAL_W;
            let grouped = groups
                .iter()
                .any(|gr| gr.bodies.iter().any(|b| b == &g.symbol));
            (g.symbol.clone(), x, grouped)
        })
        .collect();
    marks.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut track = div().relative().w(px(DIAL_W)).h(px(AXIS_Y + 22.0));

    // Eje base.
    track = track.child(
        div()
            .absolute()
            .left(px(0.0))
            .top(px(AXIS_Y))
            .w(px(DIAL_W))
            .h(px(1.0))
            .bg(with_alpha(palette.dial_ring, 0.7)),
    );
    // Ticks 0 / 22½ / 45 / 67½ / 90 — las divisiones duras del dial.
    for (deg, label) in [
        (0.0_f32, "0°"),
        (22.5, "22½°"),
        (45.0, "45°"),
        (67.5, "67½°"),
        (90.0, "90°"),
    ] {
        let x = deg / 90.0 * DIAL_W;
        track = track
            .child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(AXIS_Y))
                    .w(px(1.0))
                    .h(px(6.0))
                    .bg(with_alpha(palette.dial_ring, 0.85)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(x - 14.0))
                    .top(px(AXIS_Y + 8.0))
                    .w(px(28.0))
                    .flex()
                    .justify_center()
                    .text_size(px(8.0))
                    .text_color(theme.fg_disabled)
                    .child(SharedString::from(label)),
            );
    }

    // Glyphs, con escalonado vertical para los clusters.
    let mut last_x = f32::NEG_INFINITY;
    let mut row = 0usize;
    for (symbol, x, grouped) in &marks {
        if x - last_x < MIN_GAP {
            row = (row + 1).min(MAX_ROWS - 1);
        } else {
            row = 0;
        }
        last_x = *x;
        let color = if *grouped {
            palette.angle_highlight
        } else {
            with_alpha(planet_color(palette, symbol), 0.55)
        };
        track = track.child(
            div()
                .absolute()
                .left(px(x - 8.0))
                .top(px(row as f32 * ROW_H))
                .w(px(16.0))
                .h(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.0))
                .text_color(color)
                .child(SharedString::from(planet_unicode(symbol).to_string())),
        );
    }

    track
}

/// Color de un trigger GR según su orbe: rojo intenso (orbe cerrado,
/// contacto fuerte) que se desatura hacia gris al ensancharse. El
/// orbe de referencia (gris pleno) es el orbe del HUD, 2°.
fn gr_orb_color(orb_deg: f32) -> Hsla {
    let t = (orb_deg / 2.0).clamp(0.0, 1.0);
    let s = 0.82 + (0.10 - 0.82) * t;
    let l = 0.62 + (0.52 - 0.62) * t;
    hsla(0.0, s, l, 1.0)
}

/// Orbe en grados → texto compacto `D°MM'`.
fn format_orb(orb_deg: f32) -> String {
    let total_min = (orb_deg.abs() * 60.0).round() as i64;
    format!("{}°{:02}'", total_min / 60, total_min % 60)
}

/// Glyph corto de un punto natal objetivo: ángulos como texto,
/// cuerpos vía [`planet_unicode`].
fn gr_target_glyph(name: &str) -> String {
    match name {
        "asc" => "Asc".to_string(),
        "mc" => "MC".to_string(),
        "desc" => "Dsc".to_string(),
        "ic" => "IC".to_string(),
        other => planet_unicode(other).to_string(),
    }
}

/// Pequeña pill con la etiqueta de un overlay activo. El borde toma
/// color según el "tipo" del módulo para ayudar a mapear a su anillo
/// en el wheel: natal = neutro, outer ring share (transit/synastry/
/// planetary_return) = palette.angle_highlight, inner overlays
/// (progression/solar_arc) = palette.house_cusp.
fn badge(theme: &Theme, palette: &AstroPalette, module_id: &str, label: &str, is_natal: bool) -> gpui::Div {
    let border = if is_natal {
        theme.border
    } else {
        match module_id {
            "transit" | "synastry" | "planetary_return" => palette.angle_highlight,
            "progression" | "solar_arc" => palette.house_cusp,
            _ => theme.border,
        }
    };
    div()
        .px(px(8.0))
        .py(px(2.0))
        .rounded(px(10.0))
        .bg(theme.bg_panel_alt.clone())
        .border_1()
        .border_color(border)
        .text_size(px(10.0))
        .text_color(theme.fg_text)
        .child(SharedString::from(label.to_string()))
}

fn format_offset(minutes: i64) -> String {
    if minutes == 0 {
        return "⏱ ahora".to_string();
    }
    let sign = if minutes > 0 { '+' } else { '-' };
    let m = minutes.unsigned_abs();
    let days = m / (60 * 24);
    let hours = (m / 60) % 24;
    let mins = m % 60;
    if days > 0 {
        format!("⏱ {}{}d {:02}h {:02}m", sign, days, hours, mins)
    } else if hours > 0 {
        format!("⏱ {}{:02}h {:02}m", sign, hours, mins)
    } else {
        format!("⏱ {}{:02}m", sign, mins)
    }
}

// =====================================================================
// Painting
// =====================================================================

// `Radii` + helpers migraron a `cosmobiologia-render` (crate
// agnóstico de surface, compila a WASM y nativo). Re-export para
// que el código del canvas siga refiriendo `Radii` sin cambiar
// imports en cada call site.
use cosmobiologia_render::Radii;

#[allow(clippy::too_many_arguments)]
// `hover_focus`: symbol del planeta hovereado en este frame (si lo
// hay). Las líneas de aspecto que NO tocan a ese planeta se opacan
// para que el usuario lea claramente "qué afecta a qué". Si `None`,
// todas las líneas se pintan a alpha plena.
fn paint_wheel(
    bounds: Bounds<Pixels>,
    window: &mut Window,
    theme: &Theme,
    palette: &AstroPalette,
    layers: &[Layer],
    ascendant_deg: f32,
    midheaven_deg: f32,
    rot_offset_deg: f32,
    radii: Radii,
    visibility: &HashMap<LayerKind, bool>,
    hover_focus: Option<&str>,
    gr_triggers: &[GrTrigger],
) {
    let (cx, cy) = bounds_center(bounds);
    let show = |k: LayerKind| visibility.get(&k).copied().unwrap_or(true);

    // 1. Sectores zodiacales (parte del SignDial layer).
    if show(LayerKind::SignDial) {
        paint_sign_sectors(window, cx, cy, &radii, palette, ascendant_deg, rot_offset_deg);

        // Anillos del dial con efecto 3D: highlight interior + base +
        // shadow exterior. El highlight es 1 px hacia el centro con
        // luminancia +0.18; la shadow 1 px hacia afuera con -0.18.
        // El bevel central — varios strokes finos con alpha en bell
        // curve entre sign_inner y sign_outer — da volumen al dial.
        stroke_circle_3d(window, cx, cy, radii.sign_outer, 1.5, palette.dial_ring, theme);
        stroke_circle_3d(window, cx, cy, radii.sign_inner, 1.0, palette.dial_ring, theme);
        paint_dial_bevel(window, cx, cy, &radii, palette, theme);

        // Cusps zodiacales cada 30°.
        for i in 0..12 {
            let lon = (i as f32) * 30.0;
            let color = palette.dial_ring;
            paint_radial_line(
                window,
                cx,
                cy,
                lon,
                ascendant_deg,
                rot_offset_deg,
                radii.sign_inner,
                radii.sign_outer,
                color,
                1.0,
            );
        }
    }

    // 2. Casas — doble anillo (inner + outer) + cusps radiales +
    // énfasis Asc/IC/Desc/MC. La doble línea vuelve a la zona de
    // casas una "corona" claramente identificable. Color derivado
    // de `house_cusp` con un hue shift para que el sistema
    // ascensional (casas) se distinga visualmente del eclíptico
    // (dial zodiacal) que va en dorado.
    if show(LayerKind::Houses) {
        let house_base = house_ring_color(palette);
        let house_color = with_alpha(house_base, 0.85);
        stroke_circle_3d(window, cx, cy, radii.houses_outer, 1.1, house_color, theme);
        stroke_circle_3d(window, cx, cy, radii.houses_inner, 1.1, house_color, theme);
        // Si hay capa topocéntrica activa, pintar también sus dos
        // anillos (con stroke más sutil que el geocéntrico, para que
        // se lea como "sistema ascensional" sin competir).
        if layers
            .iter()
            .any(|l| matches!(l.kind, LayerKind::Houses) && l.module_id == "topocentric")
        {
            let topo_color = with_alpha(house_base, 0.55);
            stroke_circle(window, cx, cy, radii.topo_houses_outer, 0.8, topo_color);
            stroke_circle(window, cx, cy, radii.topo_houses_inner, 0.8, topo_color);
        }

        for layer in layers {
            if matches!(layer.kind, LayerKind::Houses) {
                let is_topo = layer.module_id == "topocentric";
                let (r_in, r_out) = if is_topo {
                    (radii.topo_houses_inner, radii.topo_houses_outer)
                } else {
                    (radii.houses_inner, radii.houses_outer)
                };
                if let Geometry::Ring { cusps_deg } = &layer.geometry {
                    for (i, c) in cusps_deg.iter().enumerate() {
                        let is_angle = i == 0 || i == 3 || i == 6 || i == 9;
                        let color = if is_topo {
                            with_alpha(house_base, 0.60)
                        } else if is_angle {
                            palette.angle_highlight
                        } else {
                            with_alpha(house_base, 0.75)
                        };
                        let width = if is_angle && !is_topo { 2.0 } else { 0.8 };
                        if is_topo {
                            // Topocéntrico: cusp como línea punteada
                            // en su propio anillo cercano al sign
                            // dial — se distingue del Placidus
                            // geocéntrico por el dash pattern y la
                            // ubicación más exterior.
                            paint_segment(
                                window,
                                cx
                                    + polar_to_screen(
                                        *c,
                                        ascendant_deg,
                                        rot_offset_deg,
                                        r_in,
                                    )
                                    .0,
                                cy
                                    + polar_to_screen(
                                        *c,
                                        ascendant_deg,
                                        rot_offset_deg,
                                        r_in,
                                    )
                                    .1,
                                cx
                                    + polar_to_screen(
                                        *c,
                                        ascendant_deg,
                                        rot_offset_deg,
                                        r_out,
                                    )
                                    .0,
                                cy
                                    + polar_to_screen(
                                        *c,
                                        ascendant_deg,
                                        rot_offset_deg,
                                        r_out,
                                    )
                                    .1,
                                color,
                                Some((3.0, 2.5)),
                                1.0,
                            );
                        } else {
                            paint_radial_line(
                                window,
                                cx,
                                cy,
                                *c,
                                ascendant_deg,
                                rot_offset_deg,
                                r_in,
                                r_out,
                                color,
                                width,
                            );
                        }
                    }
                }
            }
        }

        // Cruz completa Asc-Desc + MC-IC, alpha bastante visible para
        // que orienten la lectura sin competir con cuerpos/aspectos.
        // 4 radios desde el centro: ASC, DESC (=asc+180), MC, IC
        // (=mc+180). `paint_radial_line` con r_inner=0 pinta un radio
        // del centro al borde — la cruz es la unión de los 4.
        let axis_color = with_alpha(palette.angle_highlight, 0.55);
        for axis_deg in [
            ascendant_deg,
            ascendant_deg + 180.0,
            midheaven_deg,
            midheaven_deg + 180.0,
        ] {
            paint_radial_line(
                window,
                cx,
                cy,
                axis_deg,
                ascendant_deg,
                rot_offset_deg,
                0.0,
                radii.houses_outer,
                axis_color,
                1.4,
            );
        }
    }

    // Aro D — único anillo visible del bloque de planetas natales
    // (la idea del "carril doble" se descartó: confundía con el
    // sistema de casas). El aro E (`radii.aspects`) no se pinta por
    // diseño; solo es ancla invisible de las líneas.
    if show(LayerKind::Bodies) {
        let belt_color = with_alpha(palette.dial_ring, 0.55);
        stroke_circle_3d(window, cx, cy, radii.houses_inner, 0.9, belt_color, theme);
        // GR dual-ring: si las capas de direcciones primarias están
        // presentes, marcar sus anillos para que el visual lea como
        // "abrazo" del cinturón natal. La directa va punteada,
        // la conversa también — la diferencia entre las dos es la
        // ubicación radial (afuera vs adentro del cinturón natal).
        let has_pd = layers.iter().any(|l| {
            matches!(l.kind, LayerKind::Bodies)
                && (l.module_id == "pd_direct" || l.module_id == "pd_converse")
        });
        if has_pd {
            let pd_color = with_alpha(palette.angle_highlight, 0.50);
            for r in [radii.pd_direct, radii.pd_converse] {
                // Pintamos el anillo como tramo punteado fino: 24
                // segmentos cortos a lo largo del círculo.
                let steps = 96;
                for i in 0..steps {
                    if i % 2 != 0 {
                        continue;
                    }
                    let a0 = (i as f32) / (steps as f32) * std::f32::consts::TAU;
                    let a1 = ((i + 1) as f32) / (steps as f32) * std::f32::consts::TAU;
                    let x0 = cx + r * a0.cos();
                    let y0 = cy + r * a0.sin();
                    let x1 = cx + r * a1.cos();
                    let y1 = cy + r * a1.sin();
                    paint_segment(window, x0, y0, x1, y1, pd_color, None, 0.6);
                }
            }
        }

        // Resaltado de convergencias GR: por cada punto natal donde un
        // trigger directo y otro converso coinciden dentro del
        // micro-orbe, un eje brillante atraviesa la zona del dual-ring
        // hasta el cinturón natal. Es la señal de rectificación — si la
        // hora natal es correcta, el evento real cae sobre este eje.
        let mut event_degs: Vec<f32> = gr_triggers
            .iter()
            .filter(|t| t.event)
            .map(|t| t.natal_deg)
            .collect();
        event_degs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        event_degs.dedup_by(|a, b| (*a - *b).abs() < 0.02);
        if !event_degs.is_empty() {
            let hot = hsla(0.0, 0.86, 0.60, 1.0);
            let marker_r = (radii.sign_outer * 0.014).max(2.5);
            for deg in event_degs {
                paint_radial_line(
                    window,
                    cx,
                    cy,
                    deg,
                    ascendant_deg,
                    rot_offset_deg,
                    radii.pd_converse,
                    radii.houses_inner,
                    with_alpha(hot, 0.92),
                    2.6,
                );
                let (mx, my) =
                    polar_to_screen(deg, ascendant_deg, rot_offset_deg, radii.bodies);
                paint_glow(window, cx + mx, cy + my, marker_r * 2.0, hot);
                fill_circle(window, cx + mx, cy + my, marker_r, hot);
            }
        }
    }

    // 3. Aspectos. Cada module_id usa su par de radios — natal-natal
    // ambos en `aspects`, cross con transit en `bodies → transits`,
    // cross con progression en `bodies → progression`.
    if show(LayerKind::Aspects) {
        let mono = palette.is_monochrome();
        for layer in layers {
            if matches!(layer.kind, LayerKind::Aspects) {
                if let Geometry::Lines(segs) = &layer.geometry {
                    let (r_from, r_to) = radii.aspect_endpoints(&layer.module_id);
                    let is_cross = r_from != r_to;
                    for seg in segs {
                        // Filtro minors con orbe ancho: los aspectos
                        // menores (quincunx, semi-square, quintile…)
                        // solo se trazan si están MUY apretados
                        // (orbe ≤ 3°). Sobre 3° ensucian sin aportar.
                        if !is_major_aspect(&seg.kind) && seg.orb_deg.abs() > 3.0 {
                            continue;
                        }
                        let base = aspect_color(palette, &seg.kind);
                        let base = with_alpha(base, base.a * seg.opacity);
                        // Hover focus: si hay un planeta hovereado y
                        // este segmento NO lo toca, lo atenuamos al
                        // 18%; si lo toca o no hay hover, va pleno.
                        let touches_hover = hover_focus
                            .map(|sym| seg.from_body == sym || seg.to_body == sym)
                            .unwrap_or(true);
                        let factor = if touches_hover { 1.0 } else { 0.18 };
                        let color = with_alpha(base, base.a * factor);
                        let dash = if mono {
                            dash_pattern_for_kind(&seg.kind)
                        } else {
                            None
                        };
                        // Width inverso al orbe: orbes cerrados se ven
                        // gruesos (aspecto "fuerte"), orbes amplios
                        // finos. Mayores van un escalón más gruesos
                        // que menores en su mismo orbe.
                        let width = aspect_width(&seg.kind, seg.orb_deg, mono);
                        if is_cross {
                            paint_cross_aspect_line(
                                window,
                                cx,
                                cy,
                                seg.from_deg,
                                seg.to_deg,
                                ascendant_deg,
                                rot_offset_deg,
                                r_from,
                                r_to,
                                color,
                                dash,
                            );
                        } else {
                            paint_aspect_line(
                                window,
                                cx,
                                cy,
                                seg.from_deg,
                                seg.to_deg,
                                ascendant_deg,
                                rot_offset_deg,
                                r_from,
                                color,
                                dash,
                                width,
                            );
                        }
                    }
                }
            }
        }
    }

    // 4. Marcadores de posición exacta. Antes el dot era "el planeta";
    // ahora el glyph (con halo, en DOM) lo es. El círculo acá queda
    // como marker de precisión angular — chico, alpha alta, sobre el
    // anillo correspondiente. Glow se mantiene para Sol/Luna como
    // toque místico, pero también reducido.
    if show(LayerKind::Bodies) {
        let dot_r = (radii.sign_outer * 0.009).max(1.5);
        for layer in layers {
            if matches!(layer.kind, LayerKind::Bodies) {
                let ring = radii.body_ring(&layer.module_id);
                let is_natal = layer.module_id == "natal";
                let alpha = if is_natal { 1.0 } else { 0.85 };
                for g in &layer.glyphs {
                    let color = with_alpha(planet_color(palette, &g.symbol), alpha);
                    let (x, y) = polar_to_screen(g.deg, ascendant_deg, rot_offset_deg, ring);
                    if is_natal && (g.symbol == "sun" || g.symbol == "moon") {
                        paint_glow(window, cx + x, cy + y, dot_r * 1.8, color);
                    }
                    fill_circle(window, cx + x, cy + y, dot_r, color);
                }
            }
        }
    }

    // Anillos guía para los overlays internos (progression, solar_arc).
    let guide_inset = radii.sign_outer * 0.03;
    for (module_id, ring) in [
        ("progression", radii.progression),
        ("solar_arc", radii.solar_arc),
    ] {
        let active = layers
            .iter()
            .any(|l| matches!(l.kind, LayerKind::Bodies) && l.module_id == module_id);
        if active {
            stroke_circle(
                window,
                cx,
                cy,
                ring + guide_inset,
                0.5,
                with_alpha(palette.house_cusp, 0.35),
            );
            stroke_circle(
                window,
                cx,
                cy,
                ring - guide_inset,
                0.5,
                with_alpha(palette.house_cusp, 0.35),
            );
        }
    }

    // 5. Outer ring (transit o synastry overlay): anillo guía + dots
    // de la capa activa. Son mutuamente excluyentes a nivel de Shell;
    // si alguno de los dos está prendido, pintamos el slot.
    let outer_active = layers.iter().any(|l| {
        matches!(l.kind, LayerKind::Outer)
            && OUTER_RING_MODULES.contains(&l.module_id.as_str())
    });
    if outer_active && show(LayerKind::Outer) {
        let band = radii.sign_outer * 0.035;
        stroke_circle_3d(
            window,
            cx,
            cy,
            radii.transits + band,
            0.7,
            with_alpha(palette.dial_ring, 0.55),
            theme,
        );
        stroke_circle_3d(
            window,
            cx,
            cy,
            radii.transits - band,
            0.7,
            with_alpha(palette.dial_ring, 0.55),
            theme,
        );

        let dot_r = (radii.sign_outer * 0.008).max(1.5);
        for layer in layers {
            if matches!(layer.kind, LayerKind::Outer)
                && (OUTER_RING_MODULES.contains(&layer.module_id.as_str()))
            {
                for g in &layer.glyphs {
                    let color = with_alpha(planet_color(palette, &g.symbol), 0.85);
                    let (x, y) =
                        polar_to_screen(g.deg, ascendant_deg, rot_offset_deg, radii.transits);
                    fill_circle(window, cx + x, cy + y, dot_r, color);
                }
            }
        }
    }
}

fn paint_sign_sectors(
    window: &mut Window,
    cx: f32,
    cy: f32,
    radii: &Radii,
    palette: &AstroPalette,
    ascendant_deg: f32,
    rot_offset_deg: f32,
) {
    const SUBDIVISIONS: usize = 18;
    for i in 0..12 {
        let lon_start = (i as f32) * 30.0;
        let lon_end = lon_start + 30.0;
        let element = sign_element_by_index(i);
        let color = with_alpha(palette.element(element), 0.10);

        let mut builder = PathBuilder::fill();
        let (x0, y0) = polar_to_screen(lon_start, ascendant_deg, rot_offset_deg, radii.sign_inner);
        builder.move_to(point(px(cx + x0), px(cy + y0)));

        for k in 1..=SUBDIVISIONS {
            let t = lon_start + (lon_end - lon_start) * (k as f32) / (SUBDIVISIONS as f32);
            let (x, y) = polar_to_screen(t, ascendant_deg, rot_offset_deg, radii.sign_inner);
            builder.line_to(point(px(cx + x), px(cy + y)));
        }
        let (xe, ye) = polar_to_screen(lon_end, ascendant_deg, rot_offset_deg, radii.sign_outer);
        builder.line_to(point(px(cx + xe), px(cy + ye)));

        for k in (0..SUBDIVISIONS).rev() {
            let t = lon_start + (lon_end - lon_start) * (k as f32) / (SUBDIVISIONS as f32);
            let (x, y) = polar_to_screen(t, ascendant_deg, rot_offset_deg, radii.sign_outer);
            builder.line_to(point(px(cx + x), px(cy + y)));
        }
        builder.close();
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    }
}

fn stroke_circle(window: &mut Window, cx: f32, cy: f32, r: f32, width: f32, color: Hsla) {
    const SEGMENTS: usize = 96;
    let mut builder = PathBuilder::stroke(px(width));
    for i in 0..=SEGMENTS {
        let t = (i as f32) / (SEGMENTS as f32) * (2.0 * PI);
        let x = cx + r * t.cos();
        let y = cy + r * t.sin();
        if i == 0 {
            builder.move_to(point(px(x), px(y)));
        } else {
            builder.line_to(point(px(x), px(y)));
        }
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// Pinta 3 halos concéntricos con alpha decreciente alrededor de un
/// punto — usado para Sol/Luna natales. El radio crece, la opacidad
/// cae: el ojo lo lee como "esto irradia". Sin glow real (GPUI 0.2 no
/// tiene radial gradient), pero el shading concéntrico convence.
fn paint_glow(window: &mut Window, cx: f32, cy: f32, base_r: f32, color: Hsla) {
    const HALOS: [(f32, f32); 3] = [(5.0, 0.05), (3.0, 0.12), (1.8, 0.22)];
    for (mult, alpha) in HALOS {
        let r = base_r * mult;
        let halo = hsla(color.h, color.s, color.l, alpha);
        fill_circle(window, cx, cy, r, halo);
    }
}

fn fill_circle(window: &mut Window, cx: f32, cy: f32, r: f32, color: Hsla) {
    const SEGMENTS: usize = 32;
    let mut builder = PathBuilder::fill();
    builder.move_to(point(px(cx + r), px(cy)));
    for i in 1..=SEGMENTS {
        let t = (i as f32) / (SEGMENTS as f32) * (2.0 * PI);
        let x = cx + r * t.cos();
        let y = cy + r * t.sin();
        builder.line_to(point(px(x), px(y)));
    }
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_radial_line(
    window: &mut Window,
    cx: f32,
    cy: f32,
    longitude_deg: f32,
    ascendant_deg: f32,
    rot_offset_deg: f32,
    r_inner: f32,
    r_outer: f32,
    color: Hsla,
    width: f32,
) {
    let (xi, yi) = polar_to_screen(longitude_deg, ascendant_deg, rot_offset_deg, r_inner);
    let (xo, yo) = polar_to_screen(longitude_deg, ascendant_deg, rot_offset_deg, r_outer);
    let mut builder = PathBuilder::stroke(px(width));
    builder.move_to(point(px(cx + xi), px(cy + yi)));
    builder.line_to(point(px(cx + xo), px(cy + yo)));
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_aspect_line(
    window: &mut Window,
    cx: f32,
    cy: f32,
    a_deg: f32,
    b_deg: f32,
    ascendant_deg: f32,
    rot_offset_deg: f32,
    r: f32,
    color: Hsla,
    dash: Option<(f32, f32)>,
    width: f32,
) {
    let (xa, ya) = polar_to_screen(a_deg, ascendant_deg, rot_offset_deg, r);
    let (xb, yb) = polar_to_screen(b_deg, ascendant_deg, rot_offset_deg, r);
    paint_segment(window, cx + xa, cy + ya, cx + xb, cy + yb, color, dash, width);
}

/// Línea de aspecto natal ↔ tránsito: extremos en radios distintos.
/// El `from_deg` cae sobre el ring de cuerpos natales (`r_from`); el
/// `to_deg` sobre el ring de tránsito (`r_to`). Trazo más fino que el
/// natal-natal para no competir visualmente.
#[allow(clippy::too_many_arguments)]
fn paint_cross_aspect_line(
    window: &mut Window,
    cx: f32,
    cy: f32,
    natal_deg: f32,
    transit_deg: f32,
    ascendant_deg: f32,
    rot_offset_deg: f32,
    r_from: f32,
    r_to: f32,
    color: Hsla,
    dash: Option<(f32, f32)>,
) {
    let (xa, ya) = polar_to_screen(natal_deg, ascendant_deg, rot_offset_deg, r_from);
    let (xb, yb) = polar_to_screen(transit_deg, ascendant_deg, rot_offset_deg, r_to);
    paint_segment(window, cx + xa, cy + ya, cx + xb, cy + yb, color, dash, 0.7);
}

/// Pinta un segmento entre dos puntos. Si `dash` es `Some((on, off))`,
/// itera el vector pintando trechos de `on` px con gaps de `off` px.
/// Si `None`, una sola línea continua. Usado por todos los aspect
/// painters — el dash pattern es la forma de distinguir kinds en
/// el theme BW (donde el color no sirve).
fn paint_segment(
    window: &mut Window,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    color: Hsla,
    dash: Option<(f32, f32)>,
    width: f32,
) {
    let Some((on, off)) = dash else {
        let mut b = PathBuilder::stroke(px(width));
        b.move_to(point(px(x0), px(y0)));
        b.line_to(point(px(x1), px(y1)));
        if let Ok(p) = b.build() {
            window.paint_path(p, color);
        }
        return;
    };
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.1 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    let step = on + off;
    if step < 0.1 {
        return;
    }
    let mut t = 0.0;
    while t < len {
        let t_end = (t + on).min(len);
        let sx = x0 + ux * t;
        let sy = y0 + uy * t;
        let ex = x0 + ux * t_end;
        let ey = y0 + uy * t_end;
        let mut b = PathBuilder::stroke(px(width));
        b.move_to(point(px(sx), px(sy)));
        b.line_to(point(px(ex), px(ey)));
        if let Ok(p) = b.build() {
            window.paint_path(p, color);
        }
        t += step;
    }
}

/// `true` para los 5 aspectos Ptoloméicos (conjunction, sextile,
/// square, trine, opposition). Cualquier otro `kind` se considera
/// menor — quincunx, semi-square, quintile, sesquiquadrate, etc.
fn is_major_aspect(kind: &str) -> bool {
    matches!(
        kind,
        "conjunction" | "sextile" | "square" | "trine" | "opposition"
    )
}

/// Grosor de línea de aspecto inverso al orbe. La idea: a orbe 0°
/// (aspecto exacto) la línea va gruesa porque "pesa" más; a orbe
/// amplio se afina. Los mayores arrancan en un techo más alto que
/// los menores. En BW se le suma un poquito a todos porque las
/// líneas competen con sus dash patterns.
fn aspect_width(kind: &str, orb_deg: f32, mono: bool) -> f32 {
    let orb = orb_deg.abs();
    let major = is_major_aspect(kind);
    // Orbe de referencia para normalizar: ~8° para mayores, ~3° para
    // menores. Más allá la línea ya está afinada al mínimo.
    let max_orb = if major { 8.0 } else { 3.0 };
    let t = (1.0 - (orb / max_orb)).clamp(0.0, 1.0);
    let (min_w, max_w) = if major { (0.7, 2.1) } else { (0.5, 1.2) };
    let w = min_w + (max_w - min_w) * t;
    if mono { w + 0.2 } else { w }
}

/// Dash pattern por aspecto, para modo monocromático. En modo color
/// el caller pasa `None` y las líneas van sólidas. Patterns elegidos
/// para que cada kind sea distinguible a ojo:
/// - conjunction/opposition: sólido (más peso visual, son los
///   aspectos "fuertes")
/// - square: dash medio (4 on / 3 off)
/// - trine: dash largo (8 on / 2 off) — casi sólido pero distinguible
/// - sextile: dotted (1.5 on / 3 off)
/// - minor: dotted finísimo (1 on / 4 off)
fn dash_pattern_for_kind(kind: &str) -> Option<(f32, f32)> {
    match kind {
        "conjunction" | "opposition" => None,
        "square" => Some((4.0, 3.0)),
        "trine" => Some((8.0, 2.0)),
        "sextile" => Some((1.5, 3.0)),
        _ => Some((1.0, 4.0)),
    }
}

// =====================================================================
// Helpers
// =====================================================================

/// Distancia mínima entre un punto y un segmento de recta. Usado por
/// hover_check para detectar proximity a líneas de aspectos.
fn dist_point_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let dx = bx - ax;
    let dy = by - ay;
    let len_sq = dx * dx + dy * dy;
    if len_sq < f32::EPSILON {
        // Segmento degenerado → distancia al punto a.
        let pdx = px - ax;
        let pdy = py - ay;
        return (pdx * pdx + pdy * pdy).sqrt();
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / len_sq).clamp(0.0, 1.0);
    let proj_x = ax + t * dx;
    let proj_y = ay + t * dy;
    let dx2 = px - proj_x;
    let dy2 = py - proj_y;
    (dx2 * dx2 + dy2 * dy2).sqrt()
}

// `polar_to_screen` se importa desde `cosmobiologia-render`.
use cosmobiologia_render::polar_to_screen;

fn centered_glyph(
    x: f32,
    y: f32,
    box_size: f32,
    font_size: f32,
    text: SharedString,
    color: Hsla,
) -> gpui::Div {
    div()
        .absolute()
        .left(px(x - box_size / 2.0))
        .top(px(y - box_size / 2.0))
        .w(px(box_size))
        .h(px(box_size))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(font_size))
        .text_color(color)
        .child(text)
}

/// Glyph de planeta con disco-halo detrás del char. El disco viene en
/// `disk_bg` (semi-opaco para que se vea a través el fondo del wheel)
/// y `disk_border` (típicamente el color del planeta). El char por
/// dentro va en `text_color` — recomendado el color del planeta sobre
/// disco neutro, o color contrastante sobre disco coloreado.
fn planet_glyph(
    x: f32,
    y: f32,
    disk_size: f32,
    font_size: f32,
    text: SharedString,
    text_color: Hsla,
    disk_bg: Hsla,
    disk_border: Hsla,
) -> gpui::Div {
    div()
        .absolute()
        .left(px(x - disk_size / 2.0))
        .top(px(y - disk_size / 2.0))
        .w(px(disk_size))
        .h(px(disk_size))
        .rounded_full()
        .bg(disk_bg)
        .border_1()
        .border_color(disk_border)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(font_size))
        .text_color(text_color)
        .child(text)
}

/// Disco base (px) de un body glyph según `module_id` y kind. Lo
/// usan render_wheel (para pintar) y on_hover_check (para
/// hit-testear) — ambos deben coincidir o el hover apunta a una
/// posición distinta a donde se pinta el disco.
fn body_disk_base(module_id: &str, kind: LayerKind, view_scale: f32) -> f32 {
    let base = match kind {
        LayerKind::Outer => 20.0,
        LayerKind::Midpoints => 16.0,
        _ => match module_id {
            "natal" => 26.0,
            "topocentric" => 22.0,
            "pd_direct" | "pd_converse" => 20.0,
            _ => 22.0,
        },
    };
    base * view_scale
}

// `spread_angles` y `find_clusters` migraron a `cosmobiologia-render`.
use cosmobiologia_render::{find_clusters, spread_angles};

// `format_coord_compact` migró a `cosmobiologia-render`.
use cosmobiologia_render::format_coord_compact;

// Los tests de `spread_angles`, `find_clusters` y
// `format_coord_compact` viven ahora en `cosmobiologia-render::math`
// junto a sus implementaciones.

/// Pill pequeña con un coord ("14°♈") junto al glyph de un planeta
/// o cusp. Fondo halo + texto fg_muted, padding mínimo para no
/// saturar la rueda con etiquetas grandes.
fn coord_label(
    x: f32,
    y: f32,
    text: SharedString,
    fg: Hsla,
    halo_bg: Hsla,
    font_size: f32,
) -> gpui::Div {
    // Estimación del ancho basada en `chars().count()` (NO `text.len()`
    // — los chars unicode astronómicos cuentan 3 bytes pero ocupan
    // ~1 columna de fuente). Padding lateral muy pequeño en lugar de
    // un mínimo grande: pills con 1-3 chars no llevan "espacios en
    // negro" que sobrescriben elementos vecinos.
    let char_count = text.chars().count() as f32;
    let w = (char_count * font_size * 0.62 + font_size * 0.5).max(font_size * 1.4);
    let h = font_size + 5.0;
    div()
        .absolute()
        .left(px(x - w / 2.0))
        .top(px(y - h / 2.0))
        .w(px(w))
        .h(px(h))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(h / 2.0))
        .bg(halo_bg)
        .text_size(px(font_size))
        .text_color(fg)
        .child(text)
}

/// Color HSL semi-opaco para los halos de los glyphs — derivado del
/// theme. En dark va casi negro; en light casi blanco. Alpha alta para
/// que el char quede legible contra cualquier cosa que haya detrás
/// (anillo, líneas de aspecto, starfield).
fn glyph_halo(theme: &Theme) -> Hsla {
    if theme.is_dark {
        hsla(0.0, 0.0, 0.07, 0.92)
    } else {
        hsla(0.0, 0.0, 0.97, 0.92)
    }
}

fn with_alpha(c: Hsla, a: f32) -> Hsla {
    hsla(c.h, c.s, c.l, a.clamp(0.0, 1.0))
}

/// Devuelve `c` con la luminancia modificada por `delta` (clamp 0..1).
/// Útil para derivar highlight (+luma) y shadow (-luma) de un color
/// base manteniendo hue y saturación — efecto bevel/3D barato.
fn adjust_luma(c: Hsla, delta: f32) -> Hsla {
    hsla(c.h, c.s, (c.l + delta).clamp(0.0, 1.0), c.a)
}

/// Devuelve `c` con el hue desplazado `delta_deg` grados sobre el
/// círculo cromático (wrap a [0,1] en la escala normalizada de gpui).
/// Usado para derivar el color del anillo de casas desde el del dial
/// zodiacal — los dos sistemas (eclíptica vs ascensional) deben
/// distinguirse a primera vista pero compartir "familia" cromática.
fn shift_hue(c: Hsla, delta_deg: f32) -> Hsla {
    let new_h = (c.h + delta_deg / 360.0).rem_euclid(1.0);
    hsla(new_h, c.s, c.l, c.a)
}

/// Color para los anillos del sistema de casas (ascensional). En
/// paletas con color, lo derivamos de `house_cusp` con un hue shift
/// de ~140° para diferenciar de la eclíptica (que va con el dorado
/// de `dial_ring`). En BW devolvemos `house_cusp` tal cual — un
/// shift cromático en monocromo es ruido sin información.
fn house_ring_color(palette: &AstroPalette) -> Hsla {
    if palette.is_monochrome() {
        palette.house_cusp
    } else {
        shift_hue(palette.house_cusp, 140.0)
    }
}

/// Stroke con efecto embossed: 3 trazos concéntricos. El highlight va
/// 0.7 px hacia el centro con luminancia subida; el principal en `r`;
/// el shadow 0.7 px hacia afuera con luminancia bajada. La dirección
/// del bevel depende del theme: en dark el highlight es exterior (luz
/// "desde arriba"), en light interior (sombra "desde arriba" hacia
/// el centro).
fn stroke_circle_3d(
    window: &mut Window,
    cx: f32,
    cy: f32,
    r: f32,
    width: f32,
    color: Hsla,
    theme: &Theme,
) {
    let (hl_offset, sh_offset) = if theme.is_dark {
        (-0.7, 0.7)
    } else {
        (0.7, -0.7)
    };
    let hl = with_alpha(adjust_luma(color, 0.20), color.a * 0.55);
    let sh = with_alpha(adjust_luma(color, -0.18), color.a * 0.55);
    stroke_circle(window, cx, cy, r + hl_offset, (width * 0.7).max(0.4), hl);
    stroke_circle(window, cx, cy, r, width, color);
    stroke_circle(window, cx, cy, r + sh_offset, (width * 0.7).max(0.4), sh);
}

/// Bevel central del anillo de signos: ~10 strokes finos entre
/// sign_inner y sign_outer, con alpha en bell curve (máximo en el
/// medio, decae hacia los bordes). Genera la sensación de volumen
/// sin pintar gradient radial (no soportado en gpui canvas).
fn paint_dial_bevel(
    window: &mut Window,
    cx: f32,
    cy: f32,
    radii: &Radii,
    palette: &AstroPalette,
    theme: &Theme,
) {
    let steps = 10;
    let base = if theme.is_dark { 0.07 } else { 0.10 };
    let color = palette.dial_ring;
    for i in 0..steps {
        let t = (i as f32 + 0.5) / steps as f32;
        let r = radii.sign_inner + (radii.sign_outer - radii.sign_inner) * t;
        // Bell curve simétrica: |t-0.5|*2 da 0..1 desde el centro, lo
        // invertimos para que el centro tenga peso máximo.
        let bell = 1.0 - ((t - 0.5).abs() * 2.0);
        let a = base * bell;
        stroke_circle(window, cx, cy, r, 1.0, with_alpha(color, a));
    }
}

fn sign_unicode(name: &str) -> &'static str {
    match name {
        "aries" => "♈",
        "taurus" => "♉",
        "gemini" => "♊",
        "cancer" => "♋",
        "leo" => "♌",
        "virgo" => "♍",
        "libra" => "♎",
        "scorpio" => "♏",
        "sagittarius" => "♐",
        "capricorn" => "♑",
        "aquarius" => "♒",
        "pisces" => "♓",
        _ => "?",
    }
}

const SIGN_NAMES_ES: [&str; 12] = [
    "Aries",
    "Tauro",
    "Géminis",
    "Cáncer",
    "Leo",
    "Virgo",
    "Libra",
    "Escorpio",
    "Sagitario",
    "Capricornio",
    "Acuario",
    "Piscis",
];

fn aspect_unicode(kind: &str) -> &'static str {
    match kind {
        "conjunction" => "☌",
        "opposition" => "☍",
        "trine" => "△",
        "square" => "□",
        "sextile" => "⚹",
        "quincunx" => "⚻",
        "semi_sextile" => "⚺",
        "semi_square" => "∠",
        "sesquiquadrate" => "⚼",
        "quintile" => "Q",
        "biquintile" => "bQ",
        _ => "·",
    }
}

fn planet_unicode(name: &str) -> &'static str {
    match name {
        "sun" => "☉",
        "moon" => "☽",
        "mercury" => "☿",
        "venus" => "♀",
        "mars" => "♂",
        "jupiter" => "♃",
        "saturn" => "♄",
        "uranus" => "♅",
        "neptune" => "♆",
        "pluto" => "♇",
        "north_node" => "☊",
        "south_node" => "☋",
        "chiron" => "⚷",
        "lilith" => "⚸",
        "ceres" => "⚳",
        "pallas" => "⚴",
        "juno" => "⚵",
        "vesta" => "⚶",
        _ => "•",
    }
}

fn planet_color(p: &AstroPalette, name: &str) -> Hsla {
    let planet = match name {
        "sun" => Planet::Sun,
        "moon" => Planet::Moon,
        "mercury" => Planet::Mercury,
        "venus" => Planet::Venus,
        "mars" => Planet::Mars,
        "jupiter" => Planet::Jupiter,
        "saturn" => Planet::Saturn,
        "uranus" => Planet::Uranus,
        "neptune" => Planet::Neptune,
        "pluto" => Planet::Pluto,
        "chiron" => Planet::Chiron,
        "north_node" => Planet::NorthNode,
        "south_node" => Planet::SouthNode,
        "lilith" => Planet::Lilith,
        _ => return p.fg_text_fallback(),
    };
    p.planet(planet)
}

fn sign_element_by_index(i: usize) -> Element {
    match i % 4 {
        0 => Element::Fire,
        1 => Element::Earth,
        2 => Element::Air,
        _ => Element::Water,
    }
}

fn element_color_for_sign(p: &AstroPalette, name: &str) -> Hsla {
    let elem = match name {
        "aries" | "leo" | "sagittarius" => Element::Fire,
        "taurus" | "virgo" | "capricorn" => Element::Earth,
        "gemini" | "libra" | "aquarius" => Element::Air,
        "cancer" | "scorpio" | "pisces" => Element::Water,
        _ => return p.fg_text_fallback(),
    };
    p.element(elem)
}

fn aspect_color(p: &AstroPalette, kind: &str) -> Hsla {
    let k = match kind {
        "conjunction" => TAspectKind::Conjunction,
        "opposition" => TAspectKind::Opposition,
        "trine" => TAspectKind::Trine,
        "square" => TAspectKind::Square,
        "sextile" => TAspectKind::Sextile,
        "quincunx" => TAspectKind::Quincunx,
        "semi_sextile" => TAspectKind::Semisextile,
        "semi_square" => TAspectKind::Semisquare,
        "sesquiquadrate" => TAspectKind::Sesquisquare,
        "quintile" => TAspectKind::Quintile,
        "biquintile" => TAspectKind::Biquintile,
        _ => return p.minor_aspect,
    };
    p.aspect(k)
}

trait AstroPaletteExt {
    fn fg_text_fallback(&self) -> Hsla;
}

impl AstroPaletteExt for AstroPalette {
    fn fg_text_fallback(&self) -> Hsla {
        if self.is_dark {
            hsla(0.0, 0.0, 0.85, 1.0)
        } else {
            hsla(0.0, 0.0, 0.25, 1.0)
        }
    }
}
