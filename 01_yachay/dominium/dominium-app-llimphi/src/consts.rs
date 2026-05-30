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

/// Tamaño del ring de snapshots: ~18 segundos a 11 Hz. Permite ver hacia
/// atrás un par de minutos de simulación sin pasarse en RAM (cada snapshot
/// es un `World` clonado; con grid 40×40 y ~50 lemmings, ~30 KB).
pub(crate) const SNAPSHOT_RING_CAP: usize = 200;
/// Largo del trail por lemming. Tradeoff: muy alto y la pantalla se llena
/// de motas; muy bajo y el rastro no cuenta nada. 24 a 11 Hz ≈ 2 s de
/// historia visible — coincide con el horizonte que el ojo integra.
pub(crate) const TRAIL_CAP: usize = 24;
