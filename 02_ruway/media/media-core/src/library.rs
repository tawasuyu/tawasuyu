//! library — estado persistible "de biblioteca" del reproductor:
//! dónde quedó cada medio (resume / U2 de `PARIDAD.md`) y el historial
//! de reproducción.
//!
//! Hermano de [`crate::layout`]: igual que el orden de paneles vive acá
//! sin saber cómo se pinta, el **historial** vive acá sin saber cómo se
//! persiste. La regla #2 del repo: la lógica de dominio no sabe quién la
//! pinta ni quién la guarda. La app serializa esto a un `.ron` aparte y
//! lo recarga al arrancar.
//!
//! Identidad agnóstica: un medio se identifica por una `String`
//! arbitraria — la app decide si es la ruta del archivo, la URL de red o
//! un hash BLAKE3 del contenido. El core no mira dentro de la clave.
//!
//! El tiempo también es agnóstico: los métodos reciben `now_secs` (época
//! Unix en segundos) del caller en vez de leer el reloj, así el módulo es
//! determinista y testeable sin tocar `SystemTime`.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Un punto de reanudación: dónde quedó la reproducción de un medio.
/// El reproductor lo consulta al abrir un archivo conocido para ofrecer
/// "continuar donde quedaste" (estilo VLC/mpv).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumePoint {
    /// Identidad del medio (ruta/URL/hash — la app decide).
    pub key: String,
    /// Última posición de reproducción registrada.
    pub position: Duration,
    /// Duración total del medio, si se conoce. Permite calcular el
    /// porcentaje visto y decidir si "ya terminó".
    pub duration: Option<Duration>,
    /// Cuántas veces se empezó a reproducir (cada [`History::note_play`]).
    pub play_count: u32,
    /// Época Unix (segundos) de la última actualización — gobierna la
    /// recencia para el historial y la evicción LRU.
    pub updated_secs: u64,
}

impl ResumePoint {
    /// Fracción vista `[0, 1]`, o `None` si no se conoce la duración.
    pub fn fraction(&self) -> Option<f32> {
        let dur = self.duration?;
        let total = dur.as_secs_f64();
        if total <= 0.0 {
            return None;
        }
        Some((self.position.as_secs_f64() / total).clamp(0.0, 1.0) as f32)
    }

    /// ¿El medio quedó "terminado"? Lo está si la posición llegó a menos
    /// de `tail` del final (cola típica de créditos: ~5 s) o superó el
    /// 98 % de la duración. Sin duración conocida nunca está terminado
    /// (no hay con qué comparar). El reproductor usa esto para arrancar
    /// de cero en vez de reanudar al borde del final.
    pub fn is_finished(&self, tail: Duration) -> bool {
        match self.duration {
            Some(dur) => {
                self.position + tail >= dur
                    || self.fraction().map(|f| f >= 0.98).unwrap_or(false)
            }
            None => false,
        }
    }
}

/// Historial de reproducción: una entrada [`ResumePoint`] por medio
/// conocido, con tope de capacidad y evicción del menos reciente.
///
/// Se serializa como lista (no mapa) para que el `.ron` sea legible y
/// diffeable; las búsquedas por clave son lineales — el historial es
/// chico por diseño (cap default 200).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct History {
    /// Entradas; el orden en disco no es significativo (se consulta por
    /// clave y se ordena por recencia al pedir [`History::recent`]).
    entries: Vec<ResumePoint>,
    /// Tope de entradas; al excederlo se descarta la menos reciente.
    capacity: usize,
}

impl Default for History {
    fn default() -> Self {
        History {
            entries: Vec::new(),
            capacity: 200,
        }
    }
}

impl History {
    /// Historial vacío con la capacidad indicada (mínimo 1).
    pub fn with_capacity(capacity: usize) -> Self {
        History {
            entries: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Reconcilia un historial cargado de disco: clampea la capacidad a
    /// ≥ 1 y aplica la evicción por si el `.ron` traía más entradas que
    /// el tope vigente. Idempotente.
    pub fn sanitized(mut self) -> History {
        self.capacity = self.capacity.max(1);
        self.evict();
        self
    }

    /// Punto de reanudación registrado para `key`, si existe.
    pub fn get(&self, key: &str) -> Option<&ResumePoint> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// Actualiza (o crea) la posición de `key`. No incrementa el contador
    /// de reproducciones — eso es [`History::note_play`], que marca el
    /// *inicio* de una sesión. Pensado para llamarse periódicamente
    /// mientras corre la reproducción y en cada seek.
    pub fn update_position(
        &mut self,
        key: &str,
        position: Duration,
        duration: Option<Duration>,
        now_secs: u64,
    ) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.key == key) {
            e.position = position;
            // Una duración conocida nueva pisa una vieja; no borramos una
            // conocida con un None (un stream puede no reportarla siempre).
            if duration.is_some() {
                e.duration = duration;
            }
            e.updated_secs = now_secs;
        } else {
            self.entries.push(ResumePoint {
                key: key.to_string(),
                position,
                duration,
                play_count: 0,
                updated_secs: now_secs,
            });
            self.evict();
        }
    }

    /// Marca el inicio de una reproducción de `key`: incrementa el
    /// contador y refresca la recencia, creando la entrada si hace falta.
    pub fn note_play(&mut self, key: &str, now_secs: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.key == key) {
            e.play_count = e.play_count.saturating_add(1);
            e.updated_secs = now_secs;
        } else {
            self.entries.push(ResumePoint {
                key: key.to_string(),
                position: Duration::ZERO,
                duration: None,
                play_count: 1,
                updated_secs: now_secs,
            });
            self.evict();
        }
    }

    /// Posición desde la que conviene reanudar `key`, o `None` si no hay
    /// historial o el medio ya quedó terminado (en cuyo caso se arranca
    /// de cero). `tail` es la cola que cuenta como "terminado" (ver
    /// [`ResumePoint::is_finished`]).
    pub fn resume_position(&self, key: &str, tail: Duration) -> Option<Duration> {
        let e = self.get(key)?;
        if e.is_finished(tail) || e.position.is_zero() {
            return None;
        }
        Some(e.position)
    }

    /// Borra la entrada de `key` (p. ej. "olvidar este video"). Devuelve
    /// `true` si existía.
    pub fn forget(&mut self, key: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.key != key);
        self.entries.len() != before
    }

    /// Vacía todo el historial.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Las `n` entradas más recientes, de la más nueva a la más vieja.
    pub fn recent(&self, n: usize) -> Vec<&ResumePoint> {
        let mut refs: Vec<&ResumePoint> = self.entries.iter().collect();
        // Más reciente primero; desempate estable por clave para que el
        // orden sea determinista cuando dos comparten `updated_secs`.
        refs.sort_by(|a, b| {
            b.updated_secs
                .cmp(&a.updated_secs)
                .then_with(|| a.key.cmp(&b.key))
        });
        refs.truncate(n);
        refs
    }

    /// Descarta las entradas menos recientes hasta caber en `capacity`.
    fn evict(&mut self) {
        if self.entries.len() <= self.capacity {
            return;
        }
        // Ordena por recencia descendente y trunca; el desempate por
        // clave mantiene la operación determinista.
        self.entries.sort_by(|a, b| {
            b.updated_secs
                .cmp(&a.updated_secs)
                .then_with(|| a.key.cmp(&b.key))
        });
        self.entries.truncate(self.capacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    #[test]
    fn fraction_y_finished() {
        let rp = ResumePoint {
            key: "a".into(),
            position: d(50),
            duration: Some(d(100)),
            play_count: 1,
            updated_secs: 0,
        };
        assert!((rp.fraction().unwrap() - 0.5).abs() < 1e-4);
        assert!(!rp.is_finished(d(5)));

        // A 99 % → terminado por la regla del 98 %.
        let casi = ResumePoint {
            position: d(99),
            ..rp.clone()
        };
        assert!(casi.is_finished(d(5)));

        // Cerca del final por la cola de 5 s.
        let cola = ResumePoint {
            position: d(96),
            ..rp.clone()
        };
        assert!(cola.is_finished(d(5)));

        // Sin duración nunca termina.
        let stream = ResumePoint {
            duration: None,
            position: d(9999),
            ..rp
        };
        assert!(!stream.is_finished(d(5)));
        assert!(stream.fraction().is_none());
    }

    #[test]
    fn update_position_upsert() {
        let mut h = History::default();
        h.update_position("peli", d(10), Some(d(100)), 1000);
        assert_eq!(h.len(), 1);
        assert_eq!(h.get("peli").unwrap().position, d(10));

        // Segundo update pisa posición y recencia, no duplica.
        h.update_position("peli", d(20), None, 2000);
        assert_eq!(h.len(), 1);
        let e = h.get("peli").unwrap();
        assert_eq!(e.position, d(20));
        // Una duración None no borra la conocida.
        assert_eq!(e.duration, Some(d(100)));
        assert_eq!(e.updated_secs, 2000);
        // update_position no toca el play_count.
        assert_eq!(e.play_count, 0);
    }

    #[test]
    fn note_play_incrementa() {
        let mut h = History::default();
        h.note_play("x", 100);
        h.note_play("x", 200);
        let e = h.get("x").unwrap();
        assert_eq!(e.play_count, 2);
        assert_eq!(e.updated_secs, 200);
    }

    #[test]
    fn resume_position_respeta_terminado_y_cero() {
        let mut h = History::default();
        // Posición en cero → nada que reanudar.
        h.update_position("a", Duration::ZERO, Some(d(100)), 1);
        assert_eq!(h.resume_position("a", d(5)), None);

        // Posición intermedia → reanuda ahí.
        h.update_position("a", d(40), Some(d(100)), 2);
        assert_eq!(h.resume_position("a", d(5)), Some(d(40)));

        // Cerca del final → terminado, arranca de cero.
        h.update_position("a", d(99), Some(d(100)), 3);
        assert_eq!(h.resume_position("a", d(5)), None);

        // Clave desconocida.
        assert_eq!(h.resume_position("nope", d(5)), None);
    }

    #[test]
    fn recent_ordena_por_recencia() {
        let mut h = History::default();
        h.update_position("a", d(1), None, 100);
        h.update_position("b", d(1), None, 300);
        h.update_position("c", d(1), None, 200);
        let r = h.recent(2);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].key, "b"); // updated 300
        assert_eq!(r[1].key, "c"); // updated 200
    }

    #[test]
    fn evict_descarta_el_menos_reciente() {
        let mut h = History::with_capacity(2);
        h.update_position("a", d(1), None, 100);
        h.update_position("b", d(1), None, 200);
        h.update_position("c", d(1), None, 300); // excede cap → cae "a"
        assert_eq!(h.len(), 2);
        assert!(h.get("a").is_none());
        assert!(h.get("b").is_some());
        assert!(h.get("c").is_some());
    }

    #[test]
    fn forget_y_clear() {
        let mut h = History::default();
        h.update_position("a", d(1), None, 1);
        h.update_position("b", d(1), None, 2);
        assert!(h.forget("a"));
        assert!(!h.forget("a")); // ya no está
        assert_eq!(h.len(), 1);
        h.clear();
        assert!(h.is_empty());
    }

    #[test]
    fn sanitized_aplica_capacidad() {
        // Un .ron que traía 3 entradas pero cap=2 (editado a mano).
        let h = History {
            entries: vec![
                ResumePoint {
                    key: "a".into(),
                    position: d(1),
                    duration: None,
                    play_count: 0,
                    updated_secs: 100,
                },
                ResumePoint {
                    key: "b".into(),
                    position: d(1),
                    duration: None,
                    play_count: 0,
                    updated_secs: 300,
                },
                ResumePoint {
                    key: "c".into(),
                    position: d(1),
                    duration: None,
                    play_count: 0,
                    updated_secs: 200,
                },
            ],
            capacity: 2,
        };
        let s = h.sanitized();
        assert_eq!(s.len(), 2);
        assert!(s.get("a").is_none()); // el más viejo (100) cae
    }

    #[test]
    fn round_trip_ron() {
        let mut h = History::with_capacity(50);
        h.update_position("peli.mp4", d(73), Some(d(5400)), 1717000000);
        h.note_play("peli.mp4", 1717000001);
        let txt = ron::ser::to_string(&h).expect("serializa");
        let back: History = ron::from_str(&txt).expect("deserializa");
        assert_eq!(h, back);
    }
}
