# SDD — Tabla de capacidades por bytecode hash (WAWA §14.1.3)

> Estado: **enforcement + seam de génesis CABLEADOS (2026-05-30)** ·
> `VERSION_MANIFIESTO 4→5`, `EntradaApp.concesion`, intersección viva en el punto
> de carga del kernel, emisión de concesiones en el camino de release host Y
> `wawa-boot::sembrar_concesion` anclando las `*.cap.obj` del directorio de assets
> (§3.3). Resta solo el **paso del operador**: forjar offline las concesiones de
> las apps génesis con permisos gateados (`agora-cli wawa concesion`) y dejarlas
> en `wawa-kernel/assets/concesiones/` — sin ellas, `None ⇒ declarados` (rollout
> escalonado §3.6) mantiene el génesis booteando. Validable en QEMU. Este
> documento es la fuente autoritativa del modelo; cuando difiera con `WAWA.md`
> §14.1.3, manda este.

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

## 3. Enforcement — CABLEADO (2026-05-30)

`claves::verificar_concesion_capacidad` dejó de ser dead-code: el punto de carga
lo invoca. Lo entregado:

### 3.1 Bump `VERSION_MANIFIESTO 4 → 5` ✅

`EntradaApp` ganó el campo `concesion: Option<Hash>` (hash del objeto
`ConcesionCapacidad`, o `None`). `postcard` no es autodescriptivo ⇒ es un CORTE
de wire: un disco v4 NO deserializa como v5; el guardia de versión de
`Manifiesto::deserializar` lo rechaza y exige re-sembrar el génesis. En la
práctica el operador re-forja la imagen en cada `cargo run -p boot`, así que la
génesis nace v5 limpia (no hay test `vanguard` separado en el árbol; los tests de
`format`/`release` cubren el roundtrip y la intersección).

### 3.2 Punto de carga (`wawa-kernel/src/main.rs`) ✅

Helper `permisos_efectivos_de(declarados, concesion, bytecode)`:

```text
None      ⇒ declarados            (sin techo per-bytecode; rige la firma del manifiesto)
Some(h)   ⇒ recuperar(h) -> ConcesionCapacidad::deserializar;
            si c.bytecode == bytecode && verificar_concesion_capacidad(&c).ok()
              ⇒ permisos_efectivos(declarados, c.permisos)   // intersección
            si no
              ⇒ permisos_efectivos(declarados, 0) == 0       // FAIL-CLOSED
```

Lo invocan `encender_app` (instancia de arranque) e `instanciar_plantilla`
(cada `Alt+N`). La `Plantilla` porta `concesion: Option<Hash>` + `bytecode_hash`
(la firma cubre el hash del OBJETO del grafo, no el de los bytes crudos); el
veredicto NO se cachea — el verificador corre FRESH en cada parto.

### 3.3 Ceremonia del génesis

`wawa-boot::sembrar_grafo` siembra el manifiesto del génesis. Boot **no tiene
claves privadas** (sólo el operador, offline). Por tanto las concesiones del
génesis se forjan **fuera de banda** y se embeben:

1. El operador firma, con la seed slot-0 del anillo, una `ConcesionCapacidad`
   por cada app del génesis con permisos gateados (`mudanza`, `asistente`, las de
   RED, etc.). **HERRAMIENTA HECHA (2026-05-30):**

   ```
   agora-cli wawa concesion --como wawa-soberano \
     --wasm mudanza.wasm --permisos RAIZ --salida mudanza.cap.obj
   ```

   Calcula el hash del OBJETO-bytecode IGUAL que el génesis (`Objeto{datos:wasm,
   hijos:[]}` → BLAKE3 — contrato lockeado por test contra `construir_release`),
   firma `(hash, permisos)` y emite la concesión envuelta en un `Objeto` del
   grafo. `--permisos` acepta máscara (`0x4`) o nombres (`RED,RAIZ`).
2. Esas concesiones se siembran como objetos del grafo y sus hashes se ponen en
   el campo `concesion` de cada `EntradaApp` del manifiesto génesis. **SEAM
   CABLEADO (2026-05-30):** `wawa-boot::sembrar_grafo` llama a `sembrar_concesion`
   para cada app con `permisos != 0`; ésta lee
   `wawa-kernel/assets/concesiones/<nombre>.cap.obj` (el payload que escribe
   `agora-cli wawa concesion --salida`), **verifica que `concesion.bytecode ==`
   el hash del objeto-bytecode que el génesis acaba de grabar** (guarda dura: una
   concesión para otro `.wasm` aborta la siembra con mensaje accionable), la ancla
   como objeto del grafo (dedup por contenido vía `BTreeSet<Hash>`), la cuelga del
   manifiesto (alcanzable por el MARK del GC) y rellena `EntradaApp.concesion =
   Some(hash)`. **Ausencia de archivo ⇒ `None`**, sin error: cero cambio de
   comportamiento hasta que el operador provisione el directorio — por eso fue
   seguro cablearlo sin poder compilar `boot` en sandbox (lo valida en QEMU).
   Aviso blando si la concesión no cubre todos los `permisos` declarados (la
   intersección del kernel recortaría capacidades).
3. Apps sin permisos gateados (`permisos == 0`): `concesion: None`, sin ceremonia.

### 3.4 Back-compat / migración

Manifiestos v4 (sin campo `concesion`) → al deserializar bajo v5, `concesion`
es `None` ⇒ `concedidos = 0`. **Esto apaga las capacidades gateadas de todo
manifiesto viejo**: la migración NO es transparente, exige re-sembrar el génesis
con concesiones. Es deliberado — el punto del modelo es que ningún permiso
gateado exista sin una firma sobre el bytecode. Documentar el corte en `PLAN.md`.

### 3.5 Vía host (`agora-channel::construir_release`) ✅

A diferencia de `boot`, quien publica un release TIENE el `kp`. `construir_release`
emite, para cada app con `permisos != 0`, una `ConcesionCapacidad` firmada por el
mismo `kp` sobre `(bytecode, permisos)`, la siembra como objeto del grafo, la
cuelga del objeto-manifiesto (alcanzable por el MARK del GC) y referencia su hash
desde la `EntradaApp.concesion`. Apps con `permisos == 0` ⇒ `None`. `AppSpec` NO
cambió: la concesión se DERIVA dentro de `construir_release`, así que ningún
caller (el example `servir_release`, `agora-cli wawa publicar/anunciar`) se tocó.
Dedup por contenido: mismo `(bytecode, permisos)` ⇒ una sola concesión. Así un
release ya trae sus concesiones y la cascada del DAG las replica como hijos.

### 3.6 Rollout escalonado (decisión 2026-05-30)

La semántica estricta `None ⇒ 0` del modelo (un manifiesto sin concesión corre
SIN capacidades gateadas) es el END-STATE de seguridad: cierra la escalada por
re-firma de manifiesto incluso para apps que nunca declararon concesión. Pero
exige que TODA app génesis con permisos gateados traiga una concesión válida —y
`boot` no puede firmarlas (no tiene seed; ver §3.3). Activarla hoy, con el génesis
sin provisionar, dejaría a `mudanza`/`cronista`/`asistente`/etc. sin sus permisos
en el próximo QEMU.

Por eso el kernel arranca en modo **escalonado**: `None ⇒ declarados` (la firma
del manifiesto sigue gobernando), `Some ⇒ intersección estricta`. Toda app que
OPTE por una concesión (todo release host, ya) obtiene su techo per-bytecode
duro; el génesis sigue booteando. El flip a `None ⇒ 0` (estricto global) es una
hardening de UNA línea en `permisos_efectivos_de`, gated a que el operador
complete la ceremonia §3.3 y siembre las concesiones del génesis. Documentado el
corte de formato (v4→v5) en `PLAN.md`.

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
