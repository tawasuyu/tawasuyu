//! Detección de **palabra de llamada** (wake-word) — la compuerta F1.
//!
//! `VOZ.md` parte el manos-libres en dos cortes:
//!
//! - **F0** (ya hecho): el VAD segmenta utterances y el **STT transcribe todas**;
//!   recién el texto se compara con el llamado. Simple, pero transcribe (y, con
//!   STT de nube, **manda a la nube**) todo lo que hablás, sea o no para el
//!   asistente.
//! - **F1** (esto): un detector **barato y local** decide si la utterance suena
//!   al llamado *antes* de transcribir. Si no suena, el audio **nunca llega al
//!   STT**. Cierra el agujero de privacidad del «transcribe-todo».
//!
//! Misma doctrina que el resto del crate (*determinista primero, modelo opcional
//! después*): el contrato es [`DetectorLlamado`]; el default
//! [`DetectorPlantilla`] no usa modelo — compara la utterance contra una
//! **plantilla enrolada** (el usuario graba «shuma» unas veces) por
//! *dynamic time warping* sobre rasgos acústicos baratos (log-energía + tasa de
//! cruces por cero), sin FFT. Un wake-word neuronal entrenado
//! (`rimay-voz-openwakeword`, futuro) entra como otra impl del trait, sin tocar
//! el lazo.
//!
//! **Honestidad:** los tests certifican el *mecanismo* (una utterance idéntica a
//! la plantilla dispara; una bien distinta no), **no** la precisión real sobre
//! «shuma» — eso depende del enrolamiento y se afina en metal. El default es
//! *speaker-dependent* (precisa tu voz enrolada); el modo *speaker-independent*
//! es el backend neuronal posterior.

use crate::traits::Audio;

/// **Contrato wake-word** — ¿esta utterance es (plausiblemente) el llamado?
///
/// Corre sobre el fragmento que aisló el VAD, antes del STT. Barato y local: es
/// la compuerta que evita transcribir (y mandar a la nube) lo que no va dirigido
/// al asistente.
pub trait DetectorLlamado: Send + Sync {
    /// `true` si la utterance suena al llamado y conviene transcribirla.
    fn es_llamado(&self, audio: &Audio) -> bool;
}

/// Parámetros de extracción de rasgos (en muestras). Defaults pensados para
/// 16 kHz: ventana de 20 ms, salto de 10 ms. La plantilla y la utterance deben
/// usar los mismos (y la misma tasa de muestreo).
#[derive(Debug, Clone, Copy)]
pub struct ParamsLlamado {
    pub ventana: usize,
    pub salto: usize,
}

impl Default for ParamsLlamado {
    fn default() -> Self {
        Self { ventana: 320, salto: 160 }
    }
}

/// Secuencia de rasgos por ventana: `[log-energía centrada, tasa de cruces]`.
/// 2 dimensiones, sin FFT. La log-energía se centra (resta su media) para que
/// el match sea ~invariante al volumen.
pub fn rasgos(audio: &Audio, p: &ParamsLlamado) -> Vec<[f32; 2]> {
    let x = &audio.muestras;
    if p.ventana == 0 || p.salto == 0 || x.len() < p.ventana {
        return Vec::new();
    }
    let mut out: Vec<[f32; 2]> = Vec::new();
    let mut i = 0;
    while i + p.ventana <= x.len() {
        let v = &x[i..i + p.ventana];
        // log-energía (RMS normalizado al rango i16).
        let suma: f64 = v.iter().map(|&m| (m as f64).powi(2)).sum();
        let rms = (suma / p.ventana as f64).sqrt() / i16::MAX as f64;
        let log_e = ((rms + 1e-6).ln()) as f32;
        // tasa de cruces por cero (forma espectral grosera).
        let cruces = v.windows(2).filter(|w| (w[0] >= 0) != (w[1] >= 0)).count();
        let zcr = cruces as f32 / (p.ventana - 1) as f32;
        out.push([log_e, zcr]);
        i += p.salto;
    }
    // Centrar la log-energía → invariancia al volumen.
    let media: f32 = out.iter().map(|f| f[0]).sum::<f32>() / out.len() as f32;
    for f in &mut out {
        f[0] -= media;
    }
    out
}

/// Distancia DTW normalizada entre dos secuencias de rasgos. Menor = más
/// parecidas; `0` si son idénticas; `INFINITY` si alguna está vacía.
pub fn dtw(a: &[[f32; 2]], b: &[[f32; 2]]) -> f32 {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 {
        return f32::INFINITY;
    }
    let costo = |x: &[f32; 2], y: &[f32; 2]| -> f32 {
        let d0 = x[0] - y[0];
        let d1 = x[1] - y[1];
        (d0 * d0 + d1 * d1).sqrt()
    };
    // DP por filas (sólo guardamos la fila previa). dp[0][0]=0, bordes=∞.
    let mut prev = vec![f32::INFINITY; m + 1];
    prev[0] = 0.0;
    for i in 1..=n {
        let mut cur = vec![f32::INFINITY; m + 1];
        for j in 1..=m {
            let mejor = prev[j].min(cur[j - 1]).min(prev[j - 1]);
            cur[j] = costo(&a[i - 1], &b[j - 1]) + mejor;
        }
        prev = cur;
    }
    prev[m] / (n + m) as f32
}

/// Plantilla enrolada: los rasgos de **una** grabación del llamado.
#[derive(Debug, Clone)]
pub struct Plantilla {
    rasgos: Vec<[f32; 2]>,
}

impl Plantilla {
    /// Extrae la plantilla de una grabación del llamado.
    pub fn desde_audio(audio: &Audio, p: &ParamsLlamado) -> Self {
        Self { rasgos: rasgos(audio, p) }
    }

    pub fn vacia(&self) -> bool {
        self.rasgos.is_empty()
    }
}

/// Detector determinista por **plantilla + DTW** — el default sin modelo.
///
/// Se enrola con una o varias grabaciones del llamado (más grabaciones = más
/// robustez a la variación natural). Para cada utterance, alinea por DTW contra
/// cada plantilla y dispara si la mejor distancia cae bajo el umbral.
#[derive(Debug, Clone)]
pub struct DetectorPlantilla {
    plantillas: Vec<Plantilla>,
    umbral: f32,
    params: ParamsLlamado,
}

impl DetectorPlantilla {
    pub fn new(plantillas: Vec<Plantilla>, umbral: f32, params: ParamsLlamado) -> Self {
        Self { plantillas, umbral, params }
    }

    /// Enrola desde grabaciones crudas del llamado. Las que queden vacías (audio
    /// más corto que una ventana) se descartan.
    pub fn enrolar(audios: &[Audio], umbral: f32, params: ParamsLlamado) -> Self {
        let plantillas = audios
            .iter()
            .map(|a| Plantilla::desde_audio(a, &params))
            .filter(|p| !p.vacia())
            .collect();
        Self { plantillas, umbral, params }
    }

    /// Distancia DTW a la plantilla más parecida (`INFINITY` si no hay
    /// plantillas o la utterance es muy corta). Útil para calibrar el umbral.
    pub fn distancia(&self, audio: &Audio) -> f32 {
        let r = rasgos(audio, &self.params);
        if r.is_empty() {
            return f32::INFINITY;
        }
        self.plantillas
            .iter()
            .filter(|p| !p.vacia())
            .map(|p| dtw(&r, &p.rasgos))
            .fold(f32::INFINITY, f32::min)
    }
}

impl DetectorLlamado for DetectorPlantilla {
    fn es_llamado(&self, audio: &Audio) -> bool {
        self.distancia(audio) <= self.umbral
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audio de cruces altos (alterna signo): zcr ~1.0, voz-like.
    fn alternado(n: usize) -> Audio {
        let m = (0..n).map(|i| if i % 2 == 0 { 18_000 } else { -18_000 }).collect();
        Audio::new(m, 16_000)
    }
    /// Audio constante: zcr 0, claramente distinto del alternado.
    fn constante(n: usize) -> Audio {
        Audio::new(vec![6_000; n], 16_000)
    }

    #[test]
    fn rasgos_cuenta_ventanas_y_capta_zcr() {
        let p = ParamsLlamado { ventana: 4, salto: 2 };
        let r = rasgos(&alternado(10), &p);
        // (10-4)/2 + 1 = 4 ventanas.
        assert_eq!(r.len(), 4);
        // zcr del alternado ~1.0 (cambia de signo en cada par).
        assert!(r.iter().all(|f| f[1] > 0.9));
    }

    #[test]
    fn rasgos_audio_corto_es_vacio() {
        let p = ParamsLlamado { ventana: 100, salto: 50 };
        assert!(rasgos(&alternado(20), &p).is_empty());
    }

    #[test]
    fn dtw_de_secuencias_identicas_es_cero() {
        let p = ParamsLlamado::default();
        let r = rasgos(&alternado(2000), &p);
        assert_eq!(dtw(&r, &r), 0.0);
    }

    #[test]
    fn detector_dispara_con_lo_enrolado_y_no_con_lo_distinto() {
        let p = ParamsLlamado { ventana: 320, salto: 160 };
        // Enrolamos con el patrón alternado; umbral chico.
        let llamado = alternado(4000);
        let det = DetectorPlantilla::enrolar(&[llamado.clone()], 0.3, p);

        // La misma utterance → distancia 0 → dispara.
        assert_eq!(det.distancia(&llamado), 0.0);
        assert!(det.es_llamado(&llamado));

        // Una bien distinta (constante) → distancia alta → no dispara.
        let otra = constante(4000);
        assert!(det.distancia(&otra) > 0.3, "dist = {}", det.distancia(&otra));
        assert!(!det.es_llamado(&otra));
    }

    #[test]
    fn detector_sin_plantillas_nunca_dispara() {
        let det = DetectorPlantilla::new(Vec::new(), 1.0, ParamsLlamado::default());
        assert!(!det.es_llamado(&alternado(4000)));
    }

    #[test]
    fn utterance_demasiado_corta_no_dispara() {
        let p = ParamsLlamado { ventana: 320, salto: 160 };
        let det = DetectorPlantilla::enrolar(&[alternado(4000)], 1.0, p);
        // Un click de 100 muestras: más corto que la ventana → no dispara.
        assert!(!det.es_llamado(&alternado(100)));
    }
}
