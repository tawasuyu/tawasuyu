//! LRU cache para `NatalChart` por contenido.
//!
//! `NatalChart::compute` cuesta varios ms (VSOP2013 + casas + aspectos
//! base). En el shell, mover el slider de orbe o tocar un toggle
//! dispara un `compose()` completo donde la **misma** carta natal del
//! sujeto principal se recomputa idéntica. Lo mismo pasa con el partner
//! de Synastry / Composite — cada drag de slider rearma `partner_natal`.
//!
//! Este cache de 8 entradas es suficiente: el usuario rara vez tiene
//! más de 2 cartas activas a la vez (natal + partner) y el LRU bota la
//! más vieja cuando se llena. La clave es el **contenido** de
//! `StoredBirthData + StoredChartConfig + offset_seconds`, así que
//! editar una carta invalida automáticamente su entrada.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use cosmos_astrology::NatalChart;
use cosmos_model::{StoredBirthData, StoredChartConfig};

const CAPACITY: usize = 8;

type Key = u64;

struct Cache {
    /// Front = más reciente, back = más viejo. `VecDeque` simple — con
    /// cap 8 el search lineal cuesta menos que un HashMap.
    entries: Vec<(Key, Arc<NatalChart>)>,
}

impl Cache {
    fn new() -> Self {
        Self {
            entries: Vec::with_capacity(CAPACITY),
        }
    }

    fn get(&mut self, k: Key) -> Option<Arc<NatalChart>> {
        let idx = self.entries.iter().position(|(kk, _)| *kk == k)?;
        // Move-to-front para mantener LRU.
        let hit = self.entries.remove(idx);
        let chart = hit.1.clone();
        self.entries.insert(0, hit);
        Some(chart)
    }

    fn put(&mut self, k: Key, v: Arc<NatalChart>) {
        // Si ya existe la entrada (race: dos threads computaron lo mismo
        // antes de poblar), reemplaza in-place.
        if let Some(idx) = self.entries.iter().position(|(kk, _)| *kk == k) {
            self.entries.remove(idx);
        }
        self.entries.insert(0, (k, v));
        if self.entries.len() > CAPACITY {
            self.entries.pop();
        }
    }
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| Mutex::new(Cache::new()))
}

/// Hash de contenido: incluye todos los campos relevantes para el
/// cómputo de la carta natal. `f64` se hashea via `to_bits` para evitar
/// el `Hash` ausente de los flotantes.
pub fn key_for(
    birth: &StoredBirthData,
    config: &StoredChartConfig,
    offset_seconds: i64,
) -> u64 {
    let mut h = DefaultHasher::new();
    // Birth data — fecha/hora/lugar.
    birth.year.hash(&mut h);
    birth.month.hash(&mut h);
    birth.day.hash(&mut h);
    birth.hour.hash(&mut h);
    birth.minute.hash(&mut h);
    birth.second.to_bits().hash(&mut h);
    birth.tz_offset_minutes.hash(&mut h);
    birth.latitude_deg.to_bits().hash(&mut h);
    birth.longitude_deg.to_bits().hash(&mut h);
    birth.altitude_m.to_bits().hash(&mut h);
    // Config — todos los toggles que afectan el cómputo de placements y
    // casas. Los enums derivan Debug; reusamos eso para hashear sin
    // forzarles `Hash` manualmente.
    format!("{:?}", config.house_system).hash(&mut h);
    format!("{:?}", config.zodiac).hash(&mut h);
    config.ayanamsha.hash(&mut h);
    config.bodies.hash(&mut h);
    config.include_south_node.hash(&mut h);
    config.include_lilith.hash(&mut h);
    config.include_main_belt_asteroids.hash(&mut h);
    config.include_fixed_stars.hash(&mut h);
    // Offset temporal en segundos (microajuste de rectificación).
    offset_seconds.hash(&mut h);
    h.finish()
}

/// Consulta. Devuelve `None` en miss; el caller debe computar y llamar
/// a `insert`.
pub fn get(k: Key) -> Option<Arc<NatalChart>> {
    cache().lock().ok()?.get(k)
}

/// Inserta una entrada. Idempotente: re-insertar la misma key la mueve
/// al frente.
pub fn insert(k: Key, v: Arc<NatalChart>) {
    if let Ok(mut guard) = cache().lock() {
        guard.put(k, v);
    }
}

