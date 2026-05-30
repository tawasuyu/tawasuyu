# SDD — Rotación y revocación de claves (agora #4)

> Estado: **diseño cerrado (2026-05-30)**, primitivos en construcción. Resuelve
> el item #4 del norte de `ARQUITECTURA.md`. Fuente autoritativa del modelo.

## 0. La distinción que define todo

agora tiene **dos planos de confianza**, y rotación/revocación significan cosas
distintas en cada uno:

- **Plano social** (userspace, `agora-graph::TrustGraph`): atestaciones
  `Claim{subject,predicate,value}` de firma única. La autoridad de revocación de
  una identidad es un **set de guardianes** que ella pre-declara.
- **Plano de control** (kernel, `claves.rs::AGORA_AUTH_RING`): un `const` de N
  pubkeys compilado en el kernel; compuerta de manifiesto/canal/concesión. Es el
  norte ("raíz de confianza ejecutable"). La autoridad de revocación es el
  **propio anillo** por quórum M-of-N.

**Decisión (3 forks, 2026-05-30):** atacar **AMBOS** planos con **primitivos
compartidos** en `agora-core` cuya autoridad es *pluggable* — el set autorizador
es el `AGORA_AUTH_RING` (control) o el set de guardianes (social), pero la
maquinaria de verificación M-of-N es la misma (`MultiSignature::verify_threshold_in`).

## 1. Tres mecanismos, no uno

| Mecanismo | Caso | Autoridad | Firma |
|---|---|---|---|
| **Rotación** | handoff voluntario vieja→nueva, SIN compromiso | posesión de ambas claves | doble: `sig_old` + `sig_new` |
| **Revocación** | clave comprometida o retirada | **M-of-N** del set autorizador | `MultiSignature` |
| **Caducidad** | confianza que se pudre sola | — (TTL en la atestación) | la firma original |

La clave del modelo: **una clave comprometida no puede revocarse a sí misma**
(el atacante la tiene). Por eso la revocación por compromiso es M-of-N de OTROS
(el anillo o los guardianes), nunca self-signed. Self-signed solo vale para
retiro voluntario (`RevReason::Retired`).

## 2. Primitivos compartidos (`agora-core::lifecycle`)

### 2.1 `KeyRotation` — rotación doble-firmada

```rust
struct KeyRotation { old_key: [u8;32], new_key: [u8;32], issued_at: u64,
                     sig_old: [u8;64], sig_new: [u8;64] }
```

Mensaje canónico (domain-separated, tamaños fijos):
`b"agora-key-rotation\x01" || old_key || new_key || issued_at.to_le_bytes()`.

`verify()`: ambas firmas cierran bajo SU respectiva clave. `sig_old` prueba que
la vieja AUTORIZA el handoff; `sig_new` prueba POSESIÓN de la nueva (nadie ata la
clave de otro como sucesora sin su consentimiento). NO es M-of-N: la rotación
voluntaria se auto-autoriza con la clave vieja viva.

### 2.2 `Revocation` — revocación M-of-N

```rust
enum RevReason { Compromised, Retired, Superseded }   // discriminante en canónico
struct Revocation { target_key: [u8;32], reason: RevReason, issued_at: u64,
                    expires_at: Option<u64>, authorizers: MultiSignature }
```

Mensaje canónico:
`b"agora-revocation\x01" || target_key || [reason_byte] || issued_at.to_le_bytes()
|| [tag_expires] (|| expires.to_le_bytes())`.

`verify(min, allowed)`: `authorizers.verify_threshold_in(canónico, min, allowed)`.
`expires_at = None` ⇒ revocación PERMANENTE (compromiso); `Some(t)` ⇒ suspensión
temporal que vence en `t` (la "caducidad estricta" del roadmap, generalizada).

## 3. Consumo en el plano social (`agora-graph::TrustGraph`)

- El grafo guarda `Vec<Revocation>` y `Vec<KeyRotation>` como **tombstones de
  primera clase**, separados de las atestaciones.
- `evidence_for`/`corroboration` FILTRAN: una atestación cuyo `attester` esté
  revocado (activo a `now`) NO cuenta. Aplicado en tiempo de consulta ⇒ un
  re-gossip de lo revocado NO lo resucita (sobrevive al replay).
- `current_key(identity, now)`: sigue la cadena de `KeyRotation` válidas hasta la
  punta. Una revocación de un eslabón corta la cadena ahí.
- **Precedencia: la revocación SIEMPRE gana** sobre una rotación que involucre la
  clave revocada, sin importar timestamps (un atacante no rota para escapar una
  revocación M-of-N).
- Autoridad social: el set autorizador de una identidad son sus guardianes
  declarados (atestación reservada `predicate="guardian"`, una por guardián).

## 4. Consumo en el plano de control (kernel)

- `agora-core` es `std` + `ed25519-dalek`: el kernel NO lo enlaza. Espeja la
  verificación en `wawa-kernel/src/claves.rs` con `ed25519-compact` zero-alloc
  (mismo patrón que `verificar_anuncio_canal`), sobre los MISMOS bytes canónicos.
- El anillo sigue siendo `const` (ancla de compile-time, "frontera física"):
  rotar el ancla = **reflash deliberado** del kernel, no hay rotación online del
  root (sería recursión: quién ancla al que ancla).
- Lo que SÍ se agrega: un **overlay de revocación** — un objeto del grafo que
  lista `Revocation`s firmadas M-of-N por el anillo. `autor_en_anillo(k)` pasa a
  exigir además que `k` NO esté en el overlay (activo a `now`). Así una clave del
  anillo filtrada se deniega ENTRE reflasheos sin esperar el reflash. El overlay
  se ancla como el manifiesto (superbloque) y se verifica FRESH en cada carga.

## 5. Modelo de amenaza

| Ataque | Defensa |
|---|---|
| Clave filtrada sigue firstmando releases | Revocación M-of-N del resto del anillo; overlay la deniega en kernel |
| Atacante rota la clave filtrada a la suya | Precedencia: revocación M-of-N gana sobre cualquier `KeyRotation` |
| Re-gossip de atestaciones de una clave revocada | Filtro en tiempo de consulta contra el set revocado (no resucita) |
| Atar la clave de otro como sucesora | `sig_new` exige posesión de la nueva |
| Revocar con M-1 claves | `verify_threshold_in` cuenta firmantes DISTINTOS del set |

No protege contra: quórum del anillo comprometido (es la raíz por diseño) ni
contra reflash malicioso del kernel (frontera física fuera de alcance).

## 6. Plan de implementación

1. ✅ `agora-core::lifecycle` — `KeyRotation` + `Revocation` + `RevReason` + tests.
2. ✅ `agora-graph` — tombstones, filtrar evidencia (`corroboration_at`),
   `current_key_at`, `guardians_of`, `is_revoked_at`, `ingest_revocation`.
3. ✅ `agora-store` — rotaciones/revocaciones en el SNAPSHOT (camino frío; el
   append-log queda para las atestaciones, camino caliente), `serde(default)` ⇒
   sin corte de formato (SCHEMA 1). `load` re-verifica firma (doble en rotación,
   integridad en revocación; el umbral M-of-N es del consumidor, no del store).
4. `wawa-kernel/src/claves.rs` — espejo `verificar_revocacion` + overlay en carga.
5. `agora-cli` — `identidad rotar` / `identidad revocar` (M-of-N) / `wawa revocar`.
