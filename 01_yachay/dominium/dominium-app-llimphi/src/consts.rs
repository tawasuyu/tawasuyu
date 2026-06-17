//! Constantes de mundo y de bucle compartidas por todos los módulos de la app.

/// Lado de la grilla cuadrada del mundo. 240×240 = 57 600 celdas: continente
/// con varios biomas (mares, ríos, llanuras, sierras, picos). El motor sigue
/// siendo O(grid) en difusión y O(N²) en `nearest`, así que la población
/// arranca en miles pero limitada por los frenos termodinámicos de
/// `init`-time (ver `SimParams` override).
pub(crate) const GRID: usize = 240;
/// Población inicial de Lemmings. Miles. La densidad efectiva queda más
/// baja que en la versión 80² histórica (≈0.043 lem/celda) porque sólo
/// spawnean en tierra navegable y el motor ya no permite el crecimiento
/// exponencial sin freno.
pub(crate) const LEMMINGS: usize = 2500;
/// Periodo del bucle de simulación (~11 Hz).
pub(crate) const TICK_MS: u64 = 90;
/// Cada cuántos ticks recalculamos k-means para colorear los clusters
/// (modo PsiCluster). 30 ticks ≈ 2.7s — suficiente para ver tribus
/// emergentes sin que el costo del kmeans (O(K·N·iter)) note.
pub(crate) const KMEANS_REFRESH_TICKS: u64 = 30;
/// Ancho del panel de stats.
pub(crate) const SIDE_WIDTH: f32 = 240.0;

/// Escala iso por defecto (px por celda). El reset de cámara vuelve a este
/// valor; el zoom por rueda la multiplica dentro de `[ZOOM_MIN, ZOOM_MAX]`.
pub(crate) const CAM_SCALE_DEFAULT: f32 = 3.0;
/// z_factor iso por defecto. No cambia con el zoom, pero el reset lo restaura
/// por si algún día se toca.
pub(crate) const CAM_ZFACTOR_DEFAULT: f32 = 0.55;
/// Factor de zoom por notch de rueda (1 notch ≈ ×1.1 / ÷1.1).
pub(crate) const ZOOM_STEP: f32 = 1.1;
/// Cota inferior de `iso.scale` (no achicar más allá).
pub(crate) const ZOOM_MIN: f32 = 0.4;
/// Cota superior de `iso.scale` (no agrandar más allá).
pub(crate) const ZOOM_MAX: f32 = 16.0;
/// Radio² de agarre (en plan-coords / px de pantalla) para decidir, en el
/// press, si un drag mueve el Concepto seleccionado o panea la cámara. ~30 px
/// de radio cubre el sprite del Concepto con holgura sin "robar" pans cercanos.
pub(crate) const CONCEPT_GRAB_R2: f32 = 30.0 * 30.0;

/// Tamaño del ring de snapshots: ~18 segundos a 11 Hz. Permite ver hacia
/// atrás un par de minutos de simulación sin pasarse en RAM (cada snapshot
/// es un `World` clonado; con grid 40×40 y ~50 lemmings, ~30 KB).
pub(crate) const SNAPSHOT_RING_CAP: usize = 200;
/// Largo del trail por lemming. Tradeoff: muy alto y la pantalla se llena
/// de motas; muy bajo y el rastro no cuenta nada. 24 a 11 Hz ≈ 2 s de
/// historia visible — coincide con el horizonte que el ojo integra.
pub(crate) const TRAIL_CAP: usize = 24;
