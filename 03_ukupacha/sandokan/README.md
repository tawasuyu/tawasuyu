# sandokan — the control plane, incarnated

**Run, stop, watch and supervise units on Linux — through one contract, over any transport.**

*Leé esto en español: [LEEME.md](LEEME.md).*

[`shared/sandokan`](../../shared/sandokan/README.md) defines *what* a
control plane is (the `Engine` contract, the life policies, the read-only
`observe()` face). This folder is *how* it actually happens on Linux: four
engine flavors — one per transport — plus the CLI and the visual monitor.
Any binary (a CLI, the shuma shell, the monitor) embeds sandokan and just
picks where its units should run.

## The crates

| Crate | Role |
|---|---|
| `sandokan` | Umbrella: re-exports everything + `Engine::auto()`, which picks the best available transport by the SDD's precedence (init → daemon → local). |
| `sandokan-arje-engine` | `ArjeEngine`: speaks `arje-bus` to PID 1 — control rides the init's own bus, no parallel socket. |
| `sandokan-daemon` | `DaemonEngine` (client) + `serve()` (server) over a Unix socket, postcard length-prefixed. Tier 2 when you're not the init. |
| `sandokan-local` | `LocalEngine`: in-process. Incarnates Cards via `arje-incarnate`, tracks lifecycle, reaps lazily (`waitpid WNOHANG`). |
| `sandokan-remote` | `RemoteEngine`: tunnels the daemon wire over SSH (direct-streamlocal) to a remote host. |
| `sandokan-cli` | The `sandokan` binary: daemon / run / list / status / telemetry / stop. |
| `sandokan-monitor-llimphi` | The `sandokan-monitor` app: system tab (/proc tree + treemap), units tab (via `Engine`, CPU sparklines, real controls: terminate/kill/pause/follow), wawa tab. |

## Try it

The visual monitor, with seeded units so there is something to look at:

```bash
SANDOKAN_MONITOR_SEED=1 cargo run -p sandokan-monitor-llimphi --release
```

The CLI against a local daemon:

```bash
cargo run -p sandokan-cli -- daemon              # terminal 1
cargo run -p sandokan-cli -- run /bin/sleep 300  # terminal 2
cargo run -p sandokan-cli -- list
cargo run -p sandokan-cli -- status    <card-id>
cargo run -p sandokan-cli -- telemetry <card-id>
cargo run -p sandokan-cli -- stop      <card-id>
```

## Status

Core green, transports solid. Monitor phase 3 done (2026-06): three working
tabs, hierarchical treemap, per-unit CPU sparklines, real signal controls.
Pending: phase 4 (live executor + wawa compositor beacons) and arbitrary
`RunCard` (gated behind `Capability::Spawn`, in progress).
