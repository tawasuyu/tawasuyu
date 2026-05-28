//! Geometría del piano roll — compartida entre painter y hit-testing.

use takiy_core::Score;

use crate::{HEADER_H, KEYBOARD_W, MAX_KEY_H, MIN_BEAT_W, MIN_KEY_H};

/// Geometría completa del grid en coordenadas locales al `View` raíz.
///
/// Devuelve `(grid_x, grid_y, grid_w, grid_h, key_h, beat_w)` o `None` si
/// el rect es demasiado chico para mostrar grid. Usar el mismo cálculo
/// en painter y handlers garantiza que el hit-test nunca se desincronice.
pub fn grid_geometry(
    rect_w: f32,
    rect_h: f32,
    min_midi: u8,
    max_midi: u8,
    total_beats: f32,
) -> Option<(f32, f32, f32, f32, f32, f32)> {
    let grid_w = (rect_w - KEYBOARD_W).max(0.0);
    let grid_h = (rect_h - HEADER_H).max(0.0);
    if grid_w <= 0.0 || grid_h <= 0.0 {
        return None;
    }
    let n_keys = (max_midi - min_midi + 1) as f32;
    let key_h = (grid_h / n_keys).clamp(MIN_KEY_H, MAX_KEY_H);
    let beat_w = (grid_w / total_beats).max(MIN_BEAT_W);
    Some((KEYBOARD_W, HEADER_H, grid_w, grid_h, key_h, beat_w))
}

/// Mapea un click sobre la **banda del header** (0..HEADER_H) a la
/// posición en beats que corresponde por X. Devuelve `None` si el click
/// no está sobre el header (ej. cayó sobre el teclado, o por debajo de
/// HEADER_H). El valor devuelto puede ser fraccional para que el seek
/// caiga exacto en la posición clickeada, no snappeada a beat entero.
pub fn header_beat_at(
    lx: f32,
    ly: f32,
    rect_w: f32,
    rect_h: f32,
    min_midi: u8,
    max_midi: u8,
    total_beats: f32,
) -> Option<f32> {
    let (grid_x, grid_y, grid_w, _grid_h, _key_h, beat_w) =
        grid_geometry(rect_w, rect_h, min_midi, max_midi, total_beats)?;
    if ly < 0.0 || ly >= grid_y {
        return None;
    }
    if lx < grid_x || lx > grid_x + grid_w {
        return None;
    }
    Some(((lx - grid_x) / beat_w).max(0.0))
}

/// Mapea `(lx, ly)` — coordenadas locales — a `(beat, midi)`. Devuelve
/// `None` si el punto cae fuera del grid (teclado, header, o fuera de
/// los límites verticales/horizontales).
pub fn cell_at(
    lx: f32,
    ly: f32,
    rect_w: f32,
    rect_h: f32,
    min_midi: u8,
    max_midi: u8,
    total_beats: f32,
) -> Option<(f32, u8)> {
    let (grid_x, grid_y, grid_w, grid_h, key_h, beat_w) =
        grid_geometry(rect_w, rect_h, min_midi, max_midi, total_beats)?;
    if lx < grid_x || ly < grid_y || lx > grid_x + grid_w || ly > grid_y + grid_h {
        return None;
    }
    let row = ((ly - grid_y) / key_h).floor() as i32;
    let midi_i = max_midi as i32 - row;
    if midi_i < min_midi as i32 || midi_i > max_midi as i32 {
        return None;
    }
    // Beat fraccional — el snap del editor decide si redondea. Esto
    // permite snap libre (`Snap::Free`) sin perder precisión, y snaps
    // intermedios (`Half`, `Quarter`, etc.) operando contra el valor real.
    let beat = ((lx - grid_x) / beat_w).max(0.0);
    Some((beat, midi_i as u8))
}

/// Devuelve `(track_idx, note_idx)` de la nota bajo `(lx, ly)`, o `None`
/// si el punto no está sobre ninguna. Itera en orden estable; si dos
/// notas se solapan, gana la primera encontrada.
pub fn hit_test_note(
    score: &Score,
    lx: f32,
    ly: f32,
    rect_w: f32,
    rect_h: f32,
    min_midi: u8,
    max_midi: u8,
    total_beats: f32,
) -> Option<(usize, usize)> {
    let (grid_x, grid_y, _gw, _gh, key_h, beat_w) =
        grid_geometry(rect_w, rect_h, min_midi, max_midi, total_beats)?;
    for (ti, track) in score.tracks().iter().enumerate() {
        for (ni, note) in track.notes().iter().enumerate() {
            let midi = note.pitch.midi();
            if midi < min_midi || midi > max_midi {
                continue;
            }
            let row = (max_midi - midi) as f32;
            let nx = grid_x + note.start * beat_w;
            let ny = grid_y + row * key_h;
            let nw = (note.duration * beat_w).max(1.5);
            let nh = (key_h - 1.5).max(2.0);
            if lx >= nx && lx < nx + nw && ly >= ny && ly < ny + nh {
                return Some((ti, ni));
            }
        }
    }
    None
}

/// Rango MIDI con padding de 2 semitonos arriba y abajo. Si el score
/// está vacío devuelve C4..C5 (un rango cómodo para empezar a editar).
pub fn pitch_range(score: &Score) -> (u8, u8) {
    let mut min = u8::MAX;
    let mut max = 0u8;
    let mut found = false;
    for track in score.tracks() {
        for note in track.notes() {
            found = true;
            let m = note.pitch.midi();
            if m < min { min = m; }
            if m > max { max = m; }
        }
    }
    if !found {
        return (60, 72);
    }
    (min.saturating_sub(2), max.saturating_add(2).min(127))
}

#[cfg(test)]
mod tests {
    use super::*;
    use takiy_core::{Pitch, ScoreNote, Track};

    fn rect() -> (f32, f32) {
        (1200.0, 640.0)
    }

    #[test]
    fn pitch_range_for_empty_score_is_c4_c5() {
        let s = Score::new(120.0);
        assert_eq!(pitch_range(&s), (60, 72));
    }

    #[test]
    fn pitch_range_pads_with_two_semitones() {
        let mut s = Score::new(120.0);
        let mut t = Track::new("a");
        t.add(ScoreNote::new(Pitch::MIDDLE_C, 0.0, 1.0, 80));
        t.add(ScoreNote::new(Pitch::A4, 0.0, 1.0, 80));
        s.add_track(t);
        let (lo, hi) = pitch_range(&s);
        assert_eq!(lo, 60 - 2);
        assert_eq!(hi, 69 + 2);
    }

    #[test]
    fn pitch_range_clamps_at_midi_top() {
        let mut s = Score::new(120.0);
        let mut t = Track::new("a");
        t.add(ScoreNote::new(Pitch::from_midi(126).unwrap(), 0.0, 1.0, 80));
        s.add_track(t);
        let (_lo, hi) = pitch_range(&s);
        assert_eq!(hi, 127);
    }

    #[test]
    fn cell_at_returns_none_inside_keyboard_column() {
        let (w, h) = rect();
        // x < KEYBOARD_W cae en el teclado pintado, no en el grid.
        assert!(cell_at(10.0, 100.0, w, h, 60, 72, 16.0).is_none());
    }

    #[test]
    fn cell_at_returns_none_above_header() {
        let (w, h) = rect();
        assert!(cell_at(200.0, 5.0, w, h, 60, 72, 16.0).is_none());
    }

    #[test]
    fn cell_at_returns_fractional_beat() {
        let (w, h) = rect();
        let (min_midi, max_midi, total_beats) = (60, 72, 16.0);
        let (gx, gy, _gw, _gh, key_h, beat_w) =
            grid_geometry(w, h, min_midi, max_midi, total_beats).unwrap();
        let target_midi: u8 = 65;
        // Apuntamos exactamente al medio del beat 3 → 3.5 fraccional.
        let lx = gx + 3.0 * beat_w + beat_w * 0.5;
        let ly = gy + (max_midi - target_midi) as f32 * key_h + key_h * 0.5;
        let (beat, midi) = cell_at(lx, ly, w, h, min_midi, max_midi, total_beats).unwrap();
        assert_eq!(midi, target_midi);
        assert!((beat - 3.5).abs() < 1e-3);
    }

    #[test]
    fn hit_test_note_finds_existing_note() {
        let (w, h) = rect();
        let mut s = Score::new(120.0);
        let mut t = Track::new("a");
        t.add(ScoreNote::new(Pitch::MIDDLE_C, 4.0, 1.0, 100));
        s.add_track(t);

        let (min_midi, max_midi) = pitch_range(&s);
        let total_beats = s.duration_beats().max(8.0);
        let (gx, gy, _gw, _gh, key_h, beat_w) =
            grid_geometry(w, h, min_midi, max_midi, total_beats).unwrap();
        let lx = gx + 4.0 * beat_w + 1.0;
        let ly = gy + (max_midi - 60) as f32 * key_h + 1.0;
        assert_eq!(hit_test_note(&s, lx, ly, w, h, min_midi, max_midi, total_beats),
                   Some((0, 0)));
    }

    #[test]
    fn hit_test_note_misses_empty_cell() {
        let (w, h) = rect();
        let s = Score::new(120.0);
        assert_eq!(hit_test_note(&s, 200.0, 100.0, w, h, 60, 72, 8.0), None);
    }

    #[test]
    fn header_beat_at_returns_fractional_beat() {
        let (w, h) = rect();
        let (min_midi, max_midi, total_beats) = (60, 72, 16.0);
        let (gx, _gy, _gw, _gh, _key_h, beat_w) =
            grid_geometry(w, h, min_midi, max_midi, total_beats).unwrap();
        // Click sobre el header en el medio del beat 3.
        let lx = gx + 3.0 * beat_w + beat_w * 0.5;
        let ly = HEADER_H * 0.5;
        let beat = header_beat_at(lx, ly, w, h, min_midi, max_midi, total_beats).unwrap();
        assert!((beat - 3.5).abs() < 1e-3);
    }

    #[test]
    fn header_beat_at_rejects_clicks_below_header() {
        let (w, h) = rect();
        // y > HEADER_H cae en el grid, no en el header.
        assert!(header_beat_at(400.0, HEADER_H + 5.0, w, h, 60, 72, 16.0).is_none());
    }

    #[test]
    fn header_beat_at_rejects_clicks_over_keyboard() {
        let (w, h) = rect();
        // x < KEYBOARD_W cae sobre el teclado pintado, no sobre el header.
        assert!(header_beat_at(KEYBOARD_W * 0.5, 5.0, w, h, 60, 72, 16.0).is_none());
    }

    #[test]
    fn grid_geometry_rejects_tiny_rects() {
        // rect_w = KEYBOARD_W → grid_w = 0 → None.
        assert!(grid_geometry(KEYBOARD_W, 200.0, 60, 72, 8.0).is_none());
        assert!(grid_geometry(800.0, HEADER_H, 60, 72, 8.0).is_none());
    }
}
