//! dynamics — normalización (ganancia de makeup) + limitador de picos como
//! procesador de audio. A5 de `PARIDAD.md`: subir el volumen de un material
//! flojo sin que los picos saturen, y atajar los overshoots que mete el EQ.
//!
//! Calca el molde del ecualizador ([`crate::eq`]): un procesador puro
//! ([`Dynamics`]) y un wrapper de [`AudioSource`] ([`DynamicsAudio`])
//! gobernado por un [`DynamicsControl`] compartido, que compone en la cadena
//! del sink. Cero dependencias — sólo `f32`, corre en CI sin hardware.
//!
//! Cadena típica (después del EQ, último estadio de ganancia antes del tap
//! del visor):
//!
//! ```text
//! VolumeAudio → EqualizerAudio → DynamicsAudio → ProbedAudioSource → sink
//! ```
//!
//! ## Qué hace (y qué no, todavía)
//!
//! - **Ganancia de normalización**: un makeup en dB aplicado a cada sample
//!   (el "subí esto 6 dB" manual). La medición automática de loudness
//!   (ReplayGain / EBU R128) queda para una versión futura — hoy la
//!   ganancia la fija el usuario.
//! - **Limitador brick-wall**: tras la ganancia, clampea cada sample a un
//!   techo (`ceiling`, default 0.98) para que nunca pase de fondo de escala.
//!   Es duro (no soft-knee): predecible y barato; protege de clipping y de
//!   overshoots del EQ. Un compresor con knee/ataque/release sería otra
//!   fase.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::AudioSource;

/// Techo por defecto del limitador (≈ −0.18 dBFS): deja un pelo de headroom
/// para no tocar exactamente fondo de escala.
pub const DEFAULT_CEILING: f32 = 0.98;

/// Parámetros del procesador de dinámica.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicsParams {
    /// Ganancia de normalización en dB (makeup). `0.0` = sin cambio.
    pub gain_db: f32,
    /// Techo del limitador en lineal `0..1`. Los samples (ya con ganancia)
    /// se clampean a `[-ceiling, ceiling]`.
    pub ceiling: f32,
}

impl Default for DynamicsParams {
    fn default() -> Self {
        DynamicsParams {
            gain_db: 0.0,
            ceiling: DEFAULT_CEILING,
        }
    }
}

impl DynamicsParams {
    /// `true` si no altera la señal: sin ganancia y con el techo en (o por
    /// encima de) 1.0 — entonces el clamp nunca actúa sobre audio válido.
    pub fn is_identity(&self) -> bool {
        self.gain_db == 0.0 && self.ceiling >= 1.0
    }
}

/// dB → factor lineal de amplitud.
fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Procesador puro: aplica ganancia y limita picos in-place sobre un buffer
/// de samples `f32` intercalados. Sin estado entre bloques (cada sample es
/// independiente con un limitador brick-wall), así que es trivial de testear.
#[derive(Debug, Clone, Copy)]
pub struct Dynamics {
    params: DynamicsParams,
    gain_lin: f32,
}

impl Default for Dynamics {
    fn default() -> Self {
        Dynamics::new(DynamicsParams::default())
    }
}

impl Dynamics {
    pub fn new(params: DynamicsParams) -> Self {
        Dynamics {
            params,
            gain_lin: db_to_linear(params.gain_db),
        }
    }

    pub fn params(&self) -> DynamicsParams {
        self.params
    }

    pub fn set_params(&mut self, params: DynamicsParams) {
        self.params = params;
        self.gain_lin = db_to_linear(params.gain_db);
    }

    /// Procesa `buf` in-place. No-op si los parámetros son la identidad.
    pub fn process(&self, buf: &mut [f32]) {
        if self.params.is_identity() {
            return;
        }
        let ceiling = self.params.ceiling.max(0.0);
        let gain = self.gain_lin;
        for s in buf.iter_mut() {
            *s = (*s * gain).clamp(-ceiling, ceiling);
        }
    }
}

// ============================================================
// Control compartido (mismo patrón que EqControl / ColorControl)
// ============================================================

#[derive(Debug)]
struct DynamicsShared {
    params: DynamicsParams,
    enabled: bool,
}

/// Handle compartido y barato de clonar para gobernar un [`DynamicsAudio`]
/// en vivo. El wrapper compara un contador de versión atómico y sólo
/// resincroniza cuando algo cambió.
#[derive(Clone)]
pub struct DynamicsControl {
    shared: Arc<Mutex<DynamicsShared>>,
    version: Arc<AtomicU64>,
}

impl Default for DynamicsControl {
    fn default() -> Self {
        DynamicsControl::new(DynamicsParams::default())
    }
}

impl DynamicsControl {
    pub fn new(params: DynamicsParams) -> Self {
        DynamicsControl {
            shared: Arc::new(Mutex::new(DynamicsShared {
                params,
                enabled: true,
            })),
            version: Arc::new(AtomicU64::new(0)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DynamicsShared> {
        match self.shared.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    fn bump(&self) {
        self.version.fetch_add(1, Ordering::Release);
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    pub fn params(&self) -> DynamicsParams {
        self.lock().params
    }

    pub fn gain_db(&self) -> f32 {
        self.lock().params.gain_db
    }

    /// Suma `delta_db` a la ganancia de normalización (clampea a ±24 dB).
    pub fn add_gain_db(&self, delta_db: f32) {
        {
            let mut g = self.lock();
            g.params.gain_db = (g.params.gain_db + delta_db).clamp(-24.0, 24.0);
        }
        self.bump();
    }

    /// Fija la ganancia de normalización en un valor absoluto (clampea a
    /// ±24 dB). Lo usa la normalización automática (ReplayGain / EBU R128),
    /// que calcula la ganancia a aplicar de una sola vez.
    pub fn set_gain_db(&self, gain_db: f32) {
        {
            let mut g = self.lock();
            g.params.gain_db = gain_db.clamp(-24.0, 24.0);
        }
        self.bump();
    }

    /// Vuelve la ganancia a 0 dB (mantiene el techo).
    pub fn reset(&self) {
        {
            let mut g = self.lock();
            g.params.gain_db = 0.0;
        }
        self.bump();
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.lock().enabled = enabled;
        self.bump();
    }

    pub fn is_enabled(&self) -> bool {
        self.lock().enabled
    }
}

/// Wrapper de [`AudioSource`] que aplica un [`Dynamics`] gobernado por un
/// [`DynamicsControl`] compartido. Lee la versión atómica en cada bloque;
/// si cambió (o es la primera vez) resincroniza. Camino común lock-free;
/// con identidad o deshabilitado no recorre el buffer.
pub struct DynamicsAudio<S> {
    inner: S,
    control: DynamicsControl,
    dyn_proc: Dynamics,
    last_version: u64,
    enabled: bool,
    needs_init: bool,
}

impl<S> DynamicsAudio<S> {
    pub fn new(inner: S, control: DynamicsControl) -> Self {
        let dyn_proc = Dynamics::new(control.params());
        let enabled = control.is_enabled();
        DynamicsAudio {
            inner,
            control,
            dyn_proc,
            last_version: u64::MAX,
            enabled,
            needs_init: true,
        }
    }

    pub fn control(&self) -> DynamicsControl {
        self.control.clone()
    }

    fn sync(&mut self) {
        let v = self.control.version();
        if self.needs_init || v != self.last_version {
            self.dyn_proc.set_params(self.control.params());
            self.enabled = self.control.is_enabled();
            self.last_version = v;
            self.needs_init = false;
        }
    }
}

impl<S: AudioSource> AudioSource for DynamicsAudio<S> {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.inner.fill(buf, sample_rate, channels);
        self.sync();
        if self.enabled {
            self.dyn_proc.process(buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identidad_no_toca_el_buffer() {
        // gain 0 dB + ceiling >= 1.0 → identidad aunque haya samples > ceiling.
        let d = Dynamics::new(DynamicsParams {
            gain_db: 0.0,
            ceiling: 1.0,
        });
        assert!(d.params().is_identity());
        let mut buf = vec![0.5, -0.9, 1.5];
        d.process(&mut buf);
        assert_eq!(buf, vec![0.5, -0.9, 1.5]);
    }

    #[test]
    fn ganancia_amplifica() {
        // +6 dB ≈ ×2.
        let d = Dynamics::new(DynamicsParams {
            gain_db: 6.0,
            ceiling: 1.0,
        });
        let mut buf = vec![0.25, -0.25];
        d.process(&mut buf);
        assert!((buf[0] - 0.5).abs() < 0.01, "fue {}", buf[0]);
        assert!((buf[1] + 0.5).abs() < 0.01, "fue {}", buf[1]);
    }

    #[test]
    fn limitador_clampea_al_techo() {
        let d = Dynamics::new(DynamicsParams {
            gain_db: 0.0,
            ceiling: 0.8,
        });
        let mut buf = vec![0.9, -0.95, 0.5];
        d.process(&mut buf);
        assert_eq!(buf[0], 0.8);
        assert_eq!(buf[1], -0.8);
        assert_eq!(buf[2], 0.5); // por debajo del techo, intacto.
    }

    #[test]
    fn ganancia_mas_limitador_protege_de_clipping() {
        // +12 dB (×~4) llevaría 0.5 a ~2.0, pero el techo lo frena en 0.98.
        let d = Dynamics::new(DynamicsParams::default().tap_gain(12.0));
        let mut buf = vec![0.5, -0.5, 0.01];
        d.process(&mut buf);
        assert_eq!(buf[0], DEFAULT_CEILING);
        assert_eq!(buf[1], -DEFAULT_CEILING);
        // Un sample chiquito se amplifica sin tocar el techo.
        assert!(buf[2] > 0.03 && buf[2] < DEFAULT_CEILING);
    }

    #[test]
    fn control_clampea_y_resetea() {
        let c = DynamicsControl::default();
        c.add_gain_db(100.0);
        assert_eq!(c.gain_db(), 24.0);
        c.add_gain_db(-200.0);
        assert_eq!(c.gain_db(), -24.0);
        c.reset();
        assert_eq!(c.gain_db(), 0.0);
    }

    struct Const(f32);
    impl AudioSource for Const {
        fn fill(&mut self, buf: &mut [f32], _: u32, _: u16) {
            buf.fill(self.0);
        }
    }

    #[test]
    fn wrapper_bypass_y_aplica_en_vivo() {
        let ctrl = DynamicsControl::default(); // 0 dB, techo 0.98
        let mut a = DynamicsAudio::new(Const(0.5), ctrl.clone());
        let mut buf = vec![0.0; 4];
        // 0 dB: pasa igual (0.5 < techo).
        a.fill(&mut buf, 48_000, 2);
        assert!(buf.iter().all(|&s| (s - 0.5).abs() < 1e-6));
        // Subimos ganancia: el limitador frena en el techo.
        ctrl.add_gain_db(12.0);
        a.fill(&mut buf, 48_000, 2);
        assert!(buf.iter().all(|&s| (s - DEFAULT_CEILING).abs() < 1e-6));
        // Deshabilitado: bypass aunque haya ganancia.
        ctrl.set_enabled(false);
        a.fill(&mut buf, 48_000, 2);
        assert!(buf.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }

    // Helper de test para construir params con una ganancia dada.
    impl DynamicsParams {
        fn tap_gain(mut self, db: f32) -> Self {
            self.gain_db = db;
            self
        }
    }
}
