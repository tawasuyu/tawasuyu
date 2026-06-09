# sandokan (shared) — el contrato del plano de control

**Una sola fuente de verdad sobre quién arranca, para, supervisa y observa unidades.**

*Read this in English: [README.md](README.md).*

Todo sistema cría cuatro oficios alrededor de sus procesos:
*materializarlos*, decidir su *política de vida* (restarts, backoff,
cuotas), exponer un *contrato de control* (run/stop/status), y mover esas
órdenenes por algún *transporte*. Cuando cada app inventa su propia versión
de esos cuatro oficios, terminás con lógica de control duplicada y
levemente contradictoria por todos lados.

sandokan es la respuesta de tawasuyu: un único plano de control, definido
acá como **contratos puros, sin transporte**, implementado en otro lado
para cada mundo (Linux hoy, el SO wawa en paralelo por diseño — wawa
comparte sólo el crate `no_std` `format` y el DAG direccionado por
contenido, nunca tipos POSIX).

## Los crates

| Crate | Rol |
|---|---|
| `sandokan-core` | El contrato: trait `Engine { run, stop, list, status, telemetry }` más `Intent`, `ExecHandle`, `LifecycleEvent`, `TelemetryFrame`. Sin transporte, sin llamadas al SO. |
| `sandokan-lifecycle` | Política de vida pura: `Backoff` exponencial, TTLs, cuotas de recursos, `RestartPolicy` / `RestartTracker`, `LifecycleState`. Agnóstico de procesos y de UI. |
| `sandokan-monitor-core` | La cara de sólo lectura: `observe(&dyn Engine) -> MonitorSnapshot` — para que los monitores consuman el mismo contrato en vez de inventar una fuente de datos paralela. |

Los cuatro roles del plano de control mapean, en Linux, a: `arje-incarnate`
(materializar), `sandokan-lifecycle` (política), `sandokan-core::Engine`
(contrato), `arje-bus` (transporte). Los engines concretos y las apps viven
en [`03_ukupacha/sandokan`](../../03_ukupacha/sandokan/LEEME.md).

El diseño completo — incluyendo el libro de deduplicaciones y el lado wawa —
está en [SDD.md](SDD.md), que es autoritativo.

## Probalo

Estos crates son contratos y lógica pura; acá no hay binario.

```bash
cargo test -p sandokan-core
cargo test -p sandokan-lifecycle
cargo test -p sandokan-monitor-core
```

Para algo visible, corré la app del monitor desde el lado de las
implementaciones:

```bash
SANDOKAN_MONITOR_SEED=1 cargo run -p sandokan-monitor-llimphi --release
```

## Estado

Verde. El dedup #1 (arje-zero adopta `Backoff`) y el núcleo del dedup #3
(arje-bus habla `EnteStatus`/`EnteTelemetry`) están hechos; el camino
`observe()` del monitor funciona end-to-end con restarts visibles.
Pendiente: `RunCard` arbitraria (en curso) y el lado wawa del monitor.
