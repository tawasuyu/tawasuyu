//! Mensajes del bucle Elm, estado de drag, y hit-testing de los dots/líneas
//! de automación. Lo que el binario agrega sobre la lógica editable del lib.

use llimphi_ui::DragPhase;
use takiy_app::EditMsg;
use takiy_core::{AutomationLane, Track};

/// Distancia en píxeles al borde derecho de la nota dentro de la que un
/// press dispara drag-to-resize en lugar de drag-to-move. Pequeño
/// suficiente para no robarle clicks al cuerpo de la nota, grande
/// suficiente para acertarle con el mouse sin precisión quirúrgica.
pub(crate) const RESIZE_EDGE_PX: f32 = 6.0;

/// Radio de hit-test sobre los dots de automación. Un poco más grande
/// que el radio del dibujo (4 px) para que el target sea generoso.
const AUTO_DOT_HIT_RADIUS: f32 = 7.0;

/// Margen interno usado por `paint_automation_lane`. Reproducido acá
/// para que el hit-test coincida exactamente con el dibujo.
pub(crate) const AUTO_LANE_MARGIN_PX: f32 = 6.0;

/// Distancia vertical máxima entre cursor y la polilínea de una lane
/// para considerar que el click cayó sobre la curva (no sobre un dot
/// y no sobre el espacio vacío). Generoso para que sea fácil acertarle.
const AUTO_LINE_HIT_RADIUS_PX: f32 = 5.0;

#[derive(Clone)]
pub(crate) enum Msg {
    TogglePlay,
    /// Toca el score con un count-in de 1 compás (clicks pero sin notas).
    /// Útil para grabar a tempo desde el principio.
    PlayWithCountIn,
    /// Click sobre el header → posiciona el playhead. Si está sonando
    /// salta in-place; si no, arranca desde ese beat.
    SeekToBeat(f32),
    /// Tick periódico para refrescar el estado de playback. El cursor se
    /// pinta del `Player::position_samples()` (sample-accurate, ver F0.2).
    Tick,
    /// Edición pura — se delega a `EditorState::apply`.
    Edit(EditMsg),
    /// Toggle metrónomo (off ↔ 4/4).
    ToggleMetronome,
    /// Toggle loop. Si no hay región activa, define una de 4 compases
    /// desde el playhead (o desde beat 0). Si hay, la apaga.
    ToggleLoop,
    /// Cicla el snap de edición (Beat → Half → Quarter → Eighth → Triplet → Free).
    CycleSnap,
    /// Deshace la última edición.
    Undo,
    /// Rehace la última edición deshecha.
    Redo,
    /// Paste al playhead actual (en beats). El binario es quien lee la
    /// posición del Player y dispara el EditMsg::PasteAt correspondiente.
    PasteAtPlayhead,
    /// Cambia el programa GM de la pista activa en `delta` (wrap 0..=127).
    NudgeProgram { delta: i32 },
    /// Ancla un punto de automación de volumen en el beat actual (de la
    /// nota seleccionada, o el playhead, o 0). El valor anclado es el
    /// volumen efectivo de la pista activa.
    AnchorVolumeAutomation,
    /// Ancla un punto de automación de pan, mismo criterio.
    AnchorPanAutomation,
    /// Guarda el score actual a `TAKIY_SCORE_JSON` (o a `/tmp/...`).
    Save,
    /// Exporta el score a `<save_path>.mid` (o `/tmp/takiy_<unix>.mid`).
    ExportMidi,
    /// Render offline a WAV (44.1 kHz / estéreo PCM 16-bit). Path análogo
    /// a `ExportMidi` pero con extensión `.wav`. No incluye metrónomo ni
    /// count-in — sale crudo el score, igual que el render del test F10.
    ExportWav,
    /// Press del botón izquierdo: hace el hit-test sobre header/dot de
    /// automación/línea de automación/nota/cell y dispara la acción
    /// correspondiente (seek / arm-auto-drag / insert-auto-point / select
    /// / add note). Además cachea `(rw, rh)` en el modelo para que el
    /// drag posterior pueda convertir píxeles a `(beat, midi)` sin perderlo.
    PressAt { lx: f32, ly: f32, rw: f32, rh: f32 },
    /// Press del botón derecho. Borra el dot de automación bajo el cursor
    /// (si hay), o la nota bajo el cursor (idem F2.x). No tiene efecto si
    /// no acierta a nada.
    RightPressAt { lx: f32, ly: f32, rw: f32, rh: f32 },
    /// Eventos de drag-to-move o drag-to-resize sobre el grid. Se acumulan
    /// en `model.drag` y se aplican como `SetSelectedAbsolute` (move) o
    /// `SetSelectedDuration` (resize) sobre el `EditorState`. El modo se
    /// decide en la primera fase `Move` según si el press cayó cerca del
    /// borde derecho de la nota. El undo del drag entero queda como una
    /// sola entrada gracias a `begin_drag`/`end_drag`.
    DragNote {
        phase: DragPhase,
        dx: f32,
        dy: f32,
        lx0: f32,
        ly0: f32,
    },
    /// Wheel sobre el grid → mueve el `midi_offset` que desplaza la
    /// ventana de pitches visible. Positivo sube (pitches más agudos),
    /// negativo baja.
    ScrollMidi { delta: i32 },
    Quit,
}

/// Resultado del hit-test sobre dots de automación. Se almacena en
/// `Model.auto_pending` entre el press y el primer `Move` del drag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AutoHit {
    pub(crate) track_idx: usize,
    pub(crate) is_volume: bool,
    pub(crate) point_idx: usize,
    pub(crate) initial_beat: f32,
    pub(crate) initial_value: f32,
}

/// Hit-test sobre los dots de automación de una pista. Recorre
/// vol-lane primero (se dibuja primero) y devuelve el primer punto
/// dentro del radio `AUTO_DOT_HIT_RADIUS`. Coordenadas en local del
/// view raíz; mapeo de valor → y debe coincidir con
/// `paint_automation_lane`.
pub(crate) fn hit_test_automation_dot(
    track: &Track,
    track_idx: usize,
    lx: f32,
    ly: f32,
    grid_x: f32,
    grid_y: f32,
    grid_h: f32,
    beat_w: f32,
) -> Option<AutoHit> {
    let usable_h = (grid_h - AUTO_LANE_MARGIN_PX * 2.0).max(1.0);
    let r2 = AUTO_DOT_HIT_RADIUS * AUTO_DOT_HIT_RADIUS;
    let lanes: [(bool, Option<&AutomationLane>, f32, f32); 2] = [
        (true, track.volume_automation.as_ref(), 0.0, 1.5),
        (false, track.pan_automation.as_ref(), -1.0, 1.0),
    ];
    for (is_volume, lane, v_min, v_max) in lanes {
        let Some(lane) = lane else { continue };
        for (point_idx, p) in lane.points.iter().enumerate() {
            let x = grid_x + p.beat * beat_w;
            let norm = ((p.value - v_min) / (v_max - v_min)).clamp(0.0, 1.0);
            let y = grid_y + AUTO_LANE_MARGIN_PX + (1.0 - norm) * usable_h;
            let dx = lx - x;
            let dy = ly - y;
            if dx * dx + dy * dy <= r2 {
                return Some(AutoHit {
                    track_idx,
                    is_volume,
                    point_idx,
                    initial_beat: p.beat,
                    initial_value: p.value,
                });
            }
        }
    }
    None
}

/// Hit-test sobre la polilínea de una lane: si la `value_at(beat)` de
/// la curva proyecta a una y dentro de `AUTO_LINE_HIT_RADIUS_PX` del
/// cursor, devuelve `(is_volume, beat, value)`. Recorre vol primero
/// (igual orden que el painter); si ambas pasan el filtro, gana vol.
/// Sólo dispara dentro del grid (no sobre teclado/header).
pub(crate) fn hit_test_automation_line(
    track: &Track,
    lx: f32,
    ly: f32,
    grid_x: f32,
    grid_y: f32,
    grid_w: f32,
    grid_h: f32,
    beat_w: f32,
) -> Option<(bool, f32, f32)> {
    if lx < grid_x || lx > grid_x + grid_w || ly < grid_y || ly > grid_y + grid_h {
        return None;
    }
    let usable_h = (grid_h - AUTO_LANE_MARGIN_PX * 2.0).max(1.0);
    let lanes: [(bool, Option<&AutomationLane>, f32, f32); 2] = [
        (true, track.volume_automation.as_ref(), 0.0, 1.5),
        (false, track.pan_automation.as_ref(), -1.0, 1.0),
    ];
    let beat = ((lx - grid_x) / beat_w).max(0.0);
    for (is_volume, lane, v_min, v_max) in lanes {
        let Some(lane) = lane else { continue };
        if lane.is_empty() {
            continue;
        }
        // Default = primer punto si beat < first.beat (mismo clamp que
        // `value_at` para que el hit-test coincida con el dibujo).
        let default = lane.points.first().map(|p| p.value).unwrap_or(0.0);
        let curve_value = lane.value_at(beat, default);
        let norm = ((curve_value - v_min) / (v_max - v_min)).clamp(0.0, 1.0);
        let curve_y = grid_y + AUTO_LANE_MARGIN_PX + (1.0 - norm) * usable_h;
        if (ly - curve_y).abs() <= AUTO_LINE_HIT_RADIUS_PX {
            return Some((is_volume, beat, curve_value));
        }
    }
    None
}

/// Modo del drag activo. Se decide al inicio (primer evento `Move`)
/// según dónde cayó el press:
/// - sobre un dot de automación → `Automation`
/// - en el borde derecho de una nota (≤ `RESIZE_EDGE_PX`) → `Resize`
/// - sobre el cuerpo de una nota → `Move`
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DragMode {
    Move,
    Resize,
    /// Mover un punto de la lane de automación de una pista concreta.
    /// Capturamos `track_idx` al press para que el cambio de pista
    /// activa durante el drag no rute el cambio a otra pista.
    Automation {
        is_volume: bool,
        point_idx: usize,
        track_idx: usize,
    },
}

/// Estado del drag-to-move, drag-to-resize o drag-de-automación activo.
/// Se inicializa en la primera fase `DragPhase::Move` (cuando el cursor
/// se movió tras presionar sobre una nota o un dot), persiste hasta
/// `DragPhase::End`. Captura los valores originales para que cada
/// frame se compute en absoluto respecto del press, sin drift.
#[derive(Debug, Clone)]
pub(crate) struct DragState {
    pub(crate) mode: DragMode,
    /// Para `Move`/`Resize`: beat inicial de la nota. Para
    /// `Automation`: beat inicial del punto.
    pub(crate) initial_start: f32,
    /// `Move` only — MIDI inicial.
    pub(crate) initial_midi: u8,
    /// `Resize` only — duración inicial.
    pub(crate) initial_duration: f32,
    /// `Automation` only — valor inicial del punto.
    pub(crate) initial_value: f32,
    pub(crate) accum_dx_px: f32,
    pub(crate) accum_dy_px: f32,
    pub(crate) rw: f32,
    pub(crate) rh: f32,
    pub(crate) min_midi: u8,
    pub(crate) max_midi: u8,
    pub(crate) total_beats: f32,
}
