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

## C. Integrar el workspace tawasuyu en el lab de hammer

Que arje sea el init es sólo el piso. "tawasuyu en hammer" significa que el laboratorio
determinista de hammer **construya las apps de tawasuyu** y que **corran en su userland**.
Frentes verificados contra el código (2026-06-10):

### C.1 El muro — build Rust + GPU dinámico

1. **hammer no construye Rust.** `hammer-build` detecta `autotools/cmake/meson/make` y compila C
   con `zig cc`; **no hay `BuildSys::Cargo`**. Prerrequisito #0: vía cargo en el lab (recipe =
   repo+commit fijado → `cargo build --release` con target y toolchain pinneados).
2. **musl-estático (la vía de oro de hammer) no sirve para lo gráfico.** Verificado: Llimphi =
   `wgpu`+`winit`+`vello` (→ `libvulkan`/`libEGL`/`libwayland`/`libxkbcommon`); mirada =
   `smithay 0.7` (→ `libdrm`/`libinput`/`libseat`/`libgbm`/`libudev`). Todo C dinámico. El
   front-end gráfico va por la **vía secundaria** de hammer (`patchelf` + core dinámico curado,
   SDD 03 §4–5), invirtiendo el "static por defecto". Hay que curar y **versionar** ese core
   gráfico.
3. **Decisión abierta: musl vs glibc para la capa gráfica.** Mesa/Vulkan y sobre todo NVIDIA
   propietario asumen glibc (NVIDIA ya es pendiente en mirada). Choca con el caveat de la cage
   glibc (§B.5). Probable resolución: **el 80% no-gráfico va musl-estático; el 20% gráfico vive
   en un sub-mundo glibc-dinámico curado.** No es detalle: es bifurcación de arquitectura. ⚠️ A
   decidir.

### C.2 Toolchain y reproducibilidad

4. **No hay `rust-toolchain.toml` en la raíz** (wawa nightly, resto stable). hammer mete el
   toolchain en el hash → pin explícito por recipe o no hay determinismo.
5. **cargo no es reproducible bit-a-bit gratis:** `--remap-path-prefix`, `SOURCE_DATE_EPOCH`,
   orden de paralelismo, rutas del registry. `Cargo.lock` ya está fijado; falta el resto.

### C.3 La unidad de empaque ya está bien encaminada

hammer compila desde **repo público + commit fijado**, no desde un monorepo. Los front-doors
standalone ya extraídos (`llimphi`, `mirada` publicados con commit; `nahual`+`shuma` por git-dep)
**son las unidades naturales de recipe**. Alinear la estrategia de extracción con el modelo
recipe (cada front-door = un `.toml` con su pin).

### C.4 Runtime / userland — no duplicar supervisión

- Init = arje (§B, ADR 0007).
- **Supervisión:** hammer modela servicios como `/etc/service/<name>/run` (s6/runit); tawasuyu
  ya tiene **sandokan** como plano de control sin duplicados. Decisión: **sandokan+arje SON el
  supervisor**; el `/etc/service/run` de hammer mapea a Cards de arje. No coexisten tres.
- Red: card-net/libp2p (TCP/QUIC) corre nativo en Linux ✅. Storage sled = Rust puro ✅.

### C.5 Fronteras (qué NO integra)

- **wawa** (bare-metal `x86_64-none`) no es app de userland-hammer; lo *bootea* arje, es otro
  track. No confundir "tawasuyu en hammer" con wawa. La landing wasm no aplica.

### C.6 Milestones (secuencia de riesgo creciente)

1. **M1 — vía Rust con lo fácil:** una app **no gráfica** (`agora-cli` / `sandokan-daemon` /
   un daemon CLI) → cargo + musl-estático + commit fijado. Prueba el path end-to-end barato.
   ← *en curso, ver C.7.*
2. **M2 — primera gráfica:** un `example` de Llimphi por la vía dinámica → paga la deuda del
   core gráfico curado y materializa la decisión musl/glibc (#3).
3. **M3 — mirada:** lo más pesado (DRM/seat/input).

### C.7 Bitácora del experimento M1 — ✅ sale limpio (2026-06-10)

Candidato: **`agora-cli`** (CLI no gráfico: identidades/atestaciones/grafo). Target
`x86_64-unknown-linux-musl`, linker `musl-gcc` (sin zig). Resultado: **build OK, binario corre**.

- **Una sola fricción, y fue reveladora:** `wawa-explorer-aoe` (arrastrado por agora-cli para el
  transporte AoE) pasaba `libc::SIOCGIFINDEX`/`SIOCGIFHWADDR` a `ioctl`. El `request` de `ioctl`
  es **`c_ulong` (u64) en glibc** pero **`c_int` (i32) en musl** → `error[E0308]`. Es la clase de
  divergencia musl/glibc que M1 debía destapar. Fix portable: `… as _` (infiere el tipo por
  target, no rompe glibc). Verificado: `cargo check -p wawa-explorer-aoe` en el target gnu por
  defecto sigue en exit 0.
- **Binario:** 2.2 MB, `ldd` → *statically linked* (cero deps de `.so`). Es un **static-PIE**
  (pide `/lib/ld-musl-x86_64.so.1` como loader pero sin librerías dinámicas). Para el
  estático-clásico-sin-interpreter de la vía de oro de hammer se desactiva PIE
  (`-C relocation-model=static` / `target-feature=+crt-static`) — detalle de config, no bloqueante.
- **Lecciones para la recipe Rust de hammer:**
  1. El path cargo→musl-estático **funciona** para el tier no gráfico; el trabajo de hammer es
     añadir `BuildSys::Cargo` (C.1 #1), no pelear con el linker.
  2. El código con `libc`/raw-sockets tiene asunciones glibc latentes (ioctl, tipos de `request`,
     anchura de constantes). Auditar `unsafe { libc::… }` al portar es parte del costo M1→M2.
  3. Pin de linker (`musl-gcc` o `zig cc`) y de toolchain entran al hash de la recipe (C.2).

Próximo: **M2** (un `example` de Llimphi por la vía dinámica) — ahí se materializa la decisión
musl/glibc del tier gráfico (#3).

---

## Coordinación

Contraparte en hammer: [`docs/adr/0007-arje-como-init-propio.md`](https://gitea.gioser.net/sergio/hammer)
(decisión de adoptar arje como init, bus único, CAS BLAKE3, confianza en capas) + puntero en
`docs/10-roadmap.md` §Track posterior.
