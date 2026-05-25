//! Aggregation de OHLC por bucket de **tiempo** (no de índice).
//!
//! Bucket index = `floor((bar.t - t_start) / bucket_duration)`.
//! Cuando cambia el bucket, commit del anterior:
//!
//! - `open` = primer `open` del bucket.
//! - `close` = último `close` del bucket.
//! - `high` = max(`high`) del bucket.
//! - `low` = min(`low`) del bucket.
//! - `volume` = sum(`volume`) del bucket.
//! - `t` = timestamp del primer bar del bucket (canónico para
//!   ploteo; el doc original sugiere usar el inicio del bucket
//!   pero acá preferimos el sample real para no introducir bias).
//!
//! Buckets vacíos no se emiten — el length de salida es ≤ inputs.
//! Fallback a index-bucketing si el span temporal es cero (todos
//! los timestamps colapsados, e.g. tick-data).

use crate::ohlc_buffer::{Bar, OhlcBuffer, STRIDE};

/// Agrega `src` en buckets de duración `bucket_duration` (en
/// unidades de `bar.t`). Escribe el output en `out` extendiéndolo
/// (no se clearea; el caller decide).
///
/// Si `bucket_duration <= 0` o el span del input es cero, hace
/// fallback a index-bucketing con `samples_per_bucket = 1` (es decir,
/// copia el input tal cual). Esto evita panic con tick-data
/// colapsado.
pub fn aggregate_time_bucketed(src: &OhlcBuffer, bucket_duration: f32, out: &mut OhlcBuffer) {
    if src.is_empty() {
        return;
    }
    let n = src.len();
    let (t_first, t_last) = src.time_range().unwrap();

    if bucket_duration <= 0.0 || (t_last - t_first).abs() < f32::EPSILON {
        // Fallback: copia tal cual.
        for i in 0..n {
            out.push_bar(src.bar(i));
        }
        return;
    }

    let mut current_bucket = i64::MIN;
    let mut acc_t: f32 = 0.0;
    let mut acc_o: f32 = 0.0;
    let mut acc_h: f32 = f32::NEG_INFINITY;
    let mut acc_l: f32 = f32::INFINITY;
    let mut acc_c: f32 = 0.0;
    let mut acc_v: f32 = 0.0;
    let mut has_acc = false;

    for i in 0..n {
        let b = src.bar(i);
        let bucket = ((b.t - t_first) / bucket_duration).floor() as i64;
        if bucket != current_bucket {
            if has_acc {
                out.push_bar(Bar {
                    t: acc_t,
                    o: acc_o,
                    h: acc_h,
                    l: acc_l,
                    c: acc_c,
                    v: acc_v,
                });
            }
            current_bucket = bucket;
            acc_t = b.t;
            acc_o = b.o;
            acc_h = b.h;
            acc_l = b.l;
            acc_c = b.c;
            acc_v = b.v;
            has_acc = true;
        } else {
            if b.h > acc_h {
                acc_h = b.h;
            }
            if b.l < acc_l {
                acc_l = b.l;
            }
            acc_c = b.c;
            acc_v += b.v;
        }
    }
    if has_acc {
        out.push_bar(Bar {
            t: acc_t,
            o: acc_o,
            h: acc_h,
            l: acc_l,
            c: acc_c,
            v: acc_v,
        });
    }

    // Una métrica de cordura: el output nunca puede ser más largo
    // que el input.
    debug_assert!(out.bars().len() / STRIDE <= n);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> OhlcBuffer {
        // 10 bars con t en `[0, 9]`, valores deterministicos.
        let mut b = OhlcBuffer::with_capacity(10);
        for i in 0..10 {
            let t = i as f32;
            let base = 100.0 + (i as f32) * 0.5;
            b.push_values(t, base, base + 1.0, base - 1.0, base + 0.2, 10.0);
        }
        b
    }

    #[test]
    fn bucket_de_3_agrega_a_4_bars() {
        // 10 inputs con t en `[0, 9]`, bucket 3 → buckets 0-2, 3-5, 6-8, 9.
        // = 4 buckets.
        let src = fixture();
        let mut out = OhlcBuffer::new();
        aggregate_time_bucketed(&src, 3.0, &mut out);
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn aggregation_preserva_volatilidad() {
        // Inventamos un bucket donde un bar tiene spike alto y otro
        // spike bajo. El aggregate debe capturar AMBOS extremos.
        let mut src = OhlcBuffer::new();
        src.push_values(0.0, 10.0, 12.0, 9.0, 11.0, 5.0);
        src.push_values(0.5, 11.0, 20.0, 10.5, 11.5, 5.0); // spike up
        src.push_values(0.8, 11.5, 12.0, 2.0, 11.0, 5.0); // spike down
        let mut out = OhlcBuffer::new();
        aggregate_time_bucketed(&src, 1.0, &mut out);
        assert_eq!(out.len(), 1);
        let agg = out.bar(0);
        assert_eq!(agg.h, 20.0, "max H debe sobrevivir");
        assert_eq!(agg.l, 2.0, "min L debe sobrevivir");
        assert_eq!(agg.o, 10.0, "first open");
        assert_eq!(agg.c, 11.0, "last close");
        assert_eq!(agg.v, 15.0, "sum volumes");
    }

    #[test]
    fn fallback_a_index_si_span_cero() {
        // Todos los t iguales — fallback copia 1:1.
        let mut src = OhlcBuffer::new();
        src.push_values(7.0, 1.0, 2.0, 0.0, 1.5, 1.0);
        src.push_values(7.0, 1.5, 2.5, 1.0, 2.0, 1.0);
        src.push_values(7.0, 2.0, 3.0, 1.0, 1.0, 1.0);
        let mut out = OhlcBuffer::new();
        aggregate_time_bucketed(&src, 1.0, &mut out);
        assert_eq!(out.len(), 3, "span 0 ⇒ copy 1:1");
    }

    #[test]
    fn empty_no_emite() {
        let src = OhlcBuffer::new();
        let mut out = OhlcBuffer::new();
        aggregate_time_bucketed(&src, 1.0, &mut out);
        assert_eq!(out.len(), 0);
    }
}
