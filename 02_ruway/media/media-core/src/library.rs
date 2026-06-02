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

// ============================================================
// Bookmarks — marcas con etiqueta sobre el timeline (U6)
// ============================================================

/// Una marca puesta por el usuario en un punto de un medio: "acá está la
/// escena buena", "retomar el estudio desde acá". A diferencia del
/// [`ResumePoint`] (uno por medio, lo mueve el reproductor solo), de
/// estas hay varias por medio y las pone el usuario a mano.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    /// Identidad del medio (misma convención que [`ResumePoint::key`]).
    pub key: String,
    /// Punto marcado dentro del medio.
    pub position: Duration,
    /// Etiqueta libre del usuario (puede estar vacía).
    pub label: String,
}

/// Colección de [`Bookmark`]s de toda la biblioteca, ordenada de forma
/// canónica por `(key, position)` para que el `.ron` sea estable y la
/// navegación "marca siguiente / anterior" sea un barrido lineal barato.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Bookmarks {
    marks: Vec<Bookmark>,
}

impl Bookmarks {
    pub fn len(&self) -> usize {
        self.marks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Todas las marcas (en orden canónico `(key, position)`).
    pub fn all(&self) -> &[Bookmark] {
        &self.marks
    }

    /// Reordena las marcas a la forma canónica. Útil tras cargar un
    /// `.ron` editado a mano. Idempotente.
    pub fn sanitized(mut self) -> Bookmarks {
        self.reorder();
        self
    }

    /// Agrega una marca. Si ya existe una en (casi) la misma posición del
    /// mismo medio (dentro de `epsilon`), actualiza su etiqueta en vez de
    /// duplicar — así re-marcar el mismo punto renombra. Devuelve `true`
    /// si se insertó una marca nueva (`false` si sólo renombró).
    pub fn add(&mut self, key: &str, position: Duration, label: impl Into<String>) -> bool {
        let label = label.into();
        let epsilon = Duration::from_millis(500);
        if let Some(m) = self.marks.iter_mut().find(|m| {
            m.key == key && abs_diff(m.position, position) <= epsilon
        }) {
            m.label = label;
            return false;
        }
        self.marks.push(Bookmark {
            key: key.to_string(),
            position,
            label,
        });
        self.reorder();
        true
    }

    /// Las marcas de un medio, en orden de posición ascendente.
    pub fn for_media(&self, key: &str) -> Vec<&Bookmark> {
        // `marks` ya está ordenado por (key, position), así que el filtro
        // preserva el orden por posición.
        self.marks.iter().filter(|m| m.key == key).collect()
    }

    /// Borra la marca de `key` más cercana a `position` dentro de
    /// `epsilon`. Devuelve `true` si borró alguna.
    pub fn remove_near(&mut self, key: &str, position: Duration, epsilon: Duration) -> bool {
        let before = self.marks.len();
        // Encuentra la más cercana dentro del epsilon y bórrala.
        let target = self
            .marks
            .iter()
            .enumerate()
            .filter(|(_, m)| m.key == key && abs_diff(m.position, position) <= epsilon)
            .min_by_key(|(_, m)| abs_diff(m.position, position))
            .map(|(i, _)| i);
        if let Some(i) = target {
            self.marks.remove(i);
        }
        self.marks.len() != before
    }

    /// Borra todas las marcas de un medio. Devuelve cuántas borró.
    pub fn clear_media(&mut self, key: &str) -> usize {
        let before = self.marks.len();
        self.marks.retain(|m| m.key != key);
        before - self.marks.len()
    }

    /// Primera marca de `key` estrictamente después de `t` (para "saltar a
    /// la marca siguiente").
    pub fn next_after(&self, key: &str, t: Duration) -> Option<&Bookmark> {
        self.marks
            .iter()
            .filter(|m| m.key == key && m.position > t)
            .min_by_key(|m| m.position)
    }

    /// Última marca de `key` estrictamente antes de `t` (para "saltar a la
    /// marca anterior").
    pub fn prev_before(&self, key: &str, t: Duration) -> Option<&Bookmark> {
        self.marks
            .iter()
            .filter(|m| m.key == key && m.position < t)
            .max_by_key(|m| m.position)
    }

    fn reorder(&mut self) {
        self.marks
            .sort_by(|a, b| a.key.cmp(&b.key).then_with(|| a.position.cmp(&b.position)));
    }
}

/// Diferencia absoluta entre dos `Duration` (no hay `abs_diff` en std
/// para `Duration` en este edition).
fn abs_diff(a: Duration, b: Duration) -> Duration {
    if a >= b {
        a - b
    } else {
        b - a
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

    // ---------- Bookmarks (U6) ----------

    #[test]
    fn bookmarks_add_y_orden_canonico() {
        let mut bm = Bookmarks::default();
        assert!(bm.add("b", d(30), "tres"));
        assert!(bm.add("a", d(20), "dos"));
        assert!(bm.add("a", d(10), "uno"));
        // Orden canónico (a@10, a@20, b@30).
        let all = bm.all();
        assert_eq!(all[0].key, "a");
        assert_eq!(all[0].position, d(10));
        assert_eq!(all[1].position, d(20));
        assert_eq!(all[2].key, "b");
    }

    #[test]
    fn bookmarks_add_renombra_misma_posicion() {
        let mut bm = Bookmarks::default();
        assert!(bm.add("a", d(10), "viejo"));
        // Dentro del epsilon (500 ms) → renombra, no duplica.
        assert!(!bm.add("a", d(10) + Duration::from_millis(200), "nuevo"));
        assert_eq!(bm.len(), 1);
        assert_eq!(bm.all()[0].label, "nuevo");
    }

    #[test]
    fn bookmarks_for_media_filtra_y_ordena() {
        let mut bm = Bookmarks::default();
        bm.add("a", d(30), "");
        bm.add("a", d(10), "");
        bm.add("b", d(5), "");
        let a = bm.for_media("a");
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].position, d(10));
        assert_eq!(a[1].position, d(30));
        assert_eq!(bm.for_media("b").len(), 1);
        assert!(bm.for_media("z").is_empty());
    }

    #[test]
    fn bookmarks_next_prev() {
        let mut bm = Bookmarks::default();
        bm.add("a", d(10), "");
        bm.add("a", d(20), "");
        bm.add("a", d(30), "");
        assert_eq!(bm.next_after("a", d(15)).unwrap().position, d(20));
        assert_eq!(bm.prev_before("a", d(25)).unwrap().position, d(20));
        // En el borde exacto: next/prev son estrictos.
        assert_eq!(bm.next_after("a", d(20)).unwrap().position, d(30));
        assert_eq!(bm.prev_before("a", d(20)).unwrap().position, d(10));
        // Fuera de rango.
        assert!(bm.next_after("a", d(30)).is_none());
        assert!(bm.prev_before("a", d(10)).is_none());
    }

    #[test]
    fn bookmarks_remove_y_clear() {
        let mut bm = Bookmarks::default();
        bm.add("a", d(10), "");
        bm.add("a", d(20), "");
        bm.add("b", d(5), "");
        // Borra la cercana a 10 (dentro de 1 s).
        assert!(bm.remove_near("a", d(10) + Duration::from_millis(300), d(1)));
        assert_eq!(bm.for_media("a").len(), 1);
        // Nada cerca de 100.
        assert!(!bm.remove_near("a", d(100), d(1)));
        // clear_media borra todas las de b.
        assert_eq!(bm.clear_media("b"), 1);
        assert!(bm.for_media("b").is_empty());
    }

    #[test]
    fn bookmarks_round_trip_ron() {
        let mut bm = Bookmarks::default();
        bm.add("peli.mp4", d(73), "escena clave");
        bm.add("peli.mp4", d(300), "final");
        let txt = ron::ser::to_string(&bm).expect("serializa");
        let back: Bookmarks = ron::from_str(&txt).expect("deserializa");
        assert_eq!(bm, back);
    }
}
