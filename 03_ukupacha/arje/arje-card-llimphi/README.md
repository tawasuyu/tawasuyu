# arje-card-llimphi

Card de escritorio (Llimphi) con el **estado vivo del init `arje`**. Es la
"card escritorio (estado de arje)" que el README de arje promete; `arje-card`
nunca lo fue (quedó como alias de tipos de `card-core`).

```sh
cargo run -p arje-card-llimphi
```

## Qué muestra

Seis secciones, refrescadas por polling cada 2 s (mismo patrón que
`minga-explorer-llimphi` / `nakui-explorer-llimphi`):

| Sección | Fuente | Requiere daemon |
|---|---|---|
| **Aislamiento** | `arje_incarnate::caps::CapabilitySet::detect()` — namespaces creables (N/7) | no (solo `/proc`) |
| **Privilegios** | idem — `CAP_SYS_ADMIN`, user-ns, `max_user_namespaces` | no |
| **cgroups** | idem — v2 unified/híbrido/legacy + delegación + ruta | no |
| **Unidades** | card store (`$ARJE_CARDS_DIR`, default `/etc/arje/cards.d`); cada `<n>.json` parseado con `card_core::Card` | no (filesystem) |
| **Brain** | socket introspect — reglas vivas + entropía/muestras/tipos de evento | sí (brain) |
| **Audit log** | socket introspect — seq del head + últimas 6 entradas | sí (brain) |

Las cuatro primeras siempre están disponibles (la misma rutina que corre
`Incarnator::new` antes de encarnar una Card, más una lectura del store). Las
dos del brain se consultan por su socket; si el brain no corre, la card degrada
a un banner "brain no disponible" y el resto sigue sirviendo.

La caps **no se cachea**: sysctl/LSM/cgroup-delegation cambian entre boots (a
veces en caliente), por eso se re-detecta en cada tick.

## Acciones

- **Verificar audit** (header, solo con brain vivo): pide `VerifyAudit` al brain
  (recorre la cadena `prev_sha` hasta el génesis validando cada entry contra el
  CAS) y muestra el resultado en un banner. Read-only.

## Socket del brain

`$ENTE_BRAIN_SOCK`, o `$XDG_RUNTIME_DIR/ente-brain.sock` (fallback `$TMPDIR`,
`/tmp`) — misma convención que `arje-zero` y `brainctl`.

## No incluido a propósito

- **GC del CAS** (`GcCas`): destructivo — borra todo blob no alcanzable desde el
  head del audit salvo los pasados en `extra_roots`. Sin los hashes WASM de las
  Cards eso borraría apps vivas. El GC correcto lo posee el kernel/brain, no un
  dashboard de monitoreo.
- **Stream del audit** (`StreamAudit`): los dashboards del repo usan polling 2 s;
  el audit ya se refresca a ese ritmo. Un stream con hilo reconectante sería la
  única excepción no idiomática.

Reactivo a `wawa-config` (theme/accent).
