use super::*;

/// Pinta los polígonos de piso y techo de un subsector. El polígono se
/// construye encadenando los segs del subsector (`subsector.first_seg`,
/// `num_segs`): cada seg aporta `v1` y, el último, también su `v2`.
/// La cadena es CCW por convención BSP; cerramos directamente v2_final
/// → v1_inicial. Algunos lados pueden estar bordeados por particiones
/// BSP sin seg correspondiente y la cadena no representa el polígono
/// completo; el subsector vecino del mismo sector cubre el hueco.
/// Base sobre la que se acumula el orden BSP para los depths de planos.
/// Mucho más grande que cualquier depth euclidiano de pared o sprite
/// (los maps de Doom tienen ~3000 unidades de extensión máxima) para
/// garantizar que los planos siempre se pinten antes que walls y sprites.
pub(crate) const BSP_DEPTH_BASE: f32 = 1.0e6;

/// Devuelve, por cada subsector del snapshot, su depth de painter's
/// asignado por el orden back-to-front del árbol BSP — o `None` si el
/// subsector no fue alcanzado (no debería pasar en un BSP bien formado,
/// pero defendemos contra mapas con subtrees colgados).
///
/// El primer subsector visitado (más lejano) recibe el depth más grande;
/// el último visitado (donde está el jugador) recibe el depth más chico.
/// La painter's pinta de más-depth a menos-depth → orden BSP correcto.
pub(crate) fn compute_bsp_order_depths(snap: &SceneSnapshot) -> Vec<Option<f32>> {
    let n_subs = snap.subsectors.len();
    let mut depths: Vec<Option<f32>> = vec![None; n_subs];
    let mut traversal: Vec<u32> = Vec::with_capacity(n_subs);
    let root_child = (snap.nodes.len() - 1) as u16;
    walk_bsp(&snap.nodes, root_child, snap.player.x, snap.player.y, &mut traversal);
    let total = traversal.len();
    for (step, &ss) in traversal.iter().enumerate() {
        if let Some(slot) = depths.get_mut(ss as usize) {
            // step 0 = más lejano → depth alto; step total-1 = más cercano → depth bajo.
            *slot = Some(BSP_DEPTH_BASE + (total - step) as f32);
        }
    }
    depths
}

/// **Fase 3.13b** — rank back-to-front por subsector para el painter's
/// sort unificado de TODAS las primitivas (planos, paredes, sprites,
/// decals). El subsector más lejano recibe el rank más alto; el del
/// jugador, el más bajo (1). Subsectores no alcanzados o snapshot sin
/// BSP quedan en 0, lo que en el sort los empata con el resto y delega
/// el orden a la distancia euclidiana (comportamiento histórico).
///
/// A diferencia de [`compute_bsp_order_depths`] (que devuelve f32 con un
/// base enorme sólo para planos), esto devuelve el rank entero crudo,
/// usable como clave primaria comparable entre tipos de primitiva.
pub(crate) fn compute_bsp_ranks(snap: &SceneSnapshot) -> Vec<u32> {
    let n_subs = snap.subsectors.len();
    let mut ranks = vec![0u32; n_subs];
    if snap.nodes.is_empty() || n_subs == 0 {
        return ranks;
    }
    let mut traversal: Vec<u32> = Vec::with_capacity(n_subs);
    let root_child = (snap.nodes.len() - 1) as u16;
    walk_bsp(&snap.nodes, root_child, snap.player.x, snap.player.y, &mut traversal);
    let total = traversal.len();
    for (step, &ss) in traversal.iter().enumerate() {
        if let Some(slot) = ranks.get_mut(ss as usize) {
            // step 0 = más lejano → rank alto (pintado primero en sort desc).
            *slot = (total - step) as u32;
        }
    }
    ranks
}

/// Rank BSP del subsector que contiene `(x, y)`, o 0 si no hay BSP o el
/// punto cae fuera. Combina [`subsector_at_point`] con la tabla de
/// [`compute_bsp_ranks`]. Usado por walls/sprites/decals para asignar su
/// clave primaria de painter's sort.
pub(crate) fn bsp_rank_at(nodes: &[NodeSnap], ranks: &[u32], x: f32, y: f32) -> u32 {
    subsector_at_point(nodes, x, y)
        .and_then(|ss| ranks.get(ss as usize).copied())
        .unwrap_or(0)
}

/// Light level por default cuando no podemos determinar el sector del
/// punto consultado (mapa sin BSP, índices fuera de rango). 192 es el
/// valor "habitación tipica iluminada" de Doom — coincide con el
/// fallback de `gather_sprite` para sprites sin sector.
pub(crate) const DEFAULT_PLAYER_LIGHT: u8 = 192;

/// Devuelve el subsector que contiene el punto `(px, py)`, descendiendo
/// el árbol BSP por el lado donde cae el punto en cada partición. `None`
/// si el snapshot no tiene BSP cargado, o si el camino llega a un
/// índice fuera de rango (mapa malformado). O(log N) en BSPs balanceados.
pub(crate) fn subsector_at_point(nodes: &[NodeSnap], px: f32, py: f32) -> Option<u32> {
    if nodes.is_empty() {
        return None;
    }
    let mut cur: u16 = (nodes.len() - 1) as u16;
    loop {
        if cur & NF_SUBSECTOR != 0 {
            return Some((cur & !NF_SUBSECTOR) as u32);
        }
        let node = nodes.get(cur as usize)?;
        // Mismo signo que `walk_bsp`: side > 0 → near = children[0].
        let side = node.partition_dx * (py - node.partition_y)
            - node.partition_dy * (px - node.partition_x);
        cur = if side > 0.0 {
            node.children[0]
        } else {
            node.children[1]
        };
    }
}

/// Light level del sector donde está parado el jugador. Recorre el BSP
/// para encontrar el subsector que contiene `(player.x, player.y)`,
/// luego lee `light_level` del sector referenciado. Fallback a
/// [`DEFAULT_PLAYER_LIGHT`] si no hay BSP, o el subsector apunta fuera
/// de la lista de sectores. Usado por `draw_weapon_sprite` para tintar
/// el arma según la iluminación local (Fase 3.18).
pub(crate) fn player_sector_light(snap: &SceneSnapshot) -> u8 {
    sector_light_at(snap, snap.player.x, snap.player.y)
}

/// **Fase 3.49** — light level del sector que contiene `(px, py)`,
/// resuelto por BSP point query. Fallback a [`DEFAULT_PLAYER_LIGHT`] si
/// no hay BSP o el subsector apunta fuera de la lista de sectores.
/// Generalización de [`player_sector_light`] para iluminar decals en su
/// posición real (no la del jugador).
pub(crate) fn sector_light_at(snap: &SceneSnapshot, px: f32, py: f32) -> u8 {
    let ss_id = match subsector_at_point(&snap.nodes, px, py) {
        Some(id) => id,
        None => return DEFAULT_PLAYER_LIGHT,
    };
    let Some(ss) = snap.subsectors.get(ss_id as usize) else {
        return DEFAULT_PLAYER_LIGHT;
    };
    snap.sectors
        .get(ss.sector as usize)
        .map(|s| s.light_level)
        .unwrap_or(DEFAULT_PLAYER_LIGHT)
}

/// **Fase 3.49** — multiplica un color RGB por un factor de shade
/// `[0, 1]` (per-canal, clampeado). Oscurece el decal según la luz del
/// sector donde cae: un charco en cuarto oscuro se ve casi negro, no a
/// luz plena.
pub(crate) fn shade_rgb((r, g, b): (u8, u8, u8), shade: f32) -> (u8, u8, u8) {
    let s = shade.clamp(0.0, 1.0);
    (
        (r as f32 * s) as u8,
        (g as f32 * s) as u8,
        (b as f32 * s) as u8,
    )
}

/// Camina el árbol BSP recursivamente desde `child`, agregando los
/// subsectores hoja a `out` en orden back-to-front respecto al viewer.
///
/// `child` codifica al estilo Doom: bit 15 set = subsector, else nodo
/// interno (ver [`NF_SUBSECTOR`]).
pub(crate) fn walk_bsp(nodes: &[NodeSnap], child: u16, view_x: f32, view_y: f32, out: &mut Vec<u32>) {
    if child & NF_SUBSECTOR != 0 {
        out.push((child & !NF_SUBSECTOR) as u32);
        return;
    }
    let Some(node) = nodes.get(child as usize) else {
        return;
    };
    // Convención R_PointOnSide: side = dx·(py - y) - dy·(px - x).
    // side > 0 → viewer en el lado front (children[0]); side < 0 → back.
    let side = node.partition_dx * (view_y - node.partition_y)
        - node.partition_dy * (view_x - node.partition_x);
    let (near_child, far_child) = if side > 0.0 {
        (node.children[0], node.children[1])
    } else {
        (node.children[1], node.children[0])
    };
    // Back-to-front: visitamos el subtree lejano primero.
    walk_bsp(nodes, far_child, view_x, view_y, out);
    walk_bsp(nodes, near_child, view_x, view_y, out);
}

// =====================================================================
// Fase 3.54 — occlusion culling por subsector
// =====================================================================
//
// El renderer ordena correctamente (Fase 3.13b) pero pinta TODO: un
// subsector tapado por una pared sólida más cercana igual emite sus
// polígonos de piso/techo y sus sprites, que luego quedan cubiertos por
// overdraw. Es fill desperdiciado — el pendiente declarado en el SDD.
//
// Esto lo evita con la idea clásica de R_RenderBSPNode (cliprange /
// solidsegs) pero a granularidad de **subsector** en vez de columna,
// porque el renderer es por polígonos (vello), no por columnas. Caminamos
// el BSP front-to-back acumulando los rangos angulares ocluidos por
// paredes sólidas; un subsector cuyo span angular queda completamente
// tapado se descarta.
//
// **Conservador por diseño** — nunca descarta algo visible:
//   - Sólo cuenta como bloqueador una pared sólida one-sided (`seg.solid`)
//     con AMBOS extremos delante del near plane (sub-ocluye, jamás
//     sobre-ocluye).
//   - Sólo descarta un subsector si TODOS sus extremos están delante y su
//     span angular (con un margen de seguridad) cae dentro de lo ya
//     ocluido. Si algún extremo está detrás del near, lo deja visible.

/// Margen angular de seguridad (rad, ~3°) que se exige a la oclusión por
/// encima del span real del subsector antes de descartarlo. Cubre el caso
/// de que la cadena de segs de un subsector no represente su polígono
/// convexo completo (lados de partición BSP sin seg) y su extensión
/// angular real sea un poco mayor que la de sus vértices de seg.
const CULL_ANGLE_MARGIN: f32 = 0.05;

/// Conjunto de intervalos angulares ocluidos (rad), mantenidos disjuntos y
/// fusionados, ordenados por extremo inferior. Los ángulos viven en el
/// dominio cam-space `atan2(y_cam, x_cam)` con `x_cam > 0` ⇒ `(-π/2, π/2)`,
/// sin wraparound (sólo se insertan/consultan puntos delante de cámara).
#[derive(Default)]
pub(crate) struct OcclusionSet {
    ivals: Vec<(f32, f32)>,
}

impl OcclusionSet {
    /// Inserta `[lo, hi]` fusionando con los intervalos que toque.
    pub(crate) fn insert(&mut self, lo: f32, hi: f32) {
        if !(lo <= hi) {
            return; // NaN o invertido: ignorar.
        }
        let (mut lo, mut hi) = (lo, hi);
        let mut merged: Vec<(f32, f32)> = Vec::with_capacity(self.ivals.len() + 1);
        let mut inserted = false;
        for &(a, b) in &self.ivals {
            if b < lo {
                merged.push((a, b)); // entero a la izquierda
            } else if a > hi {
                if !inserted {
                    merged.push((lo, hi));
                    inserted = true;
                }
                merged.push((a, b)); // entero a la derecha
            } else {
                // solapa: absorber.
                lo = lo.min(a);
                hi = hi.max(b);
            }
        }
        if !inserted {
            merged.push((lo, hi));
        }
        self.ivals = merged;
    }

    /// `true` si `[lo, hi]` está íntegramente contenido en algún intervalo
    /// ocluido (como están fusionados, basta con uno solo).
    pub(crate) fn covers(&self, lo: f32, hi: f32) -> bool {
        self.ivals.iter().any(|&(a, b)| a <= lo && b >= hi)
    }
}

/// Camina el BSP front-to-back (subtree cercano primero) desde `child`,
/// agregando los subsectores hoja a `out`. Inverso de [`walk_bsp`]: el
/// primero en salir es el más cercano al viewer. Lo necesita el culling
/// para acumular bloqueadores cercanos antes de testear los lejanos.
pub(crate) fn walk_bsp_front_to_back(
    nodes: &[NodeSnap],
    child: u16,
    view_x: f32,
    view_y: f32,
    out: &mut Vec<u32>,
) {
    if child & NF_SUBSECTOR != 0 {
        out.push((child & !NF_SUBSECTOR) as u32);
        return;
    }
    let Some(node) = nodes.get(child as usize) else {
        return;
    };
    let side = node.partition_dx * (view_y - node.partition_y)
        - node.partition_dy * (view_x - node.partition_x);
    let (near_child, far_child) = if side > 0.0 {
        (node.children[0], node.children[1])
    } else {
        (node.children[1], node.children[0])
    };
    walk_bsp_front_to_back(nodes, near_child, view_x, view_y, out);
    walk_bsp_front_to_back(nodes, far_child, view_x, view_y, out);
}

/// Span angular `[min, max]` (rad, cam-space) de los segs de un subsector,
/// o `None` si: no tiene segs, o algún extremo cae detrás del near plane
/// (caso en que el span no es confiable → no descartar). El span sólo se
/// usa para *decidir si descartar*, así que devolver `None` es el lado
/// seguro (se trata como visible).
fn subsector_angular_span(
    sub: &SubsectorSnap,
    snap: &SceneSnapshot,
    cam: &Camera,
    near: f32,
) -> Option<(f32, f32)> {
    let first = sub.first_seg as usize;
    let count = sub.num_segs as usize;
    if count == 0 {
        return None;
    }
    let segs: &[SegSnap] = snap.segs.get(first..first + count)?;
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for s in segs {
        for (wx, wy) in [(s.x1, s.y1), (s.x2, s.y2)] {
            let (x_cam, y_cam) = cam.to_cam_2d(wx, wy);
            if x_cam <= near {
                return None; // extremo detrás → span no confiable.
            }
            let a = y_cam.atan2(x_cam);
            lo = lo.min(a);
            hi = hi.max(a);
        }
    }
    if lo.is_finite() && hi.is_finite() {
        Some((lo, hi))
    } else {
        None
    }
}

/// Agrega al [`OcclusionSet`] los rangos angulares de las paredes sólidas
/// (`seg.solid`) del subsector cuyos dos extremos estén delante del near.
fn add_subsector_occluders(
    sub: &SubsectorSnap,
    snap: &SceneSnapshot,
    cam: &Camera,
    near: f32,
    occ: &mut OcclusionSet,
) {
    let first = sub.first_seg as usize;
    let count = sub.num_segs as usize;
    let Some(segs) = snap.segs.get(first..first + count) else {
        return;
    };
    for s in segs {
        if !s.solid {
            continue; // portal two-sided: no bloquea visión.
        }
        let (x1, y1) = cam.to_cam_2d(s.x1, s.y1);
        let (x2, y2) = cam.to_cam_2d(s.x2, s.y2);
        if x1 <= near || x2 <= near {
            continue; // parte detrás del near → no lo usamos como bloqueador.
        }
        let a1 = y1.atan2(x1);
        let a2 = y2.atan2(x2);
        occ.insert(a1.min(a2), a1.max(a2));
    }
}

/// **Fase 3.55** — resultado del paseo de oclusión front-to-back: qué
/// subsectores y qué paredes (linedefs) quedan visibles tras descartar lo
/// tapado por muros sólidos más cercanos.
pub(crate) struct Visibility {
    /// `subs[ss] == false` ⇒ subsector tapado: saltar sus polígonos de
    /// piso/techo y los sprites cuyo punto cae en él. Indexado por subsector.
    pub subs: Vec<bool>,
    /// `walls[w] == false` ⇒ linedef `w` íntegramente tapada: saltar su
    /// `gather_wall` (slabs + strips). Indexado por índice de linedef, que
    /// coincide con el índice en [`SceneSnapshot::walls`].
    pub walls: Vec<bool>,
}

/// **Fase 3.54 + 3.55** — calcula la visibilidad de subsectores **y** de
/// paredes completas en un único paseo front-to-back del BSP. Devuelve
/// `None` si el snapshot no tiene BSP (modo stub) — el caller trata todo
/// como visible (comportamiento histórico).
///
/// El subsector se descarta si su span angular cae dentro de lo ya ocluido
/// (Fase 3.54). La pared se descarta sólo si **todos** sus segs quedaron
/// angularmente tapados por muros sólidos estrictamente más cercanos (Fase
/// 3.55): un linedef se reparte en uno o más segs entre subsectores, y
/// basta con que un seg caiga en zona visible para conservar la pared.
///
/// **Conservador por diseño** (igual que 3.54): la oclusión de cada seg se
/// testea *antes* de sumar los bloqueadores de su propio subsector, así un
/// seg nunca se ocluye por una pared a su misma profundidad; y un extremo
/// detrás del near plane vuelve el span no confiable → el seg cuenta como
/// no ocluido (lado seguro, nunca sobre-descarta).
pub(crate) fn compute_visibility(
    snap: &SceneSnapshot,
    cam: &Camera,
    near: f32,
) -> Option<Visibility> {
    if snap.nodes.is_empty() || snap.subsectors.is_empty() || snap.segs.is_empty() {
        return None;
    }
    let n_sub = snap.subsectors.len();
    let n_wall = snap.walls.len();
    let mut subs = vec![true; n_sub];
    // Conteo por pared: cuántos segs aporta y cuántos quedaron ocluidos.
    // Cull sólo cuando total > 0 && ocluidos == total.
    let mut seg_total = vec![0u32; n_wall];
    let mut seg_occ = vec![0u32; n_wall];

    let mut order: Vec<u32> = Vec::with_capacity(n_sub);
    let root = (snap.nodes.len() - 1) as u16;
    walk_bsp_front_to_back(&snap.nodes, root, cam.px, cam.py, &mut order);

    let mut occ = OcclusionSet::default();
    for &ss in &order {
        let Some(sub) = snap.subsectors.get(ss as usize) else {
            continue;
        };
        // 1. Visibilidad del subsector contra lo ya ocluido por subsectores
        //    más cercanos. Sólo descartar con span confiable + margen.
        if let Some((lo, hi)) = subsector_angular_span(sub, snap, cam, near) {
            if occ.covers(lo - CULL_ANGLE_MARGIN, hi + CULL_ANGLE_MARGIN) {
                if let Some(slot) = subs.get_mut(ss as usize) {
                    *slot = false;
                }
            }
        }
        // 2. Oclusión por seg → pared. Antes de aportar los bloqueadores de
        //    este subsector (ver nota de conservadurismo en el docstring).
        let first = sub.first_seg as usize;
        let count = sub.num_segs as usize;
        if let Some(seg_slice) = snap.segs.get(first..first + count) {
            for s in seg_slice {
                let w = s.linedef as usize;
                if w >= n_wall {
                    continue; // linedef fuera de rango (sentinel) → no cuenta.
                }
                seg_total[w] += 1;
                if let Some((lo, hi)) = seg_angular_span(s, cam, near) {
                    if occ.covers(lo - CULL_ANGLE_MARGIN, hi + CULL_ANGLE_MARGIN) {
                        seg_occ[w] += 1;
                    }
                }
            }
        }
        // 3. Aportar las paredes sólidas de este subsector como nuevos
        //    bloqueadores para los subsectores que vienen detrás.
        add_subsector_occluders(sub, snap, cam, near, &mut occ);
    }

    let walls = (0..n_wall)
        .map(|w| !(seg_total[w] > 0 && seg_occ[w] == seg_total[w]))
        .collect();
    Some(Visibility { subs, walls })
}

/// **Fase 3.54** — variante histórica que devuelve sólo la visibilidad de
/// subsectores. Delega en [`compute_visibility`]; se conserva por los tests
/// y como API simple para callers que no necesitan el culling de paredes.
pub(crate) fn compute_visible_subsectors(
    snap: &SceneSnapshot,
    cam: &Camera,
    near: f32,
) -> Option<Vec<bool>> {
    compute_visibility(snap, cam, near).map(|v| v.subs)
}

/// Span angular `[min, max]` (rad, cam-space) de un único seg, o `None` si
/// algún extremo cae detrás del near plane (span no confiable → el seg se
/// trata como no ocluido, lado seguro). Análogo a
/// [`subsector_angular_span`] pero para un seg suelto.
fn seg_angular_span(s: &SegSnap, cam: &Camera, near: f32) -> Option<(f32, f32)> {
    let (x1, y1) = cam.to_cam_2d(s.x1, s.y1);
    let (x2, y2) = cam.to_cam_2d(s.x2, s.y2);
    if x1 <= near || x2 <= near {
        return None;
    }
    let a1 = y1.atan2(x1);
    let a2 = y2.atan2(x2);
    Some((a1.min(a2), a1.max(a2)))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn gather_subsector_planes(
    out: &mut Vec<Renderable>,
    sub: &SubsectorSnap,
    snap: &SceneSnapshot,
    cam: &Camera,
    proj: &Projection,
    rect: &PaintRect,
    cfg: &RenderConfig,
    bsp_depth_override: Option<f32>,
    bsp_rank: u32,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
) {
    if sub.num_segs < 2 {
        return;
    }
    let Some(sec) = snap.sectors.get(sub.sector as usize) else {
        return;
    };
    let first = sub.first_seg as usize;
    let count = sub.num_segs as usize;
    let end = first + count;
    if end > snap.segs.len() {
        return;
    }
    let seg_slice = &snap.segs[first..end];

    // Construir polígono mundial: v1 de cada seg + v2 del último.
    let mut world: Vec<(f32, f32)> = Vec::with_capacity(count + 1);
    for s in seg_slice {
        world.push((s.x1, s.y1));
    }
    // Cerrar con v2 del último seg sólo si difiere del primer v1
    // (algunos subsectores ya cierran naturalmente).
    let last_v2 = (seg_slice[count - 1].x2, seg_slice[count - 1].y2);
    let first_v1 = world[0];
    if (last_v2.0 - first_v1.0).abs() > 0.01 || (last_v2.1 - first_v1.1).abs() > 0.01 {
        world.push(last_v2);
    }

    // Transformar a cámara 2D.
    let cam_poly: Vec<(f32, f32)> = world
        .iter()
        .map(|&(x, y)| cam.to_cam_2d(x, y))
        .collect();

    // Clip contra near plane (X_cam >= near).
    let clipped = clip_near(&cam_poly, cfg.near);
    if clipped.len() < 3 {
        return;
    }

    // Necesitamos las world (x, y) en paralelo con el camera-space para
    // poder construir la affine image→screen del flat. El clip near
    // pudo introducir vértices intermedios sin world coords reales —
    // los recuperamos por inversa: world = cam.px + cos·x_cam - sin·y_cam
    //                              world_y = cam.py + sin·x_cam + cos·y_cam
    let cam_to_world = |cx: f32, cy: f32| -> (f32, f32) {
        (
            cam.px + cx * cam.cos_pa - cy * cam.sin_pa * -1.0 + cy * cam.sin_pa - cy * cam.sin_pa,
            cam.py,
        )
    };
    let _ = cam_to_world; // bypass — refactorizamos a un método del Camera.

    let world_xy: Vec<(f32, f32)> = clipped.iter().map(|&(cx, cy)| cam.from_cam_2d(cx, cy)).collect();

    // Centroide euclidiano del polígono en cámara — necesario para
    // calcular el shade (fog + light dropoff) que depende de la distancia
    // real al observador, no del BSP order.
    let (centroid_cx, centroid_cy) = {
        let (mut cx_sum, mut cy_sum) = (0.0_f32, 0.0_f32);
        for &(x, y) in &clipped {
            cx_sum += x;
            cy_sum += y;
        }
        let n = clipped.len() as f32;
        (cx_sum / n, cy_sum / n)
    };
    let shade_depth = (centroid_cx * centroid_cx + centroid_cy * centroid_cy).sqrt();
    // Depth para painter's sort:
    // - Con BSP (Fase 3.13), usamos el depth asignado por la travesía
    //   back-to-front del árbol — orden correcto Doom, elimina glitches
    //   del sort euclidiano cuando dos subsectores comparten centroide
    //   pero tienen prioridad distinta (escaleras, sectores interpenetrados).
    // - Sin BSP (stub, mapa no cargado), euclidiano como Fase 3.7+.
    let depth = bsp_depth_override.unwrap_or(shade_depth);

    let screen_x_min = rect.x as f64;
    let screen_x_max = (rect.x + rect.w) as f64;
    let screen_y_min = rect.y as f64;
    let screen_y_max = (rect.y + rect.h) as f64;

    // Proyecta todos los vértices a screen a una altura z dada y
    // devuelve `(path, screen_points)` — o `None` si está fuera de rect.
    let project_polygon = |z_world: f32| -> Option<(BezPath, Vec<Point>)> {
        let z_cam = z_world - cam.view_z;
        let pts: Vec<Point> = clipped
            .iter()
            .map(|&(x, y)| proj.project(x, y, z_cam))
            .collect();
        let all_left = pts.iter().all(|p| p.x < screen_x_min);
        let all_right = pts.iter().all(|p| p.x > screen_x_max);
        let all_above = pts.iter().all(|p| p.y < screen_y_min);
        let all_below = pts.iter().all(|p| p.y > screen_y_max);
        if all_left || all_right || all_above || all_below {
            return None;
        }
        let mut p = BezPath::new();
        p.move_to(pts[0]);
        for pt in &pts[1..] {
            p.line_to(*pt);
        }
        p.close_path();
        Some((p, pts))
    };

    // Helper común para emitir un plano (piso o techo) con o sin tex.
    let mut emit_plane = |z_world: f32, pic_idx: u16, is_floor: bool| {
        let Some((path, screen_pts)) = project_polygon(z_world) else {
            return;
        };
        // Fase 3.33: boost específico del plano. Normal `+Z` para floor,
        // `-Z` para ceiling — la luz de un proyectil al ras del piso
        // ilumina el piso pero queda rasante para el techo. Cuando
        // `plane_rim_directional` está off, cae al path omni 3.27/3.29
        // (igual aporte para floor y ceiling).
        let z_surf_cam = z_world - cam.view_z;
        let n_z = if is_floor { 1.0 } else { -1.0 };
        let boost_rgb = combined_boost_rgb_plane_cam(
            centroid_cx,
            centroid_cy,
            z_surf_cam,
            cfg.muzzle_glow_alpha,
            sub.sector,
            lit_sectors,
            world_lights,
            n_z,
            cfg.plane_rim_directional,
            cfg.muzzle_brdf,
        );
        let boost_scalar = boost_max(boost_rgb);
        // Intentar texturizar: tenemos atlas + flat resolves a RGBA.
        if let Some(atlas) = cfg.atlas.as_ref() {
            if let Some(rgba) = atlas.flat_rgba(pic_idx) {
                // Per-triangle fan: triangulamos el polígono convexo
                // del subsector desde el vértice 0 (fan(0, j, j+1)).
                // Cada triángulo individual es perspective-correct
                // porque sus 3 vértices determinan exactamente una
                // affine — sin aproximación de "spread-out 3 picks"
                // de 3.7. Subsectores son convexos por BSP definition,
                // y el clip near (Sutherland-Hodgman) preserva la
                // convexidad, así que el fan es válido. Triángulos
                // colineales o degenerados (post-clip) se saltan.
                let n_v = world_xy.len();
                if n_v >= 3 {
                    use llimphi_ui::llimphi_raster::peniko::{
                        Blob, Extend, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
                    };
                    let img = Image::new(ImageData { data: Blob::from((*rgba).clone()), format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: supay_wad::FLAT_SIZE as u32, height: supay_wad::FLAT_SIZE as u32 })
                    .with_extend(Extend::Repeat);
                    let mut any_drawn = false;
                    for j in 1..n_v - 1 {
                        let (i0, i1, i2) = (0, j, j + 1);
                        if let Some(xform) = solve_floor_affine(
                            world_xy[i0],
                            screen_pts[i0],
                            world_xy[i1],
                            screen_pts[i1],
                            world_xy[i2],
                            screen_pts[i2],
                        ) {
                            let mut tri = BezPath::new();
                            tri.move_to(screen_pts[i0]);
                            tri.line_to(screen_pts[i1]);
                            tri.line_to(screen_pts[i2]);
                            tri.close_path();
                            out.push(Renderable {
                                bsp_rank,
                                depth: depth + 1.0,
                                color: Color::WHITE,
                                path: tri,
                                kind: RenderKind::TexturedWall {
                                    image: img.clone(),
                                    brush_xform: xform,
                                },
                            });
                            any_drawn = true;
                        }
                    }
                    if any_drawn {
                        let base_factor = if is_floor { 0.92 } else { 0.85 };
                        // Fase 3.44: gradiente de profundidad near→far.
                        // En lugar de un overlay uniforme al centroide,
                        // muestreamos fog + boost en el vértice más
                        // cercano y el más lejano al jugador, y dejamos
                        // que Vello interpole. La parte del piso a tus
                        // pies queda más clara (menos fog + pool de luz);
                        // la lejana, más oscura. Reusa los helpers del
                        // gradiente vertical de walls (3.43).
                        if cfg.plane_depth_gradient {
                            if let Some((i_near, i_far)) = plane_near_far_indices(&clipped) {
                                use llimphi_ui::llimphi_raster::peniko::Gradient;
                                let start = screen_pts[i_near];
                                let end = screen_pts[i_far];
                                // Fase 3.45: muestreamos fog + boost en
                                // *cada* vértice del polígono (más el
                                // centroide), proyectando su posición en
                                // pantalla sobre el eje near→far para
                                // obtener el offset del stop. Así el
                                // gradiente captura la variación de luz
                                // intermedia (un proyectil a mitad del
                                // piso, una esquina más iluminada) en
                                // lugar de interpolar linealmente sólo
                                // entre los dos extremos (3.44).
                                let sample_at = |vx: f32, vy: f32| -> (f32, BoostRgb) {
                                    let vdepth = (vx * vx + vy * vy).sqrt();
                                    let vb = combined_boost_rgb_plane_cam(
                                        vx,
                                        vy,
                                        z_surf_cam,
                                        cfg.muzzle_glow_alpha,
                                        sub.sector,
                                        lit_sectors,
                                        world_lights,
                                        n_z,
                                        cfg.plane_rim_directional,
                                        cfg.muzzle_brdf,
                                    );
                                    let vshade =
                                        shade_for(sec.light_level, vdepth, cfg) * base_factor;
                                    // lit-shade completo; el helper de
                                    // oscuridad recibe base 0 ⇒ alpha =
                                    // (1 - lit)·255.
                                    let lit = (vshade + boost_max(vb)).clamp(0.0, 1.0);
                                    (lit, vb)
                                };
                                // (offset, lit, boost) por vértice +
                                // centroide.
                                let mut raw: Vec<(f32, f32, BoostRgb)> =
                                    Vec::with_capacity(clipped.len() + 1);
                                for (i, &(vx, vy)) in clipped.iter().enumerate() {
                                    let off = axis_offset(screen_pts[i], start, end);
                                    let (lit, vb) = sample_at(vx, vy);
                                    raw.push((off, lit, vb));
                                }
                                // Centroide (offset por su proyección).
                                let c_screen = proj.project(
                                    centroid_cx,
                                    centroid_cy,
                                    z_world - cam.view_z,
                                );
                                let c_off = axis_offset(c_screen, start, end);
                                let (c_lit, c_vb) = sample_at(centroid_cx, centroid_cy);
                                raw.push((c_off, c_lit, c_vb));
                                // Orden por offset + dedup (Vello exige
                                // offsets no decrecientes; colapsamos los
                                // casi-iguales para evitar stops cero-ancho).
                                raw.sort_by(|a, b| {
                                    a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                                });
                                let mut dark: Vec<(f32, f32)> = Vec::with_capacity(raw.len());
                                let mut tint: Vec<(f32, BoostRgb)> = Vec::with_capacity(raw.len());
                                let mut last_off = f32::NEG_INFINITY;
                                for (off, lit, vb) in raw {
                                    if off <= last_off + 1e-4 {
                                        continue;
                                    }
                                    last_off = off;
                                    dark.push((off, lit));
                                    tint.push((off, vb));
                                }
                                let dstops = wall_darkness_gradient_stops(0.0, &dark);
                                let dgrad = Gradient::new_linear(start, end)
                                    .with_stops(dstops.as_slice());
                                out.push(Renderable {
                                bsp_rank,
                                    depth: depth + 0.999,
                                    color: Color::WHITE,
                                    path: path.clone(),
                                    kind: RenderKind::GradientFill { gradient: dgrad },
                                });
                                if let Some(tstops) = wall_tint_gradient_stops(&tint) {
                                    let tgrad = Gradient::new_linear(start, end)
                                        .with_stops(tstops.as_slice());
                                    out.push(Renderable {
                                bsp_rank,
                                        depth: depth + 0.998,
                                        color: Color::WHITE,
                                        path,
                                        kind: RenderKind::GradientFill { gradient: tgrad },
                                    });
                                }
                                return;
                            }
                        }
                        // Shade overlay sobre el polígono entero
                        // (shade es constante por plano — no necesita
                        // ser per-triangle). Mismo truco que walls.
                        // Usa `shade_depth` euclidiano (no `depth` BSP-derived)
                        // porque fog/light dropoff dependen de la distancia
                        // real al jugador.
                        //
                        // Fase 3.22: el muzzle boost levanta el `shade`
                        // (reduce el overlay oscuro) + emite un overlay
                        // amarillo aditivo sobre la textura.
                        let base_shade = shade_for(sec.light_level, shade_depth, cfg)
                            * base_factor;
                        let lit_shade = (base_shade + boost_scalar).clamp(0.0, 1.0);
                        if lit_shade < 0.95 {
                            let alpha = ((1.0 - lit_shade) * 255.0).clamp(0.0, 255.0) as u8;
                            out.push(Renderable {
                                bsp_rank,
                                depth: depth + 0.999,
                                color: Color::from_rgba8(0, 0, 0, alpha),
                                path: path.clone(),
                                kind: RenderKind::Fill,
                            });
                        }
                        if let Some((or, og, ob, oa)) = overlay_color_alpha_from_boost(boost_rgb) {
                            out.push(Renderable {
                                bsp_rank,
                                depth: depth + 0.998,
                                color: Color::from_rgba8(or, og, ob, oa),
                                path,
                                kind: RenderKind::Fill,
                            });
                        }
                        return;
                    }
                }
            }
        }
        // Fallback al color promedio (3.3 behavior).
        let color = if is_floor {
            floor_color(sec, shade_depth, cfg)
        } else {
            ceiling_color(sec, shade_depth, cfg, snap.sky_pic)
        };
        out.push(Renderable {
                                bsp_rank,
            depth: depth + 1.0,
            color: apply_color_boost(color, boost_rgb),
            path,
            kind: RenderKind::Fill,
        });
    };

    emit_plane(sec.floor_height, sec.floor_pic, true);

    let is_sky = snap.sky_pic != NO_SKY_PIC && sec.ceiling_pic == snap.sky_pic;
    if !is_sky {
        emit_plane(sec.ceiling_height, sec.ceiling_pic, false);
    }
}

/// Resuelve la affine `image (wx, wy) → screen (sx, sy)` a partir de 3
/// pares de correspondencias. Devuelve `None` si los 3 vértices están
/// near-colineales en world space (determinante ~0).
pub(crate) fn solve_floor_affine(
    w0: (f32, f32),
    s0: Point,
    w1: (f32, f32),
    s1: Point,
    w2: (f32, f32),
    s2: Point,
) -> Option<Affine> {
    let dw1x = (w1.0 - w0.0) as f64;
    let dw1y = (w1.1 - w0.1) as f64;
    let dw2x = (w2.0 - w0.0) as f64;
    let dw2y = (w2.1 - w0.1) as f64;
    let det_w = dw1x * dw2y - dw2x * dw1y;
    if det_w.abs() < 1e-3 {
        return None;
    }
    let ds1x = s1.x - s0.x;
    let ds1y = s1.y - s0.y;
    let ds2x = s2.x - s0.x;
    let ds2y = s2.y - s0.y;
    let a = (ds1x * dw2y - ds2x * dw1y) / det_w;
    let c = (dw1x * ds2x - ds1x * dw2x) / det_w;
    let e = s0.x - a * w0.0 as f64 - c * w0.1 as f64;
    let b = (ds1y * dw2y - ds2y * dw1y) / det_w;
    let d = (dw1x * ds2y - ds1y * dw2x) / det_w;
    let f = s0.y - b * w0.0 as f64 - d * w0.1 as f64;
    if !a.is_finite() || !b.is_finite() || !c.is_finite() || !d.is_finite() {
        return None;
    }
    Some(Affine::new([a, b, c, d, e, f]))
}

/// Sutherland-Hodgman para un único plano `X_cam >= near` en 2D
/// (paralelo al eje Y_cam). Vértices con `x < near` se descartan; las
/// aristas que cruzan el plano se intersectan parámetricamente.
pub(crate) fn clip_near(poly: &[(f32, f32)], near: f32) -> Vec<(f32, f32)> {
    if poly.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(poly.len() + 2);
    let n = poly.len();
    for i in 0..n {
        let curr = poly[i];
        let prev = poly[if i == 0 { n - 1 } else { i - 1 }];
        let curr_in = curr.0 >= near;
        let prev_in = prev.0 >= near;
        match (prev_in, curr_in) {
            (true, true) => out.push(curr),
            (true, false) => {
                let t = (near - prev.0) / (curr.0 - prev.0);
                let yi = prev.1 + (curr.1 - prev.1) * t;
                out.push((near, yi));
            }
            (false, true) => {
                let t = (near - prev.0) / (curr.0 - prev.0);
                let yi = prev.1 + (curr.1 - prev.1) * t;
                out.push((near, yi));
                out.push(curr);
            }
            (false, false) => {}
        }
    }
    out
}

/// **Fase 3.53** — Sutherland-Hodgman contra un semiplano `n·(p − a) ≥ 0`
/// en 2D mundo. Se mantienen los vértices del lado positivo de la normal
/// `n` (no necesita ser unitaria); las aristas que cruzan el borde se
/// intersectan paramétricamente. Usado para recortar el charco horizontal
/// a las paredes que lo bordean.
pub(crate) fn clip_half_plane(poly: &[(f32, f32)], a: (f32, f32), n: (f32, f32)) -> Vec<(f32, f32)> {
    if poly.is_empty() {
        return Vec::new();
    }
    let dist = |p: (f32, f32)| n.0 * (p.0 - a.0) + n.1 * (p.1 - a.1);
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(poly.len() + 2);
    let len = poly.len();
    for i in 0..len {
        let curr = poly[i];
        let prev = poly[if i == 0 { len - 1 } else { i - 1 }];
        let dc = dist(curr);
        let dp = dist(prev);
        let lerp = |t: f32| (prev.0 + (curr.0 - prev.0) * t, prev.1 + (curr.1 - prev.1) * t);
        match (dp >= 0.0, dc >= 0.0) {
            (true, true) => out.push(curr),
            (true, false) => out.push(lerp(dp / (dp - dc))),
            (false, true) => {
                out.push(lerp(dp / (dp - dc)));
                out.push(curr);
            }
            (false, false) => {}
        }
    }
    out
}

/// **Fase 3.53** — recorta el polígono del charco (en XY mundo) a las
/// paredes que efectivamente alcanza, manteniendo siempre el lado donde
/// está el centro. Cada pared cuyo punto más cercano al centro cae dentro
/// del radio `r` aporta un semiplano (su línea infinita, normal orientada
/// hacia el centro). El resultado es la intersección convexa local — una
/// mancha de sangre junto a un muro deja de treparlo o cruzar al cuarto
/// vecino. Las paredes que el charco no toca no recortan. Sin paredes,
/// devuelve el polígono intacto (modo stub ⇒ comportamiento 3.48).
pub(crate) fn clip_decal_to_walls(
    quad: &[(f32, f32)],
    walls: &[WallSeg],
    cx: f32,
    cy: f32,
    r: f32,
) -> Vec<(f32, f32)> {
    let mut poly = quad.to_vec();
    let r2 = r * r;
    for w in walls {
        let dx = w.x2 - w.x1;
        let dy = w.y2 - w.y1;
        let len2 = dx * dx + dy * dy;
        if len2 < 1e-6 {
            continue;
        }
        // Punto más cercano del segmento al centro: sólo las paredes que
        // el charco realmente alcanza recortan (evita que la línea de un
        // muro lejano corte en cuartos no convexos).
        let t = (((cx - w.x1) * dx + (cy - w.y1) * dy) / len2).clamp(0.0, 1.0);
        let px = w.x1 + t * dx;
        let py = w.y1 + t * dy;
        if (px - cx) * (px - cx) + (py - cy) * (py - cy) > r2 {
            continue;
        }
        // Normal del muro orientada hacia el centro.
        let mut n = (-dy, dx);
        if n.0 * (cx - w.x1) + n.1 * (cy - w.y1) < 0.0 {
            n = (dy, -dx);
        }
        poly = clip_half_plane(&poly, (w.x1, w.y1), n);
        if poly.len() < 3 {
            break;
        }
    }
    poly
}

// =====================================================================
// Sprites + sombras (Fase 3.21)
// =====================================================================
