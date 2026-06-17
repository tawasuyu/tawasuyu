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
| **M5/M6 (opcional)** | dimensiones múltiples / streaming "infinito" con brickmaps | semanas |

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

### 11.4 Esfuerzo vs el kernel de wawa

El motor voxel es **~1/3–1/2 del esfuerzo del kernel wawa y con mucho menos riesgo**:
wawa es un SASOS bare-metal desde cero (territorio research); esto es una técnica
gráfica **conocida y documentada** sobre un stack GPU que ya existe. La ruta
ray-march mueve el esfuerzo de "pipeline de meshing" a "shaders de traversal" —
distinto, pero dominio acotado. wawa fue el monte; esto es una colina con sendero.

