//! Estado editable del piano roll — Score + selección + pista activa.
//!
//! Es la lógica pura del editor: cero audio, cero UI. El binario Llimphi
//! le manda [`EditMsg`]s; el example `smoke` lo ejerce headless en CI.

use std::path::PathBuf;

use takiy_core::{Pitch, Score, ScoreNote, Track};

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
}

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
}

/// Resultado de aplicar un `EditMsg`: mensaje corto para el header.
/// `None` cuando la acción fue no-op (índice inválido, sin selección,
/// clamp sin cambio, etc.). El binario lo usa para repintar el status.
pub type ApplyOutcome = Option<String>;

impl EditorState {
    /// Aplica una edición. No persiste — eso lo decide el binario. Es
    /// pura sobre `&mut self`: no toca filesystem ni audio.
    pub fn apply(&mut self, msg: EditMsg) -> ApplyOutcome {
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
        }
    }

    fn add_note(&mut self, beat: f32, midi: u8) -> ApplyOutcome {
        let pitch = Pitch::from_midi(midi)?;
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
        let track = self.score.track_mut(track_idx)?;
        let old = track.notes().get(note_idx).copied()?;
        let new_start = old.start + d_beat;
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
        let track = self.score.track_mut(track_idx)?;
        let old = track.notes().get(note_idx).copied()?;
        let new_dur = (old.duration + d_beat).clamp(0.25, 16.0);
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
