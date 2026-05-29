# tinkuy-dsl

> DSL matemático para fuerzas pairwise — Capa 3 de [tinkuy](../README.md).

Lexer + parser Pratt → AST → bytecode stack-machine → optimizer fold/algebraico. Pensado para describir una fuerza `F_over_r(r, r2, dx, dy, dz, eps, sigma, qi, qj, mi, mj)` sin recompilar Rust — la misma expresión que vive en un archivo `.tnk`, emite un grafo de nodos visuales o produce el editor de nodos de [`tinkuy-llimphi`](../tinkuy-llimphi/).

`#![no_std] + alloc` para que una herramienta futura del userspace de Wawa pueda JIT-ear una fuerza dentro del scratch del kernel y dársela a `tk_sim_step_*`.

## Pipeline

```rust
use tinkuy_dsl::{parse, optimize, compile, eval_with_stack, VarBindings};

let ast = parse("24 * eps * (2 * pow(sigma / r, 12) - pow(sigma / r, 6)) * inv(r2)")?;
let bc  = compile(&optimize(ast))?;
let mut stack = [0.0f32; 32];
let f_over_r = eval_with_stack(&bc, &VarBindings { r, r2, eps, sigma, .. }, &mut stack)?;
```

## Módulos

- `lib.rs` — AST `Var` / `BinOp` / `Func` / `Expr`, `lex`, `parse`.
- `bytecode.rs` — `Bytecode { code, consts, stack_depth }` + `compile` + `eval_with_stack` (cero-alloc; pool de constantes dedupado por `to_bits()`).
- `optimize.rs` — fold + simplificación algebraica a fix-point (`x*1`, `x+0`, `pow(_,0/1)`, `inv(inv(x))`, `-(-x)`, …).

## Ejemplos

`examples/*.tnk` — `lj.tnk`, `coulomb.tnk`, `hooke.tnk`. Acepta comentarios `#`. Verificado por el test de integración `example_tnk_files_all_compile`.

## Benches

`benches/optimize.rs` — medido el 2026-05-29 (criterion 0.5):

| fórmula  | raw       | opt       | speedup |
| -------- | --------- | --------- | ------- |
| LJ       | 13.2 Me/s | 17.3 Me/s | ×1.31   |
| Coulomb  | 35.8 Me/s | 35.5 Me/s | ×1.00   |
| Hooke    | 46.7 Me/s | 68.6 Me/s | ×1.47   |

Correr con `cargo bench -p tinkuy-dsl`.

## Deps

- (ninguna — `#![no_std] + alloc` puro)
