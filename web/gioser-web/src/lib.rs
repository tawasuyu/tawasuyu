//! Entrypoint WASM de la landing GioSer.
//!
//! Responsabilidades:
//! - Montar canvas WebGL2 + listeners de mouse/resize/RAF loop.
//! - Reposicionar los 4 tips DOM (botones cardinales) sobre las posiciones
//!   proyectadas del aro de la chacana cada frame.
//! - Inclinar el título "GioSer" central inyectando CSS vars de tilt.
//! - Manejar click sobre cada tip → animar drawer expandiéndose desde la
//!   posición del botón hasta fullscreen, cargar el .md asociado vía
//!   `pluma-reader-web` y renderearlo themed por elemento.
//! - Cerrar drawer con close button, Escape o backdrop click.

use std::cell::RefCell;
use std::rc::Rc;

use gioser_canvas_web::{tips, Renderer};
use pluma_reader_web::Reader;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    Document, Event, HtmlCanvasElement, HtmlElement, KeyboardEvent, MouseEvent, Window,
};

/// Factor radial sobre `arm_extent` donde se anclan los botones DOM.
/// Queda entre la punta de la chacana y el aro grueso.
const BUTTON_RADIUS_FACTOR: f32 = 1.32;

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
    renderer
        .borrow_mut()
        .resize(canvas.width(), canvas.height());

    install_resize(&window, &canvas, &renderer)?;
    install_mouse(&document, &canvas, &renderer)?;
    install_drawer_handlers(&document)?;
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
        r.borrow_mut().resize(canvas.width(), canvas.height());
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

fn install_drawer_handlers(document: &Document) -> Result<(), JsValue> {
    let doc = document.clone();

    // Clicks en los 4 tips → open drawer.
    let tips = document.query_selector_all(".tip[data-md]")?;
    for i in 0..tips.length() {
        let Some(node) = tips.item(i) else { continue };
        let Ok(el) = node.dyn_into::<HtmlElement>() else { continue };
        let id = el.id();
        let element = id.strip_prefix("tip-").unwrap_or("").to_string();
        let md_url = el.get_attribute("data-md").unwrap_or_default();
        let d = doc.clone();
        let el_for_rect = el.clone();
        let cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
            e.prevent_default();
            open_drawer(&d, &element, &el_for_rect, &md_url);
        });
        el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
        cb.forget();
    }

    // Cualquier elemento marcado con `data-close-drawer` cierra.
    let closes = document.query_selector_all("[data-close-drawer]")?;
    for i in 0..closes.length() {
        let Some(node) = closes.item(i) else { continue };
        let Ok(el) = node.dyn_into::<HtmlElement>() else { continue };
        let d = doc.clone();
        let cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
            e.stop_propagation();
            close_drawers(&d);
        });
        el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
        cb.forget();
    }

    // Escape cierra.
    let d = doc.clone();
    let kcb = Closure::<dyn FnMut(KeyboardEvent)>::new(move |e: KeyboardEvent| {
        if e.key() == "Escape" {
            close_drawers(&d);
        }
    });
    document.add_event_listener_with_callback("keydown", kcb.as_ref().unchecked_ref())?;
    kcb.forget();

    Ok(())
}

fn open_drawer(doc: &Document, element: &str, button: &HtmlElement, md_url: &str) {
    let rect = button.get_bounding_client_rect();
    let cx = rect.left() + rect.width() / 2.0;
    let cy = rect.top() + rect.height() / 2.0;
    let drawer_id = format!("drawer-{}", element);
    let Some(drawer_el) = doc.get_element_by_id(&drawer_id) else {
        return;
    };
    let drawer: HtmlElement = drawer_el.unchecked_into();
    let _ = drawer
        .style()
        .set_property("--origin-x", &format!("{:.1}px", cx));
    let _ = drawer
        .style()
        .set_property("--origin-y", &format!("{:.1}px", cy));
    let _ = drawer.class_list().add_1("open");
    drawer.set_attribute("aria-hidden", "false").ok();

    if let Some(body) = doc.body() {
        let _ = body.class_list().add_1("drawer-active");
        let _ = body
            .class_list()
            .add_1(&format!("drawer-active-{}", element));
    }

    // Carga del .md en background.
    let content_id = format!("drawer-{}-content", element);
    if let Some(content_el) = doc.get_element_by_id(&content_id) {
        let content: HtmlElement = content_el.unchecked_into();
        let reader = Reader::new(content);
        let element_owned = element.to_string();
        let md_url_owned = md_url.to_string();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = reader.open_url(&md_url_owned, &element_owned).await {
                web_sys::console::warn_1(&e);
            }
        });
    }
}

fn close_drawers(doc: &Document) {
    let Ok(drawers) = doc.query_selector_all(".drawer.open") else {
        return;
    };
    for i in 0..drawers.length() {
        let Some(node) = drawers.item(i) else { continue };
        let Ok(el) = node.dyn_into::<HtmlElement>() else {
            continue;
        };
        let _ = el.class_list().remove_1("open");
        let _ = el.set_attribute("aria-hidden", "true");
    }
    if let Some(body) = doc.body() {
        let _ = body.class_list().remove_1("drawer-active");
        for e in ["aire", "fuego", "tierra", "agua"] {
            let _ = body
                .class_list()
                .remove_1(&format!("drawer-active-{}", e));
        }
    }
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
        let r = renderer.borrow_mut();
        // r es Mut, ojo: el render necesita mut, lo hacemos antes de paint.
        drop(r);
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
    for (i, (id, _color, _label)) in tips::ORDER.iter().enumerate() {
        let (nx, ny) = clips[i];
        let px = (nx + 1.0) * 0.5 * cw;
        let py = (1.0 - (ny + 1.0) * 0.5) * ch;
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
    let (pitch, yaw) = renderer.tilt_degrees();
    if let Some(brand) = document.get_element_by_id("brand") {
        if let Ok(brand) = brand.dyn_into::<HtmlElement>() {
            // CSS rotateX usa el mismo signo que nuestra pitch (mouse up tilts top toward viewer).
            let _ = brand
                .style()
                .set_property("--tilt-x", &format!("{:.2}deg", pitch));
            let _ = brand
                .style()
                .set_property("--tilt-y", &format!("{:.2}deg", yaw));
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
