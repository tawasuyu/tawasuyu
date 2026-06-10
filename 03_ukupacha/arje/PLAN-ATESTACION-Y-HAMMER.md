# PLAN — Atestación al arranque + coordinación arje ↔ hammer

> Estado: propuesta (2026-06-10). Aterriza dos cosas que salieron de revisar la
> recomendación de 5 puntos para arje contra el código real: **(A)** el único gap
> genuino de esos 5 — atestación de integridad al boot vía los primitivos de agora —
> y **(B)** el límite con [`hammer`](https://gitea.gioser.net/sergio/hammer), que
> converge en los mismos primitivos por otro camino.

## 0. Por qué este plan

La recomendación de 5 puntos (seeds binarios, absorción glibc, red+aduana, CAS+snapshots,
WASM init) describe casi crate por crate lo que arje **ya tiene** (`arje-packager`,
`arje-absorb`, `arje-net-bring-up`+`arje-brain`, `arje-cas`+`arje-snapshot`,
`arje-soma`+`arje-wasm`). El estado de arje los marca *Hecho*. De los cinco, sólo uno
tiene un gap real verificado en el código:

- **`arje-brain-audit` verifica la cadena de decisiones del brain** (`verify_chain_from_cas`),
  no los **binarios vivos** de `/bin` contra el seed antes de levantar el entorno.

Eso es atestación al arranque, y es exactamente la *raíz-de-confianza-ejecutable* que
agora ya construyó (WAWA §14.1.3). No hay que inventar criptografía: hay que **cablear**.

En paralelo, hammer (distro Linux AI-nativa, fases 0–6 cerradas sobre Alpine) tiene en su
roadmap un *"track posterior — distro propia"* cuyo primer ítem es: *"reemplazar el init de
Alpine por **tu init** (bus por pipes nativo) — habilita el `CRASHED` real que la Fase 5
dejó diferido."* **Ese init es arje** (PID 1 con supervisión real: `RestartTracker`,
`sandokan-lifecycle::Backoff`, restarts visibles end-to-end). Los dos proyectos son
complementarios; este plan fija la frontera para que no colisionen.

---

## A. Atestación al arranque

### A.1 Modelo

La Seed Card (`card-core::Card`, ya transportada por `arje-bus` en postcard) gana un
manifiesto de integridad: por cada binario crítico, una **`ConcesionCapacidad` firmada**
sobre `(blake3(binario), permisos)` bajo la clave raíz del seed. Es el mismo tipo que agora
y el kernel wawa ya verifican — no un formato nuevo.

```
seed.card.json
  └── attest: [ ConcesionCapacidad { autor: rootkey, bytecode: b3(/sbin/arje-zero), permisos, firma }, … ]
```

`arje-zero`, tras montar el bus y **antes de incarnar el target gráfico**, computa el BLAKE3
de cada binario crítico y lo verifica:

```rust
// reuso directo, cero criptografía nueva:
agora_channel::verificar_capacidad(&c)?;          // firma cubre mensaje_capacidad(bytecode, permisos)
//   └─ internamente: agora_core::verify_signature(&c.autor, &mensaje, &c.firma)
```

Resultado de cada verificación → `AuditEntry` en `arje-brain-audit` (queda en la cadena
anclada al CAS, auditable con `verify_chain_from_cas`). Si un binario crítico no casa:
política del seed decide **`halt`** (no levantar GUI) o **`degraded`** (levantar, marcar la
unidad comprometida en el brain y avisar a la shell). Por defecto `halt` para los binarios de
arranque, `degraded` para el resto.

### A.2 Reuso exacto (qué ya existe)

| Necesito | Ya existe | Dónde |
|---|---|---|
| Verificar firma Ed25519 | `agora_core::verify_signature(&[u8;32], &[u8], &[u8;64])` | `agora-core/src/identity.rs:125` |
| Verificar concesión `(hash, permisos)` | `agora_channel::verificar_capacidad(&ConcesionCapacidad)` | `agora-channel/src/lib.rs:211` |
| Espejo `no_std` (bare-metal wawa) | `verificar_concesion_capacidad` | `wawa-kernel/src/claves.rs:416` |
| Tipos de capacidad/permiso del seed | `Capability`, `Permissions` | `card-core/src/lib.rs:217,266` |
| Cadena de audit anclada al CAS | `arje-brain-audit::{AuditLog, verify_chain_from_cas}` | `runtime/arje-brain-audit/src/lib.rs` |

### A.3 Prerrequisito: alinear el hash del CAS

`arje-cas` hashea hoy con **SHA-256** (`sha256_of`); hammer, `shared/format` y el kernel wawa
usan **BLAKE3**. La atestación tiene que hablar el mismo hash que el `expected_hash` de un
`.swm` de hammer y que `mensaje_capacidad`. **Migrar `arje-cas` a BLAKE3** (la API es chica:
`store/resolve/list_all_shas/gc` + `sha256_of`→`blake3_of`). Riesgo bajo, hito previo a A.

### A.4 Punto de inserción

`init/arje-zero/src/main.rs`: tras `bus::spawn_bus(...)` (~L254) y antes del primer `RunCard`
del target gráfico. La verificación es síncrona y rápida (BLAKE3 sobre un puñado de binarios).

### A.5 Fases

1. **A0** — `arje-cas` → BLAKE3 (prerrequisito).
2. **A1** — Campo `attest: Vec<ConcesionCapacidad>` en la Seed Card + firmador en `arje-packager`
   (firma las concesiones al empaquetar con la rootkey del seed).
3. **A2** — Gate en `arje-zero`: verificar antes del target gráfico, emitir `AuditEntry`,
   aplicar política `halt`/`degraded`.
4. **A3** — Card de escritorio (`arje-card-llimphi`): mostrar el veredicto de atestación por
   unidad (verde/comprometido) en el panel del brain que ya existe.

---

## B. Límite arje ↔ hammer

### B.1 Responsabilidades (regla de oro: PID 1 fino)

| Dominio | Dueño | Por qué |
|---|---|---|
| Boot, PID 1, kernel/loader, instalación | **arje** | Es su carta (`arje` = *boot, not governing the running system*) |
| Supervisión de servicios / restart | **arje** | `RestartTracker`+`Backoff` → **entrega el `CRASHED` real que hammer Fase 5 difirió** |
| Mount del overlay (lowerdir RO / upperdir) | **arje** | El init monta; ver A.3, mismo CAS |
| Snapshot / CAS | **arje** | `arje-cas`+`arje-snapshot` (BLAKE3 tras A0) |
| Atestación al arranque | **arje** | §A |
| Build determinista (bubblewrap+zig) | **hammer** | `hammer-build`, su laboratorio hermético |
| Diario de mutaciones (`fanotify`) | **hammer** | `hammerd` — es FS-watch, **no** control de red |
| `.swm` reproducible + firma + TrustStore | **hammer** | `hammer-core`, modelo "reproducir, no confiar" |
| Bucle agéntico IA (intent→overlay→propose) | **hammer** | `hammer-agent` |

> Nota técnica que corrige la recomendación: el punto 3 ("aduana" con `fanotify` que bloquea
> puertos) confunde dos mecanismos. `fanotify` observa **filesystem** (es lo que usa el diario
> de hammer), **no** bloquea egress. Política de puertos/syscalls = eBPF/nftables/seccomp/LSM,
> y en Linux es una aproximación *blanda* a la frontera física de capabilities de wawa. Eso vive
> en `arje-brain` como política, no como `fanotify`.

### B.2 Un solo bus

Hoy hay dos buses de agente con la misma semántica (socket Unix + `SO_PEERCRED`):
`arje-bus` (postcard) y el `agent.sock` de hammer (JSON-líneas). **Decisión:** hammerd corre
como **Ente supervisado por arje** y su control de ciclo de vida va por `arje-bus`. El
`agent.sock` (JSON-líneas) de hammer **no es un segundo plano de control del init**: es la API
de IA de alto nivel *encima*. Los tipos de protocolo de hammer (`hammer-core::proto`) se
comparten; el wire de transporte es `arje-bus`. Un solo `SO_PEERCRED`, una sola política de
capacidades.

### B.3 Modelo de confianza en capas (el puente elegante)

Los dos modelos no compiten, se encadenan:

- **hammer garantiza procedencia:** un binario se *reproduce desde fuente pública* y su
  `expected_hash` (BLAKE3) casa → "vino de este código, no de tu disco".
- **arje/agora atesta autorización:** `verificar_capacidad` sobre `(blake3, permisos)` → "el
  binario que corre al boot es el autorizado por la rootkey del seed".

El nexo es el hash: **el `expected_hash` de un `.swm` de hammer ES el BLAKE3 que arje atesta.**
Flujo conjunto: un `hammer commit` promovido emite una `ConcesionCapacidad` firmada que
`arje-absorb` integra al seed → el binario que la IA mutó queda **atestado en el próximo boot**.
Así el ciclo "IA propone → humano commitea (hammer)" se cierra con "init atesta (arje)" sin
que ninguno de los dos sepa de criptografía del otro.

### B.4 Roadmap conjunto

- hammer *Track posterior → init propio* = **adoptar arje**. arje entrega el `CRASHED` real
  (supervisión) que la Fase 5 de hammer dejó diferido.
- `arje-cas` → BLAKE3 (A0) desbloquea el CAS compartido (hammer ya usa prefijo `b3:`).
- Bus unificado (B.2) antes de que hammerd corra bajo arje.

### B.5 Caveat estratégico (no diluir el norte)

El punto 2 de la recomendación (cage glibc para Steam/NVIDIA) empuja hacia la tesis
*pragmática-Linux* de hammer (Alpine-first, musl, FHS clásico), que es el **vector opuesto** al
self-hosting de wawa. arje bootea ambos kernels (*"natural bootloader for wawa-kernel"* **y**
*"Linux x86_64 primary"*), así que no hay contradicción — pero **la cage glibc es feature del
mundo hammer/Linux, no de arje core**. Meterla en PID 1 ensucia el init y traiciona el norte
wawa.

---

## Coordinación

Contraparte en hammer: [`docs/adr/0007-arje-como-init-propio.md`](https://gitea.gioser.net/sergio/hammer)
(decisión de adoptar arje como init, bus único, CAS BLAKE3, confianza en capas) + puntero en
`docs/10-roadmap.md` §Track posterior.
