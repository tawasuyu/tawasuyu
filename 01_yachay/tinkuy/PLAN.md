# PLAN — tinkuy

Visión declarada en el README (anti token-junkie, orden confirmado):

```
1. Motor Rust            ✅  B1–B5 cerrados (2026-05)
2. ABI WASM              ⬜  ← en curso (C1…C5)
3. DSL matemático        ⬜
4. Nodos visuales        ⬜
```

## Capa 1 — Motor Rust (HECHO)

Cerrada en commits `436007b`/`0b0f685`/`67d6dbf`:

- ECS SoA alineado a 64 B, handle generacional.
- Grid3D con listas intrusivas + transferencia worker-local (sin atómicos en hot).
- Velocity-Verlet paralelo por rangos disjuntos (rayon).
- Walls reflectivas, observables (KE/T/Σp), snapshots BLAKE3.
- Forces: Lennard-Jones + Coulomb con neighbor-list 27-celdas.
- Demo `tinkuy-sim` LJ end-to-end con reporte BLAKE3.

## Capa 2 — ABI WASM (PRÓXIMO)

Meta: `tinkuy-core` ejecutable dentro de Wawa userspace como app WASM.

- **C1 — Backend `wasm` single-thread.** ✅ (commit `7d90035`) Variantes `#[cfg(all(feature = "wasm", not(feature = "cpu")))]` de `kick_drift`, `finish_kick`, `lennard_jones`, `coulomb`. Tests host con `--features wasm`: 16/16 core + 8/8 forces.
- **C2 — ABI estable.** ✅ Crate `tinkuy-abi` (rlib) con superficie plana C-friendly. Handle opaco `TkSim` que agrupa `World + Grid3D + outboxes`:
  - `tk_sim_new(cap, *origin, cell_size, *dims, **out) -> i32`
  - `tk_sim_free(*sim)`
  - `tk_sim_spawn(*sim, x,y,z,vx,vy,vz,m,q, *out_idx) -> i32`
  - `tk_sim_len(*sim) -> u32`
  - `tk_sim_rebuild_grid(*sim) -> i32`
  - `tk_sim_step_lj(*sim, dt, ε, σ, cutoff, *bmin, *bmax) -> i32`
  - `tk_sim_kinetic_energy(*sim) -> f64`
  - `tk_sim_temperature(*sim, kb) -> f64`
  - `tk_sim_total_momentum(*sim, *out_xyz) -> i32`
  - `tk_sim_snapshot_cid(*sim, *out_32) -> i32`
  - `tk_sim_snapshot_export(*sim, **out_ptr, *out_len) -> i32`
  - `tk_buf_free(*ptr, len)`
  - Códigos: `TK_OK = 0`, `TK_ERR_NULL = -1`, `TK_ERR_INVALID = -2`, `TK_ERR_OOM = -3`.
  - Tests: 5/5 con `cpu` y 5/5 con `wasm`. `wasm32-unknown-unknown` compila.
- **C3 — App cdylib `03_ukupacha/wawa/apps/tinkuy`.** ✅ `pub use tinkuy_abi::*;` deja los 12 exports `tk_*` directos en el cdylib (verificado por `strings`). Pipeline release endurecido (opt-level=z + lto + codegen-units=1 + strip).
  - Decisión: la app wawa actúa como `tinkuy-wasm` (re-exporter cdylib). No hace falta un crate intermedio.
- **C4 — Integración kernel wawa.** Pendiente. El reactor `wasmi` debe cargar `assets/tinkuy.wasm`, exponer `tk_*` al host, y una app de UI (texto plano) llamar `tk_sim_new → spawn × N → step_lj × M → snapshot_cid` mostrando step/T/CID. Requiere tocar el loader del kernel; lo dejamos como sub-fase aislada porque cruza la frontera Ring 0 ↔ Ring 3.
- **C5 — `scripts/build-tinkuy.sh`.** ✅ cargo build wasm32-unknown-unknown → `wasm-opt -Os --strip-debug --strip-producers --enable-{bulk-memory,sign-ext,nontrapping-float-to-int,mutable-globals}` → consolida `wawa-kernel/assets/tinkuy.wasm`. Tamaño actual: **30 KB** (techo plan: 200 KB).

## Capa 3 — DSL matemático

Meta: definir fuerzas y condiciones iniciales sin recompilar Rust.

- **D1** ✅ Crate `tinkuy-dsl` (`#![no_std] + alloc`) con gramática mínima: vars (`r, r2, eps, sigma, qi, qj, mi, mj, dx, dy, dz`), ops aritméticos, funciones `pow/inv/sqrt`. Lexer + parser Pratt → AST (`Expr::{Num,Var,Neg,Bin,Call}`). Errores tipados (`ParseError`). Tests 12/12 con LJ, Coulomb, Hooke, precedencias, aridad. Compila a wasm32-unknown-unknown.
- **D2** ✅ Módulo `tinkuy_dsl::bytecode`. Compilador post-order `Expr → Bytecode { code: Vec<Op>, consts: Vec<f32>, stack_depth: u16 }`. Opcodes: `Const(u16)`, `LoadVar(Var)`, `Add/Sub/Mul/Div/Neg`, `Pow/Inv/Sqrt`. `eval_with_stack(bc, &VarBindings, &mut [f32]) -> Result<f32, EvalError>` con buffer del caller (cero allocs en hot). `stack_depth` calculado por simulación abstracta durante emit (peak tracking). `pow/sqrt/exp/ln` implementados sin libm (potencia entera 0..=12 directa + fallback exp/ln Taylor + NR sqrt) para sobrevivir bajo `#![no_std]`. Tests 20/20 incluyen LJ comparado contra fórmula nativa con tolerancia 1e-4.
- **D3** ✅ `tinkuy_forces::DslForce { bc, eps, sigma, cutoff, stack }`. `apply(world, grid)` itera por neighbor-list 27-celdas y evalúa el bytecode por par (i, j) con `VarBindings { r, r2, eps, sigma, qi, qj, mi, mj, dx, dy, dz }`. Convención: el DSL devuelve `F_over_r` (magnitud / r); la fuerza vectorial se compone como `F · (r_i − r_j)`, idéntica al kernel nativo LJ. Stack pre-alocado en `new`; cero allocs en `apply`. Tests: LJ DSL coincide con LJ nativo (Δ max < 0.5 sobre cubo 4³ con LJ exponente 12); Coulomb DSL coincide con nativo (Δ < 1e-3). Single-thread; paralelización vendrá si D4 (benches) la justifica.
- **D4** ✅ Módulo `tinkuy_dsl::optimize`. Const-fold (Add/Sub/Mul/Div/Neg/Pow entero/Inv/Sqrt) + simplificaciones algebraicas (x+0, x*1, x*0, x/1, pow(x,0), pow(x,1), inv(inv(x)), -(-x)) hasta fix-point. Mismo `nr_sqrt` que la VM → cero divergencia numérica. Tests 13/13: idempotencia, equivalencia LJ pre/post optimize (Δ < 1e-3), no crece el número de ops. Bench de velocidad ≥50% queda pendiente para cuando D2 mida con criterion.
- **D5** ✅ `tinkuy-dsl/examples/{lj,coulomb,hooke}.tnk` con comentarios `#` documentando la convención `F_over_r`. El lexer ignora líneas `#…`. Test `example_tnk_files_all_compile` carga los 3 con `include_str!` y verifica parse + optimize + compile.

## Capa 4 — Nodos visuales

Meta: editar fuerzas y escenas como grafo Llimphi.

- **E1** ✅ Crate `tinkuy-llimphi` con `llimphi-ui::App`. Panel único en `tiled_view_reorderable_cols(2)` con cuatro tiles draggables: visor (placeholder hasta E3), fuerzas (ε/σ/cutoff/dt/N + flag pausa), observables (step/t/KE/T/|p|/CID[..8]), snapshots (ring de 12 CIDs). Driver tinkuy-core en el hilo de UI: `Handle::spawn_periodic(33 ms, || Msg::Tick)` desde `init`; el `update` avanza 4 pasos de Velocity-Verlet + LJ + walls por tick (≈120 steps/s con N=64) y refresca observables. Atajos: `Space` pausa, `r` reset. `cargo check -p tinkuy-llimphi --example tinkuy_demo` y `cargo check --workspace` verdes. [[feedback_panel_tiles_draggables]] [[feedback_mvp_ugly_first]].
- **E2** Grafo de fuerzas sobre `llimphi-widget-nodegraph`: nodos vars + ops + salida F_ij. Compila a bytecode DSL (capa 3).
- **E3** Visor 3D mínimo: `View::paint_with(Scene)` pinta partículas como puntos coloreados por |v|. Proyección ortográfica; sin cámara orbital en MVP.
- **E4** Timeline de CIDs: cada `--snapshot-every` añade una entrada; click → carga el estado y rebobina el sim.
- **E5** Kernel `pluma-notebook-kernel-tinkuy`: invoca sims desde una celda notebook, devuelve `OutputPayload::Image{PNG visor}` + observables como texto.

## Reglas vivas

- Capas estrictamente secuenciales — no abrir D antes de cerrar C, ni E antes de D.
- Cada sub-fase cierra con commit + push (`feat(tinkuy-…)`/`feat(tinkuy-wasm)`/etc), pathspec explícito.
- `cargo check --workspace` no debe romperse jamás por trabajo en tinkuy.
- Compatibilidad Wawa: `tinkuy-core` con `--features wasm --no-default-features` debe compilar a `wasm32-unknown-unknown` sin `std::net`, `std::thread`, `std::time::Instant` (ver `snapshot.rs` y `observables.rs`).
