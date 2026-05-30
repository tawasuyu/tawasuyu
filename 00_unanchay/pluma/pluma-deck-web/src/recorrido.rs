//! Vista-web **espacial** — binding DOM del modo Recorrido (tipo Prezi).
//!
//! Espejo web del frontend Llimphi (`pluma-deck-recorrido-llimphi`): la lógica
//! de cámara/ruta/gesto vive entera en `pluma-deck-core`; aquí sólo se traduce
//! pointer/wheel/teclado → llamadas al core y se aplica la cámara como **un
//! único `transform` CSS** sobre un contenedor `mundo` (estilo impress.js).
//!
//! A diferencia del strip lineal ([`crate::Deck`], `translate3d` 1D), el modo
//! espacial coloca cada marco en coordenadas de mundo dentro de `mundo` y
//! mueve la cámara sobre todos ellos. El vuelo guiado entre pasos se delega a
//! una **transición CSS** del transform (el core sólo provee la cámara objetivo
//! vía `fit`), igual que el strip delega su deslizamiento a `transition`.
//!
//! Contrato DOM (el caller provee):
//! ```html
//! <div class="recorrido-viewport">
//!   <div class="recorrido-mundo">
//!     <div class="recorrido-marco" data-x="0"   data-y="0" data-w="640" data-h="400">…</div>
//!     <div class="recorrido-marco" data-x="900" data-y="0" data-w="640" data-h="400" data-rot="0.1">…</div>
//!   </div>
//! </div>
//! ```
//! Cada `.recorrido-marco` lleva su rect de **mundo** en `data-{x,y,w,h}` (px) y
//! un giro opcional `data-rot` (radianes). El orden DOM define la ruta. El
//! binding posiciona los marcos y mueve la cámara; el contenido HTML interno es
//! libre (texto, `<img>`, lo que sea).

use std::cell::RefCell;
use std::rc::Rc;

use pluma_deck_core::{Camara, ContenidoMarco, Marco, Recorrido, RecorridoState, Rect, DURACION_PASO_S};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, KeyboardEvent, PointerEvent, WheelEvent};

/// Base de zoom por "clic" de rueda (igual criterio que tullpu y el frontend Llimphi).
pub const ZOOM_BASE: f64 = 1.1;

/// Curva del vuelo entre pasos (misma que el strip lineal: salida suave).
const EASE_VUELO: &str = "cubic-bezier(0.22, 0.61, 0.36, 1)";

#[derive(Clone)]
pub struct RecorridoWeb {
    viewport: HtmlElement,
    mundo: HtmlElement,
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    rec: Recorrido,
    state: RecorridoState,
    /// `Some((x,y))` = paneando desde esa última posición de pointer.
    arrastrando: Option<(f64, f64)>,
    on_change: Option<Box<dyn FnMut(usize)>>,
}

impl RecorridoWeb {
    /// Monta el recorrido sobre `viewport` (clip) y `mundo` (contenedor
    /// transformado). Lee los `.recorrido-marco` hijos de `mundo`, los posiciona
    /// en coordenadas de mundo y encuadra el primero.
    pub fn mount(viewport: HtmlElement, mundo: HtmlElement) -> Result<Self, JsValue> {
        estilo(&viewport, "overflow", "hidden");
        estilo(&viewport, "position", "relative");
        estilo(&viewport, "touch-action", "none");
        estilo(&viewport, "user-select", "none");
        estilo(&mundo, "position", "absolute");
        estilo(&mundo, "left", "0");
        estilo(&mundo, "top", "0");
        estilo(&mundo, "transform-origin", "0 0");
        estilo(&mundo, "will-change", "transform");

        let rec = leer_marcos(&mundo);
        let mut state = RecorridoState::new();
        let panel = panel_de(&viewport);
        state.saltar_a_paso(&rec, 0, panel);

        let inner = Rc::new(RefCell::new(Inner { rec, state, arrastrando: None, on_change: None }));
        aplicar_camara(&mundo, &inner.borrow().state.camara, panel, false);

        let this = Self { viewport, mundo, inner };
        this.install_pointer()?;
        this.install_wheel()?;
        this.install_keys()?;
        Ok(this)
    }

    fn panel(&self) -> Rect {
        panel_de(&self.viewport)
    }

    fn aplicar(&self, transicion: bool) {
        let panel = self.panel();
        aplicar_camara(&self.mundo, &self.inner.borrow().state.camara, panel, transicion);
    }

    /// Vuela a encuadrar el paso `idx` (con transición si `smooth`). Notifica
    /// `on_change` si cambió el paso.
    pub fn goto(&self, idx: usize, smooth: bool) {
        let panel = self.panel();
        let mut i = self.inner.borrow_mut();
        let antes = i.state.paso;
        let Inner { rec, state, .. } = &mut *i;
        state.saltar_a_paso(rec, idx, panel);
        let ahora = state.paso;
        drop(i);
        self.aplicar(smooth);
        if ahora != antes {
            if let Some(cb) = self.inner.borrow_mut().on_change.as_mut() {
                cb(ahora);
            }
        }
    }

    /// Paso siguiente (clamp al final). `true` si se movió.
    pub fn siguiente(&self) -> bool {
        let i = self.inner.borrow();
        let n = i.rec.n_pasos();
        let p = i.state.paso;
        drop(i);
        if n == 0 || p + 1 >= n {
            return false;
        }
        self.goto(p + 1, true);
        true
    }

    /// Paso anterior (clamp en 0). `true` si se movió.
    pub fn anterior(&self) -> bool {
        let p = self.inner.borrow().state.paso;
        if p == 0 {
            return false;
        }
        self.goto(p - 1, true);
        true
    }

    pub fn paso_actual(&self) -> usize {
        self.inner.borrow().state.paso
    }

    pub fn on_change<F: FnMut(usize) + 'static>(&self, cb: F) {
        self.inner.borrow_mut().on_change = Some(Box::new(cb));
    }

    // ---- Cableado de eventos --------------------------------------------

    fn install_pointer(&self) -> Result<(), JsValue> {
        // down: arranca paneo (cancela cualquier vuelo, control manual).
        {
            let this = self.clone();
            let cb = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
                this.inner.borrow_mut().arrastrando = Some((e.client_x() as f64, e.client_y() as f64));
                let _ = this.viewport.set_pointer_capture(e.pointer_id());
            });
            self.viewport
                .add_event_listener_with_callback("pointerdown", cb.as_ref().unchecked_ref())?;
            cb.forget();
        }
        // move: si hay arrastre, panea por el delta de pantalla (sin transición).
        {
            let this = self.clone();
            let cb = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
                let (x, y) = (e.client_x() as f64, e.client_y() as f64);
                let mut i = this.inner.borrow_mut();
                let Some((px, py)) = i.arrastrando else { return };
                i.state.arrastrar_delta(x - px, y - py);
                i.arrastrando = Some((x, y));
                drop(i);
                this.aplicar(false);
            });
            self.viewport
                .add_event_listener_with_callback("pointermove", cb.as_ref().unchecked_ref())?;
            cb.forget();
        }
        // up/cancel/leave: fin del paneo.
        for ev in ["pointerup", "pointercancel", "pointerleave"] {
            let this = self.clone();
            let cb = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
                this.inner.borrow_mut().arrastrando = None;
                let _ = this.viewport.release_pointer_capture(e.pointer_id());
            });
            self.viewport
                .add_event_listener_with_callback(ev, cb.as_ref().unchecked_ref())?;
            cb.forget();
        }
        Ok(())
    }

    fn install_wheel(&self) -> Result<(), JsValue> {
        let this = self.clone();
        let cb = Closure::<dyn FnMut(WheelEvent)>::new(move |e: WheelEvent| {
            e.prevent_default();
            let rect = this.viewport.get_bounding_client_rect();
            let cursor = (e.client_x() as f64 - rect.left(), e.client_y() as f64 - rect.top());
            // deltaY>0 ⇒ scroll abajo ⇒ alejar (convención CSS, igual que tullpu).
            let mult = ZOOM_BASE.powf(-e.delta_y().signum());
            let panel = this.panel();
            this.inner.borrow_mut().state.wheel(mult, cursor, panel);
            this.aplicar(false);
        });
        let opts = web_sys::AddEventListenerOptions::new();
        opts.set_passive(false);
        self.viewport.add_event_listener_with_callback_and_add_event_listener_options(
            "wheel",
            cb.as_ref().unchecked_ref(),
            &opts,
        )?;
        cb.forget();
        Ok(())
    }

    fn install_keys(&self) -> Result<(), JsValue> {
        let Some(window) = web_sys::window() else { return Ok(()) };
        let this = self.clone();
        let cb = Closure::<dyn FnMut(KeyboardEvent)>::new(move |e: KeyboardEvent| {
            let movido = match e.key().as_str() {
                "ArrowRight" | "ArrowDown" | " " | "Spacebar" | "Enter" => this.siguiente(),
                "ArrowLeft" | "ArrowUp" => this.anterior(),
                _ => return,
            };
            if movido {
                e.prevent_default();
            }
        });
        window.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())?;
        cb.forget();
        Ok(())
    }
}

// ---- Lectura del DOM + geometría -----------------------------------------

/// Construye el `Recorrido` del core leyendo los `.recorrido-marco` hijos de
/// `mundo`, y posiciona cada uno en coordenadas de mundo. El orden DOM define
/// la ruta; los ids se asignan `1..=n`.
fn leer_marcos(mundo: &HtmlElement) -> Recorrido {
    let mut rec = Recorrido::new();
    let hijos = mundo.children();
    for idx in 0..hijos.length() {
        let Some(el) = hijos.item(idx).and_then(|e| e.dyn_into::<HtmlElement>().ok()) else { continue };
        let x = attr_f64(&el, "data-x", 0.0);
        let y = attr_f64(&el, "data-y", 0.0);
        let w = attr_f64(&el, "data-w", 640.0);
        let h = attr_f64(&el, "data-h", 400.0);
        let rot = attr_f64(&el, "data-rot", 0.0);
        // El marco vive en coordenadas de mundo dentro de `mundo`: left/top/size
        // en px-mundo y su giro propio alrededor del centro (transform-origin
        // por defecto = center), espejando el render Llimphi.
        estilo(&el, "position", "absolute");
        estilo(&el, "left", &format!("{x}px"));
        estilo(&el, "top", &format!("{y}px"));
        estilo(&el, "width", &format!("{w}px"));
        estilo(&el, "height", &format!("{h}px"));
        estilo(&el, "box-sizing", "border-box");
        if rot != 0.0 {
            estilo(&el, "transform", &format!("rotate({rot}rad)"));
        }
        let id = (idx + 1) as u64;
        rec.agregar_marco(Marco::new(id, Rect::new(x, y, w, h), ContenidoMarco::Vacio).con_giro(rot));
        rec.pasos.push(id);
    }
    rec
}

fn panel_de(viewport: &HtmlElement) -> Rect {
    Rect::new(0.0, 0.0, viewport.client_width() as f64, viewport.client_height() as f64)
}

/// Construye el `transform` CSS de la cámara: réplica exacta de
/// `Camara::world_to_screen` (`centro_panel + zoom·R(-rot)·(mundo - centro)`)
/// como cadena aplicable a `mundo` con `transform-origin: 0 0`.
fn camara_css(cam: &Camara, panel: Rect) -> String {
    let (pcx, pcy) = panel.centro();
    format!(
        "translate({pcx}px,{pcy}px) scale({z}) rotate({r}rad) translate({cx}px,{cy}px)",
        z = cam.zoom,
        r = -cam.rot_rad,
        cx = -cam.centro.0,
        cy = -cam.centro.1,
    )
}

fn aplicar_camara(mundo: &HtmlElement, cam: &Camara, panel: Rect, transicion: bool) {
    let trans = if transicion {
        format!("transform {}ms {EASE_VUELO}", (DURACION_PASO_S * 1000.0) as i64)
    } else {
        "none".to_string()
    };
    estilo(mundo, "transition", &trans);
    estilo(mundo, "transform", &camara_css(cam, panel));
}

fn estilo(el: &HtmlElement, prop: &str, val: &str) {
    let _ = el.style().set_property(prop, val);
}

fn attr_f64(el: &HtmlElement, name: &str, def: f64) -> f64 {
    el.get_attribute(name).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(def)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANEL: Rect = Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };

    #[test]
    fn camara_identidad_centra_el_origen() {
        // Cámara por defecto (centro 0,0 zoom 1 rot 0): el transform lleva el
        // origen de mundo al centro del panel.
        let css = camara_css(&Camara::default(), PANEL);
        assert_eq!(css, "translate(400px,300px) scale(1) rotate(-0rad) translate(-0px,-0px)");
    }

    #[test]
    fn camara_css_refleja_world_to_screen() {
        // Para varios puntos, evaluar el transform CSS a mano debe coincidir con
        // Camara::world_to_screen — el CSS es ese mismo afín.
        let cam = Camara::new((120.0, -40.0), 1.7, 0.3);
        for &(wx, wy) in &[(0.0, 0.0), (200.0, 50.0), (-80.0, 300.0)] {
            let esperado = cam.world_to_screen((wx, wy), PANEL);
            // Aplicar el transform manualmente: translate(pc)·scale·rotate(-rot)·translate(-c).
            let (pcx, pcy) = PANEL.centro();
            let (dx, dy) = (wx - cam.centro.0, wy - cam.centro.1);
            let (s, c) = (-cam.rot_rad).sin_cos();
            let (rx, ry) = (dx * c - dy * s, dx * s + dy * c);
            let got = (pcx + rx * cam.zoom, pcy + ry * cam.zoom);
            assert!((got.0 - esperado.0).abs() < 1e-9, "x: {got:?} vs {esperado:?}");
            assert!((got.1 - esperado.1).abs() < 1e-9, "y: {got:?} vs {esperado:?}");
        }
        // Y la cadena contiene los componentes esperados.
        let css = camara_css(&cam, PANEL);
        assert!(css.contains("scale(1.7)"), "{css}");
        assert!(css.contains("rotate(-0.3rad)"), "{css}");
    }
}
