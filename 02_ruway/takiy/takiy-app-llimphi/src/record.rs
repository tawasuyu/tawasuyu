//! Modo grabación — la "vista alterna" de una pista: el teclado
//! alfabético se mapea a un teclado de piano, lo que tocás suena (y
//! opcionalmente suenan las demás pistas de fondo) y queda **grabado**
//! como MIDI en la pista activa.
//!
//! Mapeo (estilo trackers/LMMS), dos filas = dos octavas desde la octava
//! base:
//! - fila inferior `z s x d c v g b h n j m` → C C# D D# E F F# G G# A A# B
//! - fila superior `q 2 w 3 e r 5 t 6 y 7 u` → la octava de arriba
//!
//! `←/→` (o los botones) corren la octava base; `Esc` o el botón detienen.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::{draw_block, Alignment as TextAlignment, TextBlock};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::{PaintRect, View};
use llimphi_widget_button::{button_view, ButtonPalette};

use crate::appmodel::Model;
use crate::msg::Msg;

/// Tabla del mapeo tecla→nota: `(carácter, Δoctava, semitono)`. El midi
/// final = `(base_octave + Δoctava + 1) * 12 + semitono`.
const KEYMAP: &[(&str, i32, i32)] = &[
    // Fila inferior — octava base.
    ("z", 0, 0), ("s", 0, 1), ("x", 0, 2), ("d", 0, 3), ("c", 0, 4),
    ("v", 0, 5), ("g", 0, 6), ("b", 0, 7), ("h", 0, 8), ("n", 0, 9),
    ("j", 0, 10), ("m", 0, 11),
    // Fila superior — octava base + 1.
    ("q", 1, 0), ("2", 1, 1), ("w", 1, 2), ("3", 1, 3), ("e", 1, 4),
    ("r", 1, 5), ("5", 1, 6), ("t", 1, 7), ("6", 1, 8), ("y", 1, 9),
    ("7", 1, 10), ("u", 1, 11),
    // Cola superior — octava base + 2 (sólo las primeras teclas).
    ("i", 2, 0), ("9", 2, 1), ("o", 2, 2), ("0", 2, 3), ("p", 2, 4),
];

/// Midi de una tecla tocada, o `None` si la tecla no está mapeada o el
/// resultado se sale del rango MIDI.
pub(crate) fn key_to_midi(ch: &str, base_octave: i32) -> Option<u8> {
    let ch = ch.to_ascii_lowercase();
    let (_, doct, semi) = KEYMAP.iter().find(|(k, _, _)| *k == ch)?;
    let midi = (base_octave + doct + 1) * 12 + semi;
    u8::try_from(midi).ok().filter(|m| *m <= 127)
}

/// Etiqueta de tecla para un midi dado (reverso del mapeo), o `None`.
fn label_for_midi(midi: u8, base_octave: i32) -> Option<&'static str> {
    KEYMAP
        .iter()
        .find(|(_, doct, semi)| (base_octave + doct + 1) * 12 + semi == midi as i32)
        .map(|(k, _, _)| *k)
}

const BLACK_CLASSES: [i32; 5] = [1, 3, 6, 8, 10];
fn is_black(midi: u8) -> bool {
    BLACK_CLASSES.contains(&(midi as i32 % 12))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_lower_row_is_octave_base() {
        // z = C de la octava base (4 → midi 60), m = B (71).
        assert_eq!(key_to_midi("z", 4), Some(60));
        assert_eq!(key_to_midi("s", 4), Some(61)); // C#
        assert_eq!(key_to_midi("c", 4), Some(64)); // E
        assert_eq!(key_to_midi("m", 4), Some(71)); // B
        // Mayúsculas (con Shift) caen igual.
        assert_eq!(key_to_midi("Z", 4), Some(60));
    }

    #[test]
    fn keymap_upper_row_is_octave_above() {
        // q = C de la octava base+1 (72 con base 4).
        assert_eq!(key_to_midi("q", 4), Some(72));
        assert_eq!(key_to_midi("u", 4), Some(83)); // B de base+1
        assert_eq!(key_to_midi("i", 4), Some(84)); // C de base+2
    }

    #[test]
    fn keymap_octave_shift_and_unmapped() {
        assert_eq!(key_to_midi("z", 5), Some(72)); // sube una octava
        assert_eq!(key_to_midi("ñ", 4), None); // tecla no mapeada
        assert_eq!(key_to_midi("1", 4), None);
    }

    #[test]
    fn label_round_trips_with_keymap() {
        // Cada midi mapeado tiene su etiqueta de vuelta.
        for &(k, _, _) in KEYMAP {
            let midi = key_to_midi(k, 4).unwrap();
            assert_eq!(label_for_midi(midi, 4), Some(k));
        }
    }
}

/// Cuerpo del modo grabación: HUD + teclado de piano en pantalla.
pub(crate) fn body(model: &Model, theme: &Theme) -> View<Msg> {
    let Some(rec) = model.recording.as_ref() else {
        return View::new(Style::default());
    };
    let name = model
        .editor
        .score
        .track(rec.track)
        .map(|t| t.name.clone())
        .unwrap_or_default();

    let hud = hud_bar(rec, &name, theme);
    let keyboard = keyboard_canvas(rec, theme);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![hud, keyboard])
}

/// Barra superior: indicador de grabación + controles (fondo, octava, stop).
fn hud_bar(rec: &crate::appmodel::RecState, name: &str, theme: &Theme) -> View<Msg> {
    let info = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        format!(
            "● GRABANDO · pista «{name}» · beat {:.1} · {} notas · octava {}",
            rec.last_beat, rec.count, rec.base_octave
        ),
        14.0,
        Color::from_rgba8(240, 110, 110, 255),
        TextAlignment::Start,
    );

    let backing = btn(
        if rec.backing { "fondo: sí" } else { "fondo: no" },
        96.0,
        rec.backing,
        theme,
        Msg::RecordToggleBacking,
    );
    let oct_dn = btn("oct −", 56.0, false, theme, Msg::RecordOctave(-1));
    let oct_up = btn("oct +", 56.0, false, theme, Msg::RecordOctave(1));
    let stop = btn("■ Detener", 96.0, true, theme, Msg::ToggleRecord);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(44.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![info, backing, oct_dn, oct_up, stop])
}

fn btn(label: &str, w: f32, active: bool, theme: &Theme, msg: Msg) -> View<Msg> {
    let mut pal = ButtonPalette::from_theme(theme);
    if active {
        pal.bg = theme.accent;
        pal.bg_hover = theme.accent;
        pal.fg = theme.bg_app;
    }
    View::new(Style {
        size: Size { width: length(w), height: length(28.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![button_view(label, &pal, msg)])
}

/// Lienzo del teclado de piano, con etiquetas de tecla y realce de las
/// notas apretadas.
fn keyboard_canvas(rec: &crate::appmodel::RecState, theme: &Theme) -> View<Msg> {
    let base_octave = rec.base_octave;
    let held: Vec<u8> = rec.held.keys().copied().collect();
    let bg = theme.bg_panel;
    let accent = theme.accent;
    let [ar, ag, ab, _] = accent.components;
    let accent_rgb = ((ar * 255.0) as u8, (ag * 255.0) as u8, (ab * 255.0) as u8);

    View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        min_size: Size { width: length(0.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(bg)
    .paint_with(move |scene, ts, rect: PaintRect| {
        paint_keyboard(scene, ts, rect, base_octave, &held, accent_rgb);
    })
}

/// Pinta ~2 octavas de piano desde la octava base. Teclas blancas a lo
/// ancho, negras encima; cada una con su etiqueta de tecla; las apretadas
/// se realzan con el acento.
fn paint_keyboard(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: PaintRect,
    base_octave: i32,
    held: &[u8],
    accent: (u8, u8, u8),
) {
    if rect.w <= 4.0 || rect.h <= 4.0 {
        return;
    }
    let pad = 12.0_f32;
    let kx = rect.x + pad;
    let ky = rect.y + pad;
    let kw = (rect.w - pad * 2.0).max(1.0);
    let kh = (rect.h - pad * 2.0).max(1.0);

    let lo = (base_octave + 1) * 12; // C de la octava base
    let semis = 26; // ~2 octavas + un par
    // Conteo de teclas blancas en el rango.
    let white_midis: Vec<u8> = (0..semis)
        .map(|s| (lo + s) as u8)
        .filter(|m| !is_black(*m))
        .collect();
    let n_white = white_midis.len().max(1);
    let white_w = kw / n_white as f32;
    let black_w = white_w * 0.62;

    let white_fill = Color::from_rgba8(228, 228, 234, 255);
    let black_fill = Color::from_rgba8(40, 42, 50, 255);
    let outline = Color::from_rgba8(90, 92, 104, 255);
    let held_white = Color::from_rgba8(accent.0, accent.1, accent.2, 235);
    let held_black = Color::from_rgba8(
        (accent.0 as f32 * 0.8) as u8,
        (accent.1 as f32 * 0.8) as u8,
        (accent.2 as f32 * 0.8) as u8,
        255,
    );

    // Blancas primero.
    for (i, &midi) in white_midis.iter().enumerate() {
        let x = kx + i as f32 * white_w;
        let r = KurboRect::new(x as f64, ky as f64, (x + white_w - 1.0) as f64, (ky + kh) as f64);
        let fill = if held.contains(&midi) { held_white } else { white_fill };
        scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &r);
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, outline, None, &r);
        if let Some(lbl) = label_for_midi(midi, base_octave) {
            let block = TextBlock {
                text: lbl,
                size_px: 13.0,
                color: Color::from_rgba8(60, 62, 72, 255),
                origin: ((x + white_w * 0.5 - 4.0) as f64, (ky + kh - 24.0) as f64),
                max_width: Some(white_w),
                alignment: TextAlignment::Start,
                line_height: 1.0,
                italic: false,
                font_family: None,
            };
            draw_block(scene, ts, &block);
        }
    }

    // Negras encima.
    let mut white_idx = 0usize;
    for s in 0..semis {
        let midi = (lo + s) as u8;
        if is_black(midi) {
            // La negra se ubica sobre la juntura con la blanca a su derecha.
            let x = kx + white_idx as f32 * white_w - black_w * 0.5;
            let bh = kh * 0.62;
            let r = KurboRect::new(x as f64, ky as f64, (x + black_w) as f64, (ky + bh) as f64);
            let fill = if held.contains(&midi) { held_black } else { black_fill };
            scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &r);
            scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, outline, None, &r);
            if let Some(lbl) = label_for_midi(midi, base_octave) {
                let block = TextBlock {
                    text: lbl,
                    size_px: 11.0,
                    color: Color::from_rgba8(220, 222, 230, 255),
                    origin: ((x + black_w * 0.5 - 3.0) as f64, (ky + bh - 18.0) as f64),
                    max_width: Some(black_w),
                    alignment: TextAlignment::Start,
                    line_height: 1.0,
                    italic: false,
                    font_family: None,
                };
                draw_block(scene, ts, &block);
            }
        } else {
            white_idx += 1;
        }
    }
}
