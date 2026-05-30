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
/// Normalizador del `deltaY` de la rueda a "notches" de zoom: deltaY en modo
/// pixel ronda ±100 por muesca, así que dividir por esto da ~1 notch por muesca
/// y a la vez deja que el pinch de trackpad (deltas chicos) zoomee proporcional.
const WHEEL_NORM: f64 = 100.0;
/// Permanencia por defecto del modo presentador, en ms (espejo de `DWELL_S`).
pub const DWELL_MS: i32 = 2500;

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
    /// Handle del `setInterval` del modo presentador (autoplay), si está activo.
    autoplay_id: Option<i32>,
    /// Closure del intervalo — se guarda para mantenerla viva mientras corre.
    autoplay_cb: Option<Closure<dyn FnMut()>>,
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

        let inner = Rc::new(RefCell::new(Inner {
            rec,
            state,
            arrastrando: None,
            on_change: None,
            autoplay_id: None,
            autoplay_cb: None,
        }));
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

    /// Vuela a la **vista general** (aleja para encuadrar todo el lienzo). Gesto
    /// Prezi de "ver el mapa". No cambia el paso narrativo. Igual que `goto`, el
    /// vuelo es una transición CSS: se fija la cámara objetivo del core (instante)
    /// y el navegador la anima — el core nunca tickea `avanzar` en web.
    pub fn vista_general(&self) {
        let panel = self.panel();
        {
            let mut i = self.inner.borrow_mut();
            let Some(bbox) = i.rec.bbox() else { return };
            i.state.camara = Camara::fit(bbox, 0.0, panel);
        }
        self.aplicar(true);
    }

    pub fn paso_actual(&self) -> usize {
        self.inner.borrow().state.paso
    }

    /// Exporta el recorrido a un documento **HTML autocontenido** (un solo
    /// `.html` sin servidor ni `.wasm`): lee los `.recorrido-marco` vivos del
    /// DOM (geometría de mundo + su HTML interno) y emite un documento con CSS y
    /// un JS vanilla que replica la cámara del core (fit, zoom-a-cursor, pan,
    /// pasos, vista general). Para publicar la presentación offline.
    pub fn exportar_html(&self, titulo: &str) -> String {
        let hijos = self.mundo.children();
        let mut marcos = Vec::new();
        for idx in 0..hijos.length() {
            let Some(el) = hijos.item(idx).and_then(|e| e.dyn_into::<HtmlElement>().ok()) else {
                continue;
            };
            marcos.push(MarcoExport {
                x: attr_f64(&el, "data-x", 0.0),
                y: attr_f64(&el, "data-y", 0.0),
                w: attr_f64(&el, "data-w", 640.0),
                h: attr_f64(&el, "data-h", 400.0),
                rot: attr_f64(&el, "data-rot", 0.0),
                html: el.inner_html(),
            });
        }
        recorrido_a_html(titulo, &marcos)
    }

    /// `true` si el modo presentador (autoplay) está corriendo.
    pub fn autoplay_activo(&self) -> bool {
        self.inner.borrow().autoplay_id.is_some()
    }

    /// Arranca el modo presentador: cada `dur_paso + dwell_ms` avanza un paso
    /// solo (vuelve al inicio al llegar al final). Cancela cualquier autoplay
    /// previo. Espejo vivo del `setInterval` del HTML exportado.
    pub fn iniciar_autoplay(&self, dwell_ms: i32) {
        self.detener_autoplay();
        let Some(window) = web_sys::window() else { return };
        let this = self.clone();
        let cb = Closure::<dyn FnMut()>::new(move || {
            let (paso, n) = {
                let i = this.inner.borrow();
                (i.state.paso, i.rec.n_pasos())
            };
            if n == 0 {
                return;
            }
            this.goto(if paso + 1 < n { paso + 1 } else { 0 }, true);
        });
        let periodo = (DURACION_PASO_S * 1000.0) as i32 + dwell_ms.max(0);
        if let Ok(id) = window.set_interval_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(),
            periodo,
        ) {
            let mut i = self.inner.borrow_mut();
            i.autoplay_id = Some(id);
            i.autoplay_cb = Some(cb);
        }
    }

    /// Detiene el modo presentador (no-op si no estaba activo).
    pub fn detener_autoplay(&self) {
        let mut i = self.inner.borrow_mut();
        if let Some(id) = i.autoplay_id.take() {
            if let Some(w) = web_sys::window() {
                w.clear_interval_with_handle(id);
            }
        }
        i.autoplay_cb = None;
    }

    /// Alterna el modo presentador con el dwell por defecto. Devuelve el nuevo
    /// estado (`true` = corriendo).
    pub fn toggle_autoplay(&self) -> bool {
        if self.autoplay_activo() {
            self.detener_autoplay();
            false
        } else {
            self.iniciar_autoplay(DWELL_MS);
            true
        }
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
            // Proporcional al delta (no sólo al signo) para que el pinch de
            // trackpad sea suave; normalizado por `WHEEL_NORM` (un "notch" de
            // rueda ≈ ±100px en modo pixel) y acotado para que un golpe fuerte no
            // teletransporte el zoom.
            let pasos = (e.delta_y() / WHEEL_NORM).clamp(-3.0, 3.0);
            let mult = ZOOM_BASE.powf(-pasos);
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
                "Home" | "Escape" => {
                    this.vista_general();
                    true
                }
                "p" | "P" => {
                    this.toggle_autoplay();
                    true
                }
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

// ---- Export a HTML autocontenido -----------------------------------------

/// Un marco listo para exportar: su rect de mundo + giro + su HTML interno.
#[derive(Clone, Debug, PartialEq)]
pub struct MarcoExport {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rot: f64,
    /// HTML interno del marco (texto, `<img>`, lo que sea).
    pub html: String,
}

/// Genera un documento HTML **autocontenido** desde los marcos (función pura,
/// sin DOM — testeable en host). Embebe el CSS y un JS vanilla que replica la
/// cámara de `pluma-deck-core` (`fit`/`zoom_a_cursor`/`pan`/pasos/`vista_general`)
/// para que el `.html` resultante presente offline sin servidor ni `.wasm`.
pub fn recorrido_a_html(titulo: &str, marcos: &[MarcoExport]) -> String {
    let t = escapar_html(titulo);
    let mut cuerpo = String::new();
    for m in marcos {
        let rot = if m.rot != 0.0 {
            format!(";transform:rotate({}rad)", m.rot)
        } else {
            String::new()
        };
        cuerpo.push_str(&format!(
            "<div class=\"recorrido-marco\" data-x=\"{x}\" data-y=\"{y}\" data-w=\"{w}\" data-h=\"{h}\" data-rot=\"{r}\" \
             style=\"left:{x}px;top:{y}px;width:{w}px;height:{h}px{rot}\">{html}</div>",
            x = m.x, y = m.y, w = m.w, h = m.h, r = m.rot, rot = rot, html = m.html,
        ));
    }
    format!(
        "<!DOCTYPE html>\n<html lang=\"es\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{t}</title><style>{css}</style></head><body>\
         <div class=\"recorrido-viewport\"><div class=\"recorrido-mundo\">{cuerpo}</div>\
         <div class=\"recorrido-hud\"></div></div><script>{js}</script></body></html>",
        t = t, css = EXPORT_CSS, cuerpo = cuerpo, js = EXPORT_JS,
    )
}

/// Escape mínimo para insertar texto en HTML (sólo para el `<title>`).
fn escapar_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

const EXPORT_CSS: &str = "\
html,body{margin:0;height:100%;background:#12141c;font-family:system-ui,sans-serif}\
.recorrido-viewport{position:relative;width:100vw;height:100vh;overflow:hidden;touch-action:none;user-select:none}\
.recorrido-mundo{position:absolute;left:0;top:0;transform-origin:0 0;will-change:transform}\
.recorrido-marco{position:absolute;box-sizing:border-box;background:#262a38;border:1px solid #505668;\
border-radius:6px;color:#e1e6f0;padding:20px;overflow:hidden}\
.recorrido-marco h1,.recorrido-marco h2{margin:0 0 .4em;color:#fff}\
.recorrido-marco img{max-width:100%;max-height:100%;object-fit:contain}\
.recorrido-hud{position:fixed;left:50%;bottom:16px;transform:translateX(-50%);\
background:rgba(12,14,20,.78);color:#cdd4e2;padding:6px 12px;border-radius:999px;font-size:14px;pointer-events:none}";

// JS vanilla — espejo de RecorridoWeb/Camara. Se inserta como argumento de
// format!, así sus llaves no necesitan escaparse.
const EXPORT_JS: &str = r#"(function(){
var vp=document.querySelector('.recorrido-viewport');
var mundo=document.querySelector('.recorrido-mundo');
var hud=document.querySelector('.recorrido-hud');
var ZB=1.1,FIT=0.9,ZMIN=0.02,ZMAX=64,DUR=800,WN=100,DWELL=2500;
var autoTimer=null;
var EASE='transform '+DUR+'ms cubic-bezier(0.22,0.61,0.36,1)';
var marcos=[].slice.call(mundo.children).map(function(el){return{
 x:+el.dataset.x||0,y:+el.dataset.y||0,w:+el.dataset.w||640,h:+el.dataset.h||400,rot:+el.dataset.rot||0};});
var cam={cx:0,cy:0,zoom:1,rot:0},paso=0;
function P(){return{w:vp.clientWidth,h:vp.clientHeight};}
function clamp(z){return Math.max(ZMIN,Math.min(ZMAX,z));}
function fit(m){var p=P();var zw=m.w>0?p.w/m.w:1,zh=m.h>0?p.h/m.h:1;
 return{cx:m.x+m.w/2,cy:m.y+m.h/2,zoom:clamp(Math.min(zw,zh)*FIT),rot:m.rot};}
function fitAll(){if(!marcos.length)return cam;var p=P();
 var minx=1/0,miny=1/0,maxx=-1/0,maxy=-1/0;
 marcos.forEach(function(m){var hw=m.w/2,hh=m.h/2,c=Math.abs(Math.cos(m.rot)),s=Math.abs(Math.sin(m.rot));
  var ex=hw*c+hh*s,ey=hw*s+hh*c,cx=m.x+hw,cy=m.y+hh;
  minx=Math.min(minx,cx-ex);miny=Math.min(miny,cy-ey);maxx=Math.max(maxx,cx+ex);maxy=Math.max(maxy,cy+ey);});
 var bw=maxx-minx,bh=maxy-miny;var zw=bw>0?p.w/bw:1,zh=bh>0?p.h/bh:1;
 return{cx:(minx+maxx)/2,cy:(miny+maxy)/2,zoom:clamp(Math.min(zw,zh)*FIT),rot:0};}
function apply(smooth){var p=P();mundo.style.transition=smooth?EASE:'none';
 mundo.style.transform='translate('+(p.w/2)+'px,'+(p.h/2)+'px) scale('+cam.zoom+') rotate('+(-cam.rot)+'rad) translate('+(-cam.cx)+'px,'+(-cam.cy)+'px)';
 if(hud)hud.textContent=(paso+1)+' / '+marcos.length;}
function s2w(px,py){var p=P();var sx=(px-p.w/2)/cam.zoom,sy=(py-p.h/2)/cam.zoom;
 var c=Math.cos(cam.rot),s=Math.sin(cam.rot);return[cam.cx+sx*c-sy*s,cam.cy+sx*s+sy*c];}
function goto(i,smooth){if(i<0||i>=marcos.length)return;paso=i;cam=fit(marcos[i]);apply(smooth);}
function setAuto(on){if(autoTimer){clearInterval(autoTimer);autoTimer=null;}
 if(on)autoTimer=setInterval(function(){goto(paso+1<marcos.length?paso+1:0,true);},DUR+DWELL);}
marcos.forEach(function(m,i){var el=mundo.children[i];el.style.position='absolute';
 el.style.left=m.x+'px';el.style.top=m.y+'px';el.style.width=m.w+'px';el.style.height=m.h+'px';
 el.style.boxSizing='border-box';if(m.rot)el.style.transform='rotate('+m.rot+'rad)';});
vp.addEventListener('wheel',function(e){e.preventDefault();var r=vp.getBoundingClientRect();
 var cx=e.clientX-r.left,cy=e.clientY-r.top;var pasos=Math.max(-3,Math.min(3,e.deltaY/WN));
 var mult=Math.pow(ZB,-pasos);var anc=s2w(cx,cy);cam.zoom=clamp(cam.zoom*mult);
 var p=P();var sx=(cx-p.w/2)/cam.zoom,sy=(cy-p.h/2)/cam.zoom;var c=Math.cos(cam.rot),s=Math.sin(cam.rot);
 cam.cx=anc[0]-(sx*c-sy*s);cam.cy=anc[1]-(sx*s+sy*c);apply(false);},{passive:false});
var drag=null;
vp.addEventListener('pointerdown',function(e){drag=[e.clientX,e.clientY];vp.setPointerCapture(e.pointerId);});
vp.addEventListener('pointermove',function(e){if(!drag)return;var dx=e.clientX-drag[0],dy=e.clientY-drag[1];
 drag=[e.clientX,e.clientY];var c=Math.cos(cam.rot),s=Math.sin(cam.rot);
 cam.cx-=(dx*c-dy*s)/cam.zoom;cam.cy-=(dx*s+dy*c)/cam.zoom;apply(false);});
['pointerup','pointercancel','pointerleave'].forEach(function(ev){vp.addEventListener(ev,function(){drag=null;});});
window.addEventListener('keydown',function(e){var k=e.key;
 if(k==='ArrowRight'||k==='ArrowDown'||k===' '||k==='Spacebar'||k==='Enter'){if(paso+1<marcos.length){goto(paso+1,true);e.preventDefault();}}
 else if(k==='ArrowLeft'||k==='ArrowUp'){if(paso>0){goto(paso-1,true);e.preventDefault();}}
 else if(k==='Home'||k==='Escape'){cam=fitAll();apply(true);e.preventDefault();}
 else if(k==='p'||k==='P'){setAuto(!autoTimer);e.preventDefault();}});
window.addEventListener('resize',function(){apply(false);});
goto(0,false);
})();"#;

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

    #[test]
    fn export_html_es_documento_autocontenido() {
        let marcos = vec![
            MarcoExport { x: 0.0, y: 0.0, w: 640.0, h: 400.0, rot: 0.0, html: "<h1>Uno</h1>".into() },
            MarcoExport { x: 900.0, y: 0.0, w: 640.0, h: 400.0, rot: 0.12, html: "<p>Dos</p>".into() },
        ];
        let html = recorrido_a_html("Mi <demo>", &marcos);
        // Documento completo con CSS + JS embebidos (sin recursos externos).
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<style>") && html.contains("<script>"));
        assert!(!html.contains("http://") && !html.contains("https://") && !html.contains(".wasm"));
        // El título se escapa.
        assert!(html.contains("<title>Mi &lt;demo&gt;</title>"), "{}", &html[..200]);
        // Cada marco con su geometría de mundo y su HTML interno.
        assert!(html.contains("data-x=\"900\"") && html.contains("data-rot=\"0.12\""));
        assert!(html.contains("<h1>Uno</h1>") && html.contains("<p>Dos</p>"));
        // El JS replica la cámara del core (funciones clave presentes).
        assert!(html.contains("function fit(") && html.contains("function fitAll("));
        assert!(html.contains("function s2w(") && html.contains("goto(0,false)"));
        // Y el modo presentador (autoplay con setInterval, tecla 'p').
        assert!(html.contains("function setAuto(") && html.contains("setInterval("));
    }

    #[test]
    fn export_html_sin_marcos_sigue_siendo_valido() {
        let html = recorrido_a_html("vacío", &[]);
        assert!(html.starts_with("<!DOCTYPE html>") && html.ends_with("</html>"));
        assert!(html.contains("recorrido-mundo"));
    }
}
