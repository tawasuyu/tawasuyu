use super::*;

/// **Fase 3.46** — marca efímera en el mundo (scorch de bala, splat de
/// sangre). El host la detecta a partir de los sprites de impacto del
/// motor (PUFF / BLUD), la persiste con un fade y la pasa al renderer
/// por [`RenderConfig::decals`] cada frame con su `alpha` ya computado.
/// El renderer la dibuja como un billboard pequeño camera-facing,
/// z-ordenado con la escena (lo ocluyen las paredes que estén delante).
#[derive(Clone, Copy, Debug)]
pub struct Decal {
    /// Posición mundo del impacto (la del mobj PUFF/BLUD que lo originó).
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Medio-tamaño en unidades mundo del quad.
    pub radius: f32,
    /// Color RGB de la marca (scorch oscuro, sangre roja).
    pub color: (u8, u8, u8),
    /// Opacidad 0..1 — el host la decae con la edad del decal.
    pub alpha: f32,
    /// **Fase 3.47** — tangente unitaria mundo `(tx, ty)` del lineseg
    /// donde impactó: el decal yace plano sobre la pared, con su eje
    /// horizontal a lo largo de la tangente y el vertical en `+Z`. Si es
    /// `(0, 0)` (sin pared cercana — p.ej. sangre en el aire) el renderer
    /// cae al billboard camera-facing de 3.46.
    pub tangent: (f32, f32),
    /// **Fase 3.48** — el impacto fue contra piso o techo: el quad yace
    /// **horizontal** (ejes en el plano XY mundo, a `z` constante) como
    /// un charco. Tiene prioridad sobre `tangent`. `false` ⇒ pared
    /// (tangente) o billboard.
    pub horizontal: bool,
    /// **Fase 3.52** — recorte horizontal del decal de pared a su lineseg.
    /// Offsets firmados `(s_min, s_max)` en unidades mundo a lo largo de
    /// la [`Decal::tangent`], medidos desde el centro del decal hasta los
    /// dos extremos del segmento donde impactó. El renderer recorta la
    /// extensión horizontal del quad a `[s_min.max(-r), s_max.min(r)]`,
    /// evitando que el decal sangre más allá del borde de la pared (la
    /// esquina). `None` ⇒ sin recorte (billboard, charco, o pared sin
    /// span resuelto) — comportamiento 3.51.
    pub wall_span: Option<(f32, f32)>,
}

/// Parámetros del renderer.
#[derive(Clone, Debug)]
pub struct RenderConfig {
    /// Field of view vertical en grados. Doom clásico ronda 60°; el
    /// default 75° da una sensación más moderna sin perder el feel.
    pub fov_y_deg: f32,
    /// Distancia near-clip en unidades Doom. Vértices con
    /// `X_cam < near` se descartan o se clipean.
    pub near: f32,
    /// Distancia donde el fog alcanza la saturación máxima.
    pub far_fog: f32,
    /// Altura visual de los sprites en unidades Doom.
    pub sprite_height: f32,
    /// Mitad del ancho de los sprites — billboard `2·hw × sprite_height`.
    pub sprite_half_width: f32,
    /// Cantidad de bandas horizontales por slab (subdivisión vertical).
    /// Más bandas = más detalle "panel/ladrillo" a costo de rects.
    pub wall_bands: u32,
    /// Cantidad de strips horizontales por slab texturizada. Cada
    /// strip resuelve su propia affine (image→screen) — el error de
    /// perspectiva queda reducido por factor `wall_strips`. 1 = sin
    /// subdivisión (3.6 behavior). 8 = compromiso razonable. Strips
    /// adicionales cuestan O(N) image fills.
    pub wall_strips: u32,
    /// Atlas WAD con paleta + colores de flats. Sin él, el renderer cae
    /// a las paletas hardcoded de 3.1.
    pub atlas: Option<Arc<WadAtlas>>,
    /// **Fase 3.19 — crosshair central**. Si `true`, pinta una marca
    /// fina en el centro del viewport (4 chevrons + dot). Modernización
    /// pura: Doom clásico no lo usa, los FPS contemporáneos sí. Cosmético
    /// total — sólo afecta el rasterizador, no la simulación.
    pub crosshair: bool,
    /// **Fase 3.19 — fuerza de la viñeta de cabina**. `0.0` = off,
    /// `1.0` = oscurecimiento muy marcado en esquinas. Default `0.55`
    /// queda sutil: ~70/255 de alpha crimson_deep en el corner más
    /// lejano del centro. Pintada antes que el crosshair y los overlays
    /// para que las flashes de damage la cubran.
    pub vignette: f32,
    /// **Fase 3.57 — god rays volumétricos**. Fuerza del resplandor radial
    /// aditivo alrededor de cada luz del mundo (sprite full-bright:
    /// antorcha, lámpara, proyectil). `0.0` = off (sin capa, sin costo);
    /// `1.0` = halo marcado. Default `0.6`: el aire alrededor del foco se
    /// ilumina sin lavar la escena. Perceptual puro — no toca geometría ni
    /// timing. Se pinta tras la geometría y bajo el arma/overlays.
    pub god_rays: f32,
    /// **Fase 3.20 — HUD inferior**. Si `true`, pinta una banda slim al
    /// pie del viewport con health/armor/ammo/keys leídos del
    /// `PlayerStats` del snapshot. Modernización de la status bar
    /// clásica de Doom (320×32 al pie del FB): mismos datos, layout
    /// "tile-by-tile" co-locado con la imagen 3D.
    pub hud: bool,
    /// **Fase 3.21 — sombras de mobjs en el piso**. Si `true`, cada
    /// sprite proyecta un disco oscuro semi-transparente en el plano
    /// del sector donde está parado, dándole sensación de peso al
    /// mundo 3D. Cosmético total — Doom clásico no tiene sombras, el
    /// renderer software pinta sprites flotando sobre el piso.
    pub sprite_shadows: bool,
    /// **Fase 3.22 — luz dinámica del muzzle flash**. Intensidad actual
    /// (0.0 = nada, 1.0 = pico) del destello de boca de arma que
    /// ilumina el mundo alrededor del jugador. El host lo settea cada
    /// frame: pico 1.0 cuando el snapshot tiene `FF_FULLBRIGHT` activo
    /// en `weapon` o `weapon_flash`, decae a 0 en `MUZZLE_DECAY_SECS`.
    /// Aplica un boost cálido (amarillo-blanco) sobre paredes, pisos,
    /// techos y sprites dentro de `MUZZLE_RADIUS_WORLD` unidades del
    /// jugador. Doom clásico cicla la PLAYPAL completa; esta es la
    /// modernización por sector/depth.
    pub muzzle_glow_alpha: f32,
    /// **Fase 3.23 — oclusión sectorial del muzzle boost**. Si `true`,
    /// el destello del arma sólo ilumina superficies del sector donde
    /// está parado el jugador y de los sectores conectados a él por al
    /// menos una linedef two-sided (puerta, escalón abierto, ventana).
    /// Una pared sólida entre el jugador y un sprite/pared lejana corta
    /// el boost: el cuarto vecino queda oscuro aunque su distancia
    /// euclidiana esté bajo `MUZZLE_RADIUS_WORLD`. Si `false`, vuelve al
    /// comportamiento 3.22 (boost ignora paredes). En stub sin BSP el
    /// flag no aplica — el renderer ilumina todo igual.
    pub muzzle_occlusion: bool,
    /// **Fase 3.26 — luces dinámicas desde mobjs full-bright**. Si
    /// `true`, los sprites con bit `FF_FULLBRIGHT` (proyectiles,
    /// puffs, frames de explosión, fog) emiten una luz puntual cálida
    /// que ilumina paredes, pisos, techos y sprites cercanos (radio
    /// `WORLD_LIGHT_RADIUS_WORLD = 192`, mitad del muzzle). Sumadas al
    /// boost del muzzle (clamp ≤ `MUZZLE_BOOST_PEAK`). Doom clásico
    /// no irradia luz desde proyectiles — esta es modernización pura.
    /// Sin gating por oclusión: las luces son efímeras (1-30 ticks),
    /// el leak fugaz a través de paredes es invisible en práctica.
    pub world_lights_enabled: bool,
    /// **Fase 3.29 — oclusión sectorial de world lights**. Si `true`,
    /// cada world light cachea su set de sectores alcanzables por BFS
    /// desde su sector origen (mismo radio y hops del muzzle gate),
    /// y sólo aporta tinte a superficies cuyo sector está en ese set.
    /// Un BFG ball pasando en el cuarto vecino con pared sólida deja
    /// de pintar verde la pared detrás del jugador. Si `false`, las
    /// luces aportan por radio solamente — comportamiento 3.27. En
    /// stub sin BSP el flag no aplica (lit_sectors queda `None` y el
    /// boost pasa). Costo: una llamada a BFS por luz por frame, ≤ 8
    /// luces, ≤ 2 hops — despreciable.
    pub world_lights_occlusion: bool,
    /// **Fase 3.28 — rim-light del arma desde world lights**. Si `true`,
    /// el sprite del arma se tinta cada frame con el boost RGB de
    /// `world_lights` evaluado en la posición del jugador (origen del
    /// cam-space). Caminar al lado de una antorcha azul (`TBLU`) tinta
    /// la pistola apenas azulada; un fireball pasando cerca le pinta
    /// un rim rojizo. Modernización pura — Doom clásico no liga el
    /// arma al ambiente (la PLAYPAL global no llega al psprite del
    /// arma). Los frames `FF_FULLBRIGHT` (muzzle flash) saltan el
    /// boost — el destello del propio fogonazo domina y el ambiente
    /// queda subsumido. El muzzle glow del jugador *no* se suma acá
    /// (el fogonazo sale *de* la pistola, no la ilumina a ella), como
    /// en 3.22.
    pub weapon_rim_light: bool,
    /// **Fase 3.30 — rim direccional**. Si `true`, el aporte de cada
    /// world light al rim del arma se atenúa según el ángulo entre la
    /// "normal" virtual del psprite (apuntando a +X_cam, hacia adelante)
    /// y la dirección a la luz: una antorcha al frente tinta a plena
    /// intensidad; una atrás baja a un piso ambient (cosine·0.5 + 0.5
    /// clampeado a `WEAPON_RIM_AMBIENT_FLOOR`). Sin direccional
    /// (`false`), todas las luces aportan igual sin importar dónde
    /// estén — comportamiento 3.28-3.29. Sólo afecta al rim del arma;
    /// el resto de la escena conserva el path omnidireccional 3.27.
    pub weapon_rim_directional: bool,
    /// **Fase 3.31 — rim direccional de mobjs**. Si `true`, cada sprite
    /// billboard (enemigos, decoración, proyectiles) usa una fake-normal
    /// apuntando hacia la cámara para atenuar el aporte de cada world
    /// light. Una antorcha **entre** el jugador y el imp tinta su frente
    /// al 100 %; una antorcha **detrás** del imp lo back-lightea — el
    /// sprite que ve el jugador es la cara frontal, ahí queda al piso
    /// (`SPRITE_RIM_AMBIENT_FLOOR`). Sin direccional (`false`), todos
    /// los sprites reciben el aporte omnidireccional del 3.27/3.29 —
    /// backwards-compat exacta. Se aplica tanto al patch texturizado
    /// como al fallback de rectángulos coloreados. El muzzle del
    /// jugador queda fuera del shading direccional (es la luz que
    /// emite el propio sprite del arma, no hay normal de mobj que la
    /// module).
    pub sprite_rim_directional: bool,
    /// **Fase 3.32 — rim direccional para paredes**. Si `true`, cada
    /// pared usa su normal (perpendicular al lineseg, orientada toward
    /// camera) para atenuar el aporte de world lights por `cos(θ)`. Una
    /// antorcha justo frente a la pared (luz "de frente") tinta al 100 %;
    /// una al costado (rasante) cae al 50 %; una efectivamente "detrás"
    /// de la pared cae al piso `WALL_RIM_AMBIENT_FLOOR` — modela el
    /// bounce indirecto cuando una linedef two-sided permite atravesar
    /// el muzzle/world-light a un sector vecino y el ángulo de rasante
    /// queda extremo. El muzzle queda omni (como en 3.30/3.31). Cuando
    /// `false`, vuelve al path omni 3.27/3.29 — backwards-compat.
    pub wall_rim_directional: bool,
    /// **Fase 3.33 — BRDF para pisos y techos**. Si `true`, los planos
    /// horizontales reciben el aporte de cada world light atenuado por
    /// el cosine entre la normal del plano (`+Z` floor, `-Z` ceiling) y
    /// la dirección 3D del plano hacia la luz. Una antorcha al ras del
    /// piso ilumina fuerte el piso cercano pero el techo lo recibe
    /// rasante (cos ≈ 0); un mobj BFG en el aire (proyectil flotante)
    /// ilumina ambos, pero más al que tiene más cara hacia él. Combina
    /// con el radio 3D-aware: una luz a 100 u horizontal y 100 u
    /// vertical (d_3D ≈ 141) cae con `f = 1 - d²/r²` donde `d` es 3D —
    /// más realista que el 2D-only del 3.27. Cuando `false`, vuelve al
    /// path omni 3.27/3.29 con radio 2D. El muzzle queda omni
    /// (consistente con 3.30-3.32).
    pub plane_rim_directional: bool,
    /// **Fase 3.37 — muzzle direccional sobre walls y planes**. Si
    /// `true`, el muzzle flash del arma se trata como una luz puntual
    /// en el origen del cam-space y se atenúa por el cosine de la
    /// superficie (igual que el rim direccional pero para la fuente
    /// muzzle). Paredes oblicuas reciben menos tinte cálido durante
    /// el flash; pisos muy lejos del jugador horizontalmente reciben
    /// el cosine reducido por el ángulo bajo. Mobjs y weapon siguen
    /// con muzzle omni — el psprite es overlay 2D sin geometría 3D
    /// y los mobjs reciben el muzzle "envolvente" característico de
    /// Doom clásico. Cuando `false` (default), preserva el path
    /// omni 3.30-3.35 — backwards-compat exacta. Sólo afecta walls
    /// y floors/ceilings.
    pub muzzle_brdf: bool,
    /// **Fase 3.42 — bandas verticales para BRDF de walls**. Número de
    /// sub-bandas horizontales sobre cada slab texturizado donde el
    /// overlay del shading y el tinte se calculan independientemente.
    /// Default `1` = un único overlay por slab (comportamiento 3.32-3.41).
    /// Valores > 1 emiten N overlays adicionales con boost computado al
    /// centro vertical de cada banda — una antorcha al ras del piso
    /// ilumina más la parte baja de la pared, una a la altura del
    /// techo más la parte alta. Coste: ~2N extra fills por slab
    /// texturizado. Recomendado: 2-4. Sólo afecta al path texturizado.
    pub wall_vertical_bands: u8,
    /// **Fase 3.43 — gradiente vertical continuo para walls**. Si `true`,
    /// el shading y el tinte del slab texturizado se pintan con un único
    /// `Gradient` lineal de Vello (bottom→top en pantalla) en lugar de N
    /// bandas discretas. El boost se muestrea a
    /// `wall_vertical_bands.max(2) + 1` alturas y Vello interpola suave
    /// entre stops — sin las costuras visibles de las bandas 3.42 y con
    /// **dos** fills por slab en lugar de 2N. Cuando está on tiene
    /// precedencia sobre `wall_vertical_bands` (que sólo controla la
    /// densidad de muestreo). Default `false` ⇒ comportamiento 3.42
    /// bit-equivalente. Sólo afecta al path texturizado.
    pub wall_vertical_gradient: bool,
    /// **Fase 3.44 — gradiente de profundidad para pisos/techos**. Si
    /// `true`, el shading/tinte del plano texturizado se pinta con un
    /// `Gradient` lineal de Vello a lo largo del eje near→far (vértice
    /// más cercano al jugador → vértice más lejano) en lugar de un
    /// overlay uniforme computado al centroide. El boost y el fog se
    /// muestrean en ambos extremos: la parte del piso cercana al jugador
    /// queda más clara (menos fog + más pool de luz del muzzle/proyectil)
    /// y la lejana más oscura. Reusa los mismos helpers que el gradiente
    /// vertical de walls (3.43). Default `false` ⇒ overlay uniforme 3.33
    /// bit-equivalente. Sólo afecta al path texturizado de planos.
    pub plane_depth_gradient: bool,
    /// **Fase 3.46 — decals efímeros de impacto**. Lista de marcas
    /// (scorch / sangre) que el host detecta de los sprites de impacto
    /// del motor y mantiene con su fade. Vacía por default ⇒ sin decals
    /// (modo stub, o el host no las alimenta). Se reconstruye cada frame;
    /// el renderer las dibuja como billboards camera-facing z-ordenados.
    pub decals: Vec<Decal>,
    /// **Fase 3.51 — boost direccional del decal por su normal**. Si
    /// `true`, el tinte RGB de world lights + muzzle sobre cada decal se
    /// atenúa por el cosine entre la normal de la superficie donde yace
    /// el decal y la dirección a cada luz, igual que walls/planes/sprites:
    /// un charco de piso usa BRDF de plano (`n_z=+1`), una marca pegada a
    /// la pared usa BRDF de pared (normal del lineseg toward-camera) y un
    /// billboard flotante (sangre en el aire) queda omni. Un scorch en
    /// pared rasante a una antorcha recibe menos tinte que uno encarado;
    /// un charco bajo un fireball alto recoge el cosine vertical. Cuando
    /// `false` (o sin BSP), cae al boost omni 3.50 bit-equivalente. Sólo
    /// afecta al tinte de los decals.
    pub decal_rim_directional: bool,
    /// **Fase 3.53 — recorte del charco al recinto de paredes**. Si
    /// `true`, el quad de un decal horizontal (charco de piso/techo) se
    /// recorta a las paredes (linedefs) que lo bordean dentro de su radio,
    /// manteniendo el lado del centro — una mancha de sangre junto a un
    /// muro deja de treparlo o cruzar al cuarto vecino. Cuando `false` (o
    /// sin paredes en el snapshot, modo stub) el charco se dibuja como el
    /// quad completo de 3.48. Sólo afecta a los decals `horizontal`.
    pub decal_clip_walls: bool,
    /// **Fase 3.54 — occlusion culling por subsector**. Si `true` (y hay
    /// BSP), antes de juntar primitivas se camina el árbol front-to-back
    /// acumulando los rangos angulares ocluidos por paredes sólidas
    /// one-sided; los subsectores cuyo span angular queda íntegramente
    /// tapado **no** emiten sus polígonos de piso/techo ni sus sprites.
    /// Reduce el overdraw/fill de geometría que de todas formas quedaría
    /// cubierta (el pendiente "Visibility BSP-walking" del SDD). Es
    /// conservador: sólo descarta lo que comprueba completamente tapado,
    /// nunca recorta algo visible. Las paredes no se descartan (son los
    /// propios bloqueadores). Cuando `false`, o en modo stub sin BSP, se
    /// pinta todo — comportamiento 3.53 bit-equivalente. Default `true`.
    pub occlusion_cull: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            fov_y_deg: 75.0,
            near: 4.0,
            far_fog: 2048.0,
            sprite_height: 56.0,
            sprite_half_width: 16.0,
            wall_bands: 4,
            wall_strips: 8,
            atlas: None,
            crosshair: true,
            vignette: 0.55,
            god_rays: 0.6,
            hud: true,
            sprite_shadows: true,
            muzzle_glow_alpha: 0.0,
            muzzle_occlusion: true,
            world_lights_enabled: true,
            world_lights_occlusion: true,
            weapon_rim_light: true,
            weapon_rim_directional: true,
            sprite_rim_directional: true,
            wall_rim_directional: true,
            plane_rim_directional: true,
            muzzle_brdf: false,
            wall_vertical_bands: 1,
            wall_vertical_gradient: false,
            plane_depth_gradient: false,
            decals: Vec::new(),
            decal_rim_directional: true,
            decal_clip_walls: true,
            occlusion_cull: true,
        }
    }
}

// =====================================================================
// Fase 3.22 — Muzzle world light
// =====================================================================
//
// El destello del arma del jugador (`FF_FULLBRIGHT` en `psprites[]`) emite
// un boost cálido que ilumina paredes, pisos, techos y sprites en un
// disco alrededor del jugador. Modela el "fogonazo del cañón" que en
// Doom original sólo afectaba la PLAYPAL global — acá lo hacemos
// world-light: las superficies cercanas reciben un tinte amarillento que
// decae con `distance² / RADIUS²`. La intensidad `cfg.muzzle_glow_alpha`
// viene del host y decae con el tiempo.
