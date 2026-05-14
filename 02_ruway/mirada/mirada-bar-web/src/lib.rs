//! Barra-web — taskbar estilo Windows, agnóstica del dominio.
//!
//! Maneja la lista dinámica de "tareas" (cajitas, una por ventana abierta)
//! dentro de un elemento `<ul>` provisto por el host. El layout del resto
//! de la barra (home button, brand, créditos, dividers, etc.) es
//! responsabilidad del host — el módulo sólo se encarga del LIST + CLICK.
//!
//! Contrato HTML mínimo:
//! ```html
//! <ul id="my-tasks" class="taskbar-list" role="presentation"></ul>
//! ```
//!
//! Convenciones de clase generadas:
//! - `.taskbar-item` — cada cajita
//! - `.taskbar-item.active` — la cajita visible/foreground
//! - `.taskbar-item-dot` — punto decorativo dentro de la cajita
//! - `data-task="<id>"` — identificador único usable por CSS para theming
//!   (`.taskbar-item[data-task="aire"] { --task-color: ... }`)
//!
//! El módulo NO inyecta CSS — el host estiliza estas clases.
//!
//! ```rust,ignore
//! let list: HtmlElement = doc.get_element_by_id("my-tasks")?.dyn_into()?;
//! let bar = barra_web::TaskList::mount(list)?;
//! bar.set_tasks(&[
//!     Task::new("aire", "AIRE"),
//!     Task::new("fuego", "FUEGO").active(),
//! ]);
//! bar.on_click(|id, cx, cy| {
//!     // El click cayó en la cajita `id`. (cx, cy) es el centro de la
//!     // cajita en CSS pixels — útil como origin de animaciones.
//! });
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, MouseEvent};

/// Una tarea (cajita) en la barra.
#[derive(Clone, Debug)]
pub struct Task {
    pub id: String,
    pub label: String,
    pub active: bool,
}

impl Task {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            active: false,
        }
    }

    pub fn active(mut self) -> Self {
        self.active = true;
        self
    }
}

#[derive(Clone)]
pub struct TaskList {
    list: HtmlElement,
    on_click: Rc<RefCell<Option<Box<dyn FnMut(&str, f64, f64)>>>>,
}

impl TaskList {
    /// Monta el módulo sobre el elemento `<ul>` provisto. Instala un único
    /// listener de click delegado: cualquier click dentro del list que caiga
    /// sobre un `.taskbar-item` dispara `on_click(id, cx, cy)`.
    pub fn mount(list: HtmlElement) -> Result<Self, JsValue> {
        let on_click: Rc<RefCell<Option<Box<dyn FnMut(&str, f64, f64)>>>> =
            Rc::new(RefCell::new(None));
        let on_click2 = on_click.clone();
        let cb = Closure::<dyn FnMut(MouseEvent)>::new(move |e: MouseEvent| {
            let Some(target) = e.target() else { return };
            let Ok(target_el): Result<Element, _> = target.dyn_into() else {
                return;
            };
            let Ok(Some(item)) = target_el.closest(".taskbar-item") else {
                return;
            };
            let Some(id) = item.get_attribute("data-task") else {
                return;
            };
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

    /// Reemplaza el contenido de la lista con las tareas dadas.
    /// Los IDs se filtran a `[a-zA-Z0-9_-]` para uso seguro en atributos.
    /// Los labels se HTML-escapan.
    pub fn set_tasks(&self, tasks: &[Task]) {
        let mut html = String::new();
        for t in tasks {
            let id_safe = sanitize_attr(&t.id);
            let label_safe = escape_text(&t.label);
            let active_cls = if t.active { " active" } else { "" };
            html.push_str(&format!(
                "<li><button class=\"taskbar-item{active_cls}\" data-task=\"{id_safe}\" type=\"button\">\
                 <span class=\"taskbar-item-dot\" aria-hidden=\"true\"></span>{label_safe}</button></li>"
            ));
        }
        self.list.set_inner_html(&html);
    }

    /// Registra (o reemplaza) el callback al click sobre una cajita.
    /// El callback recibe `(id, center_x, center_y)` en CSS pixels.
    pub fn on_click<F: FnMut(&str, f64, f64) + 'static>(&self, cb: F) {
        *self.on_click.borrow_mut() = Some(Box::new(cb));
    }

    /// Centro en CSS pixels de la cajita con `id` dado, o `None` si no existe.
    pub fn task_center(&self, id: &str) -> Option<(f64, f64)> {
        let sel = format!(".taskbar-item[data-task=\"{}\"]", sanitize_attr(id));
        let el = self.list.query_selector(&sel).ok().flatten()?;
        let rect = el.get_bounding_client_rect();
        Some((
            rect.left() + rect.width() / 2.0,
            rect.top() + rect.height() / 2.0,
        ))
    }

    /// Acceso al elemento `<ul>` host por si el caller quiere modificar
    /// styling o ARIA atributos directamente.
    pub fn list_el(&self) -> &HtmlElement {
        &self.list
    }
}

fn sanitize_attr(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_builder_defaults_inactive() {
        let t = Task::new("aire", "AIRE");
        assert!(!t.active);
        let t2 = Task::new("fuego", "FUEGO").active();
        assert!(t2.active);
    }

    #[test]
    fn sanitize_attr_removes_unsafe_chars() {
        assert_eq!(sanitize_attr("aire"), "aire");
        assert_eq!(sanitize_attr("a-b_c"), "a-b_c");
        assert_eq!(sanitize_attr("ai<re>"), "aire");
        assert_eq!(sanitize_attr("a\"b"), "ab");
    }

    #[test]
    fn escape_text_escapes_html() {
        assert_eq!(escape_text("AIRE"), "AIRE");
        assert_eq!(escape_text("<script>"), "&lt;script&gt;");
        assert_eq!(escape_text("a & b"), "a &amp; b");
    }
}
