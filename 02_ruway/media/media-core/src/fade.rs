//! fade — curvas de fundido y crossfade entre dos pistas (el kernel puro
//! de A6 "gapless / crossfade" de `PARIDAD.md`).
//!
//! La máquina de transición entre pistas vive en la capa de playlist de
//! la app (es la que sabe cuándo una pista termina y cuál sigue); este
//! módulo da la parte pura y testeable: la **curva** de ganancia y la
//! **mezcla por bloque** de dos buffers intercalados. Sin estado, sin
//! alocar en la ruta caliente.
//!
//! Dos curvas: `linear` (suma de ganancias constante = 1, útil para
//! fundidos simples) y `equal_power` (suma de *potencias* constante,
//! `g_out² + g_in² = 1`, la que no produce el bache de volumen audible a
//! mitad de un crossfade de material no correlacionado).

use std::f32::consts::FRAC_PI_2;

/// Par de ganancias `(saliente, entrante)` para una curva **lineal** en
/// `progress ∈ [0,1]`: `(1-p, p)`. La suma es siempre 1.
pub fn linear(progress: f32) -> (f32, f32) {
    let p = progress.clamp(0.0, 1.0);
    (1.0 - p, p)
}

/// Par de ganancias `(saliente, entrante)` para un crossfade de **igual
/// potencia** en `progress ∈ [0,1]`: `(cos(p·π/2), sin(p·π/2))`. Cumple
/// `g_out² + g_in² = 1`, así la energía percibida se mantiene plana — es
/// la curva correcta para encadenar pistas distintas sin bache.
pub fn equal_power(progress: f32) -> (f32, f32) {
    let p = progress.clamp(0.0, 1.0);
    let ang = p * FRAC_PI_2;
    (ang.cos(), ang.sin())
}

/// Mezcla `a` (saliente) y `b` (entrante) — ambos intercalados, mismo
/// largo y `channels` — en `out`, recorriendo el `progress` linealmente de
/// `start` a `end` a lo largo del bloque (un frame por paso). `out` debe
/// medir lo mismo que el bloque más corto de los dos; lo que sobre queda
/// intacto. `equal` elige curva de igual potencia (`true`) o lineal.
///
/// Devuelve el `progress` con el que quedaría el frame siguiente, para
/// encadenar bloques consecutivos.
pub fn crossfade_into(
    a: &[f32],
    b: &[f32],
    out: &mut [f32],
    channels: usize,
    start: f32,
    end: f32,
    equal: bool,
) -> f32 {
    let ch = channels.max(1);
    let frames = (a.len() / ch).min(b.len() / ch).min(out.len() / ch);
    if frames == 0 {
        return start.clamp(0.0, 1.0);
    }
    let step = if frames > 1 {
        (end - start) / frames as f32
    } else {
        0.0
    };
    let curve = if equal { equal_power } else { linear };
    for f in 0..frames {
        let p = start + step * f as f32;
        let (g_out, g_in) = curve(p);
        for c in 0..ch {
            let idx = f * ch + c;
            out[idx] = a[idx] * g_out + b[idx] * g_in;
        }
    }
    (start + step * frames as f32).clamp(0.0, 1.0)
}

/// Aplica un fundido **de entrada** (silencio → pleno) sobre `buf`
/// intercalado, de `start` a `end` del progreso. Variante in-place de un
/// crossfade contra silencio. Devuelve el progreso del frame siguiente.
pub fn fade_in(buf: &mut [f32], channels: usize, start: f32, end: f32) -> f32 {
    ramp(buf, channels, start, end, true)
}

/// Aplica un fundido **de salida** (pleno → silencio) sobre `buf`.
pub fn fade_out(buf: &mut [f32], channels: usize, start: f32, end: f32) -> f32 {
    ramp(buf, channels, start, end, false)
}

fn ramp(buf: &mut [f32], channels: usize, start: f32, end: f32, fade_in: bool) -> f32 {
    let ch = channels.max(1);
    let frames = buf.len() / ch;
    if frames == 0 {
        return start.clamp(0.0, 1.0);
    }
    let step = if frames > 1 {
        (end - start) / frames as f32
    } else {
        0.0
    };
    for f in 0..frames {
        let p = start + step * f as f32;
        // fade_in usa la ganancia entrante; fade_out la saliente.
        let (g_out, g_in) = equal_power(p);
        let g = if fade_in { g_in } else { g_out };
        for c in 0..ch {
            buf[f * ch + c] *= g;
        }
    }
    (start + step * frames as f32).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_endpoints_y_suma() {
        assert_eq!(linear(0.0), (1.0, 0.0));
        assert_eq!(linear(1.0), (0.0, 1.0));
        let (o, i) = linear(0.3);
        assert!((o + i - 1.0).abs() < 1e-6);
    }

    #[test]
    fn equal_power_endpoints_y_potencia() {
        let (o0, i0) = equal_power(0.0);
        assert!((o0 - 1.0).abs() < 1e-6 && i0.abs() < 1e-6);
        let (o1, i1) = equal_power(1.0);
        assert!(o1.abs() < 1e-6 && (i1 - 1.0).abs() < 1e-6);
        // g_out² + g_in² = 1 en todo el rango.
        for k in 0..=10 {
            let (o, i) = equal_power(k as f32 / 10.0);
            assert!((o * o + i * i - 1.0).abs() < 1e-5, "p={k}");
        }
    }

    #[test]
    fn clamp_fuera_de_rango() {
        assert_eq!(linear(-1.0), (1.0, 0.0));
        assert_eq!(linear(2.0), (0.0, 1.0));
        let (o, i) = equal_power(5.0);
        assert!(o.abs() < 1e-6 && (i - 1.0).abs() < 1e-6);
    }

    #[test]
    fn crossfade_mezcla_extremos() {
        // a constante 1.0, b constante 0.0; con progress 0→0 el bloque
        // entero usa g_out=1 → sale a.
        let a = vec![1.0; 4];
        let b = vec![0.0; 4];
        let mut out = vec![0.0; 4];
        crossfade_into(&a, &b, &mut out, 2, 0.0, 0.0, true);
        assert!(out.iter().all(|&v| (v - 1.0).abs() < 1e-6));

        // progress 1→1 → g_in=1 → sale b.
        let a = vec![1.0; 4];
        let b = vec![0.5; 4];
        let mut out = vec![0.0; 4];
        crossfade_into(&a, &b, &mut out, 2, 1.0, 1.0, true);
        assert!(out.iter().all(|&v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn crossfade_punto_medio_igual_potencia() {
        // En el medio del bloque, igual potencia: a y b al mismo nivel se
        // suman con g_out=g_in=cos(π/4)=sin(π/4).
        let a = vec![1.0, 1.0, 1.0, 1.0];
        let b = vec![1.0, 1.0, 1.0, 1.0];
        let mut out = vec![0.0; 4];
        // start=0.5 end=0.5 → todo el bloque en p=0.5.
        crossfade_into(&a, &b, &mut out, 1, 0.5, 0.5, true);
        let expected = (FRAC_PI_2 * 0.5).cos() + (FRAC_PI_2 * 0.5).sin();
        assert!(out.iter().all(|&v| (v - expected).abs() < 1e-5));
    }

    #[test]
    fn crossfade_avanza_progress() {
        let a = vec![0.0; 8];
        let b = vec![0.0; 8];
        let mut out = vec![0.0; 8];
        // 4 frames estéreo, de 0.0 a 0.4 → siguiente = 0.4.
        let next = crossfade_into(&a, &b, &mut out, 2, 0.0, 0.4, false);
        assert!((next - 0.4).abs() < 1e-6);
    }

    #[test]
    fn fade_in_out_extremos() {
        // fade_in de 0→0: ganancia entrante en p=0 es 0 → silencio.
        let mut buf = vec![1.0; 4];
        fade_in(&mut buf, 2, 0.0, 0.0);
        assert!(buf.iter().all(|&v| v.abs() < 1e-6));

        // fade_out de 0→0: ganancia saliente en p=0 es 1 → intacto.
        let mut buf = vec![1.0; 4];
        fade_out(&mut buf, 2, 0.0, 0.0);
        assert!(buf.iter().all(|&v| (v - 1.0).abs() < 1e-6));

        // fade_in de 1→1: entrante en p=1 es 1 → intacto.
        let mut buf = vec![0.8; 4];
        fade_in(&mut buf, 2, 1.0, 1.0);
        assert!(buf.iter().all(|&v| (v - 0.8).abs() < 1e-6));
    }
}
