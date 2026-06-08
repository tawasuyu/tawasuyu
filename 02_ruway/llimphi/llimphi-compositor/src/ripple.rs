//! **Ripple / InkWell** — el feedback de tap de Material: un círculo que se
//! expande desde el punto donde el dedo/cursor presionó, clipeado al contorno
//! del nodo, desvaneciéndose mientras crece. Es puro feedback visual: no vive
//! en el `Model` de la app (igual que las animaciones implícitas de
//! [`crate::AnimRegistry`]) sino en un registro retenido por el runtime entre
//! frames.
//!
//! **Flujo.** Un `View` se marca ripple-capaz con [`crate::View::ripple`]
//! (key estable + color). Cuando un press izquierdo cae sobre ese nodo, el
//! runtime hace [`crate::hit_test_ripple`], calcula el punto local del tap y
//! llama [`RippleRegistry::trigger`] — que guarda una "salpicadura" con su
//! reloj. En cada redraw, DESPUÉS del paint del contenido, el runtime llama
//! [`RippleRegistry::paint`], que por cada salpicadura viva resuelve el rect
//! actual del nodo (puede haber cambiado de tamaño), dibuja el círculo
//! expansivo recortado al rrect del nodo y devuelve `true` si alguna sigue
//! viva → el runtime pide otro frame (ticker autodetenido, sin `spawn_periodic`).
//!
//! **Aditivo.** El ripple NO toca el camino click/drag: se dispara en el press
//! por su propio hit-test, conviva o no el nodo con `on_click`. Un botón normal
//! (`on_click` + `.ripple(...)`) recibe ambos.
//!
//! **Limitación v1.** Como la captura de subescenas del fade-out
//! ([`crate::AnimRegistry`]), el paint usa el rect en coordenadas absolutas del
//! layout e ignora los `transform` de ancestros — alcanza para botones/cards
//! (rara vez transformados). La salpicadura es one-shot (expande + se desvanece
//! en `duration`); no hay "mantener mientras se sostiene el press" (Material
//! `hold`), que requeriría rastrear el release por key.

use std::time::{Duration, Instant};

use vello::kurbo::{Affine, Circle};
use vello::peniko::{BlendMode, Color, Fill};
use vello::Scene;

use crate::{ComputedLayout, Mounted};

/// Declara que este nodo emite un **ripple** (salpicadura Material) al recibir
/// un press. `key` debe ser estable entre rebuilds del `View` (igual que la
/// key de [`crate::Anim`]) — es lo que enlaza la salpicadura retenida con el
/// nodo entre frames. `color` es el tinte de la onda (típicamente
/// semitransparente, p. ej. blanco a alpha ~0.25 sobre superficies oscuras o
/// negro a alpha ~0.12 sobre claras); su alpha se multiplica por el fade.
#[derive(Clone, Copy, Debug)]
pub struct Ripple {
    pub key: u64,
    pub color: Color,
    pub duration: Duration,
}

/// Una salpicadura viva: el punto de origen **relativo al rect del nodo** al
/// momento del press, su color/duración y el reloj de expansión.
struct Splash {
    key: u64,
    /// Origen del tap relativo a la esquina superior-izquierda del rect del
    /// nodo (mismo espacio que los handlers `*_at`). Se reancla al rect actual
    /// del nodo en cada frame, así la onda sigue al nodo si éste se mueve.
    lx: f32,
    ly: f32,
    color: Color,
    start: Instant,
    duration: Duration,
    easing: fn(f32) -> f32,
}

impl Splash {
    /// Progreso `[0,1]` sin easing (lineal en el tiempo).
    fn raw(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32();
        (elapsed / self.duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.duration
    }
}

/// Registro de ripples vivos, retenido por el runtime entre frames. Una
/// instancia por ventana; el runtime llama [`Self::trigger`] en el press y
/// [`Self::paint`] tras el paint del contenido.
#[derive(Default)]
pub struct RippleRegistry {
    splashes: Vec<Splash>,
}

impl RippleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra una salpicadura nueva sobre el nodo de key `key`, originada en
    /// `(lx, ly)` relativo a su rect. `now` es el instante del press. Varios
    /// presses rápidos apilan ondas concurrentes (como Material).
    pub fn trigger(
        &mut self,
        key: u64,
        lx: f32,
        ly: f32,
        color: Color,
        duration: Duration,
        now: Instant,
    ) {
        self.splashes.push(Splash {
            key,
            lx,
            ly,
            color,
            start: now,
            duration,
            easing: crate::ease_out_cubic,
        });
    }

    /// `true` si hay alguna salpicadura viva (el runtime ya lo sabe por el
    /// retorno de [`Self::paint`], pero es cómodo para decidir antes).
    pub fn animating(&self) -> bool {
        !self.splashes.is_empty()
    }

    /// Pinta las salpicaduras vivas sobre `scene`, cada una como un círculo que
    /// crece (radio con ease-out hasta cubrir el nodo) y se desvanece, recortado
    /// al contorno redondeado del nodo. Resuelve el rect de cada nodo por su
    /// `ripple.key` en `mounted`/`computed` (así sigue al nodo si se redimensiona).
    /// Descarta las agotadas. Devuelve `true` si queda alguna viva → pedir frame.
    ///
    /// Llamar DESPUÉS del paint del contenido (la onda va encima, translúcida).
    pub fn paint<Msg>(
        &mut self,
        scene: &mut Scene,
        mounted: &Mounted<Msg>,
        computed: &ComputedLayout,
        now: Instant,
    ) -> bool {
        // Descartá primero las agotadas (no dependen del nodo).
        self.splashes.retain(|s| !s.done(now));
        if self.splashes.is_empty() {
            return false;
        }
        for s in &self.splashes {
            // Resolvé el nodo ripple de esta key (el primero que la declare).
            let Some(node) = mounted.nodes.iter().find(|n| {
                n.ripple.map(|r| r.key) == Some(s.key)
            }) else {
                continue;
            };
            let Some(r) = computed.get(node.id) else {
                continue;
            };
            if r.w <= 0.0 || r.h <= 0.0 {
                continue;
            }
            let cx = r.x as f64 + s.lx as f64;
            let cy = r.y as f64 + s.ly as f64;
            // Radio máximo = distancia al rincón más lejano, así la onda llega a
            // cubrir todo el nodo cualquiera sea el punto de origen.
            let corners = [
                (r.x as f64, r.y as f64),
                ((r.x + r.w) as f64, r.y as f64),
                (r.x as f64, (r.y + r.h) as f64),
                ((r.x + r.w) as f64, (r.y + r.h) as f64),
            ];
            let max_radius = corners
                .iter()
                .map(|(px, py)| ((px - cx).powi(2) + (py - cy).powi(2)).sqrt())
                .fold(0.0_f64, f64::max);
            let t = s.raw(now);
            let radius = (s.easing)(t) as f64 * max_radius;
            if radius <= 0.0 {
                continue;
            }
            // Fade: la onda arranca a su alpha y se apaga al expandirse.
            let fade = 1.0 - t;
            let mut col = s.color;
            col.components[3] *= fade;
            if col.components[3] <= 0.0 {
                continue;
            }
            // Recorte al contorno del nodo (respeta radio/esquinas), para que la
            // onda no sangre fuera de un botón redondeado.
            let rrect = crate::render::node_rrect(
                r.x as f64,
                r.y as f64,
                (r.x + r.w) as f64,
                (r.y + r.h) as f64,
                node.radius,
                node.corner_radii,
                0.0,
            );
            scene.push_layer(Fill::NonZero, BlendMode::default(), 1.0, Affine::IDENTITY, &rrect);
            let circle = Circle::new((cx, cy), radius);
            scene.fill(Fill::NonZero, Affine::IDENTITY, col, None, &circle);
            scene.pop_layer();
        }
        !self.splashes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mount, View};
    use llimphi_layout::taffy::prelude::*;
    use llimphi_layout::{LayoutTree, Style};

    fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color::from_rgba8(r, g, b, a)
    }

    /// Monta un botón 100×100 con ripple(key=5) y devuelve (mounted, computed).
    fn boton() -> (Mounted<()>, ComputedLayout) {
        let v = View::<()>::new(Style {
            size: Size { width: length(100.0), height: length(100.0) },
            ..Default::default()
        })
        .ripple(5, rgba(255, 255, 255, 80));
        let mut layout = LayoutTree::new();
        let m = mount(&mut layout, v);
        let c = layout.compute(m.root, (200.0, 200.0)).expect("layout");
        (m, c)
    }

    #[test]
    fn sin_trigger_no_anima() {
        let mut reg = RippleRegistry::new();
        let (m, c) = boton();
        let mut scene = Scene::new();
        assert!(!reg.paint(&mut scene, &m, &c, Instant::now()));
        assert!(!reg.animating());
    }

    #[test]
    fn trigger_anima_y_se_autodetiene() {
        let mut reg = RippleRegistry::new();
        let (m, c) = boton();
        let t0 = Instant::now();
        reg.trigger(5, 50.0, 50.0, rgba(255, 255, 255, 80), Duration::from_millis(200), t0);
        assert!(reg.animating(), "tras el trigger hay onda viva");
        let mut scene = Scene::new();
        // A mitad de la duración sigue animando.
        assert!(reg.paint(&mut scene, &m, &c, t0 + Duration::from_millis(100)));
        // Pasada la duración, se descarta y el ticker para.
        assert!(!reg.paint(&mut scene, &m, &c, t0 + Duration::from_millis(250)));
        assert!(!reg.animating());
    }

    #[test]
    fn presses_concurrentes_apilan_ondas() {
        let mut reg = RippleRegistry::new();
        let t0 = Instant::now();
        reg.trigger(5, 10.0, 10.0, rgba(255, 255, 255, 80), Duration::from_millis(200), t0);
        reg.trigger(5, 90.0, 90.0, rgba(255, 255, 255, 80), Duration::from_millis(200), t0 + Duration::from_millis(20));
        assert_eq!(reg.splashes.len(), 2);
        let (m, c) = boton();
        let mut scene = Scene::new();
        // En t0+100 la primera vive (80ms restantes) y la segunda también.
        assert!(reg.paint(&mut scene, &m, &c, t0 + Duration::from_millis(100)));
        assert_eq!(reg.splashes.len(), 2);
        // En t0+210 la primera murió (210>200) pero la segunda vive (190<200).
        assert!(reg.paint(&mut scene, &m, &c, t0 + Duration::from_millis(210)));
        assert_eq!(reg.splashes.len(), 1);
    }

    #[test]
    fn key_inexistente_se_descarta_al_agotarse_sin_panico() {
        // Una onda cuya key no existe en el árbol no debe pintar ni panico;
        // simplemente no encuentra nodo y se descarta cuando su reloj vence.
        let mut reg = RippleRegistry::new();
        let t0 = Instant::now();
        reg.trigger(999, 0.0, 0.0, rgba(255, 255, 255, 80), Duration::from_millis(100), t0);
        let (m, c) = boton();
        let mut scene = Scene::new();
        assert!(reg.paint(&mut scene, &m, &c, t0 + Duration::from_millis(50)));
        assert!(!reg.paint(&mut scene, &m, &c, t0 + Duration::from_millis(150)));
    }
}
