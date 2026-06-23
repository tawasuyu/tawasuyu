//! Render **puro** del splash — sin DRM, sin I/O, totalmente testeable.
//!
//! La capa DRM (`drm_present.rs`) crea el framebuffer en formato
//! `XRGB8888` y le entrega a estas funciones el buffer mapeado + su `pitch`
//! (bytes por fila, que puede exceder `w*4` por padding del driver). Acá sólo
//! pintamos píxeles en función del tiempo `t_ms`, así la animación se certifica
//! con asserts numéricos sin hardware (Regla 8: evidencia de texto, no PNG).
//!
//! ## Continuidad de marca
//!
//! Los mismos colores que `arje-loader::gop` (fondo `BG` + marca `ACCENT`):
//! el logo que pintó el loader desde UEFI y este splash comparten paleta, así
//! el handoff loader→splash no se nota. `BG` es además el `bg_app` de
//! mirada/greeter, cerrando la cadena hasta el primer frame de la GUI.

/// Fondo de marca (== `bg_app` de mirada/greeter y `BG` del loader).
pub const BG: (u8, u8, u8) = (18, 18, 24);
/// Acento de la marca (== `ACCENT` del loader).
pub const ACCENT: (u8, u8, u8) = (124, 131, 247);

/// Período de la respiración del logo, en ms.
const BREATH_MS: f32 = 2400.0;
/// Período del barrido del indicador de progreso indeterminado, en ms.
const SWEEP_MS: f32 = 1600.0;

/// Pinta un frame completo del splash en `buf` (formato `XRGB8888`, es decir
/// bytes `[B, G, R, X]` en little-endian) para el instante `t_ms`.
///
/// `pitch` es el stride en bytes de cada fila. Los bytes de padding más allá de
/// `w*4` en cada fila se dejan intactos (no son visibles). `buf` debe tener al
/// menos `pitch * h` bytes; filas/píxeles que no entren se ignoran (defensivo).
///
/// `fade` ∈ [0,1] funde todo el contenido hacia el fondo de marca `BG` (no a
/// negro): `0` = splash normal, `1` = `BG` sólido. Es el fade-out del handoff
/// hacia mirada (Fase 2) — al terminar la pantalla queda en el mismo `bg_app`
/// que mirada va a mostrar, así el traspaso no se nota.
pub fn paint_frame(buf: &mut [u8], w: usize, h: usize, pitch: usize, t_ms: u64, fade: f32) {
    let t = t_ms as f32;
    let fade = fade.clamp(0.0, 1.0);

    // Respiración: brillo del logo oscilando suave entre `lo` y 1.0.
    let breath = 0.5 + 0.5 * (t / BREATH_MS * std::f32::consts::TAU).sin(); // 0..1
    let lo = 0.45;
    let logo_k = lo + (1.0 - lo) * breath;
    let logo = scale(ACCENT, logo_k);

    // Marca central: cuadrado del acento, ~1/6 del lado menor (igual que el
    // placeholder del loader, ahora respirando).
    let side = (w.min(h) / 6).max(8);
    let lx = w / 2 - side / 2;
    let ly = h / 2 - side / 2;

    // Barra de progreso indeterminada: una banda angosta cerca del borde
    // inferior que barre de izquierda a derecha y envuelve. Da sensación de
    // actividad sin conocer el progreso real del boot.
    let bar_h = (h / 90).max(2);
    let bar_y = h.saturating_sub(h / 6);
    let band = (w / 5).max(16);
    let sweep = ((t / SWEEP_MS).fract() * (w + band) as f32) as i64 - band as i64; // -band..w
    let bar_col = scale(ACCENT, 0.7);

    let bg_bar = scale(BG, 1.8); // riel del progreso, apenas más claro

    // Pre-encodamos los píxeles ya fundidos (el fade es uniforme en el frame),
    // así el doble loop sólo elige cuál escribir — sin lerp por píxel.
    let bg = encode(lerp(BG, BG, fade)); // == BG, pero deja explícito el patrón
    let logo_px = encode(lerp(logo, BG, fade));
    let bar_px = encode(lerp(bar_col, BG, fade));
    let bg_bar_px = encode(lerp(bg_bar, BG, fade));

    for y in 0..h {
        let row = y * pitch;
        if row >= buf.len() {
            break;
        }
        let in_bar_row = y >= bar_y && y < bar_y + bar_h;
        for x in 0..w {
            let idx = row + x * 4;
            if idx + 4 > buf.len() {
                break;
            }
            let px = if x >= lx && x < lx + side && y >= ly && y < ly + side {
                logo_px
            } else if in_bar_row {
                let xi = x as i64;
                if xi >= sweep && xi < sweep + band as i64 {
                    bar_px
                } else {
                    bg_bar_px
                }
            } else {
                bg
            };
            buf[idx..idx + 4].copy_from_slice(&px);
        }
    }
}

/// Color del panel de la tarjeta del greeter simulado.
const CARD: (u8, u8, u8) = (32, 32, 46);
/// Color de los campos/inputs de la tarjeta.
const FIELD: (u8, u8, u8) = (52, 52, 70);

/// Pinta un **greeter simulado** (mockup de la tarjeta de login) sobre `BG`,
/// apareciendo según `appear` ∈ [0,1] (`0` = sólo `BG`, igual que el frame final
/// del fade-out del splash → el traspaso es continuo; `1` = tarjeta visible).
///
/// No es el greeter real de mirada (eso es EGL/GLES, necesita GPU). Es un
/// sustituto sobre DRM dumb-buffer para **ver el crossfade end-to-end** en
/// QEMU sin GPU: el splash funde a `BG`, suelta el master, y este frame hace
/// aparecer la tarjeta sobre el mismo `BG`. Demostración, no producto.
pub fn paint_greeter(buf: &mut [u8], w: usize, h: usize, pitch: usize, appear: f32) {
    let appear = appear.clamp(0.0, 1.0);
    // Tarjeta centrada, ~28% del ancho × ~46% del alto, acotada a la pantalla
    // (en pantallas chicas, como en los tests, no debe desbordar).
    let cw = (w * 28 / 100).clamp(16, w);
    let ch = (h * 46 / 100).clamp(16, h);
    let cx = (w - cw) / 2;
    let cy = (h - ch) / 2;
    // Banda de acento (cabecera) arriba de la tarjeta.
    let head_h = (ch / 8).max(8);
    // Dos "campos" (usuario / contraseña) dentro de la tarjeta.
    let pad = (cw / 10).max(8);
    let field_h = (ch / 9).max(10);
    let field_w = cw - pad * 2;
    let f1_y = cy + head_h + pad;
    let f2_y = f1_y + field_h + pad / 2;

    let bg = encode(BG);
    let card_px = encode(lerp(BG, CARD, appear));
    let head_px = encode(lerp(BG, ACCENT, appear));
    let field_px = encode(lerp(BG, FIELD, appear));

    for y in 0..h {
        let row = y * pitch;
        if row >= buf.len() {
            break;
        }
        for x in 0..w {
            let idx = row + x * 4;
            if idx + 4 > buf.len() {
                break;
            }
            let in_card = x >= cx && x < cx + cw && y >= cy && y < cy + ch;
            let px = if !in_card {
                bg
            } else if y < cy + head_h {
                head_px
            } else if x >= cx + pad
                && x < cx + pad + field_w
                && ((y >= f1_y && y < f1_y + field_h) || (y >= f2_y && y < f2_y + field_h))
            {
                field_px
            } else {
                card_px
            };
            buf[idx..idx + 4].copy_from_slice(&px);
        }
    }
}

/// Interpola linealmente `a → b` por `t` ∈ [0,1] (por canal).
fn lerp(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

/// Multiplica un color por un factor de brillo `k` (saturando a 0..255).
fn scale(c: (u8, u8, u8), k: f32) -> (u8, u8, u8) {
    let f = |v: u8| (v as f32 * k).round().clamp(0.0, 255.0) as u8;
    (f(c.0), f(c.1), f(c.2))
}

/// Empaqueta un color RGB al orden de bytes de `XRGB8888` little-endian:
/// la palabra es `0x00RRGGBB`, en memoria `[B, G, R, 0]`.
fn encode(c: (u8, u8, u8)) -> [u8; 4] {
    [c.2, c.1, c.0, 0]
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 64;
    const H: usize = 48;

    fn buf_for(pitch: usize) -> Vec<u8> {
        vec![0u8; pitch * H]
    }

    fn px_at(buf: &[u8], pitch: usize, x: usize, y: usize) -> (u8, u8, u8) {
        let i = y * pitch + x * 4;
        (buf[i + 2], buf[i + 1], buf[i]) // R, G, B (de [B,G,R,X])
    }

    #[test]
    fn fondo_en_esquina_es_bg() {
        let pitch = W * 4;
        let mut b = buf_for(pitch);
        paint_frame(&mut b, W, H, pitch, 0, 0.0);
        assert_eq!(px_at(&b, pitch, 0, 0), BG);
    }

    #[test]
    fn centro_es_el_logo_no_el_fondo() {
        let pitch = W * 4;
        let mut b = buf_for(pitch);
        paint_frame(&mut b, W, H, pitch, 0, 0.0);
        let c = px_at(&b, pitch, W / 2, H / 2);
        assert_ne!(c, BG, "el centro debe ser el logo, no el fondo");
        // El logo es el acento (atenuado por la respiración) → su rojo domina
        // sobre el del fondo.
        assert!(c.0 > BG.0, "el logo es más brillante que el fondo");
    }

    #[test]
    fn la_respiracion_cambia_el_brillo_del_logo() {
        let pitch = W * 4;
        // Pico de la respiración (t=BREATH/4 → sin=1) vs valle (t=3·BREATH/4).
        let mut pico = buf_for(pitch);
        let mut valle = buf_for(pitch);
        paint_frame(&mut pico, W, H, pitch, (BREATH_MS / 4.0) as u64, 0.0);
        paint_frame(&mut valle, W, H, pitch, (3.0 * BREATH_MS / 4.0) as u64, 0.0);
        let cp = px_at(&pico, pitch, W / 2, H / 2);
        let cv = px_at(&valle, pitch, W / 2, H / 2);
        assert!(
            cp.0 > cv.0 && cp.1 > cv.1 && cp.2 > cv.2,
            "el logo en el pico ({cp:?}) debe ser más brillante que en el valle ({cv:?})"
        );
    }

    #[test]
    fn fade_uno_funde_todo_a_bg() {
        // Con fade=1.0 todo el frame queda en BG sólido (handoff terminado):
        // ni el logo ni la barra deben sobresalir.
        let pitch = W * 4;
        let mut b = buf_for(pitch);
        paint_frame(&mut b, W, H, pitch, 0, 1.0);
        assert_eq!(px_at(&b, pitch, W / 2, H / 2), BG, "centro funde a BG");
        assert_eq!(px_at(&b, pitch, 0, 0), BG, "esquina sigue BG");
    }

    #[test]
    fn fade_intermedio_atenua_el_logo_hacia_bg() {
        // A media fundición el logo está entre su color pleno y BG.
        let pitch = W * 4;
        let mut pleno = buf_for(pitch);
        let mut medio = buf_for(pitch);
        paint_frame(&mut pleno, W, H, pitch, 0, 0.0);
        paint_frame(&mut medio, W, H, pitch, 0, 0.5);
        let cp = px_at(&pleno, pitch, W / 2, H / 2);
        let cm = px_at(&medio, pitch, W / 2, H / 2);
        // El logo es más brillante que BG; al fundir, su brillo baja hacia BG.
        assert!(cm.0 < cp.0 && cm.0 > BG.0, "el rojo del logo baja pero aún supera BG");
    }

    #[test]
    fn greeter_aparece_desde_bg() {
        let pitch = W * 4;
        // appear=0 → todo BG (continuidad con el frame final del splash).
        let mut cero = buf_for(pitch);
        paint_greeter(&mut cero, W, H, pitch, 0.0);
        assert_eq!(px_at(&cero, pitch, W / 2, H / 2), BG, "appear=0 es BG puro");
        // appear=1 → el centro (dentro de la tarjeta) deja de ser BG.
        let mut uno = buf_for(pitch);
        paint_greeter(&mut uno, W, H, pitch, 1.0);
        assert_ne!(px_at(&uno, pitch, W / 2, H / 2), BG, "appear=1 muestra la tarjeta");
        // La esquina siempre es BG (la tarjeta está centrada).
        assert_eq!(px_at(&uno, pitch, 0, 0), BG, "el fondo fuera de la tarjeta es BG");
    }

    #[test]
    fn respeta_el_padding_del_pitch() {
        // pitch con padding: los bytes más allá de w*4 quedan intactos.
        let pitch = W * 4 + 32;
        let mut b = buf_for(pitch);
        // marca de centinela en la zona de padding de la primera fila.
        for i in (W * 4)..pitch {
            b[i] = 0xAB;
        }
        paint_frame(&mut b, W, H, pitch, 0, 0.0);
        for i in (W * 4)..pitch {
            assert_eq!(b[i], 0xAB, "el padding del pitch no debe tocarse");
        }
    }

    #[test]
    fn no_desborda_con_buffer_corto() {
        // Un buffer más corto que pitch*h no debe entrar en pánico.
        let pitch = W * 4;
        let mut b = vec![0u8; pitch * (H / 2)];
        paint_frame(&mut b, W, H, pitch, 123, 0.0);
    }
}
