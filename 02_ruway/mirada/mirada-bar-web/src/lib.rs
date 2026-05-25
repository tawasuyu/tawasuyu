//! Barra-web — binding DOM de la taskbar. Re-exporta `Task` desde
//! `barra-core` y delega el render-to-html al core. Aquí sólo viven el
//! mount sobre un `<ul>`, el listener de click delegado y los lookups
//! de posición (bounding rects) que son intrínsecos al DOM.
//!
//! Contrato HTML mínimo:
//! ```html
//! <ul id="my-tasks" class="taskbar-list" role="presentation"></ul>
//! ```
//!
//! Convenciones de clase generadas:
//! - `.taskbar-item` — cada cajita
//! - `.taskbar-item.active` — la cajita visible/foreground
//! - `.taskbar-item-dot` — punto decorativo
//! - `data-task="<id>"` — identificador único usable por CSS para theming

use std::cell::RefCell;
use std::rc::Rc;

pub use barra_core::Task;
use barra_core::{render_html, sanitize_attr};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, MouseEvent};

#[derive(Clone)]
pub struct TaskList {
    list: HtmlElement,
    on_click: Rc<RefCell<Option<Box<dyn FnMut(&str, f64, f64)>>>>,
}

impl TaskList {
    pub fn mount(list: HtmlElement) -> Result<Self, JsValue> {
        let on_click: Rc<RefCell<Option<Box<dyn FnMut(&str, f64, f64)>>>> =
            Rc::new(RefCell::new(None));
        let on_click2 = on_click.clone();
        let cb = Closure::<dyn FnMut(MouseEvent)>::new(move |e: MouseEvent| {
            let Some(target) = e.target() else { return };
            let Ok(target_el): Result<Element, _> = target.dyn_into() else { return };
            let Ok(Some(item)) = target_el.closest(".taskbar-item") else { return };
            let Some(id) = item.get_attribute("data-task") else { return };
            let rect = item.get_bounding_client_rect();
            let cx = rect.left() + rect.width() / 2.0;
            let cy = rect.top() + rect.height() / 2.0;
            if let Some(cb) = on_click2.borrow_mut().as_mut() {
                cb(&id, cx, cy);
            }
        });
        list.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
        cb.forget();
        Ok(Self { list, on_click })
    }

    pub fn set_tasks(&self, tasks: &[Task]) {
        self.list.set_inner_html(&render_html(tasks));
    }

    pub fn on_click<F: FnMut(&str, f64, f64) + 'static>(&self, cb: F) {
        *self.on_click.borrow_mut() = Some(Box::new(cb));
    }

    pub fn task_center(&self, id: &str) -> Option<(f64, f64)> {
        let sel = format!(".taskbar-item[data-task=\"{}\"]", sanitize_attr(id));
        let el = self.list.query_selector(&sel).ok().flatten()?;
        let rect = el.get_bounding_client_rect();
        Some((rect.left() + rect.width() / 2.0, rect.top() + rect.height() / 2.0))
    }

    pub fn list_el(&self) -> &HtmlElement {
        &self.list
    }
}
