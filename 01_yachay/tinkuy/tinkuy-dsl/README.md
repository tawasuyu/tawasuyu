# tinkuy-dsl

> Math DSL for pairwise forces — Capa 3 of [tinkuy](../README.md).

Lexer + Pratt parser → AST → stack-machine bytecode → fold/algebraic optimizer. Designed to describe a force `F_over_r(r, r2, dx, dy, dz, eps, sigma, qi, qj, mi, mj)` without recompiling Rust — the same expression a `.tnk` file holds, a visual node graph emits, or [`tinkuy-llimphi`](../tinkuy-llimphi/)'s node editor produces.

`#![no_std] + alloc` so a future Wawa userspace tool can JIT a force inside the kernel scratch and feed it to `tk_sim_step_*`.

## Pipeline

```rust
use tinkuy_dsl::{parse, optimize, compile, eval_with_stack, VarBindings};

let ast = parse("24 * eps * (2 * pow(sigma / r, 12) - pow(sigma / r, 6)) * inv(r2)")?;
let bc  = compile(&optimize(ast))?;
let mut stack = [0.0f32; 32];
let f_over_r = eval_with_stack(&bc, &VarBindings { r, r2, eps, sigma, .. }, &mut stack)?;
```

## Modules

- `lib.rs` — `Var` / `BinOp` / `Func` / `Expr` AST, `lex`, `parse`.
- `bytecode.rs` — `Bytecode { code, consts, stack_depth }` + `compile` + `eval_with_stack` (zero-alloc; const pool deduped by `to_bits()`).
- `optimize.rs` — fix-point fold + algebraic simplification (`x*1`, `x+0`, `pow(_,0/1)`, `inv(inv(x))`, `-(-x)`, …).

## Examples

`examples/*.tnk` — `lj.tnk`, `coulomb.tnk`, `hooke.tnk`. `#`-comments allowed. Verified by the `example_tnk_files_all_compile` integration test.

## Benches

`benches/optimize.rs` — measured 2026-05-29 (criterion 0.5):

| formula  | raw       | opt       | speedup |
| -------- | --------- | --------- | ------- |
| LJ       | 13.2 Me/s | 17.3 Me/s | ×1.31   |
| Coulomb  | 35.8 Me/s | 35.5 Me/s | ×1.00   |
| Hooke    | 46.7 Me/s | 68.6 Me/s | ×1.47   |

Run with `cargo bench -p tinkuy-dsl`.

## Deps

- (none — pure `#![no_std] + alloc`)
