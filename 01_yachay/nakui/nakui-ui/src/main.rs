//! `nakui-ui` — runtime GPUI de la metainterfaz Nakui.
//!
//! Carga módulos desde un directorio (cada módulo = un
//! `module.json`), monta sidebar con sus menús, y renderea la vista
//! activa en el panel principal:
//!
//! - **List**: tabla de instancias del entity. Botones de acción en
//!   el header (típicamente "Nuevo" → form).
//! - **Form**: campos editables (con `yahweh-widget-text-input` para
//!   teclado real); al submit, escribe al `MemoryStore` in-process
//!   via `seed_entity` (alta directa) o por morphism (TODO en este
//!   iter).
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

mod backend;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, App, Application, Bounds, ClickEvent, Context, Entity, IntoElement,
    KeyDownEvent, Render, SharedString, Window, WindowBounds, WindowOptions,
};

use brahman_cards::CardBody;
use nakui_core::executor::Executor;
use yahweh_meta_runtime::{
    compute_clear_fields, compute_field_delta, human_label_for_record, parse_field_value,
    render_value, resolve_param_value, short_uuid, validate_entity_refs, value_to_input_text,
    MetaBackend, WriteOutcome,
};
use yahweh_meta_schema::{
    Action, FieldKind, FieldSpec, FormView, ListView, Module, View,
};
use serde_json::Value;
use uuid::Uuid;
use yahweh_theme::Theme;
use yahweh_widget_text_input::TextInput;

use crate::backend::NakuiBackend;

fn main() {
    Application::new().run(|cx: &mut App| {
        // El text input pide Theme::global; instalarlo antes de
        // crear el window evita que panicee.
        Theme::install_default(cx);

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

/// Estado del runtime de UI. Toda la persistencia/ejecución está
/// detrás del trait `MetaBackend`; este struct sólo conoce GPUI
/// state y el schema de los módulos.
struct MetaUi {
    /// Módulos cargados, ordenados por id.
    modules: Vec<Module>,
    /// Backend que ejecuta seed/update/delete/morphism. Para Nakui
    /// esto wirea al stack de event_log + MemoryStore + Executors.
    /// Otra app podría implementar `MetaBackend` distinto.
    backend: NakuiBackend,
    /// (módulo idx, vista key) actualmente activos.
    active: Option<(usize, String)>,
    /// Inputs vivos para el form actual: nombre del campo → TextInput.
    /// Se reemplaza al cambiar de vista (drop de los anteriores).
    form_inputs: BTreeMap<String, Entity<TextInput>>,
    /// Si está set, el próximo render del Form pre-llena los inputs
    /// con los valores del record indicado, y `commit_seed` emite
    /// un `update` (no un seed nuevo). Limpia al cambiar de view o
    /// tras submit exitoso.
    editing: Option<(String, Uuid)>,
    /// Si está set, el banner modal de confirmación de delete está
    /// activo: `(entity, id)` del record que el usuario marcó para
    /// borrar. Permanece hasta que el usuario click [Confirmar]
    /// (ejecuta `commit_delete` y limpia) o [Cancelar] (sólo limpia).
    /// Navegación a otra view también cancela.
    pending_delete: Option<(String, Uuid)>,
    /// Mensaje toast al pie (success de submit, error de carga, etc.).
    toast: Option<SharedString>,
    /// Si la carga de módulos falló al inicio.
    load_error: Option<SharedString>,
}

impl MetaUi {
    fn new(_cx: &mut Context<Self>) -> Self {
        let modules_dir = std::env::var("NAKUI_MODULES_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("nakui-modules"));

        // Carga via el brazo unificado: brahman_cards::load_cards_from_dir
        // walkea subdirs buscando card.ncl/card.json/module.ncl/module.json,
        // dispatcha al reader apropiado, devuelve Vec<Card>. Acá filtramos
        // a los UiModule body variants y aplicamos las validaciones
        // específicas de la UI (validate de cada Module + dedup de id).
        // Cards de otros kinds (Ente, Monad) que aparezcan en el dir se
        // skipean con un msg al banner — no son fatales pero el usuario
        // sabe que estaban ahí.
        let (modules, mut load_error) = match load_ui_modules(&modules_dir) {
            Ok((mods, skipped)) => {
                let toast = if skipped.is_empty() {
                    None
                } else {
                    Some(SharedString::from(format!(
                        "skipeé {} card(s) no-UiModule en {}: {:?}",
                        skipped.len(),
                        modules_dir.display(),
                        skipped
                    )))
                };
                (mods, toast)
            }
            Err(e) => (
                Vec::new(),
                Some(SharedString::from(format!(
                    "no pude cargar módulos de {}: {e}",
                    modules_dir.display()
                ))),
            ),
        };

        // Cargar Executors para los módulos que declararon
        // `nakui_module_dir`. Resolvemos paths relativos al
        // directorio del modules (NAKUI_MODULES_DIR/<id>/), no al
        // pwd. Cualquier error de carga deja la entry afuera y
        // anota al banner — el morphism queda inejecutable para ese
        // módulo pero el resto sigue funcionando.
        let mut executors: BTreeMap<String, Arc<Executor>> = BTreeMap::new();
        for m in &modules {
            let Some(rel) = &m.nakui_module_dir else {
                continue;
            };
            let module_root = modules_dir.join(&m.id);
            let nakui_dir = if std::path::Path::new(rel).is_absolute() {
                PathBuf::from(rel)
            } else {
                module_root.join(rel)
            };
            match Executor::load_module(&nakui_dir) {
                Ok(exec) => {
                    executors.insert(m.id.clone(), Arc::new(exec));
                }
                Err(e) => {
                    let msg = format!(
                        "módulo {}: no pude cargar executor nakui en {}: {e}",
                        m.id,
                        nakui_dir.display()
                    );
                    match &load_error {
                        Some(prev) => {
                            load_error = Some(SharedString::from(format!("{prev}; {msg}")));
                        }
                        None => load_error = Some(SharedString::from(msg)),
                    }
                }
            }
        }

        // Persistencia: el backend abre el log + snapshot + replay.
        // Path del log por env `NAKUI_EVENT_LOG` (default
        // `./nakui-ui-state.jsonl`). Threshold de auto-compaction
        // via env `NAKUI_SNAPSHOT_THRESHOLD` (default 50).
        let log_path = std::env::var("NAKUI_EVENT_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("nakui-ui-state.jsonl"));
        let snapshot_threshold: usize = std::env::var("NAKUI_SNAPSHOT_THRESHOLD")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50);
        let (backend, status) =
            NakuiBackend::open(log_path, snapshot_threshold, executors);
        let initial_toast = status.init_toast.map(SharedString::from);
        if let Some(msg) = status.load_error {
            load_error = Some(match load_error {
                Some(prev) => SharedString::from(format!("{prev}; {msg}")),
                None => SharedString::from(msg),
            });
        }

        let active = modules
            .first()
            .and_then(|m| m.menu.first().map(|item| (0usize, item.view.clone())));

        Self {
            modules,
            backend,
            active,
            form_inputs: BTreeMap::new(),
            editing: None,
            pending_delete: None,
            toast: initial_toast,
            load_error,
        }
    }

    /// Cambia la vista activa. Si la nueva vista es un Form, crea
    /// `TextInput` entities para cada field. Pre-llena con valores
    /// del record si hay `editing` para esa entity; si no, usa el
    /// `default` del schema.
    /// Drop de los inputs anteriores ocurre al sobreescribir el map.
    fn select_view(&mut self, mod_idx: usize, view_key: String, cx: &mut Context<Self>) {
        self.active = Some((mod_idx, view_key.clone()));
        self.toast = None;
        // Navegar a otra view cancela cualquier delete pendiente:
        // el record marcado puede no estar visible en la nueva view.
        self.pending_delete = None;
        self.form_inputs = BTreeMap::new();
        if let Some(module) = self.modules.get(mod_idx) {
            if let Some(View::Form(form)) = module.views.get(&view_key) {
                // Snapshot del record si estamos editando esta entity.
                let editing_record: Option<Value> = self.editing.as_ref().and_then(|(e, id)| {
                    if e == &form.entity {
                        self.backend.load_record(e, *id)
                    } else {
                        None
                    }
                });
                for f in &form.fields {
                    let initial = if let Some(rec) = &editing_record {
                        rec.get(&f.name)
                            .map(value_to_input_text)
                            .unwrap_or_else(|| f.default.clone().unwrap_or_default())
                    } else {
                        f.default.clone().unwrap_or_default()
                    };
                    let input = cx.new(|cx| TextInput::new(initial, cx));
                    self.form_inputs.insert(f.name.clone(), input);
                }
            } else {
                // Cambiar a una view que no es Form invalida el editing
                // pendiente.
                self.editing = None;
            }
        }
        cx.notify();
    }

    /// Inicia un edit del record: setea `editing` y abre la primera
    /// view de tipo Form del módulo (convención: la del schema).
    fn open_edit(
        &mut self,
        mod_idx: usize,
        entity: String,
        id: Uuid,
        cx: &mut Context<Self>,
    ) {
        self.editing = Some((entity.clone(), id));
        let form_view_key = self.modules.get(mod_idx).and_then(|m| {
            m.views
                .iter()
                .find_map(|(key, v)| match v {
                    View::Form(form) if form.entity == entity => Some(key.clone()),
                    _ => None,
                })
        });
        match form_view_key {
            Some(key) => self.select_view(mod_idx, key, cx),
            None => {
                self.toast = Some(SharedString::from(format!(
                    "no hay form view para entity '{entity}' en este módulo"
                )));
                self.editing = None;
                cx.notify();
            }
        }
    }

    /// Borra un record. Emite Morphism con un FieldOp::Delete + lo
    /// aplica al store via `apply` (no via remove directo, mantiene
    /// el modelo de "todo cambio post-seed pasa por ops").
    /// Borra un record vía `MetaBackend::delete`. Devuelve el outcome
    /// del backend (incluye eventual `post_status` del compact tick).
    fn commit_delete(
        &mut self,
        entity: &str,
        id: Uuid,
    ) -> Result<WriteOutcome, String> {
        self.backend.delete(entity, id)
    }

    /// Aplica una acción (click en menú, botón de form, action de
    /// list). Mutaciones contra el backend ocurren acá.
    fn apply_action(&mut self, action: Action, cx: &mut Context<Self>) {
        let mod_idx = match self.active.as_ref() {
            Some((i, _)) => *i,
            None => return,
        };
        match action {
            Action::OpenView { view, .. } => {
                // Salir a otra view cancela el edit pendiente.
                self.editing = None;
                self.select_view(mod_idx, view, cx);
            }
            Action::SeedEntity { entity, next_view } => {
                let was_editing = self.editing.is_some();
                match self.commit_seed(mod_idx, &entity, cx) {
                    Ok(outcome) => {
                        let toast_msg = format_seed_toast(&entity, was_editing, &outcome);
                        self.toast = Some(append_compact_msg(toast_msg, outcome.post_status));
                        // Limpia editing tras un commit exitoso —
                        // el record ya está sincronizado (incluso
                        // un NoChange cierra el modo edit).
                        self.editing = None;
                        if let Some(v) = next_view {
                            self.select_view(mod_idx, v, cx);
                        } else {
                            for input in self.form_inputs.values() {
                                input.update(cx, |inp, cx| inp.set_text("", cx));
                            }
                        }
                    }
                    Err(e) => {
                        self.toast = Some(SharedString::from(format!("error: {e}")));
                    }
                }
                cx.notify();
            }
            Action::Morphism {
                name,
                inputs,
                params,
                next_view,
            } => {
                match self.commit_morphism(mod_idx, &name, &inputs, &params, cx) {
                    Ok(outcome) => {
                        let base = format!(
                            "morphism '{name}' OK ({} op(s) aplicadas)",
                            outcome.changed
                        );
                        self.toast = Some(append_compact_msg(base, outcome.post_status));
                        if let Some(v) = next_view {
                            self.select_view(mod_idx, v, cx);
                        }
                    }
                    Err(e) => {
                        self.toast = Some(SharedString::from(format!(
                            "morphism '{name}' falló: {e}"
                        )));
                    }
                }
                cx.notify();
            }
        }
    }

    /// Despacha un morphism via el backend. Resuelve inputs (UUIDs)
    /// + params (Value object) leyendo los TextInput del form.
    fn commit_morphism(
        &mut self,
        mod_idx: usize,
        morphism: &str,
        inputs_map: &BTreeMap<String, String>,
        params_fields: &[String],
        cx: &mut Context<Self>,
    ) -> Result<WriteOutcome, String> {
        let module = self
            .modules
            .get(mod_idx)
            .ok_or_else(|| "módulo inválido".to_string())?;
        let module_id = module.id.clone();

        // Resolver inputs: por cada (role, field_name), parsear el
        // value del input como Uuid.
        let mut inputs: BTreeMap<String, Uuid> = BTreeMap::new();
        for (role, field_name) in inputs_map {
            let raw = self
                .form_inputs
                .get(field_name)
                .map(|inp| inp.read(&*cx).text().to_string())
                .ok_or_else(|| format!("input field '{field_name}' no existe en el form"))?;
            let id = Uuid::parse_str(raw.trim()).map_err(|_| {
                format!(
                    "input '{role}' (field '{field_name}'): '{raw}' no es UUID válido"
                )
            })?;
            inputs.insert(role.clone(), id);
        }

        // Resolver params: si la lista está vacía, todos los fields
        // del form que no estén en `inputs_map` van a params.
        let input_field_set: std::collections::BTreeSet<&String> = inputs_map.values().collect();
        let mut params_obj = serde_json::Map::new();
        let field_iter: Vec<String> = if params_fields.is_empty() {
            self.form_inputs
                .keys()
                .filter(|k| !input_field_set.contains(*k))
                .cloned()
                .collect()
        } else {
            params_fields.to_vec()
        };

        // FieldSpec del Form view activo para parseo estricto por kind.
        let active_form_fields: Option<Vec<FieldSpec>> =
            self.active.as_ref().and_then(|(_, vk)| {
                module.views.get(vk).and_then(|v| match v {
                    View::Form(f) => Some(f.fields.clone()),
                    _ => None,
                })
            });

        for field_name in field_iter {
            let raw = self
                .form_inputs
                .get(&field_name)
                .map(|inp| inp.read(&*cx).text().to_string())
                .unwrap_or_default();
            let spec = active_form_fields
                .as_ref()
                .and_then(|fs| fs.iter().find(|f| f.name == field_name));
            let value = resolve_param_value(&field_name, &raw, spec)?;
            params_obj.insert(field_name, value);
        }

        self.backend
            .morphism(&module_id, morphism, inputs, Value::Object(params_obj))
    }

    /// Construye un payload desde los TextInput vivos y delega al
    /// backend (`seed` para alta nueva, `update` con set+clear para
    /// edit). Devuelve el `WriteOutcome` del backend (incluye
    /// `changed` y `post_status` para el toast).
    fn commit_seed(
        &mut self,
        mod_idx: usize,
        entity: &str,
        cx: &mut Context<Self>,
    ) -> Result<WriteOutcome, String> {
        let module = &self.modules[mod_idx];
        let spec_fields: Vec<FieldSpec> = match self.active.as_ref() {
            Some((_, view_key)) => match module.views.get(view_key) {
                Some(View::Form(f)) => f.fields.clone(),
                _ => return Err("la vista activa no es un Form".into()),
            },
            None => return Err("ninguna vista activa".into()),
        };
        let mut obj = serde_json::Map::new();
        // Fields que el form deja vacíos y son optional: candidatos
        // a Clear en el path de EDIT.
        let mut to_clear: Vec<String> = Vec::new();
        // EntityRef refs a validar tras parse loop (en una toma del
        // backend en lugar de N).
        let mut entity_refs: Vec<(String, String, Uuid)> = Vec::new();
        for f in &spec_fields {
            let raw = self
                .form_inputs
                .get(&f.name)
                .map(|input| input.read(cx).text().to_string())
                .unwrap_or_default();
            if f.required && raw.trim().is_empty() {
                return Err(format!("campo '{}' es obligatorio", f.label));
            }
            if raw.is_empty() && !f.required {
                to_clear.push(f.name.clone());
                continue;
            }
            let value = parse_field_value(f.kind, &raw)
                .map_err(|e| format!("campo '{}': {e}", f.label))?;
            if f.kind == FieldKind::EntityRef {
                if let (Some(target), Some(uuid_str)) = (&f.ref_entity, value.as_str()) {
                    let id = Uuid::parse_str(uuid_str)
                        .expect("parse_field_value validated UUID");
                    entity_refs.push((f.label.clone(), target.clone(), id));
                }
            }
            obj.insert(f.name.clone(), value);
        }
        // Validar EntityRefs contra el backend actual. Cierre wrappea
        // backend.load_record para mantener la firma de
        // validate_entity_refs (que es store-agnóstica).
        if !entity_refs.is_empty() {
            let backend = &self.backend;
            validate_entity_refs(|e, id| backend.load_record(e, id), &entity_refs)?;
        }

        // Ramificación: edit (hay editing para esta entity) vs alta.
        let editing_match = self.editing.as_ref().filter(|(e, _)| e == entity).cloned();

        if let Some((_, id)) = editing_match {
            // EDIT: cargar current, computar delta, llamar
            // backend.update con set+clear pre-computados.
            let current = self.backend.load_record(entity, id).unwrap_or(Value::Null);
            let set_delta = compute_field_delta(&current, &obj);
            let clear_fields = compute_clear_fields(&current, &to_clear);
            self.backend.update(entity, id, set_delta, clear_fields)
        } else {
            // SEED: alta nueva — el backend genera el Uuid.
            self.backend.seed(entity, obj)
        }
    }

    /// Snapshot ordenado de records de una entity (proxy al backend).
    fn list_rows(&self, entity: &str) -> Vec<(Uuid, Value)> {
        self.backend.list_records(entity)
    }
}

/// Formatea el toast para la rama Action::SeedEntity según el
/// `WriteOutcome` del backend. `was_editing` distingue "creado"
/// vs "actualizado" — el WriteOutcome solo no alcanza porque
/// `seed` y `update` ambos devuelven `id = Some(...)`.
fn format_seed_toast(entity: &str, was_editing: bool, outcome: &WriteOutcome) -> String {
    let id_short = outcome
        .id
        .map(|id| short_uuid(&id))
        .unwrap_or_default();
    match (was_editing, outcome.changed) {
        (false, _) => format!("creado {entity} {id_short}"),
        (true, 0) => format!("{entity} {id_short} sin cambios — no log entry"),
        (true, n) => format!("actualizado {entity} {id_short} ({n} campo(s))"),
    }
}

/// Carga UiModules desde un directorio via el brazo unificado
/// `brahman_cards::load_cards_from_dir`. Aplica las reglas
/// específicas de la UI:
///  - Sólo `CardBody::UiModule` cuenta; otros body kinds
///    (Ente, Monad, ...) se reportan en el `skipped` para que el
///    runtime los muestre como banner informativo.
///  - Cada `Module` se valida via `Module::validate()`.
///  - Detecta `id` duplicados entre módulos UiModule (el runtime
///    los direcciona por id; duplicados serían ambiguos).
///
/// Devuelve `(modules, skipped_ids)` ordenados por id.
fn load_ui_modules(
    dir: &std::path::Path,
) -> Result<(Vec<Module>, Vec<String>), String> {
    let cards = brahman_cards::load_cards_from_dir(dir)
        .map_err(|e| e.to_string())?;
    let mut modules: Vec<Module> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for c in cards {
        match c.body {
            CardBody::UiModule(m) => modules.push(m),
            other => skipped.push(format!("{}({})", c.id, other.kind_name())),
        }
    }
    for m in &modules {
        m.validate()
            .map_err(|e| format!("módulo '{}' inválido: {e}", m.id))?;
    }
    modules.sort_by(|a, b| a.id.cmp(&b.id));
    let mut prev: Option<&Module> = None;
    for cur in &modules {
        if let Some(p) = prev {
            if p.id == cur.id {
                return Err(format!(
                    "id de módulo duplicado: '{}' aparece más de una vez",
                    cur.id
                ));
            }
        }
        prev = Some(cur);
    }
    Ok((modules, skipped))
}

fn append_compact_msg(base: String, compact_msg: Option<String>) -> SharedString {
    match compact_msg {
        Some(m) => SharedString::from(format!("{base}; {m}")),
        None => SharedString::from(base),
    }
}

/// Walker dentro de un `Value` por path con `.` como separador.
/// Local porque sólo lo usa la lista renderer y no tiene tests
/// dedicados afuera. Si crece su uso se puede mover a meta-runtime.
fn lookup_field<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

impl Render for MetaUi {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = gpui::rgb(0x14171c);
        let panel = gpui::rgb(0x1d2128);
        let border = gpui::rgb(0x2a2f38);
        let text = gpui::rgb(0xe6e8ec);
        let text_dim = gpui::rgb(0x9ba1ad);
        let accent = gpui::rgb(0x88c0d0);
        let accent_active = gpui::rgb(0xa3be8c);

        let sidebar = self.render_sidebar(cx, panel, border, text, text_dim, accent_active);
        let main_panel = self.render_main(cx, panel, border, text, text_dim, accent);
        let confirm_banner = self.render_confirm_delete_banner(cx);
        let toast_div = self.toast.as_ref().map(|t| {
            div()
                .px(px(12.))
                .py(px(6.))
                .bg(gpui::rgb(0x2d3a2a))
                .text_color(gpui::rgb(0xc0e0a0))
                .text_size(px(11.))
                .child(t.clone())
        });
        let error_banner = self.load_error.as_ref().map(|e| {
            div()
                .px(px(12.))
                .py(px(6.))
                .bg(gpui::rgb(0x4a2020))
                .text_color(gpui::rgb(0xffd0d0))
                .text_size(px(11.))
                .child(e.clone())
        });

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(bg)
            // Capture phase: el Esc llega al root ANTES que cualquier
            // TextInput descendiente. Si hay un delete pendiente, lo
            // cancelamos. Sin pending no hacemos nada (el evento sigue
            // su flujo normal y el TextInput recibe el Esc bubble).
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _w, cx| {
                if event.keystroke.key != "escape" {
                    return;
                }
                if let Some((entity, _id)) = this.pending_delete.take() {
                    this.toast = Some(SharedString::from(format!(
                        "delete cancelado ({entity}) [esc]"
                    )));
                    cx.notify();
                }
            }))
            .when_some(error_banner, |d, b| d.child(b))
            .when_some(confirm_banner, |d, b| d.child(b))
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
    /// Renderea el banner modal de confirmación cuando hay un delete
    /// pendiente. Devuelve `None` si no hay nada que confirmar.
    ///
    /// UX: banner amber prominente arriba de todo el contenido. No es
    /// un overlay flotante (GPUI no expone z-index fácilmente sin
    /// setup) — es un row del flex_col raíz, así que el contenido
    /// debajo se desplaza unos pixels mientras está activo. Suficiente
    /// para forzar al usuario a leer + click.
    fn render_confirm_delete_banner(&self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let (entity, id) = self.pending_delete.as_ref()?;
        let entity_owned = entity.clone();
        let id_owned = *id;
        let entity_for_cancel = entity_owned.clone();
        let id_short = short_uuid(&id_owned);
        let entity_for_confirm = entity_owned.clone();

        let banner_bg = gpui::rgb(0x4a3a1a);
        let banner_text = gpui::rgb(0xf0e0a0);
        let confirm_bg = gpui::rgb(0x6a2222);
        let confirm_text = gpui::rgb(0xffd0d0);
        let cancel_bg = gpui::rgb(0x2a2f38);
        let cancel_text = gpui::rgb(0xc0c8d0);

        Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.))
                .px(px(12.))
                .py(px(8.))
                .bg(banner_bg)
                .text_color(banner_text)
                .text_size(px(12.))
                .child(
                    div()
                        .flex_grow()
                        .flex()
                        .flex_col()
                        .child(format!("¿Borrar {entity_owned} {id_short}?"))
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(gpui::rgb(0xc0a070))
                                .child("Esc para cancelar · click [Confirmar] para borrar"),
                        ),
                )
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "confirm-del-cancel-{}",
                            id_owned
                        )))
                        .px(px(10.))
                        .py(px(4.))
                        .bg(cancel_bg)
                        .text_color(cancel_text)
                        .hover(|d| d.bg(gpui::rgb(0x3a3f48)))
                        .child("Cancelar")
                        .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                            this.pending_delete = None;
                            this.toast = Some(SharedString::from(format!(
                                "delete cancelado ({entity_for_cancel})"
                            )));
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "confirm-del-ok-{}",
                            id_owned
                        )))
                        .px(px(10.))
                        .py(px(4.))
                        .bg(confirm_bg)
                        .text_color(confirm_text)
                        .hover(|d| d.bg(gpui::rgb(0x8a2828)))
                        .child("Confirmar")
                        .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                            // Limpiar primero para que un fallo del
                            // commit no deje el banner colgado y el
                            // toast tape el banner.
                            this.pending_delete = None;
                            match this.commit_delete(&entity_for_confirm, id_owned) {
                                Ok(outcome) => {
                                    let base = format!(
                                        "borrado {entity_for_confirm} {}",
                                        short_uuid(&id_owned)
                                    );
                                    this.toast = Some(append_compact_msg(
                                        base,
                                        outcome.post_status,
                                    ));
                                }
                                Err(e) => {
                                    this.toast = Some(SharedString::from(format!(
                                        "error borrando: {e}"
                                    )));
                                }
                            }
                            cx.notify();
                        })),
                ),
        )
    }

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

        // Snapshot del active para evitar borrow del self adentro de la closure.
        let active_snapshot = self.active.clone();

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
                let is_active = active_snapshot
                    .as_ref()
                    .map(|(i, v)| *i == mod_idx && v == &item.view)
                    .unwrap_or(false);
                let label = item
                    .icon
                    .as_deref()
                    .map(|ic| format!("{ic}  {}", item.label))
                    .unwrap_or_else(|| item.label.clone());

                let view_key = item.view.clone();
                sidebar = sidebar.child(
                    div()
                        .id(SharedString::from(format!(
                            "menu-{}-{}",
                            mod_idx, item.view
                        )))
                        .px(px(20.))
                        .py(px(6.))
                        .text_size(px(12.))
                        .text_color(if is_active { accent_active } else { text_dim })
                        .when(is_active, |d| {
                            d.bg(gpui::rgb(0x232a36)).text_color(text)
                        })
                        .hover(|d| d.bg(gpui::rgb(0x1f2630)))
                        .child(label)
                        .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                            this.select_view(mod_idx, view_key.clone(), cx);
                        })),
                );
            }
        }
        sidebar
    }

    fn render_main(
        &mut self,
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

        let view = match self
            .modules
            .get(mod_idx)
            .and_then(|m| m.views.get(&view_key))
        {
            Some(v) => v.clone(),
            None => {
                return main.child(
                    div()
                        .text_color(text_dim)
                        .child(format!("Vista no encontrada: {view_key}")),
                );
            }
        };

        match view {
            View::List(lv) => self.render_list(cx, main, &lv, mod_idx, border, text, text_dim, accent),
            View::Form(fv) => self.render_form(cx, main, &fv, mod_idx, border, text, text_dim, accent),
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
            let action_clone = action.clone();
            header = header.child(
                div()
                    .id(SharedString::from(format!(
                        "list-action-{mod_idx}-{idx}"
                    )))
                    .px(px(10.))
                    .py(px(4.))
                    .bg(gpui::rgb(0x232a36))
                    .text_color(accent)
                    .text_size(px(11.))
                    .rounded(px(3.))
                    .hover(|d| d.bg(gpui::rgb(0x2c3540)))
                    .child(label)
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        this.apply_action(action_clone.clone(), cx);
                    })),
            );
        }
        main = main.child(header);

        let rows = self.list_rows(&lv.entity);
        let total = rows.len();

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
        col_header = col_header
            .child(div().w(px(80.)).text_color(text_dim).child("id"))
            .child(div().w(px(70.)).text_color(text_dim).child("acciones"));
        main = main.child(col_header);

        let entity_name = lv.entity.clone();
        for (id, value) in &rows {
            let id_copy = *id;
            let entity_for_edit = entity_name.clone();
            let entity_for_delete = entity_name.clone();
            let mut row = div()
                .flex()
                .flex_row()
                .py(px(6.))
                .border_b_1()
                .border_color(gpui::rgb(0x232a36))
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
            // Acciones: ✎ edit + ✕ delete por fila.
            row = row.child(
                div()
                    .w(px(70.))
                    .flex()
                    .flex_row()
                    .gap(px(4.))
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "row-edit-{mod_idx}-{id_copy}"
                            )))
                            .px(px(6.))
                            .text_color(accent)
                            .text_size(px(13.))
                            .hover(|d| d.bg(gpui::rgb(0x2c3540)))
                            .child("✎")
                            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                                this.open_edit(mod_idx, entity_for_edit.clone(), id_copy, cx);
                            })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "row-del-{mod_idx}-{id_copy}"
                            )))
                            .px(px(6.))
                            .text_color(gpui::rgb(0xd07070))
                            .text_size(px(13.))
                            .hover(|d| d.bg(gpui::rgb(0x4a2020)))
                            .child("✕")
                            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                                // Marca para borrar en lugar de borrar
                                // directo: el modal de confirmación se
                                // renderea arriba en `render` y maneja
                                // confirm/cancel.
                                this.pending_delete =
                                    Some((entity_for_delete.clone(), id_copy));
                                this.toast = None;
                                cx.notify();
                            })),
                    ),
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

    /// Renderea el selector clickable de records existentes para un
    /// FieldSpec con kind=EntityRef. Lista compacta debajo del input;
    /// click en una opción setea el TextInput del field con el UUID
    /// seleccionado. El item del UUID actualmente seleccionado (si
    /// hay) se resalta con accent color.
    #[allow(clippy::too_many_arguments)]
    fn render_entity_ref_selector(
        &self,
        cx: &mut Context<Self>,
        field_name: String,
        target_entity: String,
        text: gpui::Rgba,
        text_dim: gpui::Rgba,
        accent: gpui::Rgba,
    ) -> gpui::Div {
        let _ = text;
        let rows = self.list_rows(&target_entity);
        let current = self
            .form_inputs
            .get(&field_name)
            .map(|inp| inp.read(&*cx).text().to_string())
            .unwrap_or_default();

        let mut container = div()
            .mt(px(4.))
            .pl(px(8.))
            .border_l_2()
            .border_color(gpui::rgb(0x2a2f38))
            .flex()
            .flex_col()
            .gap(px(2.));

        if rows.is_empty() {
            return container.child(
                div()
                    .px(px(6.))
                    .py(px(4.))
                    .text_color(text_dim)
                    .text_size(px(10.))
                    .child(format!(
                        "(sin {target_entity}: creá uno antes para referenciar)"
                    )),
            );
        }

        container = container.child(
            div()
                .text_color(text_dim)
                .text_size(px(10.))
                .child(format!("Seleccioná un {target_entity}:")),
        );

        for (id, value) in &rows {
            let label = human_label_for_record(value, id);
            let id_str = id.to_string();
            let is_selected = current == id_str;
            let field_for_click = field_name.clone();
            let id_for_click = id_str.clone();
            container = container.child(
                div()
                    .id(SharedString::from(format!(
                        "entity-ref-{field_name}-{id_str}"
                    )))
                    .px(px(6.))
                    .py(px(2.))
                    .text_size(px(11.))
                    .text_color(if is_selected { accent } else { text_dim })
                    .when(is_selected, |d| d.bg(gpui::rgb(0x232a36)))
                    .hover(|d| d.bg(gpui::rgb(0x1f2630)))
                    .child(label)
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        if let Some(input) = this.form_inputs.get(&field_for_click) {
                            input.update(cx, |inp, cx| inp.set_text(id_for_click.clone(), cx));
                        }
                        cx.notify();
                    })),
            );
        }
        container
    }

    #[allow(clippy::too_many_arguments)]
    fn render_form(
        &self,
        cx: &mut Context<Self>,
        mut main: gpui::Div,
        fv: &FormView,
        mod_idx: usize,
        _border: gpui::Rgba,
        text: gpui::Rgba,
        text_dim: gpui::Rgba,
        accent: gpui::Rgba,
    ) -> gpui::Div {
        // En modo edit, el título refleja eso para que el user no
        // se confunda creyendo que hace alta nueva.
        let title = match self.editing.as_ref() {
            Some((e, id)) if e == &fv.entity => {
                format!("Editar {} {}", fv.entity, short_uuid(id))
            }
            _ => fv.title.clone(),
        };
        main = main.child(
            div()
                .text_color(text)
                .text_size(px(18.))
                .mb(px(12.))
                .child(title),
        );
        for f in &fv.fields {
            let label = if f.required {
                format!("{} *", f.label)
            } else {
                f.label.clone()
            };

            let mut field_box = div().flex().flex_col().mb(px(10.)).child(
                div()
                    .text_color(text_dim)
                    .text_size(px(11.))
                    .mb(px(2.))
                    .child(label),
            );

            // Mount del TextInput vivo (creado en select_view).
            if let Some(input) = self.form_inputs.get(&f.name) {
                field_box = field_box.child(input.clone());
            } else {
                // No debería pasar — select_view crea inputs por cada
                // field. Fallback display estático por seguridad.
                field_box = field_box.child(
                    div()
                        .px(px(8.))
                        .py(px(6.))
                        .bg(gpui::rgb(0x171a20))
                        .text_color(text_dim)
                        .child("(input no inicializado)"),
                );
            }

            // Para EntityRef, agregamos un selector clickable de
            // records existentes debajo del TextInput. Click en una
            // opción setea el TextInput interno con el UUID; el
            // submit lee de ahí como cualquier otro field.
            if f.kind == FieldKind::EntityRef {
                if let Some(target_entity) = &f.ref_entity {
                    field_box = field_box.child(self.render_entity_ref_selector(
                        cx,
                        f.name.clone(),
                        target_entity.clone(),
                        text,
                        text_dim,
                        accent,
                    ));
                }
            }

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

        let editing_this = matches!(
            self.editing.as_ref(),
            Some((e, _)) if e == &fv.entity
        );
        let submit_label = match &fv.on_submit {
            Action::SeedEntity { entity, .. } => {
                if editing_this {
                    format!("Guardar cambios en {entity}")
                } else {
                    format!("Crear {entity}")
                }
            }
            Action::Morphism { name, .. } => format!("Ejecutar {name}"),
            Action::OpenView { label, view } => {
                label.clone().unwrap_or_else(|| format!("Ir a {view}"))
            }
        };
        let submit_action = fv.on_submit.clone();
        main = main.child(
            div().mt(px(12.)).child(
                div()
                    .id(SharedString::from(format!("form-submit-{mod_idx}")))
                    .px(px(12.))
                    .py(px(6.))
                    .bg(gpui::rgb(0x2c3540))
                    .text_color(accent)
                    .text_size(px(12.))
                    .rounded(px(3.))
                    .hover(|d| d.bg(gpui::rgb(0x3a4555)))
                    .child(submit_label)
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        this.apply_action(submit_action.clone(), cx);
                    })),
            ),
        );

        main = main.child(
            div()
                .mt(px(20.))
                .text_color(text_dim)
                .text_size(px(10.))
                .child(
                    "Tip: click en el campo para enfocar; Enter no envía (todavía), \
                     usá el botón. Backspace borra el último carácter.",
                ),
        );
        main
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helpers de persistencia movidos al backend en Fase 2b.
    use crate::backend::{maybe_compact_log, snapshot_path_for};
    // Tests E2E de nakui-core que vivieron históricamente en este
    // crate y siguen acá (no son duplicados con yahweh-meta-runtime).
    use nakui_core::event_log::{
        replay_with_snapshot_into, EventLog, LogEntry, Snapshot,
    };
    use nakui_core::store::{MemoryStore, Store};
    use serde_json::json;

    // NOTA: `parse_field_value` / `parse_field_*` viven y se testean
    // en `yahweh-meta-runtime`. Tests duplicados aquí se borraron en
    // la Fase 2 del refactor yahweh.

    #[test]
    fn lookup_field_simple_and_nested() {
        let v = json!({
            "name": "Acme",
            "address": { "city": "Bogotá", "country": "CO" }
        });
        assert_eq!(lookup_field(&v, "name").unwrap(), &json!("Acme"));
        assert_eq!(lookup_field(&v, "address.city").unwrap(), &json!("Bogotá"));
        assert!(lookup_field(&v, "missing").is_none());
        assert!(lookup_field(&v, "address.zipcode").is_none());
    }

    // `infer_param_value`, helpers `spec`/`map`, todos los tests
    // delta_* / clear_fields_* / parse_field_* / resolve_param_* /
    // human_label_* / render_value / value_to_input_text / validate_entity_refs_*
    // viven en `yahweh-meta-runtime`. Borrados en Fase 2 — quedan acá
    // sólo tests de funcionalidad runtime-específica (compact, snapshot,
    // event log, morphism pipeline, load_ui_modules).

    #[test]
    fn append_compact_msg_handles_both_branches() {
        // Sin compact: base solo, sin separador.
        let s = append_compact_msg("creado X".into(), None);
        assert_eq!(s.as_ref(), "creado X");
        // Con compact: concatena con "; ".
        let s = append_compact_msg(
            "creado X".into(),
            Some("auto-compact: snapshot @ seq 49".into()),
        );
        assert_eq!(s.as_ref(), "creado X; auto-compact: snapshot @ seq 49");
    }

    /// Simula el ciclo de write+tick que ocurriría en runtime: con
    /// threshold=N, los primeros N-1 writes no compactan, el N-ésimo
    /// dispara compact y resetea el counter, los siguientes vuelven
    /// a acumular.
    ///
    /// Como `tick_runtime_compact` es un método de `MetaUi` (necesita
    /// el cx GPUI para construir el state completo), reproducimos el
    /// algorithm por separado: counter manual + invocación directa de
    /// `maybe_compact_log`. Si la lógica del tick cambia, este test
    /// se va a romper como signal de que hay que actualizarlo.
    #[test]
    fn runtime_compact_cycle_resets_counter_after_threshold() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);
        let snap_path = snapshot_path_for(&path);
        let threshold: usize = 3;

        let mut store = MemoryStore::new();
        let mut log = EventLog::open(&path).unwrap();
        let mut counter: u64 = 0;
        let mut total_compactions = 0u32;

        // 7 writes con threshold=3 → 2 compacts (en write 3 y en
        // write 6), counter restante = 1 al final.
        for i in 0..7u64 {
            let id = Uuid::new_v4();
            log.append(LogEntry::Seed {
                seq: i,
                entity: "row".into(),
                id,
                data: json!({"i": i}),
                schema_hash: None,
            })
            .unwrap();
            store.seed("row", id, json!({"i": i}));

            // Tick.
            counter += 1;
            if counter >= threshold as u64 {
                let res =
                    maybe_compact_log(&mut log, &snap_path, &store, threshold).unwrap();
                if res.is_some() {
                    total_compactions += 1;
                }
                counter = 0;
            }
        }

        assert_eq!(
            total_compactions, 2,
            "con 7 writes y threshold 3 deberíamos disparar 2 compacts"
        );
        assert_eq!(counter, 1, "1 write residual sin compactar");

        // El log final debería tener: el anchor del último compact +
        // el write residual = 2 entries.
        let entries_after = EventLog::open(&path).unwrap().entries().unwrap().len();
        assert_eq!(
            entries_after, 2,
            "1 anchor del último compact + 1 write residual"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&snap_path);
    }

    /// E2E del nuevo `load_ui_modules` que pasa por
    /// `brahman_cards::load_cards_from_dir`. Verifica:
    /// 1. UiModules cargados ordenados por id.
    /// 2. Validación per-module se aplica (un module.json con
    ///    menu apuntando a una view inexistente debería fallar).
    /// 3. Cards de otros body kinds (Ente fixture) se reportan
    ///    en el `skipped` sin romper la carga.
    #[test]
    fn load_ui_modules_via_brahman_cards_returns_ui_modules_and_skips_others() {
        let root = tempfile::tempdir().unwrap();

        // Subdir A: UiModule válido.
        let a = root.path().join("alpha");
        std::fs::create_dir(&a).unwrap();
        std::fs::write(
            a.join("module.json"),
            serde_json::to_vec(&json!({
                "id": "alpha",
                "label": "Alpha",
                "entities": [],
                "menu": [],
                "views": {}
            }))
            .unwrap(),
        )
        .unwrap();

        // Subdir B: Ente card (no UiModule). Debe skipearse,
        // no romper la carga.
        let b = root.path().join("bravo");
        std::fs::create_dir(&b).unwrap();
        std::fs::write(
            b.join("card.json"),
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                "label": "ente-bravo",
                "payload": "Virtual",
                "supervision": "OneShot"
            }))
            .unwrap(),
        )
        .unwrap();

        let (modules, skipped) =
            load_ui_modules(root.path()).expect("load ok");
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].id, "alpha");
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].contains("ente"));
    }

    #[test]
    fn load_ui_modules_via_brahman_cards_rejects_invalid_module() {
        let root = tempfile::tempdir().unwrap();
        let sub = root.path().join("broken");
        std::fs::create_dir(&sub).unwrap();
        // menu apunta a una view que no existe en `views`.
        std::fs::write(
            sub.join("module.json"),
            serde_json::to_vec(&json!({
                "id": "broken",
                "label": "Broken",
                "entities": [],
                "menu": [{ "label": "Phantom", "view": "ghost" }],
                "views": {}
            }))
            .unwrap(),
        )
        .unwrap();
        let err = load_ui_modules(root.path()).unwrap_err();
        assert!(err.contains("broken"), "msg debe nombrar el módulo: {err}");
    }

    #[test]
    fn load_ui_modules_detects_duplicate_id() {
        let root = tempfile::tempdir().unwrap();
        for name in ["dir_a", "dir_b"] {
            let sub = root.path().join(name);
            std::fs::create_dir(&sub).unwrap();
            std::fs::write(
                sub.join("module.json"),
                serde_json::to_vec(&json!({
                    "id": "dup",
                    "label": "Dup",
                    "entities": [], "menu": [], "views": {}
                }))
                .unwrap(),
            )
            .unwrap();
        }
        let err = load_ui_modules(root.path()).unwrap_err();
        assert!(err.contains("duplicado"), "msg debe decir duplicado: {err}");
        assert!(err.contains("dup"), "msg debe nombrar el id: {err}");
    }

    #[test]
    fn snapshot_path_for_replaces_extension() {
        use std::path::Path;
        assert_eq!(
            snapshot_path_for(Path::new("nakui-ui-state.jsonl")),
            std::path::PathBuf::from("nakui-ui-state.snap.json"),
        );
        // Sin extensión: agrega .snap.json.
        assert_eq!(
            snapshot_path_for(Path::new("/tmp/foo")),
            std::path::PathBuf::from("/tmp/foo.snap.json"),
        );
    }

    #[test]
    fn maybe_compact_log_below_threshold_noops() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);
        let snap_path = snapshot_path_for(&path);
        let mut log = EventLog::open(&path).unwrap();
        for i in 0..5 {
            log.append(LogEntry::Seed {
                seq: i,
                entity: "x".into(),
                id: Uuid::new_v4(),
                data: json!({"i": i}),
                schema_hash: None,
            })
            .unwrap();
        }
        let store = MemoryStore::new();
        let res = maybe_compact_log(&mut log, &snap_path, &store, 50).unwrap();
        assert!(res.is_none(), "5 < 50 → no debería compactar");
        assert_eq!(log.entries().unwrap().len(), 5, "log intacto");
        assert!(!snap_path.exists(), "no debería haber escrito snapshot");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn maybe_compact_log_threshold_zero_noops() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);
        let snap_path = snapshot_path_for(&path);
        let mut log = EventLog::open(&path).unwrap();
        for i in 0..3 {
            log.append(LogEntry::Seed {
                seq: i,
                entity: "x".into(),
                id: Uuid::new_v4(),
                data: json!({"i": i}),
                schema_hash: None,
            })
            .unwrap();
        }
        let store = MemoryStore::new();
        let res = maybe_compact_log(&mut log, &snap_path, &store, 0).unwrap();
        assert!(res.is_none(), "threshold 0 = disabled");
        let _ = std::fs::remove_file(&path);
    }

    /// E2E del ciclo completo: write log con N entries por encima del
    /// threshold → maybe_compact_log captura snapshot y trunca el log
    /// → re-open + replay con snapshot → store final == store que
    /// resultaría de full replay sin snapshot.
    #[test]
    fn maybe_compact_log_then_reopen_preserves_records() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);
        let snap_path = snapshot_path_for(&path);

        // 1. Escribir 60 seeds + popular store en sync.
        let mut store = MemoryStore::new();
        let mut ids = Vec::new();
        let mut log = EventLog::open(&path).unwrap();
        for i in 0..60u64 {
            let id = Uuid::new_v4();
            log.append(LogEntry::Seed {
                seq: i,
                entity: "row".into(),
                id,
                data: json!({"i": i}),
                schema_hash: None,
            })
            .unwrap();
            store.seed("row", id, json!({"i": i}));
            ids.push(id);
        }
        assert_eq!(log.next_seq(), 60);
        assert_eq!(log.entries().unwrap().len(), 60);

        // 2. Compactar (60 >= 50).
        let res = maybe_compact_log(&mut log, &snap_path, &store, 50).unwrap();
        assert!(res.is_some(), "60 >= 50 debe compactar");
        let msg = res.unwrap();
        assert!(msg.contains("seq 59"), "msg debe incluir el seq: {msg}");
        assert!(
            msg.contains("59 entries dropped"),
            "msg debe reportar 59 dropped (60 - 1 anchor): {msg}"
        );

        // 3. Verificar: snapshot existe, log queda con 1 entry
        //    (anchor del cursor), next_seq se preserva (60).
        assert!(snap_path.exists(), "snapshot debería existir");
        let log_after = EventLog::open(&path).unwrap();
        assert_eq!(
            log_after.entries().unwrap().len(),
            1,
            "log debería tener 1 anchor entry tras compact"
        );
        assert_eq!(
            log_after.next_seq(),
            60,
            "next_seq se preserva via anchor entry"
        );

        // 4. Re-open + replay desde snapshot → todos los records.
        //    El anchor entry cae bajo snap.seq así que se skipea.
        let snap = Snapshot::load(&snap_path).unwrap().expect("snap loadeable");
        assert_eq!(snap.seq, 59);
        let mut fresh_store = MemoryStore::new();
        replay_with_snapshot_into(&log_after, Some(&snap), &mut fresh_store).unwrap();
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(
                fresh_store.load("row", *id),
                Some(json!({"i": i as u64})),
                "record {i} debería estar tras snapshot+replay"
            );
        }

        // 5. Idempotencia: segunda corrida del compact con threshold=1
        //    no hace nada — queda 1 anchor entry, no hay nada útil
        //    que dropear.
        let mut log_reopened = EventLog::open(&path).unwrap();
        let res2 =
            maybe_compact_log(&mut log_reopened, &snap_path, &fresh_store, 1).unwrap();
        assert!(
            res2.is_none(),
            "post-compact: 1 anchor entry, segundo compact debe ser no-op"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&snap_path);
    }

    #[test]
    fn value_to_input_then_parse_round_trip() {
        // El round-trip es la propiedad fundamental: edit → text →
        // parse → mismo Value (modulo casts numéricos).
        let cases = vec![
            (FieldKind::Text, json!("hola")),
            (FieldKind::Boolean, json!(true)),
            (FieldKind::Boolean, json!(false)),
            (FieldKind::Number, json!(42)),
        ];
        for (kind, original) in cases {
            let text = value_to_input_text(&original);
            let parsed = parse_field_value(kind, &text).unwrap();
            assert_eq!(
                parsed, original,
                "round-trip text→parse falló para {original:?}"
            );
        }
    }

    /// E2E mínimo del WAL: armamos un log a mano con dos seeds,
    /// abrimos con `EventLog::open` + `replay_into`, y verificamos
    /// que el `MemoryStore` queda con esos records aplicados.
    /// Esto reproduce el flujo del startup de `MetaUi::new` sin
    /// necesitar GPUI.
    #[test]
    fn event_log_replay_restores_memory_store() {
        use nakui_core::event_log::{replay_into, EventLog, LogEntry};
        use nakui_core::store::{MemoryStore, Store};

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        // Cerramos el handle de tempfile pero conservamos el path
        // para que EventLog pueda re-abrir.
        drop(tmp);

        // Escribimos dos seeds via EventLog::append.
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        {
            let mut log = EventLog::open(&path).unwrap();
            log.append(LogEntry::Seed {
                seq: 0,
                entity: "customer".into(),
                id: id_a,
                data: json!({"name": "Acme"}),
                schema_hash: None,
            })
            .unwrap();
            log.append(LogEntry::Seed {
                seq: 1,
                entity: "customer".into(),
                id: id_b,
                data: json!({"name": "Globex"}),
                schema_hash: None,
            })
            .unwrap();
        }

        // Re-abrir + replay (simula startup de MetaUi).
        let log = EventLog::open(&path).unwrap();
        assert_eq!(log.next_seq(), 2, "next_seq debe ser 2 tras 2 entries");
        let mut store = MemoryStore::new();
        replay_into(&log, &mut store).unwrap();

        // Verificar que ambos records están en el store.
        assert_eq!(
            store.load("customer", id_a),
            Some(json!({"name": "Acme"})),
            "Acme debería estar tras replay"
        );
        assert_eq!(
            store.load("customer", id_b),
            Some(json!({"name": "Globex"})),
            "Globex debería estar tras replay"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// E2E del Action::Morphism: carga el módulo nakui-core real
    /// `sales` (que vive en `crates/modules/nakui/modules/sales`),
    /// arma store + log, y ejecuta el morphism `vender` vía
    /// `execute_and_log_with_recovery` (la misma función que
    /// `commit_morphism` invoca). Verifica que las ops esperadas
    /// se loguean y aplican (stock decrementa, caja incrementa).
    ///
    /// Reproduce el flujo del runtime sin necesitar GPUI.
    #[test]
    fn morphism_pipeline_executes_real_sales_vender() {
        use nakui_core::event_log::{execute_and_log_with_recovery, EventLog};
        use nakui_core::executor::Executor;
        use nakui_core::store::{MemoryStore, Store};

        // Path al módulo real (3 dirs arriba: crates/apps/nakui-ui/
        // → crates/modules/nakui/modules/sales).
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let sales_dir = here
            .join("../../..")
            .join("crates/modules/nakui/modules/sales");
        if !sales_dir.join("nsmc.json").exists() {
            // Si el módulo demo no está donde esperamos, skipeamos
            // — no es regresión del feature, es ambiente.
            eprintln!(
                "skip: sales module no encontrado en {}",
                sales_dir.display()
            );
            return;
        }

        let executor = Executor::load_module(&sales_dir).expect("cargar sales executor");

        let mut store = MemoryStore::new();
        let stock_id = Uuid::new_v4();
        let caja_id = Uuid::new_v4();
        store.seed(
            "Stock",
            stock_id,
            json!({
                "id": stock_id.to_string(),
                "sku_id": "test-sku",
                "ubicacion": "loc-1",
                "cantidad": 100_i64,
            }),
        );
        store.seed(
            "Caja",
            caja_id,
            json!({
                "id": caja_id.to_string(),
                "name": "Caja Test",
                "currency": "USD",
                "saldo": 1_000_000_i64,
            }),
        );

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let log_path = tmp.path().to_path_buf();
        drop(tmp);
        let mut log = EventLog::open(&log_path).unwrap();

        let venta_id = Uuid::new_v4();
        let inputs = vec![("stock", stock_id), ("caja", caja_id)];
        let params = json!({
            "venta_id": venta_id.to_string(),
            "cantidad": 5_i64,
            "precio_unitario": 200_i64,
            "timestamp": "2026-05-04T10:00:00Z",
        });

        let ops = execute_and_log_with_recovery(
            &executor,
            &mut store,
            &mut log,
            "vender",
            &inputs,
            params,
        )
        .expect("morphism vender debe ejecutar limpio");

        assert!(!ops.is_empty(), "vender debería producir ops");

        // Sanity post-condiciones esperadas del manifest sales:
        // - stock.cantidad bajó (vendimos 5).
        let stock_after = store
            .load("Stock", stock_id)
            .and_then(|v| v.get("cantidad").and_then(Value::as_i64))
            .expect("stock con cantidad");
        assert_eq!(stock_after, 95, "stock debería bajar de 100 a 95");
        // - caja.saldo subió (cobramos 5*200 = 1000 sobre saldo
        // inicial 1_000_000).
        let caja_after = store
            .load("Caja", caja_id)
            .and_then(|v| v.get("saldo").and_then(Value::as_i64))
            .expect("caja con saldo");
        assert_eq!(caja_after, 1_001_000, "caja debería subir 5*200=1000");

        let _ = std::fs::remove_file(&log_path);
    }

    /// E2E del ciclo CRUD vía log:
    /// 1. Seed un record.
    /// 2. Morphism con Set ops (edit) — sobreescribe campos.
    /// 3. Morphism con Delete op — borra el record.
    /// 4. Replay desde cero: el store queda como tras el delete (vacío).
    #[test]
    fn event_log_replay_handles_full_crud_cycle() {
        use nakui_core::delta::{FieldOp, FieldPath};
        use nakui_core::event_log::{replay_into, EventLog, LogEntry};
        use nakui_core::store::{MemoryStore, Store};

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        let id = Uuid::new_v4();

        // 1. Escribir 3 entries: seed, edit, delete.
        {
            let mut log = EventLog::open(&path).unwrap();
            log.append(LogEntry::Seed {
                seq: 0,
                entity: "customer".into(),
                id,
                data: json!({"name": "Acme", "active": true}),
                schema_hash: None,
            })
            .unwrap();
            log.append(LogEntry::Morphism {
                seq: 1,
                morphism: "ui.edit_record".into(),
                inputs: Default::default(),
                params: json!({}),
                ops: vec![
                    FieldOp::Set {
                        path: FieldPath {
                            entity: "customer".into(),
                            id,
                            field: "name".into(),
                        },
                        value: json!("Acme S.A."),
                    },
                    FieldOp::Set {
                        path: FieldPath {
                            entity: "customer".into(),
                            id,
                            field: "active".into(),
                        },
                        value: json!(false),
                    },
                ],
                schema_hash: None,
            })
            .unwrap();
            log.append(LogEntry::Morphism {
                seq: 2,
                morphism: "ui.delete_record".into(),
                inputs: Default::default(),
                params: json!({}),
                ops: vec![FieldOp::Delete {
                    entity: "customer".into(),
                    id,
                }],
                schema_hash: None,
            })
            .unwrap();
        }

        // 2. Replay desde cero — debe terminar con store vacío
        // (el delete fue el último op).
        let log = EventLog::open(&path).unwrap();
        let mut store = MemoryStore::new();
        replay_into(&log, &mut store).unwrap();
        assert_eq!(
            store.load("customer", id),
            None,
            "tras seed + edit + delete, el record no debería existir"
        );

        // 3. Sanity: si paramos en seq=1 (snapshot post-edit), el
        // record debería tener los valores editados.
        // (Construimos un store fresh y aplicamos sólo seq 0 y 1
        // a mano para verificar.)
        let mut store_partial = MemoryStore::new();
        store_partial.seed("customer", id, json!({"name": "Acme", "active": true}));
        store_partial
            .apply(&[
                FieldOp::Set {
                    path: FieldPath {
                        entity: "customer".into(),
                        id,
                        field: "name".into(),
                    },
                    value: json!("Acme S.A."),
                },
                FieldOp::Set {
                    path: FieldPath {
                        entity: "customer".into(),
                        id,
                        field: "active".into(),
                    },
                    value: json!(false),
                },
            ])
            .unwrap();
        assert_eq!(
            store_partial.load("customer", id),
            Some(json!({"name": "Acme S.A.", "active": false})),
            "tras seed + edit, el record debería tener los nuevos valores"
        );

        let _ = std::fs::remove_file(&path);
    }
}
