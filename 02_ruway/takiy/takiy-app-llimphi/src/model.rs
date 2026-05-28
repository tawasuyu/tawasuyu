//! Estado editable del piano roll — Score + selección + pista activa.
//!
//! Es la lógica pura del editor: cero audio, cero UI. El binario Llimphi
//! le manda [`EditMsg`]s; el example `smoke` lo ejerce headless en CI.

use std::path::PathBuf;

use takiy_core::{
    AutomationLane, DelayParams, Pitch, PitchClass, ReverbParams, Scale, Score, ScoreNote, Track,
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
    drag_snapshot: Option<Score>,
}

/// Niveles máximos del undo stack. 100 cubre flujos típicos; cada
/// snapshot son ~50 bytes/nota → 5MB max para una pieza de 1000 notas.
pub const MAX_UNDO: usize = 100;

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
}

/// Resultado de aplicar un `EditMsg`: mensaje corto para el header.
/// `None` cuando la acción fue no-op (índice inválido, sin selección,
/// clamp sin cambio, etc.). El binario lo usa para repintar el status.
pub type ApplyOutcome = Option<String>;

impl EditorState {
    /// Aplica una edición. Envuelve `apply_internal` con la lógica de
    /// undo: snapshot pre-mutación, descarte si no cambió nada,
    /// truncado de `future` ante una rama nueva.
    ///
    /// Si hay un drag activo (`drag_snapshot.is_some()`), las
    /// mutaciones intermedias *no* generan entradas de historial — el
    /// commit de undo del drag entero lo hace [`end_drag`] una sola vez.
    pub fn apply(&mut self, msg: EditMsg) -> ApplyOutcome {
        if self.drag_snapshot.is_some() {
            return self.apply_internal(msg);
        }
        let snapshot = self.score.clone();
        let out = self.apply_internal(msg);
        if self.score != snapshot {
            self.history.push(snapshot);
            if self.history.len() > MAX_UNDO {
                self.history.remove(0);
            }
            self.future.clear();
        }
        out
    }

    /// Marca el inicio de un drag-to-move (u otro batch interactivo).
    /// Toma snapshot del `score` actual; mientras esté pendiente, las
    /// `apply` siguen mutando el score pero no pushean a `history`.
    /// Idempotente: llamadas repetidas no clavan un nuevo snapshot.
    pub fn begin_drag(&mut self) {
        if self.drag_snapshot.is_none() {
            self.drag_snapshot = Some(self.score.clone());
        }
    }

    /// Cierra un drag pendiente. Si el score cambió, pushea el snapshot
    /// original a `history` (un sólo undo cubre toda la interacción) y
    /// trunca `future`. Sin cambios, descarta el snapshot.
    pub fn end_drag(&mut self) -> ApplyOutcome {
        let Some(snapshot) = self.drag_snapshot.take() else {
            return None;
        };
        if snapshot != self.score {
            self.history.push(snapshot);
            if self.history.len() > MAX_UNDO {
                self.history.remove(0);
            }
            self.future.clear();
            Some("drag committed".into())
        } else {
            None
        }
    }

    /// `true` si hay un drag iniciado y aún no commiteado. El binario
    /// lo lee para decidir si pintar la nota como ghost o evitar
    /// disparar auto-save mientras se arrastra.
    pub fn is_dragging(&self) -> bool {
        self.drag_snapshot.is_some()
    }

    fn apply_internal(&mut self, msg: EditMsg) -> ApplyOutcome {
        match msg {
            EditMsg::AddNote { beat, midi } => self.add_note(beat, midi),
            EditMsg::DeleteNote { track, idx } => self.delete_note(track, idx),
            EditMsg::Select { track, idx } => self.select(track, idx),
            EditMsg::MoveSelected { d_beat, d_semitones } => {
                self.move_selected(d_beat, d_semitones)
            }
            EditMsg::SetSelectedAbsolute { start, midi } => {
                self.set_selected_absolute(start, midi)
            }
            EditMsg::SetSelectedDuration { duration } => {
                self.set_selected_duration(duration)
            }
            EditMsg::DeleteSelected => self.delete_selected(),
            EditMsg::ResizeSelected { d_beat } => self.resize_selected(d_beat),
            EditMsg::NudgeVelocity { delta } => self.nudge_velocity(delta),
            EditMsg::NudgeTempo { delta } => self.nudge_tempo(delta),
            EditMsg::CycleTrack => self.cycle_track(),
            EditMsg::NewTrack => self.new_track(),
            EditMsg::DeleteActiveTrack => self.delete_active_track(),
            EditMsg::CopySelected => self.copy_selected(),
            EditMsg::CutSelected => self.cut_selected(),
            EditMsg::PasteAt { beat } => self.paste_at(beat),
            EditMsg::DuplicateSelected => self.duplicate_selected(),
            EditMsg::ToggleMuteActive => self.toggle_mute_active(),
            EditMsg::ToggleSoloActive => self.toggle_solo_active(),
            EditMsg::NudgeActiveVolume { delta } => self.nudge_active_volume(delta),
            EditMsg::NudgeActivePan { delta } => self.nudge_active_pan(delta),
            EditMsg::CycleKeyRoot => self.cycle_key_root(),
            EditMsg::CycleKeyMode => self.cycle_key_mode(),
            EditMsg::ToggleMasterDelay => self.toggle_master_delay(),
            EditMsg::CycleMasterDelayTime => self.cycle_master_delay_time(),
            EditMsg::ToggleMasterReverb => self.toggle_master_reverb(),
            EditMsg::CycleMasterReverbRoom => self.cycle_master_reverb_room(),
            EditMsg::AddVolumeAutomationPoint { beat } => self.add_volume_automation_point(beat),
            EditMsg::AddPanAutomationPoint { beat } => self.add_pan_automation_point(beat),
            EditMsg::ClearActiveAutomation => self.clear_active_automation(),
        }
    }

    /// Cicla el snap a la próxima granularidad. Tecla Q en el binario.
    pub fn cycle_snap(&mut self) -> ApplyOutcome {
        self.snap = self.snap.cycle();
        Some(format!("snap · {}", self.snap.label()))
    }

    /// Deshace la última edición. No es contado en historial — undo de
    /// undo es redo. Devuelve `None` si la pila está vacía.
    pub fn undo(&mut self) -> ApplyOutcome {
        let prev = self.history.pop()?;
        let current = std::mem::replace(&mut self.score, prev);
        self.future.push(current);
        self.selected = None;
        Some("undo".into())
    }

    /// Rehace la última edición deshecha. Devuelve `None` si no hay nada.
    pub fn redo(&mut self) -> ApplyOutcome {
        let next = self.future.pop()?;
        let current = std::mem::replace(&mut self.score, next);
        self.history.push(current);
        self.selected = None;
        Some("redo".into())
    }

    fn add_note(&mut self, beat: f32, midi: u8) -> ApplyOutcome {
        let pitch = Pitch::from_midi(midi)?;
        let beat = self.snap.snap(beat).max(0.0);
        let track_idx = self.active_track.min(self.score.tracks().len().saturating_sub(1));
        let new_note = ScoreNote::new(pitch, beat, 1.0, 96);
        let track = self.score.track_mut(track_idx)?;
        track.add(new_note);
        if let Some(new_idx) = find_note_idx(track.notes(), &new_note) {
            self.selected = Some((track_idx, new_idx));
        }
        Some(format!("added · pista {track_idx} · beat {beat:.0} · midi {midi}"))
    }

    fn delete_note(&mut self, track: usize, idx: usize) -> ApplyOutcome {
        let t = self.score.track_mut(track)?;
        t.remove(idx)?;
        if let Some((sel_t, sel_i)) = self.selected {
            if sel_t == track {
                if sel_i == idx {
                    self.selected = None;
                } else if sel_i > idx {
                    self.selected = Some((sel_t, sel_i - 1));
                }
            }
        }
        Some(format!("deleted · pista {track} · nota #{idx}"))
    }

    fn select(&mut self, track: usize, idx: usize) -> ApplyOutcome {
        let exists = self
            .score
            .track(track)
            .is_some_and(|t| idx < t.notes().len());
        if !exists {
            return None;
        }
        self.selected = Some((track, idx));
        Some(format!("selected · pista {track} · nota #{idx}"))
    }

    fn move_selected(&mut self, d_beat: f32, d_semitones: i32) -> ApplyOutcome {
        let (track_idx, note_idx) = self.selected?;
        let snap = self.snap;
        let track = self.score.track_mut(track_idx)?;
        let old = track.notes().get(note_idx).copied()?;
        // Si hay snap activo, redondeamos el nuevo start al múltiplo
        // exacto — facilita encadenar moves sin acumular drift.
        let raw_start = old.start + d_beat;
        let new_start = snap.snap(raw_start);
        if new_start < 0.0 {
            return None;
        }
        let new_midi = old.pitch.midi() as i32 + d_semitones;
        let new_pitch = u8::try_from(new_midi).ok().and_then(Pitch::from_midi)?;
        let new_note = ScoreNote::new(new_pitch, new_start, old.duration, old.velocity);
        track.remove(note_idx);
        track.add(new_note);
        if let Some(new_idx) = find_note_idx(track.notes(), &new_note) {
            self.selected = Some((track_idx, new_idx));
        }
        Some(format!(
            "moved · pista {track_idx} · beat {new_start:.0} · midi {}",
            new_pitch.midi()
        ))
    }

    /// Posiciona la nota seleccionada en `(start, midi)` absolutos.
    /// `start` se snappea según el snap activo y se clampa a `≥ 0`;
    /// `midi` debe estar en `0..=127`. Si el resultado coincide con la
    /// nota actual (snap mata el cambio), es no-op.
    fn set_selected_absolute(&mut self, start: f32, midi: u8) -> ApplyOutcome {
        let (track_idx, note_idx) = self.selected?;
        let snap = self.snap;
        let track = self.score.track_mut(track_idx)?;
        let old = track.notes().get(note_idx).copied()?;
        let new_start = snap.snap(start).max(0.0);
        let new_pitch = Pitch::from_midi(midi)?;
        let new_note = ScoreNote::new(new_pitch, new_start, old.duration, old.velocity);
        if new_note == old {
            return None;
        }
        track.remove(note_idx);
        track.add(new_note);
        if let Some(new_idx) = find_note_idx(track.notes(), &new_note) {
            self.selected = Some((track_idx, new_idx));
        }
        Some(format!(
            "drag · pista {track_idx} · beat {new_start:.2} · midi {midi}"
        ))
    }

    /// Setea la duración absoluta de la nota seleccionada, snappeando si
    /// hay snap activo y clampeando a `[0.125, 16.0]`. Idempotente: si el
    /// resultado coincide con la nota actual, no-op.
    fn set_selected_duration(&mut self, duration: f32) -> ApplyOutcome {
        let (track_idx, note_idx) = self.selected?;
        let snap = self.snap;
        let track = self.score.track_mut(track_idx)?;
        let old = track.notes().get(note_idx).copied()?;
        let snapped = snap.snap(duration);
        let raw = if snap.step().is_some() { snapped } else { duration };
        let new_dur = raw.clamp(0.125, 16.0);
        if (new_dur - old.duration).abs() < f32::EPSILON {
            return None;
        }
        let new_note = ScoreNote::new(old.pitch, old.start, new_dur, old.velocity);
        track.remove(note_idx);
        track.add(new_note);
        if let Some(new_idx) = find_note_idx(track.notes(), &new_note) {
            self.selected = Some((track_idx, new_idx));
        }
        Some(format!("drag-resize · pista {track_idx} · dur {new_dur:.2}"))
    }

    fn delete_selected(&mut self) -> ApplyOutcome {
        let (track, idx) = self.selected.take()?;
        let t = self.score.track_mut(track)?;
        t.remove(idx)?;
        Some(format!("deleted · pista {track} · nota #{idx}"))
    }

    fn resize_selected(&mut self, d_beat: f32) -> ApplyOutcome {
        let (track_idx, note_idx) = self.selected?;
        let snap = self.snap;
        let track = self.score.track_mut(track_idx)?;
        let old = track.notes().get(note_idx).copied()?;
        // Snap también aplica a duración para que el resize caiga en
        // grilla. El step mínimo razonable es 0.25 (hardcoded clamp).
        let raw_dur = old.duration + d_beat;
        let snapped = snap.snap(raw_dur);
        let new_dur = if snap.step().is_some() { snapped } else { raw_dur }.clamp(0.25, 16.0);
        if (new_dur - old.duration).abs() < f32::EPSILON {
            return None;
        }
        let new_note = ScoreNote::new(old.pitch, old.start, new_dur, old.velocity);
        track.remove(note_idx);
        track.add(new_note);
        if let Some(new_idx) = find_note_idx(track.notes(), &new_note) {
            self.selected = Some((track_idx, new_idx));
        }
        Some(format!("resized · pista {track_idx} · dur {new_dur:.2}"))
    }

    fn nudge_velocity(&mut self, delta: i32) -> ApplyOutcome {
        let (track_idx, note_idx) = self.selected?;
        let track = self.score.track_mut(track_idx)?;
        let old = track.notes().get(note_idx).copied()?;
        let new_vel = (old.velocity as i32 + delta).clamp(1, 127) as u8;
        if new_vel == old.velocity {
            return None;
        }
        let new_note = ScoreNote::new(old.pitch, old.start, old.duration, new_vel);
        track.remove(note_idx);
        track.add(new_note);
        if let Some(new_idx) = find_note_idx(track.notes(), &new_note) {
            self.selected = Some((track_idx, new_idx));
        }
        Some(format!("vel {new_vel} · pista {track_idx}"))
    }

    fn nudge_tempo(&mut self, delta: f32) -> ApplyOutcome {
        let new_bpm = (self.score.tempo_bpm + delta).clamp(30.0, 300.0);
        if (new_bpm - self.score.tempo_bpm).abs() < f32::EPSILON {
            return None;
        }
        self.score.tempo_bpm = new_bpm;
        Some(format!("tempo {new_bpm:.0} bpm"))
    }

    fn cycle_track(&mut self) -> ApplyOutcome {
        let n = self.score.tracks().len().max(1);
        self.active_track = (self.active_track + 1) % n;
        let name = self
            .score
            .track(self.active_track)
            .map(|t| t.name.as_str())
            .unwrap_or("?");
        Some(format!("active · pista {} ({name})", self.active_track))
    }

    fn new_track(&mut self) -> ApplyOutcome {
        let name = format!("track {}", self.next_track_n);
        self.next_track_n += 1;
        let idx = self.score.add_track(Track::new(&name));
        self.active_track = idx;
        Some(format!("new · pista {idx} ({name})"))
    }

    fn copy_selected(&mut self) -> ApplyOutcome {
        let (track, idx) = self.selected?;
        let note = self.score.track(track)?.notes().get(idx).copied()?;
        // Normalizamos al start = 0 (single note → trivial).
        self.clipboard = vec![ScoreNote::new(note.pitch, 0.0, note.duration, note.velocity)];
        Some(format!("copy · 1 nota"))
    }

    fn cut_selected(&mut self) -> ApplyOutcome {
        self.copy_selected()?;
        self.delete_selected();
        Some("cut · 1 nota".into())
    }

    fn paste_at(&mut self, beat: f32) -> ApplyOutcome {
        if self.clipboard.is_empty() {
            return None;
        }
        let snap = self.snap;
        let track_idx = self.active_track.min(self.score.tracks().len().saturating_sub(1));
        let track = self.score.track_mut(track_idx)?;
        let base = snap.snap(beat).max(0.0);
        let clipboard = std::mem::take(&mut self.clipboard);
        let n = clipboard.len();
        let mut first_inserted = None;
        for note in &clipboard {
            let new_note = ScoreNote::new(
                note.pitch,
                (base + note.start).max(0.0),
                note.duration,
                note.velocity,
            );
            track.add(new_note);
            if first_inserted.is_none() {
                first_inserted = find_note_idx(track.notes(), &new_note);
            }
        }
        // Restituimos el clipboard — el paste no lo consume.
        self.clipboard = clipboard;
        if let Some(idx) = first_inserted {
            self.selected = Some((track_idx, idx));
        }
        Some(format!("paste · {n} nota(s) @ beat {base:.1}"))
    }

    fn duplicate_selected(&mut self) -> ApplyOutcome {
        let (track_idx, idx) = self.selected?;
        let note = self.score.track(track_idx)?.notes().get(idx).copied()?;
        // Duplica una copia justo después de la nota (start + duration),
        // independientemente del clipboard. Útil para repetir un motivo.
        let track = self.score.track_mut(track_idx)?;
        let new_note = ScoreNote::new(
            note.pitch,
            note.start + note.duration,
            note.duration,
            note.velocity,
        );
        track.add(new_note);
        if let Some(new_idx) = find_note_idx(track.notes(), &new_note) {
            self.selected = Some((track_idx, new_idx));
        }
        Some(format!("duplicate · pista {track_idx}"))
    }

    fn toggle_mute_active(&mut self) -> ApplyOutcome {
        let idx = self.active_track;
        let track = self.score.track_mut(idx)?;
        track.mute = !track.mute;
        let state = if track.mute { "on" } else { "off" };
        Some(format!("pista {idx} · mute {state}"))
    }

    fn toggle_solo_active(&mut self) -> ApplyOutcome {
        let idx = self.active_track;
        let track = self.score.track_mut(idx)?;
        track.solo = !track.solo;
        let state = if track.solo { "on" } else { "off" };
        Some(format!("pista {idx} · solo {state}"))
    }

    fn nudge_active_volume(&mut self, delta: f32) -> ApplyOutcome {
        let idx = self.active_track;
        let track = self.score.track_mut(idx)?;
        let new_vol = (track.volume + delta).clamp(0.0, 1.5);
        if (new_vol - track.volume).abs() < f32::EPSILON {
            return None;
        }
        track.volume = new_vol;
        Some(format!("pista {idx} · vol {new_vol:.2}"))
    }

    /// Cicla la raíz de la tonalidad en orden cromático. None → C → C# → ... → B → None.
    fn cycle_key_root(&mut self) -> ApplyOutcome {
        let new_key = match self.score.key.as_ref().map(|s| (s.root(), classify_mode(s))) {
            None => Some((PitchClass::C, KeyMode::Major)),
            Some((PitchClass::B, mode)) => {
                // Termina el ciclo cromático — apaga la key.
                let _ = mode;
                None
            }
            Some((root, mode)) => Some((next_pitch_class(root), mode)),
        };
        self.score.key = new_key.map(|(root, mode)| mode.scale(root));
        Some(format!("key · {}", describe_key(&self.score.key)))
    }

    /// Cicla el modo de la tonalidad activa. Si no hay key, la prende
    /// en C mayor — así una sola tecla puede arrancar la consciencia.
    fn cycle_key_mode(&mut self) -> ApplyOutcome {
        let (root, mode) = match self.score.key.as_ref() {
            Some(scale) => (scale.root(), classify_mode(scale).next()),
            None => (PitchClass::C, KeyMode::Major),
        };
        self.score.key = Some(mode.scale(root));
        Some(format!("key · {}", describe_key(&self.score.key)))
    }

    fn toggle_master_delay(&mut self) -> ApplyOutcome {
        self.score.master_delay = match self.score.master_delay {
            None => Some(DelayParams::default()),
            Some(_) => None,
        };
        Some(format!("delay · {}", describe_master_delay(&self.score.master_delay)))
    }

    fn cycle_master_delay_time(&mut self) -> ApplyOutcome {
        // Presets musicales en beats: 1/8, 1/4, 1/4-puntillo, 1/8-puntillo, 1/16.
        const PRESETS: [f32; 5] = [0.5, 1.0, 1.5, 0.75, 0.25];
        let Some(params) = self.score.master_delay.as_mut() else {
            return Some("delay off (no se puede ciclar tiempo)".into());
        };
        let idx = PRESETS
            .iter()
            .position(|t| (t - params.time_beats).abs() < 1e-3)
            .unwrap_or(0);
        params.time_beats = PRESETS[(idx + 1) % PRESETS.len()];
        Some(format!("delay · {}", describe_master_delay(&self.score.master_delay)))
    }

    fn add_volume_automation_point(&mut self, beat: f32) -> ApplyOutcome {
        let idx = self.active_track;
        let track = self.score.track_mut(idx)?;
        // Usamos el `volume` estático (no `volume_at`) para que el
        // workflow sea: "ajustá el static con Alt+[/Alt+] → anclá con
        // Alt+V → repetí". Si usáramos `volume_at`, una vez que la
        // lane tiene puntos el static dejaría de tener efecto y los
        // anchors siguientes capturarían siempre la interpolación.
        let value = track.volume;
        let lane = track.volume_automation.get_or_insert_with(AutomationLane::default);
        lane.add_point(beat.max(0.0), value);
        let n = lane.len();
        Some(format!("vol auto · pista {idx} · beat {beat:.1} · {n} pt · v {value:.2}"))
    }

    fn add_pan_automation_point(&mut self, beat: f32) -> ApplyOutcome {
        let idx = self.active_track;
        let track = self.score.track_mut(idx)?;
        let value = track.pan; // mismo razonamiento que volume.
        let lane = track.pan_automation.get_or_insert_with(AutomationLane::default);
        lane.add_point(beat.max(0.0), value);
        let n = lane.len();
        Some(format!("pan auto · pista {idx} · beat {beat:.1} · {n} pt · p {value:.2}"))
    }

    fn clear_active_automation(&mut self) -> ApplyOutcome {
        let idx = self.active_track;
        let track = self.score.track_mut(idx)?;
        let had_any =
            track.volume_automation.is_some() || track.pan_automation.is_some();
        track.volume_automation = None;
        track.pan_automation = None;
        if had_any {
            Some(format!("automation off · pista {idx}"))
        } else {
            None
        }
    }

    fn toggle_master_reverb(&mut self) -> ApplyOutcome {
        self.score.master_reverb = match self.score.master_reverb {
            None => Some(ReverbParams::default()),
            Some(_) => None,
        };
        Some(format!("reverb · {}", describe_master_reverb(&self.score.master_reverb)))
    }

    fn cycle_master_reverb_room(&mut self) -> ApplyOutcome {
        // Presets espaciales: cuarto, sala, catedral.
        const PRESETS: [f32; 3] = [0.25, 0.5, 0.85];
        let Some(params) = self.score.master_reverb.as_mut() else {
            return Some("reverb off (no se puede ciclar sala)".into());
        };
        let idx = PRESETS
            .iter()
            .position(|r| (r - params.room_size).abs() < 1e-3)
            .unwrap_or(0);
        params.room_size = PRESETS[(idx + 1) % PRESETS.len()];
        Some(format!("reverb · {}", describe_master_reverb(&self.score.master_reverb)))
    }

    fn nudge_active_pan(&mut self, delta: f32) -> ApplyOutcome {
        let idx = self.active_track;
        let track = self.score.track_mut(idx)?;
        let new_pan = (track.pan + delta).clamp(-1.0, 1.0);
        if (new_pan - track.pan).abs() < f32::EPSILON {
            return None;
        }
        track.pan = new_pan;
        let label = if new_pan.abs() < 0.05 {
            "C".to_string()
        } else if new_pan < 0.0 {
            format!("L{:.0}", new_pan.abs() * 100.0)
        } else {
            format!("R{:.0}", new_pan * 100.0)
        };
        Some(format!("pista {idx} · pan {label}"))
    }

    fn delete_active_track(&mut self) -> ApplyOutcome {
        let n = self.score.tracks().len();
        if n <= 1 {
            return Some("no se puede borrar la última pista".into());
        }
        let removed = self.active_track.min(n - 1);
        let gone = self.score.remove_track(removed);
        if self.active_track >= n - 1 {
            self.active_track = n - 2;
        }
        self.selected = match self.selected {
            Some((t, _)) if t == removed => None,
            Some((t, i)) if t > removed => Some((t - 1, i)),
            other => other,
        };
        let name = gone.as_ref().map(|t| t.name.as_str()).unwrap_or("?");
        Some(format!("deleted · pista {removed} ({name})"))
    }
}

/// Encuentra el índice de `target` en una lista de notas comparando los
/// campos relevantes. Lineal pero las pistas son cortas (≪1000 notas) en
/// el uso normal — alcanza. Si hay duplicados devuelve la primera.
pub fn find_note_idx(notes: &[ScoreNote], target: &ScoreNote) -> Option<usize> {
    notes.iter().position(|n| n == target)
}

/// Modo musical soportado por el editor. Más limitado que el catálogo
/// `takiy_core::Scale` para que el ciclo Q/Shift+Q tenga pocas opciones
/// y sea predecible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyMode {
    Major,
    NaturalMinor,
    PentatonicMajor,
}

impl KeyMode {
    fn next(self) -> Self {
        match self {
            KeyMode::Major => KeyMode::NaturalMinor,
            KeyMode::NaturalMinor => KeyMode::PentatonicMajor,
            KeyMode::PentatonicMajor => KeyMode::Major,
        }
    }

    fn scale(self, root: PitchClass) -> Scale {
        match self {
            KeyMode::Major => Scale::major(root),
            KeyMode::NaturalMinor => Scale::natural_minor(root),
            KeyMode::PentatonicMajor => Scale::pentatonic_major(root),
        }
    }

    fn label(self) -> &'static str {
        match self {
            KeyMode::Major => "major",
            KeyMode::NaturalMinor => "minor",
            KeyMode::PentatonicMajor => "pent5",
        }
    }
}

/// Detecta el modo de una `Scale` por su patrón (3 modos soportados;
/// cualquier otra cae a "major" como aproximación). Útil para los
/// cyclers sin que el state guarde el modo explícitamente.
fn classify_mode(scale: &Scale) -> KeyMode {
    // Reconstruimos las 3 escalas base sobre la misma raíz y comparamos.
    let root = scale.root();
    if *scale == Scale::natural_minor(root) {
        KeyMode::NaturalMinor
    } else if *scale == Scale::pentatonic_major(root) {
        KeyMode::PentatonicMajor
    } else {
        KeyMode::Major
    }
}

/// Próxima `PitchClass` en orden cromático (C → C# → … → B → C).
fn next_pitch_class(pc: PitchClass) -> PitchClass {
    PitchClass::from_semitone(pc.semitone().wrapping_add(1) % 12)
}

/// Pretty string para el header: `"off"` si no hay delay, o
/// `"1/8 · fb 0.35 · mix 0.25"` si está prendido. El time se mapea a un
/// nombre musical cuando coincide con uno conocido, si no se imprime
/// el float bruto.
pub fn describe_master_delay(delay: &Option<DelayParams>) -> String {
    let Some(d) = delay else {
        return "off".into();
    };
    let time = match d.time_beats {
        t if (t - 0.25).abs() < 1e-3 => "1/16".to_string(),
        t if (t - 0.5).abs() < 1e-3 => "1/8".to_string(),
        t if (t - 0.75).abs() < 1e-3 => "1/8·".to_string(),
        t if (t - 1.0).abs() < 1e-3 => "1/4".to_string(),
        t if (t - 1.5).abs() < 1e-3 => "1/4·".to_string(),
        t => format!("{t:.2}b"),
    };
    format!("{time} · fb {:.2} · mix {:.2}", d.feedback, d.mix)
}

/// Resumen ultra-corto del estado de automación de una pista para el
/// header. Devuelve `""` si no hay automación; si hay, `"v3"`, `"p2"`,
/// o `"v3p2"` según qué lanes están activas y cuántos puntos tienen.
pub fn describe_track_automation(track: &Track) -> String {
    let mut s = String::new();
    if let Some(l) = track.volume_automation.as_ref() {
        if !l.is_empty() {
            s.push_str(&format!("v{}", l.len()));
        }
    }
    if let Some(l) = track.pan_automation.as_ref() {
        if !l.is_empty() {
            s.push_str(&format!("p{}", l.len()));
        }
    }
    s
}

/// Pretty string para el reverb master: `"off"` o
/// `"sala · damp 0.50 · mix 0.25"`. El `room_size` se mapea a un
/// nombre cualitativo cuando coincide con un preset conocido.
pub fn describe_master_reverb(reverb: &Option<ReverbParams>) -> String {
    let Some(r) = reverb else {
        return "off".into();
    };
    let room = match r.room_size {
        s if (s - 0.25).abs() < 1e-3 => "cuarto".to_string(),
        s if (s - 0.5).abs() < 1e-3 => "sala".to_string(),
        s if (s - 0.85).abs() < 1e-3 => "catedral".to_string(),
        s => format!("room {s:.2}"),
    };
    format!("{room} · damp {:.2} · mix {:.2}", r.damping, r.mix)
}

/// Pretty string para el header — `"C major"`, `"A minor"`, `"none"`.
pub fn describe_key(key: &Option<Scale>) -> String {
    match key {
        None => "none".into(),
        Some(s) => format!("{} {}", s.root().name(), classify_mode(s).label()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_with_one_track_and_no_selection() {
        let st = EditorState::new(120.0);
        assert_eq!(st.score.tracks().len(), 1);
        assert_eq!(st.active_track, 0);
        assert!(st.selected.is_none());
    }

    #[test]
    fn add_note_selects_the_new_note() {
        let mut st = EditorState::new(120.0);
        assert!(st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 }).is_some());
        assert_eq!(st.selected, Some((0, 0)));
        assert_eq!(st.score.track(0).unwrap().notes().len(), 1);
    }

    #[test]
    fn add_invalid_midi_is_noop() {
        let mut st = EditorState::new(120.0);
        assert!(st.apply(EditMsg::AddNote { beat: 0.0, midi: 130 }).is_none());
        assert!(st.score.track(0).unwrap().notes().is_empty());
    }

    #[test]
    fn move_selected_keeps_selection_after_reinsert() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::AddNote { beat: 2.0, midi: 62 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        st.apply(EditMsg::MoveSelected { d_beat: 4.0, d_semitones: 0 });
        // Después del move, la nota antes en idx 0 cae al final por start.
        let notes = st.score.track(0).unwrap().notes();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].pitch.midi(), 62);
        assert_eq!(notes[1].pitch.midi(), 60);
        assert_eq!(st.selected, Some((0, 1)));
    }

    #[test]
    fn move_below_zero_is_noop() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        let before = st.score.track(0).unwrap().notes()[0];
        st.apply(EditMsg::MoveSelected { d_beat: -1.0, d_semitones: 0 });
        assert_eq!(st.score.track(0).unwrap().notes()[0], before);
    }

    #[test]
    fn move_out_of_midi_range_is_noop() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 1 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        let before = st.score.track(0).unwrap().notes()[0];
        st.apply(EditMsg::MoveSelected { d_beat: 0.0, d_semitones: -5 });
        assert_eq!(st.score.track(0).unwrap().notes()[0], before);
    }

    #[test]
    fn delete_note_adjusts_selection() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::AddNote { beat: 1.0, midi: 62 });
        st.apply(EditMsg::AddNote { beat: 2.0, midi: 64 });
        st.apply(EditMsg::Select { track: 0, idx: 2 });
        st.apply(EditMsg::DeleteNote { track: 0, idx: 1 });
        // La nota seleccionada estaba en 2; al borrar la 1 baja a 1.
        assert_eq!(st.selected, Some((0, 1)));
        assert_eq!(st.score.track(0).unwrap().notes().len(), 2);
    }

    #[test]
    fn delete_selected_clears_selection() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        st.apply(EditMsg::DeleteSelected);
        assert!(st.selected.is_none());
        assert!(st.score.track(0).unwrap().notes().is_empty());
    }

    #[test]
    fn resize_clamps_to_bounds() {
        let mut st = EditorState::new(120.0);
        st.snap = Snap::Free; // sin snap para ejercer los límites exactos
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        for _ in 0..200 {
            st.apply(EditMsg::ResizeSelected { d_beat: 0.5 });
        }
        assert!((st.score.track(0).unwrap().notes()[0].duration - 16.0).abs() < 1e-3);
        for _ in 0..200 {
            st.apply(EditMsg::ResizeSelected { d_beat: -0.5 });
        }
        assert!((st.score.track(0).unwrap().notes()[0].duration - 0.25).abs() < 1e-3);
    }

    #[test]
    fn velocity_nudge_clamps_1_127() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        for _ in 0..30 {
            st.apply(EditMsg::NudgeVelocity { delta: 10 });
        }
        assert_eq!(st.score.track(0).unwrap().notes()[0].velocity, 127);
        for _ in 0..30 {
            st.apply(EditMsg::NudgeVelocity { delta: -10 });
        }
        assert_eq!(st.score.track(0).unwrap().notes()[0].velocity, 1);
    }

    #[test]
    fn tempo_nudge_clamps_30_300() {
        let mut st = EditorState::new(120.0);
        for _ in 0..200 {
            st.apply(EditMsg::NudgeTempo { delta: 5.0 });
        }
        assert!((st.score.tempo_bpm - 300.0).abs() < 1e-3);
        for _ in 0..200 {
            st.apply(EditMsg::NudgeTempo { delta: -5.0 });
        }
        assert!((st.score.tempo_bpm - 30.0).abs() < 1e-3);
    }

    #[test]
    fn cycle_track_wraps_around() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::NewTrack);
        st.apply(EditMsg::NewTrack);
        assert_eq!(st.score.tracks().len(), 3);
        assert_eq!(st.active_track, 2);
        st.apply(EditMsg::CycleTrack);
        assert_eq!(st.active_track, 0);
        st.apply(EditMsg::CycleTrack);
        assert_eq!(st.active_track, 1);
    }

    #[test]
    fn cannot_delete_last_track() {
        let mut st = EditorState::new(120.0);
        let out = st.apply(EditMsg::DeleteActiveTrack);
        assert!(out.unwrap().contains("no se puede borrar"));
        assert_eq!(st.score.tracks().len(), 1);
    }

    #[test]
    fn delete_track_shifts_selection_indices() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::NewTrack); // track 1, active = 1
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        // selected = (1, 0)
        st.apply(EditMsg::CycleTrack); // active 0
        st.apply(EditMsg::DeleteActiveTrack); // borra track 0
        // selection: track > removed (0) → t - 1 = 0
        assert_eq!(st.selected, Some((0, 0)));
        assert_eq!(st.score.tracks().len(), 1);
    }

    #[test]
    fn snap_cycles_through_all_modes_and_returns_to_free() {
        let mut s = Snap::Free;
        for _ in 0..6 {
            s = s.cycle();
        }
        assert_eq!(s, Snap::Free);
    }

    #[test]
    fn snap_step_quantizes_correctly() {
        assert!((Snap::Half.snap(0.4) - 0.5).abs() < 1e-6);
        assert!((Snap::Half.snap(0.24) - 0.0).abs() < 1e-6);
        assert!((Snap::Quarter.snap(0.6) - 0.5).abs() < 1e-6);
        assert!((Snap::Eighth.snap(0.13) - 0.125).abs() < 1e-6);
        // Free no toca el valor.
        assert!((Snap::Free.snap(0.137) - 0.137).abs() < 1e-9);
    }

    #[test]
    fn add_note_snaps_beat_when_snap_is_active() {
        let mut st = EditorState::new(120.0);
        st.snap = Snap::Half;
        st.apply(EditMsg::AddNote { beat: 1.2, midi: 60 });
        let notes = st.score.track(0).unwrap().notes();
        assert!((notes[0].start - 1.0).abs() < 1e-6, "snap a múltiplo de 0.5");
        st.snap = Snap::Free;
        st.apply(EditMsg::AddNote { beat: 1.7, midi: 62 });
        let notes = st.score.track(0).unwrap().notes();
        assert!((notes[1].start - 1.7).abs() < 1e-6, "free preserva fraccional");
    }

    #[test]
    fn undo_reverts_last_edit_and_redo_reapplies() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        assert_eq!(st.score.track(0).unwrap().notes().len(), 1);
        assert!(st.undo().is_some());
        assert_eq!(st.score.track(0).unwrap().notes().len(), 0);
        assert!(st.redo().is_some());
        assert_eq!(st.score.track(0).unwrap().notes().len(), 1);
    }

    #[test]
    fn undo_stack_limits_at_max_undo() {
        let mut st = EditorState::new(120.0);
        for i in 0..(MAX_UNDO + 30) {
            st.apply(EditMsg::AddNote { beat: i as f32, midi: 60 });
        }
        assert_eq!(st.history.len(), MAX_UNDO);
    }

    #[test]
    fn new_edit_truncates_future_branch() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::AddNote { beat: 1.0, midi: 62 });
        st.undo(); // future = [score con 2 notas]
        assert_eq!(st.future.len(), 1);
        st.apply(EditMsg::AddNote { beat: 5.0, midi: 70 }); // edición nueva
        assert!(st.future.is_empty(), "rama futura truncada");
    }

    #[test]
    fn no_op_edits_do_not_push_to_history() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        let len_before = st.history.len();
        // MIDI inválido — no muta el score.
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 200 });
        assert_eq!(st.history.len(), len_before, "no debe registrar no-ops");
    }

    #[test]
    fn copy_and_paste_creates_a_clone_at_target_beat() {
        let mut st = EditorState::new(120.0);
        st.snap = Snap::Free;
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        st.apply(EditMsg::CopySelected);
        assert_eq!(st.clipboard.len(), 1);
        st.apply(EditMsg::PasteAt { beat: 4.0 });
        let notes = st.score.track(0).unwrap().notes();
        assert_eq!(notes.len(), 2);
        // Original en beat 0, paste en beat 4.
        assert!((notes[0].start - 0.0).abs() < 1e-6);
        assert!((notes[1].start - 4.0).abs() < 1e-6);
        // Pitch + velocity + duration preservados.
        assert_eq!(notes[1].pitch.midi(), 60);
        assert_eq!(notes[1].velocity, 96);
        assert!((notes[1].duration - 1.0).abs() < 1e-6);
    }

    #[test]
    fn paste_without_clipboard_is_noop() {
        let mut st = EditorState::new(120.0);
        let out = st.apply(EditMsg::PasteAt { beat: 0.0 });
        assert!(out.is_none());
        assert!(st.score.track(0).unwrap().notes().is_empty());
    }

    #[test]
    fn cut_removes_and_fills_clipboard() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddNote { beat: 2.0, midi: 64 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        st.apply(EditMsg::CutSelected);
        assert!(st.score.track(0).unwrap().notes().is_empty());
        assert_eq!(st.clipboard.len(), 1);
        assert_eq!(st.clipboard[0].pitch.midi(), 64);
    }

    #[test]
    fn paste_respects_snap() {
        let mut st = EditorState::new(120.0);
        st.snap = Snap::Beat; // redondeo entero
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        st.apply(EditMsg::CopySelected);
        st.apply(EditMsg::PasteAt { beat: 4.3 });
        let notes = st.score.track(0).unwrap().notes();
        // 4.3 snappeado a 4.0.
        assert!((notes[1].start - 4.0).abs() < 1e-6);
    }

    #[test]
    fn duplicate_inserts_clone_after_note() {
        let mut st = EditorState::new(120.0);
        st.snap = Snap::Free;
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        st.apply(EditMsg::DuplicateSelected);
        let notes = st.score.track(0).unwrap().notes();
        assert_eq!(notes.len(), 2);
        // El duplicado va a beat = start + duration = 0 + 1 = 1.
        assert!((notes[1].start - 1.0).abs() < 1e-6);
        assert_eq!(notes[1].pitch.midi(), 60);
    }

    #[test]
    fn paste_is_undoable() {
        let mut st = EditorState::new(120.0);
        st.snap = Snap::Free;
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        st.apply(EditMsg::CopySelected);
        st.apply(EditMsg::PasteAt { beat: 4.0 });
        assert_eq!(st.score.track(0).unwrap().notes().len(), 2);
        st.undo();
        assert_eq!(st.score.track(0).unwrap().notes().len(), 1);
    }

    #[test]
    fn mixer_toggles_apply_to_active_track() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::NewTrack); // pista 1 activa
        assert_eq!(st.active_track, 1);
        st.apply(EditMsg::ToggleMuteActive);
        assert!(st.score.track(1).unwrap().mute);
        assert!(!st.score.track(0).unwrap().mute);
        st.apply(EditMsg::ToggleSoloActive);
        assert!(st.score.track(1).unwrap().solo);
    }

    #[test]
    fn volume_nudge_clamps_to_0_to_1_5() {
        let mut st = EditorState::new(120.0);
        for _ in 0..30 {
            st.apply(EditMsg::NudgeActiveVolume { delta: 0.1 });
        }
        assert!((st.score.track(0).unwrap().volume - 1.5).abs() < 1e-3);
        for _ in 0..30 {
            st.apply(EditMsg::NudgeActiveVolume { delta: -0.1 });
        }
        assert!(st.score.track(0).unwrap().volume.abs() < 1e-3);
    }

    #[test]
    fn cycle_key_root_starts_at_c_major_then_chromatic() {
        let mut st = EditorState::new(120.0);
        assert!(st.score.key.is_none());
        st.apply(EditMsg::CycleKeyRoot);
        let k = st.score.key.as_ref().unwrap();
        assert_eq!(k.root(), PitchClass::C);
        st.apply(EditMsg::CycleKeyRoot);
        assert_eq!(st.score.key.as_ref().unwrap().root(), PitchClass::Cs);
    }

    #[test]
    fn cycle_key_root_wraps_at_b_back_to_none() {
        let mut st = EditorState::new(120.0);
        // Avanzamos 12 veces desde None: arranca en C; el ciclo 12 cae en B,
        // y la siguiente vuelve a None.
        for _ in 0..12 {
            st.apply(EditMsg::CycleKeyRoot);
        }
        assert_eq!(st.score.key.as_ref().unwrap().root(), PitchClass::B);
        st.apply(EditMsg::CycleKeyRoot);
        assert!(st.score.key.is_none());
    }

    #[test]
    fn cycle_key_mode_changes_scale_pattern_keeping_root() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::CycleKeyRoot); // → C major
        let scale_before = st.score.key.clone().unwrap();
        st.apply(EditMsg::CycleKeyMode); // → C minor
        let scale_after = st.score.key.clone().unwrap();
        assert_eq!(scale_before.root(), scale_after.root());
        assert_ne!(scale_before, scale_after);
    }

    #[test]
    fn cycle_key_mode_from_none_enables_c_major() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::CycleKeyMode);
        let k = st.score.key.as_ref().unwrap();
        assert_eq!(k.root(), PitchClass::C);
    }

    #[test]
    fn describe_key_formats_root_and_mode() {
        let mut st = EditorState::new(120.0);
        assert_eq!(describe_key(&st.score.key), "none");
        st.apply(EditMsg::CycleKeyRoot); // C major
        assert_eq!(describe_key(&st.score.key), "C major");
        st.apply(EditMsg::CycleKeyMode); // C minor
        assert_eq!(describe_key(&st.score.key), "C minor");
        st.apply(EditMsg::CycleKeyMode); // C pent5
        assert_eq!(describe_key(&st.score.key), "C pent5");
    }

    #[test]
    fn pan_nudge_clamps_to_minus_one_to_one() {
        let mut st = EditorState::new(120.0);
        for _ in 0..30 {
            st.apply(EditMsg::NudgeActivePan { delta: 0.1 });
        }
        assert!((st.score.track(0).unwrap().pan - 1.0).abs() < 1e-3);
        for _ in 0..30 {
            st.apply(EditMsg::NudgeActivePan { delta: -0.1 });
        }
        assert!((st.score.track(0).unwrap().pan + 1.0).abs() < 1e-3);
    }

    #[test]
    fn mixer_changes_are_undoable() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::ToggleMuteActive);
        assert!(st.score.track(0).unwrap().mute);
        st.undo();
        assert!(!st.score.track(0).unwrap().mute);
    }

    #[test]
    fn toggle_metronome_cycles_off_to_4_4_back_to_off() {
        let mut st = EditorState::new(120.0);
        assert!(st.metronome_beats_per_bar.is_none());
        st.toggle_metronome();
        assert_eq!(st.metronome_beats_per_bar, Some(4));
        st.toggle_metronome();
        assert!(st.metronome_beats_per_bar.is_none());
    }

    #[test]
    fn set_loop_region_validates_bounds() {
        let mut st = EditorState::new(120.0);
        assert!(st.set_loop_region(Some((0.0, 4.0))).is_some());
        assert_eq!(st.loop_region, Some((0.0, 4.0)));
        // from >= to → rechazado, no cambia.
        assert!(st.set_loop_region(Some((4.0, 4.0))).is_none());
        assert_eq!(st.loop_region, Some((0.0, 4.0)));
        // from negativo → rechazado.
        assert!(st.set_loop_region(Some((-1.0, 4.0))).is_none());
        assert_eq!(st.loop_region, Some((0.0, 4.0)));
        // None apaga.
        assert!(st.set_loop_region(None).is_some());
        assert!(st.loop_region.is_none());
    }

    #[test]
    fn set_selected_duration_snaps_and_clamps() {
        let mut st = EditorState::new(120.0);
        st.snap = Snap::Half;
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        // 2.7 snappeado a 2.5 (múltiplo de 0.5).
        st.apply(EditMsg::SetSelectedDuration { duration: 2.7 });
        let n = st.score.track(0).unwrap().notes()[0];
        assert!((n.duration - 2.5).abs() < 1e-6);
        // Clamp inferior.
        st.snap = Snap::Free;
        st.apply(EditMsg::SetSelectedDuration { duration: 0.01 });
        let n = st.score.track(0).unwrap().notes()[0];
        assert!((n.duration - 0.125).abs() < 1e-6);
        // Clamp superior.
        st.apply(EditMsg::SetSelectedDuration { duration: 999.0 });
        let n = st.score.track(0).unwrap().notes()[0];
        assert!((n.duration - 16.0).abs() < 1e-6);
    }

    #[test]
    fn set_selected_duration_is_idempotent() {
        let mut st = EditorState::new(120.0);
        st.snap = Snap::Beat;
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        let len_before = st.history.len();
        // La nota arranca con duration=1.0; pedir 1.0 es no-op.
        assert!(st.apply(EditMsg::SetSelectedDuration { duration: 1.0 }).is_none());
        assert_eq!(st.history.len(), len_before);
    }

    #[test]
    fn set_selected_absolute_snaps_start_and_keeps_duration() {
        let mut st = EditorState::new(120.0);
        st.snap = Snap::Beat;
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        let dur_before = st.score.track(0).unwrap().notes()[0].duration;
        st.apply(EditMsg::SetSelectedAbsolute { start: 3.4, midi: 64 });
        let n = st.score.track(0).unwrap().notes()[0];
        assert!((n.start - 3.0).abs() < 1e-6, "snap a beat entero");
        assert_eq!(n.pitch.midi(), 64);
        assert!((n.duration - dur_before).abs() < 1e-6, "duración intacta");
    }

    #[test]
    fn set_selected_absolute_is_idempotent_on_snap_floor() {
        let mut st = EditorState::new(120.0);
        st.snap = Snap::Beat;
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        let len_before = st.history.len();
        // 3.4 → snap a 3.0
        assert!(st.apply(EditMsg::SetSelectedAbsolute { start: 3.4, midi: 60 }).is_some());
        // Re-llamada con beat distinto pero que snappea al mismo lugar: no-op.
        assert!(st.apply(EditMsg::SetSelectedAbsolute { start: 3.3, midi: 60 }).is_none());
        assert_eq!(st.history.len(), len_before + 1, "una sóla entrada de undo");
    }

    #[test]
    fn drag_batches_history_into_single_undo() {
        // Simula un drag: begin_drag + N micro-moves + end_drag = un solo undo.
        let mut st = EditorState::new(120.0);
        st.snap = Snap::Free;
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        let history_before = st.history.len();

        st.begin_drag();
        assert!(st.is_dragging());
        for step in 1..=20 {
            // Cada paso es un SetSelectedAbsolute con un beat fraccionalmente
            // distinto, todos durante el drag.
            st.apply(EditMsg::SetSelectedAbsolute {
                start: step as f32 * 0.1,
                midi: 60,
            });
        }
        assert!(st.is_dragging(), "drag sigue activo durante mutaciones");
        // Durante el drag no se acumula history:
        assert_eq!(st.history.len(), history_before);

        let out = st.end_drag();
        assert!(out.is_some(), "end_drag con cambio devuelve mensaje");
        assert!(!st.is_dragging());
        // Después del drag, exactamente UNA entrada nueva en history.
        assert_eq!(st.history.len(), history_before + 1);

        // Un solo undo lleva la nota a su posición original (beat 0).
        st.undo();
        let n = st.score.track(0).unwrap().notes()[0];
        assert!((n.start - 0.0).abs() < 1e-6, "undo restaura beat 0");
    }

    #[test]
    fn drag_without_changes_does_not_push_history() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        let len_before = st.history.len();
        st.begin_drag();
        // Sin mutaciones intermedias.
        let out = st.end_drag();
        assert!(out.is_none());
        assert_eq!(st.history.len(), len_before);
    }

    #[test]
    fn begin_drag_is_idempotent_and_preserves_first_snapshot() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddNote { beat: 0.0, midi: 60 });
        st.apply(EditMsg::Select { track: 0, idx: 0 });
        st.begin_drag();
        // Mutación en el medio del drag.
        st.apply(EditMsg::SetSelectedAbsolute { start: 2.0, midi: 60 });
        // begin_drag de nuevo no debe pisar el snapshot original.
        st.begin_drag();
        st.apply(EditMsg::SetSelectedAbsolute { start: 4.0, midi: 60 });
        st.end_drag();
        st.undo();
        // El undo debe llevar a beat 0 (snapshot original), no a 2.0.
        let n = st.score.track(0).unwrap().notes()[0];
        assert!((n.start - 0.0).abs() < 1e-6);
    }

    #[test]
    fn toggle_master_delay_round_trips_default() {
        let mut st = EditorState::new(120.0);
        assert!(st.score.master_delay.is_none());
        st.apply(EditMsg::ToggleMasterDelay);
        let d = st.score.master_delay.unwrap();
        assert_eq!(d, DelayParams::default(), "arranca con preset razonable");
        st.apply(EditMsg::ToggleMasterDelay);
        assert!(st.score.master_delay.is_none(), "vuelve a apagado");
    }

    #[test]
    fn toggle_master_delay_is_undoable() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::ToggleMasterDelay);
        assert!(st.score.master_delay.is_some());
        st.undo();
        assert!(st.score.master_delay.is_none(), "undo apaga el delay");
    }

    #[test]
    fn cycle_master_delay_time_walks_through_presets() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::ToggleMasterDelay); // arranca en 0.5
        let times: Vec<f32> = (0..6)
            .map(|_| {
                st.apply(EditMsg::CycleMasterDelayTime);
                st.score.master_delay.as_ref().unwrap().time_beats
            })
            .collect();
        // Cinco presets — al 6to ciclo vuelve al 1ro.
        assert_eq!(times.len(), 6);
        assert!((times[0] - 1.0).abs() < 1e-6, "1/8 → 1/4");
        assert!((times[5] - times[0]).abs() < 1e-6, "ciclo cerrado");
    }

    #[test]
    fn cycle_master_delay_time_when_off_is_noop_with_status() {
        let mut st = EditorState::new(120.0);
        let out = st.apply(EditMsg::CycleMasterDelayTime);
        assert!(st.score.master_delay.is_none(), "no enciende solo");
        assert!(out.unwrap().contains("off"));
    }

    #[test]
    fn add_volume_automation_point_creates_lane_at_active_track() {
        let mut st = EditorState::new(120.0);
        // Asegurate de que la pista activa tiene volumen no-default.
        st.score.track_mut(0).unwrap().volume = 0.7;
        st.apply(EditMsg::AddVolumeAutomationPoint { beat: 4.0 });
        let track = st.score.track(0).unwrap();
        let lane = track.volume_automation.as_ref().unwrap();
        assert_eq!(lane.points.len(), 1);
        assert!((lane.points[0].beat - 4.0).abs() < 1e-6);
        assert!((lane.points[0].value - 0.7).abs() < 1e-6, "anchor=volumen actual");
    }

    #[test]
    fn add_volume_automation_point_is_undoable() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddVolumeAutomationPoint { beat: 0.0 });
        assert!(st.score.track(0).unwrap().volume_automation.is_some());
        st.undo();
        assert!(st.score.track(0).unwrap().volume_automation.is_none());
    }

    #[test]
    fn add_pan_automation_point_appends_to_existing_lane() {
        let mut st = EditorState::new(120.0);
        st.score.track_mut(0).unwrap().pan = 0.5;
        st.apply(EditMsg::AddPanAutomationPoint { beat: 0.0 });
        st.score.track_mut(0).unwrap().pan = -0.5;
        st.apply(EditMsg::AddPanAutomationPoint { beat: 8.0 });
        let lane = st.score.track(0).unwrap().pan_automation.as_ref().unwrap();
        assert_eq!(lane.points.len(), 2);
        assert!((lane.points[0].value - 0.5).abs() < 1e-6);
        assert!((lane.points[1].value + 0.5).abs() < 1e-6);
    }

    #[test]
    fn clear_active_automation_wipes_both_lanes() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::AddVolumeAutomationPoint { beat: 0.0 });
        st.apply(EditMsg::AddPanAutomationPoint { beat: 4.0 });
        st.apply(EditMsg::ClearActiveAutomation);
        let t = st.score.track(0).unwrap();
        assert!(t.volume_automation.is_none() && t.pan_automation.is_none());
    }

    #[test]
    fn clear_active_automation_without_lanes_is_noop() {
        let mut st = EditorState::new(120.0);
        let len_before = st.history.len();
        let out = st.apply(EditMsg::ClearActiveAutomation);
        assert!(out.is_none(), "sin automación, sin mensaje");
        assert_eq!(st.history.len(), len_before, "sin push a history");
    }

    #[test]
    fn describe_track_automation_summarizes_lanes() {
        let mut t = Track::new("a");
        assert_eq!(describe_track_automation(&t), "");
        let mut vlane = AutomationLane::default();
        vlane.add_point(0.0, 0.5);
        vlane.add_point(4.0, 0.8);
        vlane.add_point(8.0, 0.3);
        t.volume_automation = Some(vlane);
        assert_eq!(describe_track_automation(&t), "v3");
        let mut plane = AutomationLane::default();
        plane.add_point(0.0, 0.0);
        plane.add_point(8.0, 1.0);
        t.pan_automation = Some(plane);
        assert_eq!(describe_track_automation(&t), "v3p2");
    }

    #[test]
    fn toggle_master_reverb_round_trips_default() {
        let mut st = EditorState::new(120.0);
        assert!(st.score.master_reverb.is_none());
        st.apply(EditMsg::ToggleMasterReverb);
        assert_eq!(st.score.master_reverb.unwrap(), ReverbParams::default());
        st.apply(EditMsg::ToggleMasterReverb);
        assert!(st.score.master_reverb.is_none());
    }

    #[test]
    fn toggle_master_reverb_is_undoable() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::ToggleMasterReverb);
        assert!(st.score.master_reverb.is_some());
        st.undo();
        assert!(st.score.master_reverb.is_none());
    }

    #[test]
    fn cycle_master_reverb_room_walks_through_presets() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::ToggleMasterReverb); // arranca en 0.5 (sala)
        st.apply(EditMsg::CycleMasterReverbRoom);
        assert!((st.score.master_reverb.unwrap().room_size - 0.85).abs() < 1e-6);
        st.apply(EditMsg::CycleMasterReverbRoom);
        assert!((st.score.master_reverb.unwrap().room_size - 0.25).abs() < 1e-6);
        st.apply(EditMsg::CycleMasterReverbRoom);
        assert!((st.score.master_reverb.unwrap().room_size - 0.5).abs() < 1e-6);
    }

    #[test]
    fn cycle_master_reverb_room_when_off_is_noop_with_status() {
        let mut st = EditorState::new(120.0);
        let out = st.apply(EditMsg::CycleMasterReverbRoom);
        assert!(st.score.master_reverb.is_none());
        assert!(out.unwrap().contains("off"));
    }

    #[test]
    fn new_track_names_are_unique_even_after_delete() {
        let mut st = EditorState::new(120.0);
        st.apply(EditMsg::NewTrack); // "track 2"
        st.apply(EditMsg::NewTrack); // "track 3"
        assert_eq!(st.score.track(2).unwrap().name, "track 3");
        st.apply(EditMsg::DeleteActiveTrack); // borra "track 3"
        st.apply(EditMsg::NewTrack); // debe ser "track 4"
        assert_eq!(st.score.tracks().last().unwrap().name, "track 4");
    }
}
