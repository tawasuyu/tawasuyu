//! Decodificador sixel (a mano — no hay crate en el workspace).
//!
//! Entrada: el cuerpo DCS (sin el `\eP` inicial ni el `\e\\` final). Estructura:
//! `<params>q` y luego los datos sixel:
//! - `"Pan;Pad;Ph;Pv` — atributos raster (aspecto + ancho `Ph` × alto `Pv`).
//! - `#Pc;Pu;Px;Py;Pz` — define el color del registro `Pc` (`Pu=2` ⇒ RGB en
//!   porcentaje 0..100). `#Pc` solo ⇒ selecciona el registro `Pc`.
//! - `!Pn <char>` — repite el sixel `<char>` `Pn` veces (RLE).
//! - `$` — carriage return (vuelve al inicio de la banda, misma `y`).
//! - `-` — line feed (avanza 6 filas).
//! - chars `0x3F..=0x7E` — 6 píxeles verticales; bit `i` (LSB = arriba) prende
//!   la fila `y+i` con el color actual.
//!
//! Estrategia de tamaño: si hay atributos raster los usamos; si no, dos
//! pasadas — la primera mide (`max_x`/`max_y`), la segunda pinta.

use crate::DecodedImage;

/// Tope defensivo de píxeles (16 Mpx ≈ 64 MiB RGBA) para no estallar con un
/// stream sixel malicioso/roto.
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

pub fn decode(seq: &[u8]) -> Option<DecodedImage> {
    let qpos = seq.iter().position(|&b| b == b'q')?;
    let data = &seq[qpos + 1..];

    // Tamaño: de atributos raster si están; si no, midiendo.
    let (mut w, mut h) = raster_size(data);
    if w == 0 || h == 0 {
        let (mx, my) = measure(data);
        if w == 0 {
            w = mx;
        }
        if h == 0 {
            h = my;
        }
    }
    if w == 0 || h == 0 {
        return None;
    }
    if (w as u64) * (h as u64) > MAX_PIXELS {
        return None;
    }

    let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
    paint(data, w, h, &mut rgba);
    Some(DecodedImage {
        width: w,
        height: h,
        rgba,
    })
}

/// Lee los atributos raster `"Pan;Pad;Ph;Pv` si aparecen al inicio. Devuelve
/// `(Ph, Pv)` o `(0,0)` si no hay.
fn raster_size(data: &[u8]) -> (u32, u32) {
    let mut i = 0;
    // saltar whitespace inicial
    while i < data.len() && (data[i] == b'\n' || data[i] == b'\r' || data[i] == b' ') {
        i += 1;
    }
    if i >= data.len() || data[i] != b'"' {
        return (0, 0);
    }
    let (nums, _) = parse_nums(data, i + 1);
    if nums.len() >= 4 {
        (nums[2], nums[3])
    } else {
        (0, 0)
    }
}

/// Pasada de medición: corre la máquina sin pintar, devolviendo
/// `(max_x+1, max_y+1)`.
fn measure(data: &[u8]) -> (u32, u32) {
    let mut x: u32 = 0;
    let mut y0: u32 = 0;
    let mut max_x: u32 = 0;
    let mut max_y: u32 = 0;
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        match b {
            b'"' | b'#' => {
                let (_, ni) = parse_nums(data, i + 1);
                i = ni;
            }
            b'!' => {
                let (nums, ni) = parse_nums(data, i + 1);
                i = ni;
                let count = nums.first().copied().unwrap_or(0).max(1);
                if i < data.len() {
                    let c = data[i];
                    i += 1;
                    if (0x3f..=0x7e).contains(&c) {
                        let bits = c - 0x3f;
                        if bits != 0 {
                            max_x = max_x.max(x + count - 1);
                            max_y = max_y.max(y0 + top_bit(bits));
                        }
                        x += count;
                    }
                }
            }
            b'$' => {
                x = 0;
                i += 1;
            }
            b'-' => {
                x = 0;
                y0 += 6;
                i += 1;
            }
            0x3f..=0x7e => {
                let bits = b - 0x3f;
                if bits != 0 {
                    max_x = max_x.max(x);
                    max_y = max_y.max(y0 + top_bit(bits));
                }
                x += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    (max_x + 1, max_y + 1)
}

/// Pasada de pintura sobre `rgba` (ya dimensionado a `w`×`h`).
fn paint(data: &[u8], w: u32, h: u32, rgba: &mut [u8]) {
    let mut palette = default_palette();
    let mut cur: u16 = 0;
    let mut x: u32 = 0;
    let mut y0: u32 = 0;
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        match b {
            b'"' => {
                let (_, ni) = parse_nums(data, i + 1);
                i = ni;
            }
            b'#' => {
                let (nums, ni) = parse_nums(data, i + 1);
                i = ni;
                if let Some(&pc) = nums.first() {
                    cur = pc as u16;
                    if nums.len() >= 5 {
                        let pu = nums[1];
                        let (px, py, pz) = (nums[2], nums[3], nums[4]);
                        let color = if pu == 2 {
                            [pct(px), pct(py), pct(pz), 255]
                        } else {
                            // Pu=1 (HLS) es raro; aproximamos como RGB para no
                            // bajar a negro. Suficiente para chafa/img2sixel
                            // que emiten siempre Pu=2.
                            [pct(px), pct(py), pct(pz), 255]
                        };
                        palette.insert(cur, color);
                    }
                }
            }
            b'!' => {
                let (nums, ni) = parse_nums(data, i + 1);
                i = ni;
                let count = nums.first().copied().unwrap_or(0).max(1);
                if i < data.len() {
                    let c = data[i];
                    i += 1;
                    if (0x3f..=0x7e).contains(&c) {
                        let bits = c - 0x3f;
                        let color = *palette.get(&cur).unwrap_or(&[0, 0, 0, 255]);
                        for _ in 0..count {
                            put(rgba, w, h, x, y0, bits, color);
                            x += 1;
                        }
                    }
                }
            }
            b'$' => {
                x = 0;
                i += 1;
            }
            b'-' => {
                x = 0;
                y0 += 6;
                i += 1;
            }
            0x3f..=0x7e => {
                let bits = b - 0x3f;
                let color = *palette.get(&cur).unwrap_or(&[0, 0, 0, 255]);
                put(rgba, w, h, x, y0, bits, color);
                x += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
}

/// Pinta los 6 píxeles verticales de un sixel en `(x, y0..y0+6)`.
fn put(rgba: &mut [u8], w: u32, h: u32, x: u32, y0: u32, bits: u8, color: [u8; 4]) {
    if x >= w {
        return;
    }
    for r in 0..6u32 {
        if bits & (1 << r) != 0 {
            let y = y0 + r;
            if y >= h {
                continue;
            }
            let idx = ((y as usize) * (w as usize) + x as usize) * 4;
            rgba[idx..idx + 4].copy_from_slice(&color);
        }
    }
}

/// Fila más alta (0-based) prendida por `bits` (LSB = arriba).
fn top_bit(bits: u8) -> u32 {
    let mut top = 0;
    for r in 0..6u32 {
        if bits & (1 << r) != 0 {
            top = r;
        }
    }
    top
}

/// Porcentaje sixel (0..100) → byte (0..255).
fn pct(v: u32) -> u8 {
    ((v.min(100) * 255 + 50) / 100) as u8
}

/// Lee una secuencia de enteros separados por `;` a partir de `start`.
/// Devuelve los números y el índice del primer byte no consumido.
fn parse_nums(data: &[u8], start: usize) -> (Vec<u32>, usize) {
    let mut nums = Vec::new();
    let mut i = start;
    let mut cur: u32 = 0;
    let mut any = false;
    loop {
        if i >= data.len() {
            break;
        }
        let b = data[i];
        if b.is_ascii_digit() {
            cur = cur.saturating_mul(10).saturating_add((b - b'0') as u32);
            any = true;
            i += 1;
        } else if b == b';' {
            nums.push(cur);
            cur = 0;
            any = true;
            i += 1;
        } else {
            break;
        }
    }
    if any {
        nums.push(cur);
    }
    (nums, i)
}

/// Paleta por defecto (negro en el registro 0; el resto se llena con las
/// definiciones `#Pc;2;…` del stream). chafa/img2sixel siempre definen sus
/// colores, así que no hace falta la paleta VT340 completa.
fn default_palette() -> std::collections::HashMap<u16, [u8; 4]> {
    let mut m = std::collections::HashMap::new();
    m.insert(0, [0, 0, 0, 255]);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sixel mínimo 1×6: un color RGB puro y un carácter con todos los bits.
    #[test]
    fn sixel_columna_1x6() {
        // ESC P ya removido. Params vacíos + 'q'. Color 0 = rojo (100,0,0).
        // '~' = 0x7e → bits = 0x3f = 0b111111 → 6 px prendidos.
        let body = b"q#0;2;100;0;0~";
        let img = decode(body).expect("decodifica");
        assert_eq!((img.width, img.height), (1, 6));
        // Todos los 6 px rojos opacos.
        for px in img.rgba.chunks_exact(4) {
            assert_eq!(px, &[255, 0, 0, 255]);
        }
    }

    /// RLE: `!5~` = 5 columnas de 6 px.
    #[test]
    fn sixel_rle_ancho() {
        let body = b"q#0;2;0;100;0!5~";
        let img = decode(body).expect("decodifica");
        assert_eq!((img.width, img.height), (5, 6));
        // verde
        assert_eq!(&img.rgba[0..4], &[0, 255, 0, 255]);
    }

    /// Atributos raster fijan el tamaño aunque los datos sean más chicos.
    #[test]
    fn sixel_raster_attrs() {
        let body = b"q\"1;1;10;12#0;2;0;0;100~";
        let img = decode(body).expect("decodifica");
        assert_eq!((img.width, img.height), (10, 12));
    }

    /// LF `-` baja una banda (6 px) → alto 12.
    #[test]
    fn sixel_dos_bandas() {
        let body = b"q#0;2;100;100;100~-~";
        let img = decode(body).expect("decodifica");
        assert_eq!(img.height, 12);
    }
}
