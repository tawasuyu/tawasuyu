//! Vista-web — binding DOM del deck horizontal. Toda la lógica de
//! decisión de gesto/snap vive en `vista-core`; este crate sólo traduce
//! `PointerEvent`s en eventos de `DeckState` y aplica el offset al DOM.
//!
//! Contrato CSS (el caller provee):
//! ```css
//! .vista-deck { overflow: hidden; touch-action: pan-y; }
//! .vista-strip {
//!     display: flex;
//!     width: 100%;
//!     height: 100%;
//!     transform: translate3d(var(--vista-offset, 0px), 0, 0);
//!     transition: transform 360ms cubic-bezier(0.22, 0.61, 0.36, 1);
//! }
//! .vista-strip.vista-dragging,
//! .vista-strip.vista-instant { transition: none; }
//! .vista-page { flex: 0 0 100%; height: 100%; overflow-y: auto; }
//! ```

pub mod recorrido;

use std::cell::RefCell;
use std::rc::Rc;

use pluma_deck_core::{DeckState, DragOutcome};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Event, HtmlElement, PointerEvent};

#[derive(Clone)]
pub struct Deck {
    strip: HtmlElement,
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    state: DeckState,
    on_change: Option<Box<dyn FnMut(usize)>>,
}

impl Deck {
    pub fn mount(strip: HtmlElement) -> Result<Self, JsValue> {
        let inner = Rc::new(RefCell::new(Inner {
            state: DeckState::new(),
            on_change: None,
        }));
        install_pointerdown(&strip, &inner)?;
        install_pointermove(&strip, &inner)?;
        install_pointerend(&strip, &inner, "pointerup")?;
        install_pointerend(&strip, &inner, "pointercancel")?;
        install_pointerend(&strip, &inner, "pointerleave")?;
        install_resize(&strip, &inner)?;
        Ok(Self { strip, inner })
    }

    pub fn goto(&self, index: usize, smooth: bool) {
        let width = self.strip.client_width() as f64;
        let mut i = self.inner.borrow_mut();
        let r = i.state.goto(index, width);
        drop(i);
        if !smooth {
            let _ = self.strip.class_list().add_1("vista-instant");
        }
        set_offset(&self.strip, r.offset_px);
        if !smooth {
            clear_instant_next_frame(&self.strip);
        }
        if r.changed {
            let mut i = self.inner.borrow_mut();
            if let Some(cb) = i.on_change.as_mut() {
                cb(r.target_index);
            }
        }
    }

    pub fn current_index(&self) -> usize {
        self.inner.borrow().state.current_index
    }

    pub fn page_count(&self) -> u32 {
        self.strip.child_element_count()
    }

    pub fn on_change<F: FnMut(usize) + 'static>(&self, cb: F) {
        self.inner.borrow_mut().on_change = Some(Box::new(cb));
    }

    pub fn strip(&self) -> &HtmlElement {
        &self.strip
    }
}

fn install_pointerdown(strip: &HtmlElement, inner: &Rc<RefCell<Inner>>) -> Result<(), JsValue> {
    let strip2 = strip.clone();
    let inner2 = inner.clone();
    let cb = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        let width = strip2.client_width() as f64;
        inner2.borrow_mut().state.pointer_down(
            e.client_x() as f64,
            e.client_y() as f64,
            e.pointer_id(),
            width,
        );
    });
    strip.add_event_listener_with_callback("pointerdown", cb.as_ref().unchecked_ref())?;
    cb.forget();
    Ok(())
}

fn install_pointermove(strip: &HtmlElement, inner: &Rc<RefCell<Inner>>) -> Result<(), JsValue> {
    let strip2 = strip.clone();
    let inner2 = inner.clone();
    let cb = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        let outcome = inner2
            .borrow_mut()
            .state
            .pointer_move(e.client_x() as f64, e.client_y() as f64);
        match outcome {
            DragOutcome::StartHorizontal { pointer_id } => {
                let _ = strip2.class_list().add_1("vista-dragging");
                let _ = strip2.set_pointer_capture(pointer_id);
            }
            DragOutcome::DragOffset(offset) => {
                set_offset(&strip2, offset);
                e.prevent_default();
            }
            DragOutcome::Idle | DragOutcome::CancelVertical => {}
        }
    });
    let opts = web_sys::AddEventListenerOptions::new();
    opts.set_passive(false);
    strip.add_event_listener_with_callback_and_add_event_listener_options(
        "pointermove",
        cb.as_ref().unchecked_ref(),
        &opts,
    )?;
    cb.forget();
    Ok(())
}

fn install_pointerend(
    strip: &HtmlElement,
    inner: &Rc<RefCell<Inner>>,
    event_name: &str,
) -> Result<(), JsValue> {
    let strip2 = strip.clone();
    let inner2 = inner.clone();
    let cb = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        let width = strip2.client_width() as f64;
        let offset = current_offset_px(&strip2);
        let n_pages = strip2.child_element_count() as usize;
        let res = inner2.borrow_mut().state.pointer_end(offset, width, n_pages);
        let _ = strip2.class_list().remove_1("vista-dragging");
        let _ = strip2.release_pointer_capture(e.pointer_id());
        if let Some(r) = res {
            set_offset(&strip2, r.offset_px);
            if r.changed {
                let mut i = inner2.borrow_mut();
                if let Some(cb) = i.on_change.as_mut() {
                    cb(r.target_index);
                }
            }
        }
    });
    strip.add_event_listener_with_callback(event_name, cb.as_ref().unchecked_ref())?;
    cb.forget();
    Ok(())
}

fn install_resize(strip: &HtmlElement, inner: &Rc<RefCell<Inner>>) -> Result<(), JsValue> {
    let Some(window) = web_sys::window() else { return Ok(()) };
    let strip2 = strip.clone();
    let inner2 = inner.clone();
    let cb = Closure::<dyn FnMut()>::new(move || {
        let width = strip2.client_width() as f64;
        let offset = inner2.borrow().state.reposition(width);
        let _ = strip2.class_list().add_1("vista-instant");
        set_offset(&strip2, offset);
        clear_instant_next_frame(&strip2);
    });
    window.add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref())?;
    cb.forget();
    Ok(())
}

fn set_offset(strip: &HtmlElement, offset_px: f64) {
    let _ = strip
        .style()
        .set_property("--vista-offset", &format!("{}px", offset_px));
}

fn current_offset_px(strip: &HtmlElement) -> f64 {
    let s = strip
        .style()
        .get_property_value("--vista-offset")
        .unwrap_or_default();
    s.trim().trim_end_matches("px").parse::<f64>().unwrap_or(0.0)
}

fn clear_instant_next_frame(strip: &HtmlElement) {
    let strip2 = strip.clone();
    let cb = Closure::once(Box::new(move || {
        let _ = strip2.class_list().remove_1("vista-instant");
    }) as Box<dyn FnOnce()>);
    if let Some(w) = web_sys::window() {
        let _ = w.request_animation_frame(cb.as_ref().unchecked_ref());
    }
    cb.forget();
}

#[doc(hidden)]
pub fn __unused_event_marker(_e: &Event) {}
