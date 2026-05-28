//! Mapeos GM (General MIDI) — nombres de grupo y heurística por pista.

/// Nombre del grupo GM al que pertenece un programa `0..=127`. No
/// devuelve el nombre exacto del instrumento (eso requeriría una tabla
/// de 128) sino el grupo (Pianos, Bass, Brass, etc.), que es lo que
/// necesita el header para feedback al cambiar con `p`/`P`.
pub fn gm_program_name(program: u8) -> &'static str {
    match program / 8 {
        0 => "Piano",
        1 => "Chromatic Perc.",
        2 => "Organ",
        3 => "Guitar",
        4 => "Bass",
        5 => "Strings",
        6 => "Ensemble",
        7 => "Brass",
        8 => "Reed",
        9 => "Pipe",
        10 => "Synth Lead",
        11 => "Synth Pad",
        12 => "Synth Effects",
        13 => "Ethnic",
        14 => "Percussive",
        15 => "Sound Effects",
        _ => "?",
    }
}

/// Mapeo heurístico nombre de pista → programa GM `0..=127`. Pensado
/// para que el demo built-in suene razonable sin configuración: cae a
/// piano (0) si no reconoce el nombre.
pub fn gm_program_for_track_name(name: &str) -> u8 {
    let n = name.to_lowercase();
    if n.contains("bass") || n.contains("bajo") {
        32 // Acoustic Bass
    } else if n.contains("guitar") || n.contains("guitarra") {
        24 // Acoustic Guitar (nylon)
    } else if n.contains("string") || n.contains("cuerda") {
        48 // String Ensemble 1
    } else if n.contains("organ") || n.contains("órgano") || n.contains("organo") {
        19 // Church Organ
    } else if n.contains("flute") || n.contains("flauta") {
        73 // Flute
    } else if n.contains("trumpet") || n.contains("trompeta") {
        56 // Trumpet
    } else if n.contains("pad") {
        88 // Pad 1 (new age)
    } else {
        0 // Acoustic Grand Piano
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gm_program_name_covers_all_groups() {
        // Cada grupo de 8 programas debe tener nombre — 0..=127 nunca
        // cae al fallback "?".
        for p in 0..=127u8 {
            let name = gm_program_name(p);
            assert_ne!(name, "?", "program {p} sin nombre");
        }
    }

    #[test]
    fn gm_program_name_groups_correctly() {
        assert_eq!(gm_program_name(0), "Piano");           // Acoustic Grand Piano
        assert_eq!(gm_program_name(32), "Bass");           // Acoustic Bass
        assert_eq!(gm_program_name(40), "Strings");        // Violin (Strings = 40-47)
        assert_eq!(gm_program_name(48), "Ensemble");       // String Ensemble 1 (Ensemble = 48-55)
        assert_eq!(gm_program_name(56), "Brass");          // Trumpet
        assert_eq!(gm_program_name(80), "Synth Lead");
        assert_eq!(gm_program_name(120), "Sound Effects");
    }

    #[test]
    fn track_name_mapping_es_en() {
        assert_eq!(gm_program_for_track_name("bajo"), 32);
        assert_eq!(gm_program_for_track_name("Bass guitar"), 32);
        assert_eq!(gm_program_for_track_name("Strings 1"), 48);
        assert_eq!(gm_program_for_track_name("cuerdas"), 48);
        assert_eq!(gm_program_for_track_name("Órgano"), 19);
        assert_eq!(gm_program_for_track_name("Trompeta"), 56);
        assert_eq!(gm_program_for_track_name("flauta dulce"), 73);
        assert_eq!(gm_program_for_track_name("guitarra criolla"), 24);
        assert_eq!(gm_program_for_track_name("synth pad"), 88);
    }

    #[test]
    fn unknown_track_name_falls_back_to_piano() {
        assert_eq!(gm_program_for_track_name(""), 0);
        assert_eq!(gm_program_for_track_name("melodía"), 0);
        assert_eq!(gm_program_for_track_name("track 1"), 0);
    }
}
