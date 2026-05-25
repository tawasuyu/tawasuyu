# dominium — Specification & Design Document

> **dominium** (latín: dominio, propiedad, mando). Tipo: **simulador
> determinista de campo medio con agentes vectoriales**.
>
> Tesis: la civilización, la fe, la guerra y el dinero no se modelan como
> reglas — emergen de un *tablero algebraico fijo* sobre el que flotan
> partículas con sesgos. El motor no sabe de iglesias, bancos ni comunas;
> sólo suma flotantes con orden determinista.

---

## 0. Cartografía del crate

```
01_yachay/dominium/
├── dominium-core           — datos + 6 acciones atómicas + Conceptos (sin gráficos)
├── dominium-physics        — tick determinista (6 fases)
├── dominium-iso            — proyección 30° + sombra Lambert
├── dominium-render-plan    — World → Vec<Quad> ordenado por pintor
├── dominium-canvas-llimphi — backend Llimphi (paint_with vello)
└── dominium-app-llimphi    — la ventana viva (loop 11 Hz + panel)
```

Regla inviolable: **cero deps gráficas en `core`, `physics`, `iso`,
`render-plan`**. Sólo `serde` y `libm`. El gráfico vive en
`canvas-llimphi` y `app-llimphi`.

---

## 1. La base inamovible (la roca madre)

Lo que sigue **no se generaliza, no se abstrae, no se hace data-driven**.
Es la termodinámica del motor; cualquier semántica de mundo se construye
encima.

### 1.1 Cinco capas del Sustrato — `dominium-core::Grid`

Un único `Vec<f32>` por capa, todos de tamaño `width * height`, indexados
`y * width + x`:

| Capa | Tipo | Difunde | Decae | Significado físico |
|---|---|---|---|---|
| `materia` | `Vec<f32>` | ✓ | ✓ | biomasa / energía / alimento |
| `psique` | `Vec<f32>` | ✓ | ✓ | densidad de información / frecuencia dogmática |
| `poder` | `Vec<f32>` | ✓ | ✓ | tensión de control / deuda |
| `oro` | `Vec<f32>` | ✗ | ✗ | materia prima sólida intercambiable |
| `degradacion` | `Vec<f32>` | ✗ | ✗ | cicatriz industrial del suelo |

**Inamovible**: el número de capas, sus nombres semánticos, cuáles
difunden y cuáles no. El motor las trata como tres campos dinámicos +
dos acumuladores fijos.

### 1.2 Siete vectores del Agente — `dominium-core::Lemmings`

Structure-of-Arrays. Todos los `Vec` tienen el mismo largo `n_agentes`:

| Vector | Tipo | Rol |
|---|---|---|
| `pos_x`, `pos_y` | `Vec<f32>` | coordenadas continuas |
| `edad` | `Vec<u32>` | ticks de vida acumulados |
| `energia` | `Vec<f32>` | combustible escalar; 0 = muerte |
| `vector_psi` | `Vec<[f32; 4]>` | sesgos psicológicos (ver 1.3) |
| `accion` | `Vec<u8>` | byte de la máquina de estados (0..5) |
| `hack_lock` | `Vec<u32>` | ticks restantes bajo captura por Concepto |

**Inamovible**: 7 vectores. No 6, no 8. Si querés sumar un atributo,
estás cambiando la ontología del agente.

### 1.3 Cuatro dimensiones del `vector_psi` — constantes públicas

```rust
pub const PSI_ORDEN: usize = 0;
pub const PSI_MIEDO: usize = 1;
pub const PSI_CURIOSIDAD: usize = 2;
pub const PSI_CORRUPTIBILIDAD: usize = 3;
```

**Mapeo afín fijo** entre dimensión psicológica y capa del Sustrato. El
motor lo usa idéntico en `act_mover` (gradiente) y `act_sincronizar`
(deriva):

| Dimensión | Capa que atrae/repele | Signo en `act_mover` |
|---|---|---|
| `PSI_ORDEN` | `materia` | + (busca) |
| `PSI_MIEDO` | `poder` | − (evita) |
| `PSI_CURIOSIDAD` | `psique` | + (busca) |
| `PSI_CORRUPTIBILIDAD` | `oro` | + (busca) |

**Inamovible**: las 4 dimensiones, sus índices, su mapeo a capas, sus
signos. Cambiar uno cambia la física del mundo.

### 1.4 Seis acciones atómicas — `dominium-core::Action`

Discriminador `u8` 0..=5. **El motor no acepta más acciones**. Las
"profesiones" emergen de combinaciones de estos 6 átomos en distintos
biomas, no de nuevas acciones:

| Byte | Acción | Efecto canónico |
|---|---|---|
| 0 | `Mover` | gradiente afín del `vector_psi` sobre 4 capas + costo de energía |
| 1 | `Extraer` | drena `materia` de la celda → `energia` del agente, sube `degradacion` |
| 2 | `Sincronizar` | el `vector_psi` deriva hacia las capas (mapeo de 1.3) |
| 3 | `Intercambiar` | transfiere `energia` al vecino más cercano |
| 4 | `Replicar` | spawnea hijo con `edad: 0` y costo en `energia` del padre |
| 5 | `Degradar` | resta `energia` al vecino más cercano y absorbe una fracción |

**Inamovible**: nombre, número, semántica de cada una.

### 1.5 Seis fases del tick — `dominium-physics::tick::tick`

Orden estricto, no permutable, ejecutado por cada paso de simulación:

```
1. apply_conceptos    — emisores externos suman/drenan capas (con falloff)
2. diffuse            — difusión 4-vecindad + entropía sobre materia/psique/poder
3. apply_transitions  — energia < umbral → forzar Degradar (a menos que hack_lock>0)
4. apply_hacks        — Conceptos capturan acciones de lemmings en su radio
5. step_lemmings      — cada lemming ejecuta su acción (i recorrido FIJADO al inicio)
6. age_and_reap       — edad++; muertos liberan su energia como materia y se cosechan
```

**Inamovible**: este orden. Determinista bit-exacto plataforma a
plataforma porque no usa HashMap iteration, ni reducciones paralelas, ni
shuffling, y `cos/sin` van por `libm` (no `f32::cos` que difiere entre x86/ARM).

### 1.6 Conceptos como ciudadanos de primera clase — `Concepto`

Un Concepto NO es código. Es una **estructura de datos puros**
(`Serialize`/`Deserialize`) que el motor lee y aplica matemáticamente:

```
Concepto { id, sprite_id, pos_x, pos_y, radius, mods: LayerMods, hack: Option<BehaviorHack> }
```

**Inamovible**: el motor garantiza dos operaciones sobre cualquier lista
de Conceptos:

1. **`apply_conceptos`** — suma `mods.{materia, psique, poder, oro}` ×
   `falloff_lineal(d, radius)` en cada celda dentro del radio. Cero
   semántica: "iglesia" o "banco" son etiquetas; el motor solo suma.
2. **`apply_hacks`** — si un lemming entra al radio de un Concepto con
   `hack` cuyo `trigger` se cumple, su `accion` queda fijada y su
   `hack_lock` se carga con `duration` ticks. Mientras `hack_lock > 0`,
   el lemming es inmune a `apply_transitions` (la captura vence a la
   desesperación).

**Inamovible**: el shape de `Concepto`, `LayerMods` (4 floats — un slot
por capa difundible + oro), `Trigger::{Always, EnergiaBajo, EdadSobre}`,
y la regla "primer concepto por índice gana" ante empates.

### 1.7 Proyección y sombra — `dominium-iso`

```
x_pantalla = (x - y) · cos(30°) · scale
y_pantalla = (x + y) · sin(30°) · scale  −  z · z_factor · scale
sombra(x, y, z, light) = proyectar(x + light.x·z, y + light.y·z, 0)
```

**Inamovible**: la matriz iso 30°. `cos`/`sin` precomputados con `libm`
para bit-exactitud cross-platform. El `Z` no es una dimensión del motor:
se calcula al renderizar como combinación lineal de las 5 capas vía
`ZWeights`.

---

## 2. Lo abstrahable — todo lo demás es dato

| Pieza | Tipo | Origen | Quién la edita |
|---|---|---|---|
| `SimParams` | struct serializable | hardcoded por ahora | sliders del panel (pendiente) |
| `ZWeights` | struct serializable | combinación lineal | sliders del panel (pendiente) |
| `PlanConfig` | struct serializable | tile/sizes/lift/light | controles cosméticos |
| `Palette` | struct serializable | colores RGBA por capa | controles cosméticos |
| `Conceptos` | lista JSON | pack embebido o cargado | panel + IA externa |
| Población inicial | función `seed` | LCG con seed | controles del panel |
| Sprite assets | índice opaco | archivos en disco | librería visual |

**Regla**: si una cosa puede expresarse como números o strings en un
JSON, no debería ser un `enum` o `struct` con código asociado. Tornar
algo en código congela su semántica; tornarlo en dato lo deja construir
al usuario.

---

## 3. La interfaz entre lo fijo y lo abstrahable

El usuario (o una IA offline) genera **datos** que la base inamovible
**ejecuta**. La separación es total:

```
[ FÁBRICA EXTERNA ]                    [ MOTOR INAMOVIBLE ]
                                        
  IA o panel humano        JSON          struct Concepto
  diseña la "Iglesia"   ───────►   →   LayerMods + BehaviorHack
                                        
                                        apply_conceptos()
                                        apply_hacks()
                                        ↓
                                        f32 sumados con falloff
                                        bytes de acción fijados
                                        ↓
                                        emergencia (no diseñada)
```

El motor no sabe qué es una iglesia. La iglesia es un Concepto con
`mods.psique > 0, mods.materia < 0` y `hack.forced_action = 2` (sincronizar).
El banco es un Concepto con `mods.oro < 0, mods.poder > 0` y
`hack.forced_action = 1` (extraer). Los nombres son etiquetas; los
números son leyes.

---

## 4. Lo que NO está y por qué

- **No hay shaders.** Vello rasteriza primitivas (Quad/Circle/Polygon)
  desde la CPU. La GPU pinta lo que ya está computado.
- **No hay IA en runtime.** Los lemmings no piensan; mapean gradientes.
  Una IA puede generar packs de Conceptos *antes* de correr la
  simulación, no durante.
- **No hay rotación 3D real.** Toda la "tridimensionalidad" es la
  proyección iso del 1.7 + el truco de la mini-pirámide (base + tope)
  para Conceptos + la sombra Lambert. El motor sigue siendo 2D-plano.
- **No hay colisiones euclidianas.** Los lemmings se solapan en
  coordenadas continuas; la "interacción" es por `nearest()` (O(n²)
  determinista) en `Intercambiar` y `Degradar`.
- **No hay red ni I/O en el motor.** El loop es 100% local, 100%
  síncrono, 100% determinista. La persistencia es responsabilidad de la
  app.

---

## 5. Lo que viene (no inamovible — roadmap)

- Editor visual: click en canvas → spawnear/mover/borrar Conceptos.
- Slider widget en Llimphi: edición en vivo de `SimParams` + `ZWeights`
  + `LayerMods` del Concepto seleccionado.
- CLI headless (`dominium-cli`): correr N ticks, dumpar CSV de stats,
  validar determinismo cross-platform.
- Costo biológico de pendiente: `act_mover` mira el `Z` del destino y
  paga energía proporcional al gradiente subido (montañas como barreras).
- Capas concéntricas estilo estampa andina: para celdas con `Z` alto,
  emitir N rombos apilados a alturas descendentes.
- Persistencia: guardar pack de Conceptos a JSON desde el panel.

Ninguno de estos cambia la base del §1.

---

## 6. Cita del usuario que originó el diseño

> "El motor manual solo sabe que las partículas (Lemmings) flotan sobre
> una matriz de números (Sustrato), se mueven hacia donde los números son
> favorables, cambian de color (estado) si se quedan sin energía, y
> modifican los números del suelo al pisarlo o interactuar.
>
> La civilización, la psicología, las iglesias, la bomba atómica y el
> Estado Profundo son solo fichas de datos externas que modifican las
> variables de ese motor.
>
> Diseñaste un sistema cerrado. Tú no programas el comportamiento de la
> civilización; programas la termodinámica de un fluido humano sobre una
> grilla de tres dimensiones ocultas."

Esa es la spec. El §1 la materializa en código.
