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

### Fase 1 — Doom real (próximos bloques)

`supay-core` enlaza `doomgeneric` vendoreado (~10k LOC C bien aislado) vía `cc` + bindings minimalistas. Loadeá `DOOM1.WAD` shareware (distribución legal). El framebuffer 320×200 ARGB que doomgeneric llena se pinta como `View::image` dentro de una ventana Llimphi — "Doom corriendo en una ventana Llimphi", baja resolución, software renderer original. Input: traducir keys de Llimphi a `DG_GetKey`. Es modesto pero valida la integración entera.

### Fase 2 — Scene extraction

Parcho a doomgeneric para exponer estado interno (linedefs, sprites, sectors, brightness) además del framebuffer. Crate `supay-scene` mantiene snapshots inmutables por tick que el renderer consume con interpolación entre dos snapshots consecutivos para alcanzar 144+ Hz suave. El framebuffer original deja de pintarse — solo es referencia de validación.

### Fase 3 — Renderer 3D moderno

`supay-render-llimphi` deja `View::paint_with` y pasa a `View::custom_pass(wgpu)` (feature nueva de llimphi-ui a agregar). Pipeline:

- **Mesh cache** linedef → vértices; invalida solo en cambio (linedef movible: doors, lifts).
- **Sprites**: billboards Y-up con BRDF basado en sector lights — el sprite reacciona a la iluminación real.
- **Iluminación**: sector brightness como point lights atenuadas; volumetric fog por sector.
- **Shadows**: stencil shadows desde sector lights (baseline); RT shadows si `VK_KHR_ray_tracing_pipeline` está y el usuario opta in.
- **Temporal**: TAA accumulation; ACES tonemap.

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
| Render moderno (Fase 3+) | `supay-render-llimphi` | `llimphi-ui` (+ `wgpu` directo cuando llimphi-ui gane `custom_pass`) |
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
- **2026-05-25 (cierre+3):** Fase 0.9 — pickups + game over + victoria + reset:
  - **Pickups** estáticos en mapa: 3× AmmoBox (+12 munición) cyan + 2× HealthKit (+25 vida, max 100) verde. Sprite scale 0.35, apoyados al piso. `consume_pickups` chequea dist² al jugador cada tick (radio 0.55), aplica bonus + spawn flash del color del pickup, remueve. Drop-on-pickup, no respawnean.
  - **Game over**: cuando `health == 0` al final del tick, `m.game_over = true`. Bloquea movimiento + disparo; advance solo envejece flashes. Space pasa a dispatchar `Msg::Reset` en vez de `Msg::Fire`.
  - **Victoria**: cuando `enemies.iter().all(|e| Dead)` y no hubo muerte previa, `m.victory = true`. Mismo handling que game_over (Space reinicia).
  - **`reset_game(&mut Model)`** restaura posición/ángulo/HP/ammo + `initial_enemies()` + `initial_pickups()` + limpia listas dinámicas.
  - **Overlay full-screen** vía `View::paint_with` que recibe `Typesetter` cacheado del runtime: rect negro semi-transparente (alpha 175) + título 64 px (MUERTO rojo / VICTORIA verde) + subtítulo 18 px ("SPACE para reiniciar") centrados con parley.
  - **Refactor `on_key`**: ahora recibe `&Model` (siempre lo hizo, lo aprovechamos) para decidir qué Msg disparar Space según `game_over || victory`.
