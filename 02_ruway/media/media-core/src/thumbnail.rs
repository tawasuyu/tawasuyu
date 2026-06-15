//! thumbnail — lógica **pura** de miniaturas (sin I/O ni decode).
//!
//! La extracción real de un frame vive en `shared/foreign-av::extract_frame`
//! (regla #4) y el decode/cacheo de la imagen en la app (Llimphi). Acá sólo el
//! cálculo agnóstico que ambos comparten: a qué instante representativo le
//! corresponde una posición del timeline, y cómo cuantizarlo en *buckets* para
//! no extraer un frame por cada píxel bajo el cursor (estilo thumbfast de mpv).

use core::time::Duration;

/// Cuantización del timeline en `buckets` instantes representativos. Un hover
/// que cae en el mismo bucket reusa la misma miniatura cacheada — clave estable
/// para la caché y techo de cuántos frames se extraen por medio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbGrid {
    /// Cantidad de buckets a lo largo de toda la duración (≥ 1).
    pub buckets: u32,
}

impl Default for ThumbGrid {
    fn default() -> Self {
        // 100 buckets ≈ 1 miniatura por cada 1% de la barra: fino al ojo, barato.
        ThumbGrid { buckets: 100 }
    }
}

impl ThumbGrid {
    pub fn new(buckets: u32) -> Self {
        ThumbGrid {
            buckets: buckets.max(1),
        }
    }

    /// Índice de bucket [0, buckets) para una fracción `[0,1]` del timeline.
    /// Clampa fuera de rango. Determinista — sirve de clave de caché.
    pub fn bucket_for_fraction(&self, fraction: f32) -> u32 {
        let n = self.buckets.max(1);
        if !fraction.is_finite() || fraction <= 0.0 {
            return 0;
        }
        if fraction >= 1.0 {
            return n - 1;
        }
        let b = (fraction * n as f32) as u32;
        b.min(n - 1)
    }

    /// Índice de bucket para una posición absoluta dentro de `duration`.
    pub fn bucket_for_pos(&self, pos: Duration, duration: Duration) -> u32 {
        if duration.is_zero() {
            return 0;
        }
        let f = pos.as_secs_f64() / duration.as_secs_f64();
        self.bucket_for_fraction(f as f32)
    }

    /// Instante **representativo** (centro del bucket) para extraer la miniatura
    /// de `bucket`, dentro de `duration`. Clampado a `[0, duration)`.
    pub fn instant_for_bucket(&self, bucket: u32, duration: Duration) -> Duration {
        let n = self.buckets.max(1);
        let b = bucket.min(n - 1);
        // Centro del bucket: (b + 0.5) / n.
        let f = (b as f64 + 0.5) / n as f64;
        let secs = (f * duration.as_secs_f64()).max(0.0);
        // No pedir exactamente el final (suele no haber frame ahí).
        let cap = (duration.as_secs_f64() - 0.05).max(0.0);
        Duration::from_secs_f64(secs.min(cap))
    }

    /// Atajo: instante representativo directo para una fracción del timeline.
    pub fn instant_for_fraction(&self, fraction: f32, duration: Duration) -> Duration {
        self.instant_for_bucket(self.bucket_for_fraction(fraction), duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_se_clampean() {
        let g = ThumbGrid::new(10);
        assert_eq!(g.bucket_for_fraction(-1.0), 0);
        assert_eq!(g.bucket_for_fraction(0.0), 0);
        assert_eq!(g.bucket_for_fraction(0.05), 0);
        assert_eq!(g.bucket_for_fraction(0.15), 1);
        assert_eq!(g.bucket_for_fraction(0.99), 9);
        assert_eq!(g.bucket_for_fraction(1.0), 9);
        assert_eq!(g.bucket_for_fraction(2.0), 9);
    }

    #[test]
    fn new_nunca_da_cero_buckets() {
        assert_eq!(ThumbGrid::new(0).buckets, 1);
        assert_eq!(ThumbGrid::new(0).bucket_for_fraction(0.7), 0);
    }

    #[test]
    fn pos_a_bucket_usa_duracion() {
        let g = ThumbGrid::new(4);
        let dur = Duration::from_secs(100);
        assert_eq!(g.bucket_for_pos(Duration::from_secs(0), dur), 0);
        assert_eq!(g.bucket_for_pos(Duration::from_secs(30), dur), 1);
        assert_eq!(g.bucket_for_pos(Duration::from_secs(80), dur), 3);
        // Duración cero → bucket 0 (sin división por cero).
        assert_eq!(g.bucket_for_pos(Duration::from_secs(5), Duration::ZERO), 0);
    }

    #[test]
    fn instante_es_centro_del_bucket_y_no_excede() {
        let g = ThumbGrid::new(10);
        let dur = Duration::from_secs(100);
        // Bucket 0 → centro en 5%.
        assert!((g.instant_for_bucket(0, dur).as_secs_f64() - 5.0).abs() < 1e-6);
        // Bucket 5 → centro en 55%.
        assert!((g.instant_for_bucket(5, dur).as_secs_f64() - 55.0).abs() < 1e-6);
        // Último bucket no pide el final exacto (cap a dur-0.05).
        let last = g.instant_for_bucket(9, dur);
        assert!(last.as_secs_f64() <= 100.0 - 0.05 + 1e-9);
        // Bucket fuera de rango se clampa al último.
        assert_eq!(g.instant_for_bucket(999, dur), last);
    }

    #[test]
    fn round_trip_fraccion_a_instante() {
        let g = ThumbGrid::default();
        let dur = Duration::from_secs(600);
        let t = g.instant_for_fraction(0.5, dur);
        // ~mitad del video.
        assert!((t.as_secs_f64() - 300.0).abs() < 5.0);
    }
}
