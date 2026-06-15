//! Helpers de presentación e identificación: búsqueda de notas, modos de
//! tonalidad, y los `describe_*` que arman strings cortos para el header.

use takiy_core::{DelayParams, PitchClass, ReverbParams, Scale, ScoreNote, Track};

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
pub(crate) enum KeyMode {
    Major,
    NaturalMinor,
    PentatonicMajor,
}

impl KeyMode {
    pub(crate) fn next(self) -> Self {
        match self {
            KeyMode::Major => KeyMode::NaturalMinor,
            KeyMode::NaturalMinor => KeyMode::PentatonicMajor,
            KeyMode::PentatonicMajor => KeyMode::Major,
        }
    }

    pub(crate) fn scale(self, root: PitchClass) -> Scale {
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
pub(crate) fn classify_mode(scale: &Scale) -> KeyMode {
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
pub(crate) fn next_pitch_class(pc: PitchClass) -> PitchClass {
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
