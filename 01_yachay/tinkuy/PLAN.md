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

- **C1 — Backend `wasm` single-thread.** Añadir variantes `#[cfg(feature = "wasm")]` de `kick_drift`, `finish_kick`, `lennard_jones`, `coulomb` que iteran 0..n en single-thread (sin rayon, sin SyncPtr). `cargo check -p tinkuy-core --no-default-features --features wasm --target wasm32-unknown-unknown` debe pasar; ídem `-p tinkuy-forces`. Tests `cfg(all(test, feature = "wasm"))` que validen equivalencia con la rama cpu sobre 32 partículas LJ (mismo CID).
- **C2 — ABI estable.** Diseñar superficie C-friendly (`#[repr(C)]`, `extern "C"`, sin lifetimes, errores como `i32`, punteros opacos a `World`/`Grid3D`):
  - `tk_world_new(cap, *mut *mut World) -> i32`
  - `tk_world_spawn(w, x, y, z, vx, vy, vz, m, q, *mut u32) -> i32`
  - `tk_world_step(w, grid, dt, *mut LjParams) -> i32`
  - `tk_world_snapshot_cid(w, *mut [u8; 32]) -> i32`
  - `tk_world_export_postcard(w, *mut *mut u8, *mut usize) -> i32` (alloc en `tinkuy-wasm` allocator, free explícito).
  - `tk_world_import_postcard(*const u8, len, *mut *mut World) -> i32`
- **C3 — Crate `tinkuy-wasm`.** `crate-type = ["cdylib"]`, target `wasm32-unknown-unknown`. Re-exporta la ABI. Objetivo: <200 KB post `wasm-opt -Os`. Sin `wasm-bindgen` para mantener ABI plana (lo consume Wawa, no JS).
- **C4 — App wawa `03_ukupacha/wawa/apps/tinkuy/`.** Carga `tinkuy-wasm.wasm`, corre LJ 256 partículas, muestra step/T/CID en texto plano. Reusa el pipeline de `apps/pluma/` (host: `wasmi`).
- **C5 — `scripts/build-tinkuy.sh`.** Análogo a `build-pluma.sh`: cargo build wasm32 → wasm-opt -Os → copia a `wawa-kernel/assets/`.

## Capa 3 — DSL matemático

Meta: definir fuerzas y condiciones iniciales sin recompilar Rust.

- **D1** Gramática mínima (`tinkuy-dsl`): vars (r, ε, σ, q_i, q_j, m_i), ops aritméticos, pow, 1/r. Sin lambdas, sin control de flujo.
- **D2** Lexer + Pratt parser → AST → bytecode stack-machine (`u8` opcodes, constantes en pool). Sin allocs en eval.
- **D3** `BytecodeForce: Force` enchufable a `World`. Eval por pareja (i, j) en neighbor-list igual que LJ/Coulomb.
- **D4** Optimizador: const-fold + reconocimiento de patrones comunes (`1/r²`, `(σ/r)⁶`). Meta: ≥50% de la velocidad de LJ nativo en bench de 100k partículas.
- **D5** Ejemplos en `tinkuy-dsl/examples/`: `lj.tnk`, `coulomb.tnk`, `hooke.tnk`.

## Capa 4 — Nodos visuales

Meta: editar fuerzas y escenas como grafo Llimphi.

- **E1** Crate `tinkuy-llimphi` con `llimphi-ui::App`. Layout obligatorio: panel único con tiles draggables (visor 3D · panel fuerzas · observables · scrubber snapshots). [[feedback_panel_tiles_draggables]].
- **E2** Grafo de fuerzas sobre `llimphi-widget-nodegraph`: nodos vars + ops + salida F_ij. Compila a bytecode DSL (capa 3).
- **E3** Visor 3D mínimo: `View::paint_with(Scene)` pinta partículas como puntos coloreados por |v|. Proyección ortográfica; sin cámara orbital en MVP.
- **E4** Timeline de CIDs: cada `--snapshot-every` añade una entrada; click → carga el estado y rebobina el sim.
- **E5** Kernel `pluma-notebook-kernel-tinkuy`: invoca sims desde una celda notebook, devuelve `OutputPayload::Image{PNG visor}` + observables como texto.

## Reglas vivas

- Capas estrictamente secuenciales — no abrir D antes de cerrar C, ni E antes de D.
- Cada sub-fase cierra con commit + push (`feat(tinkuy-…)`/`feat(tinkuy-wasm)`/etc), pathspec explícito.
- `cargo check --workspace` no debe romperse jamás por trabajo en tinkuy.
- Compatibilidad Wawa: `tinkuy-core` con `--features wasm --no-default-features` debe compilar a `wasm32-unknown-unknown` sin `std::net`, `std::thread`, `std::time::Instant` (ver `snapshot.rs` y `observables.rs`).
