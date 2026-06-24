//! Panel de logs de arranque, estilo «details» de Plymouth pero **automático**:
//! aparece sólo si el arranque tarda de más o si el kernel reporta un error.
//! Sin GL: lee `/dev/kmsg`, y dibuja el texto con la fuente bitmap 8×8 de
//! dominio público (`font8x8`) sobre el mismo framebuffer del splash.
//!
//! Best-effort: si no se puede abrir `/dev/kmsg`, no hay panel (el splash sigue
//! limpio). Ver `SDD-ARRANQUE-SIN-PARPADEO.md`.

use std::collections::VecDeque;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;

use font8x8::legacy::BASIC_LEGACY;

/// Cuántos mensajes recientes conservamos / mostramos como máximo.
const RING: usize = 64;

/// Lector best-effort de `/dev/kmsg`. Acumula los últimos mensajes y marca si
/// vio un error (prioridad de syslog ≤ 3: err/crit/alert/emerg).
pub struct Kmsg {
    file: Option<std::fs::File>,
    lines: VecDeque<String>,
    error_seen: bool,
}

impl Kmsg {
    /// Abre `/dev/kmsg` no bloqueante. Best-effort.
    pub fn open() -> Self {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/kmsg")
            .ok();
        Kmsg { file, lines: VecDeque::with_capacity(RING), error_seen: false }
    }

    /// ¿Pudo abrir el log?
    pub fn active(&self) -> bool {
        self.file.is_some()
    }

    /// ¿Apareció algún error en el log?
    pub fn error_seen(&self) -> bool {
        self.error_seen
    }

    /// Lee todos los registros disponibles sin bloquear y los acumula. Cada
    /// `read()` de `/dev/kmsg` devuelve UN registro: `prio,seq,us,flags;texto`.
    pub fn poll(&mut self) {
        let Some(f) = self.file.as_mut() else { return };
        let mut buf = [0u8; 8192];
        loop {
            match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Some((level, msg)) = parse_record(&buf[..n]) {
                        if level <= 3 {
                            self.error_seen = true;
                        }
                        if self.lines.len() == RING {
                            self.lines.pop_front();
                        }
                        self.lines.push_back(msg);
                    }
                }
                // EAGAIN (no hay más por ahora) o cualquier error → cortamos.
                Err(_) => break,
            }
        }
    }

    /// Las últimas `n` líneas (las más nuevas al final).
    pub fn recent(&self, n: usize) -> Vec<String> {
        let len = self.lines.len();
        let start = len.saturating_sub(n);
        self.lines.iter().skip(start).cloned().collect()
    }
}

/// Parsea un registro de `/dev/kmsg` → `(level, mensaje)`. El prefijo antes de
/// `;` es metadata `prio,seq,timestamp,flags`; `level = prio % 8`.
fn parse_record(rec: &[u8]) -> Option<(u8, String)> {
    let s = std::str::from_utf8(rec).ok()?;
    let (meta, msg) = s.split_once(';')?;
    let prio: u32 = meta.split(',').next()?.trim().parse().ok()?;
    let level = (prio % 8) as u8;
    // El mensaje puede traer continuaciones con \n — nos quedamos con la 1ª.
    let msg = msg.lines().next().unwrap_or("").trim_end().to_string();
    if msg.is_empty() {
        return None;
    }
    Some((level, msg))
}

// ── Render ──────────────────────────────────────────────────────────────────

/// Escribe `s` con la fuente 8×8 escalada `scale`×, color `fg`, esquina
/// sup-izq en `(x0, y0)`. XRGB8888. Recorta a la pantalla.
pub fn draw_text(
    buf: &mut [u8],
    w: usize,
    h: usize,
    pitch: usize,
    x0: usize,
    y0: usize,
    s: &str,
    scale: usize,
    fg: (u8, u8, u8),
) {
    let mut cx = x0;
    for ch in s.chars() {
        let code = ch as usize;
        let glyph = if code < 128 { BASIC_LEGACY[code] } else { BASIC_LEGACY['?' as usize] };
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8 {
                // font8x8: bit 0 (LSB) = píxel más a la izquierda.
                if (bits >> col) & 1 == 0 {
                    continue;
                }
                fill_block(buf, w, h, pitch, cx + col * scale, y0 + row * scale, scale, fg);
            }
        }
        cx += 8 * scale; // monoespaciado
        if cx >= w {
            break;
        }
    }
}

/// Rellena un bloque `scale×scale` (un "píxel gordo" de la fuente).
fn fill_block(buf: &mut [u8], w: usize, h: usize, pitch: usize, x: usize, y: usize, scale: usize, c: (u8, u8, u8)) {
    for dy in 0..scale {
        let yy = y + dy;
        if yy >= h {
            break;
        }
        let row = yy * pitch;
        for dx in 0..scale {
            let xx = x + dx;
            if xx >= w {
                break;
            }
            let idx = row + xx * 4;
            if idx + 4 <= buf.len() {
                buf[idx] = c.2;
                buf[idx + 1] = c.1;
                buf[idx + 2] = c.0;
                buf[idx + 3] = 0;
            }
        }
    }
}

/// Oscurece (atenúa hacia negro) un rectángulo del buffer, para el fondo
/// translúcido del panel: el contenido de abajo se intuye sin estorbar.
fn darken_rect(buf: &mut [u8], w: usize, h: usize, pitch: usize, y0: usize, y1: usize, k: f32) {
    let k = k.clamp(0.0, 1.0);
    for y in y0..y1.min(h) {
        let row = y * pitch;
        for x in 0..w {
            let idx = row + x * 4;
            if idx + 4 > buf.len() {
                break;
            }
            // hacia un azul-negro (8,8,12), no negro puro.
            buf[idx] = lerp8(buf[idx], 12, k);
            buf[idx + 1] = lerp8(buf[idx + 1], 8, k);
            buf[idx + 2] = lerp8(buf[idx + 2], 8, k);
        }
    }
}

fn lerp8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8
}

/// Dibuja el panel desplegable de logs sobre el framebuffer ya pintado.
/// `reveal` ∈ [0,1] anima el descenso desde el borde superior. `lines` son los
/// mensajes (los más nuevos al final).
pub fn render_panel(buf: &mut [u8], w: usize, h: usize, pitch: usize, lines: &[String], reveal: f32) {
    let reveal = reveal.clamp(0.0, 1.0);
    if reveal <= 0.0 {
        return;
    }
    // Escala del texto según el ancho (legible en 1280 y en 800).
    let scale = (w / 320).max(1);
    let line_h = 9 * scale; // 8 px glifo + 1 de interlínea
    let pad = 8 * scale;
    // Altura plena del panel: ~45% del alto. Animada por reveal.
    let full = (h * 45 / 100).max(line_h * 3 + pad * 2);
    let panel_h = ((full as f32) * reveal) as usize;
    if panel_h < line_h {
        return;
    }
    darken_rect(buf, w, h, pitch, 0, panel_h, 0.82);
    // Banda de acento en el borde inferior del panel (1 línea fina).
    let edge = scale.max(1);
    if panel_h >= edge {
        for y in (panel_h - edge)..panel_h {
            let row = y * pitch;
            for x in 0..w {
                let idx = row + x * 4;
                if idx + 4 <= buf.len() {
                    buf[idx] = 247;
                    buf[idx + 1] = 131;
                    buf[idx + 2] = 124; // #7c83f7 en BGR
                }
            }
        }
    }
    // Título + cuántas líneas entran.
    let fg = (210, 214, 230);
    draw_text(buf, w, h, pitch, pad, pad, "logs de arranque (auto)", scale, (140, 146, 200));
    let rows = (panel_h.saturating_sub(pad * 2 + line_h)) / line_h;
    if rows == 0 {
        return;
    }
    let start = lines.len().saturating_sub(rows);
    for (i, line) in lines[start..].iter().enumerate() {
        let y = pad + line_h + i * line_h;
        // Cortamos la línea al ancho disponible (monoespaciado 8*scale por char).
        let max_chars = (w.saturating_sub(pad * 2)) / (8 * scale);
        let text: String = line.chars().take(max_chars).collect();
        draw_text(buf, w, h, pitch, pad, y, &text, scale, fg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn any_set(buf: &[u8]) -> bool {
        buf.chunks(4).any(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
    }

    #[test]
    fn espacio_no_dibuja_nada() {
        let (w, h, pitch) = (64, 16, 64 * 4);
        let mut buf = vec![0u8; pitch * h];
        draw_text(&mut buf, w, h, pitch, 0, 0, "   ", 1, (255, 255, 255));
        assert!(!any_set(&buf), "los espacios no encienden píxeles");
    }

    #[test]
    fn letra_dibuja_pixeles() {
        let (w, h, pitch) = (64, 16, 64 * 4);
        let mut buf = vec![0u8; pitch * h];
        draw_text(&mut buf, w, h, pitch, 0, 0, "A", 1, (255, 255, 255));
        assert!(any_set(&buf), "una letra enciende píxeles");
    }

    #[test]
    fn parse_kmsg_extrae_nivel_y_texto() {
        let rec = "3,123,456789,-;algo fallo en el driver\n".as_bytes();
        let (level, msg) = parse_record(rec).unwrap();
        assert_eq!(level, 3);
        assert_eq!(msg, "algo fallo en el driver");
    }

    #[test]
    fn panel_oscurece_la_zona_superior() {
        let (w, h, pitch) = (320, 200, 320 * 4);
        // Fondo blanco; tras el panel, la franja superior debe oscurecerse.
        let mut buf = vec![255u8; pitch * h];
        render_panel(&mut buf, w, h, pitch, &["hola".into()], 1.0);
        // Un píxel cerca del tope (dentro del panel) ya no es blanco puro.
        let i = (2 * pitch) + (w / 2) * 4;
        assert!(buf[i] < 255 || buf[i + 1] < 255 || buf[i + 2] < 255, "el panel oscurece el fondo");
    }
}
