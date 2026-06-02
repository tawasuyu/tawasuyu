//! channels — conversión de layout de canales (downmix / upmix), el hueco
//! que `PARIDAD.md` deja en A5 ("downmix/upmix").
//!
//! El trait [`crate::AudioSource`] le pide a cada fuente que llene el
//! buffer con la cantidad de canales que pide el sink, así que la
//! conversión de N→M canales es responsabilidad de quien decodifica. Este
//! módulo da la pieza pura y reutilizable para hacerlo bien: matrices de
//! mezcla estándar (ITU-R BS.775 para 5.1→estéreo) en vez de que cada
//! fuente improvise.
//!
//! Todo es por-frame, sin estado y sin alocar en la ruta caliente
//! ([`remix_into`] escribe en un buffer del caller). `remix` es el atajo
//! que sí aloca, para tests y rutas no-realtime.

/// Coeficiente -3 dB (≈ 1/√2). Peso estándar de los canales central y
/// surround al plegar a estéreo.
const M3DB: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Convierte `input` (intercalado, `in_ch` canales) a `out` (intercalado,
/// `out_ch` canales). `out` debe medir `frames * out_ch` donde
/// `frames = input.len() / in_ch`; si no, se procesa el mínimo y el
/// sobrante de `out` queda intacto.
///
/// Reglas (cubre lo que aparece en la práctica; el resto cae a un mapeo
/// sensato sin romper):
/// - `in_ch == out_ch`: copia directa.
/// - hacia **mono** (`out_ch == 1`): promedia todos los canales de entrada.
/// - desde **mono** (`in_ch == 1`): replica la muestra a todos los de salida.
/// - **5.1 → estéreo** (`in_ch == 6`, orden L R C LFE Ls Rs): downmix ITU
///   `Lo = L + .707·C + .707·Ls`, `Ro = R + .707·C + .707·Rs` (LFE fuera).
/// - **estéreo → 3+** : L/R al frente, los demás en silencio.
/// - resto: copia los `min(in_ch,out_ch)` primeros y rellena con silencio.
pub fn remix_into(input: &[f32], in_ch: usize, out: &mut [f32], out_ch: usize) {
    let in_ch = in_ch.max(1);
    let out_ch = out_ch.max(1);
    let frames = (input.len() / in_ch).min(if out_ch == 0 { 0 } else { out.len() / out_ch });

    if in_ch == out_ch {
        let n = frames * in_ch;
        out[..n].copy_from_slice(&input[..n]);
        return;
    }

    for f in 0..frames {
        let i = &input[f * in_ch..f * in_ch + in_ch];
        let o = &mut out[f * out_ch..f * out_ch + out_ch];

        if out_ch == 1 {
            // Downmix a mono: promedio simple.
            let sum: f32 = i.iter().sum();
            o[0] = sum / in_ch as f32;
        } else if in_ch == 1 {
            // Upmix desde mono: misma muestra a todos.
            for s in o.iter_mut() {
                *s = i[0];
            }
        } else if in_ch == 6 && out_ch == 2 {
            // 5.1 (L R C LFE Ls Rs) → estéreo, downmix ITU-R BS.775.
            let (l, r, c, ls, rs) = (i[0], i[1], i[2], i[4], i[5]);
            o[0] = l + M3DB * c + M3DB * ls;
            o[1] = r + M3DB * c + M3DB * rs;
        } else {
            // Caso general: copia el solapamiento canal-a-canal y rellena
            // el resto con silencio (estéreo→4ch deja front L/R y calla
            // el resto; 3ch→2ch toma los dos primeros).
            let common = in_ch.min(out_ch);
            o[..common].copy_from_slice(&i[..common]);
            for s in o[common..].iter_mut() {
                *s = 0.0;
            }
        }
    }
}

/// Igual que [`remix_into`] pero alocando el buffer de salida. Para tests
/// y rutas no-realtime.
pub fn remix(input: &[f32], in_ch: usize, out_ch: usize) -> Vec<f32> {
    let in_ch = in_ch.max(1);
    let out_ch = out_ch.max(1);
    let frames = input.len() / in_ch;
    let mut out = vec![0.0; frames * out_ch];
    remix_into(input, in_ch, &mut out, out_ch);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_si_igual() {
        let input = vec![0.1, 0.2, 0.3, 0.4];
        let out = remix(&input, 2, 2);
        assert_eq!(out, input);
    }

    #[test]
    fn mono_a_estereo_duplica() {
        let input = vec![0.5, -0.3]; // 2 frames mono
        let out = remix(&input, 1, 2);
        assert_eq!(out, vec![0.5, 0.5, -0.3, -0.3]);
    }

    #[test]
    fn estereo_a_mono_promedia() {
        // frame 1: (0.4, 0.6) → 0.5 ; frame 2: (-0.2, 0.2) → 0.0
        let input = vec![0.4, 0.6, -0.2, 0.2];
        let out = remix(&input, 2, 1);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.5).abs() < 1e-6);
        assert!(out[1].abs() < 1e-6);
    }

    #[test]
    fn downmix_51_a_estereo_coeficientes_itu() {
        // L R C LFE Ls Rs = 1, 2, 4, 9(LFE ignorado), 8, 16
        let input = vec![1.0, 2.0, 4.0, 9.0, 8.0, 16.0];
        let out = remix(&input, 6, 2);
        assert_eq!(out.len(), 2);
        // Lo = L + .707·C + .707·Ls = 1 + .707·4 + .707·8
        let lo = 1.0 + M3DB * 4.0 + M3DB * 8.0;
        let ro = 2.0 + M3DB * 4.0 + M3DB * 16.0;
        assert!((out[0] - lo).abs() < 1e-5, "Lo = {}, esperaba {lo}", out[0]);
        assert!((out[1] - ro).abs() < 1e-5, "Ro = {}, esperaba {ro}", out[1]);
    }

    #[test]
    fn estereo_a_cuatro_pone_front_y_calla_resto() {
        let input = vec![0.7, -0.7]; // 1 frame estéreo
        let out = remix(&input, 2, 4);
        assert_eq!(out, vec![0.7, -0.7, 0.0, 0.0]);
    }

    #[test]
    fn multicanal_a_mono_promedia_todo() {
        // 4 canales en un frame: media de (1,1,1,1)·... → 0.25*sum
        let input = vec![1.0, 0.0, 0.0, 0.0];
        let out = remix(&input, 4, 1);
        assert_eq!(out.len(), 1);
        assert!((out[0] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn remix_into_respeta_buffer_corto() {
        // out sólo tiene lugar para 1 frame estéreo; input trae 2 frames.
        let input = vec![1.0, 2.0, 3.0, 4.0]; // 2 frames estéreo
        let mut out = vec![0.0; 2]; // 1 frame estéreo
        remix_into(&input, 2, &mut out, 2);
        assert_eq!(out, vec![1.0, 2.0]); // sólo el primer frame
    }
}
