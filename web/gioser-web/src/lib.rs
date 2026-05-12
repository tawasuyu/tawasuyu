//! Entrypoint WASM de la landing GioSer.
//!
//! Monta el canvas, instala listeners de mouse y resize, corre el loop
//! con `requestAnimationFrame`, y reposiciona los 4 botones DOM
//! `#tip-{aire|fuego|tierra|agua}` sobre las puntas proyectadas
//! de la chacana cada frame.

use std::cell::RefCell;
use std::rc::Rc;

use gioser_canvas_web::{tips, Renderer};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Document, HtmlCanvasElement, HtmlElement, MouseEvent, Window};

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
        // Origen en el centro del canvas, +y arriba.
        let x = e.client_x() as f32 - w * 0.5;
        let y = h * 0.5 - e.client_y() as f32;
        r.borrow_mut().set_mouse_px(x, y);
    });
    document.add_event_listener_with_callback("mousemove", cb.as_ref().unchecked_ref())?;
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
        position_tips(&document, &canvas, &renderer.borrow());
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
    let clips = renderer.tips_ndc();
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

fn install_panic_hook() {
    static SET: std::sync::Once = std::sync::Once::new();
    SET.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let msg = format!("{}", info);
            web_sys::console::error_1(&JsValue::from_str(&msg));
        }));
    });
}
