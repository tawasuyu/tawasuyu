//! «rusty rain» — el fondo de lluvia de glifos estilo *Matrix* del greeter.
//!
//! Inspirado en la animación del DM `ly` y en la rutina `rusty-rain` de Rust,
//! pero reescrito como **render puro y determinista**: dado el tiempo `t` (en
//! segundos) y el rect del lienzo, cada columna deriva su estado por hashing de
//! su índice. No hay `Vec<Columna>` mutable que persistir entre frames, así que
//! el efecto sobrevive a resizes y al modelo Elm sin estado extra — sólo se
//! avanza un `f32` de reloj.
//!
//! Se dibuja dentro de `paint_with` del cuerpo del greeter, por debajo de la
//! tarjeta de login (el painter de un nodo pinta antes que sus hijos).

use std::sync::OnceLock;

use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::{self, Alignment, Typesetter};
use llimphi_ui::PaintRect;

/// Semilla global del campo. Fija ⇒ el patrón es estable entre arranques; toda
/// la variedad sale del hashing por columna/celda.
const SEED: u64 = 0x6361726d_656e0001; // "carmen" + 1

/// Geometría de la grilla, en px.
const FONT_PX: f32 = 16.0;
const CELL_W: f32 = 14.0;
const CELL_H: f32 = 18.0;

/// splitmix64 — hash entero rápido y de buena dispersión.
fn hash(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hash → `f32` en `[0, 1)`.
fn hf(x: u64) -> f32 {
    (hash(x) >> 40) as f32 / (1u64 << 24) as f32
}

/// El repertorio de glifos: katakana de ancho medio (el sello *Matrix*) más
/// dígitos, latinas y símbolos. Si la fuente del sistema no trae katakana,
/// fontique cae a un fallback CJK; lo peor es algún `.notdef`, que en este
/// contexto pasa por «glifo cifrado» igual.
fn glyphs() -> &'static [char] {
    static G: OnceLock<Vec<char>> = OnceLock::new();
    G.get_or_init(|| {
        let mut v: Vec<char> = Vec::new();
        for c in 0xFF66u32..=0xFF9D {
            if let Some(ch) = char::from_u32(c) {
                v.push(ch);
            }
        }
        v.extend('0'..='9');
        v.extend('A'..='Z');
        v.extend([
            '+', '-', '*', '/', '=', '<', '>', ':', ';', '#', '@', '%', '&', '$', '?',
        ]);
        v
    })
}

/// El glifo de la celda `(col, row)` en el instante `t`. Cada celda parpadea a
/// su propio ritmo (1–4 Hz) con fase propia, así la lluvia «muta» de forma
/// orgánica en vez de cambiar toda a la vez.
fn glyph_at(col: usize, row: i32, t: f32) -> char {
    let g = glyphs();
    let cell = hash(SEED ^ ((col as u64) << 20) ^ (row as u64 & 0xFFFFF));
    let rate = 1.0 + hf(cell ^ 0xAB) * 3.0;
    let flip = (t * rate) as u64;
    g[(hash(cell ^ flip.wrapping_mul(0x9E37)) as usize) % g.len()]
}

/// Mezcla lineal de dos canales de 8 bits.
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8
}

/// Color de un glifo según su distancia a la cabeza del chorro: la cabeza
/// (`dist == 0`) casi blanca, el resto el color base atenuándose y volviéndose
/// translúcido hacia la cola.
fn glyph_color(dist: i32, tail: i32, bright: (u8, u8, u8)) -> Color {
    let (br, bg, bb) = bright;
    if dist <= 0 {
        // Cabeza: tinte casi blanco para que «encienda» la columna.
        return Color::from_rgba8(
            lerp_u8(br, 255, 0.75),
            lerp_u8(bg, 255, 0.85),
            lerp_u8(bb, 255, 0.75),
            255,
        );
    }
    let f = (1.0 - dist as f32 / tail.max(1) as f32).clamp(0.0, 1.0);
    let scale = 0.22 + 0.78 * f;
    let a = (30.0 + 220.0 * f.powf(1.15)).clamp(0.0, 255.0) as u8;
    Color::from_rgba8(
        (br as f32 * scale) as u8,
        (bg as f32 * scale) as u8,
        (bb as f32 * scale) as u8,
        a,
    )
}

/// Pinta un frame de la lluvia sobre `rect`. `t` es el reloj en segundos;
/// `bright` el color base ya resuelto (RGB del tema o de la paleta elegida).
pub fn paint(scene: &mut vello::Scene, ts: &mut Typesetter, rect: PaintRect, t: f32, bright: (u8, u8, u8)) {
    if rect.w < CELL_W || rect.h < CELL_H {
        return;
    }
    let cols = (rect.w / CELL_W).ceil() as usize;
    let rows = (rect.h / CELL_H).ceil() as i32 + 1;
    let line_height = CELL_H / FONT_PX;
    let dark = Color::from_rgba8(
        (bright.0 as f32 * 0.2) as u8,
        (bright.1 as f32 * 0.2) as u8,
        (bright.2 as f32 * 0.2) as u8,
        255,
    );

    for col in 0..cols {
        // Parámetros estables de la columna, por hashing de su índice.
        let base = hash(SEED ^ (col as u64).wrapping_mul(0x1_0000_01B3));
        let speed = 5.0 + hf(base ^ 1) * 17.0; // filas por segundo
        let tail = 6 + (hf(base ^ 2) * 22.0) as i32; // largo del chorro
        let gap = 4.0 + hf(base ^ 3) * 28.0; // filas vacías entre pasadas
        let phase = hf(base ^ 4);
        let period = rows as f32 + tail as f32 + gap;
        let head = (phase * period + t * speed).rem_euclid(period);
        let head_i = head.floor() as i32;

        let first_r = (head_i - tail + 1).max(0);
        let last_r = head_i.min(rows - 1);
        if last_r < first_r {
            continue; // chorro completamente fuera de pantalla
        }

        // Una sola pasada de shaping por columna: el chorro es un string
        // con los glifos separados por '\n' y un color por glifo.
        let mut s = String::new();
        let mut runs: Vec<(usize, usize, Color)> = Vec::new();
        let mut byte = 0usize;
        for r in first_r..=last_r {
            let ch = glyph_at(col, r, t);
            let color = glyph_color(head_i - r, tail, bright);
            let mut buf = [0u8; 4];
            let enc = ch.encode_utf8(&mut buf);
            let start = byte;
            s.push_str(enc);
            byte += enc.len();
            runs.push((start, byte, color));
            if r != last_r {
                s.push('\n');
                byte += 1;
            }
        }

        let x = rect.x + col as f32 * CELL_W;
        let y = rect.y + first_r as f32 * CELL_H;
        let layout = ts.layout_runs(&s, FONT_PX, dark, &runs, Alignment::Start, line_height, 400.0);
        llimphi_text::draw_layout_runs(scene, &layout, (x as f64, y as f64));
    }
}
