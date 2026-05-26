# supay — modernizar Doom sin tocar su alma

> Supay (quechua: espíritu del inframundo). Tipo: **juego retrocompatible con renderer moderno**.

## Tesis

Tomar la simulación bit-exact de Doom (ticks 35 Hz, BSP, RNG, hitboxes, demos `.lmp` reproducibles) y reemplazar solo la **percepción visual** con un renderer moderno. **No** reescribir Doom como FPS contemporáneo: en cuanto cambia un timing, una fricción, un quirk de colisión, deja de sentirse Doom.

> Modernizar la percepción, no el comportamiento.

## Arquitectura — 3 capas estrictas

```
[ CUADRANTE III · 0x02 RUWAY ]

3. supay-render-llimphi    — Renderer wgpu 3D (corre a 144+ Hz por interpolación)
   │                          (mesh cache, sprite relighting, RT shadows opt-in,
   │                           volumetric fog, TAA, ACES tonemap)
   ▼
2. supay-scene             — Scene extractor (read-only sobre supay-core)
   │                          (walls visibles, sprites, sector lights, fx flags;
   │                           snapshot inmutable por tick para interpolar)
   ▼
1. supay-core              — Lógica Doom intacta (tick 35 Hz)
   │                          (Fase 0: raycast hardcoded; Fase 1: FFI a doomgeneric;
   │                           Fase 2: port nativo Rust con `cc` compilando id1 modificado)
   ▼
[ HARDWARE · GPU vía Llimphi-HAL ]
```

**Contrato hardline:** las demos `.lmp` deben reproducir bit-exact en cualquier renderer. El extractor de escena es **read-only**, sin side-effects sobre la simulación. Test suite que checksumea estado por tick — cualquier cambio del renderer que la rompa es bug.

## Fases de forja

### Fase 0 — Hello inframundo (este bloque)

**`supay-app-llimphi`** — app standalone con un raycaster estilo Wolfenstein/Doom-early para validar el pipeline sin pelearse con FFI todavía:

- Mapa 16×16 hardcoded (grilla binaria, paredes con material por celda).
- Jugador con `(x, y, angle)` + movimiento WASD + giro con flechas.
- Tick deterministic a 35 Hz vía `Handle::spawn_periodic`.
- Render desacoplado vía `View::paint_with`: raycast columna por columna, alturas calculadas con perpendicular distance (no fish-eye), shading por distancia, niebla volumétrica.
- Sin Doom real todavía — pero el modelo "tick separado del render" queda probado.

### Fase 1 — Doom real (en código)

**`supay-core`** (Fase 1.0, 2026-05-25): andamiaje completo.
- `Cargo.toml` con `links = "doomgeneric"` y `build = "build.rs"`.
- `build.rs`: busca `vendor/doomgeneric/doomgeneric/*.c`; si existe los compila con `cc` (excluye los `doomgeneric_<plataforma>.c` para no tener doble-host), define `FEATURE_SOUND=0`, flags `-w` para silenciar warnings legacy del id1. Si no existe, emite `cfg(doomgeneric_stub)` para que el lib compile como no-op y el workspace siga verde.
- `src/lib.rs`: exporta callbacks `extern "C" #[no_mangle]` que doomgeneric llama (`DG_Init`, `DG_DrawFrame`, `DG_SleepMs`, `DG_GetTicksMs`, `DG_GetKey`, `DG_SetWindowTitle`); todos delegan a un `HostState` singleton (`OnceLock<Mutex<...>>`) que mantiene el framebuffer copiado + FIFO de input + ticks desde arranque + título solicitado. API safe envuelta en `DoomEngine::{new, tick, push_key, framebuffer, title}`. Módulo `keys` con códigos canónicos Doom (`KEY_FIRE`, `KEY_USE`, los `KEY_*ARROW`, etc.).
- `vendor/README.md` explica cómo bajar doomgeneric (`git clone https://github.com/ozkl/doomgeneric.git`) + WAD shareware.

**`supay-doom-llimphi`** (Fase 1.0): consumidor.
- App Llimphi que crea `DoomEngine::new(["doomgeneric", "-iwad", "doom1.wad"])`, dispara tick a 35 Hz vía `Handle::spawn_periodic`, lee el framebuffer 320×200 u32 ARGB, lo convierte a Rgba8 (forzando alpha 255 porque doomgeneric llena el canal alpha con 0) y lo pinta como `View::image` con aspect-fit. `on_key` traduce eventos Llimphi → códigos Doom (`Key::Named(ArrowUp)→KEY_UPARROW`, `'w'→KEY_UPARROW`, `'a'→KEY_LEFTARROW`, `Space→KEY_USE`, `Control→KEY_FIRE`, etc.) y los enqueuea con `push_key`. F12 cierra la ventana.
- En modo stub (sin vendor), arranca igual y pinta un panel explicando cómo proveer doomgeneric + WAD. Útil para validar plumbing Llimphi sin requerir el clone.

**Cómo correr** (cuando vendor está provisto + DOOM1.WAD en cwd):

```sh
cargo run -p supay-doom-llimphi --release
```

### Fase 2 — Scene extraction

Parcho a doomgeneric para exponer estado interno (linedefs, sprites, sectors, brightness) además del framebuffer. Crate `supay-scene` mantiene snapshots inmutables por tick que el renderer consume con interpolación entre dos snapshots consecutivos para alcanzar 144+ Hz suave. El framebuffer original deja de pintarse — solo es referencia de validación.

**Fase 2.0 (2026-05-25, este bloque):** andamiaje del modelo de escena.

- **`supay-scene`** (crate nuevo, puro Rust, sin FFI): tipos inmutables `SceneSnapshot { tick, player, walls: Arc<[WallSeg]>, sectors: Arc<[SectorSnap]>, sprites: Arc<[SpriteSnap]> }`. `Arc<[T]>` permite que un snapshot clonado sea bumping refcount, sin pagar copia de listas grandes (mapas Doom tienen ~1000 linedefs). `SnapshotPair` rotatorio (prev + next) que el renderer mantendrá vivo. `interpolate(prev, next, alpha)` lineal en player/sectores/sprites, arc-shortest en ángulos (maneja wraparound 2π — sin esto, girar de 350° a 10° tarda 180° en la interpolación). Walls no se interpolan: en Doom las linedefs son inmóviles, lo que se mueve son alturas de sectores (doors, lifts) que sí entran al lerp. Si las longitudes de sectores/sprites difieren entre snapshots (spawn / destroy de mobj entre ticks) cae a `next` puro — el sprite nuevo aparece en su pos real, no hay glitch. Cinco unit-tests + 1 doc-test cubren rotación de `SnapshotPair`, midpoint del lerp, arc-shortest del ángulo, clamping de `alpha` fuera de rango y fallback por mismatch de longitudes.
- **`supay-core` extensión**: dep `supay-scene`, re-exporta los tipos. `DoomEngine::capture_scene(tick) -> SceneSnapshot`. En **modo stub** genera un snapshot sintético — sala 8×8 con cuatro paredes, un sector con brightness pulsando sinusoidalmente, jugador caminando en círculo y un sprite siguiéndolo a dos unidades por detrás. Permite avanzar el renderer (Fase 3) sin vendor doomgeneric. En **modo real** llama a getters C de `src/scene_export.c` (ver abajo). El cache interno de mobjs no es thread-safe — se exige llamar `capture_scene` desde el mismo thread que `tick()` (el host de Llimphi lo cumple por construcción: ambos viven en el event loop).
- **`supay-core/src/scene_export.c`** (~150 LOC C): glue que expone estado interno de doomgeneric. Convierte fixed-point 16.16 a `float` (división por 65536) y `angle_t` 32-bit a radianes. Lee `lines[]`, `sectors[]`, `players[consoleplayer]` y camina la lista enlazada `thinkercap` filtrando thinkers cuyo callback es `P_MobjThinker` (i.e. mobjs). Cachea los punteros a mobjs en `supay_scene_num_sprites` para que `supay_scene_sprite(i)` sea O(1) sin recorrer la lista N veces. El archivo se compila junto con audio_stubs.c cuando vendor doomgeneric existe; en modo stub `build.rs` ni lo toca, así el LSP de clang protesta por headers no encontrados (`doomdef.h` etc.) — esperado.
- **`supay-doom-llimphi` wiring**: `Model` gana `snapshots: SnapshotPair`. Cada `Msg::Tick` captura snapshot tras `engine.tick()` y lo empuja al par. El header muestra `scene[w=N sec=N spr=N]` con los conteos del último snapshot — evidencia visual de que el plumbing camina. El renderer 3D (Fase 3) reemplazará el `View::image` del framebuffer por un pase wgpu que consuma `pair.prev() + pair.next() + alpha`.

Pendiente para Fase 2.1 (cuando lo demande el renderer 3D): BSP / segs / subsectors / nodes para front-to-back ordering correcto. Linedefs alcanzan para una primera pasada wireframe + paredes sin orden.

### Fase 3 — Renderer 3D moderno

`supay-render-llimphi` deja `View::paint_with` y pasa a `View::custom_pass(wgpu)` (feature nueva de llimphi-ui a agregar). Pipeline:

- **Mesh cache** linedef → vértices; invalida solo en cambio (linedef movible: doors, lifts).
- **Sprites**: billboards Y-up con BRDF basado en sector lights — el sprite reacciona a la iluminación real.
- **Iluminación**: sector brightness como point lights atenuadas; volumetric fog por sector.
- **Shadows**: stencil shadows desde sector lights (baseline); RT shadows si `VK_KHR_ray_tracing_pipeline` está y el usuario opta in.
- **Temporal**: TAA accumulation; ACES tonemap.

**Fase 3.0 (2026-05-25, este bloque):** renderer 3D software sobre vello.

- **`supay-render-llimphi`** (crate nuevo). `scene_view(pair, last_tick_at, tick_period, config) -> View<Msg>` devuelve un nodo Llimphi que en su `paint_with` proyecta el snapshot interpolado a polígonos vello. Sin wgpu directo todavía: llimphi-ui hoy expone `View::paint_with(vello::Scene)` pero no `View::custom_pass(wgpu)` — esta fase valida la cadena `snapshot → renderer` con el surface existente. Cuando llimphi-ui gane el custom_pass (Fase 3.1+), el back-end migra a wgpu nativo sin tocar los tipos de `supay-scene` ni la API pública.
- **Pipeline.** Cada frame: interpolar `prev`/`next` con `alpha = elapsed/TICK_PERIOD` → cámara 2D (rotar mundo por `-player.angle` alrededor de Z, +X_cam=adelante, +Y_cam=derecha, +Z_cam=arriba) → back-face cull (convención Doom: front side donde `(v2-v1)×(pt-v1)_z < 0`) → near-clip 2D del linedef contra `X_cam = near` (parametric `t = (near - x1)/(x2 - x1)`) → slabs visibles (one-sided: `[floor, ceiling]` completo; two-sided: lower si `far.floor > near.floor` y upper si `far.ceiling < near.ceiling`) → proyección perspectiva con `focal = h/(2·tan(fov_y/2))`, pixels cuadrados → painter's algorithm por distancia euclidiana en cámara → vello fill (BezPath de 4 puntos por slab).
- **Shading.** `shade = light_level/255 · fog_factor` con `fog_factor = max(0.2, 1 - depth/far_fog)`. Color base por paleta indexada por `front_sector` (6 entradas: tan, gray-tan, brown-red, slate, sand, dark-gray) — variedad sin texturas. Sprites pintados como billboards Y-up rojizo-oranje (≈ Imp) con offset `±sprite_half_width` en Y_cam para que el rectángulo siempre encare la cámara.
- **Frame rate.** El host (`supay-doom-llimphi`) agenda `Msg::Frame` a 60 Hz aparte del `Msg::Tick` a 35 Hz. `Msg::Frame` no toca el modelo pero dispara view rebuild + redraw — el closure de paint_with recomputa `alpha` desde `Instant::now()`. Sin esto, la interpolación entre snapshots no es visible porque Llimphi sólo redibuja en dispatch de Msg.
- **Stub adaptado.** `synth_snapshot` en supay-core ahora emite una sala 256×256 (≈ 4 celdas Doom de 64) con ceiling 192 y sprite trailing a 96 unidades — antes era 8×8 y la near-plane (4.0) cortaba paredes. Winding de las 4 paredes CW desde +Z para que `front_sector=0` quede correctamente como el interior bajo la convención Doom.
- **Toggle.** El host agrega `F3` para alternar entre `view=FB` (Fase 1, framebuffer 320×200 del motor) y `view=3D` (Fase 3.0, renderer moderno). El header muestra el modo activo. F12 sigue cerrando.
- **Tests.** `supay-render-llimphi` tiene 4 unit-tests cubriendo la identidad cámara en ángulo 0, signo de Y_cam para puntos a la izquierda, centrado del origen proyectado al centro de pantalla, y monotonicidad horizontal (+Y_cam → +x_screen).
- **No incluido en 3.0 (defer a 3.1+).** Texturing real desde lumps WAD; BSP/segs para front-to-back ordering correcto en geometrías raras; floor/ceiling polygons (subsector triangulation); stencil/RT shadows; TAA; sprite-real lookup por `sprite/frame` desde el WAD; volumetric fog por sector; transparencias.

**Cómo usar.**

```sh
cargo run -p supay-doom-llimphi --release
# arranca en view=FB (Fase 1); F3 alterna a view=3D (Fase 3.0).
```

**Fase 3.1 (2026-05-26, este bloque):** salas cerradas + variedad por pared.

- **Suelos y techos por pared** ("fake floor"). Cada pared, además de su slab, emite dos trapezoides: uno hacia el borde inferior de la pantalla con `floor_color(near_sec)` y otro hacia el borde superior con `ceiling_color(near_sec)`. Painter's algorithm + `depth + 0.5` para que los strips se ordenen detrás de las paredes pero adelante de slabs lejanas. No es geométricamente exacto sin BSP/subsectors (los strips no respetan polígonos reales), pero en escenas axis-aligned típicas de Doom — habitaciones rectangulares, pasillos, escaleras — el resultado se lee como "habitación cerrada con piso y techo de la sectorial correcta" en lugar del horizonte bicolor plano de 3.0.
- **Bandas horizontales por slab**. `wall_bands = 4` (configurable en [`RenderConfig`]) — cada slab se subdivide en 4 bandas verticales con shade modulado por `(linedef_idx, band_idx)`. Multiplier base 0.78→1.12 con `band_t = band/(bands-1)` (banda inferior más oscura, superior más clara — simulación cheap de iluminación cenital) + jitter pseudo-aleatorio ±8 % por `xorshift32(wall_idx ^ band*0x12345)`. Da feel de "paneles" sin samplear texture WAD.
- **Paleta Doom-ish**. Reemplazamos los 6 colores muddy de 3.0 por tres paletas:
  - **`WALL_PALETTE`** (16 entradas): riffs reverse-engineered de BROVINE / STARTAN / GRAYBIG / SLADWALL — marrones cálidos, grises piedra, tans UAC, rust. Indexed por `xorshift32(wall_idx) ⊕ (front_sector·7)`: cada habitación tiende a una familia tonal sin uniformidad.
  - **`FLOOR_PALETTE`** (8): dirt, stone, slime, marble, wood, tech, sand, ash. Index por `floor_pic % 8` — refleja la elección estética del nivel sin needing lump real.
  - **`CEIL_PALETTE`** (4): dark slate, light slate, black rock, tech panel. Index por `ceiling_pic % 4`.
- **Sprites coloreados por tipo**. `SPRITE_PALETTE` (12 entradas) indexed por `spritenum_t % 12` — imp red-brown, zombi verde, barril, llaves amarilla/azul/roja, hueso, antorcha cálida, armadura, etc. Cuando Fase 3.2 traiga lookup real al WAD esto desaparece, pero hoy hace que un imp se distinga de una llave a primera vista en lugar de todos rectángulos rojizos idénticos.
- **Backdrop con tinte de habitación**. `draw_backdrop` reemplaza el horizon plano por: arriba `SKY_BAND_TOP` (azul noche), abajo el color del piso del sector más iluminado del snapshot multiplicado por 0.45. Heurística — el sector más brillante suele ser donde está el jugador o adyacente. Cuando no hay paredes ocluyendo (mirás al vacío fuera del mapa, snapshot vacío), no quedan huecos negros.
- **Tests**. Se suman 2 al renderer: `wall_bands_vary_shade_monotonic_lighter_up` (banda superior debe ser más clara que la inferior con misma profundidad) y `floor_and_ceiling_palettes_indexed_by_pic` (dos `floor_pic` distintos producen colores distintos). 6 tests verde.
- **Header bump**: `PHASE 1` → `PHASE 3.1` en el subtítulo del logo.

**No incluido en 3.1 (defer a 3.2+):** sampling de texturas WAD reales (PNAMES/TEXTURE1/SIDEDEF/COLUMN); polígonos de subsector exactos (necesita exponer `subsectors`+`segs` desde `scene_export.c` — los structs ya están localizados en `r_defs.h`); detección de `skyflatnum` para distinguir techo-cielo; BSP front-to-back ordering; stencil/RT shadows; TAA; sprite real lookup por `sprite/frame` del WAD.

**Fase 3.2 (2026-05-26, este bloque):** pisos y techos como polígonos reales de subsector.

- **C-side `scene_export.c`** gana cuatro getters: `supay_scene_num_subsectors`, `supay_scene_subsector(i, *sector, *first_seg, *num_segs)`, `supay_scene_num_segs`, `supay_scene_seg(i, *x1, *y1, *x2, *y2)`, más `supay_scene_sky_pic()` que devuelve `skyflatnum` (0xFFFF como sentinel cuando el mapa aún no cargó). Headers nuevos: `doomstat.h` (skyflatnum). Sin caches: subsectors y segs son arrays planos del motor, indexado O(1).
- **`supay-scene`** gana `SubsectorSnap { sector, first_seg, num_segs }`, `SegSnap { x1, y1, x2, y2 }` y `sky_pic: u16`. `SceneSnapshot` deja de ser `Default` derivado (porque los nuevos `Arc<[...]>` no infieren) y trae un `Default` manual que delega a `empty(0)`. `interpolate` pasa los nuevos campos directos desde `next` sin lerp — la topología BSP es estable por mapa cargado.
- **`supay-core`** captura los nuevos campos en `capture_scene_real` con tres loops adicionales (subsectors, segs, sky_pic). En `synth_snapshot` (stub) los campos quedan vacíos / `NO_SKY_PIC`, lo que dispara el fallback fake-floor en el renderer.
- **Renderer** (`gather_subsector_planes`): por cada subsector construye el polígono mundial encadenando `seg.v1` de cada seg + `seg.v2` del último (cierra automáticamente si la cadena no es ya cerrada — tolerancia 0.01 unit). Transforma a cámara 2D, clipea contra el plano `X_cam ≥ near` con **Sutherland-Hodgman** 2D (`clip_near`), proyecta cada vértice a la altura del piso (`floor_height − view_z`) y del techo. Cull off-screen rápido (si todos los vértices proyectados caen del mismo lado del rect, salta). Painter's algorithm con depth = distancia euclidiana del centroide en cámara + `1.0` (planos van *detrás* de paredes y sprites al mismo depth).
- **Cielo**. Si `ceiling_pic == sky_pic` el subsector **no emite polígono de techo** — el backdrop azul-noche queda visible. Áreas abiertas tipo entrada E1M1 ahora tienen "cielo real" en lugar de techo plano absurdo. El test `ceiling_sky_detection_matches_pic` cubre los tres casos: match exacto, mismatch, sentinel `NO_SKY_PIC`.
- **Fallback 3.1**. Si el snapshot no trae subsectors (stub o mapa todavía no cargado), `use_subsectors = false` y `gather_wall` vuelve a emitir las strips fake-floor de 3.1. La transición es seamless — el modo stub se ve idéntico a antes, y el modo real se ve mucho mejor cuando el mapa carga.
- **Tests** (+4 = 10 total verde): `clip_near_keeps_polygon_fully_in_front`, `clip_near_drops_polygon_fully_behind`, `clip_near_clips_polygon_crossing_plane` (chequea que 2 intersecciones quedan exactamente en `x = near`), `ceiling_sky_detection_matches_pic`.
- **Header bump**: `PHASE 3.1` → `PHASE 3.2`.

**Limitaciones conocidas de 3.2.** La cadena de segs de un subsector a veces no cubre todos los lados del polígono convexo (los lados que son particiones BSP internas no tienen seg). En esos casos el polígono dibujado es más chico que el subsector real, pero el subsector vecino del mismo sector cubre el hueco visible — la unión termina siendo correcta para sectores conexos. Si vieras gaps de piso en niveles con muchos splits BSP raros (rare-ish), la fix definitiva es triangular con info de particiones (defer a 3.4 — necesita exponer también `nodes[]` y caminar el árbol).

**No incluido en 3.2 (defer a 3.3+):** sampling de texturas WAD (lumps PNAMES/TEXTURE1/SIDEDEF/COLUMN — el salto grande de feel); BSP front-to-back ordering exacto; stencil/RT shadows; TAA; sprite real lookup por `sprite/frame` del WAD; relighting realista por sector specials.

**Fase 3.3 (2026-05-26, este bloque):** colores reales de pisos y techos desde el WAD.

- **`supay-wad`** (crate nuevo, pura Rust, sin FFI). Parser mínimo del formato WAD: header IWAD/PWAD + directorio + lookup case-insensitive de lumps por nombre ≤ 8 chars. Decoders inline para PLAYPAL (256×RGB en bytes 0..768 de la primera de las 14 paletas), FLAT 64×64 (`flat`, `flat_average_color`, `flat_center_color`, `flat_rgba`). 8 unit-tests cubren parseo de header sintético + rechazo de magic inválido + truncado + grayscale palette + checker flat 50/50 average=150. Sólo lee del WAD lo necesario para texturing — niveles (THINGS/LINEDEFS/SIDEDEFS) y patches column-format quedan en doomgeneric. ~330 LOC.
- **`scene_export.c`** gana `supay_scene_flat_name(pic_idx, char out[9])` — resuelve un índice de flat (lo que `sector.floor_pic`/`ceiling_pic` traen) al nombre del lump leyendo `lumpinfo[firstflat + pic_idx].name`. Incluye `w_wad.h` y exterriea `firstflat`. Devuelve 1 si éxito, 0 si fuera de rango o motor sin mapa cargado.
- **`supay-core`** envuelve la FFI en `DoomEngine::flat_name(pic_idx) -> Option<String>` — convierte el buffer C de 9 bytes a `String` recortando en el nul.
- **`supay-render-llimphi::WadAtlas`**: estructura que el host construye una vez con un `Wad` + mapa pic_idx→nombre. **Interior mutability** vía `Mutex<AtlasInner>` para que pic_idx nuevos puedan registrarse desde `&WadAtlas` (esencial: el atlas vive bajo un `Arc` compartido con el renderer, no podemos `Arc::get_mut`). Cache lazy de colores promedio por pic_idx (`flat_color()` resuelve la primera vez y cachea). Pic_idx que no resuelven el flat (e.g. F_SKY1 sin bytes) cachean `None` y nunca se reintentan.
- **`RenderConfig`** gana `atlas: Option<Arc<WadAtlas>>`. `floor_color`/`ceiling_color`/`draw_backdrop` consultan `atlas.flat_color(sec.floor_pic)` y caen al `FLOOR_PALETTE`/`CEIL_PALETTE` de 3.1 sólo si el atlas no tiene el flat (placeholder de cielo, modo stub, WAD no encontrado). El backdrop también usa el atlas para el tinte del sector más iluminado.
- **`scene_view`** envuelve `config` en `Arc<RenderConfig>` para que el closure `move` no clone el WAD cada frame (sería cara la copia del Mutex+HashMap incluso si Arc lo amortiza).
- **`supay-doom-llimphi`** carga `doom1.wad` con `Wad::open("doom1.wad")` al inicio, construye el atlas con HashMap vacío. Si falla (no existe en cwd, mal formato), printf a stderr y sigue con `atlas: None` — el renderer cae a las paletas hardcoded. En cada `Msg::Tick` recorre los sectores del snapshot y para cada `floor_pic`/`ceiling_pic` no visto antes (`HashSet<u16>` propio) consulta `engine.flat_name(pic)` y lo registra en el atlas vía `set_flat_name(&pic, name)`. Costo: O(unique pics on map) acumulado a lo largo de la vida del proceso — ≈ 30–50 flats únicos en E1M1.
- **Tests** (+1 a render = 11 total verde): `floor_color_uses_atlas_when_available` — construye un WAD sintético inline con un flat F_T1 = todo índice 42, verifica que (i) sin nombre registrado, `floor_color` cae al fallback `FLOOR_PALETTE[7%8] = ash`; (ii) tras `set_flat_name(7, "F_T1")`, devuelve ≈ `42*shade ≈ 38` por canal grayscale. Hace público `Wad::parse(bytes)` para los tests.
- **Header bump**: `PHASE 3.2` → `PHASE 3.3`.

**No incluido en 3.3 (defer a 3.4+):** sampling de patches column-format (sprites + walls) — necesita parseo del lump format con posts y composición de TEXTURE1/PNAMES; UV mapping perspective-correct para paredes con texturas; sprites reales por `sprite/frame` del WAD; BSP front-to-back ordering exacto; floor texturing real (no sólo color promedio — actualmente perdés el patrón visual del flat). El path está limpio: `WadAtlas` ya tiene el `Wad` adentro, sólo hay que agregar decoders y refactorear las llamadas de `*_color` a `*_brush`.

**Fase 3.4 (2026-05-26, este bloque):** sprites reales del WAD — adiós blobs rojos.

- **`supay-wad::decode_patch`** + `Patch { width, height, leftoffset, topoffset, rgba }`. Decodificador del formato column-format clásico de Doom: header de 8 bytes (w/h/loff/toff i16 LE) + `width` u32 offsets de columna + por columna una serie de "posts" (`topdelta u8`, `length u8`, pad u8, length palette indexes, pad u8) terminados con `topdelta=0xFF`. Pixels no cubiertos quedan transparentes (alpha 0). Resuelve cada palette index contra `PLAYPAL` a RGBA8. Defensa: rechaza dimensiones bogus (≤0 o >4096), header truncado, columnofs apuntando fuera del lump, post truncado. Smoke contra `DOOM1.WAD`: TROOA1 (Imp facing camera) decodifica como 41×57 con 1145/2337 pixels opacos (≈49 % — la silueta humanoide cubre la mitad del bbox, resto transparente).
- **+3 tests** en supay-wad (11 total): `patch_decode_synthetic` construye un patch 4×4 con dos posts en columnas distintas + huecos transparentes + columnas vacías y verifica pixel-por-pixel; `patch_decode_rejects_bogus_dimensions` (width=0); `patch_decode_handles_truncated_header`.
- **`scene_export.c::supay_scene_sprite_name(spritenum, char out[5])`**: resuelve `spritenum_t` (e.g. SPR_TROO=29) al 4-char string `sprnames[spritenum]` (e.g. "TROO"). Verifica `< NUMSPRITES` + null check del puntero. `info.h` includido.
- **`DoomEngine::sprite_name(spritenum) -> Option<String>`** wrapper de la FFI.
- **`WadAtlas`** gana `sprite_names: HashMap<u16, String>` + `sprite_patches: HashMap<(u16, u8), Option<Arc<Patch>>>` (cache por `(spritenum, frame_letter)`). Métodos nuevos: `set_sprite_name(spritenum, name)` (invalida patches con ese spritenum), `has_sprite_name`, `sprite_patch(spritenum, frame) -> Option<Arc<Patch>>`. Convención de naming: probamos primero `<NAME><LETTER>0` (sprites omnidireccionales — llaves, munición, decoración) y si no existe `<NAME><LETTER>1` (sprites direccionales con 8 ángulos, ángulo 1 = facing camera). Patches resueltos cacheados como `Arc<Patch>`; misses cacheados como `None` (no reintenta). El bit 7 de `frame` (full bright) lo ignoramos por ahora.
- **`Renderable`** se vuelve un struct + enum `RenderKind { Fill, Sprite { image, xform } }`. La loop final del frame matchea: `Fill` → `scene.fill`, `Sprite` → `scene.draw_image(image, xform)`.
- **`gather_sprite`** intenta el camino texturizado primero: si `cfg.atlas.sprite_patch(spritenum, frame)` devuelve un patch, calculamos las esquinas del billboard en world (`y_left = y_cam + leftoffset`, `y_right = y_left - width`, `z_top = floor + topoffset`, `z_bot = z_top - height`) y como ambos lados del billboard están al mismo `X_cam`, la proyección es un **rectángulo axis-aligned** en pantalla, por lo que la `Affine::translate · scale_non_uniform` que mapea image-space → screen-space es exacta. Sin atlas o patch faltante → fallback al rectángulo coloreado de 3.1.
- **`supay-doom-llimphi`** gana `known_sprites: HashSet<u16>` paralelo a `known_pics`. Cada `Msg::Tick` itera sobre los sprites del snapshot y registra el `sprite_name` la primera vez que aparece cada spritenum — O(unique mobjs types vistos) acumulado, típicamente <20 en un nivel.
- **Header bump**: `PHASE 3.3` → `PHASE 3.4`. Tests totales 27 verde (11 wad + 5 scene + 11 render).

**Limitaciones conocidas de 3.4.**
- **Sin rotación**: usamos siempre el ángulo `1` (facing camera) — un imp visto de costado se ve igual que de frente. Fase 3.5 leerá `mobj.angle - player.angle`, lo mapeará a 1..8, y elegirá entre `<NAME><LETTER>N` o `<NAME><LETTER>NA<MIRROR>` (algunos lumps combinan dos ángulos por mirror; el flag está en el offset).
- **Sin shading por sector**: los sprites se ven a luz plena (no se atenúan con `light_level` ni con fog). El renderer 3.5 mezclará un overlay de color modulado por alpha sobre el image para emular el shading.
- **Sin animación de frame**: el `frame` del snapshot ya viene actualizado por el motor (el `tick` del simulación avanza P_MobjThinker que muta `frame`), pero como aún no exponemos `state->sprite`/`state->frame` separados, sólo vemos el frame "current" del mobj — sin secuencia walk/attack/die diferenciable visualmente al nivel de patch (la posición sí se mueve, las animaciones de mobj sí pasan, pero el frame letter sigue el ciclo del motor — debería funcionar igual, validar al correr).

**No incluido en 3.4 (defer a 3.5+):** rotación de sprites por ángulo de view; mirror flag en el offset; shading de sprites por sector light + fog; texturing de paredes (lumps PATCH composited via TEXTURE1/PNAMES); UV perspective-correct vertical para paredes; texturing real de pisos/techos (no sólo color promedio); BSP front-to-back ordering exacto.

**Fase 3.5 (2026-05-26, este bloque):** sprites rotan + se atenúan con la luz.

- **`supay-wad::sprite_lump(name, frame_letter, angle)`** — lookup de sprites con tres niveles de fallback: (i) `<NAME><F><angle>` directo; (ii) `<NAME><F>0` omnidireccional (keys/ammo/decoración); (iii) escaneo entre `S_START`..`S_END` buscando un lump de 8 chars que matchee `<NAME><F>·<F><angle>` — son los combos tipo `TROOA2A8` donde un único lump cubre dos ángulos vía espejado horizontal. Devuelve `(lump_name, mirror_flag)`.
- **`WadAtlas::sprite_patch(spritenum, frame, angle)`** firma cambia: ahora toma el ángulo (1..8) y devuelve `Option<(Arc<Patch>, bool)>` donde el bool es el mirror flag. Cache rekeyed a `(spritenum, frame_letter, angle)` — 30 sprites × ~5 frames × 8 ángulos = ~1200 entradas worst case, en práctica mucho menos.
- **`compute_display_angle(mobj_x, mobj_y, mobj_angle, viewer_x, viewer_y) -> u8`** — implementa la convención Doom: `R_PointToAngle2(mobj, viewer) − mobj.angle`, normalizado a `[0, 2π)`, redondeado al wedge de π/4 más cercano (con bias de π/8 para centrar cada wedge), +1 para mapear a 1..8. Resultado: 1=front, 3=right side, 5=back, 7=left side. Tres tests cubren los tres casos canónicos.
- **`gather_sprite`** calcula `display_angle` por frame, pide el patch al atlas, y si viene `mirror=true` arma el `Affine` con `scale_x` negativo + corrimiento al borde opuesto (`Affine::translate((br.x, tl.y)) * scale_non_uniform(-sx, sy)`). Los sprites mirror-naming ahora se renderean correctamente espejados.
- **Shading**. `gather_sprite` calcula `shade = shade_for(sector.light_level, depth, cfg)` y llama a `make_tinted_sprite_image(patch, shade)` que construye un `peniko::Image` aplicando un multiplicador R/G/B (alpha preservada — pixels transparentes siguen transparentes). Fast path full-bright (shade≈1.0): clone directo sin transformar. Costo: ~10 KB/sprite/frame de RAM por construir el Vec, ~30 sprites visibles típicos = 300 KB/frame, asumible a 60 fps. Cuando alcancemos memory pressure, podemos quantizar shade a 8 niveles y cachear por `(spritenum, frame, angle, shade_q)`.
- **+3 tests** render (14 total): `display_angle_facing_camera_is_1`, `display_angle_back_is_5`, `display_angle_right_side_is_3`. 25 verde supay total.
- **Header bump**: `PHASE 3.4` → `PHASE 3.5`.

**No incluido en 3.5 (defer a 3.6+):** texturing real de pisos/techos (subdividir el polígono del subsector en triángulos + affine por triángulo aproximando perspectiva → tile del flat 64×64); texturing de paredes (lumps PATCH composited via TEXTURE1/PNAMES + per-strip affine); BSP front-to-back ordering exacto; full-bright frame flag (bit 7 del `frame`); decals + transparencias; relighting por sector specials.

**Fase 3.6 (2026-05-26, este bloque):** paredes texturizadas con TEXTURE1+PNAMES.

- **`supay-wad::pnames()`** decodifica el lump PNAMES: `i32` count seguido de N×8 bytes con nombres null-padded uppercase. Devuelve `Vec<String>` indexable por `patch_idx` del maptexture.
- **`supay-wad::texture(name, palette) -> Option<Texture>`** parsea `TEXTURE1` (fallback a `TEXTURE2` si no hay) y compone la textura por nombre. Por cada `maptexture_t` matching: lee width/height/patchcount + lista de `(originx, originy, patch_idx)` → resuelve cada patch via PNAMES + `patch_rgba` + `blit_patch` al buffer RGBA destino. Patches superpuestos blittean back-to-front; pixels transparentes del patch no escriben (preserva máscaras). Smoke contra DOOM1.WAD: STARTAN3 128×128 100% opaque, SLADWALL 64×128, DOOR1 64×72 — todas las wall textures del shareware decodifican correctamente.
- **`WallSeg` gana `textures: [[u8; 8]; 6]`** — layout `[front_mid, front_up, front_lo, back_mid, back_up, back_lo]`, cada slot 8 chars null-padded (todo cero = sin textura asignada, convención Doom id 0). Cero alocación por wall, 48 bytes adicionales por linedef × ~1000 linedefs = 48 KB por snapshot.
- **`supay-scene::texture_name(slot) -> Option<&str>`** helper para extraer el string ascii recortando en el primer 0.
- **`scene_export.c::supay_scene_wall_texture(wall_idx, side, kind, char out[9])`** resuelve la textura de pared al nombre del lump leyendo `sides[lines[wall_idx].sidenum[side]].{mid,top,bottom}texture` → `textures[tex_id]->name`. Forward-declara `struct texture_s` (sólo los campos que necesitamos) en lugar de incluir `r_data.c` que no es header. side=0/1, kind=0=mid/1=up/2=lo.
- **`DoomEngine::wall_texture(wall_idx, side, kind) -> Option<String>`** wrapper.
- **`supay-core::capture_scene_real`** post-procesa cada wall: itera 2 sides × 3 kinds, llama `supay_scene_wall_texture`, copia el nombre al slot correspondiente. ~6 FFI calls por wall × ~1000 walls = 6000 calls por snapshot (35 Hz → 210K calls/s) — barato porque el motor sólo lee `sides[].midtexture` etc., sin string compare.
- **`WadAtlas::wall_texture(name) -> Option<Arc<Texture>>`** cache lazy por nombre uppercased. Misses cacheados como `None`.
- **`RenderKind` gana `TexturedWall { image, brush_xform }`** + branch en la loop final: `scene.fill(NonZero, IDENTITY, image, Some(brush_xform), &path)` rellena el quad samplando el image como brush con la transform que mapea image-px → world position.
- **`gather_wall` reescribe el slab path**: si hay textura asignada + atlas tiene el composite, emite **un único** `TexturedWall` por slab con `Extend::Repeat` activado y brush_xform calculado de los vértices proyectados (`image (u, v) → tl + u·step_u + v·step_v` con `step_u = (tr - tl)/wall_len_world`, `step_v = (bl - tl)/slab_h_world` — 1 image-pixel = 1 world-unit, Doom-standard). Para que las texturas no se vean siempre full-bright, emite un overlay negro semi-transparente con `alpha = (1 - shade)·255` ligeramente *delante* del wall (`depth - 0.001`) — vello respeta el sort y lo pinta encima. Fallback: si no hay textura asignada o no resuelve en TEXTURE1, vuelve a las bandas de 3.1.
- **`wall_slab_kind(slab_i, n_slabs, two_sided)`** resuelve qué sidedef-kind (mid/up/lo) corresponde a cada slab emitido por el path de slabs. One-sided → mid. Two-sided con dos slabs → lower primero, upper segundo (mismo orden que el path en `gather_wall`). Two-sided con un único slab → upper (heurística, más común en E1M1).
- **No `let`-borrow conflict**: el path del wall ahora pasa `wall.textures` por *array indexing* sin necesidad de borrows mut/shared cruzados. Compila clean en release.
- **Tests**: 30 verde supay (sin tests nuevos esta fase — el wad parser ya tenía 11 verde cubriendo PLAYPAL/flat/patch; las funciones nuevas pnames/texture/blit_patch están cubiertas por el smoke de integración contra DOOM1.WAD real).
- **Header bump**: `PHASE 3.5` → `PHASE 3.6`.

**Limitaciones conocidas de 3.6.**
- **Sin perspective-correct UV**: cada slab usa una sola affine. Las paredes largas vistas en ángulo agudo muestran el "affine sheen" — el texturing se ve linear en pantalla pero debería seguir la perspectiva del depth. Visible sobre todo en paredes >256 unidades vistas oblicuas. Fix: subdividir cada slab en N vertical strips o per-screen-column (Doom-style). Defer a 3.7.
- **Shading via overlay**: la oscuridad se aplica como rect negro semi-transparente encima del texture — preserva el detalle pero la mezcla no es la misma curva que el shading de la paleta original de Doom. Para fidelidad exacta habría que pre-tintar la texture por sector light (cache por `(texture_name, shade_q)`).
- **Slab-kind heurístico** cuando `n_slabs==1` en pared two-sided: asumimos upper. Si el motor expone más distinguibilidad (alguna paredes con `n_slabs==1` son lower steps en realidad), corregir en 3.7.
- **Sin `rowoffset` / `textureoffset`**: ignoramos los offsets que Doom usa para alinear texturas entre paredes. Visible en las costuras entre paredes adyacentes — el texture salta.

**No incluido en 3.6 (defer a 3.7+):** perspective-correct UV (per-column rendering al estilo Doom clásico, o subdivisión en strips con affine por strip); texturing real de pisos/techos (tile del flat 64×64 sobre polígono proyectado del subsector); `rowoffset`/`textureoffset` para alineación correcta entre paredes; switches y animaciones de textura; full-bright sprite flag (bit 7 del `frame`); BSP front-to-back ordering exacto.

**Fase 3.7 (2026-05-26, este bloque):** pisos y techos texturizados con flats reales.

- **`WadAtlas::flat_rgba(pic_idx) -> Option<Arc<Vec<u8>>>`** cache lazy: la primera vez por idx resuelve el nombre del flat → 64×64 indexed bytes → RGBA expansion via PLAYPAL. ~16 KB cacheado por flat × ≈40 flats únicos en E1M1 = ~640 KB total. `set_flat_name` invalida `flat_rgbas[idx]` además del color cache para que la siguiente resolución re-decodifique.
- **`Camera::from_cam_2d`** — inverso de `to_cam_2d`. Necesario para recuperar world XY de vértices intermedios que generó el `clip_near` 2D (que opera en cam space). Test: round-trip `to → from` recupera el mundo dentro de 1e-3.
- **`gather_subsector_planes` reescribe el path de pisos/techos**: por cada plano (floor, ceiling — sky se sigue salteando), intenta camino texturizado primero: (i) atlas tiene RGBA del flat para `floor_pic`/`ceiling_pic`; (ii) calcula `world_xy` por vértice del polígono clipeado vía `cam.from_cam_2d`; (iii) elige 3 vértices spread-out (`i0=0, i1=N/3, i2=2N/3`); (iv) llama `solve_floor_affine` para encontrar la affine `image(wx, wy) → screen` que satisfaga los 3 pares de correspondencia (rejecta determinante <1e-3 = casi colineales); (v) emite `Renderable::TexturedWall` con `Extend::Repeat` activado — vello tilea el flat 64×64 según `floor_pic mod 64`. Overlay negro semi-transparente (`alpha = (1 - shade)·255`) emitido a `depth + 0.999` (entre el plano `+1.0` y las paredes `+0`) para darken sin perder detalle.
- **Fallback al color promedio** (3.3 behavior): si no hay atlas, falta el flat en el WAD (placeholder F_SKY1, etc.) o los vértices son colineales (polígono degenerado), `floor_color`/`ceiling_color` siguen devolviendo el promedio.
- **Affine approximation**. La affine de un único polígono no es perspective-correct — para subsectores chicos (la mayoría de Doom) el error es invisible; para subsectores muy alargados con el viewer apuntando casi paralelo al piso, las tiles del flat se ven oblicuamente. Solución correcta: triangular el polígono y emitir una affine por triángulo. Defer si los artefactos molestan en práctica.
- **`solve_floor_affine`** resuelve por Cramer 2×2 (sistema desacoplado en {a, c, e} y {b, d, f}). 21 LOC, 2 tests cubren identidad cuando world == screen + rechazo de 3 vértices colineales.
- **Tests** (+3 render = 17 verde): `camera_to_from_round_trip` (inversa), `solve_floor_affine_recovers_identity_when_world_equals_screen`, `solve_floor_affine_rejects_collinear`. 33 verde supay total.
- **Header bump**: `PHASE 3.6` → `PHASE 3.7`.

**Limitaciones conocidas de 3.7.**
- **Affine sin perspectiva** en pisos/techos (igual que walls en 3.6): tile mostrado linear en pantalla, no foreshortened. Visible en pisos largos vistos en ángulo agudo.
- **Sin subdivision**: el polígono del subsector se rasteriza con una sola affine. Para fidelidad pixel-perfect haría falta triangular.
- **Sky ceiling**: sigue siendo el backdrop del 3.2 — sin "sky texture" del WAD (SKY1, SKY2, SKY3). Defer a 3.8 con scrolling según view angle.

**No incluido en 3.7 (defer a 3.8+):** per-triangle subdivision para perspective-correct floors; sky texture real (SKY1/SKY2/SKY3) con scrolling horizontal; per-column wall rendering para perspective-correct walls; `rowoffset`/`textureoffset`; switches y animaciones de textura; full-bright sprite flag; BSP front-to-back ordering exacto; relighting por sector specials.

**Fase 3.8 (2026-05-26, este bloque):** sky texture real con scrolling horizontal.

- **`draw_backdrop`** ahora pinta SKY1 como image fill en la banda superior cuando el atlas la tiene (cae al `SKY_BAND_TOP` plano si no). Sigue la convención Doom: 360° de rotación = 4 × `sky.width` = 1024 pixels en el panorama horizontal. `scroll_x = -player.angle · panorama_px / 2π` (signo negativo porque cuando giro a la izquierda, el sky se ve moverse a la derecha).
- **FOV aproximada** para el rango horizontal mostrado: `fov_x_rad ≈ fov_y_rad · aspect_ratio`. `pixels_to_show = fov_x_rad / 2π · panorama_px`. `scale_x = pixels_to_show / rect.w`. La textura tilea horizontalmente con `Extend::Repeat` en X y se "pegga" verticalmente con `Extend::Pad` en Y (el sky no tilea arriba/abajo en Doom).
- **Brush affine** `image(ix, iy) → screen` con `a = 1/scale_x, d = 1/scale_y, e = rect.x - scroll_x/scale_x, f = rect.y`. Vello rellena el `sky_rect` (mitad superior) samplando del image tileado.
- **Fallback**: cuando no hay atlas o `SKY1` no resuelve, sigue pintando el `SKY_BAND_TOP` plano de 3.1.
- **Limitación**: el sky no se "fija" al horizonte (sin pitch correcto). Por ahora ocupa la mitad superior fija; al moverse el jugador no se ve "más sky" arriba, sólo scroll horizontal. Para fix completo hace falta wire pitch (mouse-look) + ajustar la `f` del affine. No es prioridad mientras no haya mouse-look.
- **Tests** (sin nuevos esta fase — el sky rendering depende del atlas en runtime; el smoke contra DOOM1.WAD verificó que `SKY1` decodifica como 256×128 con 131072 bytes RGBA).
- **Header bump**: `PHASE 3.7` → `PHASE 3.8`. 33 verde supay total.

**No incluido en 3.8 (defer a 3.9+):** pitch / mouse-look para que el sky se mueva con la vertical; `rowoffset`/`textureoffset` en walls; switches y animaciones de textura; full-bright sprite flag; per-triangle subdivision para perspective-correct floors; per-column wall rendering perspective-correct; BSP front-to-back ordering exacto.

**Fase 3.9 (2026-05-26, este bloque):** per-strip wall rendering — perspective approximation por trozos.

- **`RenderConfig.wall_strips`** (default 8): cantidad de strips horizontales por slab texturizada. Cada strip resuelve su propio affine image→screen — el error de perspectiva queda factor 1/N respecto al single-affine de 3.6.
- **`gather_wall` slab texturizado refactor**: en lugar de una sola fill por slab, lerps en cam-space entre `(x1, y1)` y `(x2, y2)` con `t ∈ [0, 1]` dividido en `wall_strips` segmentos. Por strip: proyecta los 4 corners en cam-space (no near-clipped — ya está fuera), arma su propia `Affine` con `step_u = (s_tr - s_tl) / strip_w_world` y offset `strip_u_base = wall_len · t0` para preservar la continuidad del U coord entre strips adyacentes (`e = s_tl.x - strip_u_base · step_ux`). Image clonado por refcount (`Blob` es `Arc`-like) — emitir 8 fills por slab = 8 image refs sin duplicar datos.
- **Costo**: ~50 walls visibles × 8 strips = 400 image fills/frame. Vello batchea internamente, costo amortizado mínimo.
- **Overlay de shade único** por slab (no per-strip — el shade es constante para todo el slab al mismo depth; emitir un overlay por strip sería redundante y caro). Path del overlay = el slab completo, depth = depth - 0.001 como antes.
- **Continuidad de U**: el affine de cada strip tiene `strip_u_base` que es la coordenada U del image al inicio del strip. Con `Extend::Repeat` activado, el image se tilea consistente entre strips adjacent — sin cuts visibles en las juntas.
- **Header bump**: `PHASE 3.8` → `PHASE 3.9`. 33 verde supay total (sin tests nuevos esta fase — el cambio es interno al render loop, validable por inspección visual).

**No incluido en 3.9 (defer a 3.10+):** adaptive strip count (más strips para slabs anchas en pantalla); per-strip rendering equivalente para floors/ceilings (triangulación); pitch / mouse-look; `rowoffset`/`textureoffset`; switches/animaciones; full-bright sprite flag; BSP front-to-back ordering exacto.

### Fase 4 — Capa de modernización opt-in

Cada feature como toggle:

- Normal maps inferidos por shape-from-shading sobre las texturas WAD originales (sin reemplazo HD).
- Convolution reverb por sector (oclusión + late reverb por BSP). Mismo patrón: lógica intacta, percepción modernizada — audio es mitad del juego.
- Volumetric god rays desde luces puntuales.
- Sprite relighting más rico (specular, fresnel).
- Decals efímeros (chispas, scorch marks).

## Anti-features (rechazadas con motivo)

- **Geometry enrichment procedural** (tuberías/molduras añadidas a paredes): rompe la correspondencia visual-hitbox. El jugador apunta a la tubería, el lineseg está donde estaba. Toda decoración nueva queda **flush** con linedefs originales.
- **Voxelización del mundo**: los muros Doom son finos, voxelizarlos los hace ver plásticos. Pierde el carácter de fachada.
- **ML sprite hallucination → 3D impostors**: homogeniza estéticamente, pierde el feel pintado a mano. Mejor billboards 2D iluminados que volúmenes ML.
- **SDF renderer**: bonito conceptualmente, malo para texturas planas detalladas que es 100 % de Doom.
- **Cambiar timings/RNG/hitboxes para mejorar "feel" moderno**: rompe el contrato del proyecto. Si querés un FPS moderno hay 200 — Doom es Doom.

## Pila exacta (sin negociación)

| Capa | Crate raíz | Deps externas |
|---|---|---|
| Core (Fase 0) | `supay-app-llimphi` (monolito) | `llimphi-ui` |
| Core (Fase 1+) | `supay-core` | `cc` + vendored doomgeneric |
| Scene (Fase 2+) | `supay-scene` | — |
| WAD assets (Fase 3.3+) | `supay-wad` | — (puro Rust, lectura del DOOM1.WAD shareware) |
| Render moderno (Fase 3+) | `supay-render-llimphi` | `llimphi-ui`, `supay-wad` (+ `wgpu` directo cuando llimphi-ui gane `custom_pass`) |
| Audio modernizado (Fase 4+) | `supay-audio` | `cpal`, `fundsp` (TBD) |

## Referencias

- **Quake II RTX** (NVIDIA, 2019) — prueba industrial de "id Tech antiguo + path tracing en tiempo real". Confirma que la simplicidad de geometría hace a Doom ideal para RT.
- **RTX Remix** (NVIDIA) — intercepta draw stream DX8/9 y reemplaza assets en runtime. La misma idea de "modernizar sin tocar binario", a otra escala.
- **GZDoom** — referencia obligada para qué decisiones tomar. Mode hardware, fog, brights, voxels mod (con cuidado), interpolación r_interpolate.
- **doomgeneric** (ozkl) — fork de Chocolate Doom con motor aislado del renderer. ~10 KLOC C limpio. Es nuestra ruta Fase 1.

## Estado

- **2026-05-25:** SDD escrito.
- **2026-05-25 (tarde):** Fase 0 (raycaster hardcoded como Hello inframundo) en código — DDA + perp_dist + niebla + bias E/W + minimap.
- **2026-05-25 (noche):** Fase 0.5 — sumamos **sprites billboarded con z-test por columna** (cuatro tipos: barrel, pillar, imp, torch) + **sector lights** (puntos con falloff `1/(1+0.6·d²)` que afectan paredes y sprites) + AMBIENT global. El z-buffer se llena durante el raycast de paredes y los sprites lo consultan por columna para ocultarse correctamente. Sprites ordenados por distancia descendente para que los cercanos pinten encima cuando se superponen. Minimap muestra sprites como dots coloreados y luces como anillos del color de la luz con radio proporcional a `√strength`.
- **2026-05-25 (cierre):** Fase 0.6 — engrosado visual:
  - **Texturas procedurales por material** (sin bitmaps): cada slice se subdivide en `SLICE_SEGMENTS = 8` bandas verticales y cada una multiplica su shade por `texture_mul(material, wall_x, wall_y, tick)`. Cuatro patrones implementados — techbase (junta horizontal + gradiente), ladrillo (running bond + juntas H/V + variación por id de ladrillo), metal (paneles verticales con tornillos en esquinas), slime (oleaje sinusoidal + speckles animados con `tick`). `RayHit` gana `wall_x` que el DDA calcula como fract del hit world-coord en el eje correspondiente al lado de pared.
  - **Animación de sprites** vinculada a `tick`: Imp respira con bob vertical sinusoidal (~5 % altura), Torch oscila sutil para acompañar el flicker, barril/pillar estáticos.
  - **Flicker de luces cálidas**: las luces con `color.0 > color.2` (tinte naranja, identificadas como antorchas) parpadean orgánicamente con dos sinusoidales de frecuencias distintas + fase por índice — luces frías quedan estables.
  - **Crosshair central** (dos rects cruzados + dot oscuro).
  - Costo: ~2_500 rects por frame (320 cols × 8 segs); release build trivial. Tick determinístico sigue intocado — toda la animación es función pura de `tick`.
- **2026-05-25 (cierre+1):** Fase 0.7 — interacción:
  - **Disparo** (Space): `Msg::Fire` spawnea un `Bullet` con velocidad `0.45 u/tick` en la dirección del jugador + decrementa `ammo`. Cada bullet se avanza por tick; al chocar pared (`tile(nx, ny) != 0`) muere y deja un `Decal` con `TTL = 240 ticks`. TTL del bullet `60 ticks` por si nunca golpea.
  - **Decals**: lista circular `MAX_DECALS = 32`; cuando se llena, dropea el más viejo. Pintados como sprites pequeños (scale 0.20) apoyados al piso del slice, oscuros con tinte rojizo carbonizado.
  - **Bullets iluminan**: cada bullet aporta una luz puntual amarilla `BULLET_LIGHT_STRENGTH = 1.4` con falloff fuerte (`1/(1 + 1.2·d²)`) a `lighting_contribution`. El proyectil ilumina dinámicamente las paredes que pasa cerca, efecto "trazante caliente".
  - **HUD inferior** estilo Doom clásico: 52 px con borde rojo superior, tres celdas centradas (VIDA / MUNICION / OBJETIVO). Vida cambia de color por umbral (verde > 50, ámbar > 25, rojo); munición ámbar mientras quede, roja en 0. Sin lógica de daño todavía — vida queda en 100, no hay enemigos atacando.
  - **Bullet anchor**: a diferencia de los otros sprites (que apoyan al piso del slice), bullets se centran a la altura del jugador para volar horizontal.
  - **Refactor**: `SPRITES` → `STATIC_SPRITES`; `draw_sprites` ahora toma `&[Sprite]` (lista combinada estáticos + bullets + decals construida por frame en `draw_scene`).
- **2026-05-25 (cierre+2):** Fase 0.8 — enemigos vivos:
  - **Enemy + EnemyState**: `Enemy { x, y, hp: i32, state, attack_cd }` con `EnemyState::{Idle, Walking, Dying(ticks), Dead}`. HP inicial 100. Cargo en `Model.enemies: Vec<Enemy>`.
  - **Refactor**: `STATIC_SPRITES` const → `initial_static_sprites() -> Vec<Sprite>` (solo barrels/pillars/torches); los dos imps anteriores ahora son `Enemy`.
  - **AI de persecución**: por enemy alive, calcula dist al jugador y `has_los(ex, ey, px, py)` (DDA sample cada 0.1 unidades). Si dist < `ENEMY_AGGRO_RANGE = 8.0` y LOS clear → `Walking`, mover hacia jugador a `ENEMY_SPEED = 0.045 u/tick` con colisión cell-based sliding.
  - **Ataque cuerpo a cuerpo**: si dist < `ENEMY_MELEE_RANGE = 0.9` y `attack_cd == 0` → restar `ENEMY_MELEE_DAMAGE = 8` a `health` + resetear cooldown a `25 ticks`.
  - **Bullets vs enemies**: `advance_bullets` chequea cada bullet contra cada enemy alive (dist² < `BULLET_HIT_RADIUS² = 0.35²`). Hit → `enemy.hp -= 25`, bullet muere sin decal, spawn flash. Si `hp <= 0` → `state = Dying(14 ticks)` → `Dead` (cadáver inmóvil pintado como sprite `Corpse`).
  - **TempLight + flash de impacto**: nueva lista `Vec<TempLight>` con `(x, y, color, strength, ttl, ttl_max)`. Cada flash dura `FLASH_TTL = 4 ticks` y su `strength` decae linealmente con el TTL. `lighting_contribution` los suma; el resultado es un destello cálido cuando un bullet impacta. Spawn en colisión pared + colisión enemy.
  - **SpriteKinds nuevos**: `DyingImp` (rojo opaco scale 0.65) y `Corpse` (mancha rojiza scale 0.30) — el enemy en `draw_scene` se convierte al kind apropiado según state.
  - El jugador puede morir (vida llega a 0 y queda en 0); por ahora sin pantalla de game over — el input sigue activo. La pantalla del HUD muestra todo en rojo cuando vida < 25.
- **2026-05-26 (+8):** Fase 3.9 — paredes per-strip (8 por slab default) para perspective approximation. El affine sheen de 3.6 desaparece en paredes largas vistas oblicuas.
- **2026-05-26 (+7):** Fase 3.8 — sky SKY1 real con scroll horizontal según ángulo del jugador. Convención Doom 360° = 4×sky.width.
- **2026-05-26 (+6):** Fase 3.7 — pisos y techos texturizados con flats reales (FLOOR4_8, NUKAGE1, etc.) sampleados por affine de 3-puntos con Extend::Repeat. Las salas tienen textura piso a techo.
- **2026-05-26 (+5):** Fase 3.6 — paredes texturizadas con TEXTURE1+PNAMES + composites de patches + overlay de shading. Las paredes de E1M1 ya muestran STARTAN/BROWN/SLADWALL real.
- **2026-05-26 (+4):** Fase 3.5 — sprites rotan según ángulo viewer + se atenúan con sector light + mirror lumps (TROOA2A8 etc.) bien manejados.
- **2026-05-26 (+3):** Fase 3.4 — sprites reales del WAD via patch column-format decoder + `WadAtlas::sprite_patch` + render por `scene.draw_image`. Adiós blobs rojos.
- **2026-05-26 (+2):** Fase 3.3 — colores reales de pisos/techos desde el WAD vía nuevo crate `supay-wad` + `WadAtlas` en el renderer. Detalle en la sección "Fase 3 — Renderer 3D moderno" arriba.
- **2026-05-26 (+1):** Fase 3.2 — pisos/techos como polígonos reales de subsector + detección de cielo via `skyflatnum`. Detalle en la sección "Fase 3 — Renderer 3D moderno" arriba.
- **2026-05-26:** Fase 3.1 — salas con piso/techo (fake-floor) + paredes con paneles + paleta Doom-ish. Detalle en la sección "Fase 3 — Renderer 3D moderno" arriba.
- **2026-05-25 (cierre+3):** Fase 0.9 — pickups + game over + victoria + reset:
  - **Pickups** estáticos en mapa: 3× AmmoBox (+12 munición) cyan + 2× HealthKit (+25 vida, max 100) verde. Sprite scale 0.35, apoyados al piso. `consume_pickups` chequea dist² al jugador cada tick (radio 0.55), aplica bonus + spawn flash del color del pickup, remueve. Drop-on-pickup, no respawnean.
  - **Game over**: cuando `health == 0` al final del tick, `m.game_over = true`. Bloquea movimiento + disparo; advance solo envejece flashes. Space pasa a dispatchar `Msg::Reset` en vez de `Msg::Fire`.
  - **Victoria**: cuando `enemies.iter().all(|e| Dead)` y no hubo muerte previa, `m.victory = true`. Mismo handling que game_over (Space reinicia).
  - **`reset_game(&mut Model)`** restaura posición/ángulo/HP/ammo + `initial_enemies()` + `initial_pickups()` + limpia listas dinámicas.
  - **Overlay full-screen** vía `View::paint_with` que recibe `Typesetter` cacheado del runtime: rect negro semi-transparente (alpha 175) + título 64 px (MUERTO rojo / VICTORIA verde) + subtítulo 18 px ("SPACE para reiniciar") centrados con parley.
  - **Refactor `on_key`**: ahora recibe `&Model` (siempre lo hizo, lo aprovechamos) para decidir qué Msg disparar Space según `game_over || victory`.
