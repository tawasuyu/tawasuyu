//! Estado editable del piano roll — Score + selección + pista activa.
//!
//! Es la lógica pura del editor: cero audio, cero UI. El binario Llimphi
//! le manda [`EditMsg`]s; el example `smoke` lo ejerce headless en CI.

use std::path::PathBuf;

use takiy_core::{Pitch, Score, ScoreNote, Track};

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
}

/// Resultado de aplicar un `EditMsg`: mensaje corto para el header.
/// `None` cuando la acción fue no-op (índice inválido, sin selección,
/// clamp sin cambio, etc.). El binario lo usa para repintar el status.
pub type ApplyOutcome = Option<String>;

impl EditorState {
    /// Aplica una edición. Envuelve `apply_internal` con la lógica de
    /// undo: snapshot pre-mutación, descarte si no cambió nada,
    /// truncado de `future` ante una rama nueva.
    pub fn apply(&mut self, msg: EditMsg) -> ApplyOutcome {
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

    fn apply_internal(&mut self, msg: EditMsg) -> ApplyOutcome {
        match msg {
            EditMsg::AddNote { beat, midi } => self.add_note(beat, midi),
            EditMsg::DeleteNote { track, idx } => self.delete_note(track, idx),
            EditMsg::Select { track, idx } => self.select(track, idx),
            EditMsg::MoveSelected { d_beat, d_semitones } => {
                self.move_selected(d_beat, d_semitones)
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
