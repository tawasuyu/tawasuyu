//! Menú de arranque **gráfico** sobre el GOP.
//!
//! arje-loader ya es dueño del framebuffer (`gop::paint_boot_splash`), así que
//! el menú se dibuja con la misma fuente bitmap (`font8x8`) que el panel de logs
//! del splash — **cero modo texto de firmware**. Flechas ↑/↓ mueven la
//! selección, Enter arranca; si el usuario no toca nada, un timeout cae a la
//! entrada por defecto. Best-effort: sin GOP accesible no dibuja (el caller
//! igual sabe qué bootear por el default).

use alloc::string::String;

use font8x8::legacy::BASIC_LEGACY;
use uefi::boot;
use uefi::proto::console::gop::{FrameBuffer, GraphicsOutput, PixelFormat};
use uefi::proto::console::text::{Key, ScanCode};

const BG: (u8, u8, u8) = (18, 18, 24);
const FG: (u8, u8, u8) = (200, 202, 214);
const ACCENT: (u8, u8, u8) = (124, 131, 247);
const SEL_BG: (u8, u8, u8) = (38, 40, 64);
const DIM: (u8, u8, u8) = (110, 112, 130);

/// Muestra el menú y devuelve el índice elegido. `titles` debe ser no-vacío.
/// `timeout_s == 0` → sin cuenta regresiva (espera Enter).
pub fn pick(titles: &[String], default: usize, timeout_s: u64) -> usize {
    let n = titles.len();
    let mut sel = default.min(n - 1);
    let mut counting = timeout_s > 0;
    let mut remaining_ms: i64 = timeout_s as i64 * 1000;
    draw(titles, sel, if counting { remaining_ms / 1000 } else { -1 });
    loop {
        let key = uefi::system::with_stdin(|s| s.read_key().ok().flatten());
        match key {
            Some(Key::Special(ScanCode::UP)) => {
                sel = if sel == 0 { n - 1 } else { sel - 1 };
                counting = false;
                draw(titles, sel, -1);
            }
            Some(Key::Special(ScanCode::DOWN)) => {
                sel = (sel + 1) % n;
                counting = false;
                draw(titles, sel, -1);
            }
            Some(Key::Printable(c)) => {
                let ch = char::from(c);
                if ch == '\r' || ch == '\n' {
                    return sel;
                }
                counting = false;
            }
            _ => {
                boot::stall(40_000); // 40 ms
                if counting {
                    remaining_ms -= 40;
                    if remaining_ms <= 0 {
                        return default.min(n - 1);
                    }
                    // Redibujar el contador ~1×/s.
                    if remaining_ms % 1000 < 40 {
                        draw(titles, sel, remaining_ms / 1000);
                    }
                }
            }
        }
    }
}

/// Geometría/formato del framebuffer del modo vigente.
struct Ctx {
    w: usize,
    h: usize,
    stride: usize,
    cap: usize,
    fmt: PixelFormat,
}

fn draw(titles: &[String], sel: usize, countdown_s: i64) {
    let Ok(handle) = boot::get_handle_for_protocol::<GraphicsOutput>() else {
        return;
    };
    let Ok(mut gop) = boot::open_protocol_exclusive::<GraphicsOutput>(handle) else {
        return;
    };
    let info = gop.current_mode_info();
    let (w, h) = info.resolution();
    if matches!(info.pixel_format(), PixelFormat::BltOnly) || w == 0 || h == 0 {
        return;
    }
    let mut fb = gop.frame_buffer();
    let ctx = Ctx {
        w,
        h,
        stride: info.stride(),
        cap: fb.size(),
        fmt: info.pixel_format(),
    };

    // Fondo de marca (continuidad con el splash y el bg_app del greeter).
    fill_rect(&mut fb, &ctx, 0, 0, w, h, BG);

    // Glifo de 8 px escalado según la resolución para que sea legible.
    let scale = (h / 200).max(2);
    let gh = 8 * scale;
    let line_h = gh + scale * 4;
    let x0 = w / 6;
    let block_h = titles.len() * line_h;
    let start_y = h / 2 - block_h / 2;

    // Encabezado de marca.
    draw_text(&mut fb, &ctx, x0, start_y - gh * 2, "tawasuyu", scale, ACCENT);

    for (i, t) in titles.iter().enumerate() {
        let y = start_y + i * line_h;
        if i == sel {
            fill_rect(
                &mut fb,
                &ctx,
                x0.saturating_sub(scale * 3),
                y.saturating_sub(scale * 2),
                w.saturating_sub(x0 * 2) + scale * 6,
                gh + scale * 3,
                SEL_BG,
            );
        }
        let color = if i == sel { ACCENT } else { FG };
        draw_text(&mut fb, &ctx, x0, y, t, scale, color);
    }

    // Pie: ayuda + cuenta regresiva.
    let foot_y = start_y + block_h + gh;
    let mut foot = String::new();
    if countdown_s >= 0 {
        foot.push_str("arranca solo en ");
        push_num(&mut foot, countdown_s as u64);
        foot.push_str("s   ");
    }
    foot.push_str("flechas: elegir   Enter: arrancar");
    draw_text(&mut fb, &ctx, x0, foot_y, &foot, scale, DIM);
}

/// Rellena un rectángulo (recortado al framebuffer).
fn fill_rect(fb: &mut FrameBuffer<'_>, c: &Ctx, x: usize, y: usize, rw: usize, rh: usize, color: (u8, u8, u8)) {
    let px = enc(c.fmt, color);
    let yend = (y + rh).min(c.h);
    let xend = (x + rw).min(c.w);
    for yy in y..yend {
        let row = yy * c.stride;
        for xx in x..xend {
            let idx = (row + xx) * 4;
            if idx + 4 <= c.cap {
                // SAFETY: idx+4 ≤ cap, formato/stride del modo vigente.
                unsafe { fb.write_value(idx, px) };
            }
        }
    }
}

/// Dibuja un texto ASCII con font8x8 escalado, top-left en `(x, y)`.
fn draw_text(fb: &mut FrameBuffer<'_>, c: &Ctx, x: usize, y: usize, text: &str, scale: usize, color: (u8, u8, u8)) {
    let px = enc(c.fmt, color);
    let mut cx = x;
    for ch in text.chars() {
        let code = ch as usize;
        // font8x8: bit 0 (LSB) = píxel más a la izquierda (igual que el splash).
        let glyph = if code < 128 { BASIC_LEGACY[code] } else { BASIC_LEGACY['?' as usize] };
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8usize {
                if bits & (1 << col) == 0 {
                    continue;
                }
                let bx = cx + col * scale;
                let by = y + row * scale;
                for dy in 0..scale {
                    for dx in 0..scale {
                        let xx = bx + dx;
                        let yy = by + dy;
                        if xx < c.w && yy < c.h {
                            let idx = (yy * c.stride + xx) * 4;
                            if idx + 4 <= c.cap {
                                unsafe { fb.write_value(idx, px) };
                            }
                        }
                    }
                }
            }
        }
        cx += 8 * scale; // avance fijo (monoespaciado, sin kerning)
    }
}

/// Empuja `n` en base 10 al String (sin `format!`, que es pesado en no_std).
fn push_num(s: &mut String, mut n: u64) {
    if n == 0 {
        s.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        s.push(buf[i] as char);
    }
}

/// Empaqueta RGB al orden de bytes del modo (32 bpp). `Bgr` invierte.
fn enc(fmt: PixelFormat, c: (u8, u8, u8)) -> [u8; 4] {
    match fmt {
        PixelFormat::Bgr => [c.2, c.1, c.0, 0],
        _ => [c.0, c.1, c.2, 0],
    }
}
