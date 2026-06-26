//! `takiy-editor-core` — estado editable del piano roll (Score + selección +
//! pista activa).
//!
//! Es la lógica pura del editor: cero audio, cero UI. Cualquier frontend
//! (el binario Llimphi, una CLI, web) lo consume y le manda [`EditMsg`]s; el
//! example `smoke` de `takiy-app-llimphi` lo ejerce headless en CI.
//!
//! El crate está partido en: este archivo (Snap + EditorState + EditMsg +
//! constructores), [`apply`] (el motor `apply` con undo/redo y cada
//! operación de edición), [`describe`] (helpers de presentación y modos de
//! tonalidad) y `tests`.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use takiy_core::{Score, ScoreNote, Track};

mod apply;
mod describe;
#[cfg(test)]
mod tests;

pub use describe::{
    describe_key, describe_master_delay, describe_master_reverb, describe_track_automation,
    find_note_idx,
};

/// Granularidad de snap para edición. Determina cuánto se redondea el
/// beat al hacer click, mover con flechas o pegar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Snap {
    /// Sin redondeo. El beat queda exacto al cursor (puede ser irracional).
    Free,
    /// Pulso entero.
    Beat,
    /// Mitad de pulso.
    Half,
    /// Cuarto de pulso (corchea en 4/4).
    Quarter,
    /// Octavo de pulso (semicorchea).
    Eighth,
    /// Tresillo de corchea = 1/3 de pulso.
    Triplet8,
}

impl Snap {
    /// Tamaño del paso en beats. `None` para `Free`.
    pub fn step(self) -> Option<f32> {
        match self {
            Snap::Free => None,
            Snap::Beat => Some(1.0),
            Snap::Half => Some(0.5),
            Snap::Quarter => Some(0.25),
            Snap::Eighth => Some(0.125),
            Snap::Triplet8 => Some(1.0 / 3.0),
        }
    }

    /// Redondea `beat` al múltiplo más cercano del paso. Si es `Free`,
    /// lo devuelve sin cambios.
    pub fn snap(self, beat: f32) -> f32 {
        match self.step() {
            None => beat,
            Some(s) => (beat / s).round() * s,
        }
    }

    /// Cicla en orden creciente de granularidad fina → vuelve a `Free`.
    pub fn cycle(self) -> Self {
        match self {
            Snap::Free => Snap::Beat,
            Snap::Beat => Snap::Half,
            Snap::Half => Snap::Quarter,
            Snap::Quarter => Snap::Eighth,
            Snap::Eighth => Snap::Triplet8,
            Snap::Triplet8 => Snap::Free,
        }
    }

    /// Etiqueta corta para el header.
    pub fn label(self) -> &'static str {
        match self {
            Snap::Free => "free",
            Snap::Beat => "1/1",
            Snap::Half => "1/2",
            Snap::Quarter => "1/4",
            Snap::Eighth => "1/8",
            Snap::Triplet8 => "1/8t",
        }
    }
}

impl Default for Snap {
    fn default() -> Self {
        Snap::Beat
    }
}

/// Estado mutable de edición. Lo demás (Player, SF2, theme, status) vive
/// en `Model` del binario.
#[derive(Debug, Clone)]
pub struct EditorState {
    pub score: Score,
    pub active_track: usize,
    pub next_track_n: usize,
    pub selected: Option<(usize, usize)>,
    pub save_path: Option<PathBuf>,
    /// Región de loop activa en beats `[from, to)`. Cuando es `Some` el
    /// playback rebobina al alcanzar `to`. Sólo afecta a la próxima
    /// reproducción — al cambiar en vivo, el binario reenvía un play
    /// nuevo para que el callback la respete.
    pub loop_region: Option<(f32, f32)>,
    /// Compases del metrónomo (`beats_per_bar`). `None` = metrónomo off.
    /// `Some(4)` = clicks en 4/4, etc.
    pub metronome_beats_per_bar: Option<u8>,
    /// Granularidad de snap. Default `Snap::Beat` (compat con F0/F1).
    pub snap: Snap,
    /// Si está `true` y hay [`Score::key`] activa, al agregar/mover una
    /// nota el midi se redondea al pitch en escala más cercano (add y
    /// drag absoluto) o salta por grados de escala (move relativo). Sin
    /// key activa, el flag no tiene efecto — el cromático sigue siendo
    /// el default. Tecla Alt+K en el binario.
    pub snap_to_key: bool,
    /// Stack de undo — snapshots de `score` antes de cada edición que mutó.
    /// Limitado a [`MAX_UNDO`] niveles; el más viejo se descarta. Expuesto
    /// para que el binario pueda mostrar la profundidad en el header.
    pub history: Vec<Score>,
    /// Stack de redo — snapshots a re-aplicar. Se vacía con cualquier
    /// edición nueva (rama futura abandonada).
    pub future: Vec<Score>,
    /// Clipboard interno: notas normalizadas (start - min_start ≥ 0) listas
    /// para pegarse en cualquier beat preservando intervalos relativos.
    pub clipboard: Vec<ScoreNote>,
    /// Snapshot tomado al iniciar un drag. Mientras está `Some`, [`apply`]
    /// no agrega entradas a `history` — todo el drag será un sólo undo,
    /// commiteado al cerrar el drag con [`end_drag`].
    pub(crate) drag_snapshot: Option<Score>,
}

/// Niveles máximos del undo stack. 100 cubre flujos típicos; cada
/// snapshot son ~50 bytes/nota → 5MB max para una pieza de 1000 notas.
pub const MAX_UNDO: usize = 100;

/// Resultado de aplicar un `EditMsg`: mensaje corto para el header.
/// `None` cuando la acción fue no-op (índice inválido, sin selección,
/// clamp sin cambio, etc.). El binario lo usa para repintar el status.
pub type ApplyOutcome = Option<String>;

impl EditorState {
    /// Crea un estado vacío con una pista por default — garantizamos que
    /// el primer click izquierdo tiene dónde aterrizar.
    pub fn new(tempo_bpm: f32) -> Self {
        let mut score = Score::new(tempo_bpm);
        score.add_track(Track::new("track 1"));
        Self {
            score,
            active_track: 0,
            next_track_n: 2,
            selected: None,
            save_path: None,
            loop_region: None,
            metronome_beats_per_bar: None,
            snap: Snap::default(),
            snap_to_key: false,
            history: Vec::new(),
            future: Vec::new(),
            clipboard: Vec::new(),
            drag_snapshot: None,
        }
    }

    /// Envuelve un `Score` ya hecho. Si está vacío, crea una pista.
    pub fn with_score(mut score: Score) -> Self {
        if score.tracks().is_empty() {
            score.add_track(Track::new("track 1"));
        }
        let n = score.tracks().len();
        Self {
            score,
            active_track: 0,
            next_track_n: n + 1,
            selected: None,
            save_path: None,
            loop_region: None,
            metronome_beats_per_bar: None,
            snap: Snap::default(),
            snap_to_key: false,
            history: Vec::new(),
            future: Vec::new(),
            clipboard: Vec::new(),
            drag_snapshot: None,
        }
    }

    /// Toggle del metrónomo a 4/4 (lo más común). Si está en otro
    /// compás, lo apaga; si está apagado, lo prende en 4/4.
    pub fn toggle_metronome(&mut self) -> ApplyOutcome {
        self.metronome_beats_per_bar = match self.metronome_beats_per_bar {
            None => Some(4),
            Some(_) => None,
        };
        Some(match self.metronome_beats_per_bar {
            Some(b) => format!("metrónomo on · {b}/4"),
            None => "metrónomo off".into(),
        })
    }

    /// Define una región de loop en beats. `set_loop_region(None)` la
    /// desactiva. Se valida `from < to`; si no, devuelve `None`.
    pub fn set_loop_region(&mut self, region: Option<(f32, f32)>) -> ApplyOutcome {
        match region {
            Some((from, to)) if from < to && from >= 0.0 => {
                self.loop_region = Some((from, to));
                Some(format!("loop · [{from:.1}, {to:.1})"))
            }
            None => {
                self.loop_region = None;
                Some("loop off".into())
            }
            _ => None,
        }
    }
}

/// Mensajes que mutan el estado editable. Subconjunto de `Msg` del
/// binario — sólo las acciones que se pueden testear sin Handle/Player.
#[derive(Debug, Clone, PartialEq)]
pub enum EditMsg {
    AddNote { beat: f32, midi: u8 },
    DeleteNote { track: usize, idx: usize },
    Select { track: usize, idx: usize },
    /// Hace activa la pista `track` (click sobre su fila en el mixer).
    /// No-op si el índice no existe. No toca la selección de notas.
    SetActiveTrack { track: usize },
    /// Suma `delta` al volumen de la pista `track` (clamp `[0, 1.5]`).
    /// Versión por-índice de [`EditMsg::NudgeActiveVolume`] para los
    /// faders del mixer, que operan sobre cualquier pista sin requerir
    /// que sea la activa.
    NudgeTrackVolume { track: usize, delta: f32 },
    /// Suma `delta` al pan de la pista `track` (clamp `[-1, 1]`).
    NudgeTrackPan { track: usize, delta: f32 },
    /// Toggle mute de la pista `track` (versión por-índice).
    ToggleMuteTrack { track: usize },
    /// Toggle solo de la pista `track` (versión por-índice).
    ToggleSoloTrack { track: usize },
    /// Fija la granularidad de snap (segmented del panel de tonalidad).
    SetSnap { snap: Snap },
    /// Fija el `time_beats` del delay master por índice de preset
    /// (`[1/8, 1/4, 1/4·, 1/8·, 1/16]`). No-op si el delay está apagado.
    SetMasterDelayTime { idx: usize },
    /// Fija el `room_size` del reverb master por índice de preset
    /// (`[cuarto, sala, catedral]`). No-op si el reverb está apagado.
    SetMasterReverbRoom { idx: usize },
    MoveSelected { d_beat: f32, d_semitones: i32 },
    /// Posiciona la nota seleccionada en `(start, midi)` absolutos
    /// (snap se aplica al `start`). Idempotente: si el snap no la mueve,
    /// es no-op. Pensado para drag-to-move por mouse — el binario
    /// recalcula la posición target en cada frame del drag.
    SetSelectedAbsolute { start: f32, midi: u8 },
    /// Setea la duración de la nota seleccionada en absoluto. Snap aplica;
    /// clamp a `[0.125, 16.0]` (mínimo razonable, máximo del editor).
    /// Pensado para drag-to-resize por el borde derecho.
    SetSelectedDuration { duration: f32 },
    DeleteSelected,
    ResizeSelected { d_beat: f32 },
    NudgeVelocity { delta: i32 },
    NudgeTempo { delta: f32 },
    CycleTrack,
    NewTrack,
    DeleteActiveTrack,
    /// Copia la nota seleccionada al clipboard interno.
    CopySelected,
    /// Copia + borra.
    CutSelected,
    /// Pega el contenido del clipboard al `beat` indicado en la pista
    /// activa, preservando offsets relativos. Sin clipboard, no-op.
    PasteAt { beat: f32 },
    /// Duplica la selección al final del compás siguiente.
    DuplicateSelected,
    /// Toggle mute de la pista activa.
    ToggleMuteActive,
    /// Toggle solo de la pista activa.
    ToggleSoloActive,
    /// Cambia el volumen de la pista activa en `delta` (clamp [0, 1.5]).
    NudgeActiveVolume { delta: f32 },
    /// Cambia el pan de la pista activa en `delta` (clamp [-1, 1]).
    NudgeActivePan { delta: f32 },
    /// Cicla la raíz de la tonalidad: None → C → C# → D … → B → None.
    /// Mantiene el modo previo (mayor por default).
    CycleKeyRoot,
    /// Cicla el modo entre los soportados: mayor → menor natural →
    /// pentatónica mayor → vuelta a mayor. Sólo aplica si hay key activa.
    CycleKeyMode,
    /// Prende/apaga el delay master. Al prenderse arranca con
    /// `DelayParams::default()` (0.5 beats / fb 0.35 / mix 0.25).
    ToggleMasterDelay,
    /// Cicla el `time_beats` del delay master por presets musicales:
    /// 1/8 (0.5) → 1/4 (1.0) → 1/4-puntillo (1.5) → 1/8-puntillo (0.75)
    /// → 1/16 (0.25) → vuelta a 1/8. No-op si el delay está apagado.
    CycleMasterDelayTime,
    /// Prende/apaga el reverb master. Al prenderse arranca con
    /// `ReverbParams::default()` (room 0.5 / damping 0.5 / mix 0.25).
    ToggleMasterReverb,
    /// Cicla el `room_size` del reverb master por presets espaciales:
    /// cuarto (0.25) → sala (0.5) → catedral (0.85) → vuelta a cuarto.
    /// No-op si el reverb está apagado.
    CycleMasterReverbRoom,
    /// Ancla un punto en la automación de volumen de la pista activa.
    /// El `beat` viene del binario (típicamente el beat de la nota
    /// seleccionada o el playhead). El `value` es el volumen efectivo
    /// actual de la pista — anclar congela ese valor en ese beat.
    AddVolumeAutomationPoint { beat: f32 },
    /// Ancla un punto en la automación de pan, mismo criterio que
    /// `AddVolumeAutomationPoint`.
    AddPanAutomationPoint { beat: f32 },
    /// Borra ambas curvas de automación de la pista activa (vol + pan).
    /// Útil para volver al valor estático sin escribir un script.
    ClearActiveAutomation,
    /// Inserta un punto de automación en `(beat, value)` en la pista
    /// indicada. Si el `beat` ya existe en la lane, sobrescribe el
    /// valor (no duplica). Pensado para la gesture "click sobre la
    /// curva" — el binario calcula `value` evaluando la curva actual
    /// en `beat` para que el punto insertado no genere un salto.
    InsertAutomationPoint {
        track_idx: usize,
        is_volume: bool,
        beat: f32,
        value: f32,
    },
    /// Borra el punto en `idx` de la lane indicada. No-op si `idx`
    /// queda fuera de rango. Si la lane queda vacía tras el borrado,
    /// la `Option` se setea a `None` (limpia el flag de "automatizada"
    /// en la pista) para evitar lanes-fantasma sin puntos.
    DeleteAutomationPoint {
        track_idx: usize,
        is_volume: bool,
        idx: usize,
    },
    /// Mueve un punto de la lane de automación a `(beat, value)`. La
    /// `lane` indica volumen o pan (vía `is_volume`); `track_idx` fija
    /// la pista (no usa `active_track` porque el drag arranca con un
    /// track que puede no ser el activo al terminar). Si `idx` queda
    /// fuera de rango, no-op. Clampea `beat` entre los vecinos para
    /// no romper el orden de la lane; clampea `value` al rango del
    /// parámetro (vol `[0, 1.5]`, pan `[-1, 1]`).
    SetAutomationPoint {
        track_idx: usize,
        is_volume: bool,
        idx: usize,
        beat: f32,
        value: f32,
    },
    /// Prende/apaga el snap a la tonalidad activa. Idempotente como
    /// toggle; sin `Score::key` declarada el flag queda igual setteado
    /// pero no tiene efecto hasta que se asigne una key.
    ToggleSnapToKey,
    /// Agrega una op de edición de onda a una pista, sobre `[from, to)`
    /// en beats. La capa `WaveLayer` se crea si no existía. Es no
    /// destructiva (modula una envolvente de ganancia, no toca samples).
    WaveOp { track: usize, op: takiy_core::WaveOp },
    /// Borra todas las ops de onda de una pista (`wave = None`).
    WaveClear { track: usize },
}
