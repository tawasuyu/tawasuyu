//! El mundo: grilla + lemmings, y las 6 acciones atómicas fijas.
//!
//! Cualquier "profesión" o "rol" del macro es sólo un Lemming ejecutando
//! una de estas 6 acciones en un entorno específico.

use crate::conceptos::Conceptos;
use crate::grid::Grid;
use crate::lemmings::{Lemmings, PSI_CORRUPTIBILIDAD, PSI_CURIOSIDAD, PSI_MIEDO, PSI_ORDEN};
use crate::params::{SimParams, TradeTarget};
use serde::{Deserialize, Serialize};

/// Selecciona el byte de acción que maximiza `action_weights · psi`. Tie-break
/// determinista por menor índice. Devuelve el byte en `0..=5`.
///
/// Esta función es la mecánica matemática de [`ActionPolicy::PsiArgmax`]: sin
/// RNG, sin softmax, sin libm. Cualquier sintonía de pesos produce el mismo
/// resultado en x86 y ARM porque sólo hay multiplicaciones y sumas `f32` en
/// orden fijo.
pub fn select_action_argmax(psi: &[f32; 4], weights: &[[f32; 4]; 6]) -> u8 {
    let mut best_idx: u8 = 0;
    let mut best_score: f32 = f32::MIN;
    for (a, w) in weights.iter().enumerate() {
        let s = w[0] * psi[0] + w[1] * psi[1] + w[2] * psi[2] + w[3] * psi[3];
        if s > best_score {
            best_score = s;
            best_idx = a as u8;
        }
    }
    best_idx
}

/// Variante Big Five de [`select_action_argmax`]. Suma al score la
/// contribución de la 5ª dimensión `psi5` ponderada por `weights_ext`.
/// Tie-break determinista por menor índice — idéntico al motor Big Four
/// cuando `psi5 == 0` y `weights_ext == [0; 6]`.
pub fn select_action_argmax_big5(
    psi: &[f32; 4],
    psi5: f32,
    weights: &[[f32; 4]; 6],
    weights_ext: &[f32; 6],
) -> u8 {
    let mut best_idx: u8 = 0;
    let mut best_score: f32 = f32::MIN;
    for a in 0..6 {
        let w = &weights[a];
        let s = w[0] * psi[0] + w[1] * psi[1] + w[2] * psi[2] + w[3] * psi[3]
            + weights_ext[a] * psi5;
        if s > best_score {
            best_score = s;
            best_idx = a as u8;
        }
    }
    best_idx
}

/// Las 6 acciones atómicas. El byte `accion` del Lemming es uno de estos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Action {
    /// Lee gradientes vecinos, se mueve hacia el óptimo, gasta energía.
    Mover = 0,
    /// Resta de la celda actual, suma a su energía, degrada el suelo.
    Extraer = 1,
    /// Acerca su `vector_psi` a los campos de la celda actual.
    Sincronizar = 2,
    /// Transfiere energía al vecino más cercano.
    Intercambiar = 3,
    /// Gasta energía para instanciar un Lemming hijo (edad 0).
    Replicar = 4,
    /// Resta energía al vecino más cercano y absorbe una fracción.
    Degradar = 5,
}

impl Action {
    /// Convierte el byte discriminador. `None` si está fuera de rango.
    pub fn from_u8(b: u8) -> Option<Action> {
        match b {
            0 => Some(Action::Mover),
            1 => Some(Action::Extraer),
            2 => Some(Action::Sincronizar),
            3 => Some(Action::Intercambiar),
            4 => Some(Action::Replicar),
            5 => Some(Action::Degradar),
            _ => None,
        }
    }
}

/// Cachés recalculados **una vez por tick** para volver O(N) la fase de
/// acciones (en vez del O(N²) que produce escanear toda la población por
/// agente). No es estado del mundo — es scratch derivado, por eso queda
/// fuera de serde (`#[serde(skip)]` en `World`) y se reconstruye al inicio
/// de cada tick con [`World::rebuild_tick_ctx`].
///
/// - `poorest_idx`: el agente con menor energía del mundo es un objetivo
///   **global único** → se calcula 1× y todos los traders `Poorest` lo
///   reusan, en lugar de un `lemmings.poorest()` O(N) por agente.
/// - `occupancy` + `occ_nx`: grilla de ocupación por bloque
///   `density_block × density_block`. La densidad local del bloque de un
///   agente se lee en O(1) para gatear la réplica (capacidad de carga).
#[derive(Debug, Clone, Default)]
pub struct TickCtx {
    /// Índice del lemming más pobre del tick (objetivo global de `Poorest`).
    /// `None` cuando hay 0 o 1 agentes, o cuando aún no se reconstruyó.
    pub poorest_idx: Option<usize>,
    /// Índice del **segundo** más pobre. Se usa cuando el donante `i` ES el
    /// más pobre global: el `Lemmings::poorest(i)` histórico excluye a `i`, así
    /// que en ese caso debe donar al segundo. Cachear ambos vuelve la
    /// optimización bit-exacta al escaneo O(N) por agente. `None` si hay < 2
    /// agentes.
    pub second_poorest_idx: Option<usize>,
    /// Conteo de lemmings por bloque de densidad. Vacío si la
    /// densidad-dependencia está desactivada (`density_block == 0`).
    pub occupancy: Vec<u32>,
    /// Cantidad de bloques en X (ancho de `occupancy` / fila).
    pub occ_nx: usize,
    /// Lado del bloque en celdas (espejo de `SimParams::density_block`) — se
    /// guarda para que las lecturas no dependan de re-leer los params.
    pub occ_block: u32,
    /// Índice espacial de agentes para `nearest` en ~O(1): `bins[bid]`
    /// contiene los índices de lemming cuya celda cae en el bloque `bid` de
    /// lado `bin_block`. Vacío si el índice no está activo. Lo construye
    /// `rebuild_tick_ctx` cuando `nearest` va a estar caliente (degradar /
    /// trade Nearest). El radio de búsqueda barre el bloque propio + los 8
    /// adyacentes; si no hay candidato ahí, cae al escaneo O(N) (raro con
    /// población densa, que es justo el caso patológico que queremos domar).
    pub bins: Vec<Vec<u32>>,
    /// Cantidad de bloques en X del índice de `bins`.
    pub bin_nx: usize,
    /// Cantidad de bloques en Y del índice de `bins`.
    pub bin_ny: usize,
    /// Lado en celdas de cada bin. `0` = índice desactivado.
    pub bin_block: u32,
}

/// El estado completo de la simulación.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    pub grid: Grid,
    pub lemmings: Lemmings,
    #[serde(default)]
    pub conceptos: Conceptos,
    /// Tick global del mundo — `physics::tick` lo incrementa al final de
    /// cada paso. Es el reloj que alimenta la modulación estacional de
    /// `SimParams::season_period`. Saves viejos sin este campo arrancan
    /// en 0 vía `serde(default)`.
    #[serde(default)]
    pub tick_count: u64,
    /// Cachés por-tick para domar el O(N²). No se serializa (es scratch
    /// derivado) y se reconstruye al inicio de cada fase de acciones.
    #[serde(skip)]
    pub tick_ctx: TickCtx,
}

impl World {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            grid: Grid::new(width, height),
            lemmings: Lemmings::new(),
            conceptos: Conceptos::new(),
            tick_count: 0,
            tick_ctx: TickCtx::default(),
        }
    }

    /// Reconstruye los cachés por-tick ([`TickCtx`]) en O(N): el agente más
    /// pobre (objetivo global de `Poorest`) y la grilla de ocupación por
    /// bloque de densidad. Debe llamarse **una vez** al inicio de la fase de
    /// acciones de cada tick. Es la pieza que vuelve la fase O(N) en lugar de
    /// O(N²) (ver `act_intercambiar`/`act_replicar`).
    ///
    /// La grilla de ocupación sólo se computa si `p.density_block > 0`
    /// (densidad-dependencia activada); con `0` queda vacía y la réplica no
    /// la consulta — bit-exacto al motor histórico.
    pub fn rebuild_tick_ctx(&mut self, p: &SimParams) {
        let n = self.lemmings.len();
        // Umbral de población a partir del cual encendemos los cachés de perf
        // (más-pobre global + índice espacial). Por DEBAJO de él, las acciones
        // siguen consultando el estado VIVO (`Lemmings::poorest`/`nearest`),
        // que es bit-exacto al motor histórico — la fase O(N²) con < 256
        // agentes es trivial y no necesita domado. Por ENCIMA (el régimen que
        // colgaba), los cachés vuelven la fase O(N): el más-pobre es un valor
        // de campo-medio congelado al inicio del tick (semántica documentada,
        // ligeramente distinta del escaneo vivo, pero el objetivo —redistribuir
        // a quien estaba peor— se preserva).
        const PERF_MIN_POP: usize = 256;
        let perf_on = n >= PERF_MIN_POP;

        // ── Dos más pobres globales (1× por tick), con el mismo tie-break que
        //    `Lemmings::poorest` (estricto `<` → ante empate gana el menor
        //    índice). El segundo cubre el caso "el donante ES el más pobre":
        //    el escaneo histórico excluye a `i`, así que dona al segundo. Sólo
        //    se cachea en el régimen de perf; debajo, `None` → escaneo vivo.
        if perf_on {
            let mut p1: Option<(usize, f32)> = None;
            let mut p2: Option<(usize, f32)> = None;
            for j in 0..n {
                let e = self.lemmings.energia[j];
                if p1.map(|(_, be)| e < be).unwrap_or(true) {
                    p2 = p1;
                    p1 = Some((j, e));
                } else if p2.map(|(_, be)| e < be).unwrap_or(true) {
                    p2 = Some((j, e));
                }
            }
            self.tick_ctx.poorest_idx = p1.map(|(j, _)| j);
            self.tick_ctx.second_poorest_idx = p2.map(|(j, _)| j);
        } else {
            self.tick_ctx.poorest_idx = None;
            self.tick_ctx.second_poorest_idx = None;
        }

        // ── Ocupación por bloque (sólo si densidad-dependencia activa). ──
        // Esta grilla la consume el freno ECOLÓGICO de réplica (`local_density`).
        let block = p.density_block;
        if block == 0 {
            self.tick_ctx.occupancy.clear();
            self.tick_ctx.occ_nx = 0;
            self.tick_ctx.occ_block = 0;
        } else {
            let b = block as usize;
            let occ_nx = self.grid.width.div_ceil(b);
            let occ_ny = self.grid.height.div_ceil(b);
            let total = (occ_nx * occ_ny).max(1);
            self.tick_ctx.occupancy.clear();
            self.tick_ctx.occupancy.resize(total, 0);
            for i in 0..n {
                let (cx, cy) = self
                    .grid
                    .clamp_cell(self.lemmings.pos_x[i], self.lemmings.pos_y[i]);
                let bid = (cy / b) * occ_nx + (cx / b);
                self.tick_ctx.occupancy[bid] += 1;
            }
            self.tick_ctx.occ_nx = occ_nx;
            self.tick_ctx.occ_block = block;
        }

        // ── Índice espacial para `nearest` (domado del O(N²) de degradar). ──
        // `nearest_indexed` es BIT-EXACTO al escaneo (mismo vecino, mismo
        // tie-break), así que el índice no cambia la dinámica — sólo el costo.
        // Se construye en el mismo régimen de perf (`perf_on`); debajo, el
        // escaneo O(N) ingenuo es más barato que mantener bins. Tamaño de bin:
        // el `density_block` si está activo, si no un default sano.
        const BIN_DEFAULT: u32 = 8;
        if !perf_on {
            // Población chica → sin índice; `nearest_indexed` cae al O(N).
            self.tick_ctx.bin_block = 0;
        } else {
            let bin = if block > 0 { block as usize } else { BIN_DEFAULT as usize };
            let bin_nx = self.grid.width.div_ceil(bin);
            let bin_ny = self.grid.height.div_ceil(bin);
            let bin_total = (bin_nx * bin_ny).max(1);
            // Reusa los Vec internos para no realocar cada tick.
            if self.tick_ctx.bins.len() != bin_total {
                self.tick_ctx.bins = vec![Vec::new(); bin_total];
            } else {
                for v in self.tick_ctx.bins.iter_mut() {
                    v.clear();
                }
            }
            for i in 0..n {
                let (cx, cy) = self
                    .grid
                    .clamp_cell(self.lemmings.pos_x[i], self.lemmings.pos_y[i]);
                let bid = (cy / bin) * bin_nx + (cx / bin);
                self.tick_ctx.bins[bid].push(i as u32);
            }
            self.tick_ctx.bin_nx = bin_nx;
            self.tick_ctx.bin_ny = bin_ny;
            self.tick_ctx.bin_block = bin as u32;
        }
    }

    /// Vecino vivo más cercano a `i` usando el índice espacial del tick
    /// ([`TickCtx::bins`]). **Bit-exacto** a [`Lemmings::nearest`]: devuelve el
    /// mismo índice (incluido el tie-break por menor índice ante igual
    /// distancia), pero en ~O(K) amortizado en lugar de O(N).
    ///
    /// Expande anillos de bins desde el del agente. Tras barrer un anillo de
    /// radio `r` (en bins), si la mejor distancia hallada es ≤ la **mínima
    /// distancia posible** a cualquier bin del anillo `r+1` (que es
    /// `(r · bin_size)²` — la pared interna de ese anillo está a `r` bins de
    /// distancia del bin propio), entonces ningún agente fuera puede mejorar y
    /// el resultado es definitivo. Si el grid se agotó sin candidato, no hay
    /// vecino (N ≤ 1). Cae a `Lemmings::nearest` sólo si el índice no existe.
    fn nearest_indexed(&self, i: usize) -> Option<usize> {
        if self.tick_ctx.bin_block == 0 || self.tick_ctx.bins.is_empty() {
            return self.lemmings.nearest(i);
        }
        let b = self.tick_ctx.bin_block as f32;
        let bin = self.tick_ctx.bin_block as usize;
        let nx = self.tick_ctx.bin_nx as i64;
        let ny = self.tick_ctx.bin_ny as i64;
        let (cx, cy) = self
            .grid
            .clamp_cell(self.lemmings.pos_x[i], self.lemmings.pos_y[i]);
        let bx = (cx / bin) as i64;
        let by = (cy / bin) as i64;
        let max_r = nx.max(ny); // cota dura: cubre todo el grid
        let mut best: Option<(usize, f32)> = None;
        let mut r: i64 = 0;
        loop {
            // Barre SÓLO el borde del cuadrado de radio `r` (anillo), para no
            // re-visitar bins internos ya cubiertos en iteraciones previas.
            for nby in (by - r)..=(by + r) {
                if nby < 0 || nby >= ny {
                    continue;
                }
                for nbx in (bx - r)..=(bx + r) {
                    if nbx < 0 || nbx >= nx {
                        continue;
                    }
                    // Sólo el perímetro del anillo `r` (los internos ya se vieron).
                    let on_ring =
                        nbx == bx - r || nbx == bx + r || nby == by - r || nby == by + r;
                    if r > 0 && !on_ring {
                        continue;
                    }
                    let bid = nby as usize * self.tick_ctx.bin_nx + nbx as usize;
                    for &ju in &self.tick_ctx.bins[bid] {
                        let j = ju as usize;
                        if j == i {
                            continue;
                        }
                        let d = self.lemmings.dist2(i, j);
                        let better = match best {
                            None => true,
                            Some((bj, bd)) => d < bd || (d == bd && j < bj),
                        };
                        if better {
                            best = Some((j, d));
                        }
                    }
                }
            }
            // ¿Podemos garantizar que nada en anillos externos mejora? La
            // distancia mínima a cualquier punto del anillo `r+1` es `r · bin`
            // (la celda del agente está dentro del bin propio; el anillo r+1
            // empieza a `r` bins de pared). Si la mejor² ≤ (r·bin)², listo.
            if let Some((_, bd)) = best {
                let safe = (r as f32) * b;
                if bd <= safe * safe {
                    break;
                }
            }
            r += 1;
            if r > max_r {
                break; // se barrió todo el grid
            }
        }
        best.map(|(j, _)| j)
    }

    /// Densidad del bloque que ocupa el agente `i` (cantidad de lemmings en
    /// su bloque `density_block × density_block`). Devuelve `0` si la grilla
    /// de ocupación está vacía (densidad-dependencia desactivada).
    fn local_density(&self, i: usize) -> u32 {
        if self.tick_ctx.occ_block == 0 || self.tick_ctx.occupancy.is_empty() {
            return 0;
        }
        let b = self.tick_ctx.occ_block as usize;
        let (cx, cy) = self
            .grid
            .clamp_cell(self.lemmings.pos_x[i], self.lemmings.pos_y[i]);
        let bid = (cy / b) * self.tick_ctx.occ_nx + (cx / b);
        self.tick_ctx.occupancy.get(bid).copied().unwrap_or(0)
    }

    /// `true` si el agente `i` tiene permitido replicarse este tick según los
    /// frenos de población:
    ///
    /// - **Tope duro** (`max_population`): si la población viva ya alcanzó el
    ///   techo, nadie replica (garantía anti-cuelgue).
    /// - **Capacidad de carga local** (`density_block`/`density_cap`): si el
    ///   bloque local ya tiene `density_cap` lemmings o más, este agente no
    ///   replica — la natalidad se autolimita por hacinamiento, así `N*`
    ///   emerge sin overshoot exponencial.
    ///
    /// Con ambos frenos en su default (`0`) devuelve siempre `true` →
    /// bit-exacto al motor histórico. Cuenta de ocupación incrementada en la
    /// réplica para que múltiples saciados del mismo bloque en el mismo tick
    /// no lo desborden.
    fn replication_allowed(&self, p: &SimParams) -> bool {
        p.max_population == 0 || (self.lemmings.len() as u32) < p.max_population
    }

    /// Celda que ocupa el Lemming `i`.
    fn cell_of(&self, i: usize) -> usize {
        let (cx, cy) = self.grid.clamp_cell(self.lemmings.pos_x[i], self.lemmings.pos_y[i]);
        self.grid.idx(cx, cy)
    }

    /// Relieve físico de la celda `idx` — combinación lineal de las 5
    /// capas pesada por `p.relieve`. Es la altura que **siente** un
    /// lemming, no la que se renderiza (esa la define `ZWeights`).
    fn relieve_at(&self, idx: usize, p: &SimParams) -> f32 {
        let g = &self.grid;
        p.relieve[0] * g.materia[idx]
            + p.relieve[1] * g.psique[idx]
            + p.relieve[2] * g.poder[idx]
            + p.relieve[3] * g.oro[idx]
            + p.relieve[4] * g.degradacion[idx]
    }

    /// 0 · Mover — gravedad mental hacia el vecino más afín al `vector_psi`,
    /// penalizado por el costo de pendiente. Las "montañas" emergentes de
    /// alta `materia` o de alta `psique` (según `p.relieve`) se vuelven
    /// barreras físicas: cuesta más score subir y se paga energía extra
    /// proporcional a la altura efectivamente subida.
    pub fn act_mover(&mut self, i: usize, p: &SimParams) {
        let (cx, cy) =
            self.grid.clamp_cell(self.lemmings.pos_x[i], self.lemmings.pos_y[i]);
        let psi = self.lemmings.vector_psi[i];
        let cur_idx = self.grid.idx(cx, cy);
        let z_cur = self.relieve_at(cur_idx, p);
        let mut best_dir = (0.0f32, 0.0f32);
        let mut best_z = z_cur;
        let mut best_score = f32::MIN;
        for (dx, dy) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (cx as i64 + dx, cy as i64 + dy);
            if !self.grid.in_bounds(nx, ny) {
                continue;
            }
            let idx = self.grid.idx(nx as usize, ny as usize);
            // Orden busca materia, Miedo evita poder, Curiosidad busca
            // psique, Corruptibilidad busca oro.
            let mut score = psi[PSI_ORDEN] * self.grid.materia[idx]
                - psi[PSI_MIEDO] * self.grid.poder[idx]
                + psi[PSI_CURIOSIDAD] * self.grid.psique[idx]
                + psi[PSI_CORRUPTIBILIDAD] * self.grid.oro[idx];
            let z_n = self.relieve_at(idx, p);
            let climb = (z_n - z_cur).max(0.0);
            score -= p.climb_cost * climb;
            if score > best_score {
                best_score = score;
                best_dir = (dx as f32, dy as f32);
                best_z = z_n;
            }
        }
        let w = self.grid.width as f32 - 1.0;
        let h = self.grid.height as f32 - 1.0;
        self.lemmings.pos_x[i] =
            (self.lemmings.pos_x[i] + best_dir.0 * p.move_speed).clamp(0.0, w);
        self.lemmings.pos_y[i] =
            (self.lemmings.pos_y[i] + best_dir.1 * p.move_speed).clamp(0.0, h);
        // Costo base + costo de pendiente realmente subida.
        let climb_paid = (best_z - z_cur).max(0.0) * p.climb_cost;
        // Psi-modulación: el miedoso se cansa más al moverse (chequea el
        // entorno, vuelve, duda). Factor 1.0 cuando modulation == 0.
        let move_cost_eff =
            p.move_cost * (1.0 + p.psi_effect_modulation * 0.5 * psi[PSI_MIEDO]).max(0.0);
        self.lemmings.energia[i] -= move_cost_eff + climb_paid;
    }

    /// 1 · Extraer — vacía materia de la celda hacia la energía del agente.
    ///
    /// Psi-modulación: el agente con `psi[CORRUPTIBILIDAD]` alto saca más
    /// de la celda (y deja proporcionalmente más cicatriz). Sin modulación
    /// (factor 1.0) el comportamiento es idéntico al motor histórico.
    pub fn act_extraer(&mut self, i: usize, p: &SimParams) {
        let idx = self.cell_of(i);
        let psi = self.lemmings.vector_psi[i];
        let factor = (1.0 + p.psi_effect_modulation * psi[PSI_CORRUPTIBILIDAD]).max(0.0);
        let rate_eff = p.extract_rate * factor;
        let taken = self.grid.materia[idx].min(rate_eff).max(0.0);
        self.grid.materia[idx] -= taken;
        self.lemmings.energia[i] += taken;
        self.grid.degradacion[idx] += p.degr_per_extract * factor;
    }

    /// 2 · Sincronizar — el `vector_psi` deriva hacia los campos de la celda.
    /// Mapeo coherente con `act_mover`: ORDEN↔materia, MIEDO↔poder,
    /// CURIOSIDAD↔psique, CORRUPTIBILIDAD↔oro.
    pub fn act_sincronizar(&mut self, i: usize, p: &SimParams) {
        let idx = self.cell_of(i);
        let mut targets = [0.0f32; 4];
        targets[PSI_ORDEN] = self.grid.materia[idx];
        targets[PSI_MIEDO] = self.grid.poder[idx];
        targets[PSI_CURIOSIDAD] = self.grid.psique[idx];
        targets[PSI_CORRUPTIBILIDAD] = self.grid.oro[idx];
        for k in 0..4 {
            let v = self.lemmings.vector_psi[i][k];
            self.lemmings.vector_psi[i][k] = v + (targets[k] - v) * p.sync_rate;
        }
    }

    /// 3 · Intercambiar — transfiere energía a otro agente. El destinatario
    /// depende de `p.trade_target`: `Nearest` mantiene la semántica original
    /// (vecino físico más cercano), `Poorest` redistribuye al más necesitado
    /// del mundo. La elección controla si el sistema alcanza un punto fijo
    /// `N* > 0` o se extingue por desigualdad creciente.
    pub fn act_intercambiar(&mut self, i: usize, p: &SimParams) {
        let target = match p.trade_target {
            TradeTarget::Nearest => self.nearest_indexed(i),
            // `Poorest` busca al más pobre EXCLUYENDO al donante. Se cachean
            // los dos más pobres globales 1× por tick (`rebuild_tick_ctx`),
            // así esto es O(1) en vez del escaneo O(N) por agente (la mitad
            // del O(N²)/tick) — y BIT-EXACTO al `Lemmings::poorest(i)`
            // histórico: si el donante es el más pobre, dona al segundo.
            // Fallback al escaneo si la caché no fue construida (llamadas
            // sueltas fuera del tick, p.ej. tests unitarios → poorest_idx None).
            TradeTarget::Poorest => match self.tick_ctx.poorest_idx {
                Some(j) if j != i => Some(j),
                Some(_) => self.tick_ctx.second_poorest_idx,
                None => self.lemmings.poorest(i),
            },
        };
        let Some(j) = target else { return };
        // Psi-modulación: el ordenado comparte, el corruptible retiene.
        // Factor clamp ≥ 0 — un psi extremo en CORRUPTIBILIDAD puede
        // anular el intercambio pero no invertirlo (eso sería robo, que
        // no es la semántica de `act_intercambiar`).
        let psi = self.lemmings.vector_psi[i];
        let factor =
            (1.0 + p.psi_effect_modulation * (psi[PSI_ORDEN] - psi[PSI_CORRUPTIBILIDAD])).max(0.0);
        let amount = (p.trade_amount * factor).min(self.lemmings.energia[i]).max(0.0);
        self.lemmings.energia[i] -= amount;
        self.lemmings.energia[j] += amount;
    }

    /// 4 · Replicar — instancia un hijo con edad 0 en una celda **vecina**
    /// (no la misma del padre). El hijo hereda la acción y el `vector_psi`.
    ///
    /// Dispersión determinista: la dirección del hijo viene de
    /// `(edad_padre + idx_padre) % 4`, así N hijos del mismo padre se
    /// reparten en las 4 vecinas. Sin esta dispersión, los hijos saturan
    /// la celda del padre, agotan la materia local y colapsan en cascada
    /// — incluso con regrowth + costo metabólico activos.
    ///
    /// La herencia + dispersión + side-effect de abundancia (ver
    /// `step_lemming`) son las tres piezas que dan al sistema un punto
    /// fijo `N* > 0`.
    pub fn act_replicar(&mut self, i: usize, p: &SimParams) {
        // Freno de población (defaults = sin freno → motor histórico):
        //   1. Tope duro `max_population` — red de seguridad anti-cuelgue.
        //   2. Capacidad de carga local `density_cap` — si el bloque que
        //      ocupa el agente ya está hacinado, no nace nadie acá. Es el
        //      mecanismo que hace emerger `N*` sin overshoot exponencial.
        if !self.replication_allowed(p) {
            return;
        }
        if p.density_block > 0 && p.density_cap > 0 && self.local_density(i) >= p.density_cap {
            return;
        }
        let psi = self.lemmings.vector_psi[i];
        // Psi-modulación: el ordenado baja su umbral de reproducción
        // (forma familia antes). Clamp inferior a 0.1·threshold para
        // evitar reproducción explosiva con psi extremos.
        let thr_factor = (1.0 - p.psi_effect_modulation * 0.3 * psi[PSI_ORDEN]).max(0.1);
        let thr_eff = p.replicate_threshold * thr_factor;
        if self.lemmings.energia[i] <= thr_eff {
            return;
        }
        let cost = self.lemmings.energia[i] * p.child_energy_frac;
        self.lemmings.energia[i] -= cost;
        let accion = self.lemmings.accion[i];
        // Dirección de dispersión: 0=E, 1=O, 2=S, 3=N. Determinista por
        // (edad + i) — distribuye los hijos sucesivos en las 4 vecinas.
        let dir = (self.lemmings.edad[i].wrapping_add(i as u32) & 0x3) as u8;
        let (dx, dy) = match dir {
            0 => (1.0, 0.0),
            1 => (-1.0, 0.0),
            2 => (0.0, 1.0),
            _ => (0.0, -1.0),
        };
        let max_x = self.grid.width as f32 - 1.0;
        let max_y = self.grid.height as f32 - 1.0;
        let x = (self.lemmings.pos_x[i] + dx).clamp(0.0, max_x);
        let y = (self.lemmings.pos_y[i] + dy).clamp(0.0, max_y);
        // El hijo hereda el psi5 del padre — sin esto, el linaje Big Five
        // se borraría a cada generación.
        let psi5 = self.lemmings.psi5_at(i);
        let child = self.lemmings.spawn_big5(x, y, cost, psi, psi5);
        self.lemmings.accion[child] = accion;
        // Mantener los cachés del tick consistentes con la población VIVA —
        // el motor histórico consulta `nearest`/`poorest` sobre la población
        // completa, incluidos los hijos nacidos antes en el mismo tick. Sin
        // este mantenimiento incremental, los cachés (congelados al inicio del
        // tick) divergirían de la semántica histórica.
        let (cx, cy) = self.grid.clamp_cell(x, y);
        // (a) Ocupación de densidad: cuenta al recién nacido en su bloque, así
        //     varios saciados del MISMO bloque en el MISMO tick no superan el
        //     `density_cap` antes de que el conteo se recalcule el próximo tick.
        if self.tick_ctx.occ_block > 0 && !self.tick_ctx.occupancy.is_empty() {
            let b = self.tick_ctx.occ_block as usize;
            let bid = (cy / b) * self.tick_ctx.occ_nx + (cx / b);
            if let Some(c) = self.tick_ctx.occupancy.get_mut(bid) {
                *c += 1;
            }
        }
        // (b) Índice espacial: el hijo entra a su bin para que `nearest` de
        //     agentes que actúen después lo vea (igual que el escaneo O(N)).
        if self.tick_ctx.bin_block > 0 && !self.tick_ctx.bins.is_empty() {
            let b = self.tick_ctx.bin_block as usize;
            let bid = (cy / b) * self.tick_ctx.bin_nx + (cx / b);
            if let Some(v) = self.tick_ctx.bins.get_mut(bid) {
                v.push(child as u32);
            }
        }
        // (c) Top-2 más pobres: el hijo nace con `cost` de energía y puede ser
        //     el nuevo más pobre. Inserción incremental con el mismo tie-break
        //     (estricto `<` → menor índice gana ante empate; el hijo tiene el
        //     índice más alto, así que sólo desplaza con `<`).
        if self.tick_ctx.poorest_idx.is_some() {
            let ce = self.lemmings.energia[child];
            let p1e = self.tick_ctx.poorest_idx.map(|j| self.lemmings.energia[j]);
            if p1e.map(|e| ce < e).unwrap_or(true) {
                self.tick_ctx.second_poorest_idx = self.tick_ctx.poorest_idx;
                self.tick_ctx.poorest_idx = Some(child);
            } else {
                let p2e = self.tick_ctx.second_poorest_idx.map(|j| self.lemmings.energia[j]);
                if p2e.map(|e| ce < e).unwrap_or(true) {
                    self.tick_ctx.second_poorest_idx = Some(child);
                }
            }
        }
    }

    /// 5 · Degradar (Pelear) — resta energía al vecino y absorbe parte.
    ///
    /// Psi-modulación: el atacante miedoso pega menos, el corruptible más.
    /// Factor `max(0, 1 + mod · (CORR − MIEDO))` — un agente cuyo MIEDO
    /// domina deja de hacer daño (pero `act_degradar` sigue ejecutándose:
    /// es la mecánica de "amago" / "huida").
    pub fn act_degradar(&mut self, i: usize, p: &SimParams) {
        // `nearest_indexed` usa el índice espacial del tick (O(1) amortizado)
        // y cae a `Lemmings::nearest` O(N) sólo si el índice no está activo
        // — preserva la semántica histórica en tests/llamadas sueltas.
        let Some(j) = self.nearest_indexed(i) else { return };
        let psi = self.lemmings.vector_psi[i];
        let factor =
            (1.0 + p.psi_effect_modulation * (psi[PSI_CORRUPTIBILIDAD] - psi[PSI_MIEDO])).max(0.0);
        let dmg_max = p.fight_damage * factor;
        let dmg = dmg_max.min(self.lemmings.energia[j]).max(0.0);
        self.lemmings.energia[j] -= dmg;
        self.lemmings.energia[i] += dmg * p.absorb_frac;
    }

    /// Despacha la acción del Lemming `i` según su byte `accion`.
    ///
    /// **Bonus de abundancia**: si `p.abundance_threshold > 0` y la
    /// energía del agente supera ese umbral, ejecuta `act_replicar` como
    /// *side-effect* ANTES de su acción principal. Esto cierra el ciclo
    /// termodinámico: cualquier agente saciado se reproduce sin abandonar
    /// su rol (un Extractor sigue extrayendo, un Trader sigue donando).
    /// Si el lemming ya está en `Replicar`, el bonus no doble-cuenta —
    /// `act_replicar` requiere que `energia > replicate_threshold` y le
    /// resta `child_energy_frac`, así que el segundo intento dentro del
    /// mismo tick muy probablemente fallará el guardia.
    pub fn step_lemming(&mut self, i: usize, p: &SimParams) {
        if p.abundance_threshold > 0.0
            && self.lemmings.hack_lock[i] == 0
            && self.lemmings.energia[i] > p.abundance_threshold
        {
            self.act_replicar(i, p);
        }
        match Action::from_u8(self.lemmings.accion[i]) {
            Some(Action::Mover) => self.act_mover(i, p),
            Some(Action::Extraer) => self.act_extraer(i, p),
            Some(Action::Sincronizar) => self.act_sincronizar(i, p),
            Some(Action::Intercambiar) => self.act_intercambiar(i, p),
            Some(Action::Replicar) => self.act_replicar(i, p),
            Some(Action::Degradar) => self.act_degradar(i, p),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_1x_lemming() -> (World, SimParams) {
        let mut w = World::new(16, 16);
        w.lemmings.spawn(8.0, 8.0, 100.0, [1.0, 0.0, 0.0, 0.0]);
        (w, SimParams::default())
    }

    #[test]
    fn nearest_indexed_matches_brute_force_when_index_active() {
        // El índice espacial debe devolver EXACTAMENTE el mismo vecino que el
        // escaneo O(N) `Lemmings::nearest`, incluido el tie-break por menor
        // índice. Poblamos > BIN_MIN_POP para que el índice se active y
        // mezclamos posiciones repetidas (dist2 = 0) para estresar empates.
        let mut w = World::new(60, 60);
        let mut s: u64 = 0x1234_5678;
        let mut rng = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 33) as f32) / (u32::MAX as f32)
        };
        for _ in 0..500 {
            let x = (rng() * 59.0).floor(); // a celdas enteras → muchos empates
            let y = (rng() * 59.0).floor();
            w.lemmings.spawn(x, y, 50.0, [0.0; 4]);
        }
        let p = SimParams::default();
        w.rebuild_tick_ctx(&p);
        assert_ne!(w.tick_ctx.bin_block, 0, "el índice debe estar activo");
        for i in 0..w.lemmings.len() {
            let exact = w.lemmings.nearest(i);
            let indexed = w.nearest_indexed(i);
            // Pueden diferir SÓLO si hay empate de distancia y ambos son
            // válidos con la MISMA distancia (el tie-break por índice los
            // hace coincidir; verificamos igualdad estricta).
            assert_eq!(
                exact, indexed,
                "i={i}: brute {exact:?} != indexed {indexed:?}"
            );
        }
    }

    #[test]
    fn action_from_u8_covers_0_to_5() {
        for b in 0..=5u8 {
            assert!(Action::from_u8(b).is_some());
        }
        assert!(Action::from_u8(6).is_none());
    }

    #[test]
    fn mover_heads_toward_higher_materia() {
        let (mut w, p) = world_1x_lemming();
        // Materia alta a la derecha de (8,8).
        let right = w.grid.idx(9, 8);
        w.grid.materia[right] = 100.0;
        let x0 = w.lemmings.pos_x[0];
        w.act_mover(0, &p);
        assert!(w.lemmings.pos_x[0] > x0, "se movió hacia la materia");
        assert!(w.lemmings.energia[0] < 100.0, "Mover cuesta energía");
    }

    #[test]
    fn extraer_drains_cell_into_agent_and_degrades() {
        let (mut w, p) = world_1x_lemming();
        let idx = w.grid.idx(8, 8);
        w.grid.materia[idx] = 10.0;
        w.act_extraer(0, &p);
        assert!(w.grid.materia[idx] < 10.0);
        assert!(w.lemmings.energia[0] > 100.0);
        assert!(w.grid.degradacion[idx] > 0.0);
    }

    #[test]
    fn replicar_spawns_child_and_costs_energy() {
        let (mut w, p) = world_1x_lemming(); // energía 100 > umbral 50
        w.act_replicar(0, &p);
        assert_eq!(w.lemmings.len(), 2);
        assert_eq!(w.lemmings.edad[1], 0);
        assert!(w.lemmings.energia[0] < 100.0);
    }

    #[test]
    fn replicar_passes_action_to_child() {
        // Sin herencia, el subgrupo "Replicador" se pierde en una generación
        // y dN/dt < 0 estructuralmente. La herencia es el fix matemático
        // que cierra el ciclo.
        let mut w = World::new(8, 8);
        let i = w.lemmings.spawn(4.0, 4.0, 100.0, [0.5, 0.5, 0.5, 0.5]);
        w.lemmings.accion[i] = 4; // Replicar
        let p = SimParams::default();
        w.act_replicar(i, &p);
        assert_eq!(w.lemmings.len(), 2);
        // El hijo (índice 1) hereda la acción 4 del padre, no la acción 0
        // que el spawn pone por default.
        assert_eq!(w.lemmings.accion[1], 4, "hijo hereda accion del padre");
        // El psi también se hereda — eso ya funcionaba.
        assert_eq!(w.lemmings.vector_psi[1], w.lemmings.vector_psi[0]);
    }

    #[test]
    fn degradar_drains_nearest_and_absorbs() {
        let mut w = World::new(16, 16);
        w.lemmings.spawn(8.0, 8.0, 50.0, [0.0; 4]);
        w.lemmings.spawn(9.0, 8.0, 50.0, [0.0; 4]);
        let p = SimParams::default();
        w.act_degradar(0, &p);
        assert!(w.lemmings.energia[1] < 50.0, "la víctima pierde energía");
        assert!(w.lemmings.energia[0] > 50.0, "el atacante absorbe");
    }

    #[test]
    fn mover_prefiere_camino_llano_sobre_subir_pendiente() {
        // Dos vecinos atractivos por materia, uno además requiere subir
        // una montaña (relieve = materia, climb_cost alto). El lemming
        // debe elegir el llano.
        let mut w = World::new(16, 16);
        let i = w.lemmings.spawn(8.0, 8.0, 100.0, [1.0, 0.0, 0.0, 0.0]);
        // Materia idéntica a ambos lados de la celda actual.
        let right = w.grid.idx(9, 8);
        let left = w.grid.idx(7, 8);
        w.grid.materia[right] = 50.0;
        w.grid.materia[left] = 50.0;
        // Pero al subir a la derecha estamos sobre un pico alto.
        // Como `relieve = materia`, el right_idx tiene z=50; el left_idx tiene
        // z=50 también. Para forzar pendiente asimétrica subimos sólo la
        // derecha:
        w.grid.materia[right] = 200.0; // pico mucho mayor
        let mut p = SimParams::default();
        p.climb_cost = 10.0; // pendiente brutalmente cara
        let x0 = w.lemmings.pos_x[i];
        w.act_mover(i, &p);
        // El pico está a la derecha; con climb_cost = 10 cuesta demasiado.
        // Cualquier movimiento que NO sea hacia +x está bien (izq, arriba
        // o abajo son todos llanos).
        assert!(w.lemmings.pos_x[i] <= x0, "no fue hacia el pico de la derecha");
    }

    #[test]
    fn mover_cobra_energia_extra_por_subir() {
        let mut w = World::new(8, 8);
        let i = w.lemmings.spawn(4.0, 4.0, 100.0, [1.0, 0.0, 0.0, 0.0]);
        // Pico a la derecha.
        let right = w.grid.idx(5, 4);
        w.grid.materia[right] = 100.0;
        // Caso A: climb_cost = 0 (sin penalty). Energy gastada = move_cost.
        let mut p = SimParams::default();
        p.climb_cost = 0.0;
        w.act_mover(i, &p);
        let after_flat = w.lemmings.energia[i];
        let lost_flat = 100.0 - after_flat;
        // Reset y repetir con climb_cost > 0.
        let mut w2 = World::new(8, 8);
        let j = w2.lemmings.spawn(4.0, 4.0, 100.0, [1.0, 0.0, 0.0, 0.0]);
        let right2 = w2.grid.idx(5, 4);
        w2.grid.materia[right2] = 100.0;
        let mut p2 = SimParams::default();
        p2.climb_cost = 0.5;
        w2.act_mover(j, &p2);
        let lost_climb = 100.0 - w2.lemmings.energia[j];
        // Sin climb_cost, el agente puede ir igual al pico (porque la
        // materia lo atrae mucho), pero pierde más energía cuando climb_cost
        // > 0 porque paga la altura subida.
        assert!(lost_climb > lost_flat, "subir con climb_cost > 0 cuesta más");
    }

    #[test]
    fn intercambiar_conserves_total_energy() {
        let mut w = World::new(16, 16);
        w.lemmings.spawn(8.0, 8.0, 30.0, [0.0; 4]);
        w.lemmings.spawn(9.0, 8.0, 30.0, [0.0; 4]);
        let p = SimParams::default();
        w.act_intercambiar(0, &p);
        let total = w.lemmings.energia[0] + w.lemmings.energia[1];
        assert!((total - 60.0).abs() < 1e-4, "la energía se conserva");
    }

    #[test]
    fn intercambiar_poorest_donates_to_the_neediest() {
        // Default: TradeTarget::Poorest. El trader (i=0) tiene E=50.
        // El más cercano (i=1) está al lado pero tiene E=49. El más pobre
        // (i=2) está lejos pero tiene E=5. Debe donar al pobre, no al
        // cercano.
        let mut w = World::new(20, 20);
        w.lemmings.spawn(2.0, 2.0, 50.0, [0.0; 4]); // 0: trader
        w.lemmings.spawn(3.0, 2.0, 49.0, [0.0; 4]); // 1: cercano, rico
        w.lemmings.spawn(18.0, 18.0, 5.0, [0.0; 4]); // 2: lejos, pobre
        let p = SimParams::default();
        let before_close = w.lemmings.energia[1];
        let before_poor = w.lemmings.energia[2];
        w.act_intercambiar(0, &p);
        assert_eq!(w.lemmings.energia[1], before_close, "no le tocó al cercano");
        assert!(w.lemmings.energia[2] > before_poor, "le donó al pobre");
    }

    #[test]
    fn intercambiar_nearest_preserves_legacy_behavior() {
        // Con TradeTarget::Nearest, el comportamiento histórico se mantiene.
        let mut w = World::new(20, 20);
        w.lemmings.spawn(2.0, 2.0, 50.0, [0.0; 4]);
        w.lemmings.spawn(3.0, 2.0, 49.0, [0.0; 4]); // cercano
        w.lemmings.spawn(18.0, 18.0, 5.0, [0.0; 4]); // pobre lejos
        let mut p = SimParams::default();
        p.trade_target = TradeTarget::Nearest;
        let before_close = w.lemmings.energia[1];
        let before_poor = w.lemmings.energia[2];
        w.act_intercambiar(0, &p);
        assert!(w.lemmings.energia[1] > before_close, "le donó al cercano");
        assert_eq!(w.lemmings.energia[2], before_poor, "no le tocó al pobre");
    }

    // ───────────────────────── Fase A: psi modula efectos ─────────────────

    #[test]
    fn psi_modulation_zero_preserves_legacy_act_extraer() {
        // Con psi_effect_modulation = 0.0 el resultado es bit-exacto al motor
        // histórico, sin importar qué psi tenga el agente. Esta es la
        // garantía de retrocompat para todo el corpus de tests preexistentes.
        let mut a = World::new(8, 8);
        let mut b = World::new(8, 8);
        let i = a.lemmings.spawn(4.0, 4.0, 100.0, [0.9, 0.0, 0.0, 0.9]);
        let j = b.lemmings.spawn(4.0, 4.0, 100.0, [0.0, 0.0, 0.0, 0.0]);
        let idx = a.grid.idx(4, 4);
        a.grid.materia[idx] = 50.0;
        b.grid.materia[idx] = 50.0;
        let p = SimParams::default(); // psi_effect_modulation == 0
        a.act_extraer(i, &p);
        b.act_extraer(j, &p);
        assert_eq!(a.lemmings.energia[i], b.lemmings.energia[j]);
        assert_eq!(a.grid.materia[idx], b.grid.materia[idx]);
        assert_eq!(a.grid.degradacion[idx], b.grid.degradacion[idx]);
    }

    #[test]
    fn corruptible_extrae_mas_y_degrada_mas() {
        // Dos agentes idénticos salvo CORRUPTIBILIDAD: el corrupto saca más
        // materia (más energía propia) y deja más cicatriz en el suelo.
        // Es la modulación canónica de Extraer.
        let mut a = World::new(8, 8); // corrupto
        let mut b = World::new(8, 8); // honesto
        let i = a.lemmings.spawn(4.0, 4.0, 0.0, [0.0, 0.0, 0.0, 1.0]);
        let j = b.lemmings.spawn(4.0, 4.0, 0.0, [0.0, 0.0, 0.0, 0.0]);
        let idx = a.grid.idx(4, 4);
        a.grid.materia[idx] = 100.0;
        b.grid.materia[idx] = 100.0;
        let mut p = SimParams::default();
        p.psi_effect_modulation = 0.8;
        a.act_extraer(i, &p);
        b.act_extraer(j, &p);
        assert!(
            a.lemmings.energia[i] > b.lemmings.energia[j],
            "corrupto sacó más: {} vs {}",
            a.lemmings.energia[i], b.lemmings.energia[j]
        );
        assert!(
            a.grid.degradacion[idx] > b.grid.degradacion[idx],
            "corrupto dejó más cicatriz"
        );
    }

    #[test]
    fn miedoso_pega_menos_en_degradar() {
        let mut a = World::new(8, 8); // miedoso
        let mut b = World::new(8, 8); // valiente
        a.lemmings.spawn(4.0, 4.0, 50.0, [0.0, 1.0, 0.0, 0.0]); // MIEDO=1
        a.lemmings.spawn(5.0, 4.0, 50.0, [0.0; 4]); // víctima
        b.lemmings.spawn(4.0, 4.0, 50.0, [0.0; 4]); // valiente
        b.lemmings.spawn(5.0, 4.0, 50.0, [0.0; 4]); // víctima
        let mut p = SimParams::default();
        p.psi_effect_modulation = 0.8;
        let e_victima_pre = a.lemmings.energia[1];
        a.act_degradar(0, &p);
        b.act_degradar(0, &p);
        let dmg_miedoso = e_victima_pre - a.lemmings.energia[1];
        let dmg_valiente = e_victima_pre - b.lemmings.energia[1];
        assert!(
            dmg_miedoso < dmg_valiente,
            "miedoso pega menos: {dmg_miedoso} < {dmg_valiente}"
        );
    }

    #[test]
    fn ordenado_comparte_mas_en_intercambiar() {
        let mut a = World::new(8, 8); // ordenado
        let mut b = World::new(8, 8); // neutral
        a.lemmings.spawn(4.0, 4.0, 50.0, [1.0, 0.0, 0.0, 0.0]); // ORDEN=1
        a.lemmings.spawn(5.0, 4.0, 1.0, [0.0; 4]); // pobre cercano
        b.lemmings.spawn(4.0, 4.0, 50.0, [0.0; 4]);
        b.lemmings.spawn(5.0, 4.0, 1.0, [0.0; 4]);
        let mut p = SimParams::default();
        p.psi_effect_modulation = 0.8;
        p.trade_target = TradeTarget::Nearest; // forzamos al cercano para test reproducible
        a.act_intercambiar(0, &p);
        b.act_intercambiar(0, &p);
        let donado_orden = a.lemmings.energia[1] - 1.0;
        let donado_base = b.lemmings.energia[1] - 1.0;
        assert!(
            donado_orden > donado_base,
            "ordenado donó más: {donado_orden} > {donado_base}"
        );
    }

    #[test]
    fn argmax_big5_se_reduce_a_big4_con_pesos_ext_cero() {
        // Sanity: con `action_weights_ext = [0; 6]` y cualquier psi5,
        // `select_action_argmax_big5` debe coincidir con la versión Big Four.
        let weights = crate::params::SimParams::default().action_weights;
        let weights_ext = [0.0f32; 6];
        let psis = [
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.5, 0.5, 0.5, 0.5],
        ];
        for psi in &psis {
            let a4 = select_action_argmax(psi, &weights);
            for psi5 in [0.0, 0.5, 1.0] {
                let a5 = select_action_argmax_big5(psi, psi5, &weights, &weights_ext);
                assert_eq!(a4, a5, "psi {:?} psi5 {}", psi, psi5);
            }
        }
    }

    #[test]
    fn argmax_big5_cambia_decision_cuando_extra_pesa() {
        // Con un peso ext alto en Intercambiar (3) y psi5 = 1.0, un agente
        // que en Big Four iría a Degradar (5) — psi=[0,0,0,1] — debería
        // saltar a Intercambiar porque la 5ª columna lo empuja.
        let mut weights_ext = [0.0f32; 6];
        weights_ext[3] = 5.0; // empujamos fuerte a Intercambiar
        let weights = crate::params::SimParams::default().action_weights;
        let psi = [0.0, 0.0, 0.0, 1.0];
        let psi5 = 1.0;
        let a = select_action_argmax_big5(&psi, psi5, &weights, &weights_ext);
        assert_eq!(a, 3, "el 5º peso debe ganarle a Degradar");
    }

    #[test]
    fn replicar_hereda_psi5_del_padre() {
        let mut w = World::new(8, 8);
        let i = w.lemmings.spawn_big5(4.0, 4.0, 100.0, [0.5; 4], 0.73);
        w.lemmings.accion[i] = 4;
        let p = SimParams::default();
        w.act_replicar(i, &p);
        assert_eq!(w.lemmings.len(), 2);
        assert!((w.lemmings.psi5_at(1) - 0.73).abs() < 1e-5, "hijo {} != 0.73", w.lemmings.psi5_at(1));
    }

    #[test]
    fn argmax_picks_action_with_highest_psi_dot_weights() {
        // psi puro en CORRUPTIBILIDAD → con los pesos por default, la
        // acción ganadora es Extraer (peso 0.8) o Degradar (peso 1.0). Como
        // Degradar tiene mayor peso para CORRUPTIBILIDAD, gana.
        let weights = crate::params::SimParams::default().action_weights;
        let psi = [0.0, 0.0, 0.0, 1.0];
        assert_eq!(select_action_argmax(&psi, &weights), 5);
        // psi puro en CURIOSIDAD → Mover (1.0) y Sincronizar (1.0) empatan
        // → gana el menor índice = Mover (0).
        let psi = [0.0, 0.0, 1.0, 0.0];
        assert_eq!(select_action_argmax(&psi, &weights), 0);
        // psi puro en ORDEN → Intercambiar (1.0) y Replicar (1.0) empatan
        // → gana el menor índice = Intercambiar (3).
        let psi = [1.0, 0.0, 0.0, 0.0];
        assert_eq!(select_action_argmax(&psi, &weights), 3);
    }
}
