# sandokan (shared) — the control-plane contract

**One source of truth for who starts, stops, supervises and observes units.**

*Leé esto en español: [LEEME.md](LEEME.md).*

Every system grows four jobs around its processes: *materialize* them,
decide their *life policy* (restarts, backoff, quotas), expose a *control
contract* (run/stop/status), and move those orders over some *transport*.
When each app invents its own version of those four jobs, you get
duplicated, slightly-disagreeing control logic everywhere.

sandokan is tawasuyu's answer: a single control plane, defined here as
**pure, transport-free contracts**, implemented elsewhere for each world
(Linux today, the wawa OS in parallel by design — wawa shares only the
`no_std` `format` crate and the content-addressed DAG, never POSIX types).

## The crates

| Crate | Role |
|---|---|
| `sandokan-core` | The contract: trait `Engine { run, stop, list, status, telemetry }` plus `Intent`, `ExecHandle`, `LifecycleEvent`, `TelemetryFrame`. No transport, no OS calls. |
| `sandokan-lifecycle` | Pure life-policy logic: exponential `Backoff`, TTLs, resource quotas, `RestartPolicy` / `RestartTracker`, `LifecycleState`. Agnostic of processes and UI. |
| `sandokan-monitor-core` | The read-only face: `observe(&dyn Engine) -> MonitorSnapshot` — so monitors consume the same contract instead of inventing a parallel data source. |

The four control-plane roles map, on Linux, to: `arje-incarnate`
(materialize), `sandokan-lifecycle` (policy), `sandokan-core::Engine`
(contract), `arje-bus` (transport). The concrete engines and apps live in
[`03_ukupacha/sandokan`](../../03_ukupacha/sandokan/README.md).

The full design — including the deduplication ledger and the wawa side —
is in [SDD.md](SDD.md), which is authoritative.

## Try it

These crates are contracts and pure logic; there is no binary here.

```bash
cargo test -p sandokan-core
cargo test -p sandokan-lifecycle
cargo test -p sandokan-monitor-core
```

For something visible, run the monitor app from the implementation side:

```bash
SANDOKAN_MONITOR_SEED=1 cargo run -p sandokan-monitor-llimphi --release
```

## Status

Green. Dedup #1 (arje-zero adopts `Backoff`) and the core of dedup #3
(arje-bus speaks `EnteStatus`/`EnteTelemetry`) are done; the monitor's
`observe()` path works end-to-end with visible restarts. Pending: arbitrary
`RunCard` (in progress) and the wawa side of the monitor.
