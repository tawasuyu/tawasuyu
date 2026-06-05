//! Animaciones **implícitas** (estilo Flutter `AnimatedContainer`): un nodo
//! del `View` declara una `key` estable y, cuando sus props visuales de paint
//! cambian entre frames, el runtime **interpola** en vez de saltar — sin que
//! la app cablee un `Tween` en su `Model` ni un loop de ticks.
//!
//! El modelo de Llimphi reconstruye el árbol `View` cada frame desde el
//! `Model`, así que no hay estado retenido por nodo. Este registro lo aporta:
//! mapea `key → AnimEntry` (valor actual + objetivo + reloj) y vive en el
//! runtime entre frames. En cada redraw, DESPUÉS de `compute` y ANTES de
//! `paint`, el runtime llama [`AnimRegistry::reconcile`], que:
//!
//! 1. Para cada nodo con [`Anim`], toma su valor objetivo (lo que la `view`
//!    pintó este frame).
//! 2. Si el objetivo cambió respecto del guardado, arranca un tween desde el
//!    valor interpolado actual hacia el nuevo.
//! 3. Escribe el valor interpolado de vuelta en el nodo (fill/radius) para
//!    que `paint` lo use.
//! 4. Devuelve `true` si alguna animación sigue viva → el runtime pide otro
//!    frame (`request_redraw`). Cuando todas se asientan, deja de pedir frames
//!    (el ticker se autodetiene; no hay render loop ocioso).
//!
//! La **primera** aparición de una key no anima (igual que Flutter): sólo los
//! **cambios** posteriores se interpolan. Props soportadas hoy: `fill` (color),
//! `radius` y `alpha` (opacidad). Es ampliable agregando campos a
//! [`AnimSnapshot`].
//!
//! **Animación de contenido (entrada).** Aparte de los cambios de props, una
//! key puede pedir animar su **entrada** ([`crate::View::animated_enter`]): su
//! primera aparición sube la opacidad de 0 a su valor (fade-in estilo
//! `AnimatedSwitcher`). El **exit** (fade-out al desmontarse) todavía no está:
//! requiere que el registro retenga y siga pintando un nodo que ya salió del
//! árbol — un nodo que desaparece hoy se va de una.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use vello::peniko::Color;

use crate::Mounted;

/// Declara que las props visuales de paint de este nodo se animan de forma
/// implícita. `key` debe ser estable entre rebuilds del `View` (índice de
/// item, hash de id, etc.) — es lo que enlaza "el mismo nodo" entre frames.
#[derive(Clone, Copy, Debug)]
pub struct Anim {
    pub key: u64,
    pub duration: Duration,
    /// Easing aplicado a `t ∈ [0,1]`. Las canónicas viven en
    /// `llimphi_theme::motion`; por defecto el builder usa un ease-out cúbico.
    pub easing: fn(f32) -> f32,
    /// `true` si la **primera aparición** de la key debe animar la opacidad de
    /// 0 hacia su valor (fade-in de entrada, estilo `AnimatedSwitcher`). Las
    /// animaciones de props (fill/radius/alpha) no entran por acá: sólo cambian
    /// el arranque del primer frame. Sin él, la primera aparición se asienta
    /// instantánea (default histórico de `View::animated`).
    pub enter: bool,
}

/// Ease-out cúbico, el default razonable para transiciones implícitas
/// (arranca rápido, frena suave). Copia local para no acoplar el compositor a
/// `llimphi-theme`; el caller puede pasar cualquier `fn(f32)->f32`.
pub fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// Foto de las props animables de un nodo en un frame. `alpha == None` ≡ nodo
/// opaco (1.0): es la convención de `View::alpha` y la usa el lerp para mezclar
/// hacia/desde "sin alpha explícito" sin tratarlo como un salto.
#[derive(Clone, Copy, PartialEq)]
struct AnimSnapshot {
    fill: Option<Color>,
    radius: f64,
    alpha: Option<f32>,
}

#[inline]
fn lerp_f64(a: f64, b: f64, t: f32) -> f64 {
    a + (b - a) * t as f64
}

#[inline]
fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let p = a.components;
    let q = b.components;
    Color {
        components: [
            p[0] + (q[0] - p[0]) * t,
            p[1] + (q[1] - p[1]) * t,
            p[2] + (q[2] - p[2]) * t,
            p[3] + (q[3] - p[3]) * t,
        ],
        ..a
    }
}

impl AnimSnapshot {
    /// Interpola entre `self` (origen) y `to` (objetivo). El color sólo se
    /// mezcla si ambos lados tienen fill sólido; si uno es `None` (gradiente o
    /// sin fill) se salta al objetivo sin crossfade.
    fn lerp(self, to: AnimSnapshot, t: f32) -> AnimSnapshot {
        let fill = match (self.fill, to.fill) {
            (Some(a), Some(b)) => Some(lerp_color(a, b, t)),
            _ => to.fill,
        };
        // `None` ≡ opaco (1.0): un lado sin alpha se mezcla contra 1.0 en vez
        // de saltar, así fade-in (0→opaco) y fade de un alpha explícito a/desde
        // "sin alpha" interpolan suave. None↔None se mantiene None (sin capa).
        let alpha = match (self.alpha, to.alpha) {
            (None, None) => None,
            (a, b) => {
                let from = a.unwrap_or(1.0);
                let dst = b.unwrap_or(1.0);
                Some(from + (dst - from) * t)
            }
        };
        AnimSnapshot {
            fill,
            radius: lerp_f64(self.radius, to.radius, t),
            alpha,
        }
    }
}

/// Estado retenido de una animación: tween entre `from` y `to`.
struct AnimEntry {
    from: AnimSnapshot,
    to: AnimSnapshot,
    start: Instant,
    duration: Duration,
    easing: fn(f32) -> f32,
}

impl AnimEntry {
    /// Entrada ya asentada en `snap` (from == to): no anima.
    fn settled(snap: AnimSnapshot, now: Instant) -> Self {
        Self {
            from: snap,
            to: snap,
            start: now,
            duration: Duration::ZERO,
            easing: |t| t,
        }
    }

    /// Progreso `[0,1]` con easing aplicado.
    fn t(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32();
        let raw = (elapsed / self.duration.as_secs_f32()).clamp(0.0, 1.0);
        (self.easing)(raw)
    }

    fn value(&self, now: Instant) -> AnimSnapshot {
        self.from.lerp(self.to, self.t(now))
    }

    fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.duration
    }
}

/// Registro de animaciones implícitas, vivo entre frames. El runtime mantiene
/// una instancia y llama [`Self::reconcile`] en cada redraw.
#[derive(Default)]
pub struct AnimRegistry {
    entries: HashMap<u64, AnimEntry>,
}

impl AnimRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconcilia el árbol montado con el estado retenido. Para cada nodo con
    /// [`Anim`]: detecta si el objetivo cambió (arranca tween), interpola y
    /// **escribe** el valor del frame de vuelta en el nodo (fill/radius). Las
    /// keys que no aparecieron este frame se descartan (un nodo que se va deja
    /// de animar). Devuelve `true` si alguna animación sigue en curso.
    ///
    /// Llamar DESPUÉS de `compute` y ANTES de `paint`. `now` es el instante del
    /// frame (el runtime pasa `Instant::now()`; los tests pasan instantes
    /// controlados).
    pub fn reconcile<Msg>(&mut self, mounted: &mut Mounted<Msg>, now: Instant) -> bool {
        let mut animating = false;
        let mut seen: Vec<u64> = Vec::new();
        for node in &mut mounted.nodes {
            let Some(anim) = node.anim else { continue };
            seen.push(anim.key);
            let target = AnimSnapshot {
                fill: node.fill,
                radius: node.radius,
                alpha: node.alpha,
            };
            let entry = self.entries.entry(anim.key).or_insert_with(|| {
                // Primera aparición. Con `enter`, arranca un tween de opacidad
                // 0 → objetivo (fade-in); si no, se asienta instantánea.
                if anim.enter {
                    let from = AnimSnapshot { alpha: Some(0.0), ..target };
                    AnimEntry {
                        from,
                        to: target,
                        start: now,
                        duration: anim.duration,
                        easing: anim.easing,
                    }
                } else {
                    AnimEntry::settled(target, now)
                }
            });
            // Cambió el objetivo: congelá el valor actual como nuevo origen y
            // rearrancá el reloj hacia el objetivo nuevo.
            if entry.to != target {
                entry.from = entry.value(now);
                entry.to = target;
                entry.start = now;
                entry.duration = anim.duration;
                entry.easing = anim.easing;
            }
            // Al terminar aterriza EXACTO en el objetivo (incluido `alpha:
            // None`, que evita una capa de opacidad residual frame a frame).
            let v = if entry.done(now) { entry.to } else { entry.value(now) };
            node.fill = v.fill;
            node.radius = v.radius;
            node.alpha = v.alpha;
            if !entry.done(now) {
                animating = true;
            }
        }
        if self.entries.len() != seen.len() {
            self.entries.retain(|k, _| seen.contains(k));
        }
        animating
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mount, View};
    use llimphi_layout::{LayoutTree, Style};

    fn rgba(r: u8, g: u8, b: u8) -> Color {
        Color::from_rgba8(r, g, b, 255)
    }

    /// Monta un único nodo con fill + anim(key=1) y devuelve su `Mounted`.
    fn one(fill: Color) -> Mounted<()> {
        let v = View::<()>::new(Style::default())
            .fill(fill)
            .animated(1, Duration::from_millis(200));
        let mut layout = LayoutTree::new();
        mount(&mut layout, v)
    }

    #[test]
    fn primera_aparicion_no_anima() {
        let mut reg = AnimRegistry::new();
        let mut m = one(rgba(255, 0, 0));
        let now = Instant::now();
        let animating = reg.reconcile(&mut m, now);
        assert!(!animating, "la primera vez no debe animar");
        assert_eq!(m.nodes[0].fill, Some(rgba(255, 0, 0)));
    }

    #[test]
    fn cambio_de_color_interpola_y_pide_frames() {
        let mut reg = AnimRegistry::new();
        let t0 = Instant::now();
        // Frame 1: rojo, se asienta.
        let mut m = one(rgba(255, 0, 0));
        reg.reconcile(&mut m, t0);
        // Frame 2: la view ahora pinta azul (target nuevo). En el frame en que
        // se DETECTA el cambio arranca el reloj: aún muestra el origen (rojo)
        // pero ya pide frames.
        let mut m = one(rgba(0, 0, 255));
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(100));
        assert!(animating, "al detectar el cambio debe pedir frames");
        // Frame 3: 100ms dentro del tween de 200ms. El fill ya está mezclado:
        // ni rojo puro ni azul puro.
        let mut m = one(rgba(0, 0, 255));
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(200));
        assert!(animating, "a mitad del tween debe seguir animando");
        let c = m.nodes[0].fill.expect("fill").components;
        assert!(c[0] < 1.0 && c[0] > 0.0, "rojo intermedio: {}", c[0]);
        assert!(c[2] > 0.0 && c[2] < 1.0, "azul intermedio: {}", c[2]);
    }

    #[test]
    fn al_terminar_llega_al_objetivo_y_deja_de_pedir_frames() {
        let mut reg = AnimRegistry::new();
        let t0 = Instant::now();
        let mut m = one(rgba(255, 0, 0));
        reg.reconcile(&mut m, t0);
        let mut m = one(rgba(0, 0, 255));
        reg.reconcile(&mut m, t0 + Duration::from_millis(100)); // arranca
        // Pasada la duración, llega exacto al objetivo y no pide más frames.
        let mut m = one(rgba(0, 0, 255));
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(400));
        assert!(!animating);
        assert_eq!(m.nodes[0].fill, Some(rgba(0, 0, 255)));
    }

    /// Monta un nodo con alpha + anim(key=1) y devuelve su `Mounted`.
    fn one_alpha(alpha: f32) -> Mounted<()> {
        let v = View::<()>::new(Style::default())
            .alpha(alpha)
            .animated(1, Duration::from_millis(200));
        let mut layout = LayoutTree::new();
        mount(&mut layout, v)
    }

    /// Monta un nodo opaco (sin alpha) con animación de ENTRADA.
    fn one_enter() -> Mounted<()> {
        let v = View::<()>::new(Style::default())
            .fill(rgba(10, 20, 30))
            .animated_enter(1, Duration::from_millis(200));
        let mut layout = LayoutTree::new();
        mount(&mut layout, v)
    }

    #[test]
    fn fade_in_de_entrada_arranca_transparente_y_llega_a_opaco() {
        let mut reg = AnimRegistry::new();
        let t0 = Instant::now();
        // Primera aparición de un nodo `enter`: a diferencia de `animated`,
        // SÍ anima — arranca casi transparente y pide frames.
        let mut m = one_enter();
        let animating = reg.reconcile(&mut m, t0);
        assert!(animating, "la entrada debe animar desde el primer frame");
        assert_eq!(m.nodes[0].alpha, Some(0.0), "arranca transparente");
        // A mitad del tween, alpha intermedio.
        let mut m = one_enter();
        reg.reconcile(&mut m, t0 + Duration::from_millis(100));
        let a = m.nodes[0].alpha.expect("alpha");
        assert!(a > 0.0 && a < 1.0, "alpha intermedio: {a}");
        // Pasada la duración: opaco exacto (None, sin capa residual) y quieto.
        let mut m = one_enter();
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(400));
        assert!(!animating);
        assert_eq!(m.nodes[0].alpha, None, "aterriza en opaco sin capa");
    }

    #[test]
    fn cambio_de_alpha_interpola() {
        let mut reg = AnimRegistry::new();
        let t0 = Instant::now();
        // Frame 1: alpha 1.0, se asienta (no es `enter`).
        let mut m = one_alpha(1.0);
        let animating = reg.reconcile(&mut m, t0);
        assert!(!animating, "primera aparición sin enter no anima");
        // Frame 2: la view baja a 0.0 → arranca tween.
        let mut m = one_alpha(0.0);
        reg.reconcile(&mut m, t0 + Duration::from_millis(50));
        // Frame 3: a mitad, alpha intermedio.
        let mut m = one_alpha(0.0);
        reg.reconcile(&mut m, t0 + Duration::from_millis(150));
        let a = m.nodes[0].alpha.expect("alpha");
        assert!(a > 0.0 && a < 1.0, "alpha intermedio: {a}");
    }

    #[test]
    fn keys_que_se_van_se_descartan() {
        let mut reg = AnimRegistry::new();
        let now = Instant::now();
        let mut m = one(rgba(1, 2, 3));
        reg.reconcile(&mut m, now);
        assert_eq!(reg.entries.len(), 1);
        // Frame sin ningún nodo animado: la entrada se descarta.
        let v = View::<()>::new(Style::default()).fill(rgba(9, 9, 9));
        let mut layout = LayoutTree::new();
        let mut m2 = mount(&mut layout, v);
        reg.reconcile(&mut m2, now);
        assert_eq!(reg.entries.len(), 0);
    }
}
