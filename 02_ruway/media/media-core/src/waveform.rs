//! Onda de **pista completa** (tipo Audacity): envolvente de picos (min/max)
//! del audio decimado a N "buckets", para dibujar la forma de onda de TODA la
//! pista con eje de tiempo + playhead. Núcleo **puro** (regla #2): acá vive el
//! binning y el modelo; el decode/scan de la pista lo hace `foreign-av`
//! (ffmpeg) en un hilo de fondo y alimenta el [`PeaksBuilder`] **streaming**,
//! así no hay que cargar todas las muestras en RAM (una pista de 1 h en f32
//! son cientos de MB).
//!
//! Distinto del visor en vivo (que muestra sólo los últimos ~ms que suenan):
//! esto es la onda estática de la pista entera, scrubbeable.

/// Envolvente de picos: un par `(min, max)` por bucket, en `[-1, 1]`. El
/// consumidor (UI) la dibuja como columnas verticales de `min` a `max`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Waveform {
    peaks: Vec<(f32, f32)>,
}

impl Waveform {
    pub fn peaks(&self) -> &[(f32, f32)] {
        &self.peaks
    }

    pub fn len(&self) -> usize {
        self.peaks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.peaks.is_empty()
    }

    /// Picos de un buffer completo de muestras, decimado a ~`buckets` columnas.
    /// Atajo del [`PeaksBuilder`] para cuando ya tenés todas las muestras.
    pub fn from_samples(samples: &[f32], buckets: usize) -> Waveform {
        let mut b = PeaksBuilder::new(samples.len(), buckets);
        b.push_slice(samples);
        b.finish()
    }
}

/// Acumulador **streaming** de picos: empujás muestras (de a una o por bloque)
/// y emite un `(min, max)` cada `per_bucket` muestras. `new` toma el total
/// estimado (de la duración × sample rate) para dimensionar los buckets; si el
/// conteo real difiere, sale alguna columna de más o de menos (sin romper).
pub struct PeaksBuilder {
    per_bucket: usize,
    cur_min: f32,
    cur_max: f32,
    n: usize,
    peaks: Vec<(f32, f32)>,
}

impl PeaksBuilder {
    /// `total_samples` estimado y cantidad de columnas objetivo (≥ 1).
    pub fn new(total_samples: usize, buckets: usize) -> Self {
        let buckets = buckets.max(1);
        let per_bucket = (total_samples / buckets).max(1);
        PeaksBuilder {
            per_bucket,
            cur_min: 0.0,
            cur_max: 0.0,
            n: 0,
            peaks: Vec::with_capacity(buckets + 1),
        }
    }

    pub fn push(&mut self, s: f32) {
        if self.n == 0 {
            self.cur_min = s;
            self.cur_max = s;
        } else {
            if s < self.cur_min {
                self.cur_min = s;
            }
            if s > self.cur_max {
                self.cur_max = s;
            }
        }
        self.n += 1;
        if self.n >= self.per_bucket {
            self.peaks.push((self.cur_min, self.cur_max));
            self.n = 0;
        }
    }

    pub fn push_slice(&mut self, samples: &[f32]) {
        for &s in samples {
            self.push(s);
        }
    }

    /// Cierra el bucket parcial pendiente (si lo hay) y devuelve la onda.
    pub fn finish(mut self) -> Waveform {
        if self.n > 0 {
            self.peaks.push((self.cur_min, self.cur_max));
        }
        Waveform { peaks: self.peaks }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_samples_min_max_por_bucket() {
        // 8 muestras en 2 buckets → 4 por bucket.
        let s = [0.1, -0.2, 0.5, -0.1, 0.0, 0.9, -0.9, 0.3];
        let w = Waveform::from_samples(&s, 2);
        assert_eq!(w.len(), 2);
        assert_eq!(w.peaks()[0], (-0.2, 0.5)); // primeras 4
        assert_eq!(w.peaks()[1], (-0.9, 0.9)); // últimas 4
    }

    #[test]
    fn bucket_parcial_se_cierra() {
        // 5 muestras, 2 buckets → per_bucket = 2 → buckets: [0,1],[2,3],[4]
        let s = [0.0, 1.0, -1.0, 0.5, 0.2];
        let w = Waveform::from_samples(&s, 2);
        assert_eq!(w.len(), 3);
        assert_eq!(w.peaks()[0], (0.0, 1.0));
        assert_eq!(w.peaks()[1], (-1.0, 0.5));
        assert_eq!(w.peaks()[2], (0.2, 0.2)); // resto
    }

    #[test]
    fn vacio_y_un_bucket() {
        assert!(Waveform::from_samples(&[], 10).is_empty());
        let w = Waveform::from_samples(&[0.3, -0.7, 0.5], 1);
        assert_eq!(w.len(), 1);
        assert_eq!(w.peaks()[0], (-0.7, 0.5));
    }

    #[test]
    fn streaming_igual_que_buffer() {
        let s: Vec<f32> = (0..1000).map(|i| ((i as f32) * 0.01).sin()).collect();
        let full = Waveform::from_samples(&s, 50);
        // Mismo resultado empujando por bloques irregulares.
        let mut b = PeaksBuilder::new(s.len(), 50);
        b.push_slice(&s[..333]);
        b.push_slice(&s[333..777]);
        b.push_slice(&s[777..]);
        let streamed = b.finish();
        assert_eq!(full, streamed);
    }

    #[test]
    fn buckets_cero_no_panickea() {
        let w = Waveform::from_samples(&[0.1, 0.2], 0);
        // buckets se satura a 1.
        assert_eq!(w.len(), 1);
    }
}
