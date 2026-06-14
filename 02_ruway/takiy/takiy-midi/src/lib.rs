//! `takiy-midi` — interop con Standard MIDI File (.mid).
//!
//! Bridge bi-direccional entre [`takiy_core::Score`] y SMF tipo 0/1. El
//! formato nativo de takiy (`.takiy.json`) preserva más detalle (pan,
//! volume, mute/solo, snap, etc.); el SMF guarda sólo lo que tiene
//! sentido para interop con DAWs:
//!
//! - Tempo del score → meta tempo en la pista de tiempo (pista 0 en
//!   tipo 1, intercalado en tipo 0).
//! - Cada `Track` → un MTrk con program change (heurístico por nombre,
//!   via [`takiy_core`] gm helper en el binario; acá pasamos `0` por
//!   default), name event, y un par on/off por nota.
//! - Volume y pan se mapean a CC#7 y CC#10 si difieren de los defaults.
//! - Mute/solo no tienen representación SMF directa — se ignoran al
//!   escribir (la pista igual emite sus notas; el DAW puede silenciarla
//!   manualmente). Solo afecta a la fidelidad del round-trip, no a la
//!   funcionalidad.
//!
//! División por defecto: 480 PPQ (ticks per quarter-note), un valor
//! común que cubre tresillos y semicorcheas con precisión sobrada.

#![forbid(unsafe_code)]

use midly::num::{u15, u24, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track as MidiTrack, TrackEvent, TrackEventKind};
use takiy_core::{Pitch, Score, ScoreNote, Track};

/// Ticks por quarter-note (pulso). 480 cubre semicorcheas y tresillos
/// con holgura sin inflar el archivo.
pub const PPQ: u16 = 480;

/// Errores al parsear un SMF.
#[derive(Debug)]
pub enum ParseError {
    /// Falla del parser de midly (header malformado, chunks invalidos, etc.).
    Midly(midly::Error),
    /// El archivo usa SMPTE timing en lugar de PPQ — no soportado hoy
    /// (tempo se expresa por ticks; SMPTE requiere lookups por frame).
    SmpteTiming,
    /// Una nota MIDI cayó fuera del rango `Pitch` válido (0..=127). Sólo
    /// puede ocurrir si el SMF está corrupto — midly ya cuantifica al
    /// rango legal.
    InvalidPitch(u8),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Midly(e) => write!(f, "SMF inválido: {e}"),
            Self::SmpteTiming => write!(f, "SMF con timing SMPTE no soportado"),
            Self::InvalidPitch(k) => write!(f, "pitch MIDI fuera de rango: {k}"),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Midly(e) => Some(e),
            _ => None,
        }
    }
}

impl From<midly::Error> for ParseError {
    fn from(e: midly::Error) -> Self {
        Self::Midly(e)
    }
}

/// Escribe `score` como SMF tipo 1, multi-track. Devuelve el blob crudo
/// listo para `fs::write`. La pista 0 contiene sólo el tempo; las pistas
/// 1..=N son las del score (preservan el orden).
pub fn to_smf(score: &Score) -> Vec<u8> {
    let header = Header::new(Format::Parallel, Timing::Metrical(u15::new(PPQ)));
    let mut tracks: Vec<MidiTrack> = Vec::with_capacity(score.tracks().len() + 1);

    // Pre-allocamos los nombres en un buffer paralelo: midly's
    // `MetaMessage::TrackName` toma `&'a [u8]` y los TrackEvent/Smf
    // viven hasta el `smf.write()` de más abajo. Sin esto la única
    // alternativa era `Box::leak` (deuda histórica de este crate);
    // así no leakeamos nada — el Vec entero se libera al return.
    let name_buffers: Vec<Vec<u8>> = score
        .tracks()
        .iter()
        .map(|t| t.name.as_bytes().to_vec())
        .collect();

    // Pista 0: sólo tempo + end-of-track. midly requiere u24 (3 bytes
    // microsegundos por quarter-note).
    let microseconds_per_quarter = ((60_000_000.0 / score.tempo_bpm.max(1e-6)) as u32).min(0xFFFFFF);
    let mut tempo_track: MidiTrack = Vec::new();
    tempo_track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(microseconds_per_quarter))),
    });
    tempo_track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    tracks.push(tempo_track);

    // Pistas N: una por Track con name + (opcional vol/pan CC) + notas.
    for (idx, track) in score.tracks().iter().enumerate() {
        let channel = midi_channel_for_track(idx);
        let mut events: Vec<TrackEvent> = Vec::with_capacity(track.notes().len() * 2 + 4);

        // Track name (meta 0x03). El slice apunta al buffer pre-allocado
        // de arriba (vive hasta el `smf.write()` final).
        let name_bytes: &[u8] = &name_buffers[idx];
        if !name_bytes.is_empty() {
            events.push(TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Meta(MetaMessage::TrackName(name_bytes)),
            });
        }

        // CC#7 Volume si != 1.0 (default). El rango SMF de CC#7 es 0..=127
        // (i.e. 0% a 127%), así que volúmenes lógicos > 1.27 se truncan
        // a 127 — el roundtrip a Score los lee de vuelta como 1.27. Es
        // pérdida silenciosa por límite del formato, no un bug del bridge.
        if (track.volume - 1.0).abs() > 1e-3 {
            let vol = ((track.volume * 100.0).round() as i32).clamp(0, 127) as u8;
            events.push(TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Midi {
                    channel: u4::new(channel),
                    message: MidiMessage::Controller {
                        controller: u7::new(7),
                        value: u7::new(vol),
                    },
                },
            });
        }
        // CC#10 Pan si != 0.0.
        if track.pan.abs() > 1e-3 {
            let pan = (((track.pan.clamp(-1.0, 1.0) + 1.0) * 0.5 * 127.0).round() as i32)
                .clamp(0, 127) as u8;
            events.push(TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Midi {
                    channel: u4::new(channel),
                    message: MidiMessage::Controller {
                        controller: u7::new(10),
                        value: u7::new(pan),
                    },
                },
            });
        }

        // Build event list at absolute tick positions, then convert to deltas.
        // Cada nota produce 2 eventos (on/off) en orden estable.
        let mut absolute: Vec<(u32, TrackEventKind)> = Vec::with_capacity(track.notes().len() * 2);
        for note in track.notes() {
            let on_tick = (note.start * PPQ as f32).round() as u32;
            let off_tick = ((note.start + note.duration) * PPQ as f32).round() as u32;
            absolute.push((on_tick, TrackEventKind::Midi {
                channel: u4::new(channel),
                message: MidiMessage::NoteOn {
                    key: u7::new(note.pitch.midi().min(127)),
                    vel: u7::new(note.velocity.min(127)),
                },
            }));
            // Off un tick después si on == off (notas de duración cero).
            absolute.push((on_tick.max(off_tick).max(on_tick + 1), TrackEventKind::Midi {
                channel: u4::new(channel),
                message: MidiMessage::NoteOff {
                    key: u7::new(note.pitch.midi().min(127)),
                    vel: u7::new(64), // Off velocity convencional.
                },
            }));
        }
        // Sort estable por tick — preserva el orden de inserción cuando
        // dos eventos caen en el mismo tick.
        absolute.sort_by_key(|(t, _)| *t);

        let mut cursor: u32 = 0;
        for (tick, kind) in absolute {
            let delta = tick.saturating_sub(cursor);
            cursor = tick;
            events.push(TrackEvent { delta: u28::new(delta), kind });
        }
        events.push(TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        });

        tracks.push(events);
    }

    let smf = Smf { header, tracks };
    let mut buf = Vec::with_capacity(256);
    smf.write(&mut buf).expect("midly::write a Vec no falla");
    buf
}

/// Mapea índice de pista a canal MIDI saltando el canal 9 (drums GM).
/// Idéntico a `takiy_synth::soundfont_multi::channel_for_track` pero
/// devuelve `u8` (rango `0..=15`).
fn midi_channel_for_track(track_idx: usize) -> u8 {
    let i = track_idx % 15;
    if i >= 9 { (i + 1) as u8 } else { i as u8 }
}

/// Parsea un blob SMF y reconstruye un `Score`. Soporta formato 0 (todo
/// en una pista) y 1 (pistas separadas). El primer tempo encontrado se
/// usa como `Score::tempo_bpm`; cambios posteriores se ignoran (takiy no
/// modela automatización de tempo).
pub fn from_smf(bytes: &[u8]) -> Result<Score, ParseError> {
    let smf = Smf::parse(bytes)?;

    // Buscamos el primer tempo en la pista 0 (formato 1) o en la única
    // pista (formato 0); default 120 bpm si no aparece. Va antes que el PPQ
    // porque el timing SMPTE lo necesita para convertir a negras.
    let mut tempo_bpm = 120.0_f32;
    if let Some(track) = smf.tracks.first() {
        for ev in track {
            if let TrackEventKind::Meta(MetaMessage::Tempo(us)) = &ev.kind {
                let us_per_quarter = us.as_int() as f32;
                if us_per_quarter > 0.0 {
                    tempo_bpm = 60_000_000.0 / us_per_quarter;
                    break;
                }
            }
        }
    }

    // Ticks por negra (PPQ). Con timing **métrico** el header lo da directo.
    // Con timing **SMPTE** el header da una tasa ABSOLUTA de ticks por
    // segundo (`fps × ticks_por_frame`); la convertimos a "ticks por negra"
    // con el tempo —`ppq = ticks/seg × seg/negra = fps·tpf × 60/bpm`— para
    // que el resto del pipeline (que trabaja en negras = tick/ppq) no cambie.
    let ppq = match smf.header.timing {
        Timing::Metrical(p) => p.as_int() as u32,
        Timing::Timecode(fps, ticks_per_frame) => {
            let ticks_per_sec = fps.as_f32() * ticks_per_frame as f32;
            (ticks_per_sec * 60.0 / tempo_bpm.max(1e-6)).round() as u32
        }
    };
    let ppq = ppq.max(1);

    let mut score = Score::new(tempo_bpm);

    // Para SMF tipo 1, cada pista (excepto la 0 que sólo tiene tempo)
    // es un Track. Para tipo 0, hay una sola pista que mezcla todo en
    // múltiples canales — la dividimos por canal.
    let is_format_0 = matches!(smf.header.format, Format::SingleTrack);

    if is_format_0 {
        // Recoger eventos por canal.
        let mut by_channel: std::collections::HashMap<u8, Vec<(u32, MidiMessage, u8)>> =
            std::collections::HashMap::new();
        let mut cursor = 0u32;
        if let Some(track) = smf.tracks.first() {
            for ev in track {
                cursor = cursor.saturating_add(ev.delta.as_int());
                if let TrackEventKind::Midi { channel, message } = &ev.kind {
                    by_channel
                        .entry(channel.as_int())
                        .or_default()
                        .push((cursor, *message, 0));
                }
            }
        }
        let mut channels: Vec<u8> = by_channel.keys().copied().collect();
        channels.sort();
        for ch in channels {
            let mut t = Track::new(format!("ch{ch}"));
            collect_notes_into_track(&mut t, &by_channel[&ch], ppq);
            score.add_track(t);
        }
    } else {
        // Formato 1: cada pista (a partir de la 1, la 0 es tempo) → Track.
        for (idx, raw) in smf.tracks.iter().enumerate() {
            if idx == 0 {
                continue;
            }
            let mut name = format!("track {idx}");
            let mut events: Vec<(u32, MidiMessage, u8)> = Vec::new();
            let mut cursor = 0u32;
            let mut volume_cc: Option<u8> = None;
            let mut pan_cc: Option<u8> = None;
            for ev in raw {
                cursor = cursor.saturating_add(ev.delta.as_int());
                match &ev.kind {
                    TrackEventKind::Meta(MetaMessage::TrackName(bytes)) => {
                        if let Ok(s) = std::str::from_utf8(bytes) {
                            name = s.to_string();
                        }
                    }
                    TrackEventKind::Midi { channel, message } => {
                        if let MidiMessage::Controller { controller, value } = message {
                            match controller.as_int() {
                                7 => volume_cc = Some(value.as_int()),
                                10 => pan_cc = Some(value.as_int()),
                                _ => {}
                            }
                        }
                        events.push((cursor, *message, channel.as_int()));
                    }
                    _ => {}
                }
            }
            let mut t = Track::new(name);
            if let Some(v) = volume_cc {
                t.volume = (v as f32 / 100.0).clamp(0.0, 1.5);
            }
            if let Some(p) = pan_cc {
                t.pan = (p as f32 / 127.0 * 2.0 - 1.0).clamp(-1.0, 1.0);
            }
            collect_notes_into_track(&mut t, &events, ppq);
            if !t.notes().is_empty() {
                score.add_track(t);
            }
        }
    }

    Ok(score)
}

/// Convierte un stream lineal de eventos MIDI (`MidiMessage` en tick
/// absoluto) a `ScoreNote`s en `track`. Pairs note_on con note_off por
/// (canal, key); note_on con vel=0 cuenta como off (convención común en
/// SMF para chordable note-off).
///
/// **Notas solapadas en el mismo pitch**: cuando dos `NoteOn` del mismo
/// `(canal, key)` se acumulan antes del primer `NoteOff` (clusters,
/// re-disparos rápidos, secuenciadores que repiquetean), la cola se
/// despacha **FIFO**: el siguiente `NoteOff` cierra la nota MÁS VIEJA.
/// Lo contrario (LIFO) cortaba la nota original a la duración de la más
/// reciente; FIFO preserva ambas duraciones y es lo que hacen los DAWs
/// mainstream.
fn collect_notes_into_track(track: &mut Track, events: &[(u32, MidiMessage, u8)], ppq: u32) {
    use std::collections::{HashMap, VecDeque};
    // Cola FIFO de note_ons abiertos por (canal, key). Si nunca hay
    // overlap, cada Deque tiene a lo sumo 1 entrada y el costo es el
    // mismo que el del HashMap viejo.
    let mut open: HashMap<(u8, u8), VecDeque<(u32, u8)>> = HashMap::new();
    let inv_ppq = 1.0 / ppq as f32;
    for (tick, msg, ch) in events {
        match *msg {
            MidiMessage::NoteOn { key, vel } => {
                let k = key.as_int();
                let v = vel.as_int();
                if v == 0 {
                    // note_on con vel 0 = note_off implícito.
                    if let Some(q) = open.get_mut(&(*ch, k)) {
                        if let Some((on_tick, on_vel)) = q.pop_front() {
                            push_note(track, on_tick, *tick, k, on_vel, inv_ppq);
                        }
                    }
                } else {
                    open.entry((*ch, k)).or_default().push_back((*tick, v));
                }
            }
            MidiMessage::NoteOff { key, vel: _ } => {
                let k = key.as_int();
                if let Some(q) = open.get_mut(&(*ch, k)) {
                    if let Some((on_tick, on_vel)) = q.pop_front() {
                        push_note(track, on_tick, *tick, k, on_vel, inv_ppq);
                    }
                }
            }
            _ => {}
        }
    }
}

fn push_note(track: &mut Track, on_tick: u32, off_tick: u32, key: u8, vel: u8, inv_ppq: f32) {
    if let Some(pitch) = Pitch::from_midi(key) {
        let start = on_tick as f32 * inv_ppq;
        let dur = ((off_tick.max(on_tick + 1) - on_tick) as f32 * inv_ppq).max(1e-3);
        track.add(ScoreNote::new(pitch, start, dur, vel));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use takiy_core::{PitchClass, ScoreNote, Track};

    fn build_demo() -> Score {
        let mut s = Score::new(96.0);
        let mut t = Track::new("melodía");
        t.add(ScoreNote::new(
            Pitch::from_class_octave(PitchClass::C, 4).unwrap(),
            0.0,
            1.0,
            100,
        ));
        t.add(ScoreNote::new(
            Pitch::from_class_octave(PitchClass::E, 4).unwrap(),
            1.0,
            0.5,
            80,
        ));
        t.add(ScoreNote::new(
            Pitch::from_class_octave(PitchClass::G, 4).unwrap(),
            1.5,
            0.5,
            90,
        ));
        s.add_track(t);
        let mut bass = Track::new("bajo");
        bass.add(ScoreNote::new(
            Pitch::from_class_octave(PitchClass::C, 2).unwrap(),
            0.0,
            2.0,
            110,
        ));
        s.add_track(bass);
        s
    }

    #[test]
    fn smf_header_is_format_1_with_two_plus_one_tracks() {
        let bytes = to_smf(&build_demo());
        let smf = Smf::parse(&bytes).unwrap();
        assert_eq!(smf.header.format, Format::Parallel);
        // 1 pista de tempo + 2 pistas de notas.
        assert_eq!(smf.tracks.len(), 3);
    }

    #[test]
    fn roundtrip_preserves_note_count_and_tempo() {
        let original = build_demo();
        let bytes = to_smf(&original);
        let back = from_smf(&bytes).unwrap();
        assert!((back.tempo_bpm - original.tempo_bpm).abs() < 0.5);
        assert_eq!(back.tracks().len(), original.tracks().len());
        for (a, b) in back.tracks().iter().zip(original.tracks().iter()) {
            assert_eq!(a.notes().len(), b.notes().len(), "pista {}", a.name);
        }
    }

    #[test]
    fn roundtrip_preserves_pitch_and_velocity_exactly() {
        let original = build_demo();
        let bytes = to_smf(&original);
        let back = from_smf(&bytes).unwrap();
        for (ta, tb) in back.tracks().iter().zip(original.tracks().iter()) {
            for (na, nb) in ta.notes().iter().zip(tb.notes().iter()) {
                assert_eq!(na.pitch.midi(), nb.pitch.midi());
                assert_eq!(na.velocity, nb.velocity);
            }
        }
    }

    #[test]
    fn roundtrip_preserves_start_and_duration_within_quantization() {
        let original = build_demo();
        let bytes = to_smf(&original);
        let back = from_smf(&bytes).unwrap();
        for (ta, tb) in back.tracks().iter().zip(original.tracks().iter()) {
            for (na, nb) in ta.notes().iter().zip(tb.notes().iter()) {
                // Error de cuantización ≤ 1 tick = 1/480 beat ≈ 2e-3.
                assert!((na.start - nb.start).abs() < 5e-3,
                    "start {} vs {}", na.start, nb.start);
                assert!((na.duration - nb.duration).abs() < 5e-3,
                    "dur {} vs {}", na.duration, nb.duration);
            }
        }
    }

    #[test]
    fn roundtrip_preserves_track_names() {
        let original = build_demo();
        let bytes = to_smf(&original);
        let back = from_smf(&bytes).unwrap();
        assert_eq!(back.track(0).unwrap().name, "melodía");
        assert_eq!(back.track(1).unwrap().name, "bajo");
    }

    #[test]
    fn roundtrip_preserves_volume_and_pan_cc() {
        let mut s = Score::new(120.0);
        let mut t = Track::new("piano");
        t.volume = 0.7;
        t.pan = -0.5;
        t.add(ScoreNote::new(
            Pitch::from_class_octave(PitchClass::A, 4).unwrap(),
            0.0,
            1.0,
            100,
        ));
        s.add_track(t);
        let bytes = to_smf(&s);
        let back = from_smf(&bytes).unwrap();
        let bt = back.track(0).unwrap();
        // 0.7 → CC 70/127; 70/100 = 0.7 al volver.
        assert!((bt.volume - 0.7).abs() < 0.02);
        // pan -0.5 → CC ~32; 32/127 * 2 - 1 = -0.496.
        assert!((bt.pan - (-0.5)).abs() < 0.03);
    }

    #[test]
    fn smpte_timing_se_parsea_a_negras() {
        // SMF con timing SMPTE: Fps25 × 40 ticks/frame = 1000 ticks/seg
        // (1 tick = 1 ms). Con tempo 120 bpm (2 negras/seg) el conversor da
        // ppq = 1000·60/120 = 500 ticks por negra. Una nota de 500 ticks
        // debe leerse como 1.0 negra exacta.
        let header = Header::new(Format::SingleTrack, Timing::Timecode(midly::Fps::Fps25, 40));
        // 120 bpm = 500_000 µs por negra.
        let track = vec![
            TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(500_000))),
            },
            TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Midi {
                    channel: u4::new(0),
                    message: MidiMessage::NoteOn { key: u7::new(60), vel: u7::new(100) },
                },
            },
            TrackEvent {
                delta: u28::new(500),
                kind: TrackEventKind::Midi {
                    channel: u4::new(0),
                    message: MidiMessage::NoteOff { key: u7::new(60), vel: u7::new(0) },
                },
            },
            TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
            },
        ];
        let smf = Smf { header, tracks: vec![track] };
        let mut buf = Vec::new();
        smf.write(&mut buf).unwrap();

        let score = from_smf(&buf).expect("SMPTE ya no se rechaza");
        assert!((score.tempo_bpm - 120.0).abs() < 0.5);
        let notes: Vec<_> = score.tracks().iter().flat_map(|t| t.notes()).collect();
        assert_eq!(notes.len(), 1, "una sola nota");
        assert!(notes[0].start.abs() < 1e-3, "start={}", notes[0].start);
        assert!(
            (notes[0].duration - 1.0).abs() < 0.02,
            "duración SMPTE→negras = {} (esperaba ≈1.0)",
            notes[0].duration
        );
    }

    #[test]
    fn empty_score_roundtrips_to_empty_score() {
        let s = Score::new(140.0);
        let bytes = to_smf(&s);
        let back = from_smf(&bytes).unwrap();
        assert!((back.tempo_bpm - 140.0).abs() < 0.5);
        assert_eq!(back.tracks().len(), 0);
    }

    /// El bug pre-fix: dos NoteOns del mismo pitch antes del primer
    /// NoteOff se acumulaban en un `HashMap` que sólo guardaba el
    /// último → la primera nota se "perdía" al cerrar con el primer
    /// NoteOff (cerraba la segunda en su lugar). Con la cola FIFO
    /// ambas notas roundtripean con sus duraciones intactas.
    #[test]
    fn roundtrip_notas_solapadas_del_mismo_pitch_no_se_pisan() {
        // Dos C4 solapados en el mismo canal: la A va de 0 a 2 s,
        // la B de 1 a 3 s. Sin el fix, la primera salía con duración
        // 1 s (cortada al primer NoteOff de la segunda).
        let mut score = Score::new(120.0);
        let mut t = Track::new("solapado");
        let c4 = Pitch::from_class_octave(PitchClass::C, 4).unwrap();
        t.add(ScoreNote::new(c4, 0.0, 2.0, 100));
        t.add(ScoreNote::new(c4, 1.0, 2.0, 80));
        score.add_track(t);

        let bytes = to_smf(&score);
        let back = from_smf(&bytes).unwrap();
        assert_eq!(back.tracks().len(), 1);
        let recovered = back.tracks()[0].notes();
        assert_eq!(recovered.len(), 2, "ambas notas deben sobrevivir");

        // FIFO: la nota más vieja cierra primero → la primera NoteOff
        // pertenece a la nota A (start 0). Después de ordenar por
        // tiempo de inicio, las duraciones esperadas son 2.0 y 2.0.
        let mut por_inicio: Vec<_> = recovered.iter().collect();
        por_inicio.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
        let dur_a = por_inicio[0].duration;
        let dur_b = por_inicio[1].duration;
        assert!(
            (dur_a - 2.0).abs() < 0.05,
            "nota A: dur esperada 2.0, fue {dur_a}",
        );
        assert!(
            (dur_b - 2.0).abs() < 0.05,
            "nota B: dur esperada 2.0, fue {dur_b}",
        );
    }

    /// El rango SMF de CC#7 limita el volumen a 1.27. Más allá, el
    /// bridge clampea sin perder la pista cualitativamente — esta
    /// prueba documenta el comportamiento para que un cambio futuro
    /// (warning explícito, rango extendido, etc.) lo note.
    #[test]
    fn volume_mayor_a_1_27_se_clampea_a_1_27_en_smf() {
        let mut score = Score::new(120.0);
        let mut t = Track::new("loud");
        t.volume = 1.5; // pedido del caller
        let c4 = Pitch::from_class_octave(PitchClass::C, 4).unwrap();
        t.add(ScoreNote::new(c4, 0.0, 1.0, 100));
        score.add_track(t);

        let bytes = to_smf(&score);
        let back = from_smf(&bytes).unwrap();
        let vol_back = back.tracks()[0].volume;
        assert!(
            (vol_back - 1.27).abs() < 0.01,
            "esperaba 1.27 (clamp del CC#7), fue {vol_back}",
        );
    }
}
