# chaka-shadow

> Shadow validation of the [chaka](../README.md) pipeline.

Two independent execution paths for the same COBOL source:

- **In-process interpreter** (`interpret(&Ir)` / `run_source(&str)`): walks the IR directly over `chaka-runtime` types without compiling anything. This is the fast path used by `chaka run` and by the corpus tests.
- **GnuCOBOL harness** (`cobc::compare_with_cobc(source)`): compiles the source with `cobc -x -free`, runs the binary with a timeout, and returns both stdouts side-by-side so the caller can diff them. Opt-in: requires `cobc` on the `PATH`; tests that depend on it are `#[ignore]` by default.

If the interpreter and the transpiled code diverge, `chaka-codegen` has a bug; if the interpreter and `cobc` diverge, the **interpreter** has a bug. Both halves are needed.

## API

```rust
use chaka_shadow::{interpret, run_source, Outcome};

let outcome: Outcome = run_source(cobol_source)?;
for line in &outcome.lines {
    println!("{line}");
}

// Validación opt-in contra GnuCOBOL:
use chaka_shadow::cobc;
if cobc::is_available() {
    let report = cobc::compare_with_cobc(cobol_source)?;
    assert!(report.matches());
}
```

## Out of scope (v1)

- Production "shadow deployment" with timeouts, retry budgets and divergence dashboards — the original plan. Today the shadow is a developer-time validator, not a production harness.

## Deps

- [`chaka-ir`](../chaka-ir/README.md), [`chaka-lexer`](../chaka-lexer/README.md), [`chaka-parser`](../chaka-parser/README.md), [`chaka-runtime`](../chaka-runtime/README.md).
- `thiserror` for errors. No async deps — the `cobc` harness uses `std::process::Command` with a polling timeout.
