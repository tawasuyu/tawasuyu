//! eq — ecualizador paramétrico (banco de biquads) como procesador de
//! audio. Fase A1 de `PARIDAD.md`: lo que VLC trae de fábrica (EQ
//! gráfico de 10 bandas) y `media` no tenía.
//!
//! El diseño sigue el molde del resto de `media-core`: un procesador
//! puro y testeable ([`Equalizer`]) y un wrapper de [`AudioSource`]
//! ([`EqualizerAudio`]) que compone en la cadena del sink como
//! `PausableAudio`/`VolumeAudio`/`MixerAudio`. Cero dependencias —
//! sólo `f32` y trigonometría de `std`, así que corre en CI sin
//! hardware (igual que `Spectrum`/`Levels`).
//!
//! Los coeficientes salen del *audio EQ cookbook* de Robert
//! Bristow-Johnson (RBJ): filtros peaking y shelving de segundo orden.
//! El estado se procesa en Direct Form II Transposed (DF2T), una copia
//! por canal y por banda.
//!
//! Cadena típica (entre el volumen y el probe del visor):
//!
//! ```text
//! VolumeAudio → EqualizerAudio → ProbedAudioSource → sink
//! ```
//!
//! El visor (que toma del probe) refleja el audio **ya ecualizado**.

use std::f32::consts::PI;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::AudioSource;

/// Frecuencias centro ISO del EQ gráfico clásico de 10 bandas (las que
/// usa VLC). Una banda por octava aproximada de 31 Hz a 16 kHz.
pub const ISO_10_BANDS_HZ: [f32; 10] = [
    31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// `Q` por defecto de cada banda peaking del EQ gráfico. ~1.41 ≈ ancho
/// de una octava, que es lo que corresponde a bandas espaciadas por
/// octava sin solaparse de más.
pub const DEFAULT_BAND_Q: f32 = 1.41;

/// Tipo de filtro de una banda. El EQ gráfico usa [`BandKind::Peaking`]
/// en todas; los shelving sirven para tonos graves/agudos globales.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BandKind {
    /// Campana centrada en `freq` con ancho dado por `q`.
    Peaking,
    /// Estante de graves: afecta todo lo que está por debajo de `freq`.
    LowShelf,
    /// Estante de agudos: afecta todo lo que está por encima de `freq`.
    HighShelf,
}

/// Una banda del ecualizador: dónde actúa, cuánto y qué tan ancha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqBand {
    /// Frecuencia centro (peaking) o de corte (shelf) en Hz.
    pub freq: f32,
    /// Ganancia en decibeles. 0.0 = transparente; positivo realza,
    /// negativo atenúa.
    pub gain_db: f32,
    /// Factor de calidad. Más alto = banda más angosta.
    pub q: f32,
    pub kind: BandKind,
}

impl EqBand {
    /// Banda peaking con la `Q` por defecto.
    pub fn peaking(freq: f32, gain_db: f32) -> Self {
        EqBand {
            freq,
            gain_db,
            q: DEFAULT_BAND_Q,
            kind: BandKind::Peaking,
        }
    }
}

/// Coeficientes normalizados (a0 = 1) de un filtro biquad de segundo
/// orden. `Copy` barato: recalcularlos en cada cambio de banda o de
/// sample rate es despreciable comparado con procesar el bloque.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    /// Identidad: deja la señal igual. Es el biquad de una banda a 0 dB.
    pub fn identity() -> Self {
        Biquad {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }

    /// Construye el biquad de una banda para un sample rate dado, según
    /// las fórmulas RBJ. Frecuencias fuera del rango útil (≥ Nyquist o
    /// ≤ 0) o sample rate inválido caen a la identidad — nunca producen
    /// coeficientes inestables.
    pub fn from_band(band: &EqBand, sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        if sr <= 0.0 || band.freq <= 0.0 || band.freq >= sr * 0.5 {
            return Biquad::identity();
        }
        let a = 10f32.powf(band.gain_db / 40.0); // amplitud lineal sqrt de la potencia
        let w0 = 2.0 * PI * band.freq / sr;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let q = band.q.max(1e-4);
        let alpha = sin_w0 / (2.0 * q);

        let (b0, b1, b2, a0, a1, a2) = match band.kind {
            BandKind::Peaking => (
                1.0 + alpha * a,
                -2.0 * cos_w0,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos_w0,
                1.0 - alpha / a,
            ),
            BandKind::LowShelf => {
                let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0),
                    a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha),
                    (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
                    (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha,
                )
            }
            BandKind::HighShelf => {
                let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0),
                    a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha),
                    (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
                    (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha,
                )
            }
        };

        if a0.abs() < 1e-12 {
            return Biquad::identity();
        }
        Biquad {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// Procesa una muestra avanzando el estado (DF2T). El `state` es por
    /// canal: una copia distinta por cada canal del stream.
    #[inline]
    pub fn process(&self, x: f32, state: &mut BiquadState) -> f32 {
        let y = self.b0 * x + state.z1;
        state.z1 = self.b1 * x - self.a1 * y + state.z2;
        state.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Estado de retardo de un biquad para un canal. `Default` = silencio
/// (arranca limpio).
#[derive(Debug, Clone, Copy, Default)]
pub struct BiquadState {
    z1: f32,
    z2: f32,
}

/// Banco de biquads en cascada: una banda tras otra sobre cada canal.
/// Compila los coeficientes para el sample rate vigente y mantiene un
/// estado por (canal × banda). Es el procesador puro — testeable sin
/// sink ni UI.
#[derive(Debug, Clone)]
pub struct Equalizer {
    bands: Vec<EqBand>,
    /// Sample rate con el que se compilaron `filters`. 0 = sin compilar.
    sample_rate: u32,
    /// Canales del último bloque. Define el tamaño de `states`.
    channels: u16,
    filters: Vec<Biquad>,
    /// Estado plano `channels × bands` — `states[ch * bands + band]`.
    states: Vec<BiquadState>,
}

impl Equalizer {
    /// EQ a partir de una lista de bandas (sin compilar todavía: la
    /// primera llamada a [`Equalizer::process_block`] compila para el
    /// sample rate real).
    pub fn new(bands: Vec<EqBand>) -> Self {
        Equalizer {
            bands,
            sample_rate: 0,
            channels: 0,
            filters: Vec::new(),
            states: Vec::new(),
        }
    }

    /// EQ gráfico de 10 bandas ISO (estilo VLC), todas peaking, con las
    /// ganancias dadas en dB (una por banda). Ganancias de más se
    /// ignoran; de menos se completan con 0 dB.
    pub fn graphic_10band(gains_db: &[f32]) -> Self {
        let bands = ISO_10_BANDS_HZ
            .iter()
            .enumerate()
            .map(|(i, &f)| EqBand::peaking(f, gains_db.get(i).copied().unwrap_or(0.0)))
            .collect();
        Equalizer::new(bands)
    }

    /// EQ gráfico de 10 bandas plano (0 dB en todas) — transparente
    /// hasta que se ajuste alguna banda.
    pub fn flat_10band() -> Self {
        Equalizer::graphic_10band(&[0.0; 10])
    }

    pub fn bands(&self) -> &[EqBand] {
        &self.bands
    }

    pub fn band_count(&self) -> usize {
        self.bands.len()
    }

    /// Reemplaza las bandas. Marca el banco como no compilado para que el
    /// próximo bloque recompile; conserva el estado de retardo (cambiar
    /// ganancia en vivo no debe meter un click).
    pub fn set_bands(&mut self, bands: Vec<EqBand>) {
        self.bands = bands;
        self.sample_rate = 0;
    }

    /// Fija la ganancia (dB) de una banda por índice. No-op fuera de rango.
    pub fn set_gain(&mut self, idx: usize, gain_db: f32) {
        if let Some(b) = self.bands.get_mut(idx) {
            b.gain_db = gain_db;
            self.sample_rate = 0;
        }
    }

    fn recompile(&mut self, sample_rate: u32) {
        self.filters = self
            .bands
            .iter()
            .map(|b| Biquad::from_band(b, sample_rate))
            .collect();
        self.sample_rate = sample_rate;
    }

    fn ensure_states(&mut self, channels: u16) {
        let needed = channels as usize * self.bands.len();
        if self.channels != channels || self.states.len() != needed {
            self.states = vec![BiquadState::default(); needed];
            self.channels = channels;
        }
    }

    /// Procesa un bloque intercalado in-place. Recompila si cambió el
    /// sample rate (o tras `set_gain`/`set_bands`) y redimensiona el
    /// estado si cambió la cuenta de canales. Con cero bandas es no-op.
    pub fn process_block(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        if self.bands.is_empty() || channels == 0 {
            return;
        }
        if self.sample_rate != sample_rate {
            self.recompile(sample_rate);
        }
        self.ensure_states(channels);

        let nb = self.filters.len();
        let ch = channels as usize;
        let frames = buf.len() / ch;
        for f in 0..frames {
            for c in 0..ch {
                let i = f * ch + c;
                let mut x = buf[i];
                let base = c * nb;
                for b in 0..nb {
                    x = self.filters[b].process(x, &mut self.states[base + b]);
                }
                buf[i] = x;
            }
        }
    }
}

// ============================================================
// EqControl — handle compartido para cambios en vivo
// ============================================================

/// Estado del EQ detrás de un lock, compartido entre la UI (que ajusta
/// bandas) y el wrapper de audio (que las lee).
#[derive(Debug, Clone)]
struct EqShared {
    bands: Vec<EqBand>,
    enabled: bool,
}

/// Handle clonable para mirar/ajustar el EQ desde otro hilo (la UI)
/// mientras [`EqualizerAudio`] procesa en el callback realtime. Es
/// `Clone` barato (sólo `Arc`s). El callback de audio NO toma el lock
/// en el camino común: compara un contador de versión atómico y sólo
/// re-sincroniza (lock + recompilar) cuando algo cambió.
#[derive(Clone)]
pub struct EqControl {
    shared: Arc<Mutex<EqShared>>,
    /// Se incrementa con cada cambio. El wrapper lo compara contra su
    /// última versión vista para decidir si resincroniza.
    version: Arc<AtomicU64>,
}

impl EqControl {
    /// Crea un control sobre estas bandas, habilitado.
    pub fn new(bands: Vec<EqBand>) -> Self {
        EqControl {
            shared: Arc::new(Mutex::new(EqShared {
                bands,
                enabled: true,
            })),
            version: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Control de un EQ gráfico de 10 bandas plano (estilo VLC).
    pub fn graphic_10band() -> Self {
        EqControl::new(
            ISO_10_BANDS_HZ
                .iter()
                .map(|&f| EqBand::peaking(f, 0.0))
                .collect(),
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, EqShared> {
        match self.shared.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    fn bump(&self) {
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Versión actual — el wrapper la compara para saber si recargar.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// Ajusta la ganancia (dB) de una banda por índice.
    pub fn set_gain(&self, idx: usize, gain_db: f32) {
        {
            let mut g = self.lock();
            if let Some(b) = g.bands.get_mut(idx) {
                b.gain_db = gain_db;
            } else {
                return;
            }
        }
        self.bump();
    }

    /// Reemplaza todas las ganancias de golpe (una por banda).
    pub fn set_all_gains(&self, gains_db: &[f32]) {
        {
            let mut g = self.lock();
            for (b, &db) in g.bands.iter_mut().zip(gains_db.iter()) {
                b.gain_db = db;
            }
        }
        self.bump();
    }

    /// Enciende/apaga el EQ. Apagado, el wrapper deja pasar la señal sin
    /// tocarla (bypass real, sin costo de procesado).
    pub fn set_enabled(&self, enabled: bool) {
        self.lock().enabled = enabled;
        self.bump();
    }

    pub fn is_enabled(&self) -> bool {
        self.lock().enabled
    }

    /// Snapshot de las bandas actuales (para pintar la UI).
    pub fn bands(&self) -> Vec<EqBand> {
        self.lock().bands.clone()
    }

    /// Ganancias actuales en dB (una por banda).
    pub fn gains(&self) -> Vec<f32> {
        self.lock().bands.iter().map(|b| b.gain_db).collect()
    }
}

/// Wrapper de [`AudioSource`] que aplica un [`Equalizer`] gobernado por
/// un [`EqControl`] compartido. Lee la versión atómica del control en
/// cada bloque; si cambió (o es la primera vez), resincroniza las bandas
/// y el on/off. El camino común (sin cambios) es lock-free.
pub struct EqualizerAudio<S> {
    inner: S,
    control: EqControl,
    eq: Equalizer,
    last_version: u64,
    enabled: bool,
    /// `true` hasta procesar el primer bloque — fuerza la primera
    /// sincronización aunque la versión arranque en 0.
    needs_init: bool,
}

impl<S> EqualizerAudio<S> {
    /// Envuelve `inner` con el EQ que gobierna `control`.
    pub fn new(inner: S, control: EqControl) -> Self {
        let eq = Equalizer::new(control.bands());
        let enabled = control.is_enabled();
        EqualizerAudio {
            inner,
            control,
            eq,
            last_version: control_version_placeholder(),
            enabled,
            needs_init: true,
        }
    }

    /// Devuelve un clon del handle de control (para que la UI lo ajuste).
    pub fn control(&self) -> EqControl {
        self.control.clone()
    }

    fn sync(&mut self) {
        let v = self.control.version();
        if self.needs_init || v != self.last_version {
            self.eq.set_bands(self.control.bands());
            self.enabled = self.control.is_enabled();
            self.last_version = v;
            self.needs_init = false;
        }
    }
}

/// Valor inicial de `last_version` distinto de cualquier versión real
/// probable; combinado con `needs_init` garantiza la primera sync.
fn control_version_placeholder() -> u64 {
    u64::MAX
}

impl<S: AudioSource> AudioSource for EqualizerAudio<S> {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.inner.fill(buf, sample_rate, channels);
        self.sync();
        if self.enabled {
            self.eq.process_block(buf, sample_rate, channels);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    /// Genera una senoide mono de `frames` muestras a `freq` Hz, amp 0.5.
    fn sine(freq: f32, frames: usize) -> Vec<f32> {
        let dphi = 2.0 * PI * freq / SR as f32;
        let mut phi = 0.0_f32;
        (0..frames)
            .map(|_| {
                let v = phi.sin() * 0.5;
                phi += dphi;
                v
            })
            .collect()
    }

    /// RMS de la segunda mitad del buffer (descarta el transitorio del
    /// filtro para medir el estado estacionario).
    fn rms_steady(buf: &[f32]) -> f32 {
        let start = buf.len() / 2;
        let tail = &buf[start..];
        let sq: f32 = tail.iter().map(|s| s * s).sum();
        (sq / tail.len() as f32).sqrt()
    }

    #[test]
    fn banda_plana_es_identidad() {
        // 0 dB en todas → la cascada es transparente bit-a-bit (la
        // fórmula RBJ peaking a A=1 da b == a).
        let mut eq = Equalizer::flat_10band();
        let mut buf = sine(1000.0, 4096);
        let original = buf.clone();
        eq.process_block(&mut buf, SR, 1);
        for (o, p) in original.iter().zip(buf.iter()) {
            assert!((o - p).abs() < 1e-5, "esperaba identidad: {o} vs {p}");
        }
    }

    #[test]
    fn realce_sube_energia_en_la_banda() {
        // +12 dB peaking a 1 kHz → una senoide a 1 kHz sale más fuerte.
        let mut eq = Equalizer::new(vec![EqBand::peaking(1000.0, 12.0)]);
        let mut buf = sine(1000.0, 8192);
        let antes = rms_steady(&buf);
        eq.process_block(&mut buf, SR, 1);
        let despues = rms_steady(&buf);
        assert!(
            despues > antes * 1.5,
            "realce esperado: antes={antes}, despues={despues}"
        );
    }

    #[test]
    fn atenuacion_baja_energia_en_la_banda() {
        let mut eq = Equalizer::new(vec![EqBand::peaking(1000.0, -12.0)]);
        let mut buf = sine(1000.0, 8192);
        let antes = rms_steady(&buf);
        eq.process_block(&mut buf, SR, 1);
        let despues = rms_steady(&buf);
        assert!(
            despues < antes * 0.6,
            "atenuación esperada: antes={antes}, despues={despues}"
        );
    }

    #[test]
    fn realce_no_toca_frecuencia_lejana() {
        // Realce angosto a 1 kHz no debería mover una senoide a 60 Hz.
        let mut eq = Equalizer::new(vec![EqBand {
            freq: 1000.0,
            gain_db: 12.0,
            q: 4.0,
            kind: BandKind::Peaking,
        }]);
        let mut buf = sine(60.0, 8192);
        let antes = rms_steady(&buf);
        eq.process_block(&mut buf, SR, 1);
        let despues = rms_steady(&buf);
        assert!(
            (despues - antes).abs() < antes * 0.1,
            "60 Hz no debería cambiar: antes={antes}, despues={despues}"
        );
    }

    #[test]
    fn estereo_procesa_canales_independientes() {
        // Dos canales con la misma señal deben salir idénticos (estado
        // por canal, sin cross-talk).
        let mut eq = Equalizer::new(vec![EqBand::peaking(1000.0, 9.0)]);
        let mono = sine(1000.0, 2048);
        let mut inter: Vec<f32> = mono.iter().flat_map(|&s| [s, s]).collect();
        eq.process_block(&mut inter, SR, 2);
        for pair in inter.chunks_exact(2) {
            assert!((pair[0] - pair[1]).abs() < 1e-6, "canales divergieron");
        }
    }

    #[test]
    fn graphic_10band_tiene_diez_bandas_iso() {
        let eq = Equalizer::graphic_10band(&[3.0, -3.0]);
        assert_eq!(eq.band_count(), 10);
        assert_eq!(eq.bands()[0].freq, 31.25);
        assert_eq!(eq.bands()[9].freq, 16000.0);
        // Las dos primeras ganancias se aplicaron; el resto quedó en 0.
        assert_eq!(eq.bands()[0].gain_db, 3.0);
        assert_eq!(eq.bands()[1].gain_db, -3.0);
        assert_eq!(eq.bands()[2].gain_db, 0.0);
    }

    #[test]
    fn biquad_frecuencia_sobre_nyquist_cae_a_identidad() {
        // Una banda a 30 kHz con SR 48k (Nyquist 24k) no puede existir:
        // debe dar identidad, no coeficientes inestables.
        let bq = Biquad::from_band(&EqBand::peaking(30_000.0, 12.0), SR);
        let mut st = BiquadState::default();
        for x in [0.3, -0.7, 0.5, 0.1] {
            assert!((bq.process(x, &mut st) - x).abs() < 1e-6);
        }
    }

    // ---------- EqControl + wrapper ----------

    struct Sine1k {
        phi: f32,
    }
    impl AudioSource for Sine1k {
        fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
            let dphi = 2.0 * PI * 1000.0 / sample_rate as f32;
            let ch = channels.max(1) as usize;
            let frames = buf.len() / ch;
            for f in 0..frames {
                let v = self.phi.sin() * 0.5;
                for c in 0..ch {
                    buf[f * ch + c] = v;
                }
                self.phi += dphi;
            }
        }
    }

    #[test]
    fn control_version_sube_con_cambios() {
        let ctl = EqControl::graphic_10band();
        let v0 = ctl.version();
        ctl.set_gain(5, 6.0); // banda de 1 kHz
        assert!(ctl.version() > v0);
        assert_eq!(ctl.gains()[5], 6.0);
        // set_gain fuera de rango no bumpea.
        let v1 = ctl.version();
        ctl.set_gain(99, 6.0);
        assert_eq!(ctl.version(), v1);
    }

    #[test]
    fn wrapper_aplica_cambios_en_vivo() {
        let ctl = EqControl::graphic_10band();
        let mut src = EqualizerAudio::new(Sine1k { phi: 0.0 }, ctl.clone());

        // Plano: el primer bloque sale prácticamente igual a la fuente.
        let mut buf = vec![0.0_f32; 8192];
        src.fill(&mut buf, SR, 1);
        let plano = rms_steady(&buf);

        // Realzamos la banda de 1 kHz (índice 5) en vivo y procesamos otro
        // bloque: ahora sale más fuerte.
        ctl.set_gain(5, 12.0);
        let mut buf2 = vec![0.0_f32; 8192];
        src.fill(&mut buf2, SR, 1);
        let realzado = rms_steady(&buf2);

        assert!(
            realzado > plano * 1.3,
            "el realce en vivo no se aplicó: plano={plano}, realzado={realzado}"
        );
    }

    #[test]
    fn wrapper_deshabilitado_es_bypass() {
        let ctl = EqControl::graphic_10band();
        ctl.set_gain(5, 12.0);
        ctl.set_enabled(false);
        let mut src = EqualizerAudio::new(Sine1k { phi: 0.0 }, ctl.clone());

        let mut buf = vec![0.0_f32; 8192];
        src.fill(&mut buf, SR, 1);
        let con_bypass = rms_steady(&buf);

        // Referencia: la misma fuente sin EQ.
        let mut raw = Sine1k { phi: 0.0 };
        let mut rbuf = vec![0.0_f32; 8192];
        raw.fill(&mut rbuf, SR, 1);
        let referencia = rms_steady(&rbuf);

        assert!(
            (con_bypass - referencia).abs() < referencia * 0.02,
            "bypass debería igualar la fuente: bypass={con_bypass}, ref={referencia}"
        );
    }
}
