//! El modelo de partitura — notas, pistas y un `Score` con tempo.
//!
//! El tiempo se mide en *pulsos* (beats), no en segundos: una partitura
//! es independiente del tempo hasta que se la reproduce. La conversión a
//! segundos vive en [`Score::duration_seconds`].

use serde::{Deserialize, Serialize};

use crate::pitch::Pitch;
use crate::scale::Scale;

/// Una nota dentro de una pista: altura, inicio y duración en pulsos,
/// y velocidad (intensidad MIDI).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScoreNote {
    pub pitch: Pitch,
    /// Pulso de inicio.
    pub start: f32,
    /// Duración en pulsos.
    pub duration: f32,
    /// Intensidad `0..=127`.
    pub velocity: u8,
}

impl ScoreNote {
    /// Crea una nota; la velocidad se acota a `127`.
    pub fn new(pitch: Pitch, start: f32, duration: f32, velocity: u8) -> Self {
        Self { pitch, start, duration, velocity: velocity.min(127) }
    }

    /// Pulso en que la nota termina.
    pub fn end(self) -> f32 {
        self.start + self.duration
    }

    /// `true` si la nota está sonando en el pulso `beat`.
    pub fn sounds_at(self, beat: f32) -> bool {
        beat >= self.start && beat < self.end()
    }
}

/// Un punto de automación: un valor anclado a un beat concreto. Los
/// renderers interpolan linealmente entre puntos consecutivos.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub beat: f32,
    pub value: f32,
}

/// Curva de automación: lista de puntos ordenados por `beat`. Cuando una
/// pista tiene una `AutomationLane` activa para un parámetro, el render
/// la consulta en el `note.start` de cada nota — el campo estático
/// (p.ej. `Track.volume`) sólo se usa antes del primer punto.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AutomationLane {
    /// Puntos ordenados por beat ascendente. Manipular siempre vía
    /// [`AutomationLane::add_point`] para mantener el invariante.
    pub points: Vec<AutomationPoint>,
}

impl AutomationLane {
    /// Inserta o reemplaza un punto en `beat` con `value`. Mantiene el
    /// orden por beat; si ya existía un punto en ese beat exacto, su
    /// valor se sobreescribe (no se duplica).
    pub fn add_point(&mut self, beat: f32, value: f32) {
        let i = self.points.partition_point(|p| p.beat < beat);
        if i < self.points.len() && (self.points[i].beat - beat).abs() < 1e-6 {
            self.points[i].value = value;
        } else {
            self.points.insert(i, AutomationPoint { beat, value });
        }
    }

    /// Valor de la curva en `beat`. Si la lane está vacía devuelve
    /// `default` (el valor estático del track). Antes del primer punto
    /// se mantiene el valor del primer punto; después del último se
    /// mantiene el valor del último. Interpolación lineal en el medio.
    pub fn value_at(&self, beat: f32, default: f32) -> f32 {
        match self.points.as_slice() {
            [] => default,
            [only] => only.value,
            [first, .., last] if beat <= first.beat => first.value,
            [.., last] if beat >= last.beat => last.value,
            _ => {
                // Busca el primer punto cuyo beat > `beat`. Su predecesor
                // es el punto a izquierda — entre ambos interpolamos.
                let i = self.points.partition_point(|p| p.beat <= beat);
                let prev = self.points[i - 1];
                let next = self.points[i];
                let span = (next.beat - prev.beat).max(1e-6);
                let t = (beat - prev.beat) / span;
                prev.value + t * (next.value - prev.value)
            }
        }
    }

    /// Cantidad de puntos en la curva.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// `true` si no hay puntos definidos.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

/// Una pista monofónica o polifónica: notas ordenadas por inicio.
///
/// Los campos del mixer (`volume`, `mute`, `solo`) usan `serde(default)`
/// para que los archivos `.takiy.json` escritos antes de F3 carguen sin
/// migración: faltantes equivalen a "track audible al 100%".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub name: String,
    notes: Vec<ScoreNote>,
    /// Ganancia lineal `[0, 1.5]`. `1.0` = unidad. Default `1.0`.
    #[serde(default = "default_volume")]
    pub volume: f32,
    /// `true` = la pista no aporta señal al render. Default `false`.
    #[serde(default)]
    pub mute: bool,
    /// `true` = la pista forma parte del bus solo. Si alguna pista del
    /// score está en solo, sólo las solo se mezclan en el render.
    /// Default `false`.
    #[serde(default)]
    pub solo: bool,
    /// Panorámica estéreo `[-1, 1]`. `-1` = todo izquierda, `0` = centro,
    /// `1` = todo derecha. Aplica equal-power. Default `0.0` (centro).
    #[serde(default)]
    pub pan: f32,
    /// Automación de volumen. Si está presente y tiene ≥1 puntos, el
    /// renderer la usa en lugar del campo estático `volume`. `None` o
    /// vacía → cae al `volume` estático.
    #[serde(default)]
    pub volume_automation: Option<AutomationLane>,
    /// Automación de pan, mismo criterio que `volume_automation`.
    #[serde(default)]
    pub pan_automation: Option<AutomationLane>,
}

fn default_volume() -> f32 {
    1.0
}

impl Default for Track {
    fn default() -> Self {
        Self {
            name: String::new(),
            notes: Vec::new(),
            volume: 1.0,
            mute: false,
            solo: false,
            pan: 0.0,
            volume_automation: None,
            pan_automation: None,
        }
    }
}

impl Track {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            notes: Vec::new(),
            volume: 1.0,
            mute: false,
            solo: false,
            pan: 0.0,
            volume_automation: None,
            pan_automation: None,
        }
    }

    /// Volumen efectivo en `beat`. Si hay automación con puntos,
    /// devuelve la curva interpolada; si no, el `volume` estático.
    /// Esto es lo que el renderer consulta al mezclar cada nota.
    pub fn volume_at(&self, beat: f32) -> f32 {
        match self.volume_automation.as_ref() {
            Some(lane) if !lane.is_empty() => lane.value_at(beat, self.volume),
            _ => self.volume,
        }
    }

    /// Pan efectivo en `beat`, mismo criterio que `volume_at`.
    pub fn pan_at(&self, beat: f32) -> f32 {
        match self.pan_automation.as_ref() {
            Some(lane) if !lane.is_empty() => lane.value_at(beat, self.pan),
            _ => self.pan,
        }
    }

    /// Ganancias equal-power para el par estéreo evaluadas en `beat`,
    /// honrando la automación de pan si está activa.
    pub fn pan_gains_at(&self, beat: f32) -> (f32, f32) {
        let p = self.pan_at(beat).clamp(-1.0, 1.0);
        let theta = (p + 1.0) * std::f32::consts::FRAC_PI_4;
        (theta.cos(), theta.sin())
    }

    /// Ganancia equal-power para el par estéreo dado el `pan` actual.
    /// `pan = -1` → (1, 0); `pan = 0` → (√½, √½); `pan = 1` → (0, 1).
    /// Conserva la potencia total (`gL² + gR² = 1`).
    pub fn pan_gains(&self) -> (f32, f32) {
        let p = self.pan.clamp(-1.0, 1.0);
        let theta = (p + 1.0) * std::f32::consts::FRAC_PI_4; // [0, π/2]
        (theta.cos(), theta.sin())
    }

    /// Inserta una nota manteniendo el orden por pulso de inicio.
    pub fn add(&mut self, note: ScoreNote) {
        let pos = self
            .notes
            .partition_point(|n| n.start <= note.start);
        self.notes.insert(pos, note);
    }

    /// Notas de la pista, ordenadas por inicio.
    pub fn notes(&self) -> &[ScoreNote] {
        &self.notes
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Pulso en que termina la última nota (0 si la pista está vacía).
    pub fn duration(&self) -> f32 {
        self.notes.iter().map(|n| n.end()).fold(0.0, f32::max)
    }

    /// Notas que suenan en el pulso `beat`.
    pub fn notes_at(&self, beat: f32) -> Vec<&ScoreNote> {
        self.notes.iter().filter(|n| n.sounds_at(beat)).collect()
    }

    /// Quita la nota en el índice dado. Devuelve la nota eliminada o
    /// `None` si el índice estaba fuera de rango.
    pub fn remove(&mut self, idx: usize) -> Option<ScoreNote> {
        if idx >= self.notes.len() {
            return None;
        }
        Some(self.notes.remove(idx))
    }

    /// Transpone la pista entera. Es atómico: si alguna nota se saldría
    /// del rango MIDI, no se cambia nada y devuelve `false`.
    pub fn transpose(&mut self, semitones: i32) -> bool {
        if self.notes.iter().any(|n| n.pitch.transpose(semitones).is_none()) {
            return false;
        }
        for n in &mut self.notes {
            n.pitch = n.pitch.transpose(semitones).expect("ya verificado");
        }
        true
    }
}

/// Parámetros de un delay master simple por feedback. Cuando un
/// [`Score`] lleva `master_delay = Some(_)`, el renderer aplica una
/// línea de retardo con realimentación al mix final.
///
/// El feedback se *clampa* al render a `< 1.0` para que la cola decaiga
/// — un feedback = 1 acumula amplitud y diverge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DelayParams {
    /// Tiempo entre ecos en pulsos. `0.5` = corchea a 4/4, `1.0` =
    /// negra, `0.75` = corchea con puntillo, etc. El renderer convierte
    /// a samples usando `score.tempo_bpm`.
    pub time_beats: f32,
    /// Realimentación `[0.0, 0.95]`. `0` = una sola repetición (slap),
    /// `0.5` = ~3 ecos audibles, `0.9` = cola larga. Más de `0.95` no
    /// se permite — divergiría.
    pub feedback: f32,
    /// Mezcla wet `[0.0, 1.0]`. `0` = sin efecto (dry puro),
    /// `0.5` = parejo, `1.0` = sólo wet (sin dry).
    pub mix: f32,
}

impl Default for DelayParams {
    /// Preset razonable para "encender el delay y oír algo útil": una
    /// corchea con feedback bajo y mezcla discreta. Está pensado para
    /// el toggle `Alt+D` de la UI.
    fn default() -> Self {
        Self { time_beats: 0.5, feedback: 0.35, mix: 0.25 }
    }
}

/// Parámetros de un reverb tipo Schroeder simple (4 combs paralelos +
/// 2 allpasses en serie). Cuando un [`Score`] lleva `master_reverb =
/// Some(_)`, el renderer aplica esta cola tras el delay y antes del
/// normalize.
///
/// El feedback de los combs se deriva linealmente de `room_size`; el
/// damping atenúa las frecuencias altas en el bucle de feedback para
/// emular una sala absorbente (más damping = más oscura la cola).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ReverbParams {
    /// Tamaño de la sala `[0.0, 1.0]`. Internamente mapea a feedback
    /// de los combs en `0.7 + room_size * 0.28` → `[0.70, 0.98]`.
    /// Valores típicos: 0.3 cuarto, 0.6 sala, 0.9 catedral.
    pub room_size: f32,
    /// Damping `[0.0, 1.0]` — `0` = cola brillante (sin filtro),
    /// `1.0` = cola muy oscura (low-pass agresivo en el feedback).
    pub damping: f32,
    /// Mezcla wet `[0.0, 1.0]`. `0` = sin efecto, `1.0` = sólo wet.
    pub mix: f32,
}

impl Default for ReverbParams {
    /// Sala mediana con presencia discreta — sirve como punto de
    /// partida para escuchar el efecto sin invadir la mezcla.
    fn default() -> Self {
        Self { room_size: 0.5, damping: 0.5, mix: 0.25 }
    }
}

/// Una partitura: un tempo, una tonalidad opcional y varias pistas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Score {
    /// Pulsos por minuto.
    pub tempo_bpm: f32,
    /// Tonalidad activa para el editor (resalta filas en escala,
    /// permite snap a tonalidad). `None` = sin tonalidad declarada.
    /// `serde(default)` para que los `.takiy.json` pre-F6 carguen igual.
    #[serde(default)]
    pub key: Option<Scale>,
    /// Delay aplicado al bus master al final del render. `None` =
    /// bypass (default). `serde(default)` mantiene compat con archivos
    /// pre-F8 — un score sin esta clave se carga con delay apagado y
    /// produce el mismo render byte-exact que antes.
    #[serde(default)]
    pub master_delay: Option<DelayParams>,
    /// Reverb aplicado al bus master *después* del delay. Mismo
    /// criterio de compat con `serde(default)` para scores pre-reverb.
    #[serde(default)]
    pub master_reverb: Option<ReverbParams>,
    tracks: Vec<Track>,
}

impl Score {
    /// Partitura vacía con el tempo dado.
    pub fn new(tempo_bpm: f32) -> Self {
        Self {
            tempo_bpm,
            key: None,
            master_delay: None,
            master_reverb: None,
            tracks: Vec::new(),
        }
    }

    /// Añade una pista y devuelve su índice.
    pub fn add_track(&mut self, track: Track) -> usize {
        self.tracks.push(track);
        self.tracks.len() - 1
    }

    pub fn track(&self, index: usize) -> Option<&Track> {
        self.tracks.get(index)
    }

    pub fn track_mut(&mut self, index: usize) -> Option<&mut Track> {
        self.tracks.get_mut(index)
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Elimina la pista en `index`. Devuelve la pista quitada, o
    /// `None` si el índice no existe.
    pub fn remove_track(&mut self, index: usize) -> Option<Track> {
        if index >= self.tracks.len() {
            return None;
        }
        Some(self.tracks.remove(index))
    }

    /// Duración en pulsos — la pista más larga.
    pub fn duration_beats(&self) -> f32 {
        self.tracks.iter().map(|t| t.duration()).fold(0.0, f32::max)
    }

    /// `true` si al menos una pista está en solo. Útil al renderizar
    /// para decidir si filtrar las no-solo.
    pub fn has_solo(&self) -> bool {
        self.tracks.iter().any(|t| t.solo)
    }

    /// `true` si la pista en `index` debe sonar según el bus mute/solo
    /// global: si hay alguna en solo, sólo suenan las solo; las muteadas
    /// siempre son silenciadas.
    pub fn track_is_audible(&self, index: usize) -> bool {
        let Some(t) = self.tracks.get(index) else { return false; };
        if t.mute {
            return false;
        }
        if self.has_solo() {
            return t.solo;
        }
        true
    }

    /// Duración en segundos según el tempo.
    pub fn duration_seconds(&self) -> f32 {
        if self.tempo_bpm <= 0.0 {
            return 0.0;
        }
        self.duration_beats() * 60.0 / self.tempo_bpm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::{Pitch, PitchClass};

    fn note(class: PitchClass, start: f32) -> ScoreNote {
        ScoreNote::new(Pitch::from_class_octave(class, 4).unwrap(), start, 1.0, 100)
    }

    #[test]
    fn automation_empty_returns_default() {
        let lane = AutomationLane::default();
        assert!((lane.value_at(0.0, 0.7) - 0.7).abs() < 1e-6);
        assert!((lane.value_at(100.0, 0.7) - 0.7).abs() < 1e-6);
    }

    #[test]
    fn automation_single_point_is_constant() {
        let mut lane = AutomationLane::default();
        lane.add_point(4.0, 0.3);
        // Antes, igual y después del único punto siempre da su valor.
        assert!((lane.value_at(0.0, 0.7) - 0.3).abs() < 1e-6);
        assert!((lane.value_at(4.0, 0.7) - 0.3).abs() < 1e-6);
        assert!((lane.value_at(10.0, 0.7) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn automation_two_points_interpolate_linearly() {
        let mut lane = AutomationLane::default();
        lane.add_point(0.0, 0.0);
        lane.add_point(10.0, 1.0);
        assert!((lane.value_at(0.0, 0.0) - 0.0).abs() < 1e-6);
        assert!((lane.value_at(5.0, 0.0) - 0.5).abs() < 1e-6, "interpolación al 50%");
        assert!((lane.value_at(10.0, 0.0) - 1.0).abs() < 1e-6);
        // Antes del primero / después del último → clamp.
        assert!((lane.value_at(-1.0, 0.0) - 0.0).abs() < 1e-6);
        assert!((lane.value_at(20.0, 0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn automation_add_point_keeps_order_and_replaces_duplicates() {
        let mut lane = AutomationLane::default();
        lane.add_point(2.0, 0.5);
        lane.add_point(0.0, 0.1);
        lane.add_point(1.0, 0.3);
        assert_eq!(lane.points.len(), 3);
        assert!(lane.points.windows(2).all(|w| w[0].beat <= w[1].beat));
        // Reemplazar — no duplicar.
        lane.add_point(1.0, 0.9);
        assert_eq!(lane.points.len(), 3);
        assert!((lane.points[1].value - 0.9).abs() < 1e-6);
    }

    #[test]
    fn track_volume_at_falls_back_to_static_when_lane_empty() {
        let mut t = Track::new("a");
        t.volume = 0.8;
        // Sin lane → static.
        assert!((t.volume_at(5.0) - 0.8).abs() < 1e-6);
        // Lane vacía → también static.
        t.volume_automation = Some(AutomationLane::default());
        assert!((t.volume_at(5.0) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn track_pan_gains_at_honor_automation() {
        let mut t = Track::new("a");
        t.pan = 0.0;
        let mut lane = AutomationLane::default();
        lane.add_point(0.0, -1.0); // full left
        lane.add_point(10.0, 1.0); // full right
        t.pan_automation = Some(lane);
        let (l0, r0) = t.pan_gains_at(0.0);
        assert!(l0 > 0.99 && r0 < 0.01, "(l, r) = ({l0}, {r0}) en pan -1");
        let (l_mid, r_mid) = t.pan_gains_at(5.0);
        // En pan 0 (centro): cos(π/4) = sin(π/4) ≈ 0.707
        assert!((l_mid - r_mid).abs() < 0.05, "centro casi simétrico");
    }

    #[test]
    fn add_keeps_notes_sorted_by_start() {
        let mut t = Track::new("melodía");
        t.add(note(PitchClass::E, 2.0));
        t.add(note(PitchClass::C, 0.0));
        t.add(note(PitchClass::D, 1.0));
        let starts: Vec<f32> = t.notes().iter().map(|n| n.start).collect();
        assert_eq!(starts, vec![0.0, 1.0, 2.0]);
    }

    #[test]
    fn duration_is_end_of_last_note() {
        let mut t = Track::new("x");
        t.add(note(PitchClass::C, 0.0));
        t.add(note(PitchClass::G, 3.0)); // termina en 4.0
        assert_eq!(t.duration(), 4.0);
    }

    #[test]
    fn notes_at_finds_sounding_notes() {
        let mut t = Track::new("x");
        t.add(ScoreNote::new(Pitch::MIDDLE_C, 0.0, 2.0, 80));
        t.add(ScoreNote::new(Pitch::A4, 1.0, 2.0, 80));
        // En el pulso 1.5 ambas suenan; en 2.5 sólo la segunda.
        assert_eq!(t.notes_at(1.5).len(), 2);
        assert_eq!(t.notes_at(2.5).len(), 1);
        assert_eq!(t.notes_at(5.0).len(), 0);
    }

    #[test]
    fn transpose_is_atomic_on_overflow() {
        let mut t = Track::new("x");
        t.add(ScoreNote::new(Pitch::from_midi(120).unwrap(), 0.0, 1.0, 80));
        // +10 sacaría la nota del rango → no cambia nada.
        assert!(!t.transpose(10));
        assert_eq!(t.notes()[0].pitch.midi(), 120);
        // +5 sí cabe.
        assert!(t.transpose(5));
        assert_eq!(t.notes()[0].pitch.midi(), 125);
    }

    #[test]
    fn velocity_is_clamped() {
        let n = ScoreNote::new(Pitch::MIDDLE_C, 0.0, 1.0, 200);
        assert_eq!(n.velocity, 127);
    }

    #[test]
    fn remove_takes_note_at_index_and_leaves_rest_sorted() {
        let mut t = Track::new("x");
        t.add(note(PitchClass::C, 0.0));
        t.add(note(PitchClass::D, 1.0));
        t.add(note(PitchClass::E, 2.0));
        let gone = t.remove(1).expect("idx 1 existe");
        assert!((gone.start - 1.0).abs() < 1e-6);
        let starts: Vec<f32> = t.notes().iter().map(|n| n.start).collect();
        assert_eq!(starts, vec![0.0, 2.0]);
        // Fuera de rango: no rompe.
        assert!(t.remove(99).is_none());
    }

    #[test]
    fn score_duration_in_seconds_follows_tempo() {
        let mut s = Score::new(120.0); // 120 bpm → 2 pulsos por segundo
        let mut t = Track::new("x");
        t.add(ScoreNote::new(Pitch::MIDDLE_C, 0.0, 8.0, 100));
        s.add_track(t);
        assert_eq!(s.duration_beats(), 8.0);
        // 8 pulsos a 120 bpm = 4 segundos.
        assert!((s.duration_seconds() - 4.0).abs() < 1e-4);
    }

    #[test]
    fn track_defaults_to_audible_at_unit_gain() {
        let t = Track::new("a");
        assert_eq!(t.volume, 1.0);
        assert!(!t.mute);
        assert!(!t.solo);
    }

    #[test]
    fn track_serde_with_missing_mixer_fields_uses_defaults() {
        // JSON sin volume/mute/solo/pan (formato pre-F3) debe cargar bien.
        let json = r#"{"name":"old","notes":[]}"#;
        let t: Track = serde_json::from_str(json).unwrap();
        assert_eq!(t.name, "old");
        assert_eq!(t.volume, 1.0);
        assert!(!t.mute);
        assert!(!t.solo);
        assert_eq!(t.pan, 0.0);
    }

    #[test]
    fn pan_gains_equal_power_at_center_and_extremes() {
        let mut t = Track::new("a");
        // Centro: misma ganancia en ambos.
        t.pan = 0.0;
        let (l, r) = t.pan_gains();
        assert!((l - r).abs() < 1e-6);
        assert!((l * l + r * r - 1.0).abs() < 1e-5);
        // Izquierda total.
        t.pan = -1.0;
        let (l, r) = t.pan_gains();
        assert!((l - 1.0).abs() < 1e-6);
        assert!(r.abs() < 1e-6);
        // Derecha total.
        t.pan = 1.0;
        let (l, r) = t.pan_gains();
        assert!(l.abs() < 1e-6);
        assert!((r - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pan_gains_clamps_out_of_range_input() {
        let mut t = Track::new("a");
        t.pan = 2.5; // fuera del rango
        let (l, r) = t.pan_gains();
        // Debería tratarse como pan = 1 (todo derecha).
        assert!(l.abs() < 1e-6);
        assert!((r - 1.0).abs() < 1e-6);
    }

    #[test]
    fn track_is_audible_respects_mute() {
        let mut s = Score::new(120.0);
        let mut t = Track::new("a");
        t.add(note(PitchClass::C, 0.0));
        s.add_track(t);
        assert!(s.track_is_audible(0));
        s.track_mut(0).unwrap().mute = true;
        assert!(!s.track_is_audible(0));
    }

    #[test]
    fn solo_filters_other_tracks_but_not_solo_track() {
        let mut s = Score::new(120.0);
        let mut a = Track::new("a");
        a.add(note(PitchClass::C, 0.0));
        s.add_track(a);
        let mut b = Track::new("b");
        b.add(note(PitchClass::D, 0.0));
        s.add_track(b);
        s.track_mut(0).unwrap().solo = true;
        assert!(s.has_solo());
        assert!(s.track_is_audible(0));
        assert!(!s.track_is_audible(1));
    }

    #[test]
    fn mute_overrides_solo_on_same_track() {
        let mut s = Score::new(120.0);
        let mut a = Track::new("a");
        a.add(note(PitchClass::C, 0.0));
        s.add_track(a);
        let mut b = Track::new("b");
        b.add(note(PitchClass::D, 0.0));
        s.add_track(b);
        s.track_mut(0).unwrap().solo = true;
        s.track_mut(0).unwrap().mute = true;
        // Aunque esté solo, mute la silencia. Otras pistas siguen filtradas.
        assert!(!s.track_is_audible(0));
        assert!(!s.track_is_audible(1));
    }

    #[test]
    fn score_serde_with_missing_key_uses_default_none() {
        // Score JSON pre-F6 (sin key).
        let json = r#"{"tempo_bpm":96.0,"tracks":[]}"#;
        let s: Score = serde_json::from_str(json).unwrap();
        assert!((s.tempo_bpm - 96.0).abs() < 1e-6);
        assert!(s.key.is_none());
    }

    #[test]
    fn score_with_key_roundtrips_via_serde() {
        use crate::pitch::PitchClass;
        let mut s = Score::new(120.0);
        s.key = Some(crate::scale::Scale::major(PitchClass::G));
        let json = serde_json::to_string(&s).unwrap();
        let back: Score = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn score_duration_is_the_longest_track() {
        let mut s = Score::new(100.0);
        let mut a = Track::new("a");
        a.add(ScoreNote::new(Pitch::MIDDLE_C, 0.0, 2.0, 90));
        let mut b = Track::new("b");
        b.add(ScoreNote::new(Pitch::A4, 0.0, 6.0, 90));
        s.add_track(a);
        s.add_track(b);
        assert_eq!(s.duration_beats(), 6.0);
    }
}
