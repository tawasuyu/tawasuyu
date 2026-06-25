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

### A.3 Prerrequisito: alinear el hash del CAS — ✅ HECHO (A0)

`arje-cas` hashea hoy con **SHA-256** (`sha256_of`); hammer, `shared/format` y el kernel wawa
usan **BLAKE3**. La atestación tiene que hablar el mismo hash que el `expected_hash` de un
`.swm` de hammer y que `mensaje_capacidad`. **Migrar `arje-cas` a BLAKE3** (la API es chica:
`store/resolve/list_all_shas/gc` + `sha256_of`→`blake3_of`). Riesgo bajo, hito previo a A.

> **Hecho** (A0): `arje-cas` hashea con BLAKE3 (`blake3_of`); el ancho (256 bits) no cambia, así
> que el layout del CAS y la API por hash quedan idénticos — sólo cambia el cómputo. Callers
> migrados: `arje-brain-audit`, `chasqui-nous-real`. `module_sha256` conserva el nombre histórico
> aunque hoy lleva un BLAKE3. Roundtrip cubierto por test; `cargo check` de los dependientes verde.

### A.4 Punto de inserción

`init/arje-zero/src/main.rs`: tras `bus::spawn_bus(...)` (~L254) y antes del primer `RunCard`
del target gráfico. La verificación es síncrona y rápida (BLAKE3 sobre un puñado de binarios).

### A.5 Fases

1. **A0** — `arje-cas` → BLAKE3 (prerrequisito). ✅
2. **A1** — Campo `attest: Vec<ConcesionCapacidad>` en la Seed Card + firmador en `arje-packager`. ✅
   (2026-06-14) `card-core::Card` gana `attest` + `attest_rootkey: Option<AgoraId>` +
   `attest_policy: AttestPolicy{Warn,Degraded,Halt}` (con `WireCard`/From/Default y default seguro
   `Warn`, compat con seeds previos vía `#[serde(default)]`). `arje-packager --rootkey <FILE>`
   (`--gen-rootkey` para crearla desde `/dev/urandom`, 0600) firma una concesión por binario
   crítico sobre su BLAKE3 y la ancla en la seed. La firma vive en el crate nuevo
   **`arje-attest`** (`firmar_binarios`), que reusa `agora_channel::firmar_capacidad` — cero
   criptografía nueva.

   **Paridad de las 3 rutas de seed → ✅ HECHO (2026-06-25).** Al principio sólo `arje-packager`
   (initramfs) sabía firmar; `arje-installer` (ESP/USB) y `arje-absorb` (migración) emitían el
   seed SIN `attest` — hueco: instalar a disco o migrar de otro init no daba boot atestable. El
   firmador y los helpers de rootkey viven ahora en **`arje-attest`** (su hogar natural y lean):
   `firmar_arbol` (firma un `BTreeMap` de binarios, `permisos=0`), `load_or_gen_rootkey`,
   `rootkey_a_hex`, `guia_anclado_soberano`. **Una sola implementación** que comparten las tres
   rutas → el manifiesto es **idéntico** por cualquiera (un binario firmado por una verifica bajo
   las otras). `arje-packager` conserva el azúcar `sign_seed_attest` (firma + muta la Card) y
   re-exporta los helpers para no romper callers. Cada ruta gana `--rootkey`/`--gen-rootkey`:
   - **installer**: firma el seed embebido en el initramfs Y el que escribe a la ESP.
   - **absorb**: firma los binarios de cada servicio **leídos del sistema fuente** (`root/<exec>`)
     — captura su estado confiable al absorber; binarios ausentes se avisan y saltean (survey
     read-only, no aborta). `arje-zero` NO se firma acá (no es un servicio del init absorbido; lo
     agrega el packager/installer al armar la imagen). Es la base del round-trip de B.3.

   Tests: `arje-attest` (`firmar_arbol`), `arje-installer` (lib verifica atestación Ok +
   `to_partition_firma_attest_con_rootkey` e2e por el binario), `arje-absorb`
   (`collect_exec_bins_lee_presentes_y_saltea_ausentes` + verificación), y demo CLI e2e de absorb
   sobre un OpenRC falso. (De paso se arregló `to_partition_arma_layout_completo`, roto desde que
   el seed qemu ganó `arje-splash` el 2026-06-23 y el test no le pasaba el `--bin`.)
3. **A2** — Gate en `arje-zero`: verificar antes del target gráfico, emitir `AuditEntry`,
   aplicar política `halt`/`degraded`. ✅ (2026-06-14) `attest_gate::run` corre tras `spawn_bus` y
   **antes** de `instantiate_seed_dependencies` (el genesis/target): por cada binario crítico
   (genesis Native/Legacy + `arje-zero` mismo) computa el BLAKE3 vivo y lo busca en `attest`
   (`arje_attest::atestar_bytes`); `Halt` aborta el boot ahí, `Warn`/`Degraded` siguen. Los
   veredictos se vuelcan a `AuditAction::AttestationCheck` (cadena anclada al CAS, filtrable con
   `--kind attestation-check`) una vez que el brain existe. `arje-attest` tiene 7 tests del
   núcleo (roundtrip, tampering, re-firma de atacante, firma corrupta, binding por hash); el gate
   3 (no-op, Warn detecta tampering, Halt aborta); `card-core` 2 (roundtrip JSON/wire + compat).
   **Ancla soberana — ✅ HECHO (2026-06-14).** El gate (`attest_gate::ancla_externa`) resuelve una
   rootkey anclada **fuera de la Card**, en orden de confianza: (1) compilada en `arje-zero`
   (`ARJE_ATTEST_ROOTKEY=<hex64>` al build — viaja dentro del binario que el gate también atesta),
   (2) archivo confiable (`/etc/arje/rootkey.pub` o `ARJE_ATTEST_ROOTKEY_FILE`, 32 bytes raw o hex).
   La rootkey efectiva = `ancla.or(seed.attest_rootkey)`: **si hay ancla, manda ella** — las
   concesiones del seed deben estar firmadas por la soberana, no por la que el seed declare. Así un
   seed reescrito por completo que también cambió su `attest_rootkey` cae en `AutorNoConfiable` (la
   firma no es del ancla) → con `Halt` aborta. `arje-attest` gana `rootkey_desde_hex`/`rootkey_a_hex`
   (+test roundtrip); `arje-packager` imprime la pubkey y la receta de anclado. Test estrella:
   `ancla_externa_vence_a_un_seed_reescrito` demuestra que SIN ancla el seed reescrito se auto-valida
   y CON ancla legítima es rechazado. **Resta sólo al operador:** generar la rootkey, anclarla por una
   de las dos vías, y flipear `attest_policy` a `Halt` en su seed — ya no hay hueco de diseño.

   **Dry-run off-boot — ✅ HECHO (2026-06-25).** Flipear a `Halt` y reiniciar a ciegas era el último
   "fallo tonto": si el manifiesto no casa (hash cambió, falta `/etc/arje/rootkey.pub`), la máquina
   se queda en la shell de rescate al arrancar. `arje-zero --attest-check [seed]` corre **el mismo
   gate** (`attest_gate::check` → `gather_verdicts`, idéntico al boot, con `current_exe` y ancla
   soberana) **sin volverse PID 1** y reporta por binario (`✓`/`✗ <motivo>`), de dónde salió el ancla,
   y qué haría cada política; sale con código 1 si algún binario no atesta (gateable en CI). El seed
   sale de la ruta dada o de los candidatos canónicos (`/ente/seed.card.json`, …). Refactor mínimo:
   `run_inner` se partió en `gather_verdicts` (junta veredictos) + aplicación de política, sin tocar
   la semántica de boot; +1 test (`gather_verdicts_junta_sin_abortar_aun_con_halt`). Así el operador
   valida `2 ✓ / 0 ✗` ANTES de endurecer. Verificado end-to-end con un seed firmado por el packager.

   **Persistencia del manifiesto en checkpoint/restore → ✅ HECHO (2026-06-25).** `arje-snapshot`
   guardaba `seed_id`/`seed_label`/`entes` pero NO el `attest`/`attest_rootkey`/`attest_policy` de la
   Semilla raíz, y `seed::load_from_snapshot` los reseteaba con `..Default::default()` → un
   `--checkpoint` seguido de `--restore` dejaba el seed **sin gate** (un sistema bajo `Halt` quedaba
   silenciosamente sin verificar al próximo boot). `FractalSnapshot` gana los tres campos con
   `#[serde(default)]` (compat back/forward con snapshots v1, sin bump de `SNAPSHOT_VERSION`);
   `EnteGraph::snapshot` los puebla desde `self.seed` y `load_from_snapshot` los restaura. (Las Cards
   de `entes` ya llevaban su `attest` por ser `EntityCard` completas; sólo faltaba el de la raíz.)
   El gate corre **incondicionalmente** sobre el seed restaurado (`primordial_loop` no distingue
   restore de boot fresco), así que el manifiesto recuperado se enforza igual. +4 tests:
   `arje-snapshot` round-trip preserva + v1 sin attest carga con defaults; `arje-zero`
   `restore_preserva_la_atestacion_del_seed` (load_from_snapshot devuelve el manifiesto) y
   `gate_enforza_el_manifiesto_de_un_seed_restaurado` (lazo completo: restore → gate da `Ok` con el
   binario intacto y **aborta bajo `Halt`** si se adultera tras el restore).
4. **A3** — Card de escritorio (`arje-card-llimphi`): mostrar el veredicto de atestación por
   unidad (verde/comprometido) en el panel del brain que ya existe. ✅ (2026-06-14) `formatear_entrada`
   renderiza las entradas `AuditAction::AttestationCheck` del audit log en el panel "Audit log" por
   unidad: `atestación <binario> ✓` (verde) o `atestación <binario> ✗ <motivo>` (comprometido). +2
   tests. El panel ya leía `recent_audit` vía `ListAudit`; sólo se enriqueció el formateo, así que
   los veredictos que `arje-zero` vuelca (A2) aparecen en vivo. **Pulido cerrado (2026-06-14):**
   `query_brain` hace una segunda `ListAudit` con `filter.kinds = [AttestationCheck]` (filtro
   server-side, el mismo que `--kind attestation-check`) y `resumir_atestacion` deriva el estado
   **vivo** por binario (deduplicado al `seq` más alto, comprometidos primero) + el conteo N✓/M✗.
   Se pinta como una stat card dedicada "Atestación" (verde si `fail==0`, roja si hay comprometidos),
   arriba del "Audit log". +3 tests (conteo, dedup por re-boot, vacío→None). Degrada a no-mostrar si
   el brain es viejo y no entiende el filtro.

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

**Fuente del `CRASHED` → ✅ cableada (2026-06-14).** `arje-bus` ganó el vocabulario de
observabilidad de ciclo de vida que faltaba: `BusRequest::Subscribe` (anónimo, como
`ListEntes`) + `BusPayload::Event(BusEvent)`. `arje-zero` registra los suscriptores
(`lifecycle_subscribers`) y difunde en `on_death` —el punto único de detección de muerte—
`EnteCrashed{id,label,status}` (exit≠0 o señal), `EnteRestarting{id,label,delay_ms}` (tras
backoff) y `EnteExited{id,label}` (exit 0), purgando perezosamente los suscriptores con el
extremo cerrado. 4 tests en `graph/lifecycle.rs::broadcast_tests`. Del lado hammer, el sink
`hammerd::crashes` (3 tests) traduce la señal normalizada → `Event::Crashed` y la bombea a su
`EventBus` → `agent.sock`. **Adaptador de transporte → ✅ HECHO (2026-06-25).** `hammerd::arje_link`
(thread lanzado por `spawn_if_configured` en `hammerd::main`, gateado por `ENTE_BUS_SOCK`) conecta
al bus de arje, manda `Subscribe`, y traduce cada `BusEvent` → `crashes::Lifecycle` → `Event::Crashed`
→ `EventBus`. **Decisión de wire: relectura del frame postcard** — hammer NO depende de `arje-bus`
(arrastraría arje-card→card-core→…, rompe su build hermético); espeja el subconjunto de suscriptor
con tipos `Deserialize` y discriminantes documentados. El riesgo de ese mirror (drift silencioso si
arje reordena un enum) se cierra con **dos guardas deterministas, sin init vivo:** (1) del lado arje,
`arje-bus::contrato_wire` (2 tests) fija por bytes el frame `Subscribe` (`[00 01 00 0D]`) y los
discriminantes de `BusEvent::EnteCrashed` — si arje reordena, CI falla acá apuntando al espejo; (2)
del lado hammer, `hammerd/tests/arje_link_e2e.rs` levanta un socket falso de arje-bus, valida el
`Subscribe` emitido e inyecta un `EnteCrashed`, verificando que sale como `Event::Crashed` por el
`EventBus` (ruta completa connect→subscribe→leer→traducir→publicar). Contrato verificado contra el
árbol vigente: `BusPayload` 0/1/2, `BusRequest::Subscribe`=13, `BusEvent` 0/1/2, `LifecycleStatus`
`Exited(i32)`/`Killed(i32)`.

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

**Integración de concesiones pre-firmadas → ✅ HECHO (2026-06-25).** `arje-absorb --attest-from
<json>` lee un bundle de `ConcesionCapacidad` ya firmadas (objeto o array — lo que hammer emite),
**verifica la firma de cada una** (`arje_attest::firma_valida`, descarta las inválidas: no confía
a ciegas), deduplica por `bytecode`, y las mergea en `seed.attest`. Reporta cuántas cubren un
binario de servicio del seed vs. «huérfanas» (p. ej. una concesión de `arje-zero`, legítima: el
gate la usa por hash). Combinable con `--rootkey` (firma propia). `reconciliar_anclaje` cierra el
hueco de seguridad de mezclar autores: si no hay `attest_rootkey` y todas comparten autor, lo
ancla (seed auto-consistente); si los autores son mixtos, avisa que bajo `Halt` sólo pasaría el
anclado. Verificado e2e: hammer firma una concesión sobre un binario absorbido → `--attest-from`
la integra → el seed queda anclado al autor de hammer y el binario atesta `Ok`. (`arje-attest`
gana `firma_valida` + el re-export de `ConcesionCapacidad`; un init vivo NO hace falta para esto.)

### B.4 Roadmap conjunto

- hammer *Track posterior → init propio* = **adoptar arje**. arje entrega el `CRASHED` real
  (supervisión) que la Fase 5 de hammer dejó diferido — **fuente, sink y adaptador de transporte
  cableados y validados e2e (B.2)**; el `CRASHED` de hammer ya nace de un Ente que muere bajo arje.
- `arje-cas` → BLAKE3 (A0) desbloquea el CAS compartido (hammer ya usa prefijo `b3:`).
  **Bridge sobre el cable → ✅ HECHO (2026-06-25).** `arje-cas-aoe` pone el CAS compartido sobre
  **Akasha Over Ether**: `servir_cas`/`traer_al_cas`/`anunciar` sobre `wawa_explorer_aoe::ClienteAoE`
  (raw socket, sin reimplementar nada). Funciona porque el invariante `blake3(bytes)==id` es el mismo
  en arje-cas, en AoE y en el kernel wawa → un blob (`.swm`, manifiesto de seed firmado, bytecode)
  viaja content-agnostic y se verifica de punta a punta. Núcleo testeable sin socket
  (`objetos_del_cas` + `absorber` con chequeo de hash); ejemplo `servir_cas` para el operador. Es la
  vía nativa para que un nodo arje/hammer y una wawa intercambien objetos del CAS sin TCP/IP.
  **Cosecha al instalar → ✅ HECHO (2026-06-25).** `arje-installer --harvest-cas` mete al CAS local
  cada binario que instala (`arje_cas::cosechar`, direccionados por su BLAKE3 — el MISMO hash que
  firma la atestación). Cierra el lazo **instalar → CAS → AoE**: tras instalar (y firmar el seed),
  los binarios quedan servibles por `servir_cas`, así un peer que recibió el seed firmado los baja
  por su hash (`traer_al_cas`), los verifica contra el manifiesto, y **reproduce el sistema exacto**
  por la red. Verificado e2e: el binario instala 3 stubs y aparecen como 3 blobs en el CAS aislado.
- Bus unificado (B.2) antes de que hammerd corra bajo arje — vocabulario de eventos ✅; falta
  el cable de transporte arje-bus ↔ hammerd.

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

1. ~~**hammer no construye Rust.**~~ **✅ HECHO** (hammer commit `c5ab7e4`): se agregó
   `BuildSys::Cargo` al lab — detección por `Cargo.toml`, traducción de triple zig→rustc, link
   por wrapper `zig cc` (cierra el `-lgcc_s` de M2), `--offline --locked` para hermeticidad,
   `link=dynamic`→`-crt-static` para el tier gráfico. 5 tests verdes + receta de ejemplo. Falta
   el siguiente eslabón: **deps vendoreadas** por front-door (el build hermético no tiene red).
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

### C.8 Bitácora del experimento M2 — tier gráfico, evidencia clara (2026-06-10)

Candidato: **`llimphi-ui` example `counter`** (stack completo `wgpu`+`winit`+`vello`+`parley`,
el del GIF del repo standalone). Dos hallazgos duros:

1. **El binario gráfico NO linkea las libs gráficas en tiempo de carga.** `ldd` del build glibc
   da sólo **5 `.so`**: `libc`, `libgcc_s`, `ld-linux` (+`libm`/`libdl`). `libvulkan`/`libwayland`/
   `libxkbcommon` **no aparecen** — wgpu (ash) y winit las cargan por **`dlopen` en runtime**. O
   sea: la dependencia gráfica es *runtime*, no *link-time*. Esto descarta el estático-de-oro
   (un binario fully-static no puede `dlopen` fiable) y obliga a la **vía dinámica**.
2. **Todo el árbol gráfico COMPILA a musl sin tocar una línea.** El build a
   `x86_64-unknown-linux-musl` (dinámico, `-crt-static`) compiló wgpu/naga/vello/parley/winit
   completo y **falló sólo en el link final**: `musl-gcc: cannot find -lgcc_s`. No es
   incompatibilidad de fuente: es un **hueco de toolchain** (musl dinámico quiere el `libgcc_s`
   compartido para unwinding, ausente en el sysroot musl de este host glibc).

**Diagnóstico y decisión (ahora con evidencia, ya no abierta a ciegas):**

- El `-lgcc_s` faltante es **exactamente lo que el `zig cc` de hammer resuelve** (zig empaqueta
  su `compiler-rt`/equivalente de libgcc). En el lab de hammer (zig cc, no `musl-gcc`) este link
  cierra. Alternativas: musl-cross-make con `libgcc_s`, o `panic=abort`.
- El "muro musl/glibc" del tier gráfico **no es de código**: es **coherencia de ABI en runtime**.
  Un binario musl debe `dlopen` un mesa/vulkan/wayland **también musl**. Alpine (la base de
  hammer) los trae musl-built → userland coherente → funciona. La rotura aparece sólo al
  **mezclar** (binario musl + libs gráficas glibc, como en este host Artix).
- **Resolución del #3:** el tier gráfico va **dinámico** y es **musl-viable a nivel de fuente**;
  el link exige el toolchain de hammer (zig cc) y el runtime exige un stack gráfico musl
  coherente (Alpine lo da). **glibc queda como fallback obligado sólo para NVIDIA propietario**
  (sin build musl). No hace falta un sub-mundo glibc para todo lo gráfico, sólo para el driver
  cerrado.

**No verificable en este sandbox:** el runtime (no hay display ni mesa-musl en el host glibc) —
queda para una VM Alpine con mesa musl. El link con `zig cc` queda para el lab de hammer.

Próximo natural: **M2b** — repetir el link con `zig cc` en el lab de hammer (cierra el binario)
y runtime-validar en Alpine. Luego **M3** (mirada: DRM/seat/input, link-time real contra
`libdrm`/`libinput`/`libseat`, no sólo `dlopen`).

---

## Coordinación

Contraparte en hammer: [`docs/adr/0007-arje-como-init-propio.md`](https://gitea.gioser.net/sergio/hammer)
(decisión de adoptar arje como init, bus único, CAS BLAKE3, confianza en capas) + puntero en
`docs/10-roadmap.md` §Track posterior.
