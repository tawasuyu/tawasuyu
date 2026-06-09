# sandokan — el plano de control, encarnado

**Correr, parar, mirar y supervisar unidades en Linux — por un solo contrato, sobre cualquier transporte.**

*Read this in English: [README.md](README.md).*

[`shared/sandokan`](../../shared/sandokan/LEEME.md) define *qué* es un
plano de control (el contrato `Engine`, las políticas de vida, la cara de
sólo lectura `observe()`). Esta carpeta es *cómo* ocurre de verdad en
Linux: cuatro sabores de engine — uno por transporte — más la CLI y el
monitor visual. Cualquier binario (una CLI, el shell shuma, el monitor)
embebe sandokan y simplemente elige dónde deben correr sus unidades.

## Los crates

| Crate | Rol |
|---|---|
| `sandokan` | Umbrella: re-exporta todo + `Engine::auto()`, que elige el mejor transporte disponible por la precedencia del SDD (init → daemon → local). |
| `sandokan-arje-engine` | `ArjeEngine`: habla `arje-bus` con el PID 1 — el control viaja por el bus propio del init, sin socket paralelo. |
| `sandokan-daemon` | `DaemonEngine` (cliente) + `serve()` (servidor) sobre socket Unix, postcard con prefijo de largo. Tier 2 cuando no sos el init. |
| `sandokan-local` | `LocalEngine`: in-process. Encarna Cards vía `arje-incarnate`, trackea lifecycle, cosecha perezoso (`waitpid WNOHANG`). |
| `sandokan-remote` | `RemoteEngine`: tunela el wire del daemon sobre SSH (direct-streamlocal) hacia un host remoto. |
| `sandokan-cli` | El binario `sandokan`: daemon / run / list / status / telemetry / stop. |
| `sandokan-monitor-llimphi` | La app `sandokan-monitor`: pestaña sistema (árbol /proc + treemap), pestaña unidades (vía `Engine`, sparklines de CPU, controles reales: terminar/matar/pausar/seguir), pestaña wawa. |

## Probalo

El monitor visual, con unidades sembradas para que haya algo que mirar:

```bash
SANDOKAN_MONITOR_SEED=1 cargo run -p sandokan-monitor-llimphi --release
```

La CLI contra un daemon local:

```bash
cargo run -p sandokan-cli -- daemon              # terminal 1
cargo run -p sandokan-cli -- run /bin/sleep 300  # terminal 2
cargo run -p sandokan-cli -- list
cargo run -p sandokan-cli -- status    <card-id>
cargo run -p sandokan-cli -- telemetry <card-id>
cargo run -p sandokan-cli -- stop      <card-id>
```

## Estado

Núcleo verde, transportes sólidos. Fase 3 del monitor cerrada (2026-06):
tres pestañas funcionando, treemap jerárquico, sparklines de CPU por
unidad, controles reales por señal. Pendiente: fase 4 (executor en vivo +
balizas del compositor de wawa) y `RunCard` arbitraria (gateada por
`Capability::Spawn`, en curso).
