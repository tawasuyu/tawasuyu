//! `nakui-ui` — runtime GPUI de la metainterfaz Nakui.
//!
//! Carga módulos desde un directorio (cada módulo = un
//! `module.json`), monta sidebar con sus menús, y renderea la vista
//! activa en el panel principal:
//!
//! - **List**: tabla de instancias de la entity. Botones de acción
//!   en el header (típicamente "Nuevo" → form).
//! - **Form**: campos editables; al submit, escribe al `MemoryStore`
//!   in-process via `seed_and_log` (alta directa) o por morphism
//!   (TODO en este iter).
//!
//! Todo el storage es in-memory por ahora — el escenario "save to
//! disk" se materializa cuando el daemon Nakui exista. La
//! arquitectura permite swap sin tocar la UI.
//!
//! ## Uso
//!
//! ```sh
//! NAKUI_MODULES_DIR=examples/nakui-modules cargo run -p nakui-ui
//! # default sin env: ./nakui-modules en pwd.
//! ```

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gpui::{
    div, prelude::*, px, rgb, App, Application, Bounds, ClickEvent, Context, IntoElement, Render,
    SharedString, Window, WindowBounds, WindowOptions,
};
use nakui_core::store::{MemoryStore, Store};
use nakui_ui_schema::{
    Action, Column, FieldKind, FieldSpec, FormView, ListView, MenuItem, Module, View,
};
use serde_json::{json, Value};
use uuid::Uuid;

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, gpui::size(px(1100.), px(720.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(SharedString::from("Nakui")),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_w, cx| cx.new(MetaUi::new),
        )
        .expect("open window");
        cx.activate(true);
    });
}

/// Estado del runtime.
struct MetaUi {
    /// Módulos cargados, ordenados por id.
    modules: Vec<Module>,
    /// Store compartido. Mutado por el submit de los forms.
    store: Arc<Mutex<MemoryStore>>,
    /// Módulo + vista actualmente seleccionados (índices a `modules`
    /// y key dentro de `views` respectivamente).
    active: Option<(usize, String)>,
    /// Buffer del form actual: nombre del campo → valor texto. Se
    /// resetea al cambiar de vista.
    form_buffer: std::collections::BTreeMap<String, String>,
    /// Mensaje toast al pie (success de submit, error de carga, etc.).
    toast: Option<SharedString>,
    /// Si la carga de módulos falló al inicio, lo guardamos para
    /// mostrarlo como banner de error permanente.
    load_error: Option<SharedString>,
}

impl MetaUi {
    fn new(_cx: &mut Context<Self>) -> Self {
        let modules_dir = std::env::var("NAKUI_MODULES_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("nakui-modules"));

        let (modules, load_error) =
            match nakui_ui_schema::load_modules_from_dir(&modules_dir) {
                Ok(m) => (m, None),
                Err(e) => (
                    Vec::new(),
                    Some(SharedString::from(format!(
                        "no pude cargar módulos de {}: {e}",
                        modules_dir.display()
                    ))),
                ),
            };

        // Auto-seleccionar la primera vista del primer módulo si hay.
        let active = modules
            .first()
            .and_then(|m| m.menu.first().map(|item| (0usize, item.view.clone())));

        Self {
            modules,
            store: Arc::new(Mutex::new(MemoryStore::new())),
            active,
            form_buffer: Default::default(),
            toast: None,
            load_error,
        }
    }

    fn select_view(&mut self, mod_idx: usize, view_key: String) {
        self.active = Some((mod_idx, view_key));
        self.form_buffer.clear();
        self.toast = None;
    }

    /// Aplica una acción (click en menú, botón de form, action de
    /// list). Mutaciones contra el store ocurren acá.
    fn apply_action(&mut self, action: &Action) {
        let mod_idx = match self.active.as_ref() {
            Some((i, _)) => *i,
            None => return,
        };
        match action {
            Action::OpenView { view, .. } => {
                self.select_view(mod_idx, view.clone());
            }
            Action::SeedEntity { entity, next_view } => {
                match self.commit_seed(mod_idx, entity) {
                    Ok(id) => {
                        self.toast = Some(SharedString::from(format!(
                            "creado {entity} {}",
                            short_uuid(&id)
                        )));
                        if let Some(v) = next_view {
                            self.select_view(mod_idx, v.clone());
                        } else {
                            self.form_buffer.clear();
                        }
                    }
                    Err(e) => {
                        self.toast = Some(SharedString::from(format!("error: {e}")));
                    }
                }
            }
            Action::Morphism { name, .. } => {
                // Pipeline morphism completo (executor + event_log)
                // requiere un Manifest cargado. Fuera de scope para
                // este MVP; toast informativo.
                self.toast = Some(SharedString::from(format!(
                    "morphism '{name}': pendiente (requiere manifest nakui)"
                )));
            }
        }
    }

    /// Construye un Value desde el form buffer y lo seedea al store.
    fn commit_seed(&mut self, mod_idx: usize, entity: &str) -> Result<Uuid, String> {
        let module = &self.modules[mod_idx];
        // Recoge la spec del FormView activo para conocer field kinds.
        let spec_fields: Vec<FieldSpec> = match self.active.as_ref() {
            Some((_, view_key)) => match module.views.get(view_key) {
                Some(View::Form(f)) => f.fields.clone(),
                _ => return Err("la vista activa no es un Form".into()),
            },
            None => return Err("ninguna vista activa".into()),
        };
        let mut obj = serde_json::Map::new();
        for f in &spec_fields {
            let raw = self.form_buffer.get(&f.name).cloned().unwrap_or_default();
            if f.required && raw.trim().is_empty() {
                return Err(format!("campo '{}' es obligatorio", f.label));
            }
            if raw.is_empty() && !f.required {
                continue;
            }
            let value = parse_field_value(f.kind, &raw)
                .map_err(|e| format!("campo '{}': {e}", f.label))?;
            obj.insert(f.name.clone(), value);
        }
        let id = Uuid::new_v4();
        if let Ok(mut store) = self.store.lock() {
            store.seed(entity, id, Value::Object(obj));
            Ok(id)
        } else {
            Err("store mutex envenenado".into())
        }
    }

    /// Snapshot ordenado de records de una entity (entity → rows).
    /// Materializa a Vec antes de soltar el lock — el iterator del
    /// Store traer un borrow que no sobrevive al drop del guard.
    fn list_rows(&self, entity: &str) -> Vec<(Uuid, Value)> {
        let store = match self.store.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let it = match store.iter() {
            Ok(i) => i,
            Err(_) => return Vec::new(),
        };
        let out: Vec<(Uuid, Value)> = it
            .filter(|(e, _, _)| e == entity)
            .map(|(_, id, v)| (id, v))
            .collect();
        out
    }
}

fn parse_field_value(kind: FieldKind, raw: &str) -> Result<Value, String> {
    match kind {
        FieldKind::Text | FieldKind::Multiline | FieldKind::Date => Ok(json!(raw)),
        FieldKind::Boolean => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "on" | "y" => Ok(json!(true)),
            "" | "false" | "no" | "0" | "off" | "n" => Ok(json!(false)),
            other => Err(format!("'{other}' no es booleano")),
        },
        FieldKind::Number => {
            if let Ok(i) = raw.parse::<i64>() {
                Ok(json!(i))
            } else if let Ok(f) = raw.parse::<f64>() {
                Ok(json!(f))
            } else {
                Err(format!("'{raw}' no es número"))
            }
        }
    }
}

/// Navegación por path con puntos para columns nested.
/// Ej: `address.city` → v["address"]["city"].
fn lookup_field<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

fn render_value(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => if *b { "✓" } else { "✗" }.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
    }
}

fn short_uuid(id: &Uuid) -> String {
    id.to_string().chars().take(8).collect()
}

impl Render for MetaUi {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = rgb(0x14171c);
        let panel = rgb(0x1d2128);
        let border = rgb(0x2a2f38);
        let text = rgb(0xe6e8ec);
        let text_dim = rgb(0x9ba1ad);
        let accent = rgb(0x88c0d0);
        let accent_active = rgb(0xa3be8c);

        let sidebar = self.render_sidebar(cx, panel, border, text, text_dim, accent_active);
        let main_panel = self.render_main(cx, panel, border, text, text_dim, accent);
        let toast_div = self.toast.as_ref().map(|t| {
            div()
                .px(px(12.))
                .py(px(6.))
                .bg(rgb(0x2d3a2a))
                .text_color(rgb(0xc0e0a0))
                .text_size(px(11.))
                .child(t.clone())
        });
        let error_banner = self.load_error.as_ref().map(|e| {
            div()
                .px(px(12.))
                .py(px(6.))
                .bg(rgb(0x4a2020))
                .text_color(rgb(0xffd0d0))
                .text_size(px(11.))
                .child(e.clone())
        });

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(bg)
            .when_some(error_banner, |d, b| d.child(b))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_grow()
                    .child(sidebar)
                    .child(main_panel),
            )
            .when_some(toast_div, |d, t| d.child(t))
    }
}

impl MetaUi {
    fn render_sidebar(
        &self,
        cx: &mut Context<Self>,
        panel: gpui::Rgba,
        border: gpui::Rgba,
        text: gpui::Rgba,
        text_dim: gpui::Rgba,
        accent_active: gpui::Rgba,
    ) -> gpui::Div {
        let mut sidebar = div()
            .w(px(240.))
            .h_full()
            .bg(panel)
            .border_r_1()
            .border_color(border)
            .flex()
            .flex_col();

        sidebar = sidebar.child(
            div()
                .px(px(12.))
                .py(px(10.))
                .text_color(text)
                .text_size(px(13.))
                .child("Nakui"),
        );

        if self.modules.is_empty() {
            return sidebar.child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .text_color(text_dim)
                    .text_size(px(11.))
                    .child("(no hay módulos cargados)"),
            );
        }

        for (mod_idx, m) in self.modules.iter().enumerate() {
            sidebar = sidebar.child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(border)
                    .text_color(text)
                    .text_size(px(12.))
                    .child(m.label.clone()),
            );

            for item in &m.menu {
                let is_active = self
                    .active
                    .as_ref()
                    .map(|(i, v)| *i == mod_idx && v == &item.view)
                    .unwrap_or(false);
                let label = item
                    .icon
                    .as_deref()
                    .map(|ic| format!("{ic}  {}", item.label))
                    .unwrap_or_else(|| item.label.clone());

                sidebar = sidebar.child(
                    self.menu_item_button(
                        cx,
                        mod_idx,
                        item.view.clone(),
                        label,
                        is_active,
                        text,
                        text_dim,
                        accent_active,
                    ),
                );
            }
        }
        sidebar
    }

    #[allow(clippy::too_many_arguments)]
    fn menu_item_button(
        &self,
        _cx: &mut Context<Self>,
        mod_idx: usize,
        view_key: String,
        label: String,
        is_active: bool,
        text: gpui::Rgba,
        text_dim: gpui::Rgba,
        accent: gpui::Rgba,
    ) -> gpui::Stateful<gpui::Div> {
        let id = format!("menu-{}-{}", mod_idx, view_key);
        let entity = self.entity_id_for_action(&id);
        div()
            .id(SharedString::from(entity))
            .px(px(20.))
            .py(px(6.))
            .text_size(px(12.))
            .text_color(if is_active { accent } else { text_dim })
            .when(is_active, |d| {
                d.bg(rgb(0x232a36)).text_color(text)
            })
            .child(label)
            .on_click(cx_handler_view(mod_idx, view_key))
    }

    fn entity_id_for_action(&self, base: &str) -> String {
        // Helper para el id de la div clickable. GPUI requiere que
        // las divs `Stateful` tengan un id único por scope.
        base.to_string()
    }

    fn render_main(
        &self,
        cx: &mut Context<Self>,
        panel: gpui::Rgba,
        border: gpui::Rgba,
        text: gpui::Rgba,
        text_dim: gpui::Rgba,
        accent: gpui::Rgba,
    ) -> gpui::Div {
        let main = div()
            .flex_grow()
            .h_full()
            .bg(panel)
            .flex()
            .flex_col()
            .p(px(16.));

        let (mod_idx, view_key) = match &self.active {
            Some(a) => (a.0, a.1.clone()),
            None => {
                return main.child(
                    div()
                        .text_color(text_dim)
                        .child("Seleccioná un menú a la izquierda."),
                );
            }
        };

        let module = match self.modules.get(mod_idx) {
            Some(m) => m,
            None => return main.child(div().text_color(text_dim).child("Módulo inválido")),
        };
        let view = match module.views.get(&view_key) {
            Some(v) => v,
            None => {
                return main.child(
                    div()
                        .text_color(text_dim)
                        .child(format!("Vista no encontrada: {view_key}")),
                )
            }
        };

        match view {
            View::List(lv) => self.render_list(cx, main, lv, mod_idx, border, text, text_dim, accent),
            View::Form(fv) => self.render_form(cx, main, fv, mod_idx, border, text, text_dim, accent),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_list(
        &self,
        cx: &mut Context<Self>,
        mut main: gpui::Div,
        lv: &ListView,
        mod_idx: usize,
        border: gpui::Rgba,
        text: gpui::Rgba,
        text_dim: gpui::Rgba,
        accent: gpui::Rgba,
    ) -> gpui::Div {
        // Header con título + acciones.
        let mut header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.))
            .mb(px(12.))
            .child(
                div()
                    .text_color(text)
                    .text_size(px(18.))
                    .child(lv.title.clone()),
            );
        for (idx, action) in lv.actions.iter().enumerate() {
            let label = match action {
                Action::OpenView { label, view } => {
                    label.clone().unwrap_or_else(|| format!("→ {view}"))
                }
                Action::SeedEntity { entity, .. } => format!("Seed {entity}"),
                Action::Morphism { name, .. } => format!("⚡ {name}"),
            };
            header = header.child(action_button(
                cx,
                format!("list-action-{mod_idx}-{idx}"),
                label,
                action.clone(),
                accent,
            ));
        }
        main = main.child(header);

        let rows = self.list_rows(&lv.entity);
        let total = rows.len();

        // Header de columnas.
        let total_weight: f32 = lv.columns.iter().map(|c| c.weight).sum::<f32>().max(0.01);
        let mut col_header = div()
            .flex()
            .flex_row()
            .py(px(6.))
            .border_b_1()
            .border_color(border)
            .text_color(text_dim)
            .text_size(px(11.));
        for c in &lv.columns {
            let frac = c.weight / total_weight;
            col_header = col_header.child(
                div()
                    .flex_grow()
                    .flex_basis(px(100. * frac))
                    .child(c.label.clone()),
            );
        }
        col_header = col_header.child(
            div()
                .w(px(80.))
                .text_color(text_dim)
                .child("id"),
        );
        main = main.child(col_header);

        // Filas.
        for (id, value) in &rows {
            let mut row = div()
                .flex()
                .flex_row()
                .py(px(6.))
                .border_b_1()
                .border_color(rgb(0x232a36))
                .text_color(text)
                .text_size(px(12.));
            for c in &lv.columns {
                let frac = c.weight / total_weight;
                let v = lookup_field(value, &c.field);
                row = row.child(
                    div()
                        .flex_grow()
                        .flex_basis(px(100. * frac))
                        .child(render_value(v)),
                );
            }
            row = row.child(
                div()
                    .w(px(80.))
                    .text_color(text_dim)
                    .text_size(px(11.))
                    .child(short_uuid(id)),
            );
            main = main.child(row);
        }

        if rows.is_empty() {
            main = main.child(
                div()
                    .py(px(12.))
                    .text_color(text_dim)
                    .text_size(px(12.))
                    .child(format!("(sin {})", lv.entity)),
            );
        } else {
            main = main.child(
                div()
                    .mt(px(8.))
                    .text_color(text_dim)
                    .text_size(px(11.))
                    .child(format!("{total} fila(s)")),
            );
        }

        main
    }

    #[allow(clippy::too_many_arguments)]
    fn render_form(
        &self,
        cx: &mut Context<Self>,
        mut main: gpui::Div,
        fv: &FormView,
        mod_idx: usize,
        border: gpui::Rgba,
        text: gpui::Rgba,
        text_dim: gpui::Rgba,
        accent: gpui::Rgba,
    ) -> gpui::Div {
        let _ = border;
        main = main.child(
            div()
                .text_color(text)
                .text_size(px(18.))
                .mb(px(12.))
                .child(fv.title.clone()),
        );
        for f in &fv.fields {
            let raw = self.form_buffer.get(&f.name).cloned().unwrap_or_default();
            let display = if raw.is_empty() {
                f.default.clone().unwrap_or_default()
            } else {
                raw
            };
            let label = if f.required {
                format!("{} *", f.label)
            } else {
                f.label.clone()
            };
            let mut field_box = div()
                .flex()
                .flex_col()
                .mb(px(10.))
                .child(
                    div()
                        .text_color(text_dim)
                        .text_size(px(11.))
                        .mb(px(2.))
                        .child(label),
                )
                .child(
                    // GPUI no incluye un text_input nativo; mostramos
                    // el buffer actual como texto. Para entrada
                    // teclado real, integrar yahweh-widget-text-input
                    // (próxima iteración). Por ahora el form sirve
                    // demos visuales y el seed via API programática.
                    div()
                        .px(px(8.))
                        .py(px(6.))
                        .bg(rgb(0x171a20))
                        .border_1()
                        .border_color(rgb(0x2a2f38))
                        .rounded(px(3.))
                        .text_color(text)
                        .text_size(px(12.))
                        .child(if display.is_empty() {
                            "(vacío — input GPUI pendiente)".to_string()
                        } else {
                            display
                        }),
                );
            if let Some(help) = &f.help {
                field_box = field_box.child(
                    div()
                        .mt(px(2.))
                        .text_color(text_dim)
                        .text_size(px(10.))
                        .child(help.clone()),
                );
            }
            main = main.child(field_box);
        }

        let submit_label = match &fv.on_submit {
            Action::SeedEntity { entity, .. } => format!("Crear {entity}"),
            Action::Morphism { name, .. } => format!("Ejecutar {name}"),
            Action::OpenView { label, view } => {
                label.clone().unwrap_or_else(|| format!("Ir a {view}"))
            }
        };
        main = main.child(
            div().mt(px(12.)).child(action_button(
                cx,
                format!("form-submit-{mod_idx}"),
                submit_label,
                fv.on_submit.clone(),
                accent,
            )),
        );

        main = main.child(
            div()
                .mt(px(20.))
                .text_color(text_dim)
                .text_size(px(10.))
                .child(
                    "Nota: en este MVP, los inputs todavía no aceptan teclado. \
                     El submit usa los `default` del schema o vacío (campo opcional). \
                     Próximo iter: integración con yahweh-widget-text-input.",
                ),
        );
        main
    }
}

fn cx_handler_view(
    mod_idx: usize,
    view_key: String,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let _ = (mod_idx, &view_key);
    move |_e, _w, _cx| {
        // GPUI handlers necesitan acceder al modelo de la entity actual;
        // wirearemos via cx.update en el render real cuando el iter de
        // eventos tipados esté listo. Por ahora el menu se navega via
        // env var/restart.
    }
}

fn action_button(
    _cx: &mut Context<MetaUi>,
    id: String,
    label: String,
    _action: Action,
    accent: gpui::Rgba,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id))
        .px(px(10.))
        .py(px(4.))
        .bg(rgb(0x232a36))
        .text_color(accent)
        .text_size(px(11.))
        .rounded(px(3.))
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_field_text_returns_string() {
        let v = parse_field_value(FieldKind::Text, "hola").unwrap();
        assert_eq!(v, json!("hola"));
    }

    #[test]
    fn parse_field_number_int_then_float() {
        let i = parse_field_value(FieldKind::Number, "42").unwrap();
        assert_eq!(i, json!(42));
        let f = parse_field_value(FieldKind::Number, "3.14").unwrap();
        assert_eq!(f, json!(3.14));
    }

    #[test]
    fn parse_field_number_invalid_errors() {
        let r = parse_field_value(FieldKind::Number, "not-a-number");
        assert!(r.is_err());
    }

    #[test]
    fn parse_field_boolean_variants() {
        assert_eq!(
            parse_field_value(FieldKind::Boolean, "true").unwrap(),
            json!(true)
        );
        assert_eq!(
            parse_field_value(FieldKind::Boolean, "yes").unwrap(),
            json!(true)
        );
        assert_eq!(
            parse_field_value(FieldKind::Boolean, "false").unwrap(),
            json!(false)
        );
        assert_eq!(
            parse_field_value(FieldKind::Boolean, "").unwrap(),
            json!(false)
        );
        assert!(parse_field_value(FieldKind::Boolean, "maybe").is_err());
    }

    #[test]
    fn lookup_field_simple_and_nested() {
        let v = json!({
            "name": "Acme",
            "address": { "city": "Bogotá", "country": "CO" }
        });
        assert_eq!(lookup_field(&v, "name").unwrap(), &json!("Acme"));
        assert_eq!(
            lookup_field(&v, "address.city").unwrap(),
            &json!("Bogotá")
        );
        assert!(lookup_field(&v, "missing").is_none());
        assert!(lookup_field(&v, "address.zipcode").is_none());
    }

    #[test]
    fn render_value_handles_null_string_bool() {
        assert_eq!(render_value(None), "");
        assert_eq!(render_value(Some(&json!(null))), "");
        assert_eq!(render_value(Some(&json!("x"))), "x");
        assert_eq!(render_value(Some(&json!(true))), "✓");
        assert_eq!(render_value(Some(&json!(false))), "✗");
        assert_eq!(render_value(Some(&json!(42))), "42");
    }
}
