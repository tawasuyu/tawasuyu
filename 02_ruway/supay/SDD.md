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

**Fase 3.10 (2026-05-26, este bloque):** texturas alineadas — `textureoffset` / `rowoffset` + pegging Doom.

- **C-side `scene_export.c::supay_scene_wall_offsets(wall_idx, side, *xoff, *yoff)`**: nuevo getter que lee `sides[].textureoffset` y `sides[].rowoffset` (fixed-point 16.16 → float). side=0=front, 1=back. Devuelve 0 si fuera de rango.
- **`supay-scene::WallSeg`** gana `tex_x_offsets: [f32; 2]` y `tex_y_offsets: [f32; 2]` (front/back). 16 bytes adicionales por wall (×1000 walls = 16 KB por snapshot, despreciable). El crate también re-exporta `ML_DONTPEGTOP = 0x0008` y `ML_DONTPEGBOTTOM = 0x0010`, las dos flags de Doom que controlan dónde se "pegga" la textura verticalmente cada kind (mid/upper/lower).
- **`supay-core::capture_scene_real`** llama el getter para cada wall × 2 sides — 2 FFI extras por wall (×35 Hz × ~1000 walls ≈ 70K calls/s, barato porque el motor sólo lee fields, sin string compare). En `synth_snapshot` los offsets quedan en 0.
- **`supay-render-llimphi::wall_v_top`**: helper que resuelve la coord V (image-space) en el borde superior del slab siguiendo la convención de Chocolate Doom (`r_segs.c::R_StoreWallRange`). Casos:
  - **Middle one-sided default**: top de la textura al near_ceiling → `v_top = 0` en z_top.
  - **Middle + `DONTPEGBOTTOM`**: bottom al near_floor → `v_top = tex_h - slab_h`. Usado en lifts.
  - **Upper default**: top al far_ceiling (anclado al bottom del opening) → `v_top = tex_h - slab_h`.
  - **Upper + `DONTPEGTOP`**: top al near_ceiling → `v_top = 0`. Usado en puertas para que la textura no suba con la puerta.
  - **Lower default**: top al far_floor (el escalón) → `v_top = 0`.
  - **Lower + `DONTPEGBOTTOM`**: top al near_ceiling → `v_top = near_ceiling - z_top`. Alinea con upper.
- **`gather_wall` reescribe el affine de cada strip texturizado**: en lugar de partir el U en `wall_len·t0` y el V en 0, suma `tex_x_offset` al base U y desplaza el V por `v_top`. La translación del affine queda `s_tl - (strip_u_base + tex_x_offset)·step_u - v_top·step_v`, donde `Extend::Repeat` se encarga del wrap modulo tex_width/tex_height. Continuidad de strips preservada (cada strip arrastra el mismo `strip_u_base` corrido).
- **Resultado visible.** Las costuras entre paredes adyacentes con la misma textura dejan de saltar (donde antes había un offset arbitrario entre ladrillos, ahora se ven alineados como Doom original). Las puertas mantienen su textura quieta cuando suben (efecto "deslizándose" correcto). Los escalones de lift heredan la textura de la pared principal en vez de empezar desde cero arriba.
- **Tests** (+7 render = 24 total verde, 40 supay total): `wall_v_top_middle_default_pegs_top_to_ceiling`, `wall_v_top_middle_dontpegbottom_pegs_bottom_to_floor`, `wall_v_top_upper_default_pegs_to_back_ceiling`, `wall_v_top_upper_dontpegtop_pegs_to_front_ceiling`, `wall_v_top_lower_default_pegs_to_back_floor`, `wall_v_top_lower_dontpegbottom_pegs_to_near_ceiling`, `wall_v_top_rowoffset_is_added`.
- **Header bump**: `PHASE 3.9` → `PHASE 3.10`.

**No incluido en 3.10 (defer a 3.11+):** sprites con full-bright frame flag (bit 7); animated textures (switches, NUKAGE, FIREBLU); per-triangle subdivision para perspective-correct floors; pitch / mouse-look; BSP front-to-back ordering exacto; volumetric fog real por sector.

**Fase 3.11 (2026-05-26, este bloque):** flats/paredes animados + full-bright sprite flag.

- **C-side `supay_scene_sector` aplica `flattranslation[]`**: cuando la tabla existe (post-`R_InitFlats`), reportamos `flattranslation[s->floorpic]`/`flattranslation[s->ceilingpic]` en vez del pic estático. Doom rota la tabla cada ~8 ticks vía `P_UpdateSpecials` por familias hardcoded en `p_spec.c::animdefs[]`: NUKAGE1→NUKAGE2→NUKAGE3, FIREBLU1↔FIREBLU2, BLOOD1→BLOOD2→BLOOD3, FWATER1→FWATER4, SLIME, LAVA, etc. El renderer ve un `floor_pic` distinto cada ciclo y resuelve el lump aparte vía `DoomEngine::flat_name` (cache lazy en `WadAtlas` se llena on-demand).
- **C-side `supay_scene_wall_texture` aplica `texturetranslation[]`**: switches activos cambian su lump (SW1xxx ↔ SW2xxx) cuando el jugador los activa. El renderer ve la textura "presionada" en el siguiente snapshot.
- **`gather_sprite` respeta `frame & 0x80`** (`FF_FULLBRIGHT_BYTE` en `info.h`): cuando el flag está set, saltamos `shade_for` y usamos `shade = 1.0`. Esto cubre proyectiles (BAL1, BAL7, MISL), muzzle flashes del player (TROO + state attack tiene un frame full-bright para el destello del fireball lanzado), barriles en explosión (BEXP frames), y otros mobjs que el motor marca como "emisores de luz propia". Sin esto, los proyectiles se ven igual de oscuros que el ambiente — con el flag, brillan en cuartos oscuros como en Doom original.
- **`sprite_color`** (fallback sin patch del WAD) también respeta el flag — modo stub o sprites no resueltos siguen comportándose consistente.
- **Costo de animación**: cero, la translation se aplica en C antes de devolver al snapshot. Cada nuevo pic_idx hace que el host registre el `flat_name` la primera vez (~40 flats únicos en E1M1 después del primer ciclo de animación, vs ~10 sin animación), pero el cache resuelve los siguientes ticks en O(1).
- **Tests** (+1 render = 25 total verde, 41 supay total): `sprite_color_full_bright_bypasses_shading` verifica que con `frame=0x80` + sector oscuro + fog activo, el sprite sale visiblemente más brillante que sin el flag. La animación de flats/textures la valida el smoke contra DOOM1.WAD — se observa por inspección visual.
- **Header bump**: `PHASE 3.10` → `PHASE 3.11`.

**No incluido en 3.11 (defer a 3.12+):** per-triangle subdivision para perspective-correct floors; pitch / mouse-look; BSP front-to-back ordering exacto; volumetric fog real por sector; texture scrolling (líneas con `Scroll_*` specials).

**Fase 3.12 (2026-05-26, este bloque):** pisos y techos per-triangle — perspective-correct exacto.

- **`gather_subsector_planes` reescribe el camino texturizado**: en lugar de resolver UNA affine para todo el polígono usando 3 vértices spread-out (3.7 behavior), triangulamos fan-from-vertex-0 (`(0, j, j+1)` para `j ∈ [1, N-2]`) y emitimos UN `Renderable::TexturedWall` por triángulo, cada uno con su propia affine resuelta vía `solve_floor_affine`. Tres vértices determinan exactamente una affine — sin aproximación.
- **Convexidad garantizada**. Subsectores en Doom son convex hulls del BSP por construcción. El clip Sutherland-Hodgman contra el near plane preserva la convexidad (intersección de un convex hull con un half-plane sigue siendo convex). Por eso el fan-from-vertex-0 es topológicamente válido sin necesidad de ear-clipping ni constrained Delaunay.
- **Costo**. Subsectores típicos tienen 3-6 vértices → 1-4 triángulos por plano = 2-8 fills extra por subsector cuando hay piso + techo. Mapas E1M1-tipo con ~250 subsectors visibles ≈ 1000-2000 fills/frame, asumible — vello batchea internamente y los triángulos comparten el mismo `Image` por refcount (Blob).
- **Overlay de shade**. Sigue siendo uno sólo sobre todo el polígono — el shade es constante por plano al mismo depth, no hace falta per-triangle. Path del overlay = el polígono completo cerrado.
- **Triángulos degenerados** (colineales post-clip, raro pero posible) se saltan silenciosamente vía el `None` de `solve_floor_affine` — el vecino del fan los cubre.
- **Resultado visible.** El "affine sheen" que quedaba en pisos largos vistos en ángulo agudo desaparece. Niveles con grandes habitaciones (entrada exterior de E1M1, hangar de E2M1) muestran el tile del flat siguiendo perspectiva por trozo en vez de una sola affine lineal sobre todo el polígono.
- **Tests**. Sin tests nuevos esta fase — el cambio es interno al render loop y validable por inspección visual; `solve_floor_affine` ya tenía cobertura completa de 3.7.
- **Header bump**: `PHASE 3.11` → `PHASE 3.12`. 25 tests verde supay (sin regresiones).

**No incluido en 3.12 (defer a 3.13+):** pitch / mouse-look; BSP front-to-back ordering exacto (painter's algorithm con depth euclidiano sigue, pero falla en geometrías raras con sectors interpenetrating); volumetric fog real por sector; texture scrolling (`Scroll_*` specials); decals dinámicos.

**Fase 3.13 (2026-05-26, este bloque):** BSP back-to-front ordering exacto para pisos/techos.

- **C-side `scene_export.c::supay_scene_num_nodes` + `supay_scene_node`**: exponemos el árbol BSP `nodes[]` de doomgeneric. Cada nodo trae línea de partición (`x, y, dx, dy` en unidades Doom — convertidos de fixed-point) + 2 hijos `u16` con la convención `NF_SUBSECTOR=0x8000`: si bit 15 set → subsector (idx = child & ~NF_SUBSECTOR), si no → otro nodo (idx = child). La raíz es `nodes[numnodes - 1]`. Includes `r_state.h` ya estaba; sólo agregamos `extern int numnodes; extern node_t *nodes;`.
- **`supay-scene::NodeSnap`**: struct nuevo con los 4 floats + `children: [u16; 2]` (preservando el bit NF_SUBSECTOR). Constante pública `NF_SUBSECTOR = 0x8000`. `SceneSnapshot.nodes: Arc<[NodeSnap]>` (vacío en stub o pre-mapa). `interpolate` propaga `next.nodes` directo — el BSP es estable por mapa, no se interpola.
- **`supay-core::capture_scene_real`** suma un bucle de captura que llama `supay_scene_node(i, ...)` por nodo. `synth_snapshot` deja `nodes` vacío.
- **`supay-render-llimphi::walk_bsp`** recursivo: para cada nodo interno calcula `side = dx·(view_y - y) - dy·(view_x - x)` (convención `R_PointOnSide`). Cuando `side > 0` el viewer está en el lado front (children[0]); cuando `side < 0` está en el back (children[1]). Para back-to-front, visita primero el subtree lejano y luego el cercano. Hojas (subsectores) se appendean al `Vec<u32>` resultado.
- **`compute_bsp_order_depths(snap) -> Vec<Option<f32>>`**: por cada subsector, depth de painter's = `BSP_DEPTH_BASE + (total - step)` donde `step` es la posición del subsector en la travesía back-to-front (0 = primer visitado = más lejano). `BSP_DEPTH_BASE = 1e6` — mucho más grande que cualquier depth euclidiano de pared/sprite (mapas Doom ≤ ~3000 unidades), garantizando que los planos siempre se pinten **antes** que walls + sprites, conservando el orden BSP entre ellos.
- **`gather_subsector_planes` ahora acepta `bsp_depth_override: Option<f32>`**. Cuando viene Some, lo usa para el `Renderable.depth` (sort de painter's); cuando es None, cae al cálculo viejo de centroide euclidiano (stub o mapa sin BSP). El **shading** (light dropoff + fog) sigue usando el centroide euclidiano por separado — la distancia real al jugador determina cómo se atenúa la luz, independiente del orden de pintado. Los overlays de oscuridad mantienen el offset `+0.999` sobre el depth base para quedar entre el plano y las paredes.
- **Resultado visible**. Escaleras con varios escalones consecutivos (E1M2 entrada al hangar, E1M3 corridor a la armadura) dejan de mostrar "tearing" de pisos: el escalón cercano deja de pintarse en orden ambiguo respecto al lejano cuando ambos tienen centroides equidistantes. Sectores interpenetrados (las plataformas de la sala de la barata en E1M4) ya no tienen flicker de techo. En vista general los maps se ven igual — el fix es de **correctness** en geometrías ambiguas.
- **Tests** (+3 render = 28 total verde, 36 supay total): `bsp_walk_viewer_on_front_visits_back_first` (viewer al +X de partición vertical visita ss0 — el lado +X — antes que ss1 cuando ss0 es el far en la convención implementada), `bsp_walk_viewer_on_back_visits_front_first` (caso espejo), `bsp_compute_depths_assigns_decreasing_values` (verifica que el subsector visitado primero recibe depth mayor que el segundo + ambos sobre BSP_DEPTH_BASE).
- **Header bump**: `PHASE 3.12` → `PHASE 3.13`.

**Limitaciones conocidas de 3.13.**
- **Walls + sprites siguen euclidiano.** Cada wall renderable sigue siendo un linedef (no un seg por subsector), y mapearlo al BSP order requeriría un pase extra (seg→subsector→bsp_step) además de splittear walls cuando un linedef cruza subsectores. Práctico en mapas Doom típicos no se nota porque el sort euclidiano de walls en una habitación cerrada es correcto la mayor parte del tiempo.
- **Sprites siguen euclidiano** por la misma razón — `SpriteSnap` trae `sector` pero no `subsector`. Para vista correcta de sprites a través de portales (mob detrás de una puerta entreabierta) habría que extender el snapshot.
- **No es front-to-back con occlusion buffer.** Painter's puro sigue siendo wasteful — pintamos cada plano completo aunque esté ocluido. La optimización de Doom clásico (visplanes + clipsegs front-to-back) deferida indefinidamente: vello+GPU paga el overdraw barato vs el costo de mantener clipsegs CPU-side.

**No incluido en 3.13 (defer a 3.14+):** wall + sprite BSP ordering (requiere refactorizar el iter de walls a iter de segs por subsector); pitch / mouse-look; volumetric fog real por sector; texture scrolling validation (3.10 capture de `textureoffset` ya debería funcionar para SCROLL Left lines tipo 48 — verificar visual); decals dinámicos; relighting por sector specials.

**Fase 3.14 (2026-05-26, este bloque):** player palette overlays — damage red, pickup yellow, radsuit green, invuln white.

- **Contexto.** Doom intercambia PLAYPAL[1..13] cuando algo le pasa al jugador (rojo de daño, amarillo de pickup, verde con radsuit, inversión con invulnerabilidad). Como sampleamos siempre con PLAYPAL[0] desde el renderer 3D, esos overlays no aparecen "gratis" — la modernización es overlay alpha full-screen al final del frame.
- **C-side `supay_scene_player_overlays(damagecount, bonuscount, power_invuln, power_radsuit)`**: getter nuevo que devuelve los 4 counters del `player_t` (los dos contadores de flash + dos powers relevantes). Devuelve 0 si no hay player mobj (pre-mapa); outs en cero. Costo: 4 reads + 4 writes — despreciable a 35 Hz.
- **`supay-scene::PlayerOverlays`** struct nuevo con los counters crudos (`u16` para los flashes + `u32` para los powers, alineado a los tipos doomgeneric). `SceneSnapshot.player_overlays` field. `interpolate` toma `next` puro — el flash sube/baja en pasos discretos de 1/tick, interpolar tendría sentido pero a 60 Hz la transición visual es suave por la decadencia natural del counter.
- **`supay-core::capture_scene_real`** llama el getter post-tick y rellena el field. `synth_snapshot` deja todo en cero — el modo stub no tiene flashes (lo cual está bien, no hay enemigos que peguen ni pickups que tomar).
- **`supay-render-llimphi::draw_player_overlays`**: pinta un único `Rect::fill` con `Color::from_rgba8(r, g, b, alpha)` sobre todo el viewport. Costo: 1 fill extra por frame.
- **Prioridad y curva.** `overlay_rgba(overlays, tick)` resuelve cuál pintar:
  - **Invuln** (gana sobre todo) → blanco semi-translúcido `(220, 220, 232, 110)`. Blinkea en los últimos 4 tics (`& 0x8` del tick). Aproximación cheap del invert-colors de Doom — para fidelidad real haría falta una segunda pasada con un colormap invertido.
  - **Damage** → rojo `(220, 30, 30)`, alpha `24 + level·24` con level = `(damagecount + 7) >> 3` clampado a 8 (NUMREDPALS de Doom). Rango alpha 48..216 sobre 8 niveles.
  - **Bonus** → amarillo cálido `(215, 180, 70)`, alpha `24 + level·18` con level clampado a 4 (NUMBONUSPALS). Rango 42..96.
  - **Radsuit** → verde `(45, 140, 60, 64)`. Constante mientras `power > 4*32` (~3.6 s); luego blinkea con `tick & 0x8`.
- **Resultado visible.** En modo real con DOOM1.WAD: pegar a un zombie produce flashes rojos cuyo alpha es proporcional al daño recibido. Recoger una llave o ammo produce un flash amarillo de 1-2 segundos. Caminar sobre slime con el traje de protección tinta verdoso constante. Cuando el traje se agota, blinka antes de quitarse.
- **Tests** (+5 render = 33 total verde, 41 supay total): `overlay_none_when_all_counters_zero`, `overlay_damage_red_priority_over_bonus`, `overlay_damage_alpha_scales_with_count`, `overlay_radsuit_blinks_in_last_seconds`, `overlay_invuln_dominates_damage`.
- **Header bump**: `PHASE 3.13` → `PHASE 3.14`.

**Limitaciones conocidas de 3.14.**
- **Invuln no invierte colores** — usa un overlay blanco aproximado. Para invertir habría que pasar la escena completa por un compositor que aplique `1 - c` por canal. Vello no expone blend modes que hagan exactamente eso sin shaders custom (`Mix::Difference` con (255,255,255) se acerca pero no es exacto). Defer cuando llimphi-ui exponga custom_pass.
- **Sin berserk red tint.** El `pw_strength` también tinta rojo en Doom, con fade-out por counter. No lo expongo todavía (es menos visible que damage; en E1M1 ni siquiera hay berserk).
- **Sin transición palette → palette del Doom original.** El motor usa 14 paletas discretas; nosotros tenemos un alpha gradiente continuo. Diferente "feel" pero más limpio visualmente.

**No incluido en 3.14 (defer a 3.15+):** wall + sprite BSP ordering; pitch / mouse-look; volumetric fog real por sector; texture scrolling visual validation; decals dinámicos; berserk red tint; invuln invert-colors real (necesita custom shader).

**Fase 3.15 (2026-05-26, este bloque):** weapon psprite — el arma en mano.

- **Contexto.** Doom pinta `players[].psprites[ps_weapon]` como overlay 2D sobre la vista 3D (pistol, shotgun, chaingun, etc.). Sin esto, nuestra vista 3D se ve sin manos — un FPS sin arma es raro. La modernización es leer el psprite del motor y pintarlo encima de la escena, antes del overlay PLAYPAL.
- **C-side `supay_scene_player_weapon(spritenum, frame, sx, sy)`**: lee `players[consoleplayer].psprites[ps_weapon]`. Devuelve 0 si `psp->state == NULL` (player dead, pre-mapa); outs en cero. Extrae `state->sprite` (e.g. SPR_PISG), `state->frame` con bit FF_FULLBRIGHT preservado (movido al bit 7 para nuestra convención `u8`), `psp->sx/sy` convertidos de fixed-point a float (coords nominales 320×200 del viewport Doom).
- **`supay-scene::WeaponSpriteSnap`** struct con `active: bool, sprite: u16, frame: u8, sx: f32, sy: f32`. `SceneSnapshot.weapon: WeaponSpriteSnap`. `interpolate` interpola `sx/sy` cuando `prev` y `next` comparten sprite + ambos activos (smoothing del weapon bob al caminar); cambio de sprite o transitions a inactive → toma `next` puro.
- **`supay-core::capture_scene_real`** llama el getter post-tick. Stub deja `weapon: Default` (inactivo).
- **`supay-render-llimphi::draw_weapon_sprite`**: pintado entre el sort de renderables y `draw_player_overlays`. Reutiliza `atlas.sprite_patch(spritenum, frame, 1)` — Doom usa angle=0 para weapon lumps; nuestro `sprite_lump` cae al fallback `<NAME><F>0` automáticamente.
  - **Escalado**: `scale = min(rect.w / 320, rect.h / 200)` (4:3 nominal de Doom). Aspectos más altos letterboxean horizontalmente.
  - **Horizontal**: centro del rect + `sx * scale` como offset (sx=0 idle = centrado).
  - **Vertical**: anclado al bottom del rect (bottom de patch = bottom de rect cuando sy=32 idle). Cuando sy crece (WEAPONBOTTOM=128 al guardar arma), el patch desciende `(sy - 32) * scale` pixels — la animación de switch-down funciona automáticamente.
  - **Mirror flag** del lump combinado-ángulo se respeta (improbable en weapon sprites pero el código está ahí por consistencia con sprite_patch).
  - **Full-bright bit** (bit 7 del frame) no se aplica especialmente — el sprite ya viene sin shading porque está "en la mano del jugador" (modernización: el arma siempre se ve clara, vanilla Doom dimmea con el sector pero queremos que el feedback visual del arma sea siempre legible).
- **Z-order respecto a overlays.** `draw_weapon_sprite` se llama *antes* de `draw_player_overlays` — el flash rojo de daño tinta también el arma (esperado en Doom original, donde la PLAYPAL afecta todo el frame incluido el psprite).
- **Tests**. Sin tests unitarios nuevos esta fase (la lógica vive en una función con efectos visuales puros; las posiciones se validan empíricamente con el binario). 33 tests verde se mantienen.
- **Header bump**: `PHASE 3.14` → `PHASE 3.15`.

**Limitaciones conocidas de 3.15.**
- **`ps_flash` no se renderiza** — Doom usa un psprite secundario para el muzzle flash de algunas armas (BFG, plasma). Sólo pintamos `ps_weapon` por ahora. El flash bright cuando la pistola dispara igual lo tenemos vía el bit FF_FULLBRIGHT del frame en `ps_weapon` (PISGB frame del fire).
- **Weapon bob no es perfecto** — interpolar sx/sy entre snapshots da smoothing, pero el feel del bob viene del motor C; cualquier diff entre 35 Hz y 60 Hz se mantiene como artefacto leve.
- **Sin scale por viewport activo** — Doom escala el psprite por `viewwidth/SCREENWIDTH` cuando se cambia el detail level. Asumimos siempre fullscreen 320×200; si el rect no es 4:3 puro, hay letterbox horizontal en lugar del 100% nuestro fov.

**No incluido en 3.15 (defer a 3.16+):** wall + sprite BSP ordering; pitch / mouse-look; volumetric fog real; texture scrolling validation; decals dinámicos; berserk red tint; invuln invert-colors real; `ps_flash` (muzzle flash separado); weapon bob smoothing extra.

**Fase 3.16 (2026-05-26, este bloque):** `ps_flash` (muzzle flash) + berserk red tint.

- **`ps_flash`** — Doom mantiene un segundo psprite `psprites[ps_flash]` que se superpone al arma durante el disparo. Algunas armas (BFG, plasma, chaingun, shotgun de combate) lo usan para el destello brillante; pistola y motosierra no. Sin este overlay, los disparos de plasma/BFG se ven planos.
- **C-side `supay_scene_player_flash`** — espejo exacto de `supay_scene_player_weapon`, pero leyendo `psprites[ps_flash]`. Devuelve 0 cuando el slot no tiene state (la mayor parte del tiempo).
- **Berserk red tint** — Doom usa `pw_strength` (counter que sube monotónicamente desde 1) para tintar la paleta hacia el rojo al agarrar el berserk pack, con fade-out lento. La paleta se elige con `12 - (strength >> 6)` clampada a 0..7.
- **C-side `supay_scene_player_overlays_ext`** — variante extendida que también devuelve `power_strength`. Reemplaza la versión vieja en `capture_scene_real`; la vieja FFI declaration se removió.
- **`supay-scene::PlayerOverlays.power_strength`** field nuevo + `SceneSnapshot.weapon_flash: WeaponSpriteSnap` field nuevo.
- **`interpolate`** factorizada en `lerp_weapon(prev, next, alpha)` para reusarse entre `weapon` y `weapon_flash`.
- **`supay-render-llimphi::draw_weapon_sprite`** ahora se llama dos veces — una para `weapon`, otra para `weapon_flash`. El flash queda inmediatamente encima del arma (mismo escalado + anchor).
- **`overlay_rgba` con berserk** — branch nueva al final, prioridad después de radsuit (más débil que invuln/damage/bonus/radsuit, fade-out de fondo del nivel completo). Color `(180, 40, 30)`, alpha ramp 10..80 a medida que `strength >> 6` crece.
- **Prioridad final de overlays**: invuln > damage > bonus > radsuit > berserk. Mirrors la prioridad implícita de PLAYPAL en Doom (radsuit y strength comparten paletas red+green, pero radsuit gana en el motor cuando ambos activos).
- **Tests** (+2 render = 35 total verde): `overlay_berserk_fades_with_strength` (alpha cae al subir el counter), `overlay_radsuit_priority_over_berserk` (radsuit verde domina cuando ambos activos).
- **Header bump**: `PHASE 3.15` → `PHASE 3.16`.

**Limitaciones conocidas de 3.16.**
- **El flash siempre se pinta full-bright** — no aplicamos atenuación por luz de sector (consistente con 3.15 para `ps_weapon`). El bit FF_FULLBRIGHT del frame del flash queda capturado en el snapshot pero sin uso renderer-side (los flashes ya vienen brillantes en sus patches del WAD).
- **Sin smoothing extra del bob** — `lerp_weapon` interpola sx/sy entre snapshots pero el bob mismo viene del motor a 35 Hz. Para feel ultra-suave habría que reconstruir el bob renderer-side a partir de la velocidad del jugador. Defer.
- **Strength en E1 shareware** — los niveles del shareware DOOM1.WAD no incluyen berserk pack (es de DOOM2 + nightmare difficulty), así que el tint berserk no se va a ver corriendo el WAD shareware. Funciona con freedoom2 o doom1.wad full.

**No incluido en 3.16 (defer a 3.17+):** wall + sprite BSP ordering; pitch / mouse-look; volumetric fog real; texture scrolling validation; decals dinámicos; invuln invert-colors real (necesita custom shader); weapon shading por luz de sector.

**Fase 3.17 (2026-05-26, este bloque):** mouse-look cosmético (pitch via y-shear).

- **Contexto.** Doom clásico no tiene pitch — las hitboxes son cilindros infinitos verticales y los proyectiles autoapuntan en Y. Implementar "mirar arriba/abajo" como pitch real del motor rompería la simulación (cambia raycasts, AI sight, etc.). En cambio, modernizamos sólo la **percepción** vía la técnica clásica de los engines pre-real-3D (Build, ZDoom software, Heretic): un *y-shear* del rasterizador que mueve la línea del horizonte en pantalla, sin tocar timing/RNG/hitboxes.
- **`supay-scene::PlayerSnap.view_pitch`** field nuevo (radianes; positivo = mirando arriba). `Default` lo deja en 0.0. `interpolate` hace lerp lineal entre prev/next — coherente con la suavidad de los cambios de pitch del usuario por tap de PageUp/PageDown.
- **`supay-core::capture_scene_real`** no toca el campo (queda en 0.0 de `PlayerSnap::default`). `synth_snapshot` igual. El motor Doom no conoce pitch; el host lo inyecta post-capture si quiere mouse-look.
- **`supay-render-llimphi::Projection`** gana `pitch_offset_px = focal · tan(view_pitch)` precomputado en `Projection::new_pitched(rect, fov_y, pitch)`. `project(x_cam, y_cam, z_cam)` suma este offset a `sy` — afecta uniformemente todos los puntos proyectados (independiente de profundidad), equivalente a deslizar la línea del horizonte arriba/abajo en pantalla. Clampeo defensivo a `±PITCH_MAX = π/3` para evitar `tan()` explotando y horizontes fuera del viewport.
- **`render_frame`** lee `snap.player.view_pitch` y construye el `Projection` con `new_pitched`. Pisos/techos, paredes y sprites usan todos `proj.project`, así que el shear se propaga sin tocar gather_*. El weapon sprite y los player overlays no van por la proyección (son HUD layer) → quedan anclados a la pantalla, como debe ser.
- **`draw_backdrop`** sigue el horizonte: el `mid_y` que separa sky/floor se desplaza por `focal · tan(pitch)` (clampeado a los bordes del rect). El affine de SKY1 mantiene `scale_y` constante (`tex_h / (rect.h/2)`) — el panorama no se estira — pero su offset `f` (donde cae `iy=0`) se ajusta para que `iy=tex_h` quede sobre `mid_y_unclamped`. Vello recorta con el `sky_rect` cuando el pitch es agresivo. El fallback `SKY_BAND_TOP` (color plano) hereda el `sky_rect` shifted automáticamente.
- **Host (`supay-doom-llimphi`)**: `Model.view_pitch: f32` + `Msg::PitchDelta { delta, reset }`. `on_key` intercepta PageUp (+0.105 rad ≈ +6°), PageDown (-6°), Home (reset a 0). Cada Msg::Tick hace `snap.player.view_pitch = m.view_pitch` justo antes de `pair.push(snap)`. Las teclas no se forwardean al motor C (Doom no usa PgUp/PgDn/Home en gameplay). Latencia máxima de 1 tick (~28.5 ms) entre tap y cambio visual — imperceptible.
- **Tests** (+4 render = 39 total verde, 44 supay total): `projection_pitch_up_shifts_horizon_down` (verifica offset = `focal · tan(pitch)`), `projection_pitch_down_shifts_horizon_up` (caso espejo), `projection_pitch_does_not_alter_x` (y-shear es vertical puro), `projection_pitch_clamps_extremes` (valores absurdos clampean a PITCH_MAX).
- **Header bump**: `PHASE 3.16` → `PHASE 3.17`.

**Limitaciones conocidas de 3.17.**
- **No es 3D real**, es y-shear. Mirando muy arriba/abajo las paredes se ven "geometricamente extrañas" — los costados verticales no se inclinan correctamente. Es exactamente el artefacto que tenían Build/ZDoom software; aceptable mientras la jugabilidad sea zenital-ish. Pitch máximo ±π/3 mitiga.
- **Hitboxes / disparo siguen sin pitch** — exactamente lo que queremos (preserva el contrato). Pero significa que mirar hacia arriba "para apuntarle a un enemigo en una plataforma alta" es cosmético — el motor autoapunta como siempre.
- **No hay mouse capture**, sólo PageUp/PageDown. Cuando llimphi-ui exponga mouse delta + cursor capture, conectar el delta vertical a `Msg::PitchDelta { delta: dy * sensitivity, reset: false }` es trivial.
- **Sin smoothing del input** — un tap = un step de 6°. Con un poco de spam queda usable; con mouse real va a ser orgánicamente suave por sí solo (deltas pequeños).

**No incluido en 3.17 (defer a 3.18+):** wall + sprite BSP ordering; mouse capture real (depende de llimphi-ui); volumetric fog real; texture scrolling validation; decals dinámicos; invuln invert-colors real; weapon shading por luz de sector; pitch realmente 3D (paredes inclinadas) — defer indefinidamente, exige reescribir todo el render pipeline.

**Fase 3.18 (2026-05-27, este bloque):** weapon shading por luz del sector del jugador.

- **Contexto.** Desde Fase 3.15 el sprite del arma (`psprites[ps_weapon]`) se pinta a luz plena siempre — daba igual si el jugador estaba en un cuarto a oscuras o frente a una antorcha encendida. Visualmente quedaba "stickered" sobre la escena: el cuarto fundía a negro pero la pistola seguía amarilla. Doom original también pinta el arma sin shading (sólo el PLAYPAL aplica), pero en un renderer 3D con luz por sector se nota más. Modernizamos: el arma se atenúa con el `light_level` del sector donde está parado el jugador.
- **`supay-render-llimphi::subsector_at_point`** (nuevo): O(log N) walk del árbol BSP que devuelve el subsector que contiene `(px, py)`. Sigue siempre el lado "near" (mismo signo que `walk_bsp`) hasta caer en una hoja. `None` si el snapshot no trae BSP (modo stub) o el camino apunta fuera de rango.
- **`supay-render-llimphi::player_sector_light`** (nuevo): compone la cadena `subsector_at_point → subsectors[ss].sector → sectors[sec].light_level`. Fallback `DEFAULT_PLAYER_LIGHT = 192` (mismo valor que `gather_sprite` usa para sprites sin sector).
- **`draw_weapon_sprite`** gana parámetro `player_light: u8`. El RGBA del patch pasa por `make_tinted_sprite_image(patch, shade)` (helper que ya existía para sprites de mundo). Depth = 0 al `shade_for` — el arma está "en la mano", no debería atenuarse por niebla aunque el cuarto sí lo esté. `FF_FULLBRIGHT` (bit 7 del frame) saltea el shading — muzzle flashes y plasma idle siguen brillantes a luz plena.
- **`render_frame`** resuelve `player_sector_light(snap)` una sola vez y lo pasa a ambos `draw_weapon_sprite` (weapon + flash). Cero costo extra si no hay BSP.
- **Header bump**: `PHASE 3.17` → `PHASE 3.18`.
- **Tests** (+4 render = 43 total verde): `subsector_at_point_picks_leaf_containing_point` (verifica que la dirección "near" lleva al leaf correcto), `subsector_at_point_none_without_bsp` (snapshot stub), `player_sector_light_picks_local_light_level` (dos sectores con luces opuestas, el player en cada lado lee la suya), `player_sector_light_falls_back_without_bsp` (sin BSP devuelve 192).

**Limitaciones conocidas de 3.18.**
- **Sin smoothing**: el arma cambia de brillo instantáneo al cruzar un sector — un step. Doom mismo se comporta así (cada tick muestra el light_level del sector actual sin transición). Si se quiere fade, sería un campo del Model en el host con lerp por tick.
- **No considera luces dinámicas**: si un muzzle flash ilumina la pared, el arma sigue al brillo del sector base. Para luces dinámicas hace falta un canal de iluminación adicional (Fase 4+).
- **Player_overlays todavía no respetan luz**: los PLAYPAL flashes (damage, pickup, radsuit, invuln) son full-screen overlay, sin shading. Es correcto — son tinte de pantalla, no objetos del mundo.

**No incluido en 3.18 (defer a 3.19+):** wall + sprite BSP ordering (sigue pendiente); mouse capture real; volumetric fog; decals dinámicos; invuln invert-colors real; smoothing de la luz del arma.

**Fase 3.22 (2026-05-29, este bloque):** muzzle world light — el fogonazo del arma ilumina el mundo.

- **Contexto.** Desde 3.15 / 3.16 el destello del arma (`psprites[ps_weapon]` con `PISGB`/`SHTGB` y `psprites[ps_flash]` para chaingun/plasma/BFG/shotgun) se renderea como overlay 2D sobre la vista, brillante por su flag `FF_FULLBRIGHT` (bit 7 del frame). Pero el mundo a su alrededor no reacciona: en cuartos oscuros disparás y el cuarto sigue negro. Doom original tampoco lo modela renderer-side — cicla la PLAYPAL global para tintar la pantalla entera. Esta fase moderniza: el destello irradia una luz cálida puntual desde el jugador que tinte paredes, pisos, techos y sprites cercanos durante unos pocos ticks.
- **Detección host-side (`supay-doom-llimphi`).** Cada `Msg::Tick`, tras capturar el snapshot, se chequea si `snap.weapon.frame & 0x80` o `snap.weapon_flash.frame & 0x80` (ambos sólo si `active`). Si cualquiera está set, se peguea `Model.muzzle_glow_at = Some(Instant::now())`. `muzzle_alpha_now(model)` calcula `(1 - elapsed/MUZZLE_DECAY_SECS).max(0)` cada vez que `view()` se rebuilda (60 Hz), pasando al `RenderConfig.muzzle_glow_alpha`. `MUZZLE_DECAY_SECS = 0.16` (~5-6 ticks Doom): el fogonazo decae perceptiblemente entre tiros sucesivos, pero se reescribe a 1.0 si el siguiente tick vuelve a ver `FF_FULLBRIGHT`.
- **Constantes del renderer.** `MUZZLE_RADIUS_WORLD = 384.0` unidades Doom (~ 6 cells de 64 — una habitación pequeña o un pasillo medio). `MUZZLE_BOOST_PEAK = 0.55` (incremento máximo de `shade` en el centro). `MUZZLE_TINT_RGB = (255, 220, 140)` — blanco cálido amarillento, riff sobre la temperatura de un disparo de pólvora.
- **`muzzle_boost_cam(x_cam, y_cam, alpha) -> f32`**. El jugador está siempre en el origen del cam-space, así que la distancia² al punto es `x²+y²`. Falloff cuadrático: `f = 1 - d²/r²`, `boost = f² · alpha · PEAK`. Fuera del radio o con `alpha ≤ 0` devuelve 0 (fast path — no tocar el color).
- **`apply_muzzle_tint(c, boost) -> Color`**. Suma aditivamente `MUZZLE_TINT_RGB · boost` por canal, preservando el alpha. Aplicado al color final de paredes untextured (fallback de bandas en `gather_wall`), pisos/techos untextured (fallback de `gather_subsector_planes`), pisos/techos del path fake-floor de `gather_wall`, y al sprite_color del fallback de `gather_sprite`.
- **Overlay yellow textured.** Para paredes y planos texturizados emitimos dos overlays sobre la textura: (i) el oscuro existente con alpha derivado de `lit_shade = shade + boost` clampeado (≤ 1) — el boost reduce la oscuridad; (ii) un nuevo overlay aditivo del tinte cálido con `alpha = boost · 180` clampeado, sólo si `boost > 0.02`. Vello blendea ambos sobre la imagen del flat/wall — el efecto neto es "habitación oscura iluminada brevemente por el fogonazo" sin recachear texturas ni tocar la pipeline de Image.
- **Sprites texturizados.** `make_tinted_sprite_image_rgb(patch, [r, g, b])` reemplaza al `make_tinted_sprite_image(patch, shade)` (que ahora es wrapper que llama con `[shade, shade, shade]`). El multiplicador per-canal viene de `sprite_shade_with_muzzle(shade, boost)`: cuando `boost > 0` cada canal queda `(shade · (1 + boost · tint_canal/255)).clamp(0, 1)`. Los proyectiles full-bright reciben el tinte amarillo sin saturar (peak en R=1.0 ya satura, los demás canales se acercan a 1 sin pasarse).
- **Toggle F8.** `Model.muzzle_world_light: bool` (default `true`). `RenderConfig.muzzle_glow_alpha = muzzle_alpha_now(model)` — cuando el toggle está apagado, devuelve 0 sin importar el último fogonazo y limpia `muzzle_glow_at`. F8 alterna desde el host. Hint del footer suma `F8 fogonazo` (es) / `F8 muzzle` (en) / `F8 q'ancha` (qu) — al máximo permitido por la línea, las etiquetas se acortan.
- **Costo.** Por frame: 1 fill extra por plano texturizado (cuando hay boost), 1 fill extra por slab texturizado de pared (cuando hay boost), un `make_tinted_sprite_image_rgb` con multiplicador per-canal (la lambda toma el mismo costo que el path histórico — se sigue clonando el RGBA tinted del patch). En modo "no flash" (la mayor parte del tiempo), `boost ≤ 0` y todos los paths devuelven el color base con un branch barato.
- **Header bump**: `PHASE 3.21` → `PHASE 3.22`.
- **Tests** (+8 render = 51 total verde, 51 supay total — supay-wad/scene/core ya en 0): `muzzle_boost_zero_when_alpha_zero`, `muzzle_boost_zero_outside_radius`, `muzzle_boost_peak_at_center_with_full_alpha`, `muzzle_boost_falls_off_with_distance_squared` (ratio close/mid > 1.4 verifica el falloff cuadrático), `apply_muzzle_tint_warms_color` (R ≥ G > B + alpha preservada), `apply_muzzle_tint_zero_is_identity`, `sprite_shade_with_muzzle_zero_is_grayscale`, `sprite_shade_with_muzzle_warm_when_boost_positive`.

**Limitaciones conocidas de 3.22.**
- **Sin oclusión.** El boost ignora paredes entre el jugador y la superficie — un sprite atrás de una pared cercana también recibe boost si su distancia euclidiana está bajo el radio. Para Doom típico es invisible (las habitaciones son chicas y el radio cubre apenas el cuarto actual + adyacente), pero en mapas con corridors largos o sectors interpenetrados puede notarse "ilumina a través de la pared". Fix: BSP point query desde el jugador + comparar subsector con el del target — defer hasta sprite-BSP ordering (pendiente desde 3.13).
- **Sin smoothing del peak.** Cada tick que ve `FF_FULLBRIGHT` reseta el alpha a 1.0; si el motor lo deja set durante 2 ticks consecutivos (frame `PISGB` típicamente dura 4 ticks pero el flash dura 1-2), tenés un sub-decay. Imperceptible en práctica.
- **`draw_weapon_sprite` no se boostea**. El arma se sigue tintando con el `player_light` del sector (Fase 3.18); el muzzle boost no se le suma. Es coherente — el arma está "en la mano del jugador", el destello sale *desde* ella, no la ilumina a ella. Si llegara a verse falsa, se puede sumar boost al `shade` que entra a `make_tinted_sprite_image` por consistencia.

**No incluido en 3.22 (defer a 3.23+):** sprite-BSP query para oclusión real del boost; smoothing por interp entre snapshots del alpha; muzzle dynamic shadow del jugador (player rim-lit con su propia luz); volumetric god rays desde el cañón.

**Fase 3.23 (2026-05-29, este bloque):** oclusión sectorial del muzzle boost — el fogonazo respeta paredes.

- **Contexto.** El boost cálido del 3.22 ignoraba la geometría: un sprite atrás de una pared sólida, o un cuarto vecino al otro lado de un muro cerrado, igual recibían tinte amarillo si la distancia euclidiana al player caía bajo `MUZZLE_RADIUS_WORLD = 384`. En mapas Doom con pasillos largos o sectors interpenetrados se notaba como "luz a través de la pared". El fix de pixel-perfect (rayos por sprite + check por subsector ↔ subsector del player) requería extender `SpriteSnap`/`WallSeg` con BSP-step. Esta fase entrega oclusión barata por **sector + vecinos directos** que cubre los casos visibles sin tocar la simulación.
- **`supay-render-llimphi::compute_muzzle_lit_sectors(snap) -> Option<HashSet<u32>>`** (nuevo). Devuelve el conjunto de sectores donde el muzzle boost está permitido: el sector del player (resuelto vía `subsector_at_point` → `subsectors[ss].sector`, Fase 3.18) más todos los sectores conectados a él por al menos una linedef two-sided (`wall.back_sector != NO_SECTOR && (front == player_sec || back == player_sec)`). Costo: O(walls) por frame — ~1000 walls × ~30 muzzle frames/segundo. `None` si no hay BSP (modo stub, mapa pre-carga) — caller asume "todo lit" y se preserva el comportamiento 3.22.
- **`muzzle_boost_gated(boost, sector_id, lit) -> f32`** (nuevo). Gate del boost por sector: si `lit.is_some() && !lit.contains(sector_id)`, devuelve 0. Sin lit set o sector_id presente, pasa el boost crudo.
- **`gather_wall/subsector_planes/sprite`** ganan un parámetro nuevo `lit_sectors: Option<&HashSet<u32>>`. El muzzle_boost se computa como antes pero pasa por `muzzle_boost_gated(..., near_idx | sub.sector | sprite.sector, lit_sectors)` justo después. El resto del path (overlays cálidos, sprite_shade_with_muzzle, apply_muzzle_tint) consume el boost gateado sin saber del lit set.
- **`render_frame`** computa `lit_sectors` una sola vez por frame, sólo cuando `cfg.muzzle_occlusion && cfg.muzzle_glow_alpha > 0.0` (fast path: si no hay flash activo o el toggle está apagado, no calculamos nada). El `Option<&HashSet>` se propaga a las tres funciones gather sin clonar.
- **`RenderConfig.muzzle_occlusion: bool`** (default `true`). Cuando `false`, `render_frame` no llama a `compute_muzzle_lit_sectors` → `lit_sectors = None` → comportamiento 3.22 restaurado para comparar.
- **Host (`supay-doom-llimphi`)**: `Model.muzzle_occlusion: bool` (default `true`) + `Msg::ToggleMuzzleOcclusion` + tecla **F9**. Footer hint suma `F9 occl` (en) / `F9 oclusión` (es) / `F9 hark'ay` (qu — "hark'ay" significa detener/contener en quechua, semánticamente apropiado para oclusión).
- **Resultado visible.** En E1M1 dentro del cuarto inicial (sector tipo "starting room"), disparar contra una pared cerrada: las paredes y suelos del cuarto se iluminan con el flash amarillo cálido (sector del player). Pasillos detrás de puertas cerradas, sprites del cuarto siguiente sin opening directo, geometría más allá del primer vecino → quedan a la luz base sin recibir el boost. Al abrir la puerta, el next sector entra al lit set automáticamente y el siguiente fogonazo lo ilumina.
- **Tests** (+7 render = 58 total verde, 58 supay total): `lit_sectors_includes_player_sector`, `lit_sectors_includes_adjacent_via_twosided`, `lit_sectors_excludes_unconnected_sector` (sector con sólo paredes one-sided queda fuera), `lit_sectors_none_without_bsp` (stub mode), `muzzle_boost_gated_passes_through_when_lit_none` (preservación 3.22 sin BSP), `muzzle_boost_gated_keeps_when_sector_in_lit`, `muzzle_boost_gated_zeroes_when_sector_not_in_lit`.
- **Header bump**: `PHASE 3.22` → `PHASE 3.23`.

**Limitaciones conocidas de 3.23.**
- **Sólo un nivel de adyacencia.** Vecinos del vecino (sectors a 2 puertas del player) no entran al lit set, aunque en mapas típicos el radio de 384 unidades los alcanza raras veces. Si se ve "corte abrupto" en un pasillo largo abierto, considerar BFS multi-nivel limitada por radio.
- **Vecindad por linedef, no por visibilidad.** Si dos sectores están conectados por una linedef two-sided pero con un escalón que oculta uno desde el otro (e.g. un balcón), igual son "adyacentes" y se iluminan mutuamente. Para visibilidad real haría falta `R_CheckSight` (raycast por seg) — defer.
- **El threshold del overlay cálido (`muzzle_boost > 0.02`)** sigue activo: gateo a 0 elimina los overlays por completo en sectores no-lit, lo cual es el efecto deseado. No hay regresión visual cuando un sector debería estar iluminado y antes lo estaba — sólo se apagan los que sobraban.

**No incluido en 3.23 (defer a 3.24+):** sprite-BSP true occlusion vía R_CheckSight; smoothing del muzzle alpha por interp entre snapshots; muzzle dynamic shadow del jugador (player rim-lit con su propia luz); volumetric god rays; BFS multi-nivel de adyacencia limitada por radio.

**Fase 3.24 (2026-05-29, este bloque):** BFS multi-hop + filtro por radio en el lit set del muzzle.

- **Contexto.** El 3.23 sólo agregaba al lit set los **vecinos directos** del sector del jugador (1 hop por linedef two-sided). En mapas Doom con dos puertas en cadena — por ejemplo, el corredor de salida de E1M1 visto desde el cuarto inicial — el sector más lejano de la cadena quedaba oscuro durante el fogonazo aunque su geometría estuviera dentro de los 384 unidades del radio. Esta fase extiende la propagación a **2 hops** con un corte adicional: cada wall "puente" del BFS debe tener su midpoint dentro de `MUZZLE_RADIUS_WORLD` para contar. El filtro físico evita iluminar habitaciones colgadas al final de pasillos largos donde la adyacencia topológica es real pero la luz nunca llegaría.
- **`MUZZLE_BFS_MAX_HOPS = 2`** (constante nueva). 1 hop preserva el comportamiento 3.23; 2 hops es el sweet spot — mayor empieza a "filtrar" luz por geometrías retorcidas sin agregar realismo (y el radio físico ya cortaría antes).
- **`compute_muzzle_lit_sectors` reescrito como BFS**. Frontier inicial = `[player_sec]`. Por cada hop, recorre `snap.walls`: descarta walls con `back_sector == NO_SECTOR`, descarta walls con midpoint a distancia² > `MUZZLE_RADIUS_WORLD²` del jugador, y propaga la membresía cuando exactamente un lado del wall ya está en la frontera. `HashSet::insert` deduplica naturalmente. Frontera vacía corta el loop temprano.
- **Costo.** O(walls · hops). En E1M1 (~400 walls × 2 hops) eso son ~800 comparaciones por frame que el flash está activo (<5 % del tiempo). Sin alocaciones extra significativas — el `next_frontier` típicamente queda <16 elementos.
- **Resultado visible.** Con F9 activo (oclusión on), disparar en el cuarto inicial de E1M1: la habitación se ilumina y el corredor saliente (1 puerta más allá) también recibe el destello, dándole "alcance" creíble al flash. Con dos puertas más adelante, el siguiente cuarto queda oscuro — buen contraste físico. F9 off vuelve a 3.22 (todo lit dentro del radio).
- **Tests** (+3 render = 61 total verde): `lit_sectors_includes_two_hop_neighbor_within_radius`, `lit_sectors_bfs_stops_at_max_hops` (sector a 3 hops no entra al lit), `lit_sectors_excludes_one_hop_when_bridge_wall_beyond_radius` (filter físico aplica también al 1er hop — vecino directo con bridge wall fuera de 384 queda excluido).
- **Compatibilidad con 3.23.** Los 7 tests de 3.23 siguen verdes — los snaps de adyacencia simple tienen walls con midpoint en `(0, 0)`, dentro del radio desde player `(-10, 0)`.
- **Header bump**: `PHASE 3.23` → `PHASE 3.24`.

**Limitaciones conocidas de 3.24.**
- **Bridge filter es por midpoint**, no por el wall completo. Una pared muy larga que cruza el radio en uno de sus extremos pero tiene midpoint afuera queda descartada. Caso raro en Doom (los linedefs son cortos por construcción BSP); aceptable.
- **Sin radio cumulativo por hop.** El filtro evalúa cada bridge wall contra el centro del player, no contra el midpoint del sector previo. En cadenas curvas (un pasillo con codo), el segundo hop podría caer fuera del radio aunque tener "menos" distancia real recorrida por la cadena. Funciona bien con geometrías rectas; en U-shapes/L-shapes la heurística es conservadora (corta más temprano).
- **Sigue sin R_CheckSight.** Dos sectores con linedef two-sided y un escalón que oculta uno desde el otro siguen siendo "vecinos lit" para el BFS. Defer.

**No incluido en 3.24 (defer a 3.25+):** sprite-BSP true occlusion vía R_CheckSight; smoothing del muzzle alpha por interp entre snapshots; player rim-lit con su propia luz; volumetric god rays; radio cumulativo por hop (camino más corto en lugar de distancia directa).

**Fase 3.25 (2026-05-30, este bloque):** radio cumulativo por hop — Dijkstra-lite sobre midpoints encadenados.

- **Contexto.** El 3.24 chequeaba cada bridge wall **contra el centro del jugador**: si el midpoint del wall estaba dentro de `MUZZLE_RADIUS_WORLD` del player, propagaba. En cadenas rectas funcionaba bien, pero en U-shapes/L-shapes daba un falso positivo — el último wall de un codo podía quedar cerca del jugador en línea recta aunque el camino real para llegar a él (atravesando puerta 1 → puerta 2) recorriera mucho más que el radio. El resultado visual: sectores que estaban "al otro lado del codo" se iluminaban con el fogonazo aunque la luz física nunca llegaría doblando dos esquinas.
- **`compute_muzzle_lit_sectors` reescrito como Dijkstra-lite.** Cada sector visitado cachea `(dist_cumulativo_desde_player, midpoint_del_bridge_de_entrada, hops)`. El midpoint del último bridge es el **entry point** del siguiente hop — el nuevo `hop_d` se mide desde ahí, no desde el jugador. Cuando un sector se relaja por un camino más corto (multi-camino en sectores con varios bridges), `dist` se actualiza al mínimo y `entry` al midpoint del camino corto. La cola es un `Vec` con re-inserción on-better (no BinaryHeap real — los sets típicos son <16 sectores, basta).
- **`MUZZLE_BFS_MAX_HOPS = 2`** se preserva como cap dual al radio cumulativo. En geometrías Doom típicas el radio corta antes (~3-5 hops alcanzables en `r=384` con sectores chicos), pero el hop cap protege contra mapas con sectores muy planos donde el radio podría no morder.
- **Resultado visible.** En cuartos con codos (L-shape pasillos, balcones que dan vuelta), el segundo cuarto detrás del codo ya no recibe el destello del arma — el camino real es más largo que `MUZZLE_RADIUS_WORLD`. En cadenas rectas el comportamiento queda idéntico al 3.24 (cumulative == player-distance cuando todo está en línea). En geometrías con dos puertas alternativas hacia el mismo sector, gana el camino más corto.
- **Costo**. O(walls · hops · sectores_visitados) por frame que el flash está activo. En E1M1 (~400 walls × 2 hops × <16 sectores) ≈ 13k checks/frame durante el ≤5 % del tiempo con flash. Sin alocaciones extra significativas (HashMap con `capacity(16)`).
- **Compatibilidad 3.24.** Los 7 tests del 3.23 + 3 del 3.24 siguen verdes: en `snap_with_chain` y `snap_with_adjacency` la geometría es lineal/colocalizada y `cumulative == player_dist`. El test `lit_sectors_bfs_stops_at_max_hops` sigue cubriendo el hop cap. Triangle inequality garantiza que 3.25 es **estrictamente más conservador** que 3.24 — nunca enciende sectores que 3.24 apagaba, sólo apaga sectores que 3.24 encendía erróneamente.
- **Tests** (+2 render = 63 total verde): `lit_sectors_cumulative_path_cuts_when_sum_exceeds_radius` (L-shape distinguidor: ambos walls pasan el chequeo per-bridge del 3.24, cumulativo 410 > 384 corta el sec 2); `lit_sectors_cumulative_uses_wall_midpoint_as_entry` (cadena con midpoints lejos del player pero cerca entre sí — sólo el entry-chaining lo deja entrar al lit, sin él caería por triangle inequality).
- **Header bump**: `PHASE 3.24` → `PHASE 3.25`.

**Limitaciones conocidas de 3.25.**
- **Sigue sin R_CheckSight.** Two-sided + escalón opaco siguen siendo "vecinos lit" para el grafo. Defer.
- **Sin smoothing del muzzle alpha por interp.** El alpha decae linealmente con `Instant::now() - muzzle_glow_at`, no interpola entre snapshots. Imperceptible a 60 fps.
- **Hop cap dual.** `MUZZLE_BFS_MAX_HOPS=2` sigue de safety net. En la mayoría de mapas el radio corta antes; el cap sólo morde en sectores muy planos. Si quisieras llegar más lejos en pasillos rectos largos, bumpear el cap es seguro porque el radio cumulativo seguirá cortando físicamente.

**No incluido en 3.25 (defer a 3.26+):** sprite-BSP true occlusion vía R_CheckSight; smoothing del muzzle alpha por interp entre snapshots; player rim-lit con su propia luz; volumetric god rays; decals dinámicos del impacto del disparo.

**Fase 3.26 (2026-05-30, este bloque):** world point lights desde mobjs `FF_FULLBRIGHT` — proyectiles y explosiones iluminan el mundo.

- **Contexto.** Doom marca varios mobjs con el bit `FF_FULLBRIGHT` (bit 7 del frame): proyectiles en vuelo (BAL1 imp fireball, BAL2 caco fireball, BAL7 baron fireball, MISL rocket, PLSS plasma, BFG ball), muzzle puffs (`PUFF`), frames de explosión (`BEXP` del barril, `MISL` cuando detona), BFG splash, teleport fog. Desde Fase 3.11 estos sprites ya se pintan a luz plena (sprite-side), pero **no irradiaban luz al mundo**: un fireball pasando por un cuarto oscuro lo dejaba oscuro. Doom clásico tampoco lo modela renderer-side. Esta fase generaliza la maquinaria de muzzle light (3.22-3.25) a cada mobj FF_FULLBRIGHT — el muzzle del jugador queda como un caso particular anclado en el origen del cam-space.
- **Constantes del renderer.** `WORLD_LIGHT_RADIUS_WORLD = 192.0` (mitad del muzzle: una bola de plasma no debe iluminar un cuarto entero). `WORLD_LIGHT_PEAK = 0.40` (vs. `MUZZLE_BOOST_PEAK = 0.55` — el sumado de varias luces se acerca al peak del muzzle pero una sola no debe "blow out" la escena). `MAX_WORLD_LIGHTS = 8` (cap para O(surfaces·lights) bounded — cubre cyberdemon spam y BFG en cluster; el resto se descarta por distancia).
- **`gather_world_lights(snap, cam) -> Vec<WorldLight>`** (nuevo). Filtra `snap.sprites` por bit `0x80` del frame + `sector != NO_SECTOR`, transforma cada posición a cam-space, descarta los que estén >2× del radio (fast reject), y `select_nth_unstable_by` recorta a los `MAX_WORLD_LIGHTS` más cercanos al jugador. O(sprites + N log N) por frame con N << MAX. En mapas Doom típicos N suele ser 0-4 (muy pocas FF_FULLBRIGHT activas a la vez).
- **`world_lights_boost_cam(x_cam, y_cam, lights) -> f32`** (nuevo). Suma `f²·PEAK` con `f = 1 - d²/r²` por cada luz, fast-return cuando sum ≥ `MUZZLE_BOOST_PEAK` (early-exit barato). Sin gating por sector — sólo por radio. La radio chica (192 vs 384) limita el leak natural a través de paredes; los mobjs FF_FULLBRIGHT son efímeros (1-30 ticks típicamente), el leak fugaz es invisible.
- **`combined_boost_cam(x, y, alpha, surf_sec, lit_sectors, world_lights) -> f32`** (nuevo). Unifica los dos componentes: `muzzle_gated + world_lights_sum`, clampeado a `MUZZLE_BOOST_PEAK`. Reemplaza el patrón `let raw = muzzle_boost_cam(...); let gated = muzzle_boost_gated(raw, sec, lit)` en `gather_wall`, `gather_subsector_planes` (centroide) y `gather_sprite` (path texturizado + fallback) — 4 sites en total.
- **Plumbing.** Las tres `gather_*` ganan parámetro `world_lights: &[WorldLight]`. `render_frame` computa la lista una sola vez por frame y la pasa por slice (zero-cost). Cuando `cfg.world_lights_enabled = false`, la lista queda vacía y el path es no-op (early-return en `world_lights_boost_cam` con `lights.is_empty()`).
- **Toggle F10.** `RenderConfig.world_lights_enabled: bool` (default `true`). `Model.world_lights_enabled` + `Msg::ToggleWorldLights` en el host. Footer hint suma `F10 mobj-lit` (en) / `F10 luz-mobj` (es) / `F10 mobj-k'anchay` (qu — "k'anchay" significa iluminar/alumbrar en quechua).
- **Tinte.** Reusamos `MUZZLE_TINT_RGB = (255, 220, 140)` para todas las world lights. Doom proyectiles tienen colores característicos (BFG verde, plasma azul, rocket naranja, fireballs rojo-naranja); por-tinte queda diferido hasta tener `WorldLight.tint_rgb` — el costo es minimal (un `(u8,u8,u8)` por luz) pero necesita una tabla `spritenum → tint` que prefiero curar en una fase aparte.
- **Resultado visible.** Disparar la chaingun en un cuarto oscuro: los proyectiles dejan trazadores cálidos que iluminan paredes mientras vuelan. Un imp lanzando un fireball detrás de una pared ahora ilumina su cuarto (con el leak limitado del radio chico). Una explosión de barril irradia el destello brevemente. Plasma en cluster genera halos overlap antes del cap. En cuartos bien iluminados (light_level alto) el efecto es discreto — donde se nota es exactamente donde Doom clásico se sentía estéril: corredores oscuros + proyectiles brillantes.
- **Costo**. `gather_world_lights` por frame: O(sprites) ≈ <100. `combined_boost_cam` por superficie shaded: O(N) con N ≤ 8 — ~330 superficies/frame × 8 = 2640 ops/frame, despreciable. Sin alocaciones extra significativas (el `Vec<WorldLight>` es minúsculo, `select_nth_unstable_by` es in-place).
- **Compatibilidad 3.25.** Los 10 tests previos del muzzle (3.22-3.25) siguen verdes — `world_lights = &[]` (snapshots sin FF_FULLBRIGHT sprites) reduce `combined_boost_cam` a `muzzle_boost_gated(muzzle_boost_cam(...))` — equivalencia bit-exacta del path 3.25.
- **Tests** (+8 render = 71 total verde): `world_lights_boost_zero_with_empty_list`, `world_lights_boost_peak_at_center_with_single_light` (single light en (0,0) → peak), `world_lights_boost_zero_outside_radius` (radio + más allá → 0), `world_lights_boost_falls_off_with_distance_squared` (ratio close/mid > 1.4), `world_lights_boost_sums_multiple_sources_clamped_to_muzzle_peak` (suma capada a peak del muzzle), `gather_world_lights_filters_non_fullbright`, `gather_world_lights_skips_no_sector_and_caps_to_max`, `combined_boost_clamps_to_muzzle_peak_when_muzzle_and_lights_overlap`.
- **Header bump**: `PHASE 3.25` → `PHASE 3.26`.

**Limitaciones conocidas de 3.26.**
- **Sin tinte per-mobj.** Todas las world lights usan el mismo amarillo cálido (`MUZZLE_TINT_RGB`). BFG green, plasma blue, rocket orange siguen quedando "cálidos" — el efecto se lee bien pero pierde character. Defer a 3.27+ con tabla `spritenum → tint_rgb`.
- **Sin gating por oclusión.** Una pared sólida entre la luz y la superficie no corta el boost (sólo el radio lo hace). En corredores largos con un fireball del otro lado, podés ver leak. El radio chico (192) y la corta vida de los mobjs FF_FULLBRIGHT (~10 ticks promedio) lo hacen invisible en práctica; si llegara a molestar, BFS por luz (caro pero acotado por radio).
- **Cap dura a 8 lights**. Cyberdemon descargando con BFG en cluster + barriles cascadeando podría empujar más, pero las 8 más cercanas dominan el efecto visual.
- **Sin shading por sector base.** Las world lights se suman al `shade` del sector como el muzzle hace — un cuarto a oscuras se ve menos oscuro durante el flash. Apropiado. No considera ocluder verticales (un techo bajando puede tapar visualmente la luz desde arriba).

**No incluido en 3.26 (defer a 3.27+):** tinte per-spritenum (BFG verde, plasma azul, rocket naranja, fireballs rojizos); sprite-BSP true occlusion vía R_CheckSight; smoothing del muzzle alpha por interp entre snapshots; player rim-lit; volumetric god rays; decals dinámicos del impacto del disparo.

**Fase 3.27 (2026-05-30, este bloque):** tinte per-spritenum + boost RGB per-canal.

- **Contexto.** Hasta 3.26 todas las world lights usaban el mismo amarillo cálido (`MUZZLE_TINT_RGB`). Pero un proyectil BFG es verde fluorescente, una bola de plasma es azul cyan, un fireball de imp es rojo-naranja, una antorcha azul de decoración tiñe su cuarto azul. Esta fase refactoriza el boost a representación per-canal (`[f32; 3]`) para que cada luz emita su tinte característico, sumándose aditivamente en RGB. La maquinaria scalar histórica (`muzzle_boost_cam`, `apply_muzzle_tint`, `sprite_shade_with_muzzle`) sobrevive como `#[cfg(test)]` (referencia + cobertura de tests existentes); el render loop pasa íntegro al path RGB.
- **Tabla `FB_SPRITE_TINTS`** (24 entradas). Match 4-char case-insensitive sobre el nombre del sprite del WAD:
  - Proyectiles: `BAL1` imp fireball `(255,130,60)`, `BAL2` caco fireball `(255,100,80)`, `BAL7` baron fireball `(140,255,140)`, `PLSS`/`PLSE` plasma `(130,180,255)`, `BFS1`/`BFE1`/`BFE2`/`BFGG` BFG `(160-180,255,160-180)`, `MISL` rocket `(255,180,100)`, `PUFF` bullet puff `(255,220,160)`, `BEXP` barrel/rocket explosion `(255,180,100)`.
  - Fogs: `TFOG` teleport `(140,200,255)`, `IFOG` item respawn `(255,240,140)`.
  - Decoración (FF_FULLBRIGHT constante): `TBLU`/`SMBT` blue torch `(110,160,255)`, `TGRN`/`SMGT` green torch `(140,255,160)`, `TRED`/`SMRT` red torch `(255,140,90)`, `CAND` candle `(255,200,130)`, `CBRA` brazier `(255,170,90)`, `TLMP`/`TLP2` lamps `(255,240,200)`.
  - Fallback: `MUZZLE_TINT_RGB` para nombres desconocidos (preserva el comportamiento 3.26).
- **`WadAtlas::sprite_name(spritenum) -> Option<String>`** (nuevo). Getter público — la maquinaria de set/has ya existía desde 3.4, esto sólo expone la lectura.
- **`WorldLight` gana `tint_rgb: (u8, u8, u8)`** resuelto en `gather_world_lights` vía `atlas.sprite_name(s.sprite)` + `sprite_tint_for_name`. Sin atlas → cae al amarillo (modo stub o WAD no cargado).
- **Tipo `BoostRgb = [f32; 3]`** y constante `ZERO_BOOST`. Cada canal en `[0, MUZZLE_BOOST_PEAK]`. Helpers nuevos:
  - `muzzle_boost_rgb_cam(x, y, alpha) -> BoostRgb`: scalar × `MUZZLE_TINT_RGB/255` per-canal.
  - `world_lights_boost_rgb_cam(x, y, &[WorldLight]) -> BoostRgb`: por luz, `f²·PEAK·(tint/255)` per-canal, sumados + clampeados a peak por canal, con fast-exit si las tres componentes saturan.
  - `muzzle_boost_gated_rgb(boost, sector, lit_sectors) -> BoostRgb`: espejo del gating scalar 3.23.
  - `combined_boost_rgb_cam(...) -> BoostRgb`: muzzle (gateado) + world lights, suma per-canal + clamp por canal.
  - `apply_color_boost(c, boost_rgb) -> Color`: suma aditiva per-canal, preserva alpha. Reemplaza `apply_muzzle_tint`.
  - `sprite_shade_with_world(shade, boost_rgb) -> [f32; 3]`: `shade · (1 + boost_rgb)` per-canal. Reemplaza `sprite_shade_with_muzzle`.
  - `overlay_color_alpha_from_boost(boost_rgb) -> Option<(u8,u8,u8,u8)>`: deriva color overlay + alpha para texturas. Color = boost normalizado al canal más alto (preserva el tinte); alpha = `boost_max · 180 / MUZZLE_BOOST_PEAK`. None si `boost_max ≤ 0.02`.
  - `boost_max(boost_rgb) -> f32`: la componente más alta, usada para "scalar lit" (reducción del overlay de oscuridad).
- **4+ sites en gather actualizados** (`gather_wall` fake-floor + slab texturizado + fallback banda, `gather_subsector_planes` texturizado + fallback color, `gather_sprite` patch texturizado + fallback): scalar `muzzle_boost` → `boost_rgb` con `boost_scalar = boost_max(boost_rgb)` derivado donde se necesita una magnitud única (overlay alpha del darkness reduce).
- **Resultado visible.**
  - Un cuarto con antorcha azul **TBLU** ahora se tinta levemente azulado, no amarillo.
  - Un BFG ball volando por un pasillo tinta paredes y techos verde fluorescente — sin tocar la simulación.
  - Un cuarto con sólo plasma (PLSS) recibe halo azul-cyan; rocket en vuelo (MISL) tinta naranja cálido.
  - Sprites cercanos a un fireball de imp se tintean rojizos por `sprite_shade_with_world([0, .4, .15])`.
  - El muzzle del jugador sigue siendo el mismo amarillo cálido (escenario común — F8 lo activa/desactiva).
- **Compatibilidad 3.26.** El path scalar legacy queda `#[cfg(test)]` con sus 8 tests verde. El path RGB es bit-equivalent al scalar 3.26 cuando todas las luces usan `MUZZLE_TINT_RGB` (caso reducible por `(255, 220, 140)` → per-canal `(1.0, 0.86, 0.55) · scalar` que mapea al mismo blend de `apply_muzzle_tint`).
- **Costo.** Por superficie: 3 multiplicaciones extras per-canal vs scalar 3.26. ~330 superficies × 8 lights × 3 canales ≈ 8000 ops/frame, despreciable. Sin alocaciones nuevas — `BoostRgb` es `[f32; 3]` por valor.
- **Tests** (+12 render = 83 total verde):
  - `sprite_tint_for_name_resolves_known_sprites` (BAL1 rojo, PLSS azul, BFS1 verde, TBLU azul).
  - `sprite_tint_for_name_falls_back_to_muzzle_tint_for_unknown` (XYZW + 4-char match en strings largos).
  - `sprite_tint_for_name_is_case_insensitive`.
  - `muzzle_boost_rgb_uses_muzzle_tint_per_channel` (R > G > B con peak en R).
  - `world_lights_boost_rgb_per_light_tint_dominates` (BFG verde → G alto).
  - `combined_boost_rgb_clamps_each_channel_to_muzzle_peak` (10 luces blancas saturadas → cada canal capeado).
  - `apply_color_boost_adds_per_channel` (boost G-only tinta verdoso, R y B intactas).
  - `apply_color_boost_zero_is_identity`.
  - `sprite_shade_with_world_per_channel` (boost G-only escala sólo G).
  - `overlay_color_alpha_from_boost_normalizes_to_brightest_channel`.
  - `overlay_color_alpha_from_boost_none_when_negligible`.
  - `gather_world_lights_uses_default_tint_without_atlas`.
- **Header bump**: `PHASE 3.26` → `PHASE 3.27`.

**Limitaciones conocidas de 3.27.**
- **Sin gating per-light por oclusión.** Mismas limitaciones del 3.26: una pared sólida entre la luz y la superficie no corta el boost (sólo el radio lo hace). En corredores largos con una BFG ball del otro lado, podés ver leak verde — el radio chico (192) y la vida corta de los mobjs lo hacen invisible en práctica.
- **Tabla curada manual.** Doom 1 + cobertura básica de Doom 2 cubierta; Final Doom, Heretic-compatible WADs o PWADs custom van a caer al amarillo cálido por nombres desconocidos. Defer hasta tener feedback visual real (¿cuáles sprites adicionales notan ausencia de tinte?).
- **Color normalizado en overlay.** El overlay sobre texturas usa SrcOver (no aditivo puro), entonces el color resultante no es matemáticamente correcto en luminancia. Visual ≈ correcto: BFG verde se ve verde, plasma azul se ve azul. Para HDR-correctness habría que sumar en linear-light + tonemap; defer hasta que llimphi-ui exponga custom passes wgpu.
- **Sin animación de intensidad por frame.** Algunos mobjs (BAL1 frame A/B alterna brillantes/dim en Doom original) emiten el mismo tinte. Refleja la simulación tal cual — el motor C ya rota frames y el FF_FULLBRIGHT bit responde por tick.

**No incluido en 3.27 (defer a 3.28+):** sprite-BSP true occlusion vía R_CheckSight; smoothing del muzzle alpha por interp entre snapshots; player rim-lit; volumetric god rays; decals dinámicos del impacto del disparo; tabla de tintes para Final Doom / Heretic-style PWADs.

**Fase 3.28 (2026-05-30, este bloque):** weapon rim-light desde world lights — la pistola recoge tinte ambiente.

- **Contexto.** Hasta 3.27 el sprite del arma del jugador (`snap.weapon`, `snap.weapon_flash`) sólo recibía dos cosas: shading scalar por `player_light` (Fase 3.18) y bypass full-bright cuando `(frame & 0x80) != 0` (muzzle flash, plasma idle). El mundo a su alrededor podía estar lleno de antorchas azules, fireballs naranjas o un BFG ball pasando — el arma seguía pintada con un gris monocromo. Visualmente "stickered": la escena tiene color, la pistola no. Esta fase la liga al ambiente RGB usando la misma maquinaria de 3.27 (`world_lights_boost_rgb_cam`), evaluada en la posición del jugador (origen del cam-space).
- **`RenderConfig::weapon_rim_light: bool`** (nuevo, default `true`). Off vuelve al 3.27 (arma sólo con `player_light`).
- **Boost RGB en origen.** En `render_frame`, una vez calculada la lista `world_lights` del frame, calculamos `weapon_rim_boost = world_lights_boost_rgb_cam(0.0, 0.0, &world_lights)` cuando el toggle está on, o `ZERO_BOOST` cuando off. Reutiliza el cache de luces existente — sin alocaciones nuevas. El muzzle del propio jugador *no* se suma acá: consistente con 3.22, el fogonazo sale *de* la pistola, no la ilumina a ella.
- **`draw_weapon_sprite` gana `rim_boost: BoostRgb`.** En lugar de `make_tinted_sprite_image(patch, shade)` (helper scalar removido en esta fase), llama a `make_tinted_sprite_image_rgb(patch, tint_rgb)` donde:
  - `tint_rgb = [shade, shade, shade]` si `(frame & 0x80) != 0` (full_bright — bypass del rim, el destello del propio fogonazo domina).
  - `tint_rgb = sprite_shade_with_world(shade, rim_boost)` en cualquier otro caso. Per-canal: `(shade · (1 + boost_canal)).clamp(0, 1)`.
- **Visibilidad práctica.** El rim sólo se nota en cuartos relativamente oscuros (shade < 1.0 deja headroom para que `1 + boost_canal` no sature). En cuartos brillantes (shade ≈ 1.0) todos los canales se clampean a 1.0 y el rim desaparece — matemáticamente consistente con "no hay headroom para más luz". Caminar por un pasillo oscuro con antorchas azules tinte la pistola apenas azulada; volver a la sala iluminada → arma vuelve al gris neutro. Un fireball de imp pasando cerca le pinta un rim rojizo de 1-2 frames.
- **Cleanup**: `make_tinted_sprite_image` (wrapper scalar de 3.22 → llamaba a la versión RGB con `[s, s, s]`) queda removido por sin callers. Sigue `make_tinted_sprite_image_rgb` como única API.
- **Toggle host**: F11 alterna `weapon_rim_light`. F8 ya cubría el muzzle del propio jugador (otra fuente de tinte sobre la escena, no sobre el arma).
- **Tests** (+5 render = 88 total verde):
  - `weapon_rim_boost_zero_at_player_with_no_world_lights` (identity sin luces).
  - `weapon_rim_boost_blue_torch_skews_blue_at_player` (TBLU a 120u → B>R, tint final per-canal con shade 0.5).
  - `weapon_rim_boost_red_fireball_skews_red_at_player` (BAL1 a 80u → R>G>B).
  - `weapon_rim_boost_zero_when_light_beyond_radius` (luz > WORLD_LIGHT_RADIUS_WORLD → ZERO_BOOST).
  - `weapon_full_bright_bypasses_rim_boost` (path full_bright es grayscale, normal preserva asimetría).
- **Locales**: en/es/qu actualizadas con `F11 rim` / `F11 rim-arma` / `F11 maki-k'anchay`.
- **Header bump**: `PHASE 3.27` → `PHASE 3.28`.
- **Costo.** Una sola llamada a `world_lights_boost_rgb_cam(0, 0, ·)` por frame (~3 × MAX_WORLD_LIGHTS=8 ops). Despreciable.

**Limitaciones conocidas de 3.28.**
- **Sin gating por oclusión.** Una pared sólida entre el jugador y una antorcha bloquea visualmente la luz pero no su rim sobre el arma — el boost ignora paredes (sólo el radio corta). En la práctica el radio chico (192) hace que la luz invisible esté siempre cerca del player anyway, y el rim ambiente "como si te llegara la luz" del torch del cuarto vecino tampoco es matemáticamente falso. Defer hasta que llimphi-ui gane custom pass wgpu.
- **Saturación a shade 1.0.** El rim es invisible en cuartos brillantes (todos los canales clampean a 1.0). Aceptable — narrativa: el ambiente sólo se nota cuando el arma de por sí está apagada.
- **Sin atenuación direccional.** El boost no depende de hacia dónde apunta el arma — un torch detrás del player tinta tanto como uno frente. Modelar dirección requeriría un fake "normal" del psprite y proyección — defer hasta tener feedback de si se nota.

**No incluido en 3.28 (defer a 3.29+):** sprite-BSP true occlusion vía R_CheckSight; smoothing del muzzle alpha por interp entre snapshots; volumetric god rays; decals dinámicos del impacto del disparo; tabla de tintes para Final Doom / Heretic-style PWADs; rim direccional (fake-normal en el psprite).

**Fase 3.29 (2026-05-30, este bloque):** oclusión sectorial de world lights — adiós leak de proyectiles a través de paredes.

- **Contexto.** 3.26→3.28 sumaron luces dinámicas desde mobjs `FF_FULLBRIGHT` (proyectiles, antorchas, explosiones) con tinte per-canal y rim sobre el arma, pero el aporte sólo se gating por radio: una BFG ball en el cuarto vecino con pared sólida pintaba verde la pared detrás del jugador. El muzzle ya tenía oclusión sectorial desde 3.23-3.25 (set `lit_sectors` por BFS desde el cuarto del player, hops ≤ `MUZZLE_BFS_MAX_HOPS=2`, radio cumulativo). Esta fase generaliza esa maquinaria para que **cada world light** cachee su propio set, computado desde el sector que aloja al mobj.
- **Refactor del BFS.** `compute_muzzle_lit_sectors` se descompone en `compute_lit_sectors_from(snap, src_x, src_y, src_sec, radius)` — la lógica de Dijkstra-lite con `dist`/`entry`/`hops`/`queue` queda intacta, pero el origen y el radio entran por parámetro. El muzzle wrapper resuelve player_sec vía BSP y delega; las world lights pasan `(mobj.x, mobj.y, mobj.sector, WORLD_LIGHT_RADIUS_WORLD=192)`. El sector del mobj ya viaja en el snapshot (`SpriteSnap.sector` desde 3.2), no hace falta point query adicional.
- **`WorldLight` deja de ser `Copy`** y gana `lit_sectors: Option<Arc<HashSet<u32>>>`. `Arc` para que las superficies del frame compartan el set sin clonar — una BFG ball contribuye a ≈300 superficies, copiar el HashSet por consulta sería desperdicio. `None` cuando la oclusión está apagada o el snapshot no tiene BSP (stub mode, mapa pre-carga); ese caso preserva el comportamiento 3.27 (la luz aporta sin gating).
- **`gather_world_lights`** gana `enable_occlusion: bool`. Cuando `true` y hay BSP, recorre las luces post-truncado al cap y rellena `lit_sectors` por cada una. Para revertir la transformación cam→world reusamos `cam.from_cam_2d` (de Fase 3.7) sin re-iterar `snap.sprites`. Costo: ≤ `MAX_WORLD_LIGHTS=8` BFS por frame, cada uno acotado a 2 hops y ≤16 sectores — despreciable comparado con el ≈330 surfaces × 8 luces × 3 canales del path RGB.
- **`world_lights_boost_rgb_cam` toma `surf_sector: u32`.** En el loop por luz, si `l.lit_sectors = Some(set)` y `set ∉ surf_sector`, la luz se descarta (no aporta). Si es `None`, pasa como antes — clean fallback al 3.27. `combined_boost_rgb_cam` propaga `surf_sector` (que ya tenía para el gating del muzzle) al helper de world lights. 4 sites en `gather_*` siguen iguales — ya pasaban `surf_sector` a `combined_boost_rgb_cam`.
- **Weapon rim sectorial.** En `render_frame`, antes de invocar `world_lights_boost_rgb_cam(0, 0, ..., world_lights_ref)` para el rim, resolvemos el sector del player vía `subsector_at_point`. Sin BSP cae a `NO_SECTOR`, que ninguna luz incluye en su lit set ⇒ ZERO_BOOST cuando la oclusión está on y BSP ausente (degraded gracefully); cuando la oclusión está off, `lit_sectors = None` para todas y el rim funciona como en 3.28.
- **`RenderConfig::world_lights_occlusion: bool` (default `true`).** Sin toggle host (F1-F11 agotadas, F12 cierra) — el flag se controla por configuración. Si querés A/B testing visual, el toggle F10 ya apaga todas las world lights (incluyendo su gating).
- **Resultado visible.** Disparar plasma o BFG en un pasillo: los frames del proyectil iluminan el cuarto donde están, pero **no** se cuelan por la pared al cuarto contiguo del player. Las antorchas decorativas (TBLU, TRED, etc.) tintan **sólo** su propio cuarto + vecinos directos por puerta — antes leaqueaban al cuarto lejano siguiendo el radio. Caminar por un pasillo con una sala roja del otro lado: el arma deja de tintarse roja sólo por proximidad euclidiana, ahora la pared y el techo del pasillo cortan el rim. Los corredores que conectan vía puerta two-sided sí siguen recibiendo aporte (BFS hop = 2 max, exactamente como el muzzle).
- **Compatibilidad 3.28.** Con `world_lights_occlusion = false` el path es bit-equivalente al 3.28 (lit_sectors = None ⇒ surf_sector ignorado en el helper). Los 8 tests scalar legacy + 12 tests RGB del 3.27 + 5 tests rim del 3.28 siguen verdes sin tocar.
- **Tests** (+5 render = 93 total verde):
  - `lit_sectors_from_arbitrary_source_includes_source_sector` — generalización del BFS: origen + vecino two-sided incluidos, sector aislado one-sided excluido.
  - `world_lights_boost_rgb_skips_light_when_surf_not_in_lit_sectors` — luz con set restringido + surf fuera ⇒ ZERO_BOOST; surf dentro ⇒ aporta.
  - `world_lights_boost_rgb_passes_light_when_lit_sectors_is_none` — `None` ⇒ surf_sector irrelevante (backward-compat 3.27).
  - `gather_world_lights_computes_lit_sectors_when_occlusion_enabled` — con BSP + occlusion=true, cada luz tiene `Some(set)` que contiene su sector origen.
  - `gather_world_lights_skips_occlusion_when_disabled_or_no_bsp` — cubre dos vertientes: oclusión off ⇒ None; oclusión on pero sin BSP ⇒ None (fallback automático).
- **Header bump**: `PHASE 3.28` → `PHASE 3.29`.

**Limitaciones conocidas de 3.29.**
- **BFS desde el sector del mobj, no desde su posición euclidiana exacta.** Un proyectil en vuelo justo al borde del sector entra al BFS como si estuviera "centrado" en ese sector; los midpoints de los bridge walls se miden desde la posición real del mobj (que el sprite snapshot reporta), no desde un centro virtual del sector. Buena aproximación práctica.
- **`MAX_WORLD_LIGHTS = 8` BFS por frame.** En cyberdemon spam + BFG cluster, las 8 más cercanas dominan el efecto — las descartadas no pagan BFS.
- **El set se recalcula cada frame** (no hay cache cross-tick). Los proyectiles cambian de sector frecuentemente; las antorchas decorativas tendrían cache hit alto pero el costo per-light ya es despreciable. Si el perfil llega a marcarlo, se cachea por `(sector, radius_q)` en el atlas.
- **Sin gating volumétrico** (suelo/techo). Una pared "horizontal" baja (techo descendido entre dos sectores) que tapa visualmente la luz no corta el aporte — sólo importa la conectividad two-sided. Defer si los artefactos se notan.

**No incluido en 3.29 (defer a 3.30+):** sprite-BSP true occlusion vía R_CheckSight (gating por línea de vista exacta, no sectorial); smoothing del muzzle alpha por interp entre snapshots; volumetric god rays; decals dinámicos del impacto del disparo; tabla de tintes para Final Doom / Heretic-style PWADs; rim direccional (fake-normal en el psprite); cache cross-tick del lit_sectors por (sector, radio).

**Fase 3.30 (2026-05-30, este bloque):** rim direccional del arma — antorchas frontales tintan más que las traseras.

- **Contexto.** 3.28 ligó el rim del arma al ambiente (`world_lights_boost_rgb_cam(0, 0, lights)` evaluado en el origen cam-space). 3.29 sumó la oclusión sectorial. Pero el aporte seguía siendo **omnidireccional**: una antorcha azul (TBLU) frente al jugador tintaba la pistola tanto como una idéntica detrás. El arma es un sprite 2D que conceptualmente "mira" hacia adelante; modelar una fake-normal en `+X_cam` permite atenuar las contribuciones traseras y subrayar la dirección de la luz fuerte.
- **Helper nuevo: `weapon_rim_boost_rgb_cam(player_sec, lights, directional)`.** Cuando `directional=false`, delega a `world_lights_boost_rgb_cam(0, 0, player_sec, lights)` — backward-compat bit-identical con 3.29. Cuando `directional=true`, por cada luz: además del falloff cuadrático por radio y del gate sectorial 3.29, aplica `att = max(WEAPON_RIM_AMBIENT_FLOOR=0.3, 0.5 + 0.5·cos(θ))` donde `cos(θ) = l.x_cam / |l|`. Lights al frente (cos=1) ⇒ att=1.0; laterales (cos=0) ⇒ att=0.5; traseras (cos=-1) ⇒ att=0.3 (piso ambient, modela el bounce indirecto de paredes/techo, sin cortar a 0 que sentiría artificial).
- **Caso degenerado.** Si una luz queda con `d≈0` (raro: el mobj FF_FULLBRIGHT está prácticamente en la posición del jugador), `cos(θ)` no está definido. Tratamos como `att=1.0` (full) y evitamos NaN — el caso "abrazado por la luz" merece intensidad plena de todos modos.
- **`RenderConfig::weapon_rim_directional: bool` (default `true`).** Sin toggle host (F-keys agotadas). El usuario que prefiera el rim omnidireccional cambia el flag por código.
- **Resultado visible.** Caminar por un pasillo con una antorcha azul (TBLU) al frente: la pistola se tinta azul. Pasar de largo y la antorcha queda atrás: el azul cae al 30 % (ambient floor) — la dirección del foco se siente físicamente. En un cuarto con dos antorchas — una al frente, una detrás — el frente domina. Un fireball pasando por el lado: tinte fugaz al 50 %.
- **Compatibilidad 3.29.** Con `weapon_rim_directional=false` el path es bit-equivalente a 3.29: la rama temprana llama al helper omni sin modificación. Los 5 tests rim del 3.28 + los 5 del 3.29 siguen verdes.
- **Tests** (+5 render = 98 total verde):
  - `weapon_rim_directional_full_intensity_in_front` — luz en +X_cam ⇒ direccional ≈ omni canal por canal.
  - `weapon_rim_directional_attenuates_lights_behind` — luz en -X_cam ⇒ ratio direccional/omni ≈ `WEAPON_RIM_AMBIENT_FLOOR` (0.3) por canal.
  - `weapon_rim_directional_side_lights_use_half` — luz en +Y_cam ⇒ ratio direccional/omni ≈ 0.5.
  - `weapon_rim_directional_disabled_equals_omni` — toggle off ⇒ bit-identical a `world_lights_boost_rgb_cam(0, 0, ..., lights)`.
  - `weapon_rim_directional_handles_zero_distance` — `d≈0` ⇒ finite + positivo (no NaN).
- **Header bump**: `PHASE 3.29` → `PHASE 3.30`.
- **Costo.** Una división extra (`d².sqrt().recip()`) + dos multiplicaciones + un max por luz, sólo en el path del rim — ≤ 8 luces por frame, despreciable. El resto de superficies de la escena conserva el path omnidireccional 3.27 (la directionality sólo tiene sentido sobre un objeto con normal definida).

**Limitaciones conocidas de 3.30.**
- **Fake-normal fija en +X_cam.** No considera el bob del arma (sway vertical) ni los frames de retroceso. Suficiente para el efecto perceptual: el bob es chico y la normal "promedio" sigue siendo el forward.
- **El piso ambient (0.3) es heurístico.** Si un cuarto es matemáticamente todo negro (ningún bounce real), una antorcha detrás aporta el 30 % "fantasma". Justificado: Doom no tiene radiosity, este piso emula que algo de luz indirecta llega siempre que el sector esté iluminado.
- **Sólo afecta al rim del arma.** Otros sprites (enemigos, decoración) siguen recibiendo el aporte omnidireccional del 3.27. Generalizar a mobjs requeriría darles una fake-normal — para sprites omnidireccionales (decoración) sería arbitrario; para enemigos con rotation 1..8 podríamos derivarla del frame angular, defer si se justifica visualmente.
- **Sin atenuación vertical.** Una antorcha en el techo y otra en el piso aportan igual (sólo importa el plano XY del cam). Modelar pitch real requiere un Vec3 normal completo — defer.

**No incluido en 3.30 (defer a 3.31+):** sprite-BSP true occlusion vía R_CheckSight; smoothing del muzzle alpha por interp entre snapshots; volumetric god rays; decals dinámicos del impacto del disparo; tabla de tintes para Final Doom / Heretic-style PWADs; cache cross-tick del lit_sectors por (sector, radio); fake-normal vertical (Vec3 con pitch); rim direccional para enemigos con rotation angular.

**Fase 3.31 (2026-05-30, este bloque):** rim direccional para mobj sprites — back-lighting físicamente correcto.

- **Contexto.** 3.30 cosineó el aporte de las world lights sobre el psprite del arma (fake-normal en `+X_cam`). Pero el resto de los sprites — enemigos, decoración, proyectiles — seguía recibiendo el aporte omnidireccional del 3.27: una antorcha azul detrás de un imp lo tintaba lo mismo que una al frente, aunque físicamente la cara visible del imp (el sprite que ve el jugador) **no** la ilumina la luz trasera. Esta fase generaliza la maquinaria del rim direccional al `gather_sprite`, con una fake-normal específica de billboards: apunta del sprite hacia la cámara (`(-x_surf, -y_surf)/|surf|`).
- **Helpers nuevos.**
  - `world_lights_boost_rgb_for_sprite_cam(x_surf, y_surf, surf_sector, lights, directional)`: mismo esquema que `world_lights_boost_rgb_cam` pero con atenuación cosine cuando `directional=true`. La normal toward-camera se calcula al inicio; cada luz contribuye `f²·PEAK·tint·att` con `att = max(SPRITE_RIM_AMBIENT_FLOOR=0.3, 0.5 + 0.5·cos(θ))`. cos(θ) = dot(normal, dir_sprite→luz).
  - `combined_boost_rgb_sprite_cam(...)`: equivalente a `combined_boost_rgb_cam` pero usando el helper anterior para las world lights. El muzzle sigue siendo omni-from-player (es luz que emana del arma del jugador, no del sprite — direccionarla no tiene sentido geométrico).
- **Casos degenerados manejados.** (a) Sprite exactamente en la cámara (`|surf|≈0`): cae al path omni dentro del helper direccional — billboard sin normal definida. (b) Luz coincidente con el sprite (`d≈0`): `att=1.0` — la luz "abraza" al sprite. Ambos evitan NaN.
- **Plumbing.** En `gather_sprite` los dos sites (patch texturizado + fallback de rectángulos coloreados) cambian la llamada `combined_boost_rgb_cam(...)` por `combined_boost_rgb_sprite_cam(..., cfg.sprite_rim_directional)`. Walls, pisos y techos siguen llamando `combined_boost_rgb_cam` — la direccionalidad sólo tiene sentido sobre objetos con normal definida, y un piso/techo no es candidato natural (la normal sería vertical, y todas las world lights están en el plano horizontal aproximadamente).
- **`RenderConfig::sprite_rim_directional: bool` (default `true`).** Sin toggle host (F-keys agotadas). Cambiar el flag por código revierte al path omni 3.27/3.29.
- **Resultado visible.** Un imp parado al frente, con una antorcha **detrás** de él: la cara visible del imp queda al ambient floor (30 %), no full-tint como antes. Si moves al jugador para que la antorcha quede del lado del jugador (entre cámara e imp): el imp recibe full tint. Un fireball de imp pasando por delante de un barril tinta su frente rojizo fuerte; el barril del fondo no se tintea por estar detrás de la línea sprite→cámara. Los sprites laterales (a +90°) reciben 50 % del aporte — efecto medio realista. Caminar por un pasillo con una antorcha azul al fondo (TBLU): los enemigos que están frente al jugador y entre él y la antorcha **no** se ven azulados (la antorcha los back-lightea); los enemigos del fondo (más allá de la antorcha desde la cámara) sí se tintan azul.
- **Compatibilidad 3.30.** `sprite_rim_directional=false` ⇒ ambos sites de `gather_sprite` caen al path omni bit-identical 3.27/3.29. Los 88 tests anteriores siguen verdes; 5 nuevos cubren la direccionalidad.
- **Tests** (+5 render = 103 total verde):
  - `sprite_rim_directional_front_light_matches_omni` — luz entre cámara y sprite en eje X ⇒ direccional ≈ omni.
  - `sprite_rim_directional_back_light_falls_to_floor` — luz detrás del sprite ⇒ ratio direccional/omni ≈ `SPRITE_RIM_AMBIENT_FLOOR` (0.3).
  - `sprite_rim_directional_side_light_uses_half` — luz al costado del sprite ⇒ ratio 0.5.
  - `sprite_rim_directional_disabled_equals_omni_for_arbitrary_lights` — toggle off ⇒ bit-identical al path 3.29 con 3 luces mezcladas.
  - `sprite_rim_directional_degenerates_safely_at_camera` — sprite en la cámara ⇒ fallback omni, sin NaN.
- **Header bump**: `PHASE 3.30` → `PHASE 3.31`.
- **Costo.** Por sprite: 1 sqrt para normalizar la normal + 1 sqrt para normalizar dir-to-light per luz + 1 producto interno + 1 max. ≤ 8 luces × ~30 sprites visibles = ~240 ops/frame extras. Despreciable.

**Limitaciones conocidas de 3.31.**
- **Fake-normal sin rotación del enemigo.** El imp tiene un `mobj.angle` (1..8 facing) pero nuestra normal apunta toward-camera independiente del facing. Es coherente con cómo el motor renderea sprites (siempre billboard hacia la cámara). Modelar la normal real (la dirección a la que mira el enemigo) cambiaría qué luces lo iluminan, pero ese efecto sólo tendría sentido si los sprites tuvieran caras visibles distintas según el ángulo de observación — Doom no las tiene.
- **Mismo piso ambient para todos los mobjs.** Un proyectil chico (PUFF) y un cyberdemon comparten el ambient floor. Si en el futuro sumamos un "tamaño relativo" al sprite podría escalar, pero a 0.3 el valor ya es perceptualmente discreto.
- **Sin atenuación vertical** (igual que el rim del arma en 3.30). Una luz en el techo y otra en el suelo aportan igual. Defer a una fase de fake-normal Vec3 con pitch.
- **Walls/pisos/techos siguen omni.** Si en el futuro damos a las paredes una fake-normal (perpendicular al lineseg), podemos extender — pero el aporte sobre superficies con orientación variable es lo que normalmente hace un sistema de iluminación real (BRDF), y ahí queda corto el approach scalar. Defer hasta tener custom wgpu pass.

**No incluido en 3.31 (defer a 3.32+):** sprite-BSP true occlusion vía R_CheckSight; smoothing del muzzle alpha por interp entre snapshots; volumetric god rays; decals dinámicos del impacto del disparo; tabla de tintes para Final Doom / Heretic-style PWADs; cache cross-tick del lit_sectors por (sector, radio); fake-normal vertical Vec3 con pitch; normal real de paredes para BRDF aproximado.

**Fase 3.32 (2026-05-30, este bloque):** rim direccional para paredes — completa la trilogía 3.30→3.31→3.32.

- **Contexto.** Una pared, a diferencia de un billboard, tiene una normal **bien definida**: perpendicular a la dirección del lineseg, orientada al lado del frente. Modelarla permite aplicar el mismo cosine BRDF que el arma (3.30) y los mobjs (3.31). Una antorcha perpendicular al plano de la pared la tinta al 100 %; una rasante (incidencia oblicua, paralela al lineseg) al 50 %; una "detrás" del plano (puede ocurrir cuando una luz two-sided alcanza la cara opuesta por BFS) cae al piso ambient.
- **Helpers nuevos.**
  - `wall_normal_cam(x1, y1, x2, y2, mid_x, mid_y)`: dado el lineseg en cam-space + su midpoint, devuelve la perpendicular al lineseg orientada toward camera (origen del cam-space). De las dos perpendiculares posibles `±(-dy, dx)/|d|`, pickea la que cumple `dot(n, mid) < 0` (mid apunta del origen al midpoint, la normal toward-camera apunta en sentido inverso). Pared degenerada (endpoints coincidentes) ⇒ `(0, 0)`: el caller debería caer al path omni — el cosine sería 0 multiplicado por 0.
  - `world_lights_boost_rgb_for_wall_cam(x, y, sec, lights, normal, directional)`: mismo esquema que el helper sprite-direccional pero recibe la normal precomputada por el caller. Cada luz contribuye `f²·PEAK·tint·att` con `att = max(WALL_RIM_AMBIENT_FLOOR=0.3, 0.5 + 0.5·cos(θ))`. Caso `d²<1e-6` ⇒ att=1.0 (sin NaN).
  - `combined_boost_rgb_wall_cam(...)`: muzzle (omni, anclado al jugador) + world lights atenuadas por la normal de la pared. **Muzzle queda omni** — coherente con 3.30/3.31 (la decisión de no direccionar el muzzle queda consistente entre los tres rims). El muzzle emana del jugador; aplicar BRDF sobre paredes oblicuas ya quedaba implícito en el modelo de Doom clásico como atenuación por distancia.
- **Plumbing.** En `gather_wall`, una vez calculados `mid_x`/`mid_y` y los endpoints cam-space, calculamos `wall_normal = wall_normal_cam(x1, y1, x2, y2, mid_x, mid_y)` y reemplazamos `combined_boost_rgb_cam(...)` por `combined_boost_rgb_wall_cam(..., wall_normal, cfg.wall_rim_directional)`. Los otros sites de pared (subsector planes, sprites) no cambian — pisos/techos siguen omni (su "normal" sería vertical, las world lights están en el plano XY, el cosine sería trivial casi siempre y el modelo del 3.7 ya cubre la atenuación por distancia).
- **`RenderConfig::wall_rim_directional: bool` (default `true`).** Sin toggle host (F-keys agotadas). Cambiar el flag por código revierte al path omni 3.27/3.29.
- **Resultado visible.** Caminar por un pasillo con una antorcha azul en una pared lateral: el muro **del** lado de la antorcha se tinta fuerte; el muro **opuesto** (que la "vería" rasante) recibe el 50 %; las paredes del fondo (cara perpendicular al rayo) reciben full atenuación según distancia más cosine cercano a 1. Antes (3.31) las cuatro paredes recibían el mismo aporte por distancia — quedaba uniforme y plástico. Las esquinas resaltan: una pared en ángulo entre dos antorchas se tinta más por la que tiene incidencia perpendicular.
- **Compatibilidad 3.31.** `wall_rim_directional=false` ⇒ bit-identical al path omni 3.29. Los 88 tests previos del rim del arma (3.30) y los 5 nuevos del sprite (3.31) siguen verdes.
- **Tests** (+6 render = 109 total verde):
  - `wall_normal_cam_orients_toward_camera` — pared a `x=100` con dirección `+Y` ⇒ normal `(-1, 0)`.
  - `wall_normal_cam_degenerate_zero_length` — endpoints coincidentes ⇒ `(0, 0)`.
  - `wall_rim_directional_perpendicular_light_full_intensity` — luz frente al plano ⇒ direccional ≈ omni.
  - `wall_rim_directional_grazing_uses_half` — luz paralela al lineseg ⇒ ratio 0.5.
  - `wall_rim_directional_back_light_falls_to_floor` — luz detrás del plano ⇒ ratio `WALL_RIM_AMBIENT_FLOOR`.
  - `wall_rim_directional_disabled_equals_omni` — toggle off ⇒ bit-identical al path 3.29.
- **Header bump**: `PHASE 3.31` → `PHASE 3.32`.
- **Costo.** Por pared: 1 sqrt para normalizar la normal + 1 sqrt + producto interno por luz. ~50 paredes visibles × 8 luces = ~400 ops/frame extras. Despreciable.

**Limitaciones conocidas de 3.32.**
- **Normal puramente horizontal.** No modela el techo bajo ni el piso elevado — las paredes son verticales en Doom igual, pero una luz arriba/abajo se trata igual que una en eje XY. Defer a una fase de Vec3 con pitch.
- **Sin cache de normales.** El cálculo se hace por frame; las paredes se mueven sólo en el sentido cam-space, no en world-space. Si el perfil llega a importar podríamos cachear las normales mundo-space y rotarlas al cam-space (1 trig por frame en lugar de 2 por pared).
- **El muzzle queda omni** (decisión consistente con 3.30/3.31). En walls oblicuos visibles desde un ángulo agudo, el muzzle físicamente "rasante" debería atenuar — para fidelidad estricta habría que aplicar también el cosine al muzzle. Pero la lectura "fogonazo barre todo el cono delante del jugador" se preserva mejor sin esa atenuación. Toggle separado en una fase futura si se necesita.
- **Pisos y techos siguen omni.** Su "normal" sería ±Z, ortogonal al plano XY donde viven las world lights — el cosine sería casi siempre 0 y atenuaría todo al piso ambient. Modelar correctamente requiere la posición Z de cada luz (que hoy no exportamos al WorldLight) y un Vec3 cosine completo. Defer.

**No incluido en 3.32 (defer a 3.33+):** sprite-BSP true occlusion vía R_CheckSight; smoothing del muzzle alpha por interp entre snapshots; volumetric god rays; decals dinámicos del impacto del disparo; tabla de tintes para Final Doom / Heretic-style PWADs; cache cross-tick del lit_sectors por (sector, radio); fake-normal vertical Vec3 con pitch; cosine BRDF para pisos/techos con z exportado al WorldLight; muzzle direccional sobre walls oblicuos.

**Fase 3.33 (2026-05-30, este bloque):** BRDF 3D para pisos y techos — completa el cuarteto direccional.

- **Contexto.** 3.30 (weapon), 3.31 (mobjs), 3.32 (walls) direccionaron todas las superficies con normal en el plano horizontal. Faltaba el caso vertical: pisos (normal `+Z`) y techos (normal `-Z`). Esta fase exporta el `z` del sprite FF_FULLBRIGHT al `WorldLight.z_cam` (relativo a `cam.view_z`) y modela el cosine completo en 3D para los planos horizontales — además promueve el falloff por radio de 2D a **3D real**, lo que también cambia el aporte: un proyectil BFG flotando a 100 u arriba del piso pero al lado del jugador aporta menos al piso de lo que daba el 2D-only del 3.27.
- **`WorldLight` gana `z_cam: f32`** — `sprite.z - cam.view_z` calculado en `gather_world_lights`. El filtro 2D inicial (×4 del radio) sigue como fast reject, pero el chequeo exacto dentro del helper de plano usa d² 3D.
- **Helpers nuevos.**
  - `world_lights_boost_rgb_for_plane_cam(x, y, z_surf_cam, sec, lights, n_z, directional)`: por cada luz, `(dx, dy, dz) = light - surf`, `d² = dx² + dy² + dz²`, falloff `f = 1 - d²/r²` con `r = WORLD_LIGHT_RADIUS_WORLD`. Si `directional`, `cos(θ) = n_z · dz / d_3D`, `att = max(PLANE_RIM_AMBIENT_FLOOR=0.3, 0.5 + 0.5·cos)`. Sin direccional, cae al path omni 2D del 3.29 — backwards-compat. `n_z` = `+1.0` para floor, `-1.0` para ceiling.
  - `combined_boost_rgb_plane_cam(...)`: muzzle (omni 2D, anclado al jugador, sin direccionar — consistente con 3.30-3.32) + world lights con BRDF 3D.
- **Plumbing en `gather_subsector_planes`.** Antes había una única llamada a `combined_boost_rgb_cam` con el centroide y se reutilizaba ese `boost_rgb` para floor + ceiling. Ahora el boost se calcula **dentro** de `emit_plane`, donde ya tenemos `is_floor: bool` para elegir `n_z`. Cero duplicación de cómputo — emit_plane corre una vez por floor y una por ceiling, ambas con el boost que les corresponde.
- **`RenderConfig::plane_rim_directional: bool` (default `true`).** `false` ⇒ ambos planos vuelven al path omni 2D 3.29 — bit-equivalente al 3.32 para snapshot dado.
- **`combined_boost_rgb_cam` queda `#[cfg(test)]`.** Ningún caller del render loop la usa: walls, sprites, planes y weapon ya tienen sus variantes especializadas. Se conserva como referencia para los tests existentes del 3.26-3.27 que verifican el clamping per-canal sobre el path omni.
- **Resultado visible.** Un proyectil de imp (BAL1) volando a 30 u del piso ilumina el piso fuerte cerca + apaga su aporte al techo lejano. Una bola BFG a media altura (≈ 64 u) ilumina ambos planos balanceadamente. Una antorcha alta (TLMP a z=80) ilumina más el techo que el piso. Antes (3.32) los dos planos recibían el mismo aporte por luz; ahora la separación vertical pega. Caminar bajo un techo bajo (z_ceiling = 64) con un fireball pasando a 32 u: el techo se tinta fuerte; el piso recibe la mitad porque la incidencia es rasante. El radio 3D corta luces que el 2D-only dejaba pasar: una antorcha al fondo del pasillo + alta en una vista de pasillo + bajada al ojo ya no contribuye si su distancia 3D excede 192.
- **Compatibilidad 3.32.** Con `plane_rim_directional = false` el path es bit-equivalente al 3.32 (cae al `world_lights_boost_rgb_cam` 2D). Los 109 tests previos siguen verdes. Los 11 test fixtures con `WorldLight { ... }` se actualizaron con `z_cam: 0.0` (válido para el 3.27/3.29 path que ignora z).
- **Tests** (+5 render = 114 total verde):
  - `plane_rim_directional_floor_strongest_when_light_above` — dos luces a igual `d_3D=50` pero distinto cos ⇒ la de cima (cos=0.8) aporta más que la lateral (cos=0).
  - `plane_rim_directional_ceiling_strongest_when_light_below` — espejo del anterior con `n_z=-1` y luz abajo del ceiling.
  - `plane_rim_directional_3d_radius_cuts_far_vertical` — luz a `z=250` con XY=0 ⇒ direccional 3D la corta (`d_3D=250 > r=192`); omni 2D la conserva (`d_2D=0`).
  - `plane_rim_directional_disabled_equals_omni_2d` — toggle off ⇒ bit-equivalente al path omni 2D 3.29.
  - `plane_rim_directional_floor_back_lit_from_below_falls_to_floor` — luz debajo del floor ⇒ cos negativo ⇒ att clampea al piso ambient `PLANE_RIM_AMBIENT_FLOOR=0.3`.
- **Header bump**: `PHASE 3.32` → `PHASE 3.33`.
- **Costo.** Por plano: 1 sqrt para `d_3D` por luz (3 multiplicaciones extras vs el cómputo 2D del 3.27). ~50 subsectores visibles × 2 planos × 8 luces = ~800 ops/frame extras. Despreciable. La nueva `z_cam` en cada `WorldLight` añade 4 bytes/luz × `MAX_WORLD_LIGHTS=8` = 32 bytes/frame, también despreciable.

**Limitaciones conocidas de 3.33.**
- **Cosine sólo en Z para planos.** Pisos y techos no consideran la dirección XY al calcular el `att` — sólo el componente Z. Apropiado para planos horizontales (su normal es vertical pura), pero si en el futuro hay planos inclinados (slopes — pendientes que Doom clásico no soporta pero algunos ports sí) habría que generalizar.
- **El muzzle no aplica BRDF en planos.** Consistente con 3.30-3.32. Visualmente el muzzle "barre" el cono delante del jugador igual sobre piso, techo y paredes — lectura intencional.
- **Z relativa al view_z**, no al sector. Si un sector se mueve (door subiendo) el z absoluto del sprite cambia pero el z relativo al ojo del jugador no — el cosine sigue siendo correcto frame por frame.
- **No considera ocluder verticales.** Una pared interna que tape visualmente la luz no la corta (el `lit_sectors` BFS del 3.29 sí, pero a nivel sector — la pared dentro del mismo sector queda).

**No incluido en 3.33 (defer a 3.34+):** sprite-BSP true occlusion vía R_CheckSight; smoothing del muzzle alpha por interp entre snapshots; volumetric god rays; decals dinámicos del impacto del disparo; tabla de tintes para Final Doom / Heretic-style PWADs; cache cross-tick del lit_sectors por (sector, radio); BRDF 3D también en walls (extender wall_normal a Vec3 con componente vertical = 0); muzzle direccional sobre walls oblicuos; planos inclinados (slopes).

**Fase 3.34 (2026-05-30, este bloque):** BRDF 3D para paredes — radio + cosine en 3D real.

- **Contexto.** 3.32 le dio a las paredes cosine direccional con normal 2D (XY pura). Ahora con `z_cam` en `WorldLight` (3.33) podemos hacer el wall path **completamente 3D**: distancia por d² 3D y cosine por `(nx·dx + ny·dy) / d_3D` (la wall normal tiene `nz=0`, las paredes son verticales). Una antorcha alta a la misma XY que la pared queda con `cos < cos_2D` porque `d_3D > d_XY`; el radio 3D excluye luces remotas en vertical aunque su XY caiga adentro.
- **`world_lights_boost_rgb_for_wall_cam` toma `z_surf_cam: f32`.** Punto de muestreo vertical de la pared, típicamente `0.0` (eye level) — natural single-sample point. Se calcula `dz = l.z_cam - z_surf_cam`, `d² = dx²+dy²+dz²`, y `cos = (nx·dx + ny·dy) / d_3D` (la `dz` no aparece en el cos porque `wall_normal_z = 0`, pero sí escala el denominador).
- **`combined_boost_rgb_wall_cam` también toma `z_surf_cam`** y lo reenvía. `gather_wall` pasa `0.0` para el sample point — el midpoint XY de la pared en cam-space ya se calculaba antes (`mid_x, mid_y`); el `z_surf_cam=0` representa "eye level del jugador" como referencia de altura.
- **Compatibilidad 3.32.** Para todas las luces con `z_cam = 0`, el path 3D colapsa al cálculo 2D (`dz=0` ⇒ `d_3D = d_2D` ⇒ `cos_3D = cos_2D`). Los 6 tests previos del 3.32 actualizaron su firma vía perl + se mantienen verde porque las luces de prueba siempre tienen `z_cam = 0` (válido). El path `wall_rim_directional=false` sigue cayendo al omni 2D del 3.29 — bit-equivalente.
- **Resultado visible.** Caminar por un cuarto con una antorcha alta (TLMP a `z=80`, lamp post típica de Doom 2) y observar las paredes desde el suelo: la luz aporta menos de lo que daba 3.32, pero proporcional a la incidencia real — la pared "ve" la luz con cierto ángulo descendente. Un fireball (BAL1) volando a 100 u por encima del eye level deja de pintar verde a la pared lejana (3D > radio); a la pared cercana le pega con cos rasante.
- **Tests** (+5 render = 119 total verde):
  - `wall_rim_3d_high_light_attenuates_compared_to_planar` — dos luces a misma XY pero distinta z ⇒ la alta aporta menos por canal.
  - `wall_rim_3d_radius_cuts_far_vertical_light` — luz a `z=250` con `d_XY=0` ⇒ direccional 3D la excluye; omni 2D no.
  - `wall_rim_3d_planar_light_finite_and_positive` — luz con `z=0` ⇒ valores positivos y finitos (smoke).
  - `wall_rim_3d_disabled_uses_omni_2d` — toggle off ⇒ bit-equivalente al omni 2D 3.29 aún con z_cam alto.
  - `wall_rim_3d_handles_zero_distance_safely` — luz coincidente con superficie ⇒ no NaN.
- **Header bump**: `PHASE 3.33` → `PHASE 3.34`.
- **Costo.** 1 multiplicación + 1 suma adicional por luz (componente z). Despreciable.

**Limitaciones conocidas de 3.34.**
- **Eye-level sampling.** El punto de muestreo vertical de la pared es `z=0` (eye level del jugador). Para paredes muy altas o muy bajas, el cosine y el radio se evalúan sólo en ese plano horizontal. Subdividir la pared verticalmente (similar a `wall_strips` para perspective approx) daría un BRDF más fiel, defer.
- **Walls oblicuos verticalmente.** Como en 3.32, el muzzle queda omni — un muro inclinado en cam-space que se ve casi paralelo al jugador no sufre dimming extra del muzzle. Decisión consistente.
- **Sprites siguen con cosine 2D.** El path direccional de sprites (3.31) no usa `z_cam` para el cosine — los billboards siempre miran a la cámara y la fake-normal es perpendicular al eje cam-to-sprite en XY. Extender a 3D ahí cambiaría el modelo conceptual del billboard (no son objetos físicos con extensión Z explícita). Defer si se justifica.

**No incluido en 3.34 (defer a 3.35+):** sprite-BSP true occlusion vía R_CheckSight; smoothing del muzzle alpha por interp entre snapshots; volumetric god rays; decals dinámicos del impacto del disparo; tabla de tintes para Final Doom / Heretic-style PWADs; cache cross-tick del lit_sectors por (sector, radio); BRDF 3D también en sprites; subdivisión vertical de walls para BRDF per-strip; muzzle direccional sobre walls; planos inclinados (slopes).

**Fase 3.35 (2026-05-30, este bloque):** BRDF 3D para mobj sprites — cierra el ciclo direccional 3D completo.

- **Contexto.** 3.32→3.34 movió walls, planes y de vuelta walls al BRDF 3D. Faltaban sprites — el path direccional de 3.31 seguía siendo 2D pure (`d² = dx² + dy²`, `cos = (nx·dx + ny·dy)/d_2D`). Con `z_cam` en `WorldLight` desde 3.33, sprites pueden adoptar el mismo modelo que walls: normal 2D toward-camera (consistente con el billboard que siempre encara al jugador), pero distancia y normalización del cosine en 3D. Resultado: un mobj queda menos iluminado por una luz que está alta arriba de él y más por una al mismo nivel.
- **`world_lights_boost_rgb_for_sprite_cam` toma `z_surf_cam: f32`.** Sample point vertical del sprite — `gather_sprite` lo computa como `sprite.z - cam.view_z` (mobj.z floor del sprite, relativo al ojo del jugador). La normal sigue siendo 2D `(-x_surf, -y_surf)/|surf|` (la billboard mira al jugador en XY, sin tilt vertical). `d² = dx² + dy² + dz²`, `cos = (nx·dx + ny·dy)/d_3D` — la `dz` no aparece en el numerador del cos (la normal no tiene componente Z), pero sí en el denominador.
- **`combined_boost_rgb_sprite_cam` también toma `z_surf_cam`** y lo reenvía. Ambos sites de `gather_sprite` (patch texturizado + fallback) pasan `z_surf_cam = sprite.z - cam.view_z`.
- **Sample point.** Elegimos el floor del sprite (mobj.z, donde el sprite "se apoya") en lugar del centro vertical. Razón: simple y consistente con la convención Doom de anclaje al piso; la diferencia con el centro (~24 u para un imp) es pequeña respecto al radio 192. Cuando la altura del patch decodificado realmente importe, podemos sumar `patch.height * 0.5` en una fase futura.
- **Compatibilidad 3.31.** Todas las luces con `z_cam=0` y sample con `z_surf_cam=0` colapsan al cálculo 2D del 3.31. Los 5 tests previos de sprite directional siguen verde porque `rim_light` helper usa z_cam=0 y los tests llaman con z_surf_cam=0 (actualizados via perl one-liner). El path `sprite_rim_directional=false` cae al omni 2D 3.29 — bit-equivalente.
- **Resultado visible.** Caminar bajo un techo con una lámpara alta (TLMP en z=80) que hace stand-up frente a un imp (z=0): el imp recibe menos tinte que en 3.31 porque la luz queda con cosine rasante (`d_3D > d_XY`). Un fireball volando muy alto (BAL1 a z=120 over el player's head) deja de tintar a los enemigos del suelo: 3D distance los excluye del radio. Un imp con un mismo fireball pasando al ras de su altura (z=24): tinte rojo fuerte. La separación vertical pega — antes (3.31) un fireball flotante daba el mismo tinte que uno al nivel del mobj.
- **Tests** (+5 render = 124 total verde):
  - `sprite_rim_3d_high_light_attenuates_compared_to_planar` — dos luces a misma XY pero distinta z ⇒ la alta atenúa más.
  - `sprite_rim_3d_radius_cuts_far_vertical_light` — luz `z=250` con `d_XY=0` ⇒ direccional 3D la excluye; omni 2D no.
  - `sprite_rim_3d_planar_light_finite_and_positive` — sanity con luz a `z=0` (colapso al 2D).
  - `sprite_rim_3d_disabled_uses_omni_2d` — toggle off ⇒ bit-equivalente al 3.29 aún con z_cam alto.
  - `sprite_rim_3d_handles_sprite_below_eye_level` — sprite en piso debajo del ojo (z_surf=−32) + luz al ras del piso ⇒ finito, positivo (sanity del path con z_surf < 0).
- **Header bump**: `PHASE 3.34` → `PHASE 3.35`.
- **Costo.** Igual que walls 3.34: 1 multiplicación + 1 suma por luz (componente z). Despreciable.

**Cierre del ciclo direccional 3D.**

Con 3.35 cerrado, **todas las superficies del renderer con normal definida** pasan por BRDF 3D unificado:
| Superficie | Fase | Normal | d/cos |
|---|---|---|---|
| Arma (psprite) | 3.30 | `+X_cam` fija | 2D (sólo XY) |
| Mobj sprite (billboard) | 3.31→3.35 | `(-x, -y)/|s|` toward-cam | **3D** |
| Wall | 3.32→3.34 | perpendicular al lineseg toward-cam | **3D** |
| Floor | 3.33 | `+Z` | **3D** |
| Ceiling | 3.33 | `-Z` | **3D** |

El arma sigue siendo 2D (el psprite no tiene posición Z real — es overlay 2D sobre el viewport, evaluado en el origen del cam-space). El resto es BRDF 3D coherente.

**Limitaciones conocidas de 3.35.**
- **Sample point en el floor del sprite.** Para mobjs altos (cyberdemon ~110 u), el sample subestima el cos para luces a media altura. Un offset `+ patch.height·0.5` sería más fiel — defer hasta que se necesite distinguir.
- **Sprite normal sigue 2D.** Si en el futuro queremos modelar la inclinación del billboard cuando el jugador mira hacia arriba/abajo (pitch), habría que extender la normal a Vec3. Hoy el billboard se proyecta plano hacia la cámara independientemente del pitch — coherente con Doom clásico.
- **Sprites no usan los lit_sectors específicos del sprite.** El gating usa `sprite.sector`, igual que el resto. Si un mobj está en un sector y una luz en otro pero conectados, los dos quedan iluminados por el BFS 3.29 — esperado.

**No incluido en 3.35 (defer a 3.36+):** sprite-BSP true occlusion vía R_CheckSight (gating por línea de vista exacta); smoothing del muzzle alpha por interp entre snapshots; volumetric god rays; decals dinámicos del impacto del disparo; tabla de tintes para Final Doom / Heretic-style PWADs; cache cross-tick del lit_sectors por (sector, radio); subdivisión vertical de walls para BRDF per-strip; muzzle direccional sobre walls; planos inclinados (slopes); sprite sample point en el centro del patch (offset `+height·0.5`).

**Fase 3.36 (2026-05-30, este bloque):** tabla de tintes para Doom 2 + pickups + keys — cierra el gap del shareware.

- **Contexto.** 3.27 trajo `FB_SPRITE_TINTS` con 24 entradas cubriendo proyectiles, fogs y decoración del shareware Doom 1. Pero muchos mobjs FF_FULLBRIGHT de Doom 2 quedaban con el fallback amarillo cálido (`MUZZLE_TINT_RGB`): mancubus fireball, revenant tracer, archvile flame, lost soul, keys coloreadas, soul sphere, mega armor. Esta fase amplía la tabla a **38 entradas** con tintes específicos para cada uno.
- **Nuevas entradas (14).**
  - Proyectiles Doom 2: `MANF` mancubus naranja `(255, 160, 90)`, `FATB` revenant tracer pálido `(255, 220, 160)`, `SKEL` revenant attack `(255, 200, 150)`.
  - Archvile: `VILE` attack flames rojo `(255, 130, 70)`, `FIRE` fire pillar saturado `(255, 100, 50)`.
  - Mobjs Doom 1 que faltaban: `SKUL` lost soul blue-white `(180, 220, 255)`.
  - Pickups brillantes: `SOUL` soul sphere cyan `(130, 200, 255)`, `MEGA` mega armor verde-cyan `(130, 220, 200)`.
  - Keycards: `BKEY` `(110, 160, 255)`, `YKEY` `(255, 240, 130)`, `RKEY` `(255, 130, 90)`.
  - Skullkeys: `BSKU`, `YSKU`, `RSKU` — mismos tintes que las cards equivalentes (el HUD las muestra del mismo color).
- **Cero cambio de mecánica.** Sigue la misma función `sprite_tint_for_name(name)` con match case-insensitive sobre los primeros 4 chars. El loop lineal sobre la tabla pasa de 24 a 38 iteraciones en el peor caso — ~30 luces visibles/frame × 38 entries × 7 ns/comparación ≈ 8 µs/frame, despreciable.
- **Backwards-compat.** Las 24 entradas previas se mantienen idénticas en tinte. Los tests 3.27 siguen verdes sin modificación. Los mobjs no listados siguen cayendo al fallback amarillo.
- **Resultado visible.** En Doom 2 maps con mancubus + revenant + archvile: cada proyectil emite su tinte característico (naranja, cálido pálido, rojo flame). Un cuarto con un lost soul flotando ahora se tinta blue-white local. Recoger una blue keycard ilumina su entorno azul brevemente (FF_FULLBRIGHT mientras está en el mapa). Soul sphere flotando bajo un techo lo tinta cyan. PWADs vanilla-compatibles que usen estos sprites también se ven correctamente sin tabla custom adicional.
- **Tests** (+5 render = 129 total verde):
  - `sprite_tint_for_name_resolves_doom2_projectiles` — MANF, FATB, SKEL ⇒ tintes específicos distintos del fallback.
  - `sprite_tint_for_name_resolves_archvile_flame` — VILE, FIRE ⇒ rojo flame; FIRE más saturado.
  - `sprite_tint_for_name_resolves_lost_soul_and_pickups` — SKUL, SOUL, MEGA ⇒ azul/cyan; cada uno B > R.
  - `sprite_tint_for_name_resolves_colored_keys` — BKEY/YKEY/RKEY + BSKU/YSKU/RSKU ⇒ colores correctos; card y skull del mismo color matchean.
  - `sprite_tint_for_name_doom2_lookups_case_insensitive` — mixed-case + sufijos (MANFA1, SKULA0) resuelven igual.
- **Header bump**: `PHASE 3.35` → `PHASE 3.36`.

**Limitaciones conocidas de 3.36.**
- **Final Doom no agrega sprites FF_FULLBRIGHT propios.** TNT/Plutonia reusan los assets de Doom 2; ya están cubiertos.
- **Heretic / Hexen sprites quedan al fallback.** Sus naming convention difiere de Doom (e.g. blue elf `ELFB`) y vanilla doomgeneric no los entiende natívamente. Quedaría como sub-tabla `HERETIC_SPRITE_TINTS` si se integra Heretic en una fase futura.
- **Sin entradas para variantes obscuras de PWADs custom.** Los Boom/MBF extensions traen sprites únicos (`POL5`, custom mod assets) — quedan al fallback.

**No incluido en 3.36 (defer a 3.37+):** sprite-BSP true occlusion vía R_CheckSight; smoothing del muzzle alpha por interp entre snapshots; volumetric god rays; decals dinámicos del impacto del disparo; cache cross-tick del lit_sectors por (sector, radio); subdivisión vertical de walls para BRDF per-strip; muzzle direccional sobre walls; planos inclinados (slopes); sprite sample point en el centro del patch; tabla de tintes Heretic / Hexen.

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
- **2026-05-30 (+11):** Fase 3.36 — tabla de tintes para Doom 2 + pickups + keys. `FB_SPRITE_TINTS` pasa de 24 a 38 entradas: MANF (mancubus fireball), FATB (revenant tracer), SKEL (revenant attack), VILE (archvile flame), FIRE (archvile fire pillar), SKUL (lost soul blue-white), SOUL (soul sphere), MEGA (mega armor), BKEY/YKEY/RKEY (keycards) y BSKU/YSKU/RSKU (skullkeys). Cero cambio de mecánica — sigue la misma `sprite_tint_for_name(name)` con loop lineal case-insensitive. Backwards-compat: las 24 entradas previas idénticas. Doom 2 maps con mancubus + revenant + archvile ahora emiten cada uno su tinte característico; lost souls tintan blue-white local; pickups soul/mega irradian cyan; keys recogidas iluminan su entorno con el color del HUD. 129 tests verde (+5: doom2-projectiles, archvile-flame, lost-soul-pickups, colored-keys, case-insensitive-doom2). Header `PHASE 3.35` → `PHASE 3.36`.
- **2026-05-30 (+10):** Fase 3.35 — BRDF 3D para mobj sprites. `world_lights_boost_rgb_for_sprite_cam` y `combined_boost_rgb_sprite_cam` toman `z_surf_cam: f32` (sample point vertical, `sprite.z - cam.view_z` desde `gather_sprite`). Normal sigue 2D toward-camera (billboard model); distancia y cosine en 3D: `d² = dx²+dy²+dz²`, `cos = (nx·dx + ny·dy)/d_3D`. Mobj recibe menos tinte de luces verticalmente lejanas; radio 3D corta proyectiles altos. Plumbing en los dos sites de `gather_sprite`. 9 callers de test actualizados via perl. Compatibilidad 3.31 bit-equivalent cuando z_cam=z_surf=0. **Cierra el ciclo direccional 3D** — todas las superficies con normal definida (sprites, walls, floors, ceilings) usan BRDF 3D unificado. 124 tests verde (+5). Header `PHASE 3.34` → `PHASE 3.35`.
- **2026-05-30 (+9):** Fase 3.34 — BRDF 3D para paredes. `world_lights_boost_rgb_for_wall_cam` y `combined_boost_rgb_wall_cam` toman `z_surf_cam: f32` (sample point vertical, `0.0` = eye level del jugador desde `gather_wall`). La distancia y el cosine pasan a 3D: `d² = dx²+dy²+dz²`, `cos = (nx·dx + ny·dy)/d_3D` (la wall normal tiene `nz=0`, las paredes son verticales). Antorcha alta a misma XY que la pared atenúa más (`cos < cos_2D`); radio 3D corta luces remotas en vertical aunque XY caiga adentro. 7 calls test fixures actualizadas con `z_surf_cam=0.0` via perl. Compatibilidad 3.32 bit-equivalent cuando todas las luces tienen `z_cam=0` (caso común en los fixtures previos). `wall_rim_directional=false` ⇒ omni 2D 3.29 sin cambio. 119 tests verde (+5: high-attenuates-vs-planar, 3d-radius-cuts-vertical, planar-finite, disabled-omni, zero-distance-safe). Header `PHASE 3.33` → `PHASE 3.34`.
- **2026-05-30 (+8):** Fase 3.33 — BRDF 3D para pisos y techos. `WorldLight` gana `z_cam` (sprite.z menos `cam.view_z`). Nuevos helpers `world_lights_boost_rgb_for_plane_cam` + `combined_boost_rgb_plane_cam` con falloff por d² 3D + cosine `n_z·dz/d_3D`. Pisos usan `n_z=+1`, techos `n_z=-1`. `gather_subsector_planes` ahora calcula el boost dentro de `emit_plane` (una vez por floor, una por ceiling) — eliminado el cómputo único compartido. `RenderConfig::plane_rim_directional` default on; `false` ⇒ bit-equivalente al path omni 2D 3.29. `combined_boost_rgb_cam` queda `#[cfg(test)]` — todos los callers del render loop pasaron a variantes especializadas (wall, sprite, plane, weapon). Proyectil al ras del piso ilumina fuerte el piso y rasante el techo; antorcha alta ilumina más el techo. Radio 3D corta luces que el 2D dejaba pasar. 11 fixtures de tests actualizados con `z_cam: 0.0` vía perl. 114 tests verde (+5). Header `PHASE 3.32` → `PHASE 3.33`.
- **2026-05-30 (+7):** Fase 3.32 — rim direccional para paredes. Cierra la trilogía 3.30→3.31→3.32. Cada pared usa `wall_normal_cam(x1, y1, x2, y2, mid_x, mid_y)` para resolver su perpendicular toward-camera; el aporte de cada world light se atenúa por `max(0.3, 0.5 + 0.5·cos(θ))`. Antorcha perpendicular ⇒ 100 %, rasante (paralela al lineseg) ⇒ 50 %, detrás del plano ⇒ piso `WALL_RIM_AMBIENT_FLOOR=0.3`. Muzzle queda omni (consistente con 3.30/3.31). Nuevos helpers `world_lights_boost_rgb_for_wall_cam` + `combined_boost_rgb_wall_cam` aplicados en el site del boost de `gather_wall`. `RenderConfig::wall_rim_directional` default on; `false` ⇒ bit-identical al path omni 3.29. Pisos/techos siguen omni. 109 tests verde (+6: normal-orients-camera, normal-degenerate, perpendicular-full, grazing-half, back-floor, disabled-equals-omni). Header `PHASE 3.31` → `PHASE 3.32`.
- **2026-05-30 (+6):** Fase 3.31 — rim direccional de mobj sprites. Generaliza el cosine-falloff del 3.30 a billboards: cada sprite usa fake-normal toward-camera `(-x_surf, -y_surf)/|surf|` y atenúa el aporte de cada world light por `max(0.3, 0.5 + 0.5·cos(θ))`. Antorcha entre cámara y sprite ⇒ 100 %, lateral 50 %, detrás 30 % (piso `SPRITE_RIM_AMBIENT_FLOOR`). Nuevos helpers `world_lights_boost_rgb_for_sprite_cam` + `combined_boost_rgb_sprite_cam` aplicados en los dos sites de `gather_sprite` (patch texturizado + fallback). Casos degenerados (sprite en cámara, luz coincidente con sprite) caen al path omni sin NaN. `RenderConfig::sprite_rim_directional` default on; `false` ⇒ bit-identical al path omni 3.27/3.29. Walls/pisos/techos siguen omni. 103 tests verde (+5: front-matches-omni, back-falls-to-floor, side-half, disabled-equals-omni, zero-position-safe). Header `PHASE 3.30` → `PHASE 3.31`.
- **2026-05-30 (+5):** Fase 3.30 — rim direccional del arma. Nuevo helper `weapon_rim_boost_rgb_cam(player_sec, lights, directional)` con atenuación cosine (`+X_cam` como fake-normal del psprite). Luces frontales aportan al 100 %, laterales al 50 %, traseras al 30 % (piso `WEAPON_RIM_AMBIENT_FLOOR`). Caso degenerado `d≈0` ⇒ att=1.0 (evita NaN). `RenderConfig::weapon_rim_directional` default on; `directional=false` cae al path omni 3.29 bit-identical. Sólo afecta al rim del arma — la escena conserva el 3.27. 98 tests verde (+5: front-full, behind-attenuates, side-half, disabled-equals-omni, zero-distance). Header `PHASE 3.29` → `PHASE 3.30`.
- **2026-05-30 (+4):** Fase 3.29 — oclusión sectorial de world lights. `compute_muzzle_lit_sectors` se factorea en `compute_lit_sectors_from(snap, src_x, src_y, src_sec, radius)` para que muzzle y world lights compartan la BFS de Dijkstra-lite. `WorldLight` deja de ser `Copy` y carga `lit_sectors: Option<Arc<HashSet<u32>>>` precomputado por `gather_world_lights(.., enable_occlusion)` con el sector y la posición mundo del mobj como origen. `world_lights_boost_rgb_cam` toma `surf_sector` y descarta luces cuyo set no lo incluye. `combined_boost_rgb_cam` propaga `surf_sector`; los 4 gather sites no cambian. Weapon rim usa `subsector_at_point` para el sector del player. `RenderConfig::world_lights_occlusion` default on; sin toggle host (F-keys agotadas). BFG ball en cuarto vecino deja de leakear verde a paredes del cuarto del player; antorchas tintan sólo su cuarto + vecinos directos. Costo ≤ 8 BFS/frame, ≤ 2 hops, ≤ 16 sectores cada uno. 93 tests verde (+5: source-includes-self, surf-not-in-set-skips, none-is-passthrough, computes-when-on-with-bsp, skips-when-off-or-no-bsp). Compatibilidad 3.28 bit-equivalent con `world_lights_occlusion=false`. Header `PHASE 3.28` → `PHASE 3.29`.
- **2026-05-30 (+3):** Fase 3.28 — weapon rim-light desde world lights. El sprite del psprite del jugador (`snap.weapon` + `snap.weapon_flash`) recibe `world_lights_boost_rgb_cam(0, 0, &lights)` evaluado en la posición del player, aplicado per-canal vía `sprite_shade_with_world` + `make_tinted_sprite_image_rgb`. Caminar al lado de un TBLU torch tinta la pistola azulada; un BAL1 fireball pasando cerca le pinta rim rojizo. F11 toggle. Bypass full-bright preserva muzzle flash a luz plena. Cleanup: removido `make_tinted_sprite_image` (wrapper scalar obsoleto), `make_tinted_sprite_image_rgb` queda como única API. `RenderConfig::weapon_rim_light` default on. 88 tests verde (+5 nuevos: zero/blue-skew/red-skew/out-of-radius/full-bright bypass). Locales en/es/qu. Header `PHASE 3.27` → `PHASE 3.28`.
- **2026-05-30 (+2):** Fase 3.27 — tinte per-spritenum + boost RGB per-canal. Tabla `FB_SPRITE_TINTS` con 24 entradas (BFG verde, plasma azul, fireballs rojos, antorchas teñidas, fogs, candles, lamps). `WadAtlas::sprite_name` getter público + `sprite_tint_for_name` resuelve por 4-char case-insensitive con fallback al amarillo cálido del muzzle. `WorldLight.tint_rgb` resuelto al gatherearse. Refactor del boost a `BoostRgb = [f32; 3]` per-canal: nuevos `muzzle_boost_rgb_cam`, `world_lights_boost_rgb_cam`, `combined_boost_rgb_cam`, `apply_color_boost`, `sprite_shade_with_world`, `overlay_color_alpha_from_boost`. Path scalar legacy queda `#[cfg(test)]` con sus 8 tests verde. 4 sites en `gather_*` migrados al path RGB. 83 tests verde (+12 nuevos: tabla/case/RGB-per-canal/clamp/overlay normalization). Compatibilidad 3.26 preservada cuando todas las luces usan el tinte default.
- **2026-05-30 (+1):** Fase 3.26 — world point lights desde mobjs `FF_FULLBRIGHT`. `gather_world_lights` recolecta sprites con bit 7 set, los transforma a cam-space y trunca a `MAX_WORLD_LIGHTS=8` más cercanos al player. `world_lights_boost_cam` suma falloffs `f²·PEAK` con `WORLD_LIGHT_RADIUS_WORLD=192`, `WORLD_LIGHT_PEAK=0.40`, clampeado a `MUZZLE_BOOST_PEAK`. `combined_boost_cam` unifica muzzle + world lights en un solo helper, reemplaza 4 sites de cómputo en `gather_wall`/`gather_subsector_planes`/`gather_sprite`. Proyectiles iluminan corridors oscuros, explosiones irradian destellos, plasma deja halos. F10 toggle host. Locales i18n actualizadas (en/es/qu). 71 tests verde renderer (+8 nuevos). Compatibilidad 3.25 preservada bit-exact cuando no hay sprites FF_FULLBRIGHT.
- **2026-05-30:** Fase 3.25 — radio cumulativo por hop. `compute_muzzle_lit_sectors` ahora hace Dijkstra-lite sobre midpoints encadenados: cada sector cachea su entry midpoint, el siguiente hop se mide desde ahí. En cadenas rectas el comportamiento es idéntico al 3.24 (cumulative == player_dist); en U/L-shapes el cumulativo cuts donde 3.24 dejaba pasar el falso positivo. Hop cap `MUZZLE_BFS_MAX_HOPS=2` preservado como safety net dual al radio. Triangle inequality garantiza 3.25 ⊂ 3.24 en cobertura. 63 tests verde renderer (+2 nuevos: L-shape cumulative cut + entry-chaining correctness). 10 tests del 3.23/3.24 siguen verdes.
- **2026-05-29 (+3):** Fase 3.24 — BFS multi-hop + filtro por radio en el lit set del muzzle. `compute_muzzle_lit_sectors` ahora BFS hasta `MUZZLE_BFS_MAX_HOPS=2` con cada bridge wall filtrado por midpoint dentro de `MUZZLE_RADIUS_WORLD`. El corredor saliente del cuarto del player (1 puerta más allá) entra al lit cuando antes quedaba oscuro; cuartos al final de pasillos largos siguen excluidos por el radius cut. 61 tests verde renderer (+3 nuevos: 2-hop included / BFS bound at MAX / 1-hop excluido por bridge fuera de radio). Compatibilidad 3.23 preservada.
- **2026-05-29 (+2):** Fase 3.23 — oclusión sectorial del muzzle boost: `compute_muzzle_lit_sectors` resuelve el sector del player vía BSP + agrega vecinos via two-sided linedefs; `muzzle_boost_gated` corta el boost en sectores fuera del set. Threaded como `Option<&HashSet<u32>>` por `gather_wall/subsector_planes/sprite`. F9 host toggle, default on. Stub mode preserva el comportamiento 3.22 (`lit_sectors = None` ⇒ pasa-todo). 58 tests verde renderer (+7 nuevos: lit_set contains/excludes + gated pass/keep/zero).
- **2026-05-29 (+1):** Fase 3.22 — muzzle world light: el fogonazo del arma irradia un boost cálido (`MUZZLE_TINT (255,220,140)`) en disco de 384 unidades alrededor del jugador con falloff cuadrático, decae en `MUZZLE_DECAY_SECS = 0.16`. Detección host por bit `FF_FULLBRIGHT` en `weapon` o `weapon_flash`, alpha decae linealmente. Aplica a paredes/pisos/techos/sprites untextured (suma color), texturizados (overlay aditivo + reducción del darkness overlay), sprites texturizados (multiplicador per-canal en `make_tinted_sprite_image_rgb`). F8 toggle. 51 tests verde renderer (+8 nuevos: boost zero/outside/peak/falloff² + tint warmth/identity + per-canal shade variants).
- **2026-05-27:** Fase 3.18 — weapon shading por luz del sector del jugador. Helper `subsector_at_point` (BSP point query O(log N)) + `player_sector_light` resuelven el light del cuarto donde está parado el player; `draw_weapon_sprite` tinta el patch con `make_tinted_sprite_image`. Muzzle flash mantiene full-bright por FF_FULLBRIGHT (bit 7 del frame). Cuartos oscuros ya no muestran la pistola pegada como sticker iluminado. 43 tests verde renderer (+4 nuevos).
- **2026-05-26 (+16):** Fase 3.17 — mouse-look cosmético (y-shear del rasterizador + sky backdrop siguiendo el horizonte). PageUp/PageDown mueven el horizonte ±6° por tap; Home resetea. La simulación queda intacta (hitboxes/autoapuntado siguen sin pitch). 39 tests verde renderer (+4 nuevos).
- **2026-05-26 (+15):** Fase 3.16 — `ps_flash` (muzzle flash overlay) + berserk red tint en overlays. Plasma/BFG/chaingun ahora muestran el destello brillante por encima del arma; agarrar el berserk tinte rojo el frame por un rato. 35 tests verde renderer.
- **2026-05-26 (+14):** Fase 3.15 — weapon psprite (pistol/shotgun/etc. en mano). Capture de `players[].psprites[ps_weapon]` desde doomgeneric, render como image overlay 2D anclado al bottom del viewport. Smoothing de sx/sy entre snapshots para weapon bob suave.
- **2026-05-26 (+13):** Fase 3.14 — player palette overlays (damage red, pickup yellow, radsuit green, invuln white) como overlay alpha full-screen. Modernización de PLAYPAL[1..13] swap → un único fill semi-translúcido por frame. 33 tests verde renderer.
- **2026-05-26 (+12):** Fase 3.13 — BSP back-to-front ordering exacto para pisos/techos (expone `nodes[]`, walker recursivo, depth `1e6 + step` reemplaza el centroide euclidiano para Renderable.depth de planos). Escaleras y sectores interpenetrados dejan de glitchear en el painter's. Walls/sprites siguen euclidiano. 28 tests verde renderer.
- **2026-05-26 (+11):** Fase 3.12 — pisos y techos per-triangle (fan triangulation desde vértice 0 + affine exacta por triángulo). Desaparece el "affine sheen" residual de 3.7 en pisos grandes vistos oblicuos.
- **2026-05-26 (+10):** Fase 3.11 — flats/paredes animados (NUKAGE/FIREBLU/BLOOD via `flattranslation[]`, switches via `texturetranslation[]`) + sprites full-bright (bit 7 del frame). Proyectiles y muzzle flashes ahora brillan en cuartos oscuros.
- **2026-05-26 (+9):** Fase 3.10 — `textureoffset`/`rowoffset` del sidedef + pegging Doom (`ML_DONTPEGTOP`/`ML_DONTPEGBOTTOM`). Las costuras entre paredes adyacentes se alinean correctamente; las puertas mantienen su textura quieta cuando suben.
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
