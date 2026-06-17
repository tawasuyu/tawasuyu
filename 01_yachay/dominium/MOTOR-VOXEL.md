# MOTOR-VOXEL — Tier 2 destripado

> Diseño técnico de un motor de voxels 3D estilo Minecraft sobre **Llimphi**, que
> **dominium**-la-sim alimentaría. Es un documento de *intención y arquitectura*,
> no código. Escrito 2026-06-17 a raíz de la pregunta "¿por qué un 50×50 me copa
> los FPS y un Minecraft infinito no?". La respuesta corta vive en el chat; acá
> está el cómo se construiría el motor que cierra esa brecha.

---

## 0. Encuadre: qué es Tier 2 y qué NO

**Tier 0/1** (otro doc/tarea) arreglan el render actual de dominium (2.5D vectorial
sobre vello): caché de plan + culling (Tier 0), instancing GPU (Tier 1). Eso da
50×50–256×256 isométrico fluido **sin cambiar de paradigma**.

**Tier 2** es cambiar de paradigma: un **motor de voxels 3D de verdad** — cámara 3D
libre, mundo en chunks, meshing con face-culling, z-buffer, iluminación, entidades,
y (opcional) streaming "infinito" y dimensiones paralelas. Es lo que hace que
Minecraft renderice barato un mundo gigante: **no dibuja el mundo, dibuja la
cáscara visible cacheada en la GPU**.

**Anti-objetivos** (lo que Tier 2 NO debe volverse):
- No es un clon de Minecraft-el-juego (crafting, inventario, gameplay). Es un
  **motor de visualización 3D voxel** para una simulación.
- No reemplaza vello/2D de Llimphi. Es un **pase 3D que convive** con el 2D.
- No obliga a "infinito". El infinito es un *feature opcional* (§7); dominium
  probablemente quiere un mundo **acotado pero grande** (256³–1024²×altura).

**Dónde vive:** en **Llimphi** (es capacidad de motor gráfico: wgpu, 3D, meshing,
shaders). dominium es **consumidor** — aporta el contenido (las 5 capas + agentes →
voxels), no el renderer. Crate nuevo propuesto: `llimphi-voxel` (+ posible
`llimphi-3d` para la base de cámara/depth/pipeline 3D reusable).

---

## 1. La idea central (por qué Minecraft vuela)

Minecraft es rápido porque hace lo opuesto al render naïve en **cada eje**. Tier 2
copia exactamente esas cinco decisiones:

| Eje | Naïve (dominium hoy) | Minecraft / Tier 2 |
|---|---|---|
| **Qué dibuja** | todas las celdas, cada frame | solo la **superficie visible** de los chunks cargados |
| **Cuándo genera geometría** | cada frame, desde cero | **una vez por chunk**, cacheada; re-mesh solo si el chunk cambia |
| **Dónde vive la geometría** | se reconstruye en CPU c/frame | **vertex/index buffers en la GPU**, persistentes |
| **Caras ocultas** | dibuja techo + caras igual | **face culling**: caras entre dos sólidos no se emiten |
| **Fuera de cámara** | sobredibujo (painter) | **frustum + distancia + (occlusion)** culling |

Resultado: un chunk estático cuesta ~**una draw-call de un buffer que ya está en la
GPU**. Mover la cámara no regenera nada. Por eso 10.000 chunks "existen" pero solo
~200 se dibujan, y esos 200 son buffers cacheados. La sim puede ser enorme; el
render solo paga la cáscara visible.

---

## 2. Arquitectura en capas

```
┌──────────────────────────────────────────────────────────────┐
│  dominium-sim  (5 capas f32 + agentes)   ← contenido, no motor │
└───────────────┬──────────────────────────────────────────────┘
                │  voxelizador (mapea grid+agentes → VoxelWorld)
                ▼
┌──────────────────────────────────────────────────────────────┐
│  llimphi-voxel                                                  │
│                                                                │
│  WorldStore ── ChunkStore (paletted, 32³) ── dirty-set         │
│       │                                                         │
│       ├── Mesher  (greedy + face-cull + AO)  ──► ChunkMesh      │
│       │        (incremental: solo chunks dirty)                 │
│       │                                                         │
│       ├── LightEngine (flood-fill sky+block)  ──► luz por voxel │
│       │                                                         │
│       └── EntityLayer (agentes → instancias)                    │
│                                                                │
│  Renderer3D (wgpu)                                              │
│       ├── Camera (view/proj) + Frustum                          │
│       ├── ChunkGPU (vertex/index buffer por chunk, cacheado)    │
│       ├── pase opaco (z-buffer) → pase transparente → entidades │
│       └── shaders WGSL (vertex + fragment, atlas/paleta, luz)   │
└───────────────┬──────────────────────────────────────────────┘
                │  textura/target
                ▼
        Llimphi (compose: el target 3D entra como una capa del View 2D)
```

El bucle Elm de Llimphi no cambia conceptualmente: `update(Msg)` muta el mundo /
cámara; `view()` declara un nodo "superficie voxel"; el `Renderer3D` pinta a una
textura que se compone en la escena. La diferencia con `paint_with` (que da una
`vello::Scene` 2D) es que acá hay un **pase de rasterización 3D propio** antes.

---

## 3. Subsistema por subsistema (destripado)

### 3.1 Modelo del mundo — `ChunkStore`

- **Voxel** = un `u16` índice a una **paleta por chunk** (palette compression). Un
  chunk de cielo/aire o de un solo material ocupa ~nada (paleta de 1-2 entradas +
  bitpacking). Minecraft usa esto (paletted containers) y es la clave del costo de
  memoria.
- **Chunk** = bloque de **32×32×32** (o 16³). 32³ = 32.768 voxels; con paleta +
  bitpacking, un chunk típico son KBs, no 64KB.
- **Coordenadas**: world → `(chunk_xyz, local_xyz)`. Hash map `ChunkCoord → Chunk`
  (mundo disperso/infinito) o un array ddenso (mundo acotado).
- **Altura**: para dominium probablemente basta `Y` acotado (p.ej. 0..128) — no
  necesitás cuevas profundas. Eso simplifica TODO (menos chunks verticales).
- **dirty-set**: `HashSet<ChunkCoord>` de chunks cuya geometría/luz quedó inválida
  (porque la sim cambió un voxel). El re-mesh y re-light solo tocan esos. **Este es
  el contrato que conecta sim↔render** (§6).

### 3.2 Contenido del mundo — voxelizador (dominium-específico)

Acá está el matiz que te ahorra MEDIO motor: **dominium no necesita generación
procedural de mundo (noise/biomas/cuevas).** El "mundo" lo produce la **sim**:

- Cada celda `(x,y)` del grid → una **columna** de voxels: altura = función del
  relieve (`poder`/`degradacion`/`materia`), material/color = mezcla de las 5
  capas (exactamente el `cell_color` que ya existe en `dominium-render-plan`).
- Agentes (lemmings) → entidades (§3.6), no voxels del terreno.
- Conceptos (iglesia/banco/…) → estructuras de voxels o sprites 3D.

O sea: el voxelizador es **un mapeo determinista grid→voxels** que ya tenés a medias
escrito en `render-plan` (solo que hoy emite rombos 2D en vez de poblar un
`ChunkStore`). Saltás todo el subsistema de world-gen de Minecraft (§7 lo cubre
solo si algún día querés "infinito real").

### 3.3 Meshing — de voxels a triángulos (el corazón)

Convertir voxels en una malla es donde se gana o se pierde. Tres niveles:

1. **Naïve**: por cada voxel sólido, 6 caras (12 tris). 32³ lleno = ~400k tris/chunk.
   Inviable.
2. **Face culling**: emitir una cara **solo si el vecino es transparente/aire**. Un
   bloque interno de montaña no emite nada. Reduce a la **superficie**: una montaña
   sólida = su cáscara. 10–50× menos tris. **Imprescindible.**
3. **Greedy meshing**: fusionar caras coplanares del mismo material en **rectángulos
   grandes**. Una pared de 16×16 caras iguales = **2 triángulos**, no 512. Otro
   5–20×. Algoritmo clásico (Mikola Lysenko): por cada uno de los 3 ejes × 2
   sentidos, barrés "slices" 2D y hacés merge greedy de quads. ~200 líneas, bien
   acotado.

Encima del meshing:
- **Ambient Occlusion por vértice**: oscurecer esquinas según los 3 vecinos
  diagonales (el truco de "0488: Ambient occlusion for Minecraft-like worlds"). Da
  el 80% del "se ve 3D y con volumen" por casi nada. Se calcula en el mesher y va
  como atributo de vértice.
- Salida: `ChunkMesh { vertices: Vec<Voxvert>, indices: Vec<u32> }`, donde
  `Voxvert = { pos: [f32;3], normal: u8 (6 dirs), ao: u8, light: u8, tile/color }`.
  Empaquetable a **8 bytes/vértice** (Minecraft empaqueta a 4-8 con bitfields).

**Incremental:** al re-mesh, solo los chunks `dirty` (y sus vecinos si una cara de
borde cambió). Nunca el mundo entero.

### 3.4 Render GPU — `Renderer3D` (wgpu)

Llimphi ya tiene wgpu (lo usa vello). Tier 2 agrega un **pase de triángulos 3D
propio** (no pasa por vello):

- Por chunk: subir `ChunkMesh` a un **vertex buffer + index buffer** en la GPU. Se
  sube una vez; se reusa cada frame. Si el chunk se re-meshea, se re-sube ese buffer.
- **Frame**: para cada chunk visible (tras frustum cull), `draw_indexed`. ~unos
  cientos de draw calls (o **multi-draw-indirect** para colapsarlas en 1, avanzado).
- **Depth buffer** (z-test): resuelve la oclusión por hardware. Adiós painter's
  algorithm y sobredibujo.
- **Frustum culling**: descartar chunks fuera del cono de cámara (test AABB-vs-6
  planos). Trivial y enorme.
- **Distance culling / render distance**: no dibujar más allá de N chunks.
- **Occlusion culling** (opcional, avanzado): no dibujar chunks tapados por otros
  (queries de oclusión / Hi-Z). Minecraft lo hace con un flood-fill de visibilidad;
  para un mundo acotado y abierto (dominium es un continente, no cuevas) **se puede
  omitir** al inicio.
- **Shaders (WGSL)**: vertex (transforma por view·proj, pasa normal/ao/light),
  fragment (color del atlas o de la paleta × AO × luz × directional simple). Dos
  pipelines: **opaco** (z-write) y **transparente/agua** (orden + blend) si hace
  falta.
- **Texturas**: o un **atlas** de tiles (estilo Minecraft) o **color por voxel**
  (más simple, encaja con el `cell_color` de dominium — probablemente esto).

### 3.5 Cámara 3D

- Matrices `view` (posición/orientación) y `proj` (perspectiva). El salto real:
  pasar de la **proyección isométrica fija 2.5D** actual a una **cámara 3D libre**
  (órbita alrededor del continente, o FPS/vuelo). Es ~un crate `Camera` chico
  (cuaterniones o yaw/pitch + un `glam`/`mint`).
- Para el *showreel*/demo: una **órbita + zoom** automática es trivialmente bonita y
  no necesita input.

### 3.6 Entidades (lemmings / "chanchitos")

- Miles de agentes → **instanced rendering**: un mesh chico (cubo/sprite/billboard)
  dibujado N veces con un buffer de instancias `{ pos, color, scale }`. Una draw
  call para todos. La GPU come cientos de miles de instancias sin pestañear.
- Update por tick de la sim → actualizar el buffer de instancias (no re-mesh).
- ECS opcional (no hace falta para solo dibujar; sí si querés lógica rica).

### 3.7 Iluminación

Dos caminos, de menor a mayor costo/calidad:
- **Barato (recomendado para arrancar):** AO por vértice (ya en el mesher) +
  **una luz direccional** (sol) en el fragment shader + un poco de ambient. Se ve
  muy bien y es casi gratis. dominium probablemente se queda acá.
- **Minecraft-style:** **flood-fill light** — propagación BFS de luz de cielo +
  luz de bloques por el grid de voxels, guardada por voxel, re-propagada en chunks
  dirty. Es de las partes más fiddly y caras del motor. Solo si querés cuevas
  oscuras / antorchas. **Para un continente abierto no se necesita.**

### 3.8 "Mundos infernales paralelos" (dimensiones)

Trivial conceptualmente: una **dimensión = un `WorldStore` independiente** con su
propio voxelizador/gen, su propio set de chunks y (opcional) su propia paleta/skybox.
"Viajar" = cambiar qué store renderiza la cámara (o un portal = teleport entre
stores). dominium ya tiene la noción de capas/Conceptos; un "infierno paralelo"
sería otra instancia de sim con otra calibración. **No agrega complejidad de motor**,
solo de contenido.

---

## 4. Cómo encaja con lo que YA existe

- **Llimphi**: ya trae wgpu (vía vello) y el bucle Elm + `gpu_paint_with`. Lo que
  falta es el **pase 3D de triángulos** (vello es 2D-vector). Es aditivo: el target
  3D se compone como una capa del `View`. No se toca vello.
- **dominium-render-plan**: hoy emite rombos iso 2D. En Tier 2 se **parte en dos**:
  (a) `cell_color`/mezcla de capas → **se reusa** (color del voxel); (b) la
  proyección iso 2D → **se reemplaza** por poblar el `ChunkStore`. O sea reusás la
  semántica de color, tirás la rasterización.
- **dominium-sim**: **no cambia**. Sigue mutando el grid; solo hay que marcar
  `dirty` las celdas que tocó cada tick para que el voxelizador/mesher actualice
  incremental (§6).

---

## 5. Las partes DIFÍCILES de verdad (lo que se subestima)

Ordenadas por "dónde se hunden los proyectos de voxels":

1. **Re-mesh incremental sin stutter.** Re-meshear un chunk en el hilo principal
   pincha el frame. Hay que meshear en **worker threads** (Llimphi `Handle` es
   `Send+Clone`, encaja) y subir el buffer en el siguiente frame. Manejar la cola de
   dirty con prioridad por distancia a cámara.
2. **El contrato sim↔render (dirty propagation).** dominium muta muchas celdas por
   tick. Si marcás todo dirty, re-meshás todo (= el problema de hoy). Hay que marcar
   **solo** las celdas que cambiaron de forma *visible* (cruzaron un umbral de
   altura/material) y batch-ear el re-mesh. Acá se gana o se pierde la fluidez.
3. **Greedy meshing correcto** (bordes entre chunks, AO en las costuras). El
   algoritmo es conocido pero los bordes de chunk y el AO en esquinas tienen casos
   sutiles.
4. **Culling correcto** (frustum bien, y si hacés occlusion, mucho más).
5. **Memoria y streaming** (solo si vas a "infinito": cargar/descargar chunks,
   generar en background, no leakear buffers GPU).
6. **Empaquetado de vértices y límites de la GPU** (vertex pulling, formatos).

Lo que **NO** es difícil (la gente cree que sí): la sim, el color, las entidades
instanced, la cámara. El infierno está en meshing+dirty+culling.

---

## 6. El puente sim↔render (lo más importante para dominium)

dominium es **dinámico** (la sim corre). Eso es más difícil que Minecraft, donde el
terreno es casi estático. El diseño:

```
cada tick de sim:
  world.step()                      // muta grid + agentes
  for celda cambiada:
     if cruza_umbral_visible(celda):   // altura/material cambió "de a un voxel"
        dirty.insert(chunk_de(celda))
  entidades.actualizar_buffer(agentes) // siempre, barato (instancing)

cada frame (desacoplado del tick):
  for chunk in dirty.drenar_por_prioridad(distancia_camara, presupuesto):
     mesh = mesher.greedy(chunk)        // en worker
     chunkgpu.subir(mesh)               // en main, próximo frame
  renderer.draw(camara, chunks_visibles)
```

Claves: (a) **cuantizar** el relieve a voxels para que cambios chiquitos de `f32`
NO disparen re-mesh (umbral), (b) **presupuesto** de re-mesh por frame (p.ej. máx 4
chunks/frame) para nunca pinchar, (c) entidades por instancing van aparte y siempre
baratas.

---

## 7. Mundo "infinito" (opcional, el tramo más caro)

Solo si algún día querés Minecraft-infinito de verdad (dominium probablemente NO lo
necesita — es un continente acotado). Agrega:
- **World-gen procedural**: noise 3D (Simplex/value), biomas (humedad/temp por
  noise), cuevas (3D noise threshold), estructuras. Para dominium esto lo
  reemplaza la sim, así que es contenido, no motor.
- **Streaming**: cargar chunks alrededor del jugador, descargar lejanos,
  generar/meshear en background, persistir a disco (CAS de tawasuyu encaja:
  chunk → BLAKE3). 
- **LOD** (level of detail): chunks lejanos con meshes más groseros (2³/4³ merge) o
  imposters. Lo que hace que el horizonte no cueste.
- Esto es **semanas extra** sobre el motor acotado. Es el 30% que cuesta el 60%.

---

## 8. Roadmap por fases (esfuerzo realista)

Suponiendo un dev competente + IA, sobre Llimphi (que ya tiene wgpu):

| Fase | Qué entrega | Esfuerzo |
|---|---|---|
| **M0 — pase 3D base** | crate `llimphi-3d`: cámara view/proj, depth buffer, un pipeline WGSL, dibujar un cubo en el `View` de Llimphi | ~3-5 días |
| **M1 — chunks + meshing** | `ChunkStore` paletted, mesher con **face-culling**, subir buffers, dibujar un mundo estático (dominium voxelizado, acotado) con cámara órbita | ~1-2 semanas |
| **M2 — greedy + AO + culling** | greedy meshing, AO por vértice, frustum/distance culling, color-por-voxel; mundo grande (256²×128) fluido | ~1-2 semanas |
| **M3 — dinámico (sim↔render)** | dirty-set + re-mesh incremental en workers + presupuesto por frame; dominium corriendo y el terreno actualizándose fluido | ~1-2 semanas |
| **M4 — entidades + luz** | agentes por instancing, sol direccional + AO, Conceptos como estructuras | ~1 semana |
| **M5 (opcional) — dimensiones** | múltiples `WorldStore`, switch/portales | ~días |
| **M6 (opcional) — infinito** | world-gen noise, streaming, LOD, persistencia CAS | ~3-6 semanas |

**Total para un dominium 3D voxel sólido y fluido (M0–M4): ~5-8 semanas.**
**Minecraft-infinito real (hasta M6): +1.5-2 meses encima.**

Riesgo concentrado en M3 (incremental dinámico) — ahí está el 70% del riesgo del
proyecto. M0-M2 son camino conocido.

---

## 9. Decisiones que recortan alcance (recomendadas para dominium)

dominium **no es Minecraft**, así que casi todo lo caro se puede saltar:
- **Mundo acotado**, no infinito → fuera streaming, LOD, world-gen, persistencia
  (−M6, el tramo más caro).
- **Altura corta** (0..128), sin cuevas → fuera flood-fill light (−§3.7 caro), AO
  + sol alcanza.
- **Color por voxel**, sin atlas de texturas → fuera el pipeline de texturas.
- **Sin occlusion culling** (continente abierto) → frustum+distancia alcanza.
- **Cámara órbita** (no FPS/input rico) para el demo/visualización.

Con esos recortes, "Tier 2 para dominium" = **M0-M4, ~5-8 semanas**, y queda un
motor voxel 3D dinámico genuino, reusable por cualquier app de la suite (no solo
dominium: cualquier cosa que quiera mostrar un mundo voxel).

---

## 10. Resumen ejecutivo

- El cuello de hoy es el **render** (regenera 10k paths vectoriales 2D por frame, sin
  caché ni culling, en SW en el reel), **no la sim**. Minecraft vuela porque cachea
  meshes en GPU, cullea, y solo dibuja la cáscara visible.
- Tier 2 = traer esas 5 decisiones a Llimphi como un **pase 3D voxel** (`llimphi-voxel`),
  con dominium-la-sim como contenido (la sim *genera* el mundo, así que te ahorrás
  el world-gen de Minecraft).
- Es **muy posible**, no magia. Acotado y dinámico: **~5-8 semanas** (M0-M4).
  Infinito de verdad: **+~2 meses**.
- El riesgo real está en **el re-mesh incremental dinámico** (sim↔render), no en lo
  3D en sí. Lo 3D base (M0-M2) es camino trillado.
- Vive en **Llimphi**, no en dominium — es capacidad de motor, reusable por la suite.

> Nota de realismo: esto es lo más ambicioso de los tres tiers. Si el objetivo es
> "que dominium deje de comer FPS y se vea lindo", **Tier 0+1 (días-semanas) ya lo
> logran** sin nada de esto. Tier 2 es para cuando la meta sea, literal, "un
> Minecraft" — un motor 3D voxel propio en la suite.

---

## 11. DECISIÓN y dirección moderna (2026-06-17)

**Se hace.** Pero como **motor 3D general de Llimphi**, NO por dominium —
dominium fue solo la puerta de entrada. Crate(s): `llimphi-3d` (base: cámara,
depth, pipeline) + `llimphi-voxel` (el voxel propiamente). Cualquier app de la
suite lo reusa (cosmos esfera celeste nativa, supay, viz científica, juegos).
**Timing:** después de pulir pata + cerrar la publicación. Por ahora, los FPS de
dominium se atacan con **Tier 0+1** sobre el render 2.5D actual (en curso), que
NO requiere nada de este motor.

### 11.1 Ruta elegida: **ray-marching / voxel sparse**, no mesh clásico

El §3.3 (greedy meshing → triángulos) es la ruta *clásica, probada y estática*. Para
un motor **dinámico** (que es lo que la suite quiere: mundos que cambian, sims, no
terreno congelado) la ruta **moderna y mejor encajada es no meshear**:

- En vez de voxels → malla de triángulos, se **marcha un rayo por píxel** a través
  de una estructura de voxels (**DDA** lineal / **SVO** = Sparse Voxel Octree /
  **brickmap**) en un **compute/fragment shader**. El color/normal sale del voxel
  que el rayo toca primero.
- **Ventaja decisiva**: **no hay re-mesh.** Eso **elimina M3** (el 70% del riesgo del
  proyecto: el re-mesh incremental dinámico). Mutás un voxel → se actualiza la
  estructura en GPU y el próximo frame ya lo ve. Para dominium (muta cada tick) y
  para destrucción/edición en vivo, es el paradigma correcto.
- **Costo**: los shaders son más difíciles (traversal, manejo de la estructura
  sparse en GPU), y para mundos enormes hay que mantener el SVO/brickmap actualizado
  en VRAM. Pero baja el riesgo total al sacar el pipeline de meshing entero.
- **Híbrido** (futuro): mesh para terreno cuasi-estático + ray-march para lo
  dinámico. Para arrancar: comprometerse a ray-march.
- **Referencias**: Laine & Karras, *Efficient Sparse Voxel Octrees* (NVIDIA 2010);
  Teardown (renderer voxel ray-traced); gabe rundlett / **gvox / Voxelite**;
  **NanoVDB** (estructura sparse GPU, grado film); Amanatides-Woo (DDA voxel).

### 11.2 Roadmap re-planteado para ray-march

| Fase | Qué entrega | Esfuerzo |
|---|---|---|
| **M0 — pase 3D base** | `llimphi-3d`: cámara view/proj, depth, target 3D compuesto en el `View`, un triángulo/cubo de prueba | ~3-5 días |
| **M1 — voxel store + DDA** | estructura de voxels (densa acotada para empezar) en GPU (textura 3D / buffer) + shader de **ray-march DDA** que la dibuja; cámara órbita | ~1-2 semanas |
| **M2 — sparse + color/AO/luz** | SVO o brickmap (saltar el aire), color por voxel, AO/normal en el hit, sol direccional | ~2 semanas |
| **M3 — dinámico** | actualización incremental de la estructura GPU al mutar voxels (dominium corriendo y editándose en vivo) — **mucho más barato que el re-mesh** del paradigma clásico | ~1 semana |
| **M4 — entidades** | agentes por instancing o como voxels en la misma estructura | ~días-1 sem |
| **M5 — dimensiones** | múltiples `WorldStore` (un `Multiverse`), switch/portales | ✅ hecho |
| **M6 — mundo grande / "infinito"** | world-gen + atmósfera (✅) + streaming de ventana sobre mundo ilimitado (✅ 1ª rebanada) → falta shift incremental + LOD + persistencia CAS | en curso |

**Estado (2026-06-17):** M0–M5 cerrados (ray-march DDA de dos niveles sobre brick
pool sparse, AO + sol + sombras, mutación incremental por `DirtyBox`/`sync`,
entidades analíticas, `Multiverse`). **M6 — primera rebanada hecha:** world-gen
procedural (`llimphi_3d::terrain`: heightmap fbm con océanos/playa/pasto/roca/nieve
+ árboles, sin deps de ruido) y **atmósfera** (`Atmosphere`: cielo gradiente con
disco solar + niebla por distancia, opt-in vía `fog_density` para no alterar la
composición clásica sobre vello). Demo verificado por PNG headless:
`cargo run -p llimphi-3d --example terrain_demo --release` → /tmp/m6_terrain_*.png.
Encima: **cámara libre / primera persona** (`Camera3d::fly`, complementa a `orbit`
para ver el mundo *desde adentro*), `VoxelGrid::height_at` (posar cámara/entidad
sobre el relieve) y **atmósfera por dimensión** (`Dimension::with_atmosphere` — cada
mundo de M5 con su cielo/niebla). Demos: `terrain_flythrough` (vuelo bajo por el
paisaje, /tmp/m6_fly_*.png) y `voxel_dimensiones` con skies temáticos.

**M6 — streaming (primera rebanada HECHA, 2026-06-17):** mundo procedural
**ilimitado** sobre una ventana voxel acotada que se desliza. El terreno se
redefinió como **función pura de mundo** (`llimphi_3d`/`llimphi_voxel::column_height`:
el mismo `(wx,wz)` da el mismo relieve en cualquier ventana) + `fill_terrain_window`
(rellena una ventana cuya esquina cae en una columna de mundo arbitraria) +
`VoxelGrid::clear_all` (regenerar in-place dejando dirty completo). Encima,
`llimphi_voxel::WorldStream`: ventana de `dim` con `origin` de mundo + `step`;
`follow(cx,cz)` la reubica (snap a `step`) y regenera sólo al **cruzar un paso** —
O(ventana) por reubicación, no por frame. Como todo sale de `column_height`, las
**costuras encajan**: se camina sin fin, sin muro ni repetición. Verificado: 2
tests CPU duros (alturas independientes de la ventana; dos ventanas solapadas
coinciden voxel-a-voxel >10k columnas) + demo PNG `terrain_streaming` (la ventana
sigue al foco de mundo z:-80→520 y cada cuadro rinde **paisaje nuevo y distinto** —
isla boscosa verde con agua → cordillera nevada — sin huecos; ocupación de bricks
distinta por cuadro). **MVP:** la regeneración es de la **ventana entera** por paso
cruzado (no un *shift* parcial que sólo rellene la franja nueva); el demo reconstruye
el `VoxelRenderer` por cuadro (el camino incremental `sync` existe, pero su brick
pool aún no crece si una ventana más densa lo llena). Queda de M6: **shift
incremental** (copiar la zona común + generar sólo el borde nuevo, vía toroidal
addressing en el shader o `sync` con pool que crece), **wiring en la app viva**
(coordenadas del jugador/HUD bajo la ventana móvil — necesita pantalla), persistencia
CAS de chunks y **LOD** del horizonte — el tramo caro de §7.

**Total motor dinámico sólido (M0-M4): ~5-7 semanas** (similar al mesh clásico, pero
con el riesgo movido de "re-mesh" a "shaders de traversal", que es dominio más
acotado).

### 11.3 Build vs compose (resuelto)

- **Componer**: `glam` (math); para la estructura sparse, mirar `NanoVDB` (FFI,
  evaluar) o implementar brickmap propio (no es enorme). Si algún día se hace mesh,
  `block-mesh-rs`.
- **NO meter Bevy**: es un motor entero con su ECS/render-graph/app — sería un
  **segundo motor** peleándose con el bucle Elm + vello + wgpu de Llimphi, y
  contradice el ethos soberano de la suite. **Veloren** (RPG voxel Rust sobre wgpu)
  es **referencia/prueba de que el stack funciona**, no dependencia.
- **Capas**: el motor NO compite con OpenGL — va **sobre wgpu** (que Llimphi ya usa),
  que a su vez traduce a Vulkan/Metal/DX12/GL/WebGPU.

### 11.5 Motor 3D GENERAL + arquitectura en 3 capas (2026-06-17)

El usuario fijó el norte: **3D general completo** (no "sólo voxels"), bien
modularizado, con **una app propia** que arranca como demo y puede ganar
personalidad; y que haya una **rama de dinámica tipo-Minecraft reusable como
librería por cualquier juego** de esa orientación. Resultado:

- **`llimphi-3d` = motor 3D GENERAL** (agnóstico de contenido). Keystone nuevo:
  **`Scene3d`** compone, en un único pase con **depth buffer compartido**, el
  ray-march de voxels (que ahora escribe `frag_depth`) y mallas de triángulos
  (`Renderer3d`, con `set_model`) → **voxels y polígonos se ocluyen mutuamente**.
  Eso es lo que lo vuelve "3D general" y no un motor voxel a secas. (Camera3d
  `zfar` 100→5000 y voxel depth_compare `LessEqual` por el frag_depth; ver
  commit.)
- **`llimphi-voxel` = librería de dinámica voxel/juego** sobre `llimphi-3d`:
  world-gen (`terrain`, movido acá desde el motor) + casa futura de bloques/
  biomas/streaming. NO renderiza; aporta contenido. Reusable por cualquier juego
  voxel.
- **`llimphi-voxel-app` = app showcase** (nombre provisional, rebrandeable): un
  mundo procedural orbitable con atmósfera + un monumento-malla flotante que
  prueba voxel+triángulos en vivo. Modular (`world.rs` contenido / `main.rs`
  bucle), con `--shot` headless.

Capas: `app → llimphi-voxel → llimphi-3d → wgpu`. Verificado por PNG
(`scene_mixed`: esfera voxel ∩ cubo-malla; `voxel_app --shot`: mundo+monumento
por el compositor real). Si en el camino sale un Minecraft, bienvenido — la capa
`llimphi-voxel` es justo esa rama.

**Caminar el mundo (2026-06-17).** La rama `llimphi-voxel` ganó su segundo
ingrediente de juego (tras el picking de `raycast`): **física de jugador en
primera persona** (`llimphi_voxel::Player`) — caja AABB con gravedad, salto y
**colisión move-and-resolve** eje por eje contra el `VoxelGrid`, más helpers de
base (`forward_h`/`right_h`/`look_dir`). No toca GPU; reusable por cualquier
juego voxel. La app showcase suma un **modo "explorar"** (Tab) que alterna la
órbita con una **cámara en primera persona** (`Camera3d::fly`) caminando el
terreno (WASD + Espacio para saltar), con romper/construir (`b`/`g`) por raycast
desde el ojo del jugador. Verificado: 3 tests de física (caída→suelo, muro frena,
salto sólo desde el piso) + PNG `voxel_app_fps.png` (parado en la orilla mirando
el continente, por el camino real de `step_player`). La **mira/HUD** se cerró aparte (ver abajo): un pase screen-space en GPU
*después* del ray-march, porque el crosshair vello queda tapado por el canvas GPU
full-screen y el `view_overlay` modal congelaría el mouse-look.

**Manada viva — M4 cerrado en la app (2026-06-17).** El motor ya tenía la *capa*
de entidades (`Entity3d`: cajas analíticas ray-marcheadas en el mismo pase, hasta
64, con sombra; demo `voxel_entities_demo`), pero static. Ahora la rama
`llimphi-voxel` aporta la *voluntad*: **`Critter`** — un agente que **deambula**
reusando exactamente la física de `Player` (gravedad + colisión), con IA mínima
determinista (LCG por bicho: cambia de rumbo cada tanto, salta a veces, gira al
chocar contra una pared). Para que el cuerpo sirva a ambos, `Player` ganó un
campo `speed` (el jugador anda rápido, el bicho pasta lento). La app suelta una
**manada** (`World::tick` deambula y vuelca `Critter::entity()` a
`VoxelRenderer::entities` cada frame; instancing analítico, barato). Verificado:
3 tests de `Critter` (deambula sin hundirse, rebota y queda en el corral, dos
semillas divergen) + PNG `voxel_app_fps.png` (una "oveja" voxel parada en la
orilla, encuadrada por `World::nearest_critter`). Con esto **M0–M5 + M4
(entidades con conducta) están cerrados**.

**HUD / mira en GPU (2026-06-17).** Nuevo primitivo de motor `llimphi_3d::Hud`:
un pase **screen-space** tonto (rectángulos de color con alpha en NDC, sin
texturas/bind-groups/depth) que se pinta *después* del 3D, en la misma closure
`gpu_paint_with` — la única forma de poner un overlay **encima** del canvas GPU
full-screen (el vello del árbol queda debajo, y el `view_overlay` modal mataría
el mouse-look). `HudQuad::crosshair` arma la mira centrada; el modo explorar la
dibuja sobre la escena. Verificado: la cruz blanca aparece al centro en
`voxel_app_fps.png`. Reusable para barras/marcos/HUD de cualquier app 3D.

**Texto en el HUD (2026-06-17).** `HudQuad::text` suma una **fuente bitmap 5×7**
embebida (`glyph`: `0-9`, `A-Z`, puntuación) — cada píxel encendido = un quad,
así que el texto sale sin texturas/bind-groups, dentro del mismo pipeline tonto
del HUD. La app showcase pinta en explorar un panel translúcido con el **modo**
(«EXPLORAR») y las **coordenadas** del jugador en vivo (lectura de `Player::pos`).
Verificado por PNG (`voxel_app_fps.png`: «EXPLORAR / X 38 Y 23 Z 38» legible
arriba-izquierda + mira al centro). Reusable para cualquier overlay 3D con texto.

Queda sólo el tramo caro de M6: **streaming/LOD del horizonte** (chunks que
cargan/descargan alrededor de la cámara + persistencia CAS + meshes groseros de
fondo) — ver §7.

### 11.4 Esfuerzo vs el kernel de wawa

El motor voxel es **~1/3–1/2 del esfuerzo del kernel wawa y con mucho menos riesgo**:
wawa es un SASOS bare-metal desde cero (territorio research); esto es una técnica
gráfica **conocida y documentada** sobre un stack GPU que ya existe. La ruta
ray-march mueve el esfuerzo de "pipeline de meshing" a "shaders de traversal" —
distinto, pero dominio acotado. wawa fue el monte; esto es una colina con sendero.

## 12. Rama machinima — "filmar" escenas voxel (2026-06-17)

Norte nuevo del usuario, y el *porqué* original de querer un Minecraft: **generar
escenas controladas, mover personajes y filmar una película**. Análisis: es más
práctico de lo que suena, porque la mitad cara (un renderer que dé cuadros limpios,
sin pantalla, reproducibles) **ya está** — `--shot` rinde headless a PNG de forma
determinista. El hueco no es gráfica sino una **capa de dirección** (cámara con
keyframes, actores articulados, timeline) + pipeline de assets. Techo estético:
**machinima voxel estilizado** (no foto-realismo — ese es el atractivo). Donde se
gana/pierde: la **expresividad del personaje** (los muñecos de cajas dan "animación
de Minecraft", no actuación matizada).

> **Guía práctica de uso** (cómo correr los modos, escribir un guion, agregar
> clips, importar `.vox`, componer la banda sonora):
> `02_ruway/llimphi/llimphi-voxel-app/README.md`. Esta sección es el *diseño/porqué*;
> el README es el *cómo*.

### 12.1 Rebanada vertical — HECHA y verificada

Pipeline punta a punta `--film` (`llimphi-voxel-app`), determinista y sin pantalla,
que junta cuatro piezas nuevas:

- **`llimphi_3d::CameraTrack` + `CamKey`** (motor general): cámara **guionada** por
  keyframes `(t, eye, target, fov)` interpolados con smoothstep → travelling/grúa/
  corte sin input. 3 tests.
- **`llimphi_3d::push_cube` / `Renderer3d::set_geometry`**: componer mallas
  multi-caja en CPU y **re-subir geometría por frame** (un muñeco que se re-posa).
- **`llimphi_voxel::Actor`**: **muñeco de cajas articuladas** (cabeza/torso/2 brazos/
  2 piernas), miembros que rotan en cadera/hombro con un **ciclo de caminata**
  procedural; produce malla en espacio local + `model()` de ubicación. No toca GPU;
  se compone con el terreno en `Scene3d` (oclusión correcta). 4 tests.
- **`foreign_av::encode_frames`**: secuencia de PNG → video **AV1** (libsvtav1,
  `-pix_fmt yuv420p`), audio opcional a Opus.

`World::render_with` agrega las mallas-actor al pase voxel+monumento (depth
compartido); el casting busca **tierra firme** (`find_land_strip`) para no filmar
gente sobre el agua. **Verificado**: `cargo run -p llimphi-voxel-app --release --
--film` → `/tmp/voxel_film.mkv` (AV1, 960×540, 120 cuadros, 4 s) con 3 actores de
colores caminando un pico nevado mientras la cámara hace grúa + travelling y el
monumento-malla flota al fondo (montaje de cuadros mirado a PNG).

### 12.2 Qué falta para un "director" de verdad (orden de peso)

1. ~~**Expresividad de actor**: librería de clips + blending~~ **HECHO**:
   `llimphi_voxel::Clip` (Idle/Walk/Run/Wave/Point/Cheer) — un clip es una función
   `fase → Pose` (ángulos de todas las articulaciones), así sumar una animación es
   escribir una pose, no tocar el render. `Actor` tiene `clip`/`phase`/`set_clip`/
   `advance`/`pose`. **Blending**: cambiar de clip hace un **cross-fade** (`Pose::
   lerp` + smoothstep, `BLEND_DUR`) en vez de cortar. Verificado por PNG (`--poses`
   → `/tmp/actor_clips.png`: fila de 6 actores etiquetados, cada pose legible) +
   tests. **Falta**: IK, animaciones de cara/manos (el muñeco no las tiene).
2. ~~**Timeline de dirección**~~ **HECHO** (módulo `llimphi_voxel::director`):
   timeline **determinista por tiempo**, editable como data (no más bucle
   hardcodeado). `ActorScript` = keyframes `(t, pos grilla, clip?, rumbo?)` con
   `sample(t)` (interpola posición, decide clip auto camina/quieto, gira suave el
   rumbo); `Shot` = un plano (`CameraTrack`) con instante de inicio → varios planos
   dan **cortes duros**; `Sequence` = reparto + planos + duración (`camera(t)`,
   `cast_centroid(t)`). El `--film` ahora reproduce un `screenplay()` (tres actores
   entran caminando, se giran a cámara y gesticulan; dos planos con corte). 2 tests.
3. ~~**Assets**: importador `.vox`~~ **HECHO**: puente `shared/foreign-vox`
   (lee/escribe el formato MagicaVoxel — chunks `MAIN/SIZE/XYZI/RGBA`, bytes LE,
   sin deps; `VoxModel` neutral + paleta) + conversor `llimphi_voxel::{model_to_grid,
   stamp,load_grid}` (remapea `z`-arriba del `.vox` a `y`-arriba del motor; `stamp`
   compone sets metiendo piezas en un grid). Verificado por PNG (`--vox`: genera un
   golem → `foreign_vox::write` → `.vox` → `load_grid` → render; el muñeco con ojos/
   antena se lee) + 5 tests (ida y vuelta, bytes a mano, remapeo de ejes). **Falta**:
   importar la **paleta oficial** de MagicaVoxel (hoy el fallback sin `RGBA` es una
   rampa HSV; los exportes reales traen `RGBA` y van bien) y soporte de escenas
   multi-modelo con transformación (`nTRN/nGRP`).
4. ~~**Iluminación cinematográfica**~~ **PRIMER PASO HECHO**: la luz del ray-march
   pasó de escalar (ambiente plano 0.32) a **con color** — color del sol por su
   elevación (cálido al ras → blanco en lo alto) + ambiente tintado por el color de
   cielo (rebote frío), **sin uniforms nuevos** (sale de `sun_dir`/`sky_zenith` que
   ya viajan). El mood se controla moviendo `sun_dir` y la paleta de cielo.
   Verificado por PNG (golem/actores con matiz cálido-frío, misma luminancia base).
   **Falta**: luces puntuales/coloreadas explícitas, sombras suaves.
5. ~~**Calidad de cuadro**: supersampling~~ **HECHO**: SSAA — el `--film`/`--vox`
   renderizan a **2×** (`SSW×SSH`) y bajan promediando bloques 2×2
   (`write_png_downsampled`) → antialias de los bordes duros del ray-march.
   Verificado por PNG (zoom: siluetas suaves, no escalonadas). `--poses` queda 1×
   (su HUD se mide en pixels de pantalla). **Falta**: resolución de cine (1080p/4K) y
   TAA si se quisiera abaratar.
6. ~~**Audio**~~ **HECHO**: la película tiene **banda sonora**. `soundtrack.rs`
   compone un `Score` con `takiy-core` (progresión I–V–vi–IV: pads + bajo + una
   melodía que sube al momento del gesto) y lo sintetiza a WAV con `takiy-synth`
   (`OscRenderer` triángulo + reverb suave); `--film` lo muxea con
   `foreign_av::encode_frames` (audio → Opus, `-shortest`). Verificado por ffprobe
   (el `.mkv` lleva stream Opus estéreo, dur 5.6 s) + `volumedetect` (media −14 dB,
   no silencio). **Falta**: sincronizar *hits* musicales a beats del guion
   (hoy música y acción sólo comparten duración), SoundFonts/instrumentos reales.

Encaja como **app/dominio "director" propio** sobre `llimphi-voxel` (hermano de la
showcase), no como feature del motor. La rebanada prueba que el camino es viable y
barato; lo que sigue es content tooling, no investigación gráfica.

