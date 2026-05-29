//! Bench D4 — ¿cuánto acelera `optimize` el `eval` del bytecode?
//!
//! Cierra la promesa abierta del PLAN. Comparamos por cada fórmula
//! (LJ, Coulomb, Hooke) el throughput de `eval_with_stack` sobre el bytecode
//! crudo vs el bytecode optimizado, con `VarBindings` realistas barridos en
//! cada iteración para que el optimizador del backend no constant-fold el
//! bench mismo.
//!
//! Resultados medidos (2026-05-29, host Linux x86_64, criterion 0.5):
//!
//!   ┌─────────┬───────────┬───────────┬──────────┐
//!   │ fórmula │ raw       │ opt       │ speedup  │
//!   ├─────────┼───────────┼───────────┼──────────┤
//!   │ LJ      │ 13.2 Me/s │ 17.3 Me/s │  1.31×   │
//!   │ Coulomb │ 35.8 Me/s │ 35.5 Me/s │  1.00×   │
//!   │ Hooke   │ 46.7 Me/s │ 68.6 Me/s │  1.47×   │
//!   └─────────┴───────────┴───────────┴──────────┘
//!
//! Lectura: el optimize entrega +30 a +47% cuando hay constantes que plegar,
//! y 0% en fórmulas como Coulomb donde todas las variables son del par. El
//! ≥50% que el PLAN dejó como target no se alcanza con las simplificaciones
//! actuales (fold + algebraicas). Para ganar más habría que añadir CSE (la
//! `pow(σ/r, _)` aparece dos veces en LJ) o expansión `pow(_, 6n)` a `x²×x²×x²`.
//! Decisión 2026-05-29: aceptar el speedup observado y mantener `DslForce`
//! single-thread (sigue siendo el slow path; el fast path es el kernel
//! nativo de `tinkuy-forces` ya paralelizado con rayon).

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use tinkuy_dsl::{compile, eval_with_stack, optimize, parse, Bytecode, VarBindings};

const LJ: &str =
    "24 * eps * (2 * pow(sigma / r, 12) - pow(sigma / r, 6)) * inv(r2)";
const COULOMB: &str = "qi * qj * inv(r2) * sqrt(r2)";
const HOOKE: &str = "-100.0 * (r - 1.5)";

/// Genera 64 bindings sintéticos cubriendo el rango útil del integrador LJ.
fn make_bindings() -> [VarBindings; 64] {
    let mut out = [VarBindings::default(); 64];
    for (i, b) in out.iter_mut().enumerate() {
        let t = i as f32 / 63.0;
        let r = 0.9 + t * 1.5;
        *b = VarBindings {
            r,
            r2: r * r,
            eps: 1.0,
            sigma: 1.0,
            qi: 1.0,
            qj: -1.0,
            mi: 1.0,
            mj: 1.0,
            dx: r * 0.5,
            dy: r * 0.3,
            dz: r * 0.2,
        };
    }
    out
}

fn build(src: &str) -> (Bytecode, Bytecode) {
    let ast = parse(src).expect("parse");
    let raw = compile(&ast).expect("compile raw");
    let opt = compile(&optimize(ast)).expect("compile opt");
    (raw, opt)
}

fn bench_pair(c: &mut Criterion, label: &str, src: &str) {
    let (raw, opt) = build(src);
    let bindings = make_bindings();
    let mut group = c.benchmark_group(label);
    group.throughput(Throughput::Elements(bindings.len() as u64));

    group.bench_function("raw", |b| {
        let mut stack = [0.0f32; 32];
        b.iter(|| {
            let mut acc = 0.0f32;
            for v in &bindings {
                acc += eval_with_stack(&raw, black_box(v), &mut stack).unwrap();
            }
            black_box(acc)
        })
    });

    group.bench_function("opt", |b| {
        let mut stack = [0.0f32; 32];
        b.iter(|| {
            let mut acc = 0.0f32;
            for v in &bindings {
                acc += eval_with_stack(&opt, black_box(v), &mut stack).unwrap();
            }
            black_box(acc)
        })
    });

    group.finish();
}

fn benches(c: &mut Criterion) {
    bench_pair(c, "lj", LJ);
    bench_pair(c, "coulomb", COULOMB);
    bench_pair(c, "hooke", HOOKE);
}

criterion_group!(g, benches);
criterion_main!(g);
