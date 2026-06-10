# arje-card-llimphi

Desktop card (Llimphi) with the **live state of the `arje` init**. It is the
"desktop card (arje state)" that arje's README promises; `arje-card` never was
(it stayed as a type alias of `card-core`).

```sh
cargo run -p arje-card-llimphi
```

## What it shows

Six sections, refreshed by polling every 2 s (same pattern as
`minga-explorer-llimphi` / `nakui-explorer-llimphi`):

| Section | Source | Requires daemon |
|---|---|---|
| **Isolation** | `arje_incarnate::caps::CapabilitySet::detect()` — creatable namespaces (N/7) | no (only `/proc`) |
| **Privileges** | same — `CAP_SYS_ADMIN`, user-ns, `max_user_namespaces` | no |
| **cgroups** | same — v2 unified/hybrid/legacy + delegation + path | no |
| **Units** | **live** via `Engine` over arje-bus (`sandokan-monitor-core` + `sandokan-arje-engine`): real state + telemetry. With no reachable bus, falls back to the static scan of the card store (`$ARJE_CARDS_DIR`) | no (Engine if there is a bus; otherwise, filesystem) |
| **Brain** | introspect socket — live rules + entropy/samples/event types | yes (brain) |
| **Audit log** | introspect socket — head seq + last 6 entries | yes (brain) |

The first four are always available (the same routine that `Incarnator::new`
runs before incarnating a Card, plus a read of the store). The two brain ones
are queried over its socket; if the brain is not running, the card degrades to a
"brain unavailable" banner and the rest keeps serving.

The caps are **not cached**: sysctl/LSM/cgroup-delegation change between boots
(sometimes hot), which is why they are re-detected on every tick.

## Actions

- **Verify audit** (header, only with a live brain): asks the brain for
  `VerifyAudit` (walks the `prev_sha` chain back to genesis, validating each
  entry against the CAS) and shows the result in a banner. Read-only.

## Brain socket

`$ENTE_BRAIN_SOCK`, or `$XDG_RUNTIME_DIR/ente-brain.sock` (fallback `$TMPDIR`,
`/tmp`) — same convention as `arje-zero` and `brainctl`.

## Deliberately not included

- **CAS GC** (`GcCas`): destructive — deletes every blob not reachable from the
  audit head except those passed in `extra_roots`. Without the Cards' WASM
  hashes that would delete live apps. The correct GC is owned by the
  kernel/brain, not a monitoring dashboard.
- **Audit stream** (`StreamAudit`): the repo's dashboards use 2 s polling; the
  audit already refreshes at that rate. A stream with a reconnecting thread would
  be the only non-idiomatic exception.

Reactive to `wawa-config` (theme/accent).
