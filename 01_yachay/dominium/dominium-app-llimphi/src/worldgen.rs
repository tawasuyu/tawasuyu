//! Generación procedural del mundo (frontend): paleta de biomas para el
//! render y el wrapper que invoca al motor `dominium_core::worldgen` con las
//! dimensiones, población y pack de Conceptos de esta app. El generador en sí
//! (PRNG, fbm, ríos, biomas, lemmings) vive en el core (regla #2).

use dominium_core::World;

use crate::consts::{GRID, LEMMINGS};
use crate::packs::{default_conceptos, load_user_pack};

/// Paleta retocada para que mar / tierra / cumbres se lean a primera
/// vista. Reemplaza la `Palette::default()` del render-plan en la app sin
/// tocar el crate (otros consumidores siguen con el default histórico).
pub(crate) fn bioma_palette() -> dominium_render_plan::Palette {
    dominium_render_plan::Palette {
        // Arena oscura para celdas sin capa dominante — visualmente
        // "tierra de borde" en lugar del gris-azulado original.
        floor: [0.30, 0.25, 0.20, 1.0],
        // Pasto firme.
        materia: [0.30, 0.62, 0.32, 1.0],
        // Azul océano profundo (sustituye al cian claro del default).
        psique: [0.16, 0.34, 0.66, 1.0],
        // Siena de cumbre (sustituye al rojo bandera).
        poder: [0.78, 0.52, 0.32, 1.0],
        oro: [0.92, 0.76, 0.28, 1.0],
        // Gris-violeta de roca alta (sustituye al violeta saturado).
        degradacion: [0.46, 0.40, 0.50, 1.0],
        // Marfil suave para lemmings — destaca sobre pasto y agua.
        lemming: [0.97, 0.95, 0.88, 1.0],
        concepto_aura: [0.95, 0.86, 0.55, 0.18],
        concepto_base: [0.58, 0.45, 0.18, 1.0],
        concepto: [0.98, 0.88, 0.42, 1.0],
        shadow: [0.04, 0.04, 0.06, 0.42],
    }
}

/// Siembra un mundo `GRID×GRID` con `LEMMINGS` lemmings. Si el usuario ya
/// tiene un pack guardado gana sobre el embebido (así sus ediciones
/// sobreviven al reseed/reapertura); si no, default. Delega en
/// [`dominium_core::worldgen::seed`].
pub(crate) fn seed(seed: u64) -> World {
    let conceptos = load_user_pack().unwrap_or_else(default_conceptos);
    dominium_core::worldgen::seed(seed, GRID, LEMMINGS, conceptos)
}
