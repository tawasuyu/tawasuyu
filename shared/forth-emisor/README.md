# forth-emisor — Forth→WASM compiler for wawa

Takes a **Forth** dialect and emits a valid **WASM** module. Reusable `no_std`
core: it is consumed by the wawa apps pipeline (`build-pluma.sh`, etc.) to
write low-level logic in Forth and run it in the kernel's WASM cage.

## Capabilities

- Word and macro definition.
- `(i32) -> i32` ABI.
- Cascade injection through imported macros (Phase 40).

## Status (2026-05-31)

### Done
- Emission of valid WASM modules from the Forth dialect.
- Macros + `(i32) -> i32` ABI + cascade injection across imported macros (Phase 40).
- `no_std` core (crosses into the wawa pipeline); ≈9 tests.

### Pending
- Richer ABI (multiple args/returns, non-i32 types).
- Optimization of the emitted WASM (today it relies on `wasm-opt` downstream).
- Finer compilation diagnostics/errors.

## Place in the repo

`shared/forth-emisor` — extracted from the wawa kernel (Phase 30). Consumed by the
WASM apps pipeline of `03_ukupacha/wawa`.
