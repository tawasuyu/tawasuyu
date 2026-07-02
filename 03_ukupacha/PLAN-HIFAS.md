# PLAN — Consumir `hifas` (sheafsync) como capa de decisión sin coordinación

> Estado: propuesta (2026-07-02). `hifas` (nombre en clave `sheafsync`) es un repo
> aparte (`../hifas`, git propio) — motor de consistencia *coordination-free* sobre
> cohomología de haces celulares + escrow con certificado. Este plan fija **qué de
> hifas nos sirve, cómo cablearlo al sustrato P2P del monorepo (agora + minga +
> nakui), y qué NO promete**. Sigue el precedente de `arje/PLAN-ATESTACION-Y-HAMMER.md`
> (plan que coordina dos proyectos hermanos sin que colisionen).

## 0. Qué es hifas y por qué nos toca

hifas responde, **con prueba y no a ojo**, la pregunta soberana / local-first: dado un
conjunto de réplicas que editan copias locales sin coordinar, ¿cuándo funden a un estado
global coherente y dónde hay una obstrucción que obliga a sincronizar? El corazón es puro:
sin red, sin persistencia, sin estado global (`Cargo.toml`: sólo `nalgebra` + `petgraph` +
`thiserror`). Es una **capa de decisión**, no un motor de replicación.

Nos toca porque su hito M4 (`adapter::SovereignAccount`) apunta explícitamente a
**Tawasuyu/Hammer** como el sustrato que provee **persistencia y transporte**, y **rehúsa**
(no promete) si ese sustrato no cumple. En el monorepo ese sustrato ya existe: **agora**
(identidad + gossip firmado) y **minga** (store direccionado por contenido + libp2p).

**Evidencia de que funciona (verificado 2026-07-02):** `cargo test` en `../hifas` → **89
tests, 0 fallos**. Los que corresponden a la garantía que nos interesa:

- `durable::tests::rollback_por_debajo_del_wal_siempre_rechazado`,
  `adversario_rollback_es_rechazado`, `spent_nunca_cae_por_debajo_del_ack` — el WAL no
  retrocede bajo un gasto ya ackeado.
- `escrow::tests::{certificado_dinamico, certificado_valor_nunca_baja_del_piso,
  t11a_dos_dispositivos_gastan_sin_pisarse}` — un-escritor-por-dispositivo + `valor ≥ K`
  bajo concurrencia/crash/partición.
- `fencing::tests::{el_clon_nunca_pasa_como_ok, dano_de_clon_acotado_a_una_celda}` — clon
  *bounded + detect*, no *prevent*.
- `adapter::tests::{rehusa_sin_durabilidad, rehusa_sin_identidad_por_dispositivo}` — el
  adaptador se niega si el sustrato no da las precondiciones.
- Contraprueba honesta: `durable::tests::g4_sin_durabilidad_el_rollback_sobregira` y
  `g4_el_certificado_se_pone_rojo_bajo_rollback` (should panic) — **sin** la capa, se
  sobregira. La garantía no es decorativa.

## 1. Las tres utilidades aprovechables (no sólo el escrow)

hifas trae **tres** cosas reutilizables, en orden decreciente de madurez para consumir:

### U1 — Escrow con certificado + durabilidad + fencing (la respuesta al single-writer durable)
`escrow.rs` (`EscrowState`: `granted[from][to]`/`spent[i]` monótonos, `avail` derivado;
`Σ avail = Σ b0 − Σ spent`) + `durable.rs` (`WriteAheadLog`: ack **tras** persistir;
`try_restore_snapshot` rechaza restaurar por debajo del compromiso durable) + `fencing.rs`
(época device-local; `MergeOutcome::CloneDetected` en el merge). Es la garantía
"single-writer con estado local durable que nunca restaura por debajo de un gasto
reconocido" — la mitad que en el monorepo hoy **no** existe (`nakui-sync::Writer` da
escritor único + log durable pero su `load_from_snapshot` restaura **incondicional**; hifas
es el rehúso-de-rollback que le falta).

### U2 — Asesor de coordinación (linter de topologías) — `cohomology` + `tarski` + `verdict` + `oracle`
Dada una topología de réplicas + tipos de dato, dice **"corre libre"** vs **"coordina aquí
(entre réplicas X e Y)"** con localización del nudo. El MVP lineal medía topología
(`H¹ = β₁`, falso-positivo en ciclos monótonos); la pista de retículos (`tarski.rs`,
Laplaciano de Tarski, Ghrist–Riess 2022) lo corrige: `Verdict::via_tarski` reconcilia al
*join* y sólo marca obstrucción cuando el punto fijo colapsa a `⊤` (un `reset` no monótono).
`oracle.rs` es un verificador CRDT/CALM **independiente** que valida los veredictos. Uso
para nosotros: **analizar en diseño** las topologías de gossip de minga/agora y decir dónde
la coordinación es genuinamente necesaria — antes de codearla.

### U3 — Ruteador de recurso acotado sobre malla — `router.rs`
`BudgetRouter` reparte un presupuesto acotado por el grafo real: `diffuse` (suaviza
proactivo, conserva `Σ b`), `rebalance_to` (flujo mínimo edge-local vía Laplaciano),
`spend_with_rebalance` con fail-safe: bajo partición sin ruta devuelve
`RebalanceResult::Unreachable` / `SpendOutcome::Rejected { spendable }` — **nunca sobregira
ni cuelga**. Reutilizable para distribuir **cualquier** recurso acotado sobre la malla minga
(rate-limits, cuotas de storage, créditos de cómputo), no sólo dinero.

## 2. El contrato de M4 (lo que hifas exige, literal)

`SovereignAccount::open(caps, inv, weights, edges)` con
`SubstrateCapabilities { durable_per_device, identity_as_devices }`:

- **Requiere** WAL durable de `spent` por dispositivo → si `!durable_per_device`, `open`
  devuelve `AdapterError::NoDurability`.
- **Requiere** identidad = conjunto de dispositivos (una celda single-writer por device) →
  si `!identity_as_devices`, `AdapterError::NoDeviceIdentity`.
- **Garantiza** seguridad coordination-free del invariante (`value() ≥ floor`).
- **Acota y detecta** (no previene) el clon: `sync → CloneDetected` es **alarma**, no
  recuperación (el dinero ya salió; la compensación vive en la app y puede fracasar).
- **Limita** la disponibilidad bajo partición al presupuesto local (CAP: safety > liveness).

Hoy `caps` lo **llena el caller a mano** (el adaptador es agnóstico del sustrato: cero deps
de minga/agora/red). El puente es un **contrato, no un cable todavía**.

## 3. Cómo encaja el sustrato P2P del monorepo

| Necesidad de hifas | Quién lo aporta | Detalle |
|---|---|---|
| Identidad = dispositivo (`identity_as_devices`) | **agora** | Cada device = pubkey Ed25519; `agora-core`/`agora-graph`. Mapear `AgoraId` → `escrow::Identity`. |
| Plano de merge autenticado + anti-entropy | **agora-gossip** | Transport-agnostic sobre atestaciones firmadas; **monótono** (agrega, nunca retracta) → es el caso CALM-fácil que el oráculo confirma "corre libre". |
| Persistencia durable direccionada por contenido | **minga-store** | sled + BLAKE3. Base del WAL por-dispositivo (la semántica anti-rollback la pone `durable.rs`, no minga). |
| Transporte de la malla (grants/rebalanceos reales) | **minga-p2p** | libp2p sobre `card-net` (relay/DCUtR/AutoNAT). |
| Espejo `no_std` de verificación de firmas | **agora-channel** ↔ `wawa-kernel/claves.rs` | Ya existe; reusar para atestar identidad de device sin criptografía nueva. |

**El encaje fino (por qué no es casualidad):** el gasto es **no monótono** (gastar retracta
presupuesto) → normalmente pediría coordinación. hifas lo **re-ingeniería a monótono**
(`spent[]`/`granted[][]` monótonos, merge por máximo). Recién por eso el `EscrowState` **es
enviable** por el anti-entropy monótono de agora-gossip/minga. hifas convierte lo no-CALM en
CALM; el transporte monótono que ya tenemos alcanza.

## 4. Fases

### F0 — Vendorizar y fijar (prerrequisito)
Decidir cómo entra hifas: `path`/`git`-dep con commit fijado (hoy `../hifas`, HEAD
`703ae70`). No copiar el código al workspace; consumir el crate `sheafsync` como dependencia
externa pinneada, igual que las recipes de hammer (C.3 de arje↔hammer). `cargo check
--workspace` debe seguir verde.

### F1 — Adaptador de identidad: agora → `escrow::Identity`
Mapear una identidad agora (pubkey Ed25519) a un `device` de hifas. Cada device = una celda
single-writer. Test: dos devices de la misma cuenta gastan sin serializar (espeja
`t11a_dos_dispositivos_gastan_sin_pisarse`). **Frontera:** agora **no** impone single-writer
(su gracia es que "nadie lo corre"); el single-writer-por-celda es disciplina que hifas
enforce, agora sólo aporta la identidad.

### F2 — WAL por-dispositivo sobre minga-store
Montar `durable::WriteAheadLog` encima de `minga-store` (sled): el `spend` persiste **antes**
del ack; `recover` restaura `≥ último ack`; `try_restore_snapshot` rechaza el rollback.
minga da el blob durable; el "no olvida / no retrocede" es de hifas. Esto es lo que permite
poner `durable_per_device = true` **con honestidad** — si no se puede garantizar, se deja en
`false` y `open` rehúsa (es correcto).

### F3 — Transporte del `EscrowState` por agora-gossip / minga-p2p
Serializar el `EscrowState` (monótono) como carga anti-entropy. Los `grant`/rebalanceos de
`router.rs` corren **de verdad** sobre la malla (el "M4 desbloqueado" de `STRONG_RESULT.md`).
`sync` de dos réplicas → `MergeOutcome`. **Caveat de escala:** `minga/MONTAJE.md` avisa que
el sync P2P masivo aún no escala (carga el grafo a RAM; refactor `NodeStore` pendiente) — el
transporte existe con techo conocido; empezar con cuentas chicas.

### F4 — CloneDetected → atestación de revocación en el TrustGraph de agora
Cuando `sync` devuelve `CloneDetected`, emitir una atestación firmada en agora que marque el
linaje obsoleto. Es **compensación de capa de aplicación** (`BOUNDARY.md`): acota y delata,
**no** recupera el dinero ya entregado, y **puede fracasar**. No leerlo como auto-reparación.

### F5 (paralelo, barato) — Cosechar U2/U3 sin esperar a F1–F4
- **U2 asesor:** un `example`/CLI que tome una topología de minga/agora y reporte
  "corre libre / coordina aquí". No necesita cablear escrow; usa `verdict` + `oracle`.
- **U3 ruteador:** ofrecer `BudgetRouter` como utilería para cuotas de storage/cómputo sobre
  la malla, independiente del caso "dinero".

## 5. Qué NO promete (las fronteras, dichas)

- **No previene el doble gasto por clon.** Lo acota a una celda y lo delata; el resto es
  app-layer y puede fallar (`BOUNDARY.md`).
- **No hay disponibilidad ilimitada bajo partición.** Cada device se limita a su presupuesto
  local; `diffuse` lo mitiga, no lo elimina (`AVAILABILITY.md`). CAP: safety > availability.
- **No es un motor de replicación.** La red y la persistencia las provee el monorepo; hifas
  **exige** durabilidad-por-dispositivo o rehúsa.
- **El asesor U2 no supera a CALM** (`STRONG_RESULT.md` retira ese reclamo por falso). La
  contribución defendible es el **ruteo/localización por el grafo real**, no un teorema nuevo.

## Coordinación

Contraparte: repo `../hifas` (`DESIGN.md` + `ADDENDUM.md` pista de retículos, `STRONG_RESULT.md`,
`BOUNDARY.md`, `AVAILABILITY.md`, `M4.md`). Participantes tawasuyu: `03_ukupacha/agora`
(identidad + gossip), `03_ukupacha/minga` (store + libp2p), `01_yachay/nakui/nakui-sync`
(escritor único durable — candidato de sustrato, hoy sin anti-rollback).
