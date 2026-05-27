# chaka-shadow

> Shadow mode of [chaka](../README.md): runs legacy + new in parallel and compares.

The operator deploys the new path in production **alongside** the legacy: both pipelines receive the same input, the shadow captures both outputs, diffs them and reports divergence. When divergence reaches zero over an operator-defined window, the legacy can be safely turned off.

## API

```rust
use chaka_shadow::{ShadowRun, Report};

let report: Report = ShadowRun::new(legacy_path, new_path)
    .input(input_bytes)
    .timeout(Duration::from_secs(30))
    .compare()?;
```

## Deps

- [`chaka-runtime`](../chaka-runtime/README.md) to run the new version
- `tokio` or `std::thread` to parallelize
