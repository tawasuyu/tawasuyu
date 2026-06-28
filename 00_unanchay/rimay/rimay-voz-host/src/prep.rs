//! Preparación del audio crudo del micrófono para el VAD/STT: downmix a mono,
//! remuestreo a la tasa objetivo y conversión a `i16`. Puro y testeable sin
//! micrófono — el driver de captura sólo lo encadena.
//!
//! El micrófono entrega `f32` intercalado a la tasa/canales que negoció el
//! dispositivo (típico 48 kHz estéreo); el STT real (whisper) quiere 16 kHz
//! mono. Estas tres funciones cierran esa brecha.

/// Promedia los canales intercalados a una sola pista mono.
pub fn a_mono(intercalado: &[f32], canales: u16) -> Vec<f32> {
    let c = canales.max(1) as usize;
    if c == 1 {
        return intercalado.to_vec();
    }
    intercalado
        .chunks(c)
        .map(|cuadro| cuadro.iter().sum::<f32>() / cuadro.len() as f32)
        .collect()
}

/// Convierte `f32` en `[-1, 1]` a `i16`, con clamp para no desbordar.
pub fn a_i16(mono: &[f32]) -> Vec<i16> {
    mono.iter()
        .map(|&x| (x.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect()
}

/// Remuestreador lineal **con estado**: mantiene la posición fraccional y la
/// última muestra entre llamadas, así no hay clicks en los bordes de chunk
/// (el stream del micrófono llega troceado). Si origen y destino coinciden, es
/// una copia.
#[derive(Debug, Clone)]
pub struct Remuestreador {
    de_hz: u32,
    a_hz: u32,
    /// Posición de la próxima muestra de salida, en coordenadas de entrada
    /// donde el índice 0 es [`Self::previa`] (la última muestra del chunk
    /// anterior).
    pos: f64,
    /// Última muestra de entrada vista (el `x[-1]` virtual del próximo chunk).
    previa: f32,
}

impl Remuestreador {
    pub fn new(de_hz: u32, a_hz: u32) -> Self {
        Self {
            de_hz: de_hz.max(1),
            a_hz: a_hz.max(1),
            pos: 0.0,
            previa: 0.0,
        }
    }

    /// Remuestrea un chunk, continuando desde donde quedó el anterior.
    pub fn procesar(&mut self, entrada: &[f32]) -> Vec<f32> {
        if self.de_hz == self.a_hz {
            return entrada.to_vec();
        }
        if entrada.is_empty() {
            return Vec::new();
        }
        // Muestras de entrada por muestra de salida.
        let paso = self.de_hz as f64 / self.a_hz as f64;
        let n = entrada.len();
        // Muestra en el índice virtual j: j==0 → previa, si no entrada[j-1].
        let muestra = |j: usize| -> f32 {
            if j == 0 {
                self.previa
            } else {
                entrada[j - 1]
            }
        };
        let mut salida = Vec::with_capacity(((n as f64) / paso) as usize + 1);
        while self.pos < n as f64 {
            let i = self.pos.floor() as usize;
            let f = (self.pos - i as f64) as f32;
            let a = muestra(i);
            let b = muestra(i + 1);
            salida.push(a + (b - a) * f);
            self.pos += paso;
        }
        // El próximo chunk pone su `previa` (= última de éste) en el índice 0;
        // desplazamos las coordenadas restando las `n` muestras consumidas.
        self.pos -= n as f64;
        self.previa = entrada[n - 1];
        salida
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_de_un_canal_es_identidad() {
        assert_eq!(a_mono(&[0.1, 0.2, 0.3], 1), vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn mono_promedia_estereo() {
        // [L,R, L,R] → [(L+R)/2, ...]
        let m = a_mono(&[1.0, 0.0, 0.5, 0.5], 2);
        assert_eq!(m, vec![0.5, 0.5]);
    }

    #[test]
    fn i16_clampea_los_extremos() {
        let v = a_i16(&[0.0, 1.0, -1.0, 2.0, -2.0]);
        assert_eq!(v, vec![0, i16::MAX, -i16::MAX, i16::MAX, -i16::MAX]);
    }

    #[test]
    fn remuestreo_misma_tasa_es_copia() {
        let mut r = Remuestreador::new(16_000, 16_000);
        assert_eq!(r.procesar(&[0.1, 0.2, 0.3]), vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn remuestreo_a_la_mitad_aprox_mitad_de_muestras() {
        // 48k → 16k: factor 3 → ~1/3 de las muestras.
        let mut r = Remuestreador::new(48_000, 16_000);
        let entrada: Vec<f32> = (0..300).map(|i| i as f32).collect();
        let salida = r.procesar(&entrada);
        // ~100 muestras (300/3), con ±1 por la posición fraccional.
        assert!((salida.len() as i32 - 100).abs() <= 1, "len = {}", salida.len());
    }

    #[test]
    fn remuestreo_de_constante_queda_constante() {
        let mut r = Remuestreador::new(48_000, 16_000);
        let salida = r.procesar(&vec![0.7; 300]);
        // Interpolar entre muestras iguales da el mismo valor (salvo el primer
        // punto, que arranca desde previa=0 → arranque suave).
        for &x in salida.iter().skip(1) {
            assert!((x - 0.7).abs() < 1e-6, "x = {x}");
        }
    }

    #[test]
    fn remuestreo_continuo_entre_chunks_no_pierde_el_total() {
        // Procesar en dos mitades debe dar ~lo mismo que de una.
        let entrada: Vec<f32> = (0..600).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut entero = Remuestreador::new(48_000, 16_000);
        let total_entero = entero.procesar(&entrada).len();

        let mut parts = Remuestreador::new(48_000, 16_000);
        let a = parts.procesar(&entrada[..300]).len();
        let b = parts.procesar(&entrada[300..]).len();
        assert!(((a + b) as i32 - total_entero as i32).abs() <= 1);
    }
}
