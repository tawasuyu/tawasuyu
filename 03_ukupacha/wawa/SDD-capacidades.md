# SDD — Tabla de capacidades por bytecode hash (WAWA §14.1.3)

> Estado: **fase fundacional cerrada (2026-05-30)** · enforcement pendiente de
> QEMU + ceremonia. Este documento es la fuente autoritativa del modelo de
> concesiones de capacidad; cuando difiera con `WAWA.md` §14.1.3, manda este.

## 0. El problema

Hoy los permisos de una app viven en su `EntradaApp` del manifiesto:

```
EntradaApp { ..., bytecode: Hash, permisos: Permisos }
```

El manifiesto entero se firma (`ManifiestoFirmado` = firma sobre su hash, o la
`RaizFirmada` del canal). Por transitividad, `permisos` ya está cubierto por la
firma: alterar un bit cambia el hash del manifiesto y rompe la firma.

Pero el binding **"qué binario puede hacer qué"** es sólo tan fuerte como el
manifiesto: re-firmar un manifiesto NUEVO (con autoridad `PERMISO_RAIZ`) basta
para darle al **mismo** bytecode permisos distintos. La autoridad que decide
"el layout/versión del escritorio" (reancla) es la misma que decide "este
binario puede tocar la raíz / la red". No hay separación, y el grant no viaja
con el binario: depende de en qué manifiesto aparezca.

**Objetivo (§14.1.3):** elevar el binding a un hecho INDEPENDIENTE del
manifiesto — una firma Ed25519 de una llave del `AGORA_AUTH_RING` sobre el par
`(hash_bytecode, permisos)`. La firma viaja con el binario; ningún manifiesto
puede escalar un binario más allá de lo que su concesión autoriza.

## 1. Modelo

### 1.1 La concesión (`format::ConcesionCapacidad`)

```rust
struct ConcesionCapacidad {
    bytecode: Hash,      // qué binario (hash BLAKE3 del objeto-bytecode)
    permisos: Permisos,  // qué puede hacer (bitfield PERMISO_*)
    autor: AgoraId,      // quién lo concede (debe habitar AGORA_AUTH_RING)
    firma: Firma,        // Ed25519 sobre mensaje_capacidad(bytecode, permisos)
}
```

Es un objeto del grafo (direccionado por contenido) — su hash es el hash de su
forma `postcard`, como cualquier nodo.

### 1.2 El mensaje canónico (`format::mensaje_capacidad`)

```
mensaje_capacidad(bytecode, permisos) = bytecode(32) || permisos_le(4)   // [u8; 36]
```

Zero-alloc (arreglo de pila), apto para Ring 0. Liga la firma al hash EXACTO del
binario y al bitfield EXACTO: una concesión para X no vale para Y, y subir un
bit invalida la firma. Gemelo de `mensaje_a_firmar` (canales).

### 1.3 La regla de intersección (`format::permisos_efectivos`)

```rust
const fn permisos_efectivos(declarados, concedidos) -> Permisos {
    declarados & concedidos
}
```

Los permisos EFECTIVOS de una app son la **intersección** de lo que su
`EntradaApp` declara y lo que una concesión válida concede para su bytecode:

- el manifiesto no puede escalar un binario más allá de su concesión firmada;
- una concesión generosa no enciende permisos que el manifiesto no pidió;
- **sin concesión válida ⇒ `concedidos = 0` ⇒ cero capacidades gateadas** (la
  matriz pasiva de syscalls sigue disponible; sólo las gateadas se apagan).

## 2. Lo implementado en esta fase (fundacional, testeable sin QEMU)

| Capa | Pieza | Ubicación |
|---|---|---|
| Tipo + canónico | `ConcesionCapacidad`, `mensaje_capacidad`, `permisos_efectivos` | `shared/format/src/lib.rs` |
| Firma/verif. host | `firmar_capacidad`, `verificar_capacidad` | `03_ukupacha/agora/agora-channel/src/lib.rs` |
| Verif. soberana | `claves::verificar_concesion_capacidad` | `wawa-kernel/src/claves.rs` |

Tests: `format` (canonicidad, round-trip, intersección), `agora-channel`
(firmar/verificar, permisos manipulados, bytecode transplantado, autor ajeno).
`format` se mantiene `#![no_std]` + `wasm32` limpio (`check-shared-cores.sh`).

La verificación soberana sigue el orden estricto de sus gemelos
(`verificar_manifiesto_firmado` / `verificar_cuaderno_firmado`):

1. autor fuera del `AGORA_AUTH_RING` → `CapacidadInsuficiente` (sin tocar cripto);
2. pubkey o firma no decodifican → `Ausente`;
3. firma no valida sobre `mensaje_capacidad` → `AlmacenamientoFallo`.

## 3. Enforcement (próxima fase — requiere QEMU + ceremonia)

`claves::verificar_concesion_capacidad` ya es soberano y zero-alloc, pero
todavía **no se invoca**: el punto de carga aún pasa `entrada.permisos` directo.
Cablearlo exige:

### 3.1 Bump `VERSION_MANIFIESTO 4 → 5`

`EntradaApp` gana un campo:

```rust
concesion: Option<Hash>,   // hash del objeto ConcesionCapacidad, o None
```

Toca el test `test_wawa_ecosystem_immutable_vanguard` (guard del ABI) — hay que
actualizarlo a conciencia, no por inercia.

### 3.2 Punto de carga (`wawa-kernel/src/main.rs::encender_app`)

```text
let concedidos = match entrada.concesion {
    Some(h) => {
        let c = almacen::recuperar(&h) -> ConcesionCapacidad::deserializar;
        if c.bytecode == entrada.bytecode
           && claves::verificar_concesion_capacidad(&c).is_ok() { c.permisos }
        else { 0 }   // concesión ausente/corrupta/ajena/para otro binario
    }
    None => 0,       // sin concesión declarada
};
let efectivos = permisos_efectivos(entrada.permisos, concedidos);
wasm::AplicacionWasm::cargar(..., efectivos);
```

Misma intersección en `instanciar_plantilla` (la `Plantilla` debe portar
`concesion` junto a `permisos`). El verificador es FRESH en cada carga.

### 3.3 Ceremonia del génesis

`wawa-boot::sembrar_grafo` siembra el manifiesto del génesis. Boot **no tiene
claves privadas** (sólo el operador, offline). Por tanto las concesiones del
génesis se forjan **fuera de banda** y se embeben:

1. El operador firma, con una seed del anillo (`agora-cli` → `firmar_capacidad`),
   una `ConcesionCapacidad` por cada app del génesis que requiera permisos
   gateados (`mudanza`, `asistente`, las de RED, etc.).
2. Esas concesiones se siembran como objetos del grafo y sus hashes se ponen en
   el campo `concesion` de cada `EntradaApp` del manifiesto génesis.
3. Apps sin permisos gateados (`permisos == 0`): `concesion: None`, sin ceremonia.

### 3.4 Back-compat / migración

Manifiestos v4 (sin campo `concesion`) → al deserializar bajo v5, `concesion`
es `None` ⇒ `concedidos = 0`. **Esto apaga las capacidades gateadas de todo
manifiesto viejo**: la migración NO es transparente, exige re-sembrar el génesis
con concesiones. Es deliberado — el punto del modelo es que ningún permiso
gateado exista sin una firma sobre el bytecode. Documentar el corte en `PLAN.md`.

### 3.5 Vía host (`agora-channel::construir_release`)

`AppSpec` gana la concesión: `construir_release` emite, junto al objeto-bytecode
y al manifiesto, una `ConcesionCapacidad` firmada por el mismo `kp` y referencia
su hash desde la `EntradaApp`. Así un release publicado por `agora-cli wawa
publicar` ya trae sus concesiones, y la cascada del DAG (Fase 67, descarga
recursiva) las replica como un objeto-hijo más.

## 4. Modelo de amenaza

| Ataque | Defensa |
|---|---|
| Manifiesto re-firmado que sube permisos a un binario | Intersección con la concesión: `efectivos ≤ concedidos`. La concesión está firmada sobre el bytecode, no sobre el manifiesto. |
| Transplantar una concesión a otro binario | `mensaje_capacidad` cubre `bytecode`; la firma no valida. |
| Subir un bit de permiso en una concesión | El bitfield entra en el mensaje; la firma no valida. |
| Concesión firmada por una llave ajena | `autor_en_anillo` la rechaza con `CapacidadInsuficiente`. |
| App sin concesión | `concedidos = 0`: corre sin capacidades gateadas. |

No protege contra: una llave del anillo comprometida concediendo lo que quiera
(es la raíz de confianza por diseño — el anillo es la frontera) ni contra bugs
en wasmi que rompan el aislamiento (ortogonal a este modelo).

## 5. Decisiones de diseño

- **Intersección, no reemplazo.** Los permisos efectivos = manifiesto ∩
  concesión. Alternativa descartada: que la concesión SUSTITUYA a `EntradaApp.permisos`
  — rompería el principio de menor privilegio (un manifiesto modesto no podría
  bajar permisos por debajo de una concesión generosa).
- **Concesión como objeto del grafo + referencia por hash en `EntradaApp`.**
  Alternativa descartada: inlinear la concesión en `EntradaApp` — engorda el
  manifiesto y duplica la concesión si dos apps comparten bytecode (dedup del
  grafo lo evita con el modelo por-hash).
- **El kernel no almacena concesiones aparte.** No hay "tabla" mutable: la
  concesión vive en el grafo direccionado por contenido y se verifica FRESH en
  cada carga. "Tabla de capacidades" es el nombre del concepto, no de una
  estructura en RAM del kernel.
