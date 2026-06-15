//! El motor de edición: `apply` con su lógica de undo/redo y drag-batching,
//! más cada operación concreta (notas, pistas, mixer, tonalidad, efectos y
//! automación). Todo muta el `Score` del `EditorState` y devuelve un
//! `ApplyOutcome` con el mensaje para el header.

use takiy_core::{
    AutomationLane, DelayParams, Pitch, PitchClass, ReverbParams, ScoreNote, Track,
};

use super::describe::{
    classify_mode, describe_key, describe_master_delay, describe_master_reverb, find_note_idx,
    next_pitch_class, KeyMode,
};
use super::{ApplyOutcome, EditMsg, EditorState, Snap, MAX_UNDO};

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
            EditMsg::SetActiveTrack { track } => self.set_active_track(track),
            EditMsg::NudgeTrackVolume { track, delta } => self.nudge_track_volume(track, delta),
            EditMsg::NudgeTrackPan { track, delta } => self.nudge_track_pan(track, delta),
            EditMsg::ToggleMuteTrack { track } => self.toggle_mute_track(track),
            EditMsg::ToggleSoloTrack { track } => self.toggle_solo_track(track),
            EditMsg::SetSnap { snap } => self.set_snap(snap),
            EditMsg::SetMasterDelayTime { idx } => self.set_master_delay_time(idx),
            EditMsg::SetMasterReverbRoom { idx } => self.set_master_reverb_room(idx),
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
            EditMsg::SetAutomationPoint { track_idx, is_volume, idx, beat, value } => {
                self.set_automation_point(track_idx, is_volume, idx, beat, value)
            }
            EditMsg::InsertAutomationPoint { track_idx, is_volume, beat, value } => {
                self.insert_automation_point(track_idx, is_volume, beat, value)
            }
            EditMsg::DeleteAutomationPoint { track_idx, is_volume, idx } => {
                self.delete_automation_point(track_idx, is_volume, idx)
            }
            EditMsg::ToggleSnapToKey => self.toggle_snap_to_key(),
        }
    }

    /// Si `snap_to_key` está prendido y hay key activa, redondea `midi`
    /// al pitch en escala más cercano (empate hacia arriba). Si no hay
    /// nada que cuantizar, devuelve el midi sin tocar. Retorna `None`
    /// si `midi` no es un MIDI válido.
    fn quantize_midi(&self, midi: u8) -> Option<u8> {
        let Some(pitch) = Pitch::from_midi(midi) else {
            return None;
        };
        if !self.snap_to_key {
            return Some(midi);
        }
        let Some(scale) = self.score.key.as_ref() else {
            return Some(midi);
        };
        Some(scale.nearest_in_scale(pitch).midi())
    }

    fn toggle_snap_to_key(&mut self) -> ApplyOutcome {
        self.snap_to_key = !self.snap_to_key;
        let state = if self.snap_to_key { "on" } else { "off" };
        let warn = if self.snap_to_key && self.score.key.is_none() {
            " · (sin key — Q/K para definirla)"
        } else {
            ""
        };
        Some(format!("snap-key · {state}{warn}"))
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
        let midi = self.quantize_midi(midi)?;
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

    /// Hace activa la pista por índice (click en su fila del mixer).
    fn set_active_track(&mut self, track: usize) -> ApplyOutcome {
        if track >= self.score.tracks().len() {
            return None;
        }
        if self.active_track == track {
            return None;
        }
        self.active_track = track;
        let name = self
            .score
            .track(track)
            .map(|t| t.name.as_str())
            .unwrap_or("?");
        Some(format!("active · pista {track} ({name})"))
    }

    fn nudge_track_volume(&mut self, track: usize, delta: f32) -> ApplyOutcome {
        let t = self.score.track_mut(track)?;
        let new_vol = (t.volume + delta).clamp(0.0, 1.5);
        if (new_vol - t.volume).abs() < f32::EPSILON {
            return None;
        }
        t.volume = new_vol;
        Some(format!("pista {track} · vol {new_vol:.2}"))
    }

    fn nudge_track_pan(&mut self, track: usize, delta: f32) -> ApplyOutcome {
        let t = self.score.track_mut(track)?;
        let new_pan = (t.pan + delta).clamp(-1.0, 1.0);
        if (new_pan - t.pan).abs() < f32::EPSILON {
            return None;
        }
        t.pan = new_pan;
        let label = if new_pan.abs() < 0.05 {
            "C".to_string()
        } else if new_pan < 0.0 {
            format!("L{:.0}", new_pan.abs() * 100.0)
        } else {
            format!("R{:.0}", new_pan * 100.0)
        };
        Some(format!("pista {track} · pan {label}"))
    }

    fn toggle_mute_track(&mut self, track: usize) -> ApplyOutcome {
        let t = self.score.track_mut(track)?;
        t.mute = !t.mute;
        let state = if t.mute { "on" } else { "off" };
        Some(format!("pista {track} · mute {state}"))
    }

    fn toggle_solo_track(&mut self, track: usize) -> ApplyOutcome {
        let t = self.score.track_mut(track)?;
        t.solo = !t.solo;
        let state = if t.solo { "on" } else { "off" };
        Some(format!("pista {track} · solo {state}"))
    }

    fn set_snap(&mut self, snap: Snap) -> ApplyOutcome {
        if self.snap == snap {
            return None;
        }
        self.snap = snap;
        Some(format!("snap · {}", self.snap.label()))
    }

    fn set_master_delay_time(&mut self, idx: usize) -> ApplyOutcome {
        // Mismos presets que `cycle_master_delay_time`.
        const PRESETS: [f32; 5] = [0.5, 1.0, 1.5, 0.75, 0.25];
        let Some(params) = self.score.master_delay.as_mut() else {
            return Some("delay off (no se puede fijar tiempo)".into());
        };
        let t = *PRESETS.get(idx)?;
        if (params.time_beats - t).abs() < 1e-3 {
            return None;
        }
        params.time_beats = t;
        Some(format!("delay · {}", describe_master_delay(&self.score.master_delay)))
    }

    fn set_master_reverb_room(&mut self, idx: usize) -> ApplyOutcome {
        // Mismos presets que `cycle_master_reverb_room`.
        const PRESETS: [f32; 3] = [0.25, 0.5, 0.85];
        let Some(params) = self.score.master_reverb.as_mut() else {
            return Some("reverb off (no se puede fijar sala)".into());
        };
        let r = *PRESETS.get(idx)?;
        if (params.room_size - r).abs() < 1e-3 {
            return None;
        }
        params.room_size = r;
        Some(format!("reverb · {}", describe_master_reverb(&self.score.master_reverb)))
    }

    fn move_selected(&mut self, d_beat: f32, d_semitones: i32) -> ApplyOutcome {
        let (track_idx, note_idx) = self.selected?;
        let snap = self.snap;
        // Resolvemos el nuevo pitch fuera del borrow mutable porque
        // necesitamos consultar `score.key` y `snap_to_key`.
        let snap_to_key = self.snap_to_key;
        let key = self.score.key.clone();
        let track = self.score.track_mut(track_idx)?;
        let old = track.notes().get(note_idx).copied()?;
        // Si hay snap activo, redondeamos el nuevo start al múltiplo
        // exacto — facilita encadenar moves sin acumular drift.
        let raw_start = old.start + d_beat;
        let new_start = snap.snap(raw_start);
        if new_start < 0.0 {
            return None;
        }
        // Snap a la tonalidad: si está prendido y hay key, ±1 semitono
        // = ±1 grado de escala. Permite ←/→/↑/↓ pensar en grados sin
        // dejar de usar los mismos atajos que el cromático.
        let new_pitch = if d_semitones != 0 {
            if let (true, Some(scale)) = (snap_to_key, key.as_ref()) {
                scale.step_in_scale(old.pitch, d_semitones)?
            } else {
                let new_midi = old.pitch.midi() as i32 + d_semitones;
                u8::try_from(new_midi).ok().and_then(Pitch::from_midi)?
            }
        } else {
            old.pitch
        };
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
        let midi = self.quantize_midi(midi)?;
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

    fn insert_automation_point(
        &mut self,
        track_idx: usize,
        is_volume: bool,
        beat: f32,
        value: f32,
    ) -> ApplyOutcome {
        let track = self.score.track_mut(track_idx)?;
        let (v_min, v_max) = if is_volume { (0.0, 1.5) } else { (-1.0, 1.0) };
        let beat = beat.max(0.0);
        let value = value.clamp(v_min, v_max);
        let lane = if is_volume {
            track.volume_automation.get_or_insert_with(AutomationLane::default)
        } else {
            track.pan_automation.get_or_insert_with(AutomationLane::default)
        };
        lane.add_point(beat, value);
        let n = lane.len();
        let kind = if is_volume { "vol" } else { "pan" };
        Some(format!(
            "{kind} auto · pista {track_idx} · insert beat {beat:.1} val {value:.2} · {n} pt"
        ))
    }

    fn delete_automation_point(
        &mut self,
        track_idx: usize,
        is_volume: bool,
        idx: usize,
    ) -> ApplyOutcome {
        let track = self.score.track_mut(track_idx)?;
        let lane = if is_volume {
            track.volume_automation.as_mut()?
        } else {
            track.pan_automation.as_mut()?
        };
        if idx >= lane.points.len() {
            return None;
        }
        lane.points.remove(idx);
        // Si la lane quedó vacía, apagar la `Option` para que la pista
        // vuelva a usar el static. Lanes vacías son ambiguas para el
        // resto del sistema y painter sabe ignorarlas, pero es más limpio
        // limpiar el flag.
        let became_empty = lane.points.is_empty();
        if became_empty {
            if is_volume {
                track.volume_automation = None;
            } else {
                track.pan_automation = None;
            }
        }
        let kind = if is_volume { "vol" } else { "pan" };
        Some(format!(
            "{kind} auto · pista {track_idx} · del #{idx}{}",
            if became_empty { " · lane off" } else { "" }
        ))
    }

    fn set_automation_point(
        &mut self,
        track_idx: usize,
        is_volume: bool,
        idx: usize,
        beat: f32,
        value: f32,
    ) -> ApplyOutcome {
        let track = self.score.track_mut(track_idx)?;
        let lane = if is_volume {
            track.volume_automation.as_mut()?
        } else {
            track.pan_automation.as_mut()?
        };
        if idx >= lane.points.len() {
            return None;
        }
        // Clamp beat entre los vecinos (epsilon evita coincidir y romper
        // partition_point en futuros add_point). Si no hay vecino, usa el
        // borde natural (0 para abajo, +∞ para arriba).
        let eps = 1e-4;
        let lo = if idx > 0 {
            lane.points[idx - 1].beat + eps
        } else {
            0.0
        };
        let hi = if idx + 1 < lane.points.len() {
            lane.points[idx + 1].beat - eps
        } else {
            f32::INFINITY
        };
        let new_beat = beat.clamp(lo, hi.max(lo));
        let (v_min, v_max) = if is_volume { (0.0, 1.5) } else { (-1.0, 1.0) };
        let new_value = value.clamp(v_min, v_max);
        let old = lane.points[idx];
        if (old.beat - new_beat).abs() < f32::EPSILON
            && (old.value - new_value).abs() < f32::EPSILON
        {
            return None;
        }
        lane.points[idx].beat = new_beat;
        lane.points[idx].value = new_value;
        let kind = if is_volume { "vol" } else { "pan" };
        Some(format!(
            "{kind} auto · pista {track_idx} · pt #{idx} → beat {new_beat:.1} val {new_value:.2}"
        ))
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
