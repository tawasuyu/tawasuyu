//! **Hero / shared-element transitions** — un mismo nodo lógico (key estable)
//! que aparece en posiciones distintas entre frames "vuela" del rect anterior
//! al actual en vez de saltar. Es el Hero de Flutter auténtico.
//!
//! Modelo:
//! - El caller marca un nodo con [`View::hero(key, duration)`](crate::View::hero).
//!   `key` enlaza "el mismo nodo lógico" entre dos `view()` distintos (entre
//!   rutas, paneles, layouts) — dos nodos con la misma `key` en frames distintos
//!   son la misma identidad para el runtime.
//! - El runtime mantiene una instancia de [`HeroRegistry`] entre frames y llama
//!   [`HeroRegistry::reconcile`] DESPUÉS de `compute` y ANTES de `paint`. Por
//!   cada nodo hero:
//!   - Lee su rect absoluto del [`ComputedLayout`].
//!   - Si en el frame anterior la misma `key` vivió en un rect distinto,
//!     arranca un tween: durante `duration`, escribe en `node.transform` una
//!     afín que "lleva visualmente" el nodo del rect actual al rect anterior y
//!     converge a `IDENTITY`. El nodo se ve VOLAR del rect anterior al actual.
//!   - Mientras el tween esté vivo, devuelve `true` y el runtime pide otro
//!     frame (ticker autodetenido).
//! - Al asentarse, deja `node.transform = None`: cero costo de transform
//!   residual en frames posteriores.
//!
//! No depende de [`crate::AnimRegistry`] — el wiring es independiente; sólo
//! reusa el campo `transform` del [`MountedNode`](crate::MountedNode), que el
//! `paint` ya respeta como cualquier otro afín.
//!
//! ## Reglas de uso
//!
//! - `key` debe ser estable y **única** entre los nodos hero presentes en un
//!   mismo frame. Dos hero con la misma key en el mismo árbol generan
//!   ambigüedad; el runtime se queda con la última que recorra.
//! - El rect "anterior" es el del frame anterior — no funciona como
//!   shared-element entre dos *vistas montadas a la vez* (eso requeriría dos
//!   rect simultáneos por key). Funciona entre transiciones de rutas.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use llimphi_layout::{ComputedLayout, Rect};
use vello::kurbo::Affine;

/// Declara un nodo como **hero**: la `key` enlaza la identidad entre frames; si
/// el rect cambia, el runtime anima la transición.
#[derive(Clone, Copy, Debug)]
pub struct Hero {
    pub key: u64,
    pub duration: Duration,
    /// Easing aplicado a `t ∈ [0,1]`. Por defecto, los setters de [`View`]
    /// usan un ease-out cúbico (igual que las animaciones implícitas).
    pub easing: fn(f32) -> f32,
}

/// Registro de heroes, vivo entre frames. Guarda el último rect por `key` para
/// detectar el delta y un tween activo si está animando.
#[derive(Default)]
pub struct HeroRegistry {
    /// Último rect donde se pintó un nodo con esta `key`. Se actualiza en cada
    /// `reconcile`. Es contra esto que detectamos el cambio que dispara el
    /// tween.
    last: HashMap<u64, Rect>,
    /// Tweens en curso. Cada uno conoce su `from_rect`, el reloj y el easing.
    /// Una key con tween activo NO arranca uno nuevo si vuelve a moverse —
    /// reusamos el `from_rect` original para que la trayectoria sea continua
    /// (si el target cambia a mitad, vuela hacia el nuevo destino, no cambia
    /// el origen).
    tweens: HashMap<u64, Tween>,
}

struct Tween {
    from_rect: Rect,
    start: Instant,
    duration: Duration,
    easing: fn(f32) -> f32,
}

impl HeroRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconcilia heroes con el árbol montado. Para cada nodo con [`Hero`]:
    /// - Si el rect cambió respecto del frame anterior, arranca tween.
    /// - Si hay tween activo y vivo, escribe `node.transform` con la afín
    ///   interpolada (cur → from).
    /// - Cuando el tween termina, lo limpia y deja `node.transform = None`.
    ///
    /// Llamar DESPUÉS de `compute` y ANTES de `paint`. Devuelve `true` si
    /// algún tween sigue en curso → el runtime pide otro frame.
    pub fn reconcile<Msg>(
        &mut self,
        mounted: &mut crate::Mounted<Msg>,
        computed: &ComputedLayout,
        now: Instant,
    ) -> bool {
        let mut animating = false;
        let mut seen: Vec<u64> = Vec::new();
        for node in &mut mounted.nodes {
            let Some(hero) = node.hero else { continue };
            let Some(cur) = computed.get(node.id) else { continue };
            seen.push(hero.key);

            // ¿Cambió el rect respecto del último frame? Arrancar tween (si no
            // hay uno activo; si lo hay, no re-resetamos el origen).
            if let Some(last) = self.last.get(&hero.key).copied() {
                if last != cur && !self.tweens.contains_key(&hero.key) {
                    self.tweens.insert(
                        hero.key,
                        Tween {
                            from_rect: last,
                            start: now,
                            duration: hero.duration,
                            easing: hero.easing,
                        },
                    );
                }
            }

            // Aplicar tween si está vivo. Calcula la afín que mapea `cur` al
            // `from_rect` y la interpola hacia identidad a medida que `t` crece.
            if let Some(tw) = self.tweens.get(&hero.key) {
                let elapsed = now.saturating_duration_since(tw.start).as_secs_f32();
                let raw = (elapsed / tw.duration.as_secs_f32().max(1e-6)).clamp(0.0, 1.0);
                if raw >= 1.0 {
                    // Aterrizó: dejamos el nodo sin transform y limpiamos.
                    node.transform = None;
                    self.tweens.remove(&hero.key);
                } else {
                    let t = (tw.easing)(raw);
                    let back = back_transform(cur, tw.from_rect);
                    let xf = lerp_affine(back, Affine::IDENTITY, t);
                    node.transform = Some(xf);
                    animating = true;
                }
            }

            self.last.insert(hero.key, cur);
        }
        // Las keys que no aparecieron este frame se descartan (un hero que se
        // va deja de recordarse; si vuelve, su rect "anterior" será el nuevo
        // primero — no anima desde el último que tuvo hace varios frames).
        if self.last.len() != seen.len() {
            self.last.retain(|k, _| seen.contains(k));
            self.tweens.retain(|k, _| seen.contains(k));
        }
        animating
    }
}

/// Afín local que, aplicada con [`View::transform`]'s convención (alrededor
/// del centro del rect actual), mapea visualmente cada punto del `cur_rect`
/// al punto correspondiente del `from_rect`. Es la base de un "fly":
/// el nodo se pinta en `cur` pero con esta xf VOLVIÓ a `from` —
/// interpolando hacia identidad, "vuela" de `from` a `cur`.
fn back_transform(cur: Rect, from: Rect) -> Affine {
    // El compositor aplica xf como `T(centro_cur) · xf_local · T(-centro_cur)`,
    // así que xf_local debe ser `scale + translate` que mapea:
    //   esquina superior izquierda de cur → esquina superior izquierda de from.
    //
    // Si scale = (from.w/cur.w, from.h/cur.h) y t = (cx_from - cx_cur,
    // cy_from - cy_cur), entonces `T(t) · S` cumple esa propiedad (despejo en
    // los comentarios del módulo).
    let sx = (from.w as f64) / (cur.w as f64).max(1e-6);
    let sy = (from.h as f64) / (cur.h as f64).max(1e-6);
    let cx_cur = (cur.x + cur.w * 0.5) as f64;
    let cy_cur = (cur.y + cur.h * 0.5) as f64;
    let cx_from = (from.x + from.w * 0.5) as f64;
    let cy_from = (from.y + from.h * 0.5) as f64;
    Affine::translate((cx_from - cx_cur, cy_from - cy_cur)) * Affine::scale_non_uniform(sx, sy)
}

/// Lerp componente-a-componente de las 6 coefs del afín. Idéntica al helper
/// privado de [`crate::anim`] — vive separada para mantener el módulo `hero`
/// auto-contenido (sin acoplar a Anim).
fn lerp_affine(a: Affine, b: Affine, t: f32) -> Affine {
    let p = a.as_coeffs();
    let q = b.as_coeffs();
    let ft = t as f64;
    Affine::new([
        p[0] + (q[0] - p[0]) * ft,
        p[1] + (q[1] - p[1]) * ft,
        p[2] + (q[2] - p[2]) * ft,
        p[3] + (q[3] - p[3]) * ft,
        p[4] + (q[4] - p[4]) * ft,
        p[5] + (q[5] - p[5]) * ft,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mount, View};
    use llimphi_layout::{LayoutTree, Style};
    use llimphi_layout::taffy::prelude::length;
    use llimphi_layout::taffy::Size;

    /// Monta un único nodo hero con su Style + key=1 + dur=200ms. Devuelve
    /// `Mounted` y el `ComputedLayout` ya resuelto contra un viewport de
    /// 1000×1000 — los rects salen del propio Style.
    fn one(x: f32, y: f32, w: f32, h: f32) -> (crate::Mounted<()>, ComputedLayout) {
        let v = View::<()>::new(Style {
            size: Size { width: length(w), height: length(h) },
            inset: llimphi_layout::taffy::Rect {
                left: length(x),
                top: length(y),
                right: llimphi_layout::taffy::prelude::auto(),
                bottom: llimphi_layout::taffy::prelude::auto(),
            },
            position: llimphi_layout::taffy::Position::Absolute,
            ..Default::default()
        })
        .hero(1, Duration::from_millis(200));
        let mut layout = LayoutTree::new();
        let mounted = mount(&mut layout, v);
        let computed = layout
            .compute(mounted.root, (1000.0_f32, 1000.0_f32))
            .expect("layout");
        (mounted, computed)
    }

    #[test]
    fn primera_aparicion_no_anima() {
        let mut reg = HeroRegistry::new();
        let (mut m, c) = one(10.0, 10.0, 50.0, 50.0);
        let animating = reg.reconcile(&mut m, &c, Instant::now());
        assert!(!animating, "primera aparición no debe animar");
        assert!(m.nodes[0].transform.is_none(), "sin xf en primer frame");
    }

    #[test]
    fn cambio_de_rect_arranca_tween_y_aplica_xf() {
        let mut reg = HeroRegistry::new();
        let t0 = Instant::now();
        // Frame 1: rect (10, 10, 50, 50).
        let (mut m, c) = one(10.0, 10.0, 50.0, 50.0);
        reg.reconcile(&mut m, &c, t0);
        // Frame 2: el nodo ahora vive en (200, 200, 100, 100) → arranca tween.
        let (mut m, c) = one(200.0, 200.0, 100.0, 100.0);
        let animating = reg.reconcile(&mut m, &c, t0 + Duration::from_millis(50));
        assert!(animating, "cambio de rect → tween");
        let xf = m.nodes[0].transform.expect("xf");
        // A 50ms en una anim de 200ms, raw ≈ 0.25; con ease-out cúbico t > 0.25.
        // La afín NO debe ser identidad (algún coef se ve).
        let c = xf.as_coeffs();
        assert!(c[0] != 1.0 || c[3] != 1.0 || c[4] != 0.0 || c[5] != 0.0,
                "xf no debe ser identidad a mitad del tween: {:?}", c);
    }

    #[test]
    fn al_terminar_limpia_la_xf() {
        let mut reg = HeroRegistry::new();
        let t0 = Instant::now();
        let (mut m, c) = one(10.0, 10.0, 50.0, 50.0);
        reg.reconcile(&mut m, &c, t0);
        let (mut m, c) = one(200.0, 200.0, 100.0, 100.0);
        reg.reconcile(&mut m, &c, t0 + Duration::from_millis(10));
        // Pasada la duración: el tween se descarta y deja el nodo sin xf.
        let (mut m, c) = one(200.0, 200.0, 100.0, 100.0);
        let animating = reg.reconcile(&mut m, &c, t0 + Duration::from_millis(500));
        assert!(!animating);
        assert!(m.nodes[0].transform.is_none());
    }

    #[test]
    fn back_transform_es_identidad_si_los_rects_coinciden() {
        let r = Rect { x: 50.0, y: 50.0, w: 100.0, h: 100.0 };
        let xf = back_transform(r, r);
        let c = xf.as_coeffs();
        assert!((c[0] - 1.0).abs() < 1e-9);
        assert!((c[3] - 1.0).abs() < 1e-9);
        assert!(c[4].abs() < 1e-9);
        assert!(c[5].abs() < 1e-9);
    }
}
