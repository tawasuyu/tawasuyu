//! Primer píxel **gráfico** del arranque: pinta el GOP de UEFI antes de cargar
//! el kernel, así desde el frame cero hay GUI y no texto de firmware. Fija el
//! modo nativo que el kernel hereda (efifb/simpledrm), base del arranque sin
//! parpadeo (ver `SDD-ARRANQUE-SIN-PARPADEO.md`).
//!
//! Es **best-effort**: si no hay GOP o el modo es Blt-only, no pinta y el
//! arranque sigue igual. Por ahora pinta el fondo de marca + una marca central
//! sólida (placeholder); el splash animado vive aparte en `arje-splash`.

use uefi::boot;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};

/// Fondo de marca (igual que el `bg_app` de mirada/greeter) — la continuidad de
/// color hace que el handoff a la GUI no se note.
const BG: (u8, u8, u8) = (18, 18, 24);
/// Acento de la marca para la marca central.
const ACCENT: (u8, u8, u8) = (124, 131, 247);

/// Pinta el splash de arranque sobre el framebuffer del GOP. No falla: ante
/// cualquier problema (sin GOP, modo Blt-only) simplemente no pinta.
pub fn paint_boot_splash() {
    let Ok(handle) = boot::get_handle_for_protocol::<GraphicsOutput>() else {
        return;
    };
    let Ok(mut gop) = boot::open_protocol_exclusive::<GraphicsOutput>(handle) else {
        return;
    };
    let info = gop.current_mode_info();
    let (w, h) = info.resolution();
    let stride = info.stride();
    let fmt = info.pixel_format();
    if matches!(fmt, PixelFormat::BltOnly) || w == 0 || h == 0 {
        return; // sin acceso directo al framebuffer
    }

    let bg = encode(fmt, BG);
    let accent = encode(fmt, ACCENT);

    // Marca central: un cuadrado sólido del acento, ~1/6 del lado menor.
    let side = (w.min(h) / 6).max(8);
    let lx = w / 2 - side / 2;
    let ly = h / 2 - side / 2;

    let mut fb = gop.frame_buffer();
    let cap = fb.size();
    for y in 0..h {
        let row = y * stride;
        for x in 0..w {
            let inside = x >= lx && x < lx + side && y >= ly && y < ly + side;
            let px = if inside { accent } else { bg };
            let idx = (row + x) * 4;
            if idx + 4 <= cap {
                // SAFETY: idx+4 ≤ size, y respetamos el formato/stride del modo.
                unsafe { fb.write_value(idx, px) };
            }
        }
    }
}

/// Empaqueta un color RGB al orden de bytes del modo (32 bpp, último byte
/// reservado). `Bgr` invierte; `Rgb`/`Bitmask` van como RGB (best-effort).
fn encode(fmt: PixelFormat, c: (u8, u8, u8)) -> [u8; 4] {
    match fmt {
        PixelFormat::Bgr => [c.2, c.1, c.0, 0],
        _ => [c.0, c.1, c.2, 0],
    }
}
