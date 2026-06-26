//! Panorama de pistas — la vista tipo **Audacity** con la que se abre un
//! proyecto: una lista vertical de carriles horizontales, uno por pista.
//!
//! Cada carril muestra su contenido a lo largo de la línea de tiempo
//! compartida (en beats), en uno de dos modos (`TrackView`):
//!
//! - **midi** → una tira de notas (piano roll en miniatura).
//! - **onda** → la forma de onda del audio que la pista sintetiza
//!   (picos pre-calculados en `compute_onda_peaks`, cacheados en el
//!   modelo — sintetizar es caro y no se hace por frame).
//!
//! Clickear el cuerpo de un carril (o su nombre) abre el editor de esa
//! pista —el piano roll de siempre— vía `Msg::OpenTrack`. El header de
//! cada carril trae además M/S y el conmutador midi↔onda.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::{PaintRect, View};
use llimphi_widget_button::{button_view, ButtonPalette};

use takiy_core::{Score, TrackView};
use takiy_app::EditMsg;

use crate::appmodel::Model;
use crate::msg::Msg;

/// Resolución fija del perfil de picos de onda. El painter remapea estos
/// buckets al ancho real del carril, así el cálculo no depende del tamaño
/// de ventana y se cachea una sola vez por pista.
pub(crate) const ONDA_PEAK_BUCKETS: usize = 1600;

/// Sample-rate barato para el render de la onda del panorama. La forma
/// se preserva de sobra a 16 kHz y se sintetiza en una fracción del
/// tiempo que a 44.1 kHz. Siempre por osciladores (sin SF2) — sólo
/// queremos la silueta, no el timbre.
const ONDA_RENDER_SR: u32 = 16_000;

/// Paleta por pista — idéntica a la del piano roll (`paint.rs`) para que
/// el color de una pista sea el mismo en el panorama y en su editor.
const PALETTE: [(u8, u8, u8); 6] = [
    (96, 174, 240),
    (240, 170, 90),
    (130, 220, 150),
    (220, 130, 200),
    (240, 220, 120),
    (180, 140, 240),
];

/// Alto mínimo de un carril (px). Por debajo de esto la onda/notas se
/// vuelven ilegibles; arriba, los carriles reparten el alto disponible.
const LANE_MIN_H: f32 = 56.0;
/// Ancho de la columna de header de cada carril (nombre + M/S + modo).
const LANE_HEADER_W: f32 = 196.0;

/// Calcula el perfil de picos de onda de una pista: sintetiza la pista
/// sola (sin mute/solo, sin SF2, a `ONDA_RENDER_SR`) y reduce el audio a
/// `ONDA_PEAK_BUCKETS` buckets de máximo absoluto, normalizados a `[0, 1]`.
/// Vacío/silencio → vector de ceros (el painter dibuja la línea central).
pub(crate) fn compute_onda_peaks(score: &Score, track_idx: usize) -> Vec<f32> {
    let Some(track) = score.track(track_idx) else {
        return vec![0.0; ONDA_PEAK_BUCKETS];
    };
    // Pista aislada y siempre audible — la onda se ve aunque esté muteada.
    let mut solo = Score::new(score.tempo_bpm.max(1.0));
    let mut t = track.clone();
    t.mute = false;
    t.solo = false;
    solo.add_track(t);

    let buf = crate::audio::render_score(&solo, None, ONDA_RENDER_SR);
    let frames = buf.frames();
    let mut peaks = vec![0.0f32; ONDA_PEAK_BUCKETS];
    if frames == 0 {
        return peaks;
    }
    let ch = buf.channels.max(1) as usize;
    for f in 0..frames {
        let bucket = (f * ONDA_PEAK_BUCKETS / frames).min(ONDA_PEAK_BUCKETS - 1);
        let mut a = 0.0f32;
        for c in 0..ch {
            a = a.max(buf.samples[f * ch + c].abs());
        }
        if a > peaks[bucket] {
            peaks[bucket] = a;
        }
    }
    let m = peaks.iter().copied().fold(0.0f32, f32::max);
    if m > 1e-6 {
        for p in &mut peaks {
            *p /= m;
        }
    }
    peaks
}

/// Cuerpo del panorama: la lista de carriles. Va bajo menubar + toolbar,
/// en el lugar que en `Screen::Track` ocupa el piano roll.
pub(crate) fn body(model: &Model, theme: &Theme) -> View<Msg> {
    let score = &model.editor.score;
    let total_beats = score.duration_beats().max(8.0);
    let active = model.editor.active_track;
    // Beat del playhead (compartido por todos los carriles) si está sonando.
    let playhead_beat = model
        .player
        .as_ref()
        .filter(|_| model.playing)
        .map(|p| p.position_seconds() * model.playback_bpm / 60.0);

    let mut lanes: Vec<View<Msg>> = Vec::new();
    for (i, track) in score.tracks().iter().enumerate() {
        let color = PALETTE[i % PALETTE.len()];
        let is_active = i == active;
        let header = lane_header(i, track, is_active, color, theme);
        let strip = lane_strip(
            i,
            track,
            color,
            total_beats,
            playhead_beat,
            model.onda_peaks.get(&i).cloned(),
            theme,
        );
        lanes.push(lane_row(header, strip, is_active, theme));
    }

    if lanes.is_empty() {
        lanes.push(
            row_box(LANE_MIN_H, theme.bg_panel).text_aligned(
                "proyecto sin pistas — usá «+ pista» en la barra".to_string(),
                13.0,
                theme.fg_muted,
                Alignment::Center,
            ),
        );
    }

    // Una pista más abajo: botón para agregar pista al proyecto.
    lanes.push(add_track_row(theme));

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(lanes)
}

/// Un carril: header (fijo) + tira de contenido (flex). `flex_grow` hace
/// que los carriles repartan el alto disponible (tipo Audacity), con un
/// piso de `LANE_MIN_H`.
fn lane_row(header: View<Msg>, strip: View<Msg>, is_active: bool, theme: &Theme) -> View<Msg> {
    let bg = if is_active { theme.bg_selected } else { theme.bg_panel };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        size: Size { width: percent(1.0_f32), height: percent(0.0_f32) },
        min_size: Size { width: length(0.0_f32), height: length(LANE_MIN_H) },
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(5.0_f32),
            bottom: length(5.0_f32),
        },
        align_items: Some(AlignItems::Stretch),
        ..Default::default()
    })
    .fill(bg)
    .children(vec![header, strip])
}

/// Header del carril: nombre clickable (abre el editor), fila M/S y el
/// conmutador midi↔onda.
fn lane_header(
    i: usize,
    track: &takiy_core::Track,
    is_active: bool,
    color: (u8, u8, u8),
    theme: &Theme,
) -> View<Msg> {
    let name_label = if is_active {
        format!("▶ {}", track.name)
    } else {
        format!("  {}", track.name)
    };
    let mut name_pal = ButtonPalette::from_theme(theme);
    name_pal.fg = Color::from_rgba8(color.0, color.1, color.2, 255);
    let name_btn = grow_box(26.0).children(vec![button_view(
        name_label,
        &name_pal,
        Msg::OpenTrack(i),
    )]);

    let mute = mini(
        "M",
        30.0,
        track.mute,
        theme,
        Msg::Edit(EditMsg::ToggleMuteTrack { track: i }),
    );
    let solo = mini(
        "S",
        30.0,
        track.solo,
        theme,
        Msg::Edit(EditMsg::ToggleSoloTrack { track: i }),
    );
    // Conmutador de modo: muestra el modo actual; clickear alterna.
    let toggle = mini(
        track.view.label(),
        58.0,
        track.view == TrackView::Onda,
        theme,
        Msg::SetTrackView { track: i, view: track.view.toggled() },
    );
    let controls = hrow(28.0, 4.0, vec![mute, solo, toggle]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(LANE_HEADER_W), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        ..Default::default()
    })
    .children(vec![name_btn, controls])
}

/// Tira de contenido del carril: un `paint_with` que dibuja notas (midi) o
/// la onda, más la línea de tiempo y el playhead. Click → abre el editor.
fn lane_strip(
    i: usize,
    track: &takiy_core::Track,
    color: (u8, u8, u8),
    total_beats: f32,
    playhead_beat: Option<f32>,
    peaks: Option<Vec<f32>>,
    theme: &Theme,
) -> View<Msg> {
    let view_mode = track.view;
    // Datos a capturar para el painter (evitamos referenciar `track`).
    let notes: Vec<(u8, f32, f32)> = track
        .notes()
        .iter()
        .map(|n| (n.pitch.midi(), n.start, n.duration))
        .collect();
    let (mut lo, mut hi) = (127u8, 0u8);
    for &(m, _, _) in &notes {
        lo = lo.min(m);
        hi = hi.max(m);
    }
    let strip_bg = theme.bg_panel_alt;

    View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
        min_size: Size { width: length(0.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(strip_bg)
    .on_click_at(move |_lx, _ly, _rw, _rh| Some(Msg::OpenTrack(i)))
    .paint_with(move |scene, _ts, rect: PaintRect| {
        paint_strip(
            scene, rect, view_mode, &notes, (lo, hi), peaks.as_deref(), total_beats,
            playhead_beat, color, strip_bg,
        );
    })
}

/// Dibuja el contenido de una tira (notas u onda) + línea de tiempo +
/// playhead. Coordenadas en el `rect` que llimphi le asigna al nodo.
#[allow(clippy::too_many_arguments)]
fn paint_strip(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: PaintRect,
    view_mode: TrackView,
    notes: &[(u8, f32, f32)],
    pitch_range: (u8, u8),
    peaks: Option<&[f32]>,
    total_beats: f32,
    playhead_beat: Option<f32>,
    color: (u8, u8, u8),
    bg: Color,
) {
    if rect.w <= 1.0 || rect.h <= 1.0 {
        return;
    }
    let _ = bg;
    let beat_w = (rect.w / total_beats.max(1.0)).max(0.01);
    let col = Color::from_rgba8(color.0, color.1, color.2, 240);

    // Líneas de compás cada 4 beats (tenues) — orientan la lectura.
    let bar = Color::from_rgba8(110, 112, 130, 90);
    let bars = (total_beats.ceil() as u32).min(512);
    let mut b = 0u32;
    while b <= bars {
        if b % 4 == 0 {
            let x = rect.x + b as f32 * beat_w;
            if x > rect.x + rect.w {
                break;
            }
            let mut p = BezPath::new();
            p.move_to((x as f64, rect.y as f64));
            p.line_to((x as f64, (rect.y + rect.h) as f64));
            scene.stroke(&Stroke::new(0.6), Affine::IDENTITY, bar, None, &p);
        }
        b += 1;
    }

    match view_mode {
        TrackView::Midi => {
            // Mini piano roll: el rango de pitch de la pista se mapea al
            // alto del carril (con margen). Pista vacía → nada.
            let (lo, hi) = pitch_range;
            if hi < lo {
                // sin notas
            } else {
                let margin = 4.0_f32;
                let usable = (rect.h - margin * 2.0).max(2.0);
                let span = (hi - lo).max(1) as f32;
                let key_h = (usable / (span + 1.0)).clamp(1.5, 10.0);
                for &(m, start, dur) in notes {
                    let row = (hi - m) as f32;
                    let y = rect.y + margin + row / (span + 1.0) * usable;
                    let x = rect.x + start * beat_w;
                    let w = (dur * beat_w).max(1.5);
                    let r = KurboRect::new(
                        x as f64,
                        y as f64,
                        (x + w) as f64,
                        (y + key_h as f32) as f64,
                    );
                    scene.fill(Fill::NonZero, Affine::IDENTITY, col, None, &r);
                }
            }
        }
        TrackView::Onda => {
            let mid = rect.y + rect.h * 0.5;
            // Línea central de referencia.
            let axis = Color::from_rgba8(110, 112, 130, 120);
            let mut a = BezPath::new();
            a.move_to((rect.x as f64, mid as f64));
            a.line_to(((rect.x + rect.w) as f64, mid as f64));
            scene.stroke(&Stroke::new(0.6), Affine::IDENTITY, axis, None, &a);

            if let Some(peaks) = peaks {
                if !peaks.is_empty() {
                    let half = (rect.h * 0.5 - 2.0).max(1.0);
                    let n = rect.w.ceil() as usize;
                    // Relleno espejo arriba/abajo de la línea central.
                    let mut up = BezPath::new();
                    let mut down: Vec<(f64, f64)> = Vec::with_capacity(n + 1);
                    up.move_to((rect.x as f64, mid as f64));
                    for px in 0..=n {
                        let bucket = (px * peaks.len() / n.max(1)).min(peaks.len() - 1);
                        let amp = peaks[bucket] * half;
                        let x = (rect.x + px as f32) as f64;
                        up.line_to((x, (mid - amp) as f64));
                        down.push((x, (mid + amp) as f64));
                    }
                    for &(x, y) in down.iter().rev() {
                        up.line_to((x, y));
                    }
                    up.close_path();
                    let fill = Color::from_rgba8(color.0, color.1, color.2, 150);
                    scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &up);
                }
            }
        }
    }

    // Playhead compartido.
    if let Some(beat) = playhead_beat {
        let x = rect.x + beat * beat_w;
        if x >= rect.x && x <= rect.x + rect.w {
            let cur = Color::from_rgba8(255, 240, 120, 230);
            let mut p = BezPath::new();
            p.move_to((x as f64, rect.y as f64));
            p.line_to((x as f64, (rect.y + rect.h) as f64));
            scene.stroke(&Stroke::new(1.6), Affine::IDENTITY, cur, None, &p);
        }
    }
}

/// Fila final con el botón de «+ pista».
fn add_track_row(theme: &Theme) -> View<Msg> {
    let pal = ButtonPalette::from_theme(theme);
    let btn = View::new(Style {
        size: Size { width: length(180.0_f32), height: length(30.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![button_view(
        "+ pista nueva",
        &pal,
        Msg::Edit(EditMsg::NewTrack),
    )]);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(38.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(vec![btn])
}

// ---- helpers de layout -------------------------------------------------

/// Botón compacto de ancho fijo; resalta `active` con el acento.
fn mini(label: &str, w: f32, active: bool, theme: &Theme, msg: Msg) -> View<Msg> {
    let mut pal = ButtonPalette::from_theme(theme);
    if active {
        pal.bg = theme.accent;
        pal.bg_hover = theme.accent;
        pal.fg = theme.bg_app;
    }
    View::new(Style {
        size: Size { width: length(w), height: length(26.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![button_view(label, &pal, msg)])
}

/// Caja que crece a lo ancho con alto fijo.
fn grow_box(h: f32) -> View<Msg> {
    View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: length(h) },
        ..Default::default()
    })
}

/// Caja full-width de alto fijo con fondo.
fn row_box(h: f32, fill: Color) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(h) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(fill)
}

/// Fila horizontal de hijos, alto fijo, con gap.
fn hrow(h: f32, gap: f32, kids: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(h) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(gap), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(kids)
}
