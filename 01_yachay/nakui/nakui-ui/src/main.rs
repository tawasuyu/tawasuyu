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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gpui::{
    div, prelude::*, px, App, Application, Bounds, ClickEvent, Context, Entity, IntoElement,
    Render, SharedString, Window, WindowBounds, WindowOptions,
};
use nakui_core::delta::{FieldOp, FieldPath};
use nakui_core::event_log::{execute_and_log_with_recovery, replay_into, EventLog, LogEntry};
use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store};
use nakui_ui_schema::{
    Action, FieldKind, FieldSpec, FormView, ListView, Module, View,
};
use serde_json::{json, Value};
use uuid::Uuid;
use yahweh_theme::Theme;
use yahweh_widget_text_input::TextInput;

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

/// Estado del runtime.
struct MetaUi {
    /// Módulos cargados, ordenados por id.
    modules: Vec<Module>,
    /// Store compartido. Mutado por el submit de los forms.
    store: Arc<Mutex<MemoryStore>>,
    /// Event log persistente compartido. Cada `seed_entity` se appende
    /// acá antes de mutar el store (WAL). `None` si la apertura del
    /// log falló — en ese caso el runtime degrada a in-memory only y
    /// loggea un toast informativo.
    event_log: Option<Arc<Mutex<EventLog>>>,
    /// Executors nakui cargados, indexados por `module.id`. Sólo
    /// existen los módulos que declaran `nakui_module_dir`. Las
    /// acciones `Morphism` requieren que el módulo activo tenga
    /// uno; sin él, despachan un toast informativo.
    executors: BTreeMap<String, Arc<Executor>>,
    /// (módulo idx, vista key) actualmente activos.
    active: Option<(usize, String)>,
    /// Inputs vivos para el form actual: nombre del campo → TextInput.
    /// Se reemplaza al cambiar de vista (drop de los anteriores).
    form_inputs: BTreeMap<String, Entity<TextInput>>,
    /// Si está set, el próximo render del Form pre-llena los inputs
    /// con los valores del record indicado, y `commit_seed` emite
    /// un `LogEntry::Morphism { name: "ui.edit_record", ops: [Set...] }`
    /// en lugar de un Seed nuevo. Limpia al cambiar de view o tras
    /// submit exitoso.
    editing: Option<(String, Uuid)>,
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

        let (modules, mut load_error) =
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

        // Persistencia: abrir/crear el event log y hacer replay al
        // store. Path por env `NAKUI_EVENT_LOG`, default
        // `./nakui-ui-state.jsonl`. Si abrir o replay falla, el
        // runtime sigue en modo in-memory (sin persistencia) y el
        // load_error se acumula al banner.
        let log_path = std::env::var("NAKUI_EVENT_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("nakui-ui-state.jsonl"));
        let mut store = MemoryStore::new();
        let mut initial_toast: Option<SharedString> = None;
        let event_log = match EventLog::open(&log_path) {
            Ok(log) => {
                match replay_into(&log, &mut store) {
                    Ok(()) => {
                        let n = log.next_seq();
                        if n > 0 {
                            initial_toast = Some(SharedString::from(format!(
                                "log {} cargado: {n} evento(s) replayed",
                                log_path.display()
                            )));
                        } else {
                            initial_toast = Some(SharedString::from(format!(
                                "log nuevo en {}",
                                log_path.display()
                            )));
                        }
                        Some(Arc::new(Mutex::new(log)))
                    }
                    Err(e) => {
                        let msg = format!(
                            "replay del log {} falló: {e} — running in-memory",
                            log_path.display()
                        );
                        match &load_error {
                            Some(prev) => {
                                load_error = Some(SharedString::from(format!("{prev}; {msg}")));
                            }
                            None => load_error = Some(SharedString::from(msg)),
                        }
                        None
                    }
                }
            }
            Err(e) => {
                let msg = format!(
                    "abrir log {}: {e} — running in-memory only",
                    log_path.display()
                );
                match &load_error {
                    Some(prev) => {
                        load_error = Some(SharedString::from(format!("{prev}; {msg}")));
                    }
                    None => load_error = Some(SharedString::from(msg)),
                }
                None
            }
        };

        // Cargar Executors para los módulos que declararon
        // `nakui_module_dir`. Resolvemos paths relativos al
        // directorio del modules (NAKUI_MODULES_DIR/<id>/), no al
        // pwd. Cualquier error de carga deja la entry afuera y
        // anota al banner — Action::Morphism queda no-op para ese
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

        let active = modules
            .first()
            .and_then(|m| m.menu.first().map(|item| (0usize, item.view.clone())));

        Self {
            modules,
            store: Arc::new(Mutex::new(store)),
            event_log,
            executors,
            active,
            form_inputs: BTreeMap::new(),
            editing: None,
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
        self.form_inputs = BTreeMap::new();
        if let Some(module) = self.modules.get(mod_idx) {
            if let Some(View::Form(form)) = module.views.get(&view_key) {
                // Snapshot del record si estamos editando esta entity.
                let editing_record: Option<Value> = self.editing.as_ref().and_then(|(e, id)| {
                    if e == &form.entity {
                        let store = self.store.lock().ok()?;
                        store.load(e, *id)
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
    fn commit_delete(
        &mut self,
        entity: &str,
        id: Uuid,
    ) -> Result<(), String> {
        let ops = vec![FieldOp::Delete {
            entity: entity.to_string(),
            id,
        }];
        if let Some(log_arc) = self.event_log.as_ref() {
            let mut log = log_arc
                .lock()
                .map_err(|_| "log mutex envenenado".to_string())?;
            let seq = log.next_seq();
            log.append(LogEntry::Morphism {
                seq,
                morphism: "ui.delete_record".into(),
                inputs: Default::default(),
                params: json!({ "entity": entity, "id": id.to_string() }),
                ops: ops.clone(),
                schema_hash: None,
            })
            .map_err(|e| format!("append al log: {e}"))?;
        }
        let mut store = self
            .store
            .lock()
            .map_err(|_| "store mutex envenenado".to_string())?;
        store
            .apply(&ops)
            .map_err(|e| format!("apply Delete: {e}"))?;
        Ok(())
    }

    /// Aplica una acción (click en menú, botón de form, action de
    /// list). Mutaciones contra el store ocurren acá.
    fn apply_action(&mut self, action: Action, cx: &mut Context<Self>) {
        let mod_idx = match self.active.as_ref() {
            Some((i, _)) => *i,
            None => return,
        };
        // Snapshot del editing al entrar — si commit_seed modifica
        // self.editing antes del toast, el mensaje refleja el modo
        // correcto.
        let was_editing = self.editing.is_some();
        match action {
            Action::OpenView { view, .. } => {
                // Salir a otra view cancela el edit pendiente.
                self.editing = None;
                self.select_view(mod_idx, view, cx);
            }
            Action::SeedEntity { entity, next_view } => {
                match self.commit_seed(mod_idx, &entity, cx) {
                    Ok(id) => {
                        let action_label = if was_editing { "actualizado" } else { "creado" };
                        self.toast = Some(SharedString::from(format!(
                            "{action_label} {entity} {}",
                            short_uuid(&id)
                        )));
                        // Limpia editing tras un commit exitoso —
                        // el record ya está sincronizado.
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
                    Ok(op_count) => {
                        self.toast = Some(SharedString::from(format!(
                            "morphism '{name}' OK ({op_count} op(s) aplicadas)"
                        )));
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

    /// Despacha un morphism al pipeline real de nakui-core: lee
    /// inputs (UUIDs) + params (Value object) del form, llama
    /// `execute_and_log_with_recovery`. Devuelve la cantidad de ops
    /// que el morphism produjo (para feedback).
    fn commit_morphism(
        &mut self,
        mod_idx: usize,
        morphism: &str,
        inputs_map: &BTreeMap<String, String>,
        params_fields: &[String],
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        let _ = cx;
        let module = self
            .modules
            .get(mod_idx)
            .ok_or_else(|| "módulo inválido".to_string())?;
        let executor = self
            .executors
            .get(&module.id)
            .ok_or_else(|| {
                format!(
                    "módulo '{}' no tiene executor nakui (falta nakui_module_dir o falló la carga)",
                    module.id
                )
            })?
            .clone();
        let log_arc = self
            .event_log
            .as_ref()
            .ok_or_else(|| "morphism requiere event log activo".to_string())?
            .clone();

        // Resolver inputs: por cada (role, field_name), parsear el
        // value del input como Uuid.
        let mut input_pairs: Vec<(String, Uuid)> = Vec::with_capacity(inputs_map.len());
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
            input_pairs.push((role.clone(), id));
        }

        // Resolver params: si la lista está vacía, todos los fields
        // del form que no estén en `inputs_map` van a params. Si
        // hay lista, sólo esos.
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

        // Buscamos los FieldSpec del Form view activo para conocer
        // el `kind` declarado de cada param. Usamos `parse_field_value`
        // estricto en lugar de la heurística `infer_param_value` —
        // así un "abc" en un campo Boolean rebota en la UI con un
        // mensaje claro ANTES de llegar al morphism Rhai.
        let active_form_fields: Option<Vec<FieldSpec>> = self.active.as_ref().and_then(|(_, vk)| {
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

        let inputs_ref: Vec<(&str, Uuid)> = input_pairs
            .iter()
            .map(|(r, id)| (r.as_str(), *id))
            .collect();

        let mut log = log_arc
            .lock()
            .map_err(|_| "log mutex envenenado".to_string())?;
        let mut store = self
            .store
            .lock()
            .map_err(|_| "store mutex envenenado".to_string())?;

        let ops = execute_and_log_with_recovery(
            &executor,
            &mut *store,
            &mut *log,
            morphism,
            &inputs_ref,
            Value::Object(params_obj),
        )
        .map_err(|e| format!("{e}"))?;
        Ok(ops.len())
    }

    /// Construye un Value desde los TextInput vivos y lo seedea al store.
    fn commit_seed(
        &mut self,
        mod_idx: usize,
        entity: &str,
        cx: &mut Context<Self>,
    ) -> Result<Uuid, String> {
        let module = &self.modules[mod_idx];
        let spec_fields: Vec<FieldSpec> = match self.active.as_ref() {
            Some((_, view_key)) => match module.views.get(view_key) {
                Some(View::Form(f)) => f.fields.clone(),
                _ => return Err("la vista activa no es un Form".into()),
            },
            None => return Err("ninguna vista activa".into()),
        };
        let mut obj = serde_json::Map::new();
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
                continue;
            }
            let value = parse_field_value(f.kind, &raw)
                .map_err(|e| format!("campo '{}': {e}", f.label))?;
            obj.insert(f.name.clone(), value);
        }
        // Ramificación: si `editing` está set para esta entity, es un
        // edit de un record existente — emitimos Morphism con un
        // FieldOp::Set por cada campo del form (sobreescribe). Si no,
        // es alta nueva — emitimos Seed con UUID fresco.
        let editing_match = self.editing.as_ref().filter(|(e, _)| e == entity).cloned();

        if let Some((_, id)) = editing_match {
            // EDIT path: Morphism { ui.edit_record, ops: [Set...] }
            let ops: Vec<FieldOp> = obj
                .iter()
                .map(|(field, value)| FieldOp::Set {
                    path: FieldPath {
                        entity: entity.to_string(),
                        id,
                        field: field.clone(),
                    },
                    value: value.clone(),
                })
                .collect();

            if let Some(log_arc) = self.event_log.as_ref() {
                let mut log = log_arc
                    .lock()
                    .map_err(|_| "log mutex envenenado".to_string())?;
                let seq = log.next_seq();
                log.append(LogEntry::Morphism {
                    seq,
                    morphism: "ui.edit_record".into(),
                    inputs: Default::default(),
                    params: json!({
                        "entity": entity,
                        "id": id.to_string(),
                        "fields": Value::Object(obj.clone()),
                    }),
                    ops: ops.clone(),
                    schema_hash: None,
                })
                .map_err(|e| format!("append al log: {e}"))?;
            }
            let mut store = self
                .store
                .lock()
                .map_err(|_| "store mutex envenenado".to_string())?;
            store.apply(&ops).map_err(|e| format!("apply Set: {e}"))?;
            Ok(id)
        } else {
            // SEED path: alta nueva.
            let id = Uuid::new_v4();
            let data = Value::Object(obj);
            if let Some(log_arc) = self.event_log.as_ref() {
                let mut log = log_arc
                    .lock()
                    .map_err(|_| "log mutex envenenado".to_string())?;
                let seq = log.next_seq();
                log.append(LogEntry::Seed {
                    seq,
                    entity: entity.to_string(),
                    id,
                    data: data.clone(),
                    schema_hash: None,
                })
                .map_err(|e| format!("append al log: {e}"))?;
            }
            let mut store = self
                .store
                .lock()
                .map_err(|_| "store mutex envenenado".to_string())?;
            store.seed(entity, id, data);
            Ok(id)
        }
    }

    /// Snapshot ordenado de records de una entity.
    fn list_rows(&self, entity: &str) -> Vec<(Uuid, Value)> {
        let store = match self.store.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let it = match store.iter() {
            Ok(i) => i,
            Err(_) => return Vec::new(),
        };
        it.filter(|(e, _, _)| e == entity)
            .map(|(_, id, v)| (id, v))
            .collect()
    }
}

fn parse_field_value(kind: FieldKind, raw: &str) -> Result<Value, String> {
    match kind {
        FieldKind::Text | FieldKind::Multiline | FieldKind::Date => Ok(json!(raw)),
        // EntityRef se almacena como string del UUID seleccionado.
        // El commit_morphism luego lo parsea como Uuid para inputs;
        // en seed_entity normal queda como string en el record.
        FieldKind::EntityRef => Ok(json!(raw)),
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

/// Etiqueta humana para representar un record en el selector de
/// EntityRef. Heurística: prefiere campos comunes en este orden:
/// `name`, `label`, `title`, `sku`, `sku_id`. Fallback al UUID corto.
fn human_label_for_record(value: &Value, id: &Uuid) -> String {
    for key in ["name", "label", "title", "sku", "sku_id"] {
        if let Some(v) = value.get(key).and_then(Value::as_str) {
            if !v.is_empty() {
                return format!("{} ({})", v, short_uuid(id));
            }
        }
    }
    short_uuid(id)
}

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

/// Resuelve un param de morphism a su `Value` según el `FieldSpec`
/// del form. **Strict path**: si hay spec, valida `required` y parsea
/// con el `kind` declarado (ej. Boolean rebota con "abc" antes de
/// llegar al morphism). **Fallback path**: si no hay spec (param
/// declarado en `Action::Morphism.params` que no aparece en
/// `form.fields`), usa la heurística `infer_param_value` para no
/// quedar atado a un schema mal-formado.
///
/// Errores tienen el label legible del spec, así el toast de la UI
/// es interpretable.
fn resolve_param_value(
    field_name: &str,
    raw: &str,
    spec: Option<&FieldSpec>,
) -> Result<Value, String> {
    let Some(s) = spec else {
        return Ok(infer_param_value(raw));
    };

    let label = if s.label.is_empty() { field_name } else { &s.label };

    if s.required && raw.trim().is_empty() {
        return Err(format!("param '{label}' es obligatorio y está vacío"));
    }
    if raw.is_empty() && !s.required {
        return Ok(Value::Null);
    }
    parse_field_value(s.kind, raw).map_err(|e| format!("param '{label}': {e}"))
}

/// Inferencia de tipo para values pasados como `params` a un
/// morphism. Usada como fallback en `resolve_param_value` cuando el
/// param declarado en `Action::Morphism.params` no aparece en los
/// `form.fields` (módulo mal-formado).
///
/// Heurística simple: int → i64, float → f64, "true"/"false" → bool,
/// resto → string.
fn infer_param_value(raw: &str) -> Value {
    if raw.is_empty() {
        return Value::Null;
    }
    if let Ok(i) = raw.parse::<i64>() {
        return json!(i);
    }
    if let Ok(f) = raw.parse::<f64>() {
        return json!(f);
    }
    match raw {
        "true" => return json!(true),
        "false" => return json!(false),
        _ => {}
    }
    json!(raw)
}

/// Conversión inversa a `parse_field_value`: del JSON al texto raw
/// que un input puede tomar y volver a parsearse igual al submit.
/// Usado para pre-llenar inputs en modo edit.
fn value_to_input_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn short_uuid(id: &Uuid) -> String {
    id.to_string().chars().take(8).collect()
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
                                match this.commit_delete(&entity_for_delete, id_copy) {
                                    Ok(()) => {
                                        this.toast = Some(SharedString::from(format!(
                                            "borrado {entity_for_delete} {}",
                                            short_uuid(&id_copy)
                                        )));
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

    #[test]
    fn parse_field_text_returns_string() {
        assert_eq!(parse_field_value(FieldKind::Text, "hola").unwrap(), json!("hola"));
    }

    #[test]
    fn parse_field_number_int_then_float() {
        assert_eq!(parse_field_value(FieldKind::Number, "42").unwrap(), json!(42));
        assert_eq!(parse_field_value(FieldKind::Number, "3.14").unwrap(), json!(3.14));
    }

    #[test]
    fn parse_field_number_invalid_errors() {
        assert!(parse_field_value(FieldKind::Number, "not-a-number").is_err());
    }

    #[test]
    fn parse_field_boolean_variants() {
        assert_eq!(parse_field_value(FieldKind::Boolean, "true").unwrap(), json!(true));
        assert_eq!(parse_field_value(FieldKind::Boolean, "yes").unwrap(), json!(true));
        assert_eq!(parse_field_value(FieldKind::Boolean, "false").unwrap(), json!(false));
        assert_eq!(parse_field_value(FieldKind::Boolean, "").unwrap(), json!(false));
        assert!(parse_field_value(FieldKind::Boolean, "maybe").is_err());
    }

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

    #[test]
    fn infer_param_value_int_then_float_then_bool_then_string() {
        assert_eq!(infer_param_value(""), json!(null));
        assert_eq!(infer_param_value("42"), json!(42));
        assert_eq!(infer_param_value("3.14"), json!(3.14));
        assert_eq!(infer_param_value("true"), json!(true));
        assert_eq!(infer_param_value("false"), json!(false));
        assert_eq!(infer_param_value("hola"), json!("hola"));
    }

    fn spec(name: &str, kind: FieldKind, required: bool) -> FieldSpec {
        FieldSpec {
            name: name.into(),
            label: name.into(),
            kind,
            default: None,
            required,
            help: None,
            ref_entity: None,
        }
    }

    #[test]
    fn resolve_param_strict_number_parses_i64() {
        let s = spec("qty", FieldKind::Number, true);
        let v = resolve_param_value("qty", "42", Some(&s)).unwrap();
        assert_eq!(v, json!(42));
    }

    #[test]
    fn resolve_param_strict_boolean_rejects_non_boolean() {
        let s = spec("active", FieldKind::Boolean, true);
        let err = resolve_param_value("active", "abc", Some(&s)).unwrap_err();
        assert!(err.contains("active"), "msg debe mencionar el label: {err}");
        assert!(
            err.to_lowercase().contains("bool") || err.contains("'abc'"),
            "msg debe explicar el tipo o value: {err}"
        );
    }

    #[test]
    fn resolve_param_strict_number_rejects_garbage() {
        let s = spec("qty", FieldKind::Number, true);
        let err = resolve_param_value("qty", "abc", Some(&s)).unwrap_err();
        assert!(err.contains("qty"), "msg debe mencionar el label: {err}");
    }

    #[test]
    fn resolve_param_required_empty_rejected() {
        let s = spec("name", FieldKind::Text, true);
        let err = resolve_param_value("name", "   ", Some(&s)).unwrap_err();
        assert!(
            err.contains("obligatorio"),
            "msg debe decir obligatorio: {err}"
        );
    }

    #[test]
    fn resolve_param_optional_empty_returns_null() {
        let s = spec("notes", FieldKind::Text, false);
        let v = resolve_param_value("notes", "", Some(&s)).unwrap();
        assert_eq!(v, json!(null));
    }

    #[test]
    fn resolve_param_no_spec_falls_back_to_infer() {
        // Sin FieldSpec (módulo mal-formado): infer_param_value
        // se usa como red de seguridad.
        let v = resolve_param_value("foo", "42", None).unwrap();
        assert_eq!(v, json!(42));
        let v = resolve_param_value("foo", "true", None).unwrap();
        assert_eq!(v, json!(true));
        let v = resolve_param_value("foo", "x", None).unwrap();
        assert_eq!(v, json!("x"));
    }

    #[test]
    fn parse_field_entity_ref_returns_string() {
        // EntityRef se almacena como string del UUID. parse_field_value
        // no lo valida como UUID — eso lo hace commit_morphism al
        // resolver inputs.
        let v = parse_field_value(FieldKind::EntityRef, "abc-123").unwrap();
        assert_eq!(v, json!("abc-123"));
    }

    #[test]
    fn human_label_for_record_prefers_name_over_id() {
        let id = Uuid::new_v4();
        let with_name = json!({"name": "Acme S.A.", "email": "x@y.z"});
        let label = human_label_for_record(&with_name, &id);
        assert!(label.starts_with("Acme S.A."), "got: {label}");
        assert!(label.contains(&short_uuid(&id)));
    }

    #[test]
    fn human_label_falls_back_through_label_title_sku() {
        let id = Uuid::new_v4();
        let only_label = json!({"label": "X"});
        assert!(human_label_for_record(&only_label, &id).starts_with("X "));
        let only_title = json!({"title": "Y"});
        assert!(human_label_for_record(&only_title, &id).starts_with("Y "));
        let only_sku = json!({"sku": "Z-001"});
        assert!(human_label_for_record(&only_sku, &id).starts_with("Z-001 "));
        let only_sku_id = json!({"sku_id": "W-002"});
        assert!(human_label_for_record(&only_sku_id, &id).starts_with("W-002 "));
    }

    #[test]
    fn human_label_falls_back_to_id_when_no_known_keys() {
        let id = Uuid::new_v4();
        let v = json!({"weird_field": "val"});
        assert_eq!(human_label_for_record(&v, &id), short_uuid(&id));
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

    #[test]
    fn value_to_input_text_inverse_of_parse() {
        // text → text
        assert_eq!(value_to_input_text(&json!("hola")), "hola");
        // bool → "true"/"false" (parse_field_value lo acepta)
        assert_eq!(value_to_input_text(&json!(true)), "true");
        assert_eq!(value_to_input_text(&json!(false)), "false");
        // number → string
        assert_eq!(value_to_input_text(&json!(42)), "42");
        assert_eq!(value_to_input_text(&json!(3.14)), "3.14");
        // null → ""
        assert_eq!(value_to_input_text(&json!(null)), "");
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
