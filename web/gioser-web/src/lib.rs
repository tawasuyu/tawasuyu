//! Entrypoint WASM de la landing GioSer.
//!
//! Responsabilidades:
//! - Montar canvas WebGL2 + listeners de mouse/pointer/resize/keyboard/RAF.
//! - Reposicionar los 4 tips DOM cada frame siguiendo el aro de la chacana,
//!   con clamp para que nunca salgan del viewport ni cubran la taskbar.
//! - Inclinar el título "GioSer" central inyectando CSS vars de tilt+roll.
//! - Manejar **click/tap dentro del aro** → vibración (impulso al shake spring).
//! - Manejar **mouseleave del canvas** → tilt vuelve al frente con rebote.
//! - Drawers MD por elemento que crecen desde la posición del botón
//!   clickeado hasta fullscreen (excepto la taskbar).
//! - Taskbar estilo Windows: home a la izquierda + cajitas dinámicas por
//!   cada vista MD abierta, click cambia el activo.

use std::cell::RefCell;
use std::rc::Rc;

use gioser_canvas_web::{tips, Renderer};
use pluma_reader_web::Reader;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    Document, Element, Event, HtmlCanvasElement, HtmlElement, KeyboardEvent, MouseEvent,
    PointerEvent, Window,
};

/// Botones se anclan entre la punta de la chacana y el aro grueso.
const BUTTON_RADIUS_FACTOR: f32 = 1.32;
/// Altura reservada para la taskbar abajo. Sincronizar con CSS `.taskbar { height }`.
const TASKBAR_HEIGHT_PX: f32 = 52.0;
/// Padding de seguridad alrededor de los botones para que nunca toquen los bordes.
const BUTTON_HALF_W_PX: f32 = 90.0;
const BUTTON_HALF_H_PX: f32 = 64.0;
const VIEWPORT_MARGIN_PX: f32 = 14.0;
const ELEMENTS: [&str; 4] = ["aire", "fuego", "tierra", "agua"];

#[derive(Default)]
struct TaskbarState {
    /// Elementos abiertos (en orden de apertura), aparecen como cajitas.
    open: Vec<String>,
    /// Cuál está visible. `None` = home (sin drawer activo).
    active: Option<String>,
}

struct AppState {
    document: Document,
    state: RefCell<TaskbarState>,
}

impl AppState {
    fn open_tab(&self, element: &str, origin_x: f64, origin_y: f64, md_url: &str) {
        self.set_drawer_origin(element, origin_x, origin_y);
        {
            let mut s = self.state.borrow_mut();
            if !s.open.iter().any(|e| e == element) {
                s.open.push(element.to_string());
            }
            s.active = Some(element.to_string());
        }
        self.sync();
        self.load_md_if_empty(element, md_url);
    }

    fn switch_tab(&self, element: &str, origin_x: f64, origin_y: f64) {
        self.set_drawer_origin(element, origin_x, origin_y);
        let mut s = self.state.borrow_mut();
        if s.open.iter().any(|e| e == element) {
            s.active = Some(element.to_string());
        }
        drop(s);
        self.sync();
    }

    fn close_tab(&self, element: &str) {
        let mut s = self.state.borrow_mut();
        s.open.retain(|e| e != element);
        if s.active.as_deref() == Some(element) {
            s.active = s.open.last().cloned();
        }
        drop(s);
        self.sync();
    }

    fn home(&self) {
        let mut s = self.state.borrow_mut();
        s.open.clear();
        s.active = None;
        drop(s);
        self.sync();
    }

    fn active(&self) -> Option<String> {
        self.state.borrow().active.clone()
    }

    fn set_drawer_origin(&self, element: &str, x: f64, y: f64) {
        let id = format!("drawer-{}", element);
        if let Some(el) = self.document.get_element_by_id(&id) {
            if let Ok(el) = el.dyn_into::<HtmlElement>() {
                let _ = el.style().set_property("--origin-x", &format!("{:.1}px", x));
                let _ = el.style().set_property("--origin-y", &format!("{:.1}px", y));
            }
        }
    }

    fn sync(&self) {
        let s = self.state.borrow();
        let body = match self.document.body() {
            Some(b) => b,
            None => return,
        };
        if s.active.is_some() {
            let _ = body.class_list().add_1("drawer-active");
        } else {
            let _ = body.class_list().remove_1("drawer-active");
        }
        for &e in &ELEMENTS {
            let cls = format!("drawer-active-{}", e);
            if s.active.as_deref() == Some(e) {
                let _ = body.class_list().add_1(&cls);
            } else {
                let _ = body.class_list().remove_1(&cls);
            }
        }
        for &e in &ELEMENTS {
            let id = format!("drawer-{}", e);
            if let Some(el) = self.document.get_element_by_id(&id) {
                if let Ok(el) = el.dyn_into::<HtmlElement>() {
                    if s.active.as_deref() == Some(e) {
                        let _ = el.class_list().add_1("open");
                        el.set_attribute("aria-hidden", "false").ok();
                    } else {
                        let _ = el.class_list().remove_1("open");
                        el.set_attribute("aria-hidden", "true").ok();
                    }
                }
            }
        }
        if let Some(list) = self.document.get_element_by_id("taskbar-list") {
            let mut html = String::new();
            for e in &s.open {
                let label = e.to_uppercase();
                let active = if s.active.as_deref() == Some(e.as_str()) {
                    " active"
                } else {
                    ""
                };
                html.push_str(&format!(
                    "<li><button class=\"taskbar-item{active}\" data-task=\"{e}\" type=\"button\">\
                     <span class=\"taskbar-item-dot\" aria-hidden=\"true\"></span>{label}</button></li>"
                ));
            }
            list.set_inner_html(&html);
        }
    }

    fn load_md_if_empty(&self, element: &str, md_url: &str) {
        let content_id = format!("drawer-{}-content", element);
        let Some(content_el) = self.document.get_element_by_id(&content_id) else {
            return;
        };
        let Ok(content): Result<HtmlElement, _> = content_el.dyn_into() else {
            return;
        };
        let inner = content.inner_html();
        // Si ya tiene contenido renderizado (pluma-doc) y no es loader/error, no re-fetch.
        if inner.contains("pluma-doc") {
            return;
        }
        let reader = Reader::new(content);
        let element_owned = element.to_string();
        let url_owned = md_url.to_string();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = reader.open_url(&url_owned, &element_owned).await {
                web_sys::console::warn_1(&e);
            }
        });
    }
}

#[wasm_bindgen(start)]
pub fn boot() -> Result<(), JsValue> {
    install_panic_hook();
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let canvas: HtmlCanvasElement = document
        .get_element_by_id("gioser-canvas")
        .ok_or_else(|| JsValue::from_str("no canvas#gioser-canvas"))?
        .dyn_into()?;

    fit_canvas(&canvas, &window);
    let renderer = Rc::new(RefCell::new(Renderer::new(&canvas)?));
    {
        let mut r = renderer.borrow_mut();
        r.resize(canvas.width(), canvas.height());
        r.set_client_size(
            canvas.client_width() as f32,
            canvas.client_height() as f32,
        );
    }

    let app = Rc::new(AppState {
        document: document.clone(),
        state: RefCell::default(),
    });

    install_resize(&window, &canvas, &renderer)?;
    install_mouse(&document, &canvas, &renderer)?;
    install_canvas_pointer(&canvas, &renderer)?;
    install_canvas_leave(&canvas, &renderer)?;
    install_tip_clicks(&document, &app)?;
    install_drawer_close_buttons(&document, &app)?;
    install_taskbar(&document, &app)?;
    install_keyboard(&document, &app)?;
    install_raf(&window, &document, &canvas, &renderer);

    Ok(())
}

fn install_resize(
    window: &Window,
    canvas: &HtmlCanvasElement,
    renderer: &Rc<RefCell<Renderer>>,
) -> Result<(), JsValue> {
    let canvas = canvas.clone();
    let win2 = window.clone();
    let r = renderer.clone();
    let cb = Closure::<dyn FnMut()>::new(move || {
        fit_canvas(&canvas, &win2);
        let mut rr = r.borrow_mut();
        rr.resize(canvas.width(), canvas.height());
        rr.set_client_size(
            canvas.client_width() as f32,
            canvas.client_height() as f32,
        );
    });
    window.add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref())?;
    cb.forget();
    Ok(())
}

fn install_mouse(
    document: &Document,
    canvas: &HtmlCanvasElement,
    renderer: &Rc<RefCell<Renderer>>,
) -> Result<(), JsValue> {
    let canvas = canvas.clone();
    let r = renderer.clone();
    let cb = Closure::<dyn FnMut(MouseEvent)>::new(move |e: MouseEvent| {
        let w = canvas.client_width().max(1) as f32;
        let h = canvas.client_height().max(1) as f32;
        let x = e.client_x() as f32 - w * 0.5;
        let y = h * 0.5 - e.client_y() as f32;
        r.borrow_mut().set_mouse_px(x, y);
    });
    document.add_event_listener_with_callback("mousemove", cb.as_ref().unchecked_ref())?;
    cb.forget();
    Ok(())
}

/// Pointer down dentro del aro → impulso de vibración (click/tap shake).
fn install_canvas_pointer(
    canvas: &HtmlCanvasElement,
    renderer: &Rc<RefCell<Renderer>>,
) -> Result<(), JsValue> {
    let canvas2 = canvas.clone();
    let r = renderer.clone();
    let cb = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        let rect = canvas2.get_bounding_client_rect();
        let dx = e.client_x() as f64 - (rect.left() + rect.width() / 2.0);
        let dy = e.client_y() as f64 - (rect.top() + rect.height() / 2.0);
        let dist2 = dx * dx + dy * dy;
        let ring = {
            let rb = r.borrow();
            rb.click_radius_css_px() as f64
        };
        if dist2 <= ring * ring {
            r.borrow_mut().impulse_click();
        }
    });
    canvas.add_event_listener_with_callback("pointerdown", cb.as_ref().unchecked_ref())?;
    cb.forget();
    Ok(())
}

/// Mouse sale del canvas → tilt vuelve al frente con rebote del spring.
fn install_canvas_leave(
    canvas: &HtmlCanvasElement,
    renderer: &Rc<RefCell<Renderer>>,
) -> Result<(), JsValue> {
    let r = renderer.clone();
    let cb = Closure::<dyn FnMut(Event)>::new(move |_e: Event| {
        r.borrow_mut().release_tilt();
    });
    canvas.add_event_listener_with_callback("mouseleave", cb.as_ref().unchecked_ref())?;
    cb.forget();
    Ok(())
}

fn install_tip_clicks(document: &Document, app: &Rc<AppState>) -> Result<(), JsValue> {
    let tips_nodes = document.query_selector_all(".tip[data-md]")?;
    for i in 0..tips_nodes.length() {
        let Some(node) = tips_nodes.item(i) else {
            continue;
        };
        let Ok(el) = node.dyn_into::<HtmlElement>() else {
            continue;
        };
        let id = el.id();
        let element = id.strip_prefix("tip-").unwrap_or("").to_string();
        let md_url = el.get_attribute("data-md").unwrap_or_default();
        let app2 = app.clone();
        let el_for_rect = el.clone();
        let cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
            e.prevent_default();
            let rect = el_for_rect.get_bounding_client_rect();
            let cx = rect.left() + rect.width() / 2.0;
            let cy = rect.top() + rect.height() / 2.0;
            app2.open_tab(&element, cx, cy, &md_url);
        });
        el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
        cb.forget();
    }
    Ok(())
}

fn install_drawer_close_buttons(
    document: &Document,
    app: &Rc<AppState>,
) -> Result<(), JsValue> {
    let closes = document.query_selector_all("[data-close-drawer]")?;
    for i in 0..closes.length() {
        let Some(node) = closes.item(i) else { continue };
        let Ok(el) = node.dyn_into::<HtmlElement>() else {
            continue;
        };
        let el_ref: &Element = el.as_ref();
        let element_attr = el_ref
            .closest(".drawer")
            .ok()
            .flatten()
            .and_then(|d| d.get_attribute("data-element"))
            .unwrap_or_default();
        let app2 = app.clone();
        let cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
            e.stop_propagation();
            app2.close_tab(&element_attr);
        });
        el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
        cb.forget();
    }
    Ok(())
}

fn install_taskbar(document: &Document, app: &Rc<AppState>) -> Result<(), JsValue> {
    // Botón home
    let homes = document.query_selector_all("[data-home]")?;
    for i in 0..homes.length() {
        let Some(node) = homes.item(i) else { continue };
        let Ok(el) = node.dyn_into::<HtmlElement>() else {
            continue;
        };
        let app2 = app.clone();
        let cb = Closure::<dyn FnMut(Event)>::new(move |_e: Event| {
            app2.home();
        });
        el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
        cb.forget();
    }
    // Delegación: 1 listener en la lista, dispatch por data-task del closest .taskbar-item.
    if let Some(list) = document.get_element_by_id("taskbar-list") {
        let app2 = app.clone();
        let cb = Closure::<dyn FnMut(MouseEvent)>::new(move |e: MouseEvent| {
            let Some(target) = e.target() else { return };
            let Ok(target_el): Result<Element, _> = target.dyn_into() else {
                return;
            };
            let Ok(Some(item)) = target_el.closest(".taskbar-item") else {
                return;
            };
            if let Some(task) = item.get_attribute("data-task") {
                let rect = item.get_bounding_client_rect();
                let cx = rect.left() + rect.width() / 2.0;
                let cy = rect.top() + rect.height() / 2.0;
                app2.switch_tab(&task, cx, cy);
            }
        });
        list.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
        cb.forget();
    }
    Ok(())
}

fn install_keyboard(document: &Document, app: &Rc<AppState>) -> Result<(), JsValue> {
    let app2 = app.clone();
    let cb = Closure::<dyn FnMut(KeyboardEvent)>::new(move |e: KeyboardEvent| {
        if e.key() == "Escape" {
            if let Some(active) = app2.active() {
                app2.close_tab(&active);
            }
        }
    });
    document.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())?;
    cb.forget();
    Ok(())
}

fn install_raf(
    window: &Window,
    document: &Document,
    canvas: &HtmlCanvasElement,
    renderer: &Rc<RefCell<Renderer>>,
) {
    let f = Rc::new(RefCell::new(None::<Closure<dyn FnMut(f64)>>));
    let g = f.clone();
    let renderer = renderer.clone();
    let canvas = canvas.clone();
    let document = document.clone();
    let window2 = window.clone();
    *g.borrow_mut() = Some(Closure::<dyn FnMut(f64)>::new(move |time_ms: f64| {
        renderer.borrow_mut().render(time_ms);
        let r = renderer.borrow();
        position_tips(&document, &canvas, &r);
        update_tilt_css(&document, &r);
        drop(r);
        if let Some(cb) = f.borrow().as_ref() {
            let _ = window2.request_animation_frame(cb.as_ref().unchecked_ref());
        }
    }));
    let _ = window.request_animation_frame(g.borrow().as_ref().unwrap().as_ref().unchecked_ref());
}

fn fit_canvas(canvas: &HtmlCanvasElement, window: &Window) {
    let dpr = window.device_pixel_ratio() as f32;
    let w = window
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(1280.0) as f32;
    let h = window
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(720.0) as f32;
    canvas.set_width((w * dpr) as u32);
    canvas.set_height((h * dpr) as u32);
    let el: &HtmlElement = canvas.unchecked_ref();
    let style = el.style();
    let _ = style.set_property("width", &format!("{}px", w));
    let _ = style.set_property("height", &format!("{}px", h));
}

fn position_tips(document: &Document, canvas: &HtmlCanvasElement, renderer: &Renderer) {
    let clips = renderer.cardinal_positions_ndc(BUTTON_RADIUS_FACTOR);
    let cw = canvas.client_width().max(1) as f32;
    let ch = canvas.client_height().max(1) as f32;
    // Bounds en CSS pixels donde los botones pueden moverse libremente.
    let min_x = VIEWPORT_MARGIN_PX + BUTTON_HALF_W_PX;
    let max_x = (cw - VIEWPORT_MARGIN_PX - BUTTON_HALF_W_PX).max(min_x);
    let min_y = VIEWPORT_MARGIN_PX + BUTTON_HALF_H_PX;
    let max_y = (ch - TASKBAR_HEIGHT_PX - VIEWPORT_MARGIN_PX - BUTTON_HALF_H_PX).max(min_y);
    for (i, (id, _color, _label)) in tips::ORDER.iter().enumerate() {
        let (nx, ny) = clips[i];
        let raw_x = (nx + 1.0) * 0.5 * cw;
        let raw_y = (1.0 - (ny + 1.0) * 0.5) * ch;
        let px = raw_x.clamp(min_x, max_x);
        let py = raw_y.clamp(min_y, max_y);
        let sel = format!("tip-{}", id);
        if let Some(el) = document.get_element_by_id(&sel) {
            if let Ok(el) = el.dyn_into::<HtmlElement>() {
                let _ = el.style().set_property(
                    "transform",
                    &format!("translate({:.2}px, {:.2}px) translate(-50%, -50%)", px, py),
                );
            }
        }
    }
}

fn update_tilt_css(document: &Document, renderer: &Renderer) {
    let (pitch, yaw, roll) = renderer.tilt_degrees();
    if let Some(brand) = document.get_element_by_id("brand") {
        if let Ok(brand) = brand.dyn_into::<HtmlElement>() {
            let _ = brand
                .style()
                .set_property("--tilt-x", &format!("{:.2}deg", pitch));
            let _ = brand
                .style()
                .set_property("--tilt-y", &format!("{:.2}deg", yaw));
            let _ = brand
                .style()
                .set_property("--tilt-z", &format!("{:.2}deg", roll));
        }
    }
}

fn install_panic_hook() {
    static SET: std::sync::Once = std::sync::Once::new();
    SET.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let msg = format!("{}", info);
            web_sys::console::error_1(&JsValue::from_str(&msg));
        }));
    });
}
