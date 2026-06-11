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
//! `radius`, `alpha` (opacidad) y `transform` (afín 2D — scale/rotate/translate
//! alrededor del centro del rect). Es ampliable agregando campos a
//! [`AnimSnapshot`].
//!
//! **Animación de contenido (entrada y salida).** Aparte de los cambios de
//! props, una key puede animar su **entrada** ([`crate::View::animated_enter`]:
//! la primera aparición sube la opacidad de 0 a su valor) y su **salida**
//! ([`crate::View::animated_exit`]: al desaparecer del árbol). El exit no se
//! puede hacer sólo modificando nodos vivos — el nodo ya no está. La solución:
//! el runtime captura la **subescena vello** que el nodo `exit` pinta cada
//! frame mientras vive (vía [`AnimRegistry::live_exit_nodes`] +
//! [`AnimRegistry::store_live_exit`]); cuando la key desaparece, esa subescena
//! retenida se promueve a **fantasma** y [`AnimRegistry::replay_ghosts`] la
//! reproduce con opacidad decreciente hasta que el reloj se agota.
//!
//! **Cross-fade real (`AnimatedSwitcher`).** Un nodo puede declarar
//! ([`crate::View::animated_switch`]) una `key` estable + una **variante** de
//! contenido. Cuando la variante cambia entre frames, el runtime promueve el
//! contenido anterior (retenido en `live` el frame previo) a fantasma
//! (fade-out) y arranca el subárbol nuevo desde alpha 0 (fade-in), en el mismo
//! rect — una transición entre dos identidades distintas reusando la misma
//! infra de ghosts del `exit`, sin tener que combinar enter+exit de dos keys.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill, Mix};
use vello::Scene;

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
    /// `true` si la **salida** de la key debe animar (fade-out): cuando el nodo
    /// desaparece del árbol, el runtime retiene la última subescena que pintó y
    /// la reproduce con opacidad decreciente durante `duration`, en vez de que
    /// el nodo se esfume de golpe. Tiene coste por frame (captura el subárbol
    /// mientras vive) — usar en pocos nodos (toasts, modales, paneles), no en
    /// cada fila de una lista grande.
    pub exit: bool,
    /// Transformación afín desde la que arrancar la **entrada** (`enter`). Por
    /// ej. `Some(Affine::scale(0.6))` da el "pop" del FAB; `Some(Affine::
    /// translate((0.0, 60.0)))` da slide-in vertical. Llega al target del nodo
    /// (`node.transform` o identidad) en `duration`. Sin efecto si `enter` es
    /// `false`. Combinable con el fade de entrada por defecto.
    pub enter_from_xf: Option<Affine>,
    /// Discriminador de **variante de contenido** para cross-fade real
    /// (Flutter `AnimatedSwitcher`). Cuando es `Some(v)` y `v` **cambia**
    /// entre frames bajo la misma `key`, el runtime promueve la subescena del
    /// contenido anterior a fantasma (fade-out) y hace fade-in del nuevo, en
    /// el mismo rect — una transición real entre dos identidades distintas, no
    /// la combinación enter+exit de dos keys. Implica captura `live` por frame
    /// (como `exit`). La primera aparición no cruza (sólo asienta la variante).
    pub switch: Option<u64>,
}

/// Ease-out cúbico, el default razonable para transiciones implícitas
/// (arranca rápido, frena suave). Copia local para no acoplar el compositor a
/// `llimphi-theme`; el caller puede pasar cualquier `fn(f32)->f32`.
pub fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// Declara que el **tamaño** de este nodo (CSS `width`/`height` /
/// Flutter `AnimatedSize`/Compose `animateContentSize()`) se anima de
/// forma implícita cuando cambia entre frames. Bloque 15 de
/// PARIDAD-FLUTTER (extensión faltante del Bloque 4).
///
/// A diferencia de [`Anim`] (que interpola props de **paint** después
/// del layout: fill/radius/alpha/transform), el tamaño tiene que estar
/// fijo **antes** del layout — siblings y hijos dependen del rect del
/// nodo. Por eso este registro vive aparte y el reconciler camina el
/// `View` tree **antes** de `mount`, parchando `style.size` con el
/// valor interpolado.
///
/// **Límite v1**: sólo anima cuando `style.size.width` y
/// `style.size.height` son ambas `Dimension::Length(_)`. Si una es
/// `Percent`/`Auto`, el nodo se monta tal cual sin animación (no hay
/// "tamaño en píxeles" estable para interpolar). El caller que quiera
/// animar un nodo flex debe declarar `length(...)` explícito.
#[derive(Clone, Copy, Debug)]
pub struct SizeAnim {
    pub key: u64,
    pub duration: Duration,
    pub easing: fn(f32) -> f32,
}

#[derive(Clone, Copy)]
struct SizeAnimEntry {
    from: (f32, f32),
    to: (f32, f32),
    start: Instant,
    duration: Duration,
    easing: fn(f32) -> f32,
}

impl SizeAnimEntry {
    /// Entrada "asentada" (from == to): no anima. Igual que
    /// `AnimEntry::settled`, usamos `duration: ZERO` para que `done(now)`
    /// devuelva `true` desde el frame 0 — así la primera aparición no
    /// pide más frames. Cuando llegue un target nuevo el reconciler
    /// sobreescribe `duration` con el de `SizeAnim`.
    fn settled(target: (f32, f32), now: Instant, _dur: Duration, easing: fn(f32) -> f32) -> Self {
        Self {
            from: target,
            to: target,
            start: now,
            duration: Duration::ZERO,
            easing,
        }
    }

    fn t(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32();
        let raw = (elapsed / self.duration.as_secs_f32()).clamp(0.0, 1.0);
        (self.easing)(raw)
    }

    fn value(&self, now: Instant) -> (f32, f32) {
        let t = self.t(now);
        let (fw, fh) = self.from;
        let (tw, th) = self.to;
        (fw + (tw - fw) * t, fh + (th - fh) * t)
    }

    fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.duration
    }
}

/// Registro de animaciones implícitas de **tamaño**, vivo entre
/// frames. El runtime mantiene una instancia y llama
/// [`reconcile_size_anim`] en cada redraw **antes** del mount/layout.
#[derive(Default)]
pub struct SizeAnimRegistry {
    entries: HashMap<u64, SizeAnimEntry>,
}

impl SizeAnimRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Para tests: hay animación viva para esa key.
    pub fn is_animating(&self, key: u64, now: Instant) -> bool {
        self.entries.get(&key).map(|e| !e.done(now)).unwrap_or(false)
    }
}

/// Lee `(width, height)` en píxeles si **ambos** son
/// `Dimension::Length(_)`. Devuelve `None` si alguno es `Auto`,
/// `Percent`, etc. — esos nodos no se animan en v1. (taffy 0.9 esconde
/// las variantes detrás de un `CompactLength`; chequeamos por tag.)
fn try_extract_length_size(
    style: &llimphi_layout::Style,
) -> Option<(f32, f32)> {
    use llimphi_layout::taffy::CompactLength;
    let w = style.size.width;
    let h = style.size.height;
    if w.tag() == CompactLength::LENGTH_TAG && h.tag() == CompactLength::LENGTH_TAG {
        Some((w.value(), h.value()))
    } else {
        None
    }
}

fn patch_length_size(style: &mut llimphi_layout::Style, size: (f32, f32)) {
    use llimphi_layout::taffy::Dimension;
    style.size.width = Dimension::length(size.0);
    style.size.height = Dimension::length(size.1);
}

/// Recorre el `View` tree y, para cada nodo con [`SizeAnim`], reconcila
/// su `style.size` con el registry: si cambió el objetivo, arranca un
/// tween; si está animando, parcha `style.size` con el valor
/// interpolado. Devuelve `true` si alguna animación de tamaño sigue
/// viva → el runtime debe pedir otro redraw.
///
/// **Cuándo llamarlo**: el runtime lo invoca tras `A::view(model)` y
/// **antes** de `mount`/`compute`, así el layout cascade ve el tamaño
/// interpolado en vez del objetivo crudo (siblings y hijos reflowean
/// suave).
///
/// Las keys no vistas este frame se descartan al final — un nodo que se
/// va deja de animar (mismo comportamiento que [`AnimRegistry::reconcile`]).
pub fn reconcile_size_anim<Msg>(
    view: &mut crate::View<Msg>,
    reg: &mut SizeAnimRegistry,
    now: Instant,
) -> bool {
    let mut seen: Vec<u64> = Vec::new();
    let animating = reconcile_size_anim_inner(view, reg, now, &mut seen);
    if reg.entries.len() != seen.len() {
        reg.entries.retain(|k, _| seen.contains(k));
    }
    animating
}

fn reconcile_size_anim_inner<Msg>(
    view: &mut crate::View<Msg>,
    reg: &mut SizeAnimRegistry,
    now: Instant,
    seen: &mut Vec<u64>,
) -> bool {
    let mut animating = false;
    if let Some(sa) = view.animated_size {
        if let Some(target) = try_extract_length_size(&view.style) {
            seen.push(sa.key);
            let entry = reg
                .entries
                .entry(sa.key)
                .or_insert_with(|| SizeAnimEntry::settled(target, now, sa.duration, sa.easing));
            if entry.to != target {
                // Cambió el objetivo: congelá el valor actual como nuevo
                // origen y rearrancá el reloj — mismo patrón que el
                // `AnimRegistry` de props.
                entry.from = entry.value(now);
                entry.to = target;
                entry.start = now;
                entry.duration = sa.duration;
                entry.easing = sa.easing;
            }
            let interp = if entry.done(now) { entry.to } else { entry.value(now) };
            patch_length_size(&mut view.style, interp);
            if !entry.done(now) {
                animating = true;
            }
        }
    }
    for child in view.children.iter_mut() {
        if reconcile_size_anim_inner(child, reg, now, seen) {
            animating = true;
        }
    }
    animating
}

/// Foto de las props animables de un nodo en un frame. `alpha == None` ≡ nodo
/// opaco (1.0): es la convención de `View::alpha` y la usa el lerp para mezclar
/// hacia/desde "sin alpha explícito" sin tratarlo como un salto. Lo mismo para
/// `transform == None` ≡ identidad, así "sin transform" → "con transform" anima
/// desde la identidad (estilo CSS `transform: none` → `transform: scale(1.5)`).
#[derive(Clone, Copy, PartialEq)]
struct AnimSnapshot {
    fill: Option<Color>,
    radius: f64,
    alpha: Option<f32>,
    transform: Option<Affine>,
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

/// Lerp componente-a-componente de las 6 coefs del afín (m00, m10, m01, m11,
/// m02, m12). Es lo mismo que Flutter `MatrixTween`: no preserva una rotación
/// pura entre matrices muy distintas, pero alcanza para las animaciones UI
/// típicas (scale/translate/rotaciones chicas, slide-in, pop, hero).
#[inline]
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
        // `None` ≡ identidad: idem. Un lado sin transform se mezcla contra
        // `Affine::IDENTITY` en vez de saltar, así "sin xf" → `scale(1.5)`
        // arranca desde scale(1) (Flutter/CSS hacen lo mismo). None↔None se
        // mantiene None (sin push_layer afín en paint).
        let transform = match (self.transform, to.transform) {
            (None, None) => None,
            (a, b) => {
                let from = a.unwrap_or(Affine::IDENTITY);
                let dst = b.unwrap_or(Affine::IDENTITY);
                Some(lerp_affine(from, dst, t))
            }
        };
        AnimSnapshot {
            fill,
            radius: lerp_f64(self.radius, to.radius, t),
            alpha,
            transform,
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

/// Subescena retenida de un nodo marcado para animar su salida, capturada por
/// el runtime el último frame que el nodo vivió. Mientras la key sigue presente
/// se refresca cada frame; cuando desaparece, se promueve a [`Ghost`].
struct LiveExit {
    scene: Scene,
    duration: Duration,
    easing: fn(f32) -> f32,
}

/// Un nodo que ya salió del árbol y se está desvaneciendo: su subescena retenida
/// + el reloj de fade-out.
struct Ghost {
    scene: Scene,
    start: Instant,
    duration: Duration,
    easing: fn(f32) -> f32,
}

impl Ghost {
    /// Opacidad actual del fantasma: `1 → 0` con easing aplicado.
    fn alpha(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 0.0;
        }
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32();
        let raw = (elapsed / self.duration.as_secs_f32()).clamp(0.0, 1.0);
        1.0 - (self.easing)(raw)
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
    /// Snapshots de los nodos `exit`/`switch` presentes (refrescados por el
    /// runtime tras el paint de cada frame). Membresía = "presente el frame
    /// anterior".
    live: HashMap<u64, LiveExit>,
    /// Nodos `exit` que ya desaparecieron (o contenido viejo de un `switch`)
    /// que se están desvaneciendo.
    ghosts: HashMap<u64, Ghost>,
    /// Última variante vista por cada key con `switch` — para detectar el
    /// cambio de contenido que dispara el cross-fade.
    variants: HashMap<u64, u64>,
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
        // Keys presentes que requieren captura `live` y tracking de vanish
        // (exit O switch). Membresía = "vive este frame".
        let mut present_live: Vec<u64> = Vec::new();
        // Sólo keys `exit` puras: su reaparición CANCELA el fade-out. Las de
        // `switch` están presentes todos los frames y su ghost (contenido
        // viejo) NO debe cancelarse por presencia.
        let mut present_exit_only: Vec<u64> = Vec::new();
        for node in &mut mounted.nodes {
            let Some(anim) = node.anim else { continue };
            seen.push(anim.key);
            let target = AnimSnapshot {
                fill: node.fill,
                radius: node.radius,
                alpha: node.alpha,
                transform: node.transform,
            };
            // Detección de cross-fade (switch) ANTES de tomar prestado
            // `entries`: si la variante cambió, el contenido viejo retenido en
            // `live` (del frame anterior) se promueve a fantasma (fade-out) y
            // el nodo nuevo arranca su fade-in desde alpha 0.
            let mut switched = false;
            if anim.exit {
                present_live.push(anim.key);
                present_exit_only.push(anim.key);
            } else if let Some(variant) = anim.switch {
                present_live.push(anim.key);
                if let Some(prev) = self.variants.insert(anim.key, variant) {
                    if prev != variant {
                        switched = true;
                        if let Some(le) = self.live.remove(&anim.key) {
                            self.ghosts.insert(
                                anim.key,
                                Ghost {
                                    scene: le.scene,
                                    start: now,
                                    duration: le.duration,
                                    easing: le.easing,
                                },
                            );
                        }
                    }
                }
            }
            let entry = self.entries.entry(anim.key).or_insert_with(|| {
                // Primera aparición. Con `enter`, arranca un tween de opacidad
                // 0 → objetivo (fade-in); si además hay `enter_from_xf`, también
                // arranca de esa transform → target.transform (scale-in/slide-in).
                if anim.enter {
                    let from = AnimSnapshot {
                        alpha: Some(0.0),
                        transform: anim.enter_from_xf.or(target.transform),
                        ..target
                    };
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
            if switched {
                // Cross-fade: el contenido nuevo entra desde transparente
                // (el viejo ya quedó como fantasma desvaneciéndose encima).
                entry.from = AnimSnapshot {
                    alpha: Some(0.0),
                    ..target
                };
                entry.to = target;
                entry.start = now;
                entry.duration = anim.duration;
                entry.easing = anim.easing;
            } else if entry.to != target {
                // Cambió el objetivo: congelá el valor actual como nuevo origen
                // y rearrancá el reloj hacia el objetivo nuevo.
                entry.from = entry.value(now);
                entry.to = target;
                entry.start = now;
                entry.duration = anim.duration;
                entry.easing = anim.easing;
            }
            // Al terminar aterriza EXACTO en el objetivo (incluido `alpha:
            // None` / `transform: None`, que evita capa de opacidad residual o
            // un push_layer afín espurio frame a frame).
            let v = if entry.done(now) { entry.to } else { entry.value(now) };
            node.fill = v.fill;
            node.radius = v.radius;
            node.alpha = v.alpha;
            node.transform = v.transform;
            if !entry.done(now) {
                animating = true;
            }
        }
        if self.entries.len() != seen.len() {
            self.entries.retain(|k, _| seen.contains(k));
        }
        // Las variantes de keys que ya no aparecen se descartan (si la key
        // vuelve, su primera aparición re-asienta sin cross-fade).
        if self.variants.len() != seen.len() {
            self.variants.retain(|k, _| seen.contains(k));
        }

        // Salidas (fade-out). Una key `exit`/`switch` presente el frame anterior
        // (vive en `live`) que ya no aparece → se promueve a fantasma con su
        // última subescena retenida. Si una key `exit` con fantasma reaparece,
        // se cancela el fade (no las de `switch`: su fantasma es contenido viejo
        // que debe seguir desvaneciéndose aunque la key siga presente). Por
        // último, descartamos los fantasmas cuyo reloj se agotó.
        let vanished: Vec<u64> = self
            .live
            .keys()
            .filter(|k| !present_live.contains(k))
            .copied()
            .collect();
        for key in vanished {
            if let Some(le) = self.live.remove(&key) {
                self.ghosts.insert(
                    key,
                    Ghost {
                        scene: le.scene,
                        start: now,
                        duration: le.duration,
                        easing: le.easing,
                    },
                );
            }
        }
        for key in &present_exit_only {
            self.ghosts.remove(key);
        }
        self.ghosts.retain(|_, g| !g.done(now));
        animating || !self.ghosts.is_empty()
    }

    /// Nodos `exit` presentes este frame que el runtime debe **capturar**: por
    /// cada uno devuelve `(idx, subtree_end, key)` para pintar su subárbol en
    /// una subescena con [`crate::paint_range`] y entregarla a
    /// [`Self::store_live_exit`]. Llamar DESPUÉS de `paint` (cuando el árbol y
    /// la geometría ya están firmes).
    pub fn live_exit_nodes<Msg>(&self, mounted: &Mounted<Msg>) -> Vec<(usize, usize, u64)> {
        mounted
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(idx, n)| {
                n.anim
                    .filter(|a| a.exit || a.switch.is_some())
                    .map(|a| (idx, n.subtree_end, a.key))
            })
            .collect()
    }

    /// Guarda (o refresca) la subescena retenida de un nodo `exit` presente. El
    /// runtime la captura con [`crate::paint_range`] tras el paint. `duration` y
    /// `easing` se heredan al fantasma cuando la key desaparezca.
    pub fn store_live_exit(
        &mut self,
        key: u64,
        scene: Scene,
        duration: Duration,
        easing: fn(f32) -> f32,
    ) {
        self.live.insert(key, LiveExit { scene, duration, easing });
    }

    /// Reproduce los fantasmas activos sobre `scene`, cada uno con su opacidad
    /// decreciente, clipeados al viewport `(w, h)`. Llamar DESPUÉS del paint
    /// principal (van por encima). Devuelve `true` si queda algún fantasma vivo
    /// (el runtime ya lo sabe por [`Self::reconcile`], pero es cómodo).
    pub fn replay_ghosts(&mut self, scene: &mut Scene, now: Instant, w: f32, h: f32) -> bool {
        if self.ghosts.is_empty() {
            return false;
        }
        let clip = Rect::new(0.0, 0.0, w as f64, h as f64);
        for g in self.ghosts.values() {
            let a = g.alpha(now);
            if a <= 0.0 {
                continue;
            }
            scene.push_layer(Fill::NonZero, Mix::Normal, a, Affine::IDENTITY, &clip);
            scene.append(&g.scene, None);
            scene.pop_layer();
        }
        true
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

    /// Monta un nodo `exit` (key=7) y devuelve su `Mounted`.
    fn one_exit() -> Mounted<()> {
        let v = View::<()>::new(Style::default())
            .fill(rgba(10, 20, 30))
            .animated_exit(7, Duration::from_millis(200));
        let mut layout = LayoutTree::new();
        mount(&mut layout, v)
    }

    /// Árbol vacío de nodos animados (la key `exit` ya no aparece).
    fn empty() -> Mounted<()> {
        let v = View::<()>::new(Style::default()).fill(rgba(9, 9, 9));
        let mut layout = LayoutTree::new();
        mount(&mut layout, v)
    }

    #[test]
    fn fade_out_de_salida_promueve_fantasma_y_lo_descarta_al_terminar() {
        let mut reg = AnimRegistry::new();
        let t0 = Instant::now();
        // Frame 1: el nodo exit está presente. No anima por sí solo, y el
        // runtime captura su subescena (acá una vacía de prueba).
        let mut m = one_exit();
        let animating = reg.reconcile(&mut m, t0);
        assert!(!animating, "presente y quieto no anima");
        reg.store_live_exit(7, Scene::new(), Duration::from_millis(200), ease_out_cubic);
        // Frame 2: la key desaparece → se promueve a fantasma y pide frames.
        let mut m = empty();
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(10));
        assert!(animating, "un fantasma vivo mantiene el ticker");
        assert!(reg.replay_ghosts(&mut Scene::new(), t0 + Duration::from_millis(10), 100.0, 100.0));
        // Frame 3: pasada la duración el fantasma se descarta y el loop para.
        let mut m = empty();
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(300));
        assert!(!animating, "fantasma agotado → sin más frames");
        assert!(!reg.replay_ghosts(&mut Scene::new(), t0 + Duration::from_millis(300), 100.0, 100.0));
    }

    /// Monta un nodo `switch` (key=5) con la variante dada.
    fn one_switch(variant: u64) -> Mounted<()> {
        let v = View::<()>::new(Style::default())
            .fill(rgba(10, 20, 30))
            .animated_switch(5, variant, Duration::from_millis(200));
        let mut layout = LayoutTree::new();
        mount(&mut layout, v)
    }

    #[test]
    fn switch_de_variante_cruza_contenido() {
        let mut reg = AnimRegistry::new();
        let t0 = Instant::now();
        // Frame 1: variante 1, primera aparición → asienta, no cruza.
        let mut m = one_switch(1);
        assert!(!reg.reconcile(&mut m, t0), "primera aparición no cruza");
        // El runtime captura su subescena (de prueba, vacía).
        reg.store_live_exit(5, Scene::new(), Duration::from_millis(200), ease_out_cubic);
        // Frame 2: variante 2 → cross-fade. El contenido nuevo arranca casi
        // transparente y hay un fantasma del contenido viejo desvaneciéndose.
        let mut m = one_switch(2);
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(10));
        assert!(animating, "el cross-fade pide frames");
        let a = m.nodes[0].alpha.expect("alpha de fade-in");
        assert!(a < 0.3, "el contenido nuevo arranca casi transparente: {a}");
        assert!(
            reg.replay_ghosts(&mut Scene::new(), t0 + Duration::from_millis(10), 100.0, 100.0),
            "hay un fantasma del contenido viejo"
        );
        // Re-captura del frame 2 (lo haría el runtime tras el paint).
        reg.store_live_exit(5, Scene::new(), Duration::from_millis(200), ease_out_cubic);
        // Frame 3: misma variante, pasada la duración → asentado y sin fantasma.
        let mut m = one_switch(2);
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(400));
        assert!(!animating, "asentado tras la duración");
        assert_eq!(m.nodes[0].alpha, None, "opaco exacto sin capa residual");
        assert!(
            !reg.replay_ghosts(&mut Scene::new(), t0 + Duration::from_millis(400), 100.0, 100.0),
            "fantasma agotado"
        );
    }

    #[test]
    fn switch_misma_variante_no_cruza() {
        let mut reg = AnimRegistry::new();
        let t0 = Instant::now();
        let mut m = one_switch(1);
        reg.reconcile(&mut m, t0);
        reg.store_live_exit(5, Scene::new(), Duration::from_millis(200), ease_out_cubic);
        // Misma variante en el frame siguiente: ni fade-in ni fantasma.
        let mut m = one_switch(1);
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(10));
        assert!(!animating, "sin cambio de variante no cruza");
        assert_eq!(m.nodes[0].alpha, None, "el contenido sigue opaco");
        assert!(!reg.replay_ghosts(&mut Scene::new(), t0 + Duration::from_millis(10), 100.0, 100.0));
    }

    #[test]
    fn reaparecer_cancela_el_fantasma() {
        let mut reg = AnimRegistry::new();
        let t0 = Instant::now();
        let mut m = one_exit();
        reg.reconcile(&mut m, t0);
        reg.store_live_exit(7, Scene::new(), Duration::from_millis(200), ease_out_cubic);
        // Se va → fantasma.
        let mut m = empty();
        assert!(reg.reconcile(&mut m, t0 + Duration::from_millis(10)));
        // Reaparece a mitad del fade → el fantasma se cancela (no hay doble).
        let mut m = one_exit();
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(100));
        assert!(!animating, "al reaparecer no queda fantasma");
        assert!(!reg.replay_ghosts(&mut Scene::new(), t0 + Duration::from_millis(100), 100.0, 100.0));
    }

    /// Monta un nodo con un transform afín explícito + anim(key=1).
    fn one_xf(xf: Affine) -> Mounted<()> {
        let v = View::<()>::new(Style::default())
            .transform(xf)
            .animated(1, Duration::from_millis(200));
        let mut layout = LayoutTree::new();
        mount(&mut layout, v)
    }

    /// Monta un nodo sin transform pero con anim_enter_from (scale 0.5 → 1.0).
    fn one_pop_in() -> Mounted<()> {
        let v = View::<()>::new(Style::default())
            .fill(rgba(1, 2, 3))
            .animated_enter_from(2, Duration::from_millis(200), Affine::scale(0.5));
        let mut layout = LayoutTree::new();
        mount(&mut layout, v)
    }

    #[test]
    fn cambio_de_transform_interpola_y_pide_frames() {
        let mut reg = AnimRegistry::new();
        let t0 = Instant::now();
        // Frame 1: identidad → se asienta sin animar.
        let mut m = one_xf(Affine::IDENTITY);
        assert!(!reg.reconcile(&mut m, t0), "primera aparición no anima");
        // Frame 2: la view ahora pide scale(2.0) → arranca tween.
        let mut m = one_xf(Affine::scale(2.0));
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(50));
        assert!(animating, "al cambiar la xf debe pedir frames");
        // Frame 3: a mitad, el m00 está entre 1.0 y 2.0.
        let mut m = one_xf(Affine::scale(2.0));
        reg.reconcile(&mut m, t0 + Duration::from_millis(150));
        let c = m.nodes[0].transform.expect("transform").as_coeffs();
        assert!(c[0] > 1.0 && c[0] < 2.0, "m00 intermedio: {}", c[0]);
        assert!(c[3] > 1.0 && c[3] < 2.0, "m11 intermedio: {}", c[3]);
    }

    #[test]
    fn transform_al_terminar_llega_exacto() {
        let mut reg = AnimRegistry::new();
        let t0 = Instant::now();
        let mut m = one_xf(Affine::IDENTITY);
        reg.reconcile(&mut m, t0);
        let mut m = one_xf(Affine::translate((10.0, 20.0)));
        reg.reconcile(&mut m, t0 + Duration::from_millis(50));
        // Pasada la duración: aterriza exacto en la xf objetivo.
        let mut m = one_xf(Affine::translate((10.0, 20.0)));
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(400));
        assert!(!animating);
        let c = m.nodes[0].transform.expect("xf").as_coeffs();
        assert!((c[4] - 10.0).abs() < 1e-9, "tx exacto: {}", c[4]);
        assert!((c[5] - 20.0).abs() < 1e-9, "ty exacto: {}", c[5]);
    }

    #[test]
    fn pop_in_arranca_desde_la_xf_inicial_y_aterriza_sin_xf() {
        let mut reg = AnimRegistry::new();
        let t0 = Instant::now();
        // Frame 1: el nodo no declara `.transform` pero sí `enter_from`. La
        // PRIMERA aparición arranca CON xf = scale(0.5) (lo que pide el caller)
        // y debe pedir frames.
        let mut m = one_pop_in();
        let animating = reg.reconcile(&mut m, t0);
        assert!(animating, "pop-in anima desde el primer frame");
        let c = m.nodes[0].transform.expect("xf inicial").as_coeffs();
        assert!((c[0] - 0.5).abs() < 1e-9, "arranca en scale 0.5: {}", c[0]);
        // Frame intermedio: el m00 ya creció hacia 1.0.
        let mut m = one_pop_in();
        reg.reconcile(&mut m, t0 + Duration::from_millis(100));
        let c = m.nodes[0].transform.expect("xf medio").as_coeffs();
        assert!(c[0] > 0.5 && c[0] < 1.0, "scale intermedio: {}", c[0]);
        // Frame final: aterriza en None (sin xf residual), igual que alpha.
        let mut m = one_pop_in();
        let animating = reg.reconcile(&mut m, t0 + Duration::from_millis(400));
        assert!(!animating, "asentado");
        assert_eq!(m.nodes[0].transform, None, "sin xf residual al asentarse");
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

    // ─── Bloque 15: tests de SizeAnim / animateContentSize ───

    fn sized_view(key: u64, w: f32, h: f32, dur_ms: u64) -> View<()> {
        use llimphi_layout::taffy::prelude::{length, Size};
        let mut style = Style::default();
        style.size = Size { width: length(w), height: length(h) };
        View::<()>::new(style).animated_size(key, Duration::from_millis(dur_ms))
    }

    #[test]
    fn size_anim_primera_aparicion_no_anima() {
        let mut reg = SizeAnimRegistry::new();
        let mut v = sized_view(1, 100.0, 80.0, 200);
        let now = Instant::now();
        let animating = reconcile_size_anim(&mut v, &mut reg, now);
        assert!(!animating, "primera vez: sin animación");
        // El style.size queda intacto (length(100, 80)).
        let (w, h) = (v.style.size.width.value(), v.style.size.height.value());
        assert_eq!((w, h), (100.0, 80.0));
    }

    #[test]
    fn size_anim_cambia_target_interpola() {
        let mut reg = SizeAnimRegistry::new();
        let t0 = Instant::now();
        // Frame 1: target = 100×80, se asienta.
        let mut v = sized_view(1, 100.0, 80.0, 200);
        reconcile_size_anim(&mut v, &mut reg, t0);
        // Frame 2: target nuevo = 200×160. En el frame que se detecta el
        // cambio arranca el reloj — todavía pinta cerca del origen.
        let mut v = sized_view(1, 200.0, 160.0, 200);
        let animating = reconcile_size_anim(&mut v, &mut reg, t0);
        assert!(animating, "cambio de target: pide frames");
        let (w, h) = (v.style.size.width.value(), v.style.size.height.value());
        assert!(w < 200.0 && w >= 100.0, "ancho intermedio: {w}");
        assert!(h < 160.0 && h >= 80.0, "alto intermedio: {h}");
        // Frame 3: 100 ms (mitad del tween).
        let mut v = sized_view(1, 200.0, 160.0, 200);
        let animating = reconcile_size_anim(&mut v, &mut reg, t0 + Duration::from_millis(100));
        assert!(animating, "a mitad del tween sigue animando");
        let (w, h) = (v.style.size.width.value(), v.style.size.height.value());
        assert!(w > 100.0 && w < 200.0, "ancho mitad-tween: {w}");
        assert!(h > 80.0 && h < 160.0, "alto mitad-tween: {h}");
    }

    #[test]
    fn size_anim_termina_y_se_detiene() {
        let mut reg = SizeAnimRegistry::new();
        let t0 = Instant::now();
        let mut v = sized_view(1, 100.0, 80.0, 200);
        reconcile_size_anim(&mut v, &mut reg, t0);
        let mut v = sized_view(1, 200.0, 160.0, 200);
        reconcile_size_anim(&mut v, &mut reg, t0); // arranca
        // Pasada la duración: aterriza exacto en el objetivo y no pide más.
        let mut v = sized_view(1, 200.0, 160.0, 200);
        let animating = reconcile_size_anim(&mut v, &mut reg, t0 + Duration::from_millis(400));
        assert!(!animating);
        assert_eq!(
            (v.style.size.width.value(), v.style.size.height.value()),
            (200.0, 160.0),
        );
    }

    #[test]
    fn size_anim_no_animable_si_tamano_no_es_length() {
        // Si el caller declara percent o auto, el reconciler lo deja pasar
        // sin tracking — no hay valor en píxeles estable para interpolar.
        use llimphi_layout::taffy::prelude::{percent, Dimension, Size};
        let mut reg = SizeAnimRegistry::new();
        let mut style = Style::default();
        style.size = Size { width: percent(0.5), height: Dimension::auto() };
        let mut v = View::<()>::new(style).animated_size(1, Duration::from_millis(200));
        let animating = reconcile_size_anim(&mut v, &mut reg, Instant::now());
        assert!(!animating);
        // El size no se tocó: width sigue siendo percent (no LENGTH_TAG).
        use llimphi_layout::taffy::CompactLength;
        assert_ne!(v.style.size.width.tag(), CompactLength::LENGTH_TAG);
    }

    #[test]
    fn size_anim_descarta_keys_no_vistas() {
        let mut reg = SizeAnimRegistry::new();
        let now = Instant::now();
        let mut v = sized_view(42, 50.0, 50.0, 200);
        reconcile_size_anim(&mut v, &mut reg, now);
        assert_eq!(reg.entries.len(), 1);
        // Frame sin animated_size: la entrada se descarta.
        let mut v: View<()> = View::<()>::new(Style::default());
        reconcile_size_anim(&mut v, &mut reg, now);
        assert_eq!(reg.entries.len(), 0);
    }
}
