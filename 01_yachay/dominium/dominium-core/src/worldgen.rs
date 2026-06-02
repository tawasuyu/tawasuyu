//! Generación procedural del mundo: PRNG, ruido fbm, ríos y el [`seed`] que
//! esculpe biomas y reparte Lemmings sobre tierra firme.
//!
//! Motor agnóstico de GUI (regla #2): extraído de `dominium-app-llimphi`,
//! que ahora sólo envuelve [`seed`] pasándole sus dimensiones de grilla, su
//! población de Lemmings y el pack de [`Conceptos`] (default o del usuario).
//! No conoce frontends ni paletas de render.

use crate::{Conceptos, World};

// ---------------------------------------------------------------------
// PRNG mínimo (LCG 64) — siembra reproducible sin dependencias.
// ---------------------------------------------------------------------

struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        // Shift por 32 (no 33): los 32 bits altos del LCG son los de mejor
        // calidad, y `as u32` los toma sin perder el bit 31. La versión
        // anterior usaba `>> 33`, dejando un resultado en `[0, 2^31)` →
        // `next_f32()` retornaba `[0, 0.5)` y todo el mundo era mar.
        (self.0 >> 32) as u32
    }
    fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Esculpe un río senoidal entre `(x0, y0)` y el borde opuesto, pintando
/// `psique` alta y limpiando `materia` a lo largo del trazo. El río tiene
/// ancho `width` celdas y serpentea con amplitud `wiggle` perpendicular al
/// rumbo. La curva se muestrea a paso unitario.
fn carve_river(w: &mut World, rng: &mut Lcg, vertical: bool, length: usize, width: f32, wiggle: f32) {
    let g_w = w.grid.width as f32;
    let g_h = w.grid.height as f32;
    let start = rng.next_f32() * if vertical { g_w } else { g_h };
    let phase = rng.next_f32() * core::f32::consts::TAU;
    let freq = 0.06 + rng.next_f32() * 0.05;
    for s in 0..length {
        let t = s as f32;
        let bend = libm::sinf(t * freq + phase) * wiggle;
        let (cx_f, cy_f) = if vertical {
            (start + bend, t * g_h / length as f32)
        } else {
            (t * g_w / length as f32, start + bend)
        };
        let r = width.ceil() as i64;
        for dy in -r..=r {
            for dx in -r..=r {
                let x = cx_f + dx as f32;
                let y = cy_f + dy as f32;
                if x < 0.0 || y < 0.0 || x >= g_w || y >= g_h {
                    continue;
                }
                let d = libm::sqrtf((dx as f32).powi(2) + (dy as f32).powi(2));
                if d > width {
                    continue;
                }
                let intensity = 1.0 - d / width;
                let idx = w.grid.idx(x as usize, y as usize);
                // Río = mucha psique (agua azul), nada de materia, sin oro.
                w.grid.psique[idx] = (w.grid.psique[idx] + 130.0 * intensity).min(180.0);
                w.grid.materia[idx] *= 1.0 - intensity * 0.95;
                w.grid.oro[idx] *= 1.0 - intensity * 0.8;
                w.grid.poder[idx] *= 1.0 - intensity * 0.8;
                w.grid.degradacion[idx] *= 1.0 - intensity * 0.9;
            }
        }
    }
}

/// Value noise multioctava determinista. Devuelve `Vec<f32>` de tamaño
/// `w*h` con valores aproximadamente en `[-1, 1]`. Las octavas suben en
/// frecuencia y bajan en amplitud — la primera define continentes, las
/// últimas, granulado. Smoothstep `s(t) = t²(3-2t)` entre celdas coarse.
fn fbm_noise(seed: u64, w: usize, h: usize) -> Vec<f32> {
    let mut rng = Lcg::new(seed);
    let mut field = vec![0.0_f32; w * h];
    // (frecuencia, amplitud). 4 octavas: 6×6 continentes → 96×96 ruido fino.
    let octaves: [(usize, f32); 4] = [(6, 1.0), (12, 0.55), (24, 0.30), (96, 0.18)];
    let mut amp_norm = 0.0_f32;
    for (_, a) in &octaves {
        amp_norm += a;
    }
    for (n, amp) in octaves {
        // Grilla coarse (n+1)×(n+1) de valores aleatorios en [-1, 1].
        let coarse_w = n + 1;
        let mut coarse = vec![0.0_f32; coarse_w * coarse_w];
        for v in coarse.iter_mut() {
            *v = rng.next_f32() * 2.0 - 1.0;
        }
        let sx = n as f32 / w as f32;
        let sy = n as f32 / h as f32;
        for y in 0..h {
            for x in 0..w {
                let fx = x as f32 * sx;
                let fy = y as f32 * sy;
                let cx = (fx.floor() as usize).min(n - 1);
                let cy = (fy.floor() as usize).min(n - 1);
                let tx = (fx - cx as f32).clamp(0.0, 1.0);
                let ty = (fy - cy as f32).clamp(0.0, 1.0);
                let smooth = |a: f32| a * a * (3.0 - 2.0 * a);
                let u = smooth(tx);
                let v = smooth(ty);
                let a = coarse[cy * coarse_w + cx];
                let b = coarse[cy * coarse_w + cx + 1];
                let c = coarse[(cy + 1) * coarse_w + cx];
                let d = coarse[(cy + 1) * coarse_w + cx + 1];
                let p = a * (1.0 - u) + b * u;
                let q = c * (1.0 - u) + d * u;
                field[y * w + x] += amp * (p * (1.0 - v) + q * v);
            }
        }
    }
    for v in field.iter_mut() {
        *v /= amp_norm;
    }
    field
}

/// Siembra un mundo cuadrado `grid × grid`: continentes de materia, vetas de
/// oro, niebla de psique y una población de `lemmings` Lemmings con sesgos y
/// acciones variadas. Los `conceptos` (default embebido o pack del usuario)
/// se asignan al mundo resultante — el caller decide cuáles.
pub fn seed(seed: u64, grid: usize, lemmings: usize, conceptos: Conceptos) -> World {
    let mut w = World::new(grid, grid);
    let mut rng = Lcg::new(seed);
    // --- Capas iniciales basadas en dos campos fbm independientes ---
    // `elev` ∈ ~[-1, 1] decide bioma; `humid` ∈ ~[-1, 1] modula fertilidad.
    let elev = fbm_noise(seed ^ 0xE1E_7A57, grid, grid);
    let humid = fbm_noise(seed ^ 0x4D015_7CE, grid, grid);
    for cy in 0..grid {
        for cx in 0..grid {
            let idx = w.grid.idx(cx, cy);
            let e_raw = elev[idx];
            let h = humid[idx];
            // Forma del continente:
            //  - bias +0.25 → tierra domina globalmente.
            //  - edge_drop · 0.30 → costas/bordes en mar.
            // E[edge_drop] = 2/3 en una grilla uniforme → mean(e) ≈ 0.05,
            // con FBM std ≈ 0.24 da ~35% mar, ~65% tierra.
            let nx = (cx as f32 / grid as f32) * 2.0 - 1.0;
            let ny = (cy as f32 / grid as f32) * 2.0 - 1.0;
            let edge_drop = nx.abs().max(ny.abs());
            let e = e_raw + 0.30 - edge_drop * 0.28;

            if e < -0.18 {
                // Mar profundo: psique alta para que el azul aguante la
                // difusión lenta (entropy=0.005, diffusion=0.02 → unos cientos
                // de ticks antes de notarse erosión visual). Pintar también
                // `degradacion` baja persistente refuerza el tono frío y
                // ancla la celda como "no fértil" para los lemmings que la
                // crucen.
                w.grid.psique[idx] = 180.0 + rng.next_f32() * 30.0;
                w.grid.degradacion[idx] = 2.0;
            } else if e < -0.05 {
                // Mar somero / lagunas: agua más clara, mínima vida acuática.
                w.grid.psique[idx] = 110.0 + rng.next_f32() * 20.0;
                w.grid.materia[idx] = rng.next_f32() * 4.0;
                w.grid.degradacion[idx] = 1.0;
            } else if e < 0.08 {
                // Costa / pantano fértil: alta materia + algo de agua.
                w.grid.materia[idx] = 45.0 + (h.max(0.0)) * 30.0 + rng.next_f32() * 6.0;
                w.grid.psique[idx] = 18.0 + rng.next_f32() * 8.0;
                if rng.next_f32() > 0.94 {
                    w.grid.oro[idx] = rng.next_f32() * 18.0;
                }
            } else if e < 0.30 {
                // Llanura: el granero del mundo. Materia muy alta cuando
                // hay humedad; menos donde el clima es seco.
                let fertility = (h * 0.5 + 0.5).clamp(0.2, 1.0);
                w.grid.materia[idx] = 50.0 + fertility * 50.0 + rng.next_f32() * 5.0;
                if rng.next_f32() > 0.92 {
                    w.grid.oro[idx] = rng.next_f32() * 24.0;
                }
            } else if e < 0.42 {
                // Colinas: materia decreciente, asoma el poder (vetas).
                let alpha = (e - 0.30) / 0.12;
                w.grid.materia[idx] = (1.0 - alpha) * 35.0 + rng.next_f32() * 4.0;
                w.grid.poder[idx] = alpha * 9.0;
                if rng.next_f32() > 0.82 {
                    w.grid.oro[idx] = rng.next_f32() * 30.0; // minas en colinas
                }
            } else {
                // Montañas / picos: poco material vivo, mucha estructura
                // bruta (poder) y, en los más altos, cicatriz rocosa. Umbral
                // bajado a 0.42 (en la cola del FBM con mean ≈ +0.08) para
                // que ~10% del mapa sea cordillera visible.
                let alpha = ((e - 0.42) / 0.40).clamp(0.0, 1.0);
                w.grid.poder[idx] = 6.0 + alpha * 18.0;
                w.grid.degradacion[idx] = 1.5 + alpha * alpha * 14.0;
                if rng.next_f32() > 0.97 {
                    w.grid.oro[idx] = rng.next_f32() * 35.0;
                }
            }
        }
    }
    // --- Ríos: 2 cruces. Uno vertical, uno horizontal. Sin erosión real
    //     — los ríos se pintan encima del bioma sobrescribiendo. ---
    carve_river(&mut w, &mut rng, true, grid, 2.4, grid as f32 * 0.18);
    carve_river(&mut w, &mut rng, false, grid, 1.8, grid as f32 * 0.14);

    // --- Lemmings: distribuidos solo en tierra firme (e ∈ [-0.05, 0.45]).
    //     Rechaza candidatos en mar o pico. Si tras 32 intentos no encuentra
    //     un punto válido, suelta donde caiga (failsafe para no congelar el
    //     seed). ---
    let pick_land = |rng: &mut Lcg, elev: &[f32]| -> (f32, f32) {
        for _ in 0..64 {
            let x = rng.next_f32() * (grid as f32 - 1.0);
            let y = rng.next_f32() * (grid as f32 - 1.0);
            let nx = (x / grid as f32) * 2.0 - 1.0;
            let ny = (y / grid as f32) * 2.0 - 1.0;
            let edge_drop = nx.abs().max(ny.abs());
            // Misma transformación que el biomeing arriba, así los
            // lemmings caen en celdas-tierra coherentes.
            let e = elev[(y as usize) * grid + (x as usize)] + 0.30 - edge_drop * 0.28;
            if e > -0.05 && e < 0.45 {
                return (x, y);
            }
        }
        (
            rng.next_f32() * (grid as f32 - 1.0),
            rng.next_f32() * (grid as f32 - 1.0),
        )
    };
    for k in 0..lemmings {
        let (x, y) = pick_land(&mut rng, &elev);
        let psi = [
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
        ];
        let i = w.lemmings.spawn(x, y, 40.0 + rng.next_f32() * 40.0, psi);
        // Distribución calibrada al punto fijo del sistema con herencia
        // de acción + intercambio fuerte (trade_amount = 1.5):
        //   α_e = 0.30 (Extraer · cosecha — fuente principal de E)
        //   α_t = 0.30 (Intercambiar · redistribución — evita concentración)
        //   α_m = 0.20 (Mover · exploración)
        //   α_r = 0.15 (Replicar · natalidad)
        //   α_s = 0.05 (Sincronizar · convergencia cultural)
        //
        // Balance energético por capita en equilibrio:
        //   dE/dt = α_e · e_r - α_m · c_m - α_r · f · E_r · 1[E_r>T]
        //         = 0.30·2.5 - 0.20·0.06 - 0.15·0.45·E_r
        //         = 0.738 - 0.0675·E_r
        //   E* = 0.738 / 0.0675 ≈ 11 (cerca del threshold T=12)
        // El sistema oscila alrededor de ese E*, replicando a baja
        // frecuencia pero sostenidamente.
        w.lemmings.accion[i] = match k % 20 {
            0..=5 => 1,            // 6/20 = 0.30 Extraer
            6..=11 => 3,           // 6/20 = 0.30 Intercambiar
            12..=15 => 0,          // 4/20 = 0.20 Mover
            16..=18 => 4,          // 3/20 = 0.15 Replicar
            _ => 2,                // 1/20 = 0.05 Sincronizar
        } as u8;
    }
    w.conceptos = conceptos;
    w
}

#[cfg(test)]
mod seeding_tests {
    //! Tests del seeding del mundo. No verifican la física (eso ya está en
    //! `dominium-core` / `dominium-physics`), sólo que la distribución de
    //! biomas tras `seed()` queda en proporciones razonables: ni todo mar
    //! ni todo montaña.

    use super::*;
    use crate::Conceptos;

    // Grilla y población de prueba (espejan los consts de la app:
    // GRID = 240, LEMMINGS = 2500) para que los rangos esperados valgan.
    const GRID: usize = 240;
    const LEMMINGS: usize = 2500;

    fn seed_demo(s: u64) -> World {
        seed(s, GRID, LEMMINGS, Conceptos::default())
    }

    /// Clasificación de bioma a partir de las capas de una celda. Espeja
    /// los thresholds de `seed()` para validar lo que efectivamente quedó
    /// pintado.
    fn classify_cell(g: &crate::Grid, idx: usize) -> &'static str {
        // Mar profundo: mucha psique y nada de materia/poder.
        if g.psique[idx] > 150.0 && g.materia[idx] < 1.0 {
            "mar_profundo"
        } else if g.psique[idx] > 80.0 && g.materia[idx] < 6.0 {
            "mar_somero"
        } else if g.psique[idx] > 15.0 && g.psique[idx] <= 80.0 && g.materia[idx] > 30.0 {
            "costa"
        } else if g.materia[idx] > 40.0 && g.poder[idx] < 0.5 {
            "llanura"
        } else if g.poder[idx] >= 0.5 && g.poder[idx] < 8.0 {
            "colina"
        } else if g.poder[idx] >= 8.0 || g.degradacion[idx] > 4.0 {
            "pico"
        } else {
            "otro"
        }
    }

    /// Sanity: el LCG genera valores uniformes en [-1, 1] (esta función
    /// hubiera capturado el bug `>> 33` original donde la mean era -0.5).
    #[test]
    fn lcg_genera_distribucion_simetrica() {
        let mut rng = Lcg::new(1234);
        let mut sum = 0.0_f64;
        let n = 100_000;
        for _ in 0..n {
            sum += (rng.next_f32() * 2.0 - 1.0) as f64;
        }
        let mean = sum / n as f64;
        assert!(
            mean.abs() < 0.02,
            "LCG sesgado: mean = {mean:.4} (debe estar cerca de 0)"
        );
    }

    #[test]
    fn seed_default_balances_biomas() {
        let w = seed_demo(0xD0_31_31_07);
        let total = w.grid.cells();
        let mut hist = std::collections::HashMap::<&'static str, usize>::new();
        for i in 0..total {
            *hist.entry(classify_cell(&w.grid, i)).or_default() += 1;
        }
        let pct = |k: &str| -> f32 {
            *hist.get(k).unwrap_or(&0) as f32 / total as f32 * 100.0
        };
        let mar = pct("mar_profundo") + pct("mar_somero");
        // El mar no debe dominar el mapa visualmente (versión anterior daba
        // ~50% mar y al usuario "todo se ve azul al inicio").
        assert!(
            mar < 40.0,
            "mar < 40% del mapa, fue {:.1}% — el bias continental no está empujando suficiente tierra",
            mar
        );
        // Y al menos hay mar — sin mar no hay distinción agua/tierra.
        assert!(mar > 10.0, "mar > 10%, fue {:.1}% — el mapa quedó casi sin agua", mar);
        // La tierra incluye llanura (la mayoría de los lemmings vive ahí).
        assert!(
            pct("llanura") > 18.0,
            "llanura > 18%, fue {:.1}% — sin granero el motor se ahoga",
            pct("llanura")
        );
        // Picos visibles pero no dominantes (la versión anterior daba el
        // mapa "casi plano").
        let pico = pct("pico");
        assert!(
            (5.0..28.0).contains(&pico),
            "pico ∈ [5, 28]%, fue {:.1}% — cordillera fuera de rango",
            pico
        );
    }

    #[test]
    fn lemmings_no_se_acumulan_en_un_cuadrante() {
        let w = seed_demo(0xD0_31_31_07);
        // Reparto por cuadrante.
        let mut q = [0_u32; 4];
        for i in 0..w.lemmings.len() {
            let x = w.lemmings.pos_x[i];
            let y = w.lemmings.pos_y[i];
            let h = GRID as f32 / 2.0;
            let qi = match (x >= h, y >= h) {
                (false, false) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (true, true) => 3,
            };
            q[qi] += 1;
        }
        let total = w.lemmings.len() as u32;
        // Ningún cuadrante > 75% de la población (versión anterior tenía
        // seeds donde el continente caía en un solo cuadrante y todos los
        // lemmings se apilaban ahí).
        for (i, &n) in q.iter().enumerate() {
            let pct = n as f32 / total as f32 * 100.0;
            assert!(
                pct < 75.0,
                "cuadrante {i} concentra {pct:.1}% de los lemmings — el bias continental + center_lift no está dispersando bien"
            );
        }
    }
}
