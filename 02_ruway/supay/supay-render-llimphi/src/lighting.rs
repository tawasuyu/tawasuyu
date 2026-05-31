use super::*;

/// Radio de influencia del fogonazo, en unidades Doom. ~6 cells de 64
/// → habitación pequeña entera, pasillo medio.
pub(crate) const MUZZLE_RADIUS_WORLD: f32 = 384.0;
/// Boost de shade en el centro (player position) con `alpha=1.0`. Se
/// suma al shade base, capeado a 1.0 — paredes oscuras quedan visibles
/// durante el flash sin "blow out" las claras.
pub(crate) const MUZZLE_BOOST_PEAK: f32 = 0.55;
/// Tinte cálido amarillo-blanco del fogonazo, en RGB 0..255.
pub(crate) const MUZZLE_TINT_RGB: (u8, u8, u8) = (255, 220, 140);

/// Devuelve el boost de luz del muzzle flash para un punto en cam-space
/// (player está en `(0, 0)`). Cae con distancia² hasta `MUZZLE_RADIUS_WORLD`.
pub(crate) fn muzzle_boost_cam(x_cam: f32, y_cam: f32, alpha: f32) -> f32 {
    if alpha <= 0.0 {
        return 0.0;
    }
    let d2 = x_cam * x_cam + y_cam * y_cam;
    let r2 = MUZZLE_RADIUS_WORLD * MUZZLE_RADIUS_WORLD;
    if d2 >= r2 {
        return 0.0;
    }
    let f = 1.0 - d2 / r2;
    (f * f * alpha * MUZZLE_BOOST_PEAK).clamp(0.0, MUZZLE_BOOST_PEAK)
}

/// Variante 3D del falloff del muzzle (Fase 3.40). Incluye el componente
/// `z_cam` en la distancia: una superficie alta o baja del jugador queda
/// fuera del rango del fogonazo cuando la distancia 3D la supera, aunque
/// su distancia horizontal sea chica. Usado por las versiones BRDF del
/// muzzle (`muzzle_boost_rgb_wall_3d`, `muzzle_boost_rgb_plane_3d`) para
/// que el modelo direccional sea coherentemente 3D — el scalar 2D del
/// `muzzle_boost_cam` sigue activo en el path omni (`muzzle_brdf=false`,
/// default) y en mobjs/weapon.
pub(crate) fn muzzle_boost_cam_3d(x_cam: f32, y_cam: f32, z_cam: f32, alpha: f32) -> f32 {
    if alpha <= 0.0 {
        return 0.0;
    }
    let d2 = x_cam * x_cam + y_cam * y_cam + z_cam * z_cam;
    let r2 = MUZZLE_RADIUS_WORLD * MUZZLE_RADIUS_WORLD;
    if d2 >= r2 {
        return 0.0;
    }
    let f = 1.0 - d2 / r2;
    (f * f * alpha * MUZZLE_BOOST_PEAK).clamp(0.0, MUZZLE_BOOST_PEAK)
}

/// Suma aditivamente el tinte cálido `MUZZLE_TINT_RGB · boost` al color
/// base, preservando alpha. Boost ≤ 0 ⇒ no-op.
#[cfg(test)]
pub(crate) fn apply_muzzle_tint(c: Color, boost: f32) -> Color {
    if boost <= 0.0 {
        return c;
    }
    let [r, g, b, a] = c.to_rgba8().to_u8_array();
    let add_r = (MUZZLE_TINT_RGB.0 as f32 * boost) as u32;
    let add_g = (MUZZLE_TINT_RGB.1 as f32 * boost) as u32;
    let add_b = (MUZZLE_TINT_RGB.2 as f32 * boost) as u32;
    Color::from_rgba8(
        (r as u32 + add_r).min(255) as u8,
        (g as u32 + add_g).min(255) as u8,
        (b as u32 + add_b).min(255) as u8,
        a,
    )
}

/// Multiplicador per-canal para tintar el patch del sprite cuando el
/// muzzle flash está activo. Devuelve `(shade·tint_r, shade·tint_g,
/// shade·tint_b)` con `tint = 1 + boost · MUZZLE_TINT/255` clampeado.
/// Cuando `boost = 0` devuelve `[shade, shade, shade]` — equivalente al
/// shading grayscale histórico.
#[cfg(test)]
pub(crate) fn sprite_shade_with_muzzle(shade: f32, boost: f32) -> [f32; 3] {
    if boost <= 0.0 {
        return [shade, shade, shade];
    }
    let tr = 1.0 + boost * (MUZZLE_TINT_RGB.0 as f32 / 255.0);
    let tg = 1.0 + boost * (MUZZLE_TINT_RGB.1 as f32 / 255.0);
    let tb = 1.0 + boost * (MUZZLE_TINT_RGB.2 as f32 / 255.0);
    [
        (shade * tr).clamp(0.0, 1.0),
        (shade * tg).clamp(0.0, 1.0),
        (shade * tb).clamp(0.0, 1.0),
    ]
}

/// Fase 3.24 — máximo de saltos del BFS de sectores iluminables.
///
/// 1 hop = vecino directo del jugador (puerta/ventana inmediata).
/// 2 hops = vecino del vecino — cubre escenarios típicos de Doom donde
/// el flash debería alcanzar el siguiente cuarto a través de una puerta
/// abierta sin necesidad de R_CheckSight completo. >2 hops empieza a
/// "filtrar" luz por geometrías retorcidas sin agregar valor visual; el
/// radio cumulativo (`MUZZLE_RADIUS_WORLD`) ya cortaría antes en la
/// mayoría de los casos.
pub(crate) const MUZZLE_BFS_MAX_HOPS: usize = 2;

/// Fase 3.25 — conjunto de sectores que reciben el muzzle boost,
/// computado por relajación tipo Dijkstra sobre **caminos acumulativos**
/// player→midpoint(W₁)→midpoint(W₂)→…, no por chequeo per-bridge contra
/// el centro del jugador.
///
/// El destello del arma sólo ilumina superficies del cuarto donde está
/// parado el jugador y de los cuartos alcanzables a `≤
/// MUZZLE_BFS_MAX_HOPS` linedefs two-sided **cuya distancia total
/// recorrida por la cadena de bridge walls** esté dentro de
/// `MUZZLE_RADIUS_WORLD`. Cada sector cachea el midpoint del último
/// bridge wall por el que se entró — el siguiente hop se mide desde ese
/// punto, no desde el jugador. Eso modela mejor "hasta dónde llegaría
/// la luz si tuviera que atravesar cada puerta en orden", y corta
/// correctamente en U-shapes/L-shapes donde un sector distante quedaba
/// visualmente lit en 3.24 aunque su camino real fuera más largo que
/// el disco del flash.
///
/// La relajación de Dijkstra-lite también garantiza que si un sector
/// es alcanzable por dos caminos, gana el más corto — el "entry point"
/// queda fijado al midpoint del camino más corto encontrado.
///
/// Una pared sólida entre medio sigue cortando la luz — sin two-sided
/// no hay arista en el grafo.
///
/// La heurística es O(walls · hops · sectores_visitados) por frame. En
/// E1M1 (~400 walls × 2 hops × <16 sectores visitados) ≈ 13k checks/s
/// cuando el flash está activo (<5 % del tiempo). Sin alocaciones extra
/// significativas.
///
/// Devuelve `None` cuando no hay BSP (modo stub o mapa pre-carga); en
/// ese caso el caller debe asumir "todo lit" y aplicar el comportamiento
/// 3.22 (boost everywhere).
pub(crate) fn compute_muzzle_lit_sectors(snap: &SceneSnapshot) -> Option<HashSet<u32>> {
    if snap.nodes.is_empty() || snap.subsectors.is_empty() {
        return None;
    }
    let player_ss = subsector_at_point(&snap.nodes, snap.player.x, snap.player.y)?;
    let ss = snap.subsectors.get(player_ss as usize)?;
    let player_sec = ss.sector;
    Some(compute_lit_sectors_from(
        snap,
        snap.player.x,
        snap.player.y,
        player_sec,
        MUZZLE_RADIUS_WORLD,
    ))
}

/// BFS sectorial reusable: desde un sector fuente (con su entry point
/// en coords mundo) explora vecinos via linedefs two-sided hasta
/// [`MUZZLE_BFS_MAX_HOPS`] hops, con corte cumulativo por `radius`
/// (suma de tramos entre midpoints) — la misma maquinaria de 3.25 para
/// el muzzle, parametrizada para soportar también world lights (Fase
/// 3.29). El sector fuente queda con `dist=0`, los vecinos accesibles
/// con su distancia cumulativa.
pub(crate) fn compute_lit_sectors_from(
    snap: &SceneSnapshot,
    src_x: f32,
    src_y: f32,
    src_sec: u32,
    radius: f32,
) -> HashSet<u32> {
    let mut dist: HashMap<u32, f32> = HashMap::with_capacity(16);
    let mut entry: HashMap<u32, (f32, f32)> = HashMap::with_capacity(16);
    let mut hops: HashMap<u32, usize> = HashMap::with_capacity(16);
    dist.insert(src_sec, 0.0);
    entry.insert(src_sec, (src_x, src_y));
    hops.insert(src_sec, 0);
    // Cola de trabajo. No es un BinaryHeap real porque el set típico es
    // <16 sectores; un Vec con relajación re-inserta y deja que la
    // condición `better` filtre lo redundante. Suficiente y sin deps.
    let mut queue: Vec<u32> = vec![src_sec];
    while let Some(sec) = queue.pop() {
        let d_sec = dist[&sec];
        let (ex, ey) = entry[&sec];
        let h_sec = hops[&sec];
        if h_sec >= MUZZLE_BFS_MAX_HOPS {
            continue;
        }
        for wall in snap.walls.iter() {
            if wall.back_sector == NO_SECTOR {
                continue;
            }
            let other_sec = if wall.front_sector == sec {
                wall.back_sector
            } else if wall.back_sector == sec {
                wall.front_sector
            } else {
                continue;
            };
            if other_sec == sec {
                continue;
            }
            let mx = (wall.x1 + wall.x2) * 0.5;
            let my = (wall.y1 + wall.y2) * 0.5;
            let dx = mx - ex;
            let dy = my - ey;
            let hop_d = (dx * dx + dy * dy).sqrt();
            let new_d = d_sec + hop_d;
            if new_d > radius {
                continue;
            }
            let better = match dist.get(&other_sec) {
                Some(&existing) => new_d < existing,
                None => true,
            };
            if better {
                dist.insert(other_sec, new_d);
                entry.insert(other_sec, (mx, my));
                hops.insert(other_sec, h_sec + 1);
                queue.push(other_sec);
            }
        }
    }
    dist.into_keys().collect()
}

/// Gate del muzzle boost por sector cuando la oclusión está activa.
/// `sector_id` es el sector "dueño" de la superficie (subsector.sector
/// para planos; sprite.sector para sprites; front-side sector para la
/// pared). Si la oclusión está activa y `sector_id ∉ lit_sectors`, la
/// función devuelve 0 (sin boost). Sin oclusión o sin BSP devuelve el
/// boost crudo.
#[cfg(test)]
pub(crate) fn muzzle_boost_gated(
    boost: f32,
    sector_id: u32,
    lit_sectors: Option<&HashSet<u32>>,
) -> f32 {
    match lit_sectors {
        Some(lit) if !lit.contains(&sector_id) => 0.0,
        _ => boost,
    }
}

// =====================================================================
// Fase 3.26 — World point lights desde FF_FULLBRIGHT mobjs
// =====================================================================
//
// Doom marca varios mobjs con `FF_FULLBRIGHT` (bit 7 del frame): proyectiles
// en vuelo (imp fireballs, plasma, BFG, rocket), muzzle puffs, frames de
// explosión de barriles, BFG splash, teleport fog. Estos sprites ya se
// pintaban a luz plena desde 3.11 (sprite-side), pero **no irradiaban luz
// al mundo**: un fireball pasando por un cuarto oscuro dejaba el cuarto
// oscuro. Modernización: tratamos cada mobj FF_FULLBRIGHT como una fuente
// puntual con la misma maquinaria del muzzle (tinte cálido, falloff
// cuadrático, sumado al shade base). El muzzle del jugador queda como un
// caso particular anclado en el origen del cam-space.
//
// La diferencia clave vs. muzzle: estas luces están en posiciones
// arbitrarias del mundo, no en el player. Por eso no se les aplica el
// `lit_sectors` set (que se computa relativo al cuarto del jugador). Se
// gatean sólo por radio. El radio chico (mitad que muzzle) limita el leak
// natural a través de paredes; los mobjs FF_FULLBRIGHT en Doom son
// efímeros (1-30 ticks), así que un leak fugaz es invisible en práctica.

/// Radio de influencia de una luz puntual del mundo, en unidades Doom.
/// Más chico que `MUZZLE_RADIUS_WORLD` porque la "fuerza" de un fireball
/// o un puff es muy inferior al fogonazo cercano de un arma en mano.
pub(crate) const WORLD_LIGHT_RADIUS_WORLD: f32 = 192.0;
/// Peak del boost en el centro de una luz puntual con `alpha=1.0`.
/// Menor que `MUZZLE_BOOST_PEAK` (0.55) — el sumado de varias luces
/// puede acercarse al peak del muzzle, pero una sola no debería
/// "blow out" la escena.
pub(crate) const WORLD_LIGHT_PEAK: f32 = 0.40;
/// Cap del número de world lights consideradas por frame. Cubrimos los
/// proyectiles + puffs + explosiones simultáneas razonables sin pagar
/// O(surfaces · lights) descontrolado. 8 cubre escenarios típicos
/// (cyberdemon spam, BFG en cluster), el resto se descarta por
/// distancia.
pub(crate) const MAX_WORLD_LIGHTS: usize = 8;

#[derive(Clone, Debug)]
pub(crate) struct WorldLight {
    /// Posición en cam-space (forward, right).
    pub(crate) x_cam: f32,
    pub(crate) y_cam: f32,
    /// Fase 3.33: altura del mobj relativa a `cam.view_z` — `sprite.z`
    /// menos la altura del ojo del jugador. Necesaria para el cosine
    /// BRDF de pisos/techos (normal ±Z) y para que el radio sea 3D-aware
    /// en el helper `world_lights_boost_rgb_for_plane_cam`. El gather
    /// inicial sigue filtrando por d² 2D × 4 (margen generoso) — la
    /// distancia 3D real se chequea dentro del helper de cada plano.
    pub(crate) z_cam: f32,
    /// Sector "dueño" del mobj — origen del BFS de oclusión 3.29.
    pub(crate) sector: u32,
    /// Fase 3.27: tinte característico del mobj resuelto vía
    /// `sprite_tint_for_name`. Cae al amarillo cálido del muzzle si el
    /// sprite es desconocido para la tabla.
    pub(crate) tint_rgb: (u8, u8, u8),
    /// Fase 3.29: sectores alcanzables desde `sector` por linedefs
    /// two-sided, BFS hasta [`MUZZLE_BFS_MAX_HOPS`] con corte
    /// cumulativo por [`WORLD_LIGHT_RADIUS_WORLD`]. `None` cuando la
    /// oclusión está desactivada o no hay BSP en el snapshot — el caller
    /// asume "ilumina todo" (comportamiento 3.27). `Arc` para compartir
    /// el set sin copiar entre las múltiples superficies que consultan
    /// la misma luz por frame.
    pub(crate) lit_sectors: Option<Arc<HashSet<u32>>>,
}

/// Recolecta las luces puntuales del mundo del snapshot: cada sprite
/// con bit `FF_FULLBRIGHT` (0x80) en su frame contribuye una luz en su
/// posición. Sprites con `sector == NO_SECTOR` se descartan (sin
/// referencia válida). Se transforman a cam-space y se queda con los
/// `MAX_WORLD_LIGHTS` más cercanos al jugador (origen del cam-space).
///
/// Costo: O(sprites + N·log N) por frame con N ≈ sprites; en mapas Doom
/// el número de sprites visibles es <60, despreciable.
pub(crate) fn gather_world_lights(
    snap: &SceneSnapshot,
    cam: &Camera,
    atlas: Option<&Arc<WadAtlas>>,
    enable_occlusion: bool,
) -> Vec<WorldLight> {
    let mut lights: Vec<(f32, WorldLight)> = snap
        .sprites
        .iter()
        .filter(|s| (s.frame & 0x80) != 0 && s.sector != NO_SECTOR)
        .map(|s| {
            let (x_cam, y_cam) = cam.to_cam_2d(s.x, s.y);
            // Fase 3.33: z relativa al ojo del jugador. Permite que el
            // helper de pisos/techos calcule cos(θ) con normal ±Z.
            let z_cam = s.z - cam.view_z;
            let d2 = x_cam * x_cam + y_cam * y_cam;
            // Fase 3.27: tinte per-mobj. Si el atlas no tiene el nombre
            // (o no hay atlas — modo sin WAD), cae al amarillo cálido
            // del muzzle (comportamiento 3.26).
            let tint_rgb = atlas
                .and_then(|a| a.sprite_name(s.sprite))
                .map(|name| sprite_tint_for_name(&name))
                .unwrap_or(MUZZLE_TINT_RGB);
            (
                d2,
                WorldLight {
                    x_cam,
                    y_cam,
                    z_cam,
                    sector: s.sector,
                    tint_rgb,
                    lit_sectors: None,
                },
            )
        })
        .filter(|(d2, _)| *d2 < WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD * 4.0)
        .collect();
    if lights.len() > MAX_WORLD_LIGHTS {
        lights.select_nth_unstable_by(MAX_WORLD_LIGHTS, |a, b| {
            a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
        });
        lights.truncate(MAX_WORLD_LIGHTS);
    }
    // Fase 3.29: oclusión per-light. Cada luz cachea el set de sectores
    // alcanzables desde su sector origen (BFS desde la posición mundo
    // del mobj). Sólo computamos si hay BSP — sin él el caller asume
    // "todo lit" (comportamiento 3.27). Para revertir la transformación
    // cam→world usamos `cam.from_cam_2d`, evitando re-iterar los
    // sprites originales.
    if enable_occlusion && !snap.nodes.is_empty() && !snap.subsectors.is_empty() {
        for (_, l) in lights.iter_mut() {
            let (sx, sy) = cam.from_cam_2d(l.x_cam, l.y_cam);
            let set =
                compute_lit_sectors_from(snap, sx, sy, l.sector, WORLD_LIGHT_RADIUS_WORLD);
            l.lit_sectors = Some(Arc::new(set));
        }
    }
    lights.into_iter().map(|(_, l)| l).collect()
}

/// Suma de boosts de todas las world lights en un punto cam-space.
/// Cada luz contribuye `f²·PEAK` con `f = 1 - d²/r²`, clampeado al
/// peak del muzzle (no superar el destello del arma propia es un
/// invariante del sistema — el flash debe seguir siendo el efecto
/// dominante).
#[cfg(test)]
pub(crate) fn world_lights_boost_cam(x_cam: f32, y_cam: f32, lights: &[WorldLight]) -> f32 {
    if lights.is_empty() {
        return 0.0;
    }
    let r2 = WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD;
    let mut sum = 0.0_f32;
    for l in lights {
        let dx = x_cam - l.x_cam;
        let dy = y_cam - l.y_cam;
        let d2 = dx * dx + dy * dy;
        if d2 >= r2 {
            continue;
        }
        let f = 1.0 - d2 / r2;
        sum += f * f * WORLD_LIGHT_PEAK;
        if sum >= MUZZLE_BOOST_PEAK {
            return MUZZLE_BOOST_PEAK;
        }
    }
    sum.min(MUZZLE_BOOST_PEAK)
}

/// Boost combinado (muzzle + world lights) en un punto cam-space. El
/// muzzle se gatea por `lit_sectors` (Fase 3.23-3.25); las world lights
/// sólo por radio. La suma se clampea a `MUZZLE_BOOST_PEAK` para
/// preservar el invariante "el fogonazo nunca debe sentirse más débil
/// que un proyectil distante".
#[cfg(test)]
pub(crate) fn combined_boost_cam(
    x_cam: f32,
    y_cam: f32,
    muzzle_alpha: f32,
    surf_sector: u32,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
) -> f32 {
    let muzzle = muzzle_boost_gated(
        muzzle_boost_cam(x_cam, y_cam, muzzle_alpha),
        surf_sector,
        lit_sectors,
    );
    let wl = world_lights_boost_cam(x_cam, y_cam, world_lights);
    (muzzle + wl).clamp(0.0, MUZZLE_BOOST_PEAK)
}

// =====================================================================
// Fase 3.27 — Tinte per-spritenum para world lights (BFG verde, plasma
// azul, fireballs rojos, antorchas teñidas, etc.)
// =====================================================================
//
// Hasta 3.26 todas las world lights usaban el mismo amarillo cálido
// (`MUZZLE_TINT_RGB`). Pero un proyectil BFG es verde fluorescente, una
// bola de plasma es azul cyan, un fireball de imp es rojo-naranja y una
// antorcha azul de decoración tiñe su cuarto azul. Esta fase refactoriza
// el boost a representación per-canal (`[f32; 3]`) para que cada luz
// emita su tinte característico, sumándose aditivamente en RGB.
//
// La maquinaria scalar (`muzzle_boost_cam`, `apply_muzzle_tint`,
// `sprite_shade_with_muzzle`) sobrevive para los tests existentes y como
// implementación de referencia; el render loop usa la versión RGB.

/// Boost per-canal (R, G, B), cada uno en `[0, MUZZLE_BOOST_PEAK]`.
pub(crate) type BoostRgb = [f32; 3];
pub(crate) const ZERO_BOOST: BoostRgb = [0.0, 0.0, 0.0];

/// Tabla de colores característicos por nombre de sprite Doom (4-char).
/// El nombre viene del WAD (resuelto por `WadAtlas::sprite_name`).
/// Cubre los mobjs `FF_FULLBRIGHT` notables del shareware (Doom 1) y los
/// agregados de Doom 2 / Final Doom (mancubus, revenant, archvile, lost
/// soul, keys, soul sphere, etc.).
pub(crate) const FB_SPRITE_TINTS: &[(&str, (u8, u8, u8))] = &[
    // --- Proyectiles base (Doom 1) ---
    ("BAL1", (255, 130, 60)),  // imp fireball — rojo-naranja
    ("BAL2", (255, 100, 80)),  // caco fireball — rojo
    ("BAL7", (140, 255, 140)), // baron fireball — verde
    ("PLSS", (130, 180, 255)), // plasma en vuelo — azul-cyan
    ("PLSE", (130, 180, 255)), // plasma impact
    ("BFS1", (160, 255, 160)), // BFG ball — verde fluorescente
    ("BFE1", (160, 255, 160)), // BFG explosion
    ("BFE2", (180, 255, 180)), // BFG splash
    ("BFGG", (160, 255, 160)), // BFG launching frames (algunos FB)
    ("MISL", (255, 180, 100)), // rocket — naranja cálido
    ("PUFF", (255, 220, 160)), // bullet puff — amarillo cálido
    ("BEXP", (255, 180, 100)), // barrel/rocket explosion — naranja
    // --- Proyectiles Doom 2 ---
    ("MANF", (255, 160, 90)),  // mancubus fireball — naranja
    ("FATB", (255, 220, 160)), // revenant tracer — pálido amarillo (la cabeza brilla)
    ("SKEL", (255, 200, 150)), // revenant attack frames — pálido cálido
    ("VILE", (255, 130, 70)),  // archvile attack frames — rojo flame
    ("FIRE", (255, 100, 50)),  // archvile fire pillar — rojo-naranja saturado
    // --- Mobjs full-bright en vuelo (Doom 1) ---
    ("SKUL", (180, 220, 255)), // lost soul — blue-white flame
    // --- Fogs / fx ---
    ("TFOG", (140, 200, 255)), // teleport fog — azul
    ("IFOG", (255, 240, 140)), // item respawn — amarillo-blanco
    // --- Pickups que brillan (Doom 1) ---
    ("SOUL", (130, 200, 255)), // soul sphere — azul-cyan
    ("MEGA", (130, 220, 200)), // mega armor — verde-cyan
    // --- Llaves coloreadas (Doom 1 — todas con FF_FULLBRIGHT) ---
    ("BKEY", (110, 160, 255)), // blue keycard
    ("YKEY", (255, 240, 130)), // yellow keycard
    ("RKEY", (255, 130, 90)),  // red keycard
    ("BSKU", (110, 160, 255)), // blue skullkey
    ("YSKU", (255, 240, 130)), // yellow skullkey
    ("RSKU", (255, 130, 90)),  // red skullkey
    // --- Antorchas / decoración (FF_FULLBRIGHT constante, Doom 1) ---
    ("TBLU", (110, 160, 255)), // blue torch (tall)
    ("TGRN", (140, 255, 160)), // green torch (tall)
    ("TRED", (255, 140, 90)),  // red torch (tall)
    ("SMBT", (110, 160, 255)), // short blue torch
    ("SMGT", (140, 255, 160)), // short green torch
    ("SMRT", (255, 140, 90)),  // short red torch
    ("CAND", (255, 200, 130)), // candle — cálido
    ("CBRA", (255, 170, 90)),  // brazier — naranja
    ("TLMP", (255, 240, 200)), // tall lamp — blanco cálido
    ("TLP2", (255, 240, 200)), // short lamp
];

/// Resuelve el tinte característico del sprite a partir de su nombre
/// 4-char. Cae al amarillo cálido del muzzle (`MUZZLE_TINT_RGB`) si el
/// nombre es desconocido — preserva el comportamiento 3.26 para mobjs
/// que el motor reportó pero la tabla no contempla.
pub(crate) fn sprite_tint_for_name(name: &str) -> (u8, u8, u8) {
    let key = name.get(..4).unwrap_or(name);
    for &(k, t) in FB_SPRITE_TINTS {
        if k.eq_ignore_ascii_case(key) {
            return t;
        }
    }
    MUZZLE_TINT_RGB
}

#[inline]
pub(crate) fn rgb_to_norm(rgb: (u8, u8, u8)) -> BoostRgb {
    [
        rgb.0 as f32 / 255.0,
        rgb.1 as f32 / 255.0,
        rgb.2 as f32 / 255.0,
    ]
}

#[inline]
pub(crate) fn boost_max(b: BoostRgb) -> f32 {
    b[0].max(b[1]).max(b[2])
}

/// Versión RGB del muzzle boost. Toma el scalar histórico y lo tinta
/// con `MUZZLE_TINT_RGB` per-canal — equivalente a "muzzle = world light
/// con tinte amarillo cálido anclada al jugador".
pub(crate) fn muzzle_boost_rgb_cam(x_cam: f32, y_cam: f32, alpha: f32) -> BoostRgb {
    let scalar = muzzle_boost_cam(x_cam, y_cam, alpha);
    if scalar <= 0.0 {
        return ZERO_BOOST;
    }
    let t = rgb_to_norm(MUZZLE_TINT_RGB);
    [scalar * t[0], scalar * t[1], scalar * t[2]]
}

// =====================================================================
// Fase 3.37 — Muzzle direccional sobre walls y planes
// =====================================================================
//
// Cuando `cfg.muzzle_brdf = true`, el muzzle se modela como una luz
// puntual emanada del jugador (origen del cam-space) y se atenúa por
// el cosine entre la normal de la superficie y la dirección a la luz.
// Las paredes oblicuas reciben menos tinte; los pisos planos lejos
// horizontalmente reciben el cosine reducido. El muzzle clásico 3.22
// (omni) sigue activo cuando el flag está off — preserva el feel
// "fogonazo que cubre todo el cono delante del jugador".
//
// La distancia y el cosine se evalúan en 3D para coincidir con el
// modelo BRDF de world lights 3.33-3.35.

/// Muzzle direccional sobre paredes. La normal del muro tiene
/// `nz=0` (paredes verticales), así que `cos = (nx·(-mx) + ny·(-my))/d_3D`.
/// Para paredes visibles tras back-face cull, `dot(normal, mid) < 0`,
/// por lo que `cos > 0` siempre — la atenuación queda en `[0.5, 1.0]`
/// salvo casos extremos.
pub(crate) fn muzzle_boost_rgb_wall_3d(
    x_surf: f32,
    y_surf: f32,
    z_surf_cam: f32,
    alpha: f32,
    wall_normal: (f32, f32),
) -> BoostRgb {
    // Fase 3.40: falloff 3D — el muzzle decae con d_3D, no d_2D, para
    // ser coherente con el cosine BRDF que ya considera el z.
    let scalar = muzzle_boost_cam_3d(x_surf, y_surf, z_surf_cam, alpha);
    if scalar <= 0.0 {
        return ZERO_BOOST;
    }
    let d2 = x_surf * x_surf + y_surf * y_surf + z_surf_cam * z_surf_cam;
    let att = if d2 < 1e-6 {
        1.0
    } else {
        let inv_d = d2.sqrt().recip();
        // Direction from surface to muzzle (origin): (-x_surf, -y_surf, -z) / d_3D.
        // Cosine with 2D wall normal (nz=0).
        let cos = (wall_normal.0 * -x_surf + wall_normal.1 * -y_surf) * inv_d;
        (0.5 + 0.5 * cos).max(WALL_RIM_AMBIENT_FLOOR)
    };
    let t = rgb_to_norm(MUZZLE_TINT_RGB);
    [scalar * t[0] * att, scalar * t[1] * att, scalar * t[2] * att]
}

/// Muzzle direccional sobre planos horizontales. La normal es `±Z` —
/// `cos = n_z · (-z_surf_cam) / d_3D`. Para floor (`n_z=+1`) con
/// `z_surf_cam < 0` (piso debajo del ojo), cos > 0 ⇒ att > 0.5; para
/// ceiling (`n_z=-1`) con `z_surf_cam > 0` (techo arriba), cos > 0
/// igual. Pisos/techos muy lejos horizontalmente quedan con cos bajo
/// (incidencia rasante).
pub(crate) fn muzzle_boost_rgb_plane_3d(
    x_surf: f32,
    y_surf: f32,
    z_surf_cam: f32,
    alpha: f32,
    n_z: f32,
) -> BoostRgb {
    // Fase 3.40: falloff 3D, mismo principio que en walls.
    let scalar = muzzle_boost_cam_3d(x_surf, y_surf, z_surf_cam, alpha);
    if scalar <= 0.0 {
        return ZERO_BOOST;
    }
    let d2 = x_surf * x_surf + y_surf * y_surf + z_surf_cam * z_surf_cam;
    let att = if d2 < 1e-6 {
        1.0
    } else {
        let inv_d = d2.sqrt().recip();
        let cos = n_z * -z_surf_cam * inv_d;
        (0.5 + 0.5 * cos).max(PLANE_RIM_AMBIENT_FLOOR)
    };
    let t = rgb_to_norm(MUZZLE_TINT_RGB);
    [scalar * t[0] * att, scalar * t[1] * att, scalar * t[2] * att]
}

/// Versión RGB del boost de world lights. Cada luz contribuye
/// `f²·PEAK·(tint/255)` per-canal, sumadas y clampeadas a
/// `MUZZLE_BOOST_PEAK` por canal.
///
/// Fase 3.29: oclusión sectorial per-light. Si una luz tiene
/// `lit_sectors = Some(set)` y `surf_sector ∉ set`, su contribución se
/// descarta — la luz quedó "encerrada" por geometría sólida del cuarto
/// que la contiene. Cuando `lit_sectors = None` (oclusión desactivada
/// o snapshot sin BSP) la luz aporta como antes, preservando el
/// comportamiento 3.27.
pub(crate) fn world_lights_boost_rgb_cam(
    x_cam: f32,
    y_cam: f32,
    surf_sector: u32,
    lights: &[WorldLight],
) -> BoostRgb {
    if lights.is_empty() {
        return ZERO_BOOST;
    }
    let r2 = WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD;
    let peak = WORLD_LIGHT_PEAK;
    let mut sum = [0.0_f32; 3];
    for l in lights {
        if let Some(set) = l.lit_sectors.as_deref() {
            if !set.contains(&surf_sector) {
                continue;
            }
        }
        let dx = x_cam - l.x_cam;
        let dy = y_cam - l.y_cam;
        let d2 = dx * dx + dy * dy;
        if d2 >= r2 {
            continue;
        }
        let f = 1.0 - d2 / r2;
        let amount = f * f * peak;
        let t = rgb_to_norm(l.tint_rgb);
        sum[0] += amount * t[0];
        sum[1] += amount * t[1];
        sum[2] += amount * t[2];
        if sum[0] >= MUZZLE_BOOST_PEAK
            && sum[1] >= MUZZLE_BOOST_PEAK
            && sum[2] >= MUZZLE_BOOST_PEAK
        {
            return [MUZZLE_BOOST_PEAK; 3];
        }
    }
    [
        sum[0].min(MUZZLE_BOOST_PEAK),
        sum[1].min(MUZZLE_BOOST_PEAK),
        sum[2].min(MUZZLE_BOOST_PEAK),
    ]
}

// =====================================================================
// Fase 3.30 — Rim direccional del arma
// =====================================================================
//
// El psprite del jugador es un sprite 2D, pero conceptualmente "mira"
// hacia adelante (+X_cam). Una luz frontal debería tintarlo a plena
// intensidad; una luz detrás del jugador no la "ve" la cara visible
// del arma — sólo aportaría el bounce ambiente del cuarto. Modelamos
// esto con una atenuación cosine entre la "fake normal" del psprite
// (+X_cam) y la dirección normalizada a cada luz. El piso ambient
// (`WEAPON_RIM_AMBIENT_FLOOR`) representa el bounce indirecto: una
// antorcha estrictamente detrás todavía contribuye un poco vía las
// paredes del cuarto, en lugar de cortar a 0.

/// Piso ambient del rim direccional. Una luz detrás del jugador
/// igual aporta este fracción del cosine — modela el bounce
/// indirecto de paredes/techo. 0.0 = corte hard, 1.0 = sin atenuar.
pub(crate) const WEAPON_RIM_AMBIENT_FLOOR: f32 = 0.3;

/// Boost RGB para el rim del arma con atenuación direccional opcional.
/// `directional = false` ⇒ idéntico a `world_lights_boost_rgb_cam(0, 0,
/// player_sec, lights)` — backwards-compat con 3.28/3.29.
/// `directional = true` ⇒ cada luz se escala por
/// `att = max(AMBIENT_FLOOR, 0.5 + 0.5·cos(θ))` donde `θ` es el ángulo
/// entre la fake-normal del psprite (+X_cam, hacia adelante) y la
/// dirección unitaria a la luz. Luces frontales (cos=1) ⇒ att=1.0;
/// laterales (cos=0) ⇒ att=0.5; traseras (cos=-1) ⇒ att=AMBIENT_FLOOR.
/// Una luz exactamente en la posición del jugador (d≈0) se trata como
/// frontal — el cosine no está definido y el caso límite "abrazado por
/// la luz" merece full intensity.
///
/// Fase 3.41: la distancia y la normalización del cosine pasan a 3D.
/// El psprite vive efectivamente en el eye-level del jugador (overlay
/// 2D sobre el viewport), entonces el sample point vertical es `z=0`.
/// Una antorcha alta a `(50, 0, 60)` queda con `d_3D=78`, `cos=50/78=0.64`,
/// vs el cálculo 2D que daba `cos=1` (full). El radio también es 3D —
/// una luz remota verticalmente queda fuera del rim aunque su XY sea
/// chico. Compat 3.30 cuando todas las luces tienen `z_cam=0`.
pub(crate) fn weapon_rim_boost_rgb_cam(
    player_sec: u32,
    lights: &[WorldLight],
    directional: bool,
) -> BoostRgb {
    if !directional {
        return world_lights_boost_rgb_cam(0.0, 0.0, player_sec, lights);
    }
    if lights.is_empty() {
        return ZERO_BOOST;
    }
    let r2 = WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD;
    let peak = WORLD_LIGHT_PEAK;
    let mut sum = [0.0_f32; 3];
    for l in lights {
        if let Some(set) = l.lit_sectors.as_deref() {
            if !set.contains(&player_sec) {
                continue;
            }
        }
        // Fase 3.41: distancia 3D para falloff + cos.
        let d2 = l.x_cam * l.x_cam + l.y_cam * l.y_cam + l.z_cam * l.z_cam;
        if d2 >= r2 {
            continue;
        }
        let f = 1.0 - d2 / r2;
        // Atenuación direccional: cos(θ) = dot((+X, 0, 0), (lx, ly, lz)/|l|_3D).
        // Para |l|=0 (luz encima del player) tratamos como att=1.0 (full
        // intensity), evita NaN y cubre el caso "luz pegada al jugador".
        let att = if d2 < 1e-6 {
            1.0
        } else {
            let inv_d = d2.sqrt().recip();
            let cos_theta = l.x_cam * inv_d;
            (0.5 + 0.5 * cos_theta).max(WEAPON_RIM_AMBIENT_FLOOR)
        };
        let amount = f * f * peak * att;
        let t = rgb_to_norm(l.tint_rgb);
        sum[0] += amount * t[0];
        sum[1] += amount * t[1];
        sum[2] += amount * t[2];
    }
    [
        sum[0].min(MUZZLE_BOOST_PEAK),
        sum[1].min(MUZZLE_BOOST_PEAK),
        sum[2].min(MUZZLE_BOOST_PEAK),
    ]
}

/// Gate RGB del muzzle boost por sector. Si `lit_sectors` está activo y
/// el sector no aparece, devuelve `ZERO_BOOST`; sino pasa el boost
/// crudo. Espejo del gating scalar de 3.23.
pub(crate) fn muzzle_boost_gated_rgb(
    boost: BoostRgb,
    sector_id: u32,
    lit_sectors: Option<&HashSet<u32>>,
) -> BoostRgb {
    match lit_sectors {
        Some(lit) if !lit.contains(&sector_id) => ZERO_BOOST,
        _ => boost,
    }
}

/// Versión RGB del boost combinado: muzzle (gateado por lit_sectors)
/// + world lights (sólo radio), sumados per-canal y clampeados a
/// `MUZZLE_BOOST_PEAK` por canal. Reemplaza al `combined_boost_cam`
/// scalar en el render loop.
///
/// Fase 3.33: el render loop usa los variantes specializados
/// (`combined_boost_rgb_wall_cam`, `combined_boost_rgb_sprite_cam`,
/// `combined_boost_rgb_plane_cam`). Esta versión omni se conserva como
/// referencia para tests — los specialized con `directional=false` son
/// bit-equivalentes a ella.
#[cfg(test)]
pub(crate) fn combined_boost_rgb_cam(
    x_cam: f32,
    y_cam: f32,
    muzzle_alpha: f32,
    surf_sector: u32,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
) -> BoostRgb {
    let m = muzzle_boost_gated_rgb(
        muzzle_boost_rgb_cam(x_cam, y_cam, muzzle_alpha),
        surf_sector,
        lit_sectors,
    );
    let w = world_lights_boost_rgb_cam(x_cam, y_cam, surf_sector, world_lights);
    [
        (m[0] + w[0]).min(MUZZLE_BOOST_PEAK),
        (m[1] + w[1]).min(MUZZLE_BOOST_PEAK),
        (m[2] + w[2]).min(MUZZLE_BOOST_PEAK),
    ]
}

// =====================================================================
// Fase 3.31 — Rim direccional para mobj sprites (billboards)
// =====================================================================
//
// Generaliza el shading direccional del arma (3.30) a cualquier mobj
// sprite. La billboard "mira" siempre a la cámara — su fake-normal es
// `(-x_surf, -y_surf)/|surf|` (la dirección desde el sprite hacia el
// origen del cam-space). Una luz **entre** la cámara y el sprite cae
// del lado iluminado del billboard ⇒ tinte fuerte. Una luz **detrás**
// del sprite back-lightea: la cara visible queda oscura, con piso
// ambient para emular el bounce indirecto. La maquinaria es la misma
// del 3.30 (cos(θ) clampeado a [`SPRITE_RIM_AMBIENT_FLOOR`]) pero la
// normal y la dirección a la luz son relativas a la posición del
// sprite, no al origen.

/// Piso ambient del rim direccional para mobjs — análogo a
/// [`WEAPON_RIM_AMBIENT_FLOOR`] del 3.30. Modela el bounce indirecto:
/// una antorcha exactamente detrás del imp igual ilumina su entorno
/// y un poco rebota hacia su cara visible.
pub(crate) const SPRITE_RIM_AMBIENT_FLOOR: f32 = 0.3;

/// Boost RGB de world lights en una superficie de sprite (`x_surf`,
/// `y_surf` en cam-space) con atenuación direccional opcional. Con
/// `directional=false` cae al path omni del 3.27/3.29
/// (`world_lights_boost_rgb_cam`). Con `directional=true` cada luz se
/// escala por `att = max(SPRITE_RIM_AMBIENT_FLOOR, 0.5 + 0.5·cos(θ))`
/// donde `cos(θ) = dot(normal, dir_sprite_to_light)` y la normal es
/// `(-x_surf, -y_surf)/|surf|` (toward camera) — Vec2 con `nz=0`,
/// consistente con el billboard model (sprites flat hacia la cámara
/// regardless de pitch).
///
/// Fase 3.35: la distancia y el cosine pasan a 3D usando `z_surf_cam`
/// (sprite z relativo al ojo). Una luz alta a la misma XY del mobj
/// queda con cos menor (`d_3D > d_XY` ⇒ normalización mayor) — la
/// cara del sprite "ve" menos de su intensidad. El radio también
/// es 3D-aware: una luz a 200 u en vertical queda fuera aunque su
/// XY caiga adentro.
///
/// Casos degenerados (sprite en la cámara o luz coincidente con el
/// sprite): att=1.0 — sin NaN.
pub(crate) fn world_lights_boost_rgb_for_sprite_cam(
    x_surf: f32,
    y_surf: f32,
    z_surf_cam: f32,
    surf_sector: u32,
    lights: &[WorldLight],
    directional: bool,
) -> BoostRgb {
    if !directional {
        return world_lights_boost_rgb_cam(x_surf, y_surf, surf_sector, lights);
    }
    if lights.is_empty() {
        return ZERO_BOOST;
    }
    let s2 = x_surf * x_surf + y_surf * y_surf;
    if s2 < 1e-6 {
        // Sprite en la cámara: la billboard no tiene normal definida.
        // Degeneramos al path omni para evitar NaN — el caso "sprite
        // pegado al jugador" es raro y visualmente ya está subsumido
        // por la propia geometría del jugador.
        return world_lights_boost_rgb_cam(x_surf, y_surf, surf_sector, lights);
    }
    let inv_s = s2.sqrt().recip();
    let nx = -x_surf * inv_s;
    let ny = -y_surf * inv_s;
    let r2 = WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD;
    let peak = WORLD_LIGHT_PEAK;
    let mut sum = [0.0_f32; 3];
    for l in lights {
        if let Some(set) = l.lit_sectors.as_deref() {
            if !set.contains(&surf_sector) {
                continue;
            }
        }
        let dx = l.x_cam - x_surf;
        let dy = l.y_cam - y_surf;
        let dz = l.z_cam - z_surf_cam;
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 >= r2 {
            continue;
        }
        let f = 1.0 - d2 / r2;
        let att = if d2 < 1e-6 {
            1.0
        } else {
            let inv_d = d2.sqrt().recip();
            let cos_theta = (nx * dx + ny * dy) * inv_d;
            (0.5 + 0.5 * cos_theta).max(SPRITE_RIM_AMBIENT_FLOOR)
        };
        let amount = f * f * peak * att;
        let t = rgb_to_norm(l.tint_rgb);
        sum[0] += amount * t[0];
        sum[1] += amount * t[1];
        sum[2] += amount * t[2];
    }
    [
        sum[0].min(MUZZLE_BOOST_PEAK),
        sum[1].min(MUZZLE_BOOST_PEAK),
        sum[2].min(MUZZLE_BOOST_PEAK),
    ]
}

/// Versión sprite del boost combinado: muzzle (omni 2D, anclado al
/// jugador) + world lights direccionadas por la fake-normal del
/// billboard, con BRDF 3D (Fase 3.35). El muzzle no se direcciona
/// porque emana **del** sprite del arma — esa luz "envuelve" al mobj
/// independiente de la fake-normal.
pub(crate) fn combined_boost_rgb_sprite_cam(
    x_cam: f32,
    y_cam: f32,
    z_surf_cam: f32,
    muzzle_alpha: f32,
    surf_sector: u32,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
    directional: bool,
) -> BoostRgb {
    let m = muzzle_boost_gated_rgb(
        muzzle_boost_rgb_cam(x_cam, y_cam, muzzle_alpha),
        surf_sector,
        lit_sectors,
    );
    let w = world_lights_boost_rgb_for_sprite_cam(
        x_cam,
        y_cam,
        z_surf_cam,
        surf_sector,
        world_lights,
        directional,
    );
    [
        (m[0] + w[0]).min(MUZZLE_BOOST_PEAK),
        (m[1] + w[1]).min(MUZZLE_BOOST_PEAK),
        (m[2] + w[2]).min(MUZZLE_BOOST_PEAK),
    ]
}

// =====================================================================
// Fase 3.32 — Rim direccional para paredes (BRDF aproximado)
// =====================================================================
//
// Cada pared tiene una normal bien definida: perpendicular a la dirección
// del lineseg, orientada hacia el lado del frente — el que mira la cámara.
// El cosine de la normal contra la dirección a cada world light da la
// atenuación BRDF clásica (Lambert sin shadow term). Una antorcha en
// línea perpendicular a la pared (apuntando directo) la tinta al 100 %;
// una rasante (incidencia oblicua) al 50 %; una que efectivamente quedó
// "detrás" del plano de la pared (cuando un two-sided permite que la luz
// la alcance desde la cara opuesta) cae al piso ambient.
//
// El muzzle queda fuera del cosine — emana del jugador, y en walls que
// quedan "frente a vos" (las únicas visibles tras el back-face cull) el
// cosine sería ≥ 0 igual; agregarlo sólo dimearía las paredes oblicuas
// donde el muzzle ya está modelado en la simulación clásica como omni.
// Mantener esa convención preserva la lectura "el fogonazo cubre todo
// el cono delante del jugador".

pub(crate) const WALL_RIM_AMBIENT_FLOOR: f32 = 0.3;

/// Boost RGB de world lights en una superficie de pared (`x_surf`,
/// `y_surf` en cam-space), atenuado por el cosine entre la normal de
/// la pared (orientada toward camera) y la dirección 3D a cada luz.
/// Con `directional=false` cae al path omni 3.27/3.29 (radio 2D). La
/// normal se pasa ya en cam-space y ya orientada al frente — el caller
/// resuelve la orientación una sola vez (usando la convención de
/// back-face cull de la fase 3.0).
///
/// Fase 3.34: la distancia y el cosine se calculan en 3D, usando
/// `z_surf_cam` como cota vertical del punto de muestreo (típicamente
/// 0.0 = eye level). La normal de la pared tiene `nz=0` (vertical pura),
/// así que `cos(θ) = (nx·dx + ny·dy) / d_3D`. Una antorcha alta a la
/// misma XY que el midpoint del muro queda con cos < cos_2D porque
/// `d_3D > d_XY`. El radio también es 3D — luces remotas en z quedan
/// excluidas aunque su XY caiga dentro.
pub(crate) fn world_lights_boost_rgb_for_wall_cam(
    x_surf: f32,
    y_surf: f32,
    z_surf_cam: f32,
    surf_sector: u32,
    lights: &[WorldLight],
    wall_normal: (f32, f32),
    directional: bool,
) -> BoostRgb {
    if !directional {
        return world_lights_boost_rgb_cam(x_surf, y_surf, surf_sector, lights);
    }
    if lights.is_empty() {
        return ZERO_BOOST;
    }
    let r2 = WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD;
    let peak = WORLD_LIGHT_PEAK;
    let (nx, ny) = wall_normal;
    let mut sum = [0.0_f32; 3];
    for l in lights {
        if let Some(set) = l.lit_sectors.as_deref() {
            if !set.contains(&surf_sector) {
                continue;
            }
        }
        let dx = l.x_cam - x_surf;
        let dy = l.y_cam - y_surf;
        let dz = l.z_cam - z_surf_cam;
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 >= r2 {
            continue;
        }
        let f = 1.0 - d2 / r2;
        let att = if d2 < 1e-6 {
            1.0
        } else {
            let inv_d = d2.sqrt().recip();
            let cos_theta = (nx * dx + ny * dy) * inv_d;
            (0.5 + 0.5 * cos_theta).max(WALL_RIM_AMBIENT_FLOOR)
        };
        let amount = f * f * peak * att;
        let t = rgb_to_norm(l.tint_rgb);
        sum[0] += amount * t[0];
        sum[1] += amount * t[1];
        sum[2] += amount * t[2];
    }
    [
        sum[0].min(MUZZLE_BOOST_PEAK),
        sum[1].min(MUZZLE_BOOST_PEAK),
        sum[2].min(MUZZLE_BOOST_PEAK),
    ]
}

/// Versión wall del boost combinado: muzzle (omni por default, BRDF
/// con `muzzle_brdf=true` en Fase 3.37) + world lights atenuadas por la
/// normal de la pared.
pub(crate) fn combined_boost_rgb_wall_cam(
    x_cam: f32,
    y_cam: f32,
    z_surf_cam: f32,
    muzzle_alpha: f32,
    surf_sector: u32,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
    wall_normal: (f32, f32),
    directional: bool,
    muzzle_brdf: bool,
) -> BoostRgb {
    let m_raw = if muzzle_brdf {
        muzzle_boost_rgb_wall_3d(x_cam, y_cam, z_surf_cam, muzzle_alpha, wall_normal)
    } else {
        muzzle_boost_rgb_cam(x_cam, y_cam, muzzle_alpha)
    };
    let m = muzzle_boost_gated_rgb(m_raw, surf_sector, lit_sectors);
    let w = world_lights_boost_rgb_for_wall_cam(
        x_cam,
        y_cam,
        z_surf_cam,
        surf_sector,
        world_lights,
        wall_normal,
        directional,
    );
    [
        (m[0] + w[0]).min(MUZZLE_BOOST_PEAK),
        (m[1] + w[1]).min(MUZZLE_BOOST_PEAK),
        (m[2] + w[2]).min(MUZZLE_BOOST_PEAK),
    ]
}

/// Resuelve la normal cam-space de una pared dada sus dos endpoints en
/// cam-space (`(x1, y1)`, `(x2, y2)`) y el midpoint. Devuelve la
/// componente perpendicular orientada toward camera (origen del
/// cam-space): de las dos perpendiculares posibles, pickea la que tiene
/// `dot(n, mid) < 0` (mid apunta del origen al midpoint, la normal
/// inversa apunta hacia la cámara). Devuelve `(0, 0)` si la longitud
/// del segmento es despreciable — degenerado, el caller debería caer
/// al path omni.
pub(crate) fn wall_normal_cam(x1: f32, y1: f32, x2: f32, y2: f32, mid_x: f32, mid_y: f32) -> (f32, f32) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-6 {
        return (0.0, 0.0);
    }
    let inv_len = len2.sqrt().recip();
    let (n1x, n1y) = (-dy * inv_len, dx * inv_len);
    // dot(n1, mid). Si negativo, n1 ya apunta toward camera.
    if n1x * mid_x + n1y * mid_y < 0.0 {
        (n1x, n1y)
    } else {
        (-n1x, -n1y)
    }
}

// =====================================================================
// Fase 3.33 — BRDF para pisos y techos con z exportado
// =====================================================================
//
// Los pisos y techos son los únicos elementos de la escena con normal
// **vertical** (`+Z` floor, `-Z` ceiling) — fuera del plano XY donde
// viven las world lights hasta el 3.32. Con la `z_cam` exportada al
// `WorldLight` desde el sprite (Fase 3.33), podemos calcular el cosine
// 3D: una antorcha al ras del piso ilumina el piso cercano pero apenas
// el techo (cos rasante); un proyectil BFG flotando en el aire ilumina
// ambos planos, con balance según su altura relativa al view-Z. El
// radio también pasa a 3D: una luz a 100 u horizontal y 100 u vertical
// queda a `d_3D≈141`, no `d_2D=100` — el aporte cae con el cuadrado
// del 3D real, más fiel al inverse-square que el 2D-only de 3.27.

pub(crate) const PLANE_RIM_AMBIENT_FLOOR: f32 = 0.3;

/// Boost RGB de world lights en una superficie plano-horizontal con
/// normal `±Z`. `z_surf_cam` es la altura del plano relativa al
/// `cam.view_z` (positivo arriba del ojo, negativo abajo). `n_z` =
/// `+1.0` para pisos (mirando arriba) o `-1.0` para techos (mirando
/// abajo). Cuando `directional=false`, cae al path omni 2D del
/// 3.27/3.29. Cuando `true`, usa distancia 3D para falloff + cosine
/// `n_z · dz/d_3D` para atenuar por incidencia.
pub(crate) fn world_lights_boost_rgb_for_plane_cam(
    x_surf: f32,
    y_surf: f32,
    z_surf_cam: f32,
    surf_sector: u32,
    lights: &[WorldLight],
    n_z: f32,
    directional: bool,
) -> BoostRgb {
    if !directional {
        return world_lights_boost_rgb_cam(x_surf, y_surf, surf_sector, lights);
    }
    if lights.is_empty() {
        return ZERO_BOOST;
    }
    let r2 = WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD;
    let peak = WORLD_LIGHT_PEAK;
    let mut sum = [0.0_f32; 3];
    for l in lights {
        if let Some(set) = l.lit_sectors.as_deref() {
            if !set.contains(&surf_sector) {
                continue;
            }
        }
        let dx = l.x_cam - x_surf;
        let dy = l.y_cam - y_surf;
        let dz = l.z_cam - z_surf_cam;
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 >= r2 {
            continue;
        }
        let f = 1.0 - d2 / r2;
        let att = if d2 < 1e-6 {
            1.0
        } else {
            let inv_d = d2.sqrt().recip();
            let cos_theta = n_z * dz * inv_d;
            (0.5 + 0.5 * cos_theta).max(PLANE_RIM_AMBIENT_FLOOR)
        };
        let amount = f * f * peak * att;
        let t = rgb_to_norm(l.tint_rgb);
        sum[0] += amount * t[0];
        sum[1] += amount * t[1];
        sum[2] += amount * t[2];
    }
    [
        sum[0].min(MUZZLE_BOOST_PEAK),
        sum[1].min(MUZZLE_BOOST_PEAK),
        sum[2].min(MUZZLE_BOOST_PEAK),
    ]
}

/// Versión plano del boost combinado: muzzle (omni 2D por default,
/// BRDF 3D con `muzzle_brdf=true` en Fase 3.37) + world lights con
/// BRDF 3D.
pub(crate) fn combined_boost_rgb_plane_cam(
    x_cam: f32,
    y_cam: f32,
    z_surf_cam: f32,
    muzzle_alpha: f32,
    surf_sector: u32,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
    n_z: f32,
    directional: bool,
    muzzle_brdf: bool,
) -> BoostRgb {
    let m_raw = if muzzle_brdf {
        muzzle_boost_rgb_plane_3d(x_cam, y_cam, z_surf_cam, muzzle_alpha, n_z)
    } else {
        muzzle_boost_rgb_cam(x_cam, y_cam, muzzle_alpha)
    };
    let m = muzzle_boost_gated_rgb(m_raw, surf_sector, lit_sectors);
    let w = world_lights_boost_rgb_for_plane_cam(
        x_cam,
        y_cam,
        z_surf_cam,
        surf_sector,
        world_lights,
        n_z,
        directional,
    );
    [
        (m[0] + w[0]).min(MUZZLE_BOOST_PEAK),
        (m[1] + w[1]).min(MUZZLE_BOOST_PEAK),
        (m[2] + w[2]).min(MUZZLE_BOOST_PEAK),
    ]
}

/// Suma aditivamente el boost RGB a un color base, preservando alpha.
/// Reemplaza a `apply_muzzle_tint` (scalar+yellow-fixed) en el render
/// loop. Cero boost ⇒ identidad.
pub(crate) fn apply_color_boost(c: Color, boost: BoostRgb) -> Color {
    if boost == ZERO_BOOST {
        return c;
    }
    let [r, g, b, a] = c.to_rgba8().to_u8_array();
    let add_r = (boost[0] * 255.0) as u32;
    let add_g = (boost[1] * 255.0) as u32;
    let add_b = (boost[2] * 255.0) as u32;
    Color::from_rgba8(
        (r as u32 + add_r).min(255) as u8,
        (g as u32 + add_g).min(255) as u8,
        (b as u32 + add_b).min(255) as u8,
        a,
    )
}

/// Versión RGB del tinte multiplicativo per-canal del sprite. Reemplaza
/// a `sprite_shade_with_muzzle` en el render loop. Devuelve `(shade · (1 + boost))`
/// por canal, clampeado a 1.0.
pub(crate) fn sprite_shade_with_world(shade: f32, boost: BoostRgb) -> [f32; 3] {
    [
        (shade * (1.0 + boost[0])).clamp(0.0, 1.0),
        (shade * (1.0 + boost[1])).clamp(0.0, 1.0),
        (shade * (1.0 + boost[2])).clamp(0.0, 1.0),
    ]
}

/// Deriva un par `(color, alpha)` para el overlay aditivo sobre
/// texturas (paredes + flats). El color es el boost normalizado al
/// canal más alto; el alpha escala con la magnitud del boost. Devuelve
/// `None` si el boost es despreciable (< 0.02 en cualquier canal).
pub(crate) fn overlay_color_alpha_from_boost(boost: BoostRgb) -> Option<(u8, u8, u8, u8)> {
    let m = boost_max(boost);
    if m <= 0.02 {
        return None;
    }
    let scale = 255.0 / m.max(1e-3);
    let r = (boost[0] * scale).clamp(0.0, 255.0) as u8;
    let g = (boost[1] * scale).clamp(0.0, 255.0) as u8;
    let b = (boost[2] * scale).clamp(0.0, 255.0) as u8;
    // Alpha proporcional al boost máximo, normalizado al peak del muzzle
    // para preservar la intensidad histórica del overlay.
    let alpha = (m * 180.0 / MUZZLE_BOOST_PEAK).clamp(0.0, 180.0) as u8;
    Some((r, g, b, alpha))
}

/// **Fase 3.43** — construye los color-stops del gradiente de oscuridad
/// vertical de un slab texturizado. `samples` son pares
/// `(offset 0..1, boost_scalar)` ordenados de abajo (offset 0) hacia
/// arriba (offset 1). El alpha de cada stop = `(1 - shade_iluminado)·255`
/// con `shade_iluminado = clamp(base_shade + boost_scalar)`; el color es
/// siempre negro. Vello interpola el alpha linealmente entre stops, dando
/// el gradiente continuo que reemplaza las bandas discretas de 3.42.
pub(crate) fn wall_darkness_gradient_stops(base_shade: f32, samples: &[(f32, f32)]) -> Vec<(f32, Color)> {
    samples
        .iter()
        .map(|&(off, bscalar)| {
            let lit = (base_shade + bscalar).clamp(0.0, 1.0);
            let alpha = ((1.0 - lit) * 255.0) as u8;
            (off, Color::from_rgba8(0, 0, 0, alpha))
        })
        .collect()
}

/// **Fase 3.43** — construye los color-stops del gradiente de tinte
/// vertical. `samples` son pares `(offset 0..1, boost_rgb)`. Cada stop
/// reusa la normalización de [`overlay_color_alpha_from_boost`]; los
/// stops con boost despreciable quedan transparentes (alpha 0) para no
/// cortar la continuidad del gradiente. Devuelve `None` si **ningún**
/// sample tiene tinte apreciable — en ese caso no se emite fill de tinte.
pub(crate) fn wall_tint_gradient_stops(samples: &[(f32, BoostRgb)]) -> Option<Vec<(f32, Color)>> {
    let mut any = false;
    let stops: Vec<(f32, Color)> = samples
        .iter()
        .map(|&(off, boost)| match overlay_color_alpha_from_boost(boost) {
            Some((r, g, b, a)) => {
                any = true;
                (off, Color::from_rgba8(r, g, b, a))
            }
            None => (off, Color::from_rgba8(0, 0, 0, 0)),
        })
        .collect();
    if any {
        Some(stops)
    } else {
        None
    }
}

/// **Fase 3.44** — devuelve `(idx_near, idx_far)`: los índices del
/// vértice más cercano y más lejano al observador (origen cam-space) por
/// distancia euclidiana². Eje del gradiente de profundidad de planos.
/// `None` si hay menos de 2 vértices.
pub(crate) fn plane_near_far_indices(clipped: &[(f32, f32)]) -> Option<(usize, usize)> {
    if clipped.len() < 2 {
        return None;
    }
    let (mut i_near, mut i_far) = (0usize, 0usize);
    let (mut d_near, mut d_far) = (f32::INFINITY, f32::NEG_INFINITY);
    for (i, &(x, y)) in clipped.iter().enumerate() {
        let d = x * x + y * y;
        if d < d_near {
            d_near = d;
            i_near = i;
        }
        if d > d_far {
            d_far = d;
            i_far = i;
        }
    }
    Some((i_near, i_far))
}

/// **Fase 3.45** — proyección escalar de un punto `p` sobre el eje
/// `start→end`, normalizada a `[0, 1]` (clampeada). `start` ⇒ 0,
/// `end` ⇒ 1, puntos intermedios según su proyección ortogonal. Si el
/// eje es degenerado (`start ≈ end`) devuelve 0. Usado para ubicar los
/// stops del gradiente de profundidad de planos en su offset correcto.
pub(crate) fn axis_offset(p: Point, start: Point, end: Point) -> f32 {
    let ax = end.x - start.x;
    let ay = end.y - start.y;
    let len2 = ax * ax + ay * ay;
    if len2 < 1e-9 {
        return 0.0;
    }
    let t = ((p.x - start.x) * ax + (p.y - start.y) * ay) / len2;
    t.clamp(0.0, 1.0) as f32
}

// =====================================================================
// API pública
// =====================================================================
