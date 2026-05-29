# PLAN — tinkuy

Visión declarada en el README (anti token-junkie, orden confirmado):

```
1. Motor Rust            ✅  B1–B5 cerrados (2026-05)
2. ABI WASM              ✅  C1–C5 cerrados (2026-05)
3. DSL matemático        ✅  D1–D5 cerrados (2026-05)
4. Nodos visuales        ✅  E1–E5 cerrados (2026-05)
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
- **C4 — Integración kernel wawa.** ✅ Módulo `wawa-kernel/src/tinkuy.rs` instala `assets/tinkuy.wasm` UNA sola vez en su propia sub-jaula `wasmi` (Store independiente, fuel desactivado por ser código del kernel, Linker vacío — el cdylib no importa nada). Resuelve los 9 `TypedFunc` `tk_*` y crece 64 páginas extras como scratch: dlmalloc del modulo nunca toca esas páginas, así que el kernel las usa indefinidamente como buzón de parámetros para los punteros que exigen `tk_sim_new`/`tk_sim_step_lj`/`tk_sim_snapshot_cid`. Tabla `[Option<Slot>; 8]` por `indice_app` aísla sims entre apps (matemática, no permisos). Cleanup automático en `AplicacionWasm::drop` (`liberar_owner`) evita sims huérfanas si una app cae. Nueva matriz de capacidades `sys_tinkuy_*` (7 syscalls: sim_new, sim_spawn, sim_rebuild_grid, sim_step_lj, sim_len, sim_observables, sim_snapshot_cid, sim_free) gateada por `PERMISO_TINKUY = 1 << 6` en `format`. App userspace `apps/testigo` ejerce el ciclo completo: `sim_new → spawn × 64 (lattice 4³ con velocidades xorshift32) → rebuild_grid → step_lj × 4/tick → observables → snapshot_cid` y pinta step / T / KE / CID[..16] con la 8×8 escalada ×2 + mini-barra de KE. Sembrada en GENESIS como app 15 con `PERMISO_TINKUY`, región `(600, 520, 480, 240)`, FUEL_COMUN. `cargo +nightly check --target x86_64-unknown-none -Z build-std=core,alloc` verde sobre el kernel; `cargo check --workspace` verde; `cargo build -p testigo --target wasm32-unknown-unknown --release` → 7.55 KB crudo / 5.35 KB sellado (`scripts/build-testigo.sh`).
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
- **E2** ✅ Módulo `tinkuy_llimphi::grafo`. `NodeKind::{Var, Num, Bin, Func, Neg, Output}` con `n_inputs/n_outputs/input_labels/output_labels/title` por variante. `ForceGraph { nodes, wires, next_id }` con `spawn`, `connect`, `rewire_input` (un cable por pin destino, último gana) y `move_node`. `lift_to_expr` hace DFS desde el `Output` por los cables — devuelve `Expr` o `LiftError::{SinSalida, SalidaDuplicada, PinDesconectado, Ciclo}`. `lennard_jones_default()` pre-construye los 16 nodos + 17 cables que codifican `24·ε·(2·(σ/r)¹² − (σ/r)⁶)·(1/r²)`. El tile "fuerzas" del E1 ahora pinta `nodegraph_view` con la paleta del tema; drag de title bar mueve nodos, drag de pin a pin reconecta + recompila (`lift_to_expr → optimize → compile → DslForce::from_bytecode`); el resultado reemplaza el call a `lennard_jones` dentro de `velocity_verlet_step`. Errores se muestran en la title bar del tile. Tests 4/4 (LJ DSL ≡ LJ nativo con tolerancia 1e-3 + tres caminos de error). `cargo check --workspace` verde.
- **E3** ✅ Módulo `tinkuy_llimphi::visor`. Proyección axonométrica fija pura (`project(x,y,z) = (x + 0.6·z, y + 0.4·z)`) sin cámara orbital; helpers `project_bbox`, `box_corners`, `BOX_EDGES`, `depth_key` aislados sin deps gráficas. El tile "visor 3D" del E1 ahora pinta vía `View::paint_with`: wireframe de la caja sim (12 aristas) + partículas como círculos (`kurbo::Circle`, radio 3 px) coloreadas por |v| (cold→hot, `Color::lerp_rect` en sRGB premultiplicado, `vmax` recalculado por frame). Painter's algorithm con `depth_key = z + 0.3·x` (back-to-front) para que el orden sea estable cuando varias partículas comparten z. Captura las SoA por valor en el `paint_with` (~1.5 KiB/frame con N=64; el coste del compositor lo eclipsa). Tests 6/6 sobre la proyección pura. `cargo check --workspace` verde (los errores residuales viven en agora-cli y son ajenos a tinkuy).
- **E4** ✅ Rewind por click sobre el ring de CIDs. `tinkuy_core::snapshot` gana `Snapshot::restore_into(bytes, world) -> Result<(), RestoreError>` — inverso bit-exacto de `capture` (parsea header `u64 n`, vuelca las 11 SoA, zera `ax_prev`/`ay_prev`/`az_prev` y sincroniza `len`/`generations` con `World::set_len_for_restore`). El ring buffer del frontend pasa de `(step, cid_short)` a `(step, cid_short, Arc<[u8]>)` para que el rewind sea O(1) clones y O(n) copia. `Msg::LoadSnapshot { idx }` restaura el `World`, repuebla la grilla espacial (`grid.rebuild`), retrocede `step`/`t` al instante capturado, recaptura observables y **pausa** la sim para que el usuario inspeccione antes de reanudar (Space para retomar). Cada fila del tile snapshots es clickeable con `hover_fill = theme.bg_row_hover`; el marker `▶` señala la fila correspondiente al `step` actual. Tests 6/6 en `tinkuy-core::snapshot` (round-trip idempotente, errores `HeaderFaltante`/`PayloadIncoherente`, `ax_prev` zerado) + 2 nuevos en `tinkuy-llimphi::rewind_tests` (CID bit-exacto tras 16 steps + rewind a dos estados distintos). `cargo check --workspace` y `wasm32-unknown-unknown --features wasm` verdes.
- **E5** ✅ Crate `00_unanchay/pluma/pluma-notebook-kernel-tinkuy` (`TinkuyKernel: Kernel`). Una celda con `language = "tinkuy-lj"` y source `key = value` (steps, side, dt, temp_init, sigma, epsilon, cutoff, seed, width, height — todas opcionales con defaults del demo Llimphi) corre una sim LJ end-to-end y devuelve `CellOutput { stdout: bloque de observables, value: CID hex (64 chars), payload: OutputPayload::Image { mime: "image/png", … } }`. Renderer headless propio: `Raster` RGBA8 con Bresenham + disco escaneado por filas → `png::Encoder` (sin vello, sin GPU). Misma proyección axonométrica que el visor Llimphi (inline, sin depender de la UI). Caps defensivos: `side ≤ 12`, `steps ≤ 10_000`, `width/height ≤ 1024`. Tests 8/8 (lenguaje desconocido, defaults, parse con comentarios + hex seed, determinismo, distintas seeds, clave/side inválidos, integración `run_from`). El workspace ahora lista el crate en `Cargo.toml`; `cargo check --workspace` verde.

## Reglas vivas

- Capas estrictamente secuenciales — no abrir D antes de cerrar C, ni E antes de D.
- Cada sub-fase cierra con commit + push (`feat(tinkuy-…)`/`feat(tinkuy-wasm)`/etc), pathspec explícito.
- `cargo check --workspace` no debe romperse jamás por trabajo en tinkuy.
- Compatibilidad Wawa: `tinkuy-core` con `--features wasm --no-default-features` debe compilar a `wasm32-unknown-unknown` sin `std::net`, `std::thread`, `std::time::Instant` (ver `snapshot.rs` y `observables.rs`).
