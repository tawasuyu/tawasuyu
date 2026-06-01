//! sync — política de sincronización audio/video (el cerebro de M1 en
//! `PARIDAD.md`). El reloj **maestro es el audio** (como VLC/mpv): el
//! video se acomoda a él. Por cada frame de video con un PTS conocido,
//! [`AvSync::plan`] decide si **presentarlo**, **retenerlo** (todavía no
//! es su momento) o **descartarlo** (llegó tarde y hay que alcanzar el
//! reloj).
//!
//! Esto es la lógica pura — no decodea, no duerme, no toca hardware:
//! recibe dos [`Duration`] (posición del audio + PTS del frame) y
//! devuelve un plan. Por eso corre en CI sin sonido ni GPU, igual que
//! [`crate::eq`]. El wiring (extraer PTS de `foreign-av` y darle la
//! posición del audio vía [`crate::Seekable`] en el bucle de
//! `media-app`) es el sub-paso siguiente de M1.
//!
//! El problema que resuelve: hoy el video de `media-app` avanza con un
//! timer fijo (~30 fps) independiente del framerate real y del reloj de
//! audio, así que cualquier fuente que no sea exactamente 30 fps deriva.
//! Con esta política, el frame se ata al audio y la deriva desaparece.

use std::time::Duration;

/// Qué hacer con un frame de video respecto del reloj de audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePlan {
    /// Mostrarlo ahora: su PTS cae dentro de la ventana de presentación
    /// alrededor del reloj de audio.
    Present,
    /// Todavía no es su momento (el frame se adelanta al audio). Esperar
    /// `wait` y reevaluar — el caller duerme ~`wait` y vuelve a planear.
    Hold { wait: Duration },
    /// Llegó tarde respecto del audio: descartarlo (y pedir el próximo)
    /// para que el video alcance al reloj. Es el "framedrop" de los players.
    Drop,
}

/// Ventana de tolerancia alrededor del reloj de audio. Un frame es
/// presentable si su PTS está en `[audio - present_behind, audio +
/// present_ahead]`. Más allá por arriba → [`FramePlan::Hold`]; más allá
/// por abajo → [`FramePlan::Drop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncConfig {
    /// Cuánto puede **adelantarse** un frame al audio y aún presentarse
    /// (evita micro-holds por unos ms). Por encima se retiene.
    pub present_ahead: Duration,
    /// Cuánto puede **atrasarse** un frame respecto del audio y aún
    /// presentarse. Por debajo se descarta para alcanzar el reloj.
    pub present_behind: Duration,
}

impl Default for SyncConfig {
    fn default() -> Self {
        // ±: adelanto chico (presentar pronto, sin busy-hold), atraso
        // tolerante hasta ~50 ms (≈ un frame y medio a 30 fps) antes de
        // empezar a tirar frames. Perceptualmente el desincronía A/V se
        // nota a partir de ~40-50 ms, así que es un punto razonable.
        SyncConfig {
            present_ahead: Duration::from_millis(5),
            present_behind: Duration::from_millis(50),
        }
    }
}

/// Decisión pura para un frame: compara el PTS del frame con la posición
/// del audio y devuelve el plan según `cfg`. No tiene estado — toda la
/// lógica de M1 vive acá; [`AvSync`] sólo le suma contadores.
///
/// La resta se hace en nanosegundos con signo (`i128`) porque [`Duration`]
/// no representa valores negativos: un frame puede estar antes o después
/// del reloj.
pub fn plan_frame(audio: Duration, frame_pts: Duration, cfg: &SyncConfig) -> FramePlan {
    plan_frame_offset(audio, frame_pts, 0, cfg)
}

/// Como [`plan_frame`] pero con un **desfase A/V firmado** en nanosegundos
/// (`offset_ns`) — el ajuste de lipsync (A4 de `PARIDAD.md`, el
/// `--audio-delay` de mpv/VLC). `offset_ns` **positivo retrasa el video**
/// respecto del audio (equivale a adelantar el audio); negativo lo
/// adelanta. No toca el stream de audio: sólo corre la ventana de
/// presentación, así que vale para ambas direcciones y es reversible.
///
/// El audio efectivo es `audio - offset`: con offset positivo el frame
/// queda "más adelantado" → se retiene más → el video se atrasa.
pub fn plan_frame_offset(
    audio: Duration,
    frame_pts: Duration,
    offset_ns: i64,
    cfg: &SyncConfig,
) -> FramePlan {
    let audio_ns = audio.as_nanos() as i128;
    let pts_ns = frame_pts.as_nanos() as i128;
    let ahead_ns = cfg.present_ahead.as_nanos() as i128;
    let behind_ns = cfg.present_behind.as_nanos() as i128;

    // diff > 0: el frame va por DELANTE del audio (es futuro).
    // diff < 0: el frame va por DETRÁS del audio (es pasado, tarde).
    // El offset corre la referencia: diff = pts - (audio - offset).
    let diff = pts_ns - audio_ns + offset_ns as i128;

    if diff > ahead_ns {
        // Demasiado adelantado: esperar hasta que entre a la ventana
        // (cuando el audio avance hasta `pts - present_ahead`).
        let wait = (diff - ahead_ns) as u64;
        FramePlan::Hold {
            wait: Duration::from_nanos(wait),
        }
    } else if diff < -behind_ns {
        // Demasiado atrasado: descartar para alcanzar el reloj.
        FramePlan::Drop
    } else {
        FramePlan::Present
    }
}

/// Sincronizador con la política de [`plan_frame`] más contadores de
/// diagnóstico (frames presentados / descartados / retenidos). El caller
/// lo consulta por frame; los contadores sirven para mostrar "N frames
/// dropped" en la UI o para tests. Es barato de clonar la config; el
/// estado es sólo tres `u64`.
#[derive(Debug, Clone)]
pub struct AvSync {
    cfg: SyncConfig,
    /// Desfase A/V firmado en nanos (A4). Positivo retrasa el video.
    offset_ns: i64,
    presented: u64,
    dropped: u64,
    held: u64,
}

/// Tope del desfase A/V manual (±5 s). Más allá no es lipsync sino otra
/// cosa; clampeamos para que un atajo repetido no mande el video a otro
/// planeta.
pub const MAX_OFFSET_MS: i64 = 5_000;

impl Default for AvSync {
    fn default() -> Self {
        AvSync::new(SyncConfig::default())
    }
}

impl AvSync {
    pub fn new(cfg: SyncConfig) -> Self {
        AvSync {
            cfg,
            offset_ns: 0,
            presented: 0,
            dropped: 0,
            held: 0,
        }
    }

    pub fn config(&self) -> &SyncConfig {
        &self.cfg
    }

    /// Cambia la ventana de tolerancia en vivo (p. ej. desde un control
    /// de UI). No toca los contadores.
    pub fn set_config(&mut self, cfg: SyncConfig) {
        self.cfg = cfg;
    }

    /// Desfase A/V manual actual en milisegundos (positivo = video atrasado).
    pub fn offset_ms(&self) -> i64 {
        self.offset_ns / 1_000_000
    }

    /// Fija el desfase A/V manual en ms, clampeado a ±[`MAX_OFFSET_MS`].
    pub fn set_offset_ms(&mut self, ms: i64) {
        let ms = ms.clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS);
        self.offset_ns = ms * 1_000_000;
    }

    /// Suma `delta_ms` al desfase A/V (negativo adelanta el video). Atajo
    /// para los comandos `AvSyncBy` que ciclan el lipsync con una tecla.
    pub fn add_offset_ms(&mut self, delta_ms: i64) {
        self.set_offset_ms(self.offset_ms() + delta_ms);
    }

    /// Planea un frame y actualiza los contadores. `Hold` NO cuenta como
    /// presentado ni descartado (el frame sigue pendiente); se cuenta
    /// aparte para diagnóstico. Aplica el desfase A/V manual ([`offset_ms`]).
    pub fn plan(&mut self, audio: Duration, frame_pts: Duration) -> FramePlan {
        let plan = plan_frame_offset(audio, frame_pts, self.offset_ns, &self.cfg);
        match plan {
            FramePlan::Present => self.presented += 1,
            FramePlan::Drop => self.dropped += 1,
            FramePlan::Hold { .. } => self.held += 1,
        }
        plan
    }

    pub fn presented(&self) -> u64 {
        self.presented
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn held(&self) -> u64 {
        self.held
    }

    /// Reinicia los contadores. Llamar tras un seek (los frames viejos no
    /// deben contar contra el nuevo punto de reproducción).
    pub fn reset(&mut self) {
        self.presented = 0;
        self.dropped = 0;
        self.held = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn frame_en_hora_se_presenta() {
        let cfg = SyncConfig::default();
        // PTS == audio → en el centro de la ventana.
        assert_eq!(plan_frame(ms(1000), ms(1000), &cfg), FramePlan::Present);
        // Levemente atrasado pero dentro de present_behind (50 ms).
        assert_eq!(plan_frame(ms(1000), ms(970), &cfg), FramePlan::Present);
        // Levemente adelantado pero dentro de present_ahead (5 ms).
        assert_eq!(plan_frame(ms(1000), ms(1004), &cfg), FramePlan::Present);
    }

    #[test]
    fn frame_adelantado_se_retiene_con_espera() {
        let cfg = SyncConfig::default();
        // 100 ms por delante del audio → Hold; espera ≈ 100 - 5 = 95 ms.
        match plan_frame(ms(1000), ms(1100), &cfg) {
            FramePlan::Hold { wait } => assert_eq!(wait, ms(95)),
            other => panic!("esperaba Hold, fue {other:?}"),
        }
    }

    #[test]
    fn frame_atrasado_se_descarta() {
        let cfg = SyncConfig::default();
        // 200 ms por detrás del audio → muy tarde → Drop.
        assert_eq!(plan_frame(ms(1000), ms(800), &cfg), FramePlan::Drop);
    }

    #[test]
    fn bordes_exactos_de_la_ventana() {
        let cfg = SyncConfig {
            present_ahead: ms(5),
            present_behind: ms(50),
        };
        // Exactamente en el borde de adelanto (diff == present_ahead) →
        // todavía Present (la condición de Hold es estricta `>`).
        assert_eq!(plan_frame(ms(1000), ms(1005), &cfg), FramePlan::Present);
        // Un ns más allá del borde de adelanto → Hold.
        let just_over = Duration::from_nanos(ms(1005).as_nanos() as u64 + 1);
        assert!(matches!(
            plan_frame(ms(1000), just_over, &cfg),
            FramePlan::Hold { .. }
        ));
        // Exactamente en el borde de atraso (diff == -present_behind) →
        // todavía Present (la condición de Drop es estricta `<`).
        assert_eq!(plan_frame(ms(1000), ms(950), &cfg), FramePlan::Present);
        // Un ns más allá del borde de atraso → Drop.
        let just_under = Duration::from_nanos(ms(950).as_nanos() as u64 - 1);
        assert_eq!(plan_frame(ms(1000), just_under, &cfg), FramePlan::Drop);
    }

    #[test]
    fn avsync_cuenta_cada_plan() {
        let mut sync = AvSync::default();
        sync.plan(ms(1000), ms(1000)); // Present
        sync.plan(ms(1000), ms(1004)); // Present
        sync.plan(ms(1000), ms(800)); // Drop
        sync.plan(ms(1000), ms(2000)); // Hold
        assert_eq!(sync.presented(), 2);
        assert_eq!(sync.dropped(), 1);
        assert_eq!(sync.held(), 1);
    }

    #[test]
    fn reset_pone_contadores_en_cero() {
        let mut sync = AvSync::default();
        sync.plan(ms(0), ms(0));
        sync.plan(ms(1000), ms(0)); // Drop
        assert!(sync.presented() > 0 || sync.dropped() > 0);
        sync.reset();
        assert_eq!((sync.presented(), sync.dropped(), sync.held()), (0, 0, 0));
    }

    #[test]
    fn offset_positivo_retrasa_el_video() {
        let cfg = SyncConfig::default();
        // Un frame en hora con el audio se presenta sin offset.
        assert_eq!(
            plan_frame_offset(ms(1000), ms(1000), 0, &cfg),
            FramePlan::Present
        );
        // Offset +100 ms (retrasar video): ese mismo frame queda "por
        // delante" del audio efectivo → se retiene, así el video se atrasa.
        let off = 100 * 1_000_000;
        assert!(matches!(
            plan_frame_offset(ms(1000), ms(1000), off, &cfg),
            FramePlan::Hold { .. }
        ));
    }

    #[test]
    fn offset_negativo_adelanta_el_video() {
        let cfg = SyncConfig::default();
        // Un frame en hora con el audio. Offset −100 ms (adelantar video):
        // el frame pasa a quedar 100 ms atrasado respecto del audio
        // efectivo → se descarta para que el video corra adelante.
        let off = -100 * 1_000_000;
        assert_eq!(
            plan_frame_offset(ms(1000), ms(1000), off, &cfg),
            FramePlan::Drop
        );
    }

    #[test]
    fn avsync_aplica_y_clampea_el_offset() {
        let mut sync = AvSync::default();
        assert_eq!(sync.offset_ms(), 0);
        sync.add_offset_ms(100);
        assert_eq!(sync.offset_ms(), 100);
        // Con +100 ms (retrasar video), un frame en hora pasa a retenerse.
        assert!(matches!(sync.plan(ms(1000), ms(1000)), FramePlan::Hold { .. }));
        // Clamp al tope ±MAX_OFFSET_MS.
        sync.set_offset_ms(999_999);
        assert_eq!(sync.offset_ms(), MAX_OFFSET_MS);
        sync.set_offset_ms(-999_999);
        assert_eq!(sync.offset_ms(), -MAX_OFFSET_MS);
        // reset() NO toca el offset (es preferencia, no contador).
        sync.set_offset_ms(40);
        sync.reset();
        assert_eq!(sync.offset_ms(), 40);
    }

    #[test]
    fn set_config_ensancha_la_tolerancia() {
        let mut sync = AvSync::new(SyncConfig {
            present_ahead: ms(5),
            present_behind: ms(10),
        });
        // Con behind=10 ms, un frame 30 ms tarde se descarta.
        assert_eq!(sync.plan(ms(1000), ms(970)), FramePlan::Drop);
        // Ensanchamos la ventana de atraso a 100 ms: ahora se presenta.
        sync.set_config(SyncConfig {
            present_ahead: ms(5),
            present_behind: ms(100),
        });
        assert_eq!(sync.plan(ms(1000), ms(970)), FramePlan::Present);
    }
}
