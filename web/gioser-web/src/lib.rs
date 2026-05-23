//! Entrypoint WASM de la landing GioSer.
//!
//! Capas:
//! - **Canvas WebGL**: chacana animada (gioser-canvas-web).
//! - **Tips**: 4 botones DOM en los cardinales, posicionados cada frame.
//! - **Deck** (vista-web): contenedor único con páginas swipeable estilo
//!   Flutter `PageView`. Cada elemento del logo es una página dinámica.
//! - **Taskbar**: barra abajo con home + brand "GioSer" + tabs activas +
//!   copyleft/email a la derecha. Sincronizada por WASM.
//!
//! Acciones (todas pasan por `AppState`):
//! - `open_or_switch`   — click en tip o abrir nueva pestaña.
//! - `restore_from_tab` — click en cajita de la taskbar.
//! - `minimize`         — botón ─ de la página o Escape.
//! - `close`            — botón × de la página, remueve del taskbar.
//! - `home`             — botón casa o brand, minimiza todo (mantiene tabs).
//! - `on_swipe`         — callback de vista-web cuando el snap cambia.

use std::cell::RefCell;
use std::rc::Rc;

use barra_web::{Task, TaskList};
use gioser_canvas_web::{tips, Renderer};
use fana_md_reader_web::Reader;
use revista_web::Deck;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    Document, Element, Event, HtmlCanvasElement, HtmlElement, KeyboardEvent, MouseEvent,
    PointerEvent, Window,
};

const BUTTON_RADIUS_FACTOR: f32 = 1.32;
const TASKBAR_HEIGHT_PX: f32 = 52.0;
const BUTTON_HALF_W_PX: f32 = 90.0;
const BUTTON_HALF_H_PX: f32 = 64.0;
const VIEWPORT_MARGIN_PX: f32 = 14.0;
const ELEMENTS: [&str; 4] = ["aire", "fuego", "tierra", "agua"];

#[derive(Default)]
struct DeckState {
    /// Pestañas abiertas en orden de apertura. Coincide con páginas en el strip.
    pages: Vec<String>,
    /// Cuál es la página visible. `None` = deck minimizado (pestañas siguen
    /// en la taskbar) o sin pestañas.
    active: Option<String>,
}

struct AppState {
    document: Document,
    deck: Deck,
    taskbar: TaskList,
    state: RefCell<DeckState>,
}

impl AppState {
    fn open_or_switch(&self, element: &str, origin_x: f64, origin_y: f64, md_url: &str) {
        let was_visible = self.state.borrow().active.is_some();
        let was_in_pages = self.state.borrow().pages.iter().any(|e| e == element);
        if !was_in_pages {
            self.ensure_page_dom(element);
            self.state.borrow_mut().pages.push(element.to_string());
        }
        self.state.borrow_mut().active = Some(element.to_string());
        let idx = self
            .state
            .borrow()
            .pages
            .iter()
            .position(|e| e == element)
            .unwrap_or(0);
        if was_visible {
            self.deck.goto(idx, true);
        } else {
            self.deck.goto(idx, false);
            self.show_deck(origin_x, origin_y);
        }
        self.sync_active_class();
        self.sync_taskbar();
        self.load_md_if_empty(element, md_url);
    }

    fn restore_from_tab(&self, element: &str, origin_x: f64, origin_y: f64) {
        let idx_opt = self
            .state
            .borrow()
            .pages
            .iter()
            .position(|e| e == element);
        let Some(idx) = idx_opt else { return };
        let was_visible = self.state.borrow().active.is_some();
        self.state.borrow_mut().active = Some(element.to_string());
        if was_visible {
            self.deck.goto(idx, true);
        } else {
            self.deck.goto(idx, false);
            self.show_deck(origin_x, origin_y);
        }
        self.sync_active_class();
        self.sync_taskbar();
    }

    fn minimize(&self, origin_x: f64, origin_y: f64) {
        self.state.borrow_mut().active = None;
        self.sync_active_class();
        self.sync_taskbar();
        self.hide_deck(origin_x, origin_y);
    }

    fn close(&self, element: &str, origin_x: f64, origin_y: f64) {
        let was_active = self.state.borrow().active.as_deref() == Some(element);
        self.state.borrow_mut().pages.retain(|e| e != element);
        self.remove_page_dom(element);
        if was_active {
            let pages_now: Vec<String> = self.state.borrow().pages.clone();
            let new_active = pages_now.last().cloned();
            self.state.borrow_mut().active = new_active.clone();
            if let Some(new_active) = new_active {
                let idx = pages_now
                    .iter()
                    .position(|e| e == &new_active)
                    .unwrap_or(0);
                self.deck.goto(idx, true);
            } else {
                self.hide_deck(origin_x, origin_y);
            }
        }
        self.sync_active_class();
        self.sync_taskbar();
    }

    fn home(&self) {
        // Minimiza el deck sin cerrar las pestañas (estilo "show desktop").
        let (ox, oy) = self
            .element_center(".taskbar-home")
            .unwrap_or((24.0, self.viewport_height() - 26.0));
        self.minimize(ox, oy);
    }

    fn on_swipe(&self, new_index: usize) {
        let element = self.state.borrow().pages.get(new_index).cloned();
        if let Some(element) = element {
            self.state.borrow_mut().active = Some(element);
            self.sync_active_class();
            self.sync_taskbar();
        }
    }

    fn show_deck(&self, x: f64, y: f64) {
        self.set_deck_origin(x, y);
        if let Some(deck) = self.deck_el() {
            let _ = deck.class_list().add_1("open");
            let _ = deck.set_attribute("aria-hidden", "false");
        }
        if let Some(body) = self.document.body() {
            let _ = body.class_list().add_1("deck-visible");
        }
    }

    fn hide_deck(&self, x: f64, y: f64) {
        self.set_deck_origin(x, y);
        if let Some(deck) = self.deck_el() {
            let _ = deck.class_list().remove_1("open");
            let _ = deck.set_attribute("aria-hidden", "true");
        }
        if let Some(body) = self.document.body() {
            let _ = body.class_list().remove_1("deck-visible");
        }
    }

    fn deck_el(&self) -> Option<HtmlElement> {
        self.document
            .get_element_by_id("deck")
            .and_then(|e| e.dyn_into::<HtmlElement>().ok())
    }

    fn set_deck_origin(&self, x: f64, y: f64) {
        if let Some(deck) = self.deck_el() {
            let _ = deck.style().set_property("--origin-x", &format!("{:.1}px", x));
            let _ = deck.style().set_property("--origin-y", &format!("{:.1}px", y));
        }
    }

    fn sync_active_class(&self) {
        if let Some(body) = self.document.body() {
            let active = self.state.borrow().active.clone();
            for &e in &ELEMENTS {
                let cls = format!("deck-active-{}", e);
                if active.as_deref() == Some(e) {
                    let _ = body.class_list().add_1(&cls);
                } else {
                    let _ = body.class_list().remove_1(&cls);
                }
            }
        }
    }

    fn sync_taskbar(&self) {
        let s = self.state.borrow();
        let tasks: Vec<Task> = s
            .pages
            .iter()
            .map(|e| {
                let mut t = Task::new(e.clone(), e.to_uppercase());
                if s.active.as_deref() == Some(e.as_str()) {
                    t = t.active();
                }
                t
            })
            .collect();
        self.taskbar.set_tasks(&tasks);
    }

    fn ensure_page_dom(&self, element: &str) {
        let sel = format!(".deck-page[data-element=\"{}\"]", element);
        if self
            .document
            .query_selector(&sel)
            .ok()
            .flatten()
            .is_some()
        {
            return;
        }
        let Some(strip) = self.document.get_element_by_id("deck-strip") else {
            return;
        };
        let (title, tag) = match element {
            "aire" => ("Software", "Tecnología · Open Source · IA"),
            "fuego" => ("Quién Soy", "Bitácora · Crónica"),
            "tierra" => ("Manifiesto", "Invariantes · Piedra de toque"),
            "agua" => ("Mística", "Espiritualidad aplicada"),
            _ => return,
        };
        let html = format!(
            "<article class=\"deck-page\" data-element=\"{el}\" id=\"deck-page-{el}\">\
                <div class=\"page-controls\">\
                    <button class=\"page-control-btn page-minimize\" data-minimize=\"{el}\" type=\"button\" aria-label=\"Minimizar {title}\">\
                        <svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M5 19 H19\" stroke=\"currentColor\" stroke-width=\"2\" fill=\"none\" stroke-linecap=\"round\"/></svg>\
                    </button>\
                    <button class=\"page-control-btn page-close\" data-close-page=\"{el}\" type=\"button\" aria-label=\"Cerrar {title}\">×</button>\
                </div>\
                <div class=\"page-ambience\" aria-hidden=\"true\"></div>\
                <header class=\"page-head\">\
                    <span class=\"page-mark\">{el}</span>\
                    <h2 class=\"page-title\">{title}</h2>\
                    <span class=\"page-tag\">{tag}</span>\
                </header>\
                <section class=\"page-content\" id=\"page-{el}-content\"></section>\
            </article>",
            el = element, title = title, tag = tag
        );
        let _ = strip.insert_adjacent_html("beforeend", &html);
    }

    fn remove_page_dom(&self, element: &str) {
        let id = format!("deck-page-{}", element);
        if let Some(el) = self.document.get_element_by_id(&id) {
            el.remove();
        }
    }

    fn load_md_if_empty(&self, element: &str, md_url: &str) {
        let content_id = format!("page-{}-content", element);
        let Some(content_el) = self.document.get_element_by_id(&content_id) else {
            return;
        };
        let Ok(content): Result<HtmlElement, _> = content_el.dyn_into() else {
            return;
        };
        let inner = content.inner_html();
        if inner.contains("pluma-doc") {
            return; // ya hidratado
        }
        let document_clone = self.document.clone();
        let element_owned = element.to_string();
        let url_owned = md_url.to_string();
        let reader = fana_md_reader_web::Reader::new(content.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let content_clone = content.clone();
            if let Err(e) = reader.open_url(&url_owned, &element_owned).await {
                web_sys::console::warn_1(&e);
            }
            // Montar contenedor del grafo (Cytoscape.js) debajo del md
            let graph_container_id = format!("graph-{}-container", element_owned);
            if document_clone.get_element_by_id(&graph_container_id).is_some() {
                return;
            }
            let wrapper: HtmlElement = document_clone
                .create_element("gioser-graph")
                .ok()
                .and_then(|e| e.dyn_into::<HtmlElement>().ok())
                .unwrap_or_else(|| {
                    // fallback: div normal
                    let d: HtmlElement = document_clone
                        .create_element("div")
                        .ok()
                        .and_then(|e| e.dyn_into().ok())
                        .unwrap();
                    d
                });
            wrapper.set_id(&graph_container_id);
            wrapper.set_attribute("data-api-url", "https://api.gioser.net").ok();
            wrapper.style().set_property("margin-top", "1.5rem").ok();
            wrapper.style().set_property("padding-top", "1rem").ok();
            wrapper.style().set_property("border-top", "1px solid rgba(255,255,255,0.06)").ok();
            wrapper.style().set_property("min-height", "220px").ok();
            content_clone.append_child(&wrapper).ok();
            // El script cytoscape-graph.js usa MutationObserver
            // para detectar <gioser-graph> dinámicos. No hace falta
            // disparar nada acá.
        });
    }

    fn element_center(&self, selector: &str) -> Option<(f64, f64)> {
        let el = self.document.query_selector(selector).ok().flatten()?;
        let rect = el.get_bounding_client_rect();
        Some((
            rect.left() + rect.width() / 2.0,
            rect.top() + rect.height() / 2.0,
        ))
    }

    fn taskbar_item_center(&self, element: &str) -> Option<(f64, f64)> {
        self.taskbar.task_center(element)
    }

    fn viewport_height(&self) -> f64 {
        web_sys::window()
            .and_then(|w| w.inner_height().ok())
            .and_then(|v| v.as_f64())
            .unwrap_or(720.0)
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

    // Mount vista deck
    let strip_el: HtmlElement = document
        .get_element_by_id("deck-strip")
        .ok_or_else(|| JsValue::from_str("no #deck-strip"))?
        .dyn_into()?;
    let deck = Deck::mount(strip_el)?;

    // Mount barra-web taskbar (manages the dynamic task list).
    let list_el: HtmlElement = document
        .get_element_by_id("taskbar-list")
        .ok_or_else(|| JsValue::from_str("no #taskbar-list"))?
        .dyn_into()?;
    let taskbar = TaskList::mount(list_el)?;

    let app = Rc::new(AppState {
        document: document.clone(),
        deck: deck.clone(),
        taskbar: taskbar.clone(),
        state: RefCell::default(),
    });

    // vista on_change → on_swipe del app
    {
        let app2 = app.clone();
        deck.on_change(move |idx| {
            app2.on_swipe(idx);
        });
    }

    // barra on_click → restore / toggle minimize del app
    {
        let app2 = app.clone();
        taskbar.on_click(move |id, cx, cy| {
            let is_active = app2.state.borrow().active.as_deref() == Some(id);
            if is_active {
                app2.minimize(cx, cy);
            } else {
                app2.restore_from_tab(id, cx, cy);
            }
        });
    }

    install_resize(&window, &canvas, &renderer)?;
    install_mouse(&document, &canvas, &renderer)?;
    install_canvas_pointer(&canvas, &renderer)?;
    install_canvas_leave(&canvas, &renderer)?;
    install_tip_clicks(&document, &app)?;
    install_deck_delegation(&document, &app)?;
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
            app2.open_or_switch(&element, cx, cy, &md_url);
        });
        el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
        cb.forget();
    }
    Ok(())
}

/// Un listener en el deck delega clicks de minimize y close en cada página.
/// Las páginas se crean dinámicamente, así que no podemos adjuntar listeners
/// por botón en boot.
fn install_deck_delegation(document: &Document, app: &Rc<AppState>) -> Result<(), JsValue> {
    let Some(deck_el) = document.get_element_by_id("deck") else {
        return Ok(());
    };
    let app2 = app.clone();
    let cb = Closure::<dyn FnMut(MouseEvent)>::new(move |e: MouseEvent| {
        let Some(target) = e.target() else { return };
        let Ok(target_el): Result<Element, _> = target.dyn_into() else {
            return;
        };
        // Minimize
        if let Ok(Some(btn)) = target_el.closest("[data-minimize]") {
            e.stop_propagation();
            let element = btn.get_attribute("data-minimize").unwrap_or_default();
            // Origin = la cajita correspondiente en la taskbar (efecto
            // visual: la página se "encoge" hacia su entrada del taskbar).
            let origin = app2
                .taskbar_item_center(&element)
                .unwrap_or_else(|| center_of_element(&btn));
            app2.minimize(origin.0, origin.1);
            return;
        }
        // Close
        if let Ok(Some(btn)) = target_el.closest("[data-close-page]") {
            e.stop_propagation();
            let element = btn.get_attribute("data-close-page").unwrap_or_default();
            let origin = app2
                .taskbar_item_center(&element)
                .unwrap_or_else(|| center_of_element(&btn));
            app2.close(&element, origin.0, origin.1);
        }
    });
    deck_el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
    cb.forget();
    Ok(())
}

fn center_of_element(el: &Element) -> (f64, f64) {
    let rect = el.get_bounding_client_rect();
    (
        rect.left() + rect.width() / 2.0,
        rect.top() + rect.height() / 2.0,
    )
}

/// Home button + brand link (data-home) — toda la lógica de tabs vive en
/// barra-web::TaskList. Acá sólo se instalan los handlers para [data-home].
fn install_taskbar(document: &Document, app: &Rc<AppState>) -> Result<(), JsValue> {
    let homes = document.query_selector_all("[data-home]")?;
    for i in 0..homes.length() {
        let Some(node) = homes.item(i) else { continue };
        let Ok(el) = node.dyn_into::<HtmlElement>() else {
            continue;
        };
        let app2 = app.clone();
        let cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
            e.prevent_default();
            app2.home();
        });
        el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
        cb.forget();
    }
    Ok(())
}

fn install_keyboard(document: &Document, app: &Rc<AppState>) -> Result<(), JsValue> {
    let app2 = app.clone();
    let cb = Closure::<dyn FnMut(KeyboardEvent)>::new(move |e: KeyboardEvent| {
        if e.key() == "Escape" {
            if app2.state.borrow().active.is_some() {
                app2.home();
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

/// Mapea un doc_id de Qdrant al nombre del elemento (aire/fuego/tierra/agua)
/// y su ruta md. Los doc_ids se generan con uuid5 en el indexador, pero
/// podemos inferir por el nombre del camino o del elemento.
fn map_doc_id_to_element(doc_id: &str) -> (String, String) {
    // Inferir del doc_id: contiene el nombre del elemento
    let el = if doc_id.contains("aire") || doc_id.contains("logos") {
        "aire"
    } else if doc_id.contains("fuego") || doc_id.contains("nomos") {
        "fuego"
    } else if doc_id.contains("tierra") || doc_id.contains("kay") {
        "tierra"
    } else if doc_id.contains("agua") || doc_id.contains("uku") {
        "agua"
    } else {
        "aire"
    };
    (el.to_string(), format!("./md/{}.md", el))
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
