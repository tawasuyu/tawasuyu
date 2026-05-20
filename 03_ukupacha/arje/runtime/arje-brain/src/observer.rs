//! Observador estadístico. Mantiene marginales y co-ocurrencias dentro de una
//! ventana deslizante. Calcula entropía de Shannon e información mutua para
//! identificar correlaciones significativas.
//!
//! Diseño:
//!   - Counters incrementales: cada `record()` es O(window_size) en el peor
//!     caso (actualiza co-ocurrencias con cada evento del window).
//!   - Sin recomputaciones globales: marginales y joint counts son state.
//!   - El cálculo de H(X), P(B|A), I(A;B) es O(|distinct events|).

use crate::rules::EventKind;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// Evento timestamped. El timestamp se conserva para futuras políticas de
/// expiración por tiempo (no sólo por count).
#[derive(Debug, Clone)]
pub struct TimedEvent {
    pub kind: EventKind,
    pub at: Instant,
}

/// Histograma de gaps temporales con buckets exponenciales en segundos.
/// Cubre 6 órdenes de magnitud: 1ms hasta 1000s.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct GapHistogram {
    /// Buckets cumulativos (Prometheus-style): cada índice cuenta eventos
    /// con gap ≤ ese límite. Limites: 1ms, 10ms, 100ms, 1s, 10s, 100s, 1000s.
    pub buckets: [u64; 7],
    pub count: u64,
    pub sum_secs: f64,
    /// Suma de cuadrados — permite calcular varianza/stddev en O(1).
    pub sum_squares_secs: f64,
    pub max_secs: f64,
}

/// Estadísticas resumidas de un GapHistogram, usables en cristales temporales.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GapStats {
    pub count: u64,
    pub mean_secs: f64,
    pub stddev_secs: f64,
    pub max_secs: f64,
}

const GAP_BUCKET_LIMITS_SECS: [f64; 7] = [
    0.001, 0.01, 0.1, 1.0, 10.0, 100.0, 1000.0,
];

impl GapHistogram {
    pub fn observe(&mut self, gap_secs: f64) {
        for (i, &limit) in GAP_BUCKET_LIMITS_SECS.iter().enumerate() {
            if gap_secs <= limit {
                self.buckets[i] += 1;
            }
        }
        self.count += 1;
        self.sum_secs += gap_secs;
        self.sum_squares_secs += gap_secs * gap_secs;
        if gap_secs > self.max_secs { self.max_secs = gap_secs; }
    }

    pub fn mean_secs(&self) -> f64 {
        if self.count == 0 { 0.0 } else { self.sum_secs / self.count as f64 }
    }

    /// Desviación estándar muestral. Computada vía `sum_squares - n*mean²`
    /// para precisión razonable sin almacenar las muestras.
    pub fn stddev_secs(&self) -> f64 {
        if self.count < 2 { return 0.0; }
        let n = self.count as f64;
        let mean = self.mean_secs();
        let var = (self.sum_squares_secs - n * mean * mean) / (n - 1.0);
        // Numerical floor: var puede ser ligeramente negativo por float ε.
        if var <= 0.0 { 0.0 } else { var.sqrt() }
    }

    pub fn stats(&self) -> GapStats {
        GapStats {
            count: self.count,
            mean_secs: self.mean_secs(),
            stddev_secs: self.stddev_secs(),
            max_secs: self.max_secs,
        }
    }

    pub fn bucket_limits() -> &'static [f64; 7] { &GAP_BUCKET_LIMITS_SECS }
}

pub struct Observer {
    window: VecDeque<TimedEvent>,
    window_size: usize,
    marginal: HashMap<EventKind, u64>,
    cooccur: HashMap<(EventKind, EventKind), u64>,
    total: u64,
    /// Last-seen timestamps para aplicar decay en query time. None = sin
    /// time-decay (modo tradicional).
    last_seen_marginal: HashMap<EventKind, Instant>,
    last_seen_cooccur: HashMap<(EventKind, EventKind), Instant>,
    /// Half-life del decay exponencial en segundos. None = sin decay
    /// (las consultas devuelven los counts crudos).
    half_life_secs: Option<f64>,
    /// Histograma de gaps temporales por par (a, b). Capturado al `record()`.
    gap_histograms: HashMap<(EventKind, EventKind), GapHistogram>,
    /// Sets de "qué cambió desde el último snapshot". Se vacían en
    /// `snapshot()` y `snapshot_delta()`. Usado para escritura incremental.
    dirty_marginal: std::collections::HashSet<EventKind>,
    dirty_cooccur: std::collections::HashSet<(EventKind, EventKind)>,
}

impl Observer {
    pub fn new(window_size: usize) -> Self {
        Self {
            window: VecDeque::with_capacity(window_size),
            window_size,
            marginal: HashMap::new(),
            cooccur: HashMap::new(),
            total: 0,
            last_seen_marginal: HashMap::new(),
            last_seen_cooccur: HashMap::new(),
            half_life_secs: None,
            gap_histograms: HashMap::new(),
            dirty_marginal: std::collections::HashSet::new(),
            dirty_cooccur: std::collections::HashSet::new(),
        }
    }

    /// Activa decay exponencial con half-life en segundos. λ = ln(2)/half_life.
    /// Aplicado en query time sobre los counts crudos usando last_seen.
    pub fn with_half_life(mut self, half_life_secs: f64) -> Self {
        if half_life_secs > 0.0 {
            self.half_life_secs = Some(half_life_secs);
        }
        self
    }

    pub fn half_life(&self) -> Option<f64> { self.half_life_secs }

    /// Registra un evento. Actualiza marginales y co-ocurrencias contra todo
    /// evento aún en la ventana.
    pub fn record(&mut self, kind: EventKind) {
        let now = Instant::now();
        let timed = TimedEvent { kind: kind.clone(), at: now };

        // Co-ocurrencias: este evento con cada uno previo en ventana.
        // Capturamos también el gap temporal (now - w.at) para histograma.
        for w in &self.window {
            let key = (w.kind.clone(), kind.clone());
            *self.cooccur.entry(key.clone()).or_insert(0) += 1;
            self.last_seen_cooccur.insert(key.clone(), now);
            let gap_secs = now.duration_since(w.at).as_secs_f64();
            self.gap_histograms.entry(key.clone()).or_default().observe(gap_secs);
            self.dirty_cooccur.insert(key);
        }

        self.window.push_back(timed);
        if self.window.len() > self.window_size {
            self.window.pop_front();
        }

        *self.marginal.entry(kind.clone()).or_insert(0) += 1;
        self.last_seen_marginal.insert(kind.clone(), now);
        self.dirty_marginal.insert(kind);
        self.total += 1;
    }

    /// Aplica el decay sobre un count crudo dado el `last_seen` correspondiente.
    /// Si half_life es None, devuelve el count tal cual (sin decay).
    fn decay(&self, count: u64, last_seen: Option<Instant>) -> f64 {
        let raw = count as f64;
        let (hl, last) = match (self.half_life_secs, last_seen) {
            (Some(hl), Some(t)) => (hl, t),
            _ => return raw,
        };
        let age_secs = Instant::now().duration_since(last).as_secs_f64();
        raw * 0.5_f64.powf(age_secs / hl)
    }

    /// Marginal con decay aplicado.
    pub fn marginal_decayed(&self, k: &EventKind) -> f64 {
        let raw = self.marginal.get(k).copied().unwrap_or(0);
        let last = self.last_seen_marginal.get(k).copied();
        self.decay(raw, last)
    }

    /// Cooccurrence con decay aplicado.
    pub fn cooccur_decayed(&self, a: &EventKind, b: &EventKind) -> f64 {
        let raw = self.cooccur.get(&(a.clone(), b.clone())).copied().unwrap_or(0);
        let last = self.last_seen_cooccur.get(&(a.clone(), b.clone())).copied();
        self.decay(raw, last)
    }

    /// Entropía de Shannon de la distribución marginal de eventos.
    /// H(X) = −Σ p(x) log₂ p(x). Unidad: bits.
    pub fn shannon_entropy(&self) -> f64 {
        if self.total == 0 { return 0.0; }
        let total = self.total as f64;
        self.marginal.values()
            .map(|&c| {
                let p = c as f64 / total;
                if p > 0.0 { -p * p.log2() } else { 0.0 }
            })
            .sum()
    }

    /// P(b | a) = "dado que algo siguió a `a` dentro del window, qué fracción
    /// fue `b`". Suma 1 sobre todos los b posibles para un a fijo.
    ///
    /// Implementación: cooccur_decayed(a, b) / Σ_x cooccur_decayed(a, x).
    /// Si half_life is None, los decayed values son los counts crudos.
    pub fn conditional_prob(&self, a: &EventKind, b: &EventKind) -> f64 {
        let joint = self.cooccur_decayed(a, b);
        let row_total: f64 = self.cooccur.keys()
            .filter(|(x, _)| x == a)
            .map(|(x, y)| self.cooccur_decayed(x, y))
            .sum();
        if row_total <= 0.0 { 0.0 } else { joint / row_total }
    }

    /// Información mutua puntual entre `a` y `b` con decay aplicado:
    /// PMI(a, b) = log₂( P(a, b) / (P(a) · P(b)) ).
    /// Positivo → más correlacionados de lo que sugiere independencia.
    pub fn pmi(&self, a: &EventKind, b: &EventKind) -> f64 {
        // Total decayed: suma de marginales con decay (no usamos self.total
        // directo porque debería ser consistente con los decayed values).
        let total_decayed: f64 = self.marginal.keys()
            .map(|k| self.marginal_decayed(k))
            .sum();
        if total_decayed <= 0.0 { return 0.0; }
        let joint = self.cooccur_decayed(a, b) / total_decayed;
        let pa = self.marginal_decayed(a) / total_decayed;
        let pb = self.marginal_decayed(b) / total_decayed;
        if joint <= 0.0 || pa <= 0.0 || pb <= 0.0 { return 0.0; }
        (joint / (pa * pb)).log2()
    }

    /// Información mutua acumulada de la pareja (a, b) ponderada por su
    /// probabilidad conjunta. Útil como medida de "interés" del par.
    pub fn weighted_pmi(&self, a: &EventKind, b: &EventKind) -> f64 {
        if self.total == 0 { return 0.0; }
        let joint = self.cooccur
            .get(&(a.clone(), b.clone()))
            .copied()
            .unwrap_or(0) as f64 / self.total as f64;
        joint * self.pmi(a, b)
    }

    pub fn marginals(&self) -> &HashMap<EventKind, u64> { &self.marginal }

    /// Última vez que se vio un kind. None si nunca o si fue restaurado
    /// desde snapshot (los Instants no portables se descartan).
    pub fn last_seen_marginal(&self, kind: &EventKind) -> Option<Instant> {
        self.last_seen_marginal.get(kind).copied()
    }
    pub fn cooccurrences(&self) -> &HashMap<(EventKind, EventKind), u64> { &self.cooccur }
    pub fn total(&self) -> u64 { self.total }
    pub fn window_size(&self) -> usize { self.window_size }
    pub fn current_window(&self) -> usize { self.window.len() }

    /// Últimos N eventos del window, en orden cronológico (más viejo primero).
    /// Si N > window.len(), devuelve todo el window.
    pub fn recent(&self, n: usize) -> impl Iterator<Item = &TimedEvent> {
        let start = self.window.len().saturating_sub(n);
        self.window.range(start..)
    }

    pub fn gap_histograms(&self) -> &HashMap<(EventKind, EventKind), GapHistogram> {
        &self.gap_histograms
    }

    /// Top-K pares por count del histograma (más frecuentes primero).
    /// Útil para limitar cardinalidad de métricas exportadas.
    pub fn top_gap_pairs(&self, k: usize) -> Vec<(&(EventKind, EventKind), &GapHistogram)> {
        let mut pairs: Vec<_> = self.gap_histograms.iter().collect();
        pairs.sort_by(|a, b| b.1.count.cmp(&a.1.count));
        pairs.truncate(k);
        pairs
    }

    /// Snapshot full: estado estadístico completo. Limpia los sets dirty
    /// como side-effect — los próximos `snapshot_delta()` cubren sólo los
    /// cambios posteriores.
    pub fn snapshot(&mut self) -> ObserverSnapshot {
        self.dirty_marginal.clear();
        self.dirty_cooccur.clear();
        ObserverSnapshot {
            schema_version: OBSERVER_SCHEMA_VERSION,
            is_delta: false,
            window_size: self.window_size,
            half_life_secs: self.half_life_secs,
            total: self.total,
            marginal: self.marginal.iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
            cooccur: self.cooccur.iter()
                .map(|((a, b), c)| (a.clone(), b.clone(), *c))
                .collect(),
            gap_histograms: self.gap_histograms.iter()
                .map(|((a, b), h)| (a.clone(), b.clone(), h.clone()))
                .collect(),
        }
    }

    /// Snapshot incremental: sólo incluye los kinds y pares que cambiaron
    /// desde el último `snapshot()` o `snapshot_delta()`. Útil para
    /// checkpoints frecuentes con poco overhead. Limpia los sets dirty.
    pub fn snapshot_delta(&mut self) -> ObserverSnapshot {
        let marginal: Vec<_> = self.dirty_marginal.iter()
            .filter_map(|k| self.marginal.get(k).map(|v| (k.clone(), *v)))
            .collect();
        let cooccur: Vec<_> = self.dirty_cooccur.iter()
            .filter_map(|(a, b)| {
                self.cooccur.get(&(a.clone(), b.clone()))
                    .map(|c| (a.clone(), b.clone(), *c))
            })
            .collect();
        // Para histogramas: incluimos los pares cuyo cooccur cambió.
        let gap_histograms: Vec<_> = self.dirty_cooccur.iter()
            .filter_map(|(a, b)| {
                self.gap_histograms.get(&(a.clone(), b.clone()))
                    .map(|h| (a.clone(), b.clone(), h.clone()))
            })
            .collect();
        self.dirty_marginal.clear();
        self.dirty_cooccur.clear();
        ObserverSnapshot {
            schema_version: OBSERVER_SCHEMA_VERSION,
            is_delta: true,
            window_size: self.window_size,
            half_life_secs: self.half_life_secs,
            total: self.total,
            marginal, cooccur, gap_histograms,
        }
    }

    /// Aplica un delta sobre el estado actual. Para `is_delta=true`, los
    /// valores en marginal/cooccur sobrescriben las entradas existentes.
    /// Si `is_delta=false`, equivale a `from_snapshot` pero in-place.
    pub fn apply_delta(&mut self, delta: ObserverSnapshot) {
        let now = Instant::now();
        if !delta.is_delta {
            // Full: reset state.
            *self = Self::from_snapshot(delta);
            return;
        }
        // Incremental merge.
        for (k, v) in delta.marginal {
            self.last_seen_marginal.insert(k.clone(), now);
            self.marginal.insert(k, v);
        }
        for (a, b, c) in delta.cooccur {
            self.last_seen_cooccur.insert((a.clone(), b.clone()), now);
            self.cooccur.insert((a, b), c);
        }
        for (a, b, h) in delta.gap_histograms {
            self.gap_histograms.insert((a, b), h);
        }
        // total: sólo subimos (el delta podría estar atrasado).
        if delta.total > self.total { self.total = delta.total; }
    }

    /// Reconstruye Observer desde un snapshot. El window queda vacío;
    /// last_seen_* se inicializa en `now()` para que el decay arranque
    /// "ahora" para todos los counts (aproximación razonable post-reboot).
    pub fn from_snapshot(snap: ObserverSnapshot) -> Self {
        let now = Instant::now();
        let mut marginal = HashMap::new();
        let mut last_seen_marginal = HashMap::new();
        for (k, v) in snap.marginal {
            last_seen_marginal.insert(k.clone(), now);
            marginal.insert(k, v);
        }
        let mut cooccur = HashMap::new();
        let mut last_seen_cooccur = HashMap::new();
        for (a, b, c) in snap.cooccur {
            last_seen_cooccur.insert((a.clone(), b.clone()), now);
            cooccur.insert((a, b), c);
        }
        let gap_histograms = snap.gap_histograms.into_iter()
            .map(|(a, b, h)| ((a, b), h))
            .collect();
        Self {
            window: VecDeque::with_capacity(snap.window_size),
            window_size: snap.window_size,
            marginal,
            cooccur,
            total: snap.total,
            last_seen_marginal,
            last_seen_cooccur,
            half_life_secs: snap.half_life_secs,
            gap_histograms,
            dirty_marginal: std::collections::HashSet::new(),
            dirty_cooccur: std::collections::HashSet::new(),
        }
    }
}

const OBSERVER_SCHEMA_VERSION: u16 = 1;

/// Snapshot serializable. Se persiste a JSON en disco y se restaura al
/// reboot para preservar contadores, co-ocurrencias e histogramas.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ObserverSnapshot {
    pub schema_version: u16,
    /// `true` si sólo contiene los cambios desde el último snapshot.
    /// `false` = full state, sobreescribe el observer al aplicar.
    #[serde(default)]
    pub is_delta: bool,
    pub window_size: usize,
    pub half_life_secs: Option<f64>,
    pub total: u64,
    /// Marginales serializados como Vec porque HashMap<EventKind, _> usa
    /// EventKind como key — y EventKind tiene variantes con payloads que
    /// no son JSON-key-serializable (BusInvokeOf, Custom).
    pub marginal: Vec<(EventKind, u64)>,
    pub cooccur: Vec<(EventKind, EventKind, u64)>,
    pub gap_histograms: Vec<(EventKind, EventKind, GapHistogram)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::EventKind::*;

    #[test]
    fn entropy_zero_for_single_event() {
        let mut obs = Observer::new(10);
        for _ in 0..5 { obs.record(EnteSpawned); }
        // Distribución degenerada: una sola observación posible → H = 0.
        assert!(obs.shannon_entropy() < 1e-9);
    }

    #[test]
    fn entropy_one_for_balanced_binary() {
        let mut obs = Observer::new(100);
        for _ in 0..10 { obs.record(EnteSpawned); }
        for _ in 0..10 { obs.record(EnteDied); }
        // Bernoulli(0.5) → H = 1 bit.
        assert!((obs.shannon_entropy() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn conditional_prob_perfect_dependency() {
        let mut obs = Observer::new(100);
        // Spawned siempre seguido por Died.
        for _ in 0..5 {
            obs.record(EnteSpawned);
            obs.record(EnteDied);
        }
        let p = obs.conditional_prob(&EnteSpawned, &EnteDied);
        assert!(p > 0.0, "esperamos correlación positiva, got {p}");
    }
}
