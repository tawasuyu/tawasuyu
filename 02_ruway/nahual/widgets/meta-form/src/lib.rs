//! `nahual-widget-meta-form` — widget GPUI de metainterfaz.
//!
//! Renderea cualquier conjunto de [`nahual_meta_schema::Module`] sobre
//! cualquier impl de [`nahual_meta_runtime::MetaBackend`]: sidebar con
//! menús, list/form views, modal de delete, EntityRef selector.
//!
//! El widget es **app-agnostic**: no asume Nakui, ni storage
//! particular, ni lifecycle de bootstrapping. El binario que lo
//! consume:
//!
//! 1. Carga sus módulos (típicamente via `brahman-cards`).
//! 2. Construye su backend concreto (typicamente wireado a un
//!    store/log/executor de su stack).
//! 3. Crea `MetaApp::new(modules, backend, initial_toast,
//!    initial_error, cx)` y lo monta como root view de GPUI.
//!
//! Para una metainterfaz Nakui-completa ver `nakui-ui` (binario
//! shell que provee `NakuiBackend`).

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use gpui::{
    div, prelude::*, px, ClickEvent, Context, Entity, IntoElement, KeyDownEvent, Render,
    SharedString, Window,
};

use nahual_meta_runtime::{
    cmp_values, compute_clear_fields, compute_field_delta, compute_metric, format_value,
    human_label_for_record, parse_field_value, render_value, resolve_param_value, short_uuid,
    to_csv, validate_entity_refs, value_to_input_text, MetaBackend, MetricResult, WriteOutcome,
};
use nahual_meta_schema::{
    Action, Column, DashboardView, DetailView, FieldKind, FieldSpec, FormView, ListView, Module,
    RelatedList, SelectOption, View,
};
use nahual_theme::Theme;
use nahual_widget_banner::{banner_themed, themed_colors, Banner};
use nahual_widget_text_input::TextInput;
use nahual_widget_theme_switcher::theme_switcher;
use serde_json::Value;
use uuid::Uuid;

/// Filas por página en las vistas de lista.
const PAGE_SIZE: usize = 25;

/// Estado del runtime de UI. Toda la persistencia/ejecución está
/// detrás del trait [`MetaBackend`]; este struct sólo conoce GPUI
/// state y el schema de los módulos.
///
/// Genérico sobre el backend `B`. Un binario decide qué backend
/// proveer (Nakui via `NakuiBackend`, mock para tests, etc.).
pub struct MetaApp<B: MetaBackend> {
    /// Módulos cargados, ordenados por id.
    modules: Vec<Module>,
    /// Backend que ejecuta seed/update/delete/morphism.
    backend: B,
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
    /// Si la vista activa es una `Detail`, el id del record que se
    /// muestra. Lo setea [`Self::select_detail`].
    detail_target: Option<Uuid>,
    /// Key de la vista a la que vuelve el botón «← Volver» de una
    /// ficha — la lista desde la que se abrió.
    detail_return: Option<String>,
    /// Estado de la vista de lista activa. Se reinician al navegar.
    /// `list_search`: caja de búsqueda (sólo si la lista declara
    /// `search_in`). `list_sort`: `(columna, ascendente)`.
    /// `list_page`: página actual (0-based).
    list_search: Option<Entity<TextInput>>,
    list_sort: Option<(String, bool)>,
    list_page: usize,
    /// Errores de validación por campo del formulario activo
    /// (nombre → mensaje). Se llenan al fallar un submit y el form los
    /// muestra inline, debajo del campo.
    form_errors: BTreeMap<String, String>,
    /// Mensaje toast al pie (success de submit, error de carga, etc.).
    toast: Option<SharedString>,
    /// Si la carga de módulos falló al inicio.
    load_error: Option<SharedString>,
}

impl<B: MetaBackend> MetaApp<B> {
    /// Constructor del widget. El caller pre-construye sus módulos +
    /// backend + cualquier mensaje inicial que quiera mostrar al
    /// usuario al abrir (típicamente el toast del replay y el error
    /// banner de cosas que fallaron de bootstrap).
    ///
    /// La active view default es la primera entry del menú del
    /// primer módulo (orden lexicográfico por id).
    pub fn new(
        modules: Vec<Module>,
        backend: B,
        initial_toast: Option<String>,
        initial_error: Option<String>,
        _cx: &mut Context<Self>,
    ) -> Self {
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
            detail_target: None,
            detail_return: None,
            list_search: None,
            list_sort: None,
            list_page: 0,
            form_errors: BTreeMap::new(),
            toast: initial_toast.map(SharedString::from),
            load_error: initial_error.map(SharedString::from),
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
        self.detail_target = None;
        self.form_inputs = BTreeMap::new();
        self.form_errors.clear();
        self.list_search = None;
        self.list_sort = None;
        self.list_page = 0;
        if let Some(module) = self.modules.get(mod_idx) {
            match module.views.get(&view_key) {
                Some(View::Form(form)) => {
                    // Snapshot del record si estamos editando esta entity.
                    let editing_record: Option<Value> =
                        self.editing.as_ref().and_then(|(e, id)| {
                            if e == &form.entity {
                                self.backend.load_record(e, *id)
                            } else {
                                None
                            }
                        });
                    for f in &form.fields {
                        let initial = if f.kind == FieldKind::AutoId {
                            // Editando: conservar el id del record.
                            // Alta: UUID nuevo, que el usuario no teclea.
                            editing_record
                                .as_ref()
                                .and_then(|rec| rec.get(&f.name).map(value_to_input_text))
                                .unwrap_or_else(|| Uuid::new_v4().to_string())
                        } else if let Some(rec) = &editing_record {
                            rec.get(&f.name)
                                .map(value_to_input_text)
                                .unwrap_or_else(|| f.default.clone().unwrap_or_default())
                        } else {
                            f.default.clone().unwrap_or_default()
                        };
                        let input = cx.new(|cx| TextInput::new(initial, cx));
                        self.form_inputs.insert(f.name.clone(), input);
                    }
                }
                Some(View::List(lv)) => {
                    // Caja de búsqueda viva si la lista declara search_in.
                    if !lv.search_in.is_empty() {
                        let input = cx.new(|cx| TextInput::new("", cx).with_placeholder("buscar…"));
                        // Re-render del widget cuando el input cambia →
                        // filtrado en vivo mientras se teclea.
                        cx.observe(&input, |_this, _, cx| cx.notify()).detach();
                        self.list_search = Some(input);
                    }
                    self.editing = None;
                }
                _ => {
                    // Cambiar a una view que no es Form invalida el
                    // editing pendiente.
                    self.editing = None;
                }
            }
        }
        cx.notify();
    }

    /// Abre la ficha de detalle `detail_view` para el record `id`.
    /// Recuerda la vista actual para el botón «← Volver».
    fn select_detail(
        &mut self,
        mod_idx: usize,
        detail_view: String,
        id: Uuid,
        cx: &mut Context<Self>,
    ) {
        self.detail_return = self.active.as_ref().map(|(_, v)| v.clone());
        self.active = Some((mod_idx, detail_view));
        self.detail_target = Some(id);
        self.editing = None;
        self.pending_delete = None;
        self.form_inputs = BTreeMap::new();
        self.form_errors.clear();
        self.list_search = None;
        self.list_sort = None;
        self.list_page = 0;
        self.toast = None;
        cx.notify();
    }

    /// Revisa los campos `required` del formulario activo: devuelve un
    /// mapa nombre→error con los que están vacíos. Mapa vacío = todo OK.
    /// `AutoId` se omite — se autogenera, nunca está vacío.
    fn validate_required_fields(&self, cx: &mut Context<Self>) -> BTreeMap<String, String> {
        let mut errors = BTreeMap::new();
        let Some(View::Form(fv)) = self
            .active
            .as_ref()
            .and_then(|(i, vk)| self.modules.get(*i).and_then(|m| m.views.get(vk)))
        else {
            return errors;
        };
        for f in &fv.fields {
            if !f.required || f.kind == FieldKind::AutoId {
                continue;
            }
            let empty = self
                .form_inputs
                .get(&f.name)
                .map(|i| i.read(cx).text().trim().is_empty())
                .unwrap_or(true);
            if empty {
                errors.insert(f.name.clone(), "este campo es obligatorio".to_string());
            }
        }
        errors
    }

    /// Cambia el orden de la lista al hacer clic en un header: misma
    /// columna cicla ascendente → descendente → sin orden.
    fn toggle_sort(&mut self, field: &str, cx: &mut Context<Self>) {
        self.list_sort = next_sort(self.list_sort.take(), field);
        self.list_page = 0;
        cx.notify();
    }

    /// Inicia un edit del record: setea `editing` y abre la primera
    /// view de tipo Form del módulo (convención: la del schema).
    fn open_edit(&mut self, mod_idx: usize, entity: String, id: Uuid, cx: &mut Context<Self>) {
        self.editing = Some((entity.clone(), id));
        let form_view_key = self.modules.get(mod_idx).and_then(|m| {
            m.views.iter().find_map(|(key, v)| match v {
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
    fn commit_delete(&mut self, entity: &str, id: Uuid) -> Result<WriteOutcome, String> {
        self.backend.delete(entity, id)
    }

    /// Aplica una acción (click en menú, botón de form, action de
    /// list). Mutaciones contra el backend ocurren acá.
    pub fn apply_action(&mut self, action: Action, cx: &mut Context<Self>) {
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
                let errors = self.validate_required_fields(cx);
                if !errors.is_empty() {
                    let n = errors.len();
                    self.form_errors = errors;
                    self.toast = Some(SharedString::from(format!(
                        "faltan {n} campo(s) obligatorio(s)"
                    )));
                    cx.notify();
                    return;
                }
                self.form_errors.clear();
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
                let errors = self.validate_required_fields(cx);
                if !errors.is_empty() {
                    let n = errors.len();
                    self.form_errors = errors;
                    self.toast = Some(SharedString::from(format!(
                        "faltan {n} campo(s) obligatorio(s)"
                    )));
                    cx.notify();
                    return;
                }
                self.form_errors.clear();
                match self.commit_morphism(mod_idx, &name, &inputs, &params, cx) {
                    Ok(outcome) => {
                        let base =
                            format!("morphism '{name}' OK ({} op(s) aplicadas)", outcome.changed);
                        self.toast = Some(append_compact_msg(base, outcome.post_status));
                        if let Some(v) = next_view {
                            self.select_view(mod_idx, v, cx);
                        }
                    }
                    Err(e) => {
                        self.toast =
                            Some(SharedString::from(format!("morphism '{name}' falló: {e}")));
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
                format!("input '{role}' (field '{field_name}'): '{raw}' no es UUID válido")
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
            let value =
                parse_field_value(f.kind, &raw).map_err(|e| format!("campo '{}': {e}", f.label))?;
            if f.kind == FieldKind::EntityRef {
                if let (Some(target), Some(uuid_str)) = (&f.ref_entity, value.as_str()) {
                    let id = Uuid::parse_str(uuid_str).expect("parse_field_value validated UUID");
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

    /// Filas de una lista tras aplicar la búsqueda y el orden activos
    /// (sin paginar). Compartido por el render de la lista y el export
    /// a CSV.
    fn list_filtered_sorted(&self, lv: &ListView, cx: &mut Context<Self>) -> Vec<(Uuid, Value)> {
        let mut rows = self.list_rows(&lv.entity);
        let query = self
            .list_search
            .as_ref()
            .map(|i| i.read(cx).text().trim().to_lowercase())
            .unwrap_or_default();
        if !query.is_empty() {
            rows.retain(|(_, v)| {
                lv.search_in.iter().any(|field| {
                    lookup_field(v, field)
                        .map(|cell| render_value(Some(cell)).to_lowercase().contains(&query))
                        .unwrap_or(false)
                })
            });
        }
        if let Some((field, asc)) = &self.list_sort {
            rows.sort_by(|(_, a), (_, b)| {
                let ord = cmp_values(lookup_field(a, field), lookup_field(b, field));
                if *asc {
                    ord
                } else {
                    ord.reverse()
                }
            });
        }
        rows
    }

    /// Exporta la lista (filas filtradas/ordenadas, todas las columnas)
    /// a un archivo CSV en el directorio de trabajo; avisa por toast.
    fn export_csv(&mut self, lv: &ListView, cx: &mut Context<Self>) {
        let rows = self.list_filtered_sorted(lv, cx);
        let headers: Vec<String> = lv.columns.iter().map(|c| c.label.clone()).collect();
        let data: Vec<Vec<String>> = rows
            .iter()
            .map(|(_, v)| {
                lv.columns
                    .iter()
                    .map(|c| self.render_cell(c, lookup_field(v, &c.field)))
                    .collect()
            })
            .collect();
        let csv = to_csv(&headers, &data);
        let path = export_path(&lv.entity);
        self.toast = Some(SharedString::from(match std::fs::write(&path, csv) {
            Ok(()) => format!("exporté {} fila(s) a {}", rows.len(), path.display()),
            Err(e) => format!("no pude exportar CSV: {e}"),
        }));
        cx.notify();
    }
}

/// Formatea el toast para la rama Action::SeedEntity según el
/// `WriteOutcome` del backend. `was_editing` distingue "creado"
/// vs "actualizado" — el WriteOutcome solo no alcanza porque
/// `seed` y `update` ambos devuelven `id = Some(...)`.
fn format_seed_toast(entity: &str, was_editing: bool, outcome: &WriteOutcome) -> String {
    let id_short = outcome.id.map(|id| short_uuid(&id)).unwrap_or_default();
    match (was_editing, outcome.changed) {
        (false, _) => format!("creado {entity} {id_short}"),
        (true, 0) => format!("{entity} {id_short} sin cambios — no log entry"),
        (true, n) => format!("actualizado {entity} {id_short} ({n} campo(s))"),
    }
}

/// Concatena el msg opcional del compact tick al toast del op
/// original con `"; "` separator.
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

/// Ruta destino de un export CSV: `<entity>-<unix-secs>.csv` en el
/// directorio de trabajo actual.
fn export_path(entity: &str) -> std::path::PathBuf {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = format!("{entity}-{secs}.csv");
    std::env::current_dir()
        .map(|d| d.join(&name))
        .unwrap_or_else(|_| std::path::PathBuf::from(name))
}

/// Próximo estado de orden al hacer clic en el header `field`: la misma
/// columna cicla ascendente → descendente → sin orden; otra columna
/// arranca ascendente.
fn next_sort(current: Option<(String, bool)>, field: &str) -> Option<(String, bool)> {
    match current {
        Some((f, true)) if f == field => Some((f, false)),
        Some((f, false)) if f == field => None,
        _ => Some((field.to_string(), true)),
    }
}

impl<B: MetaBackend> Render for MetaApp<B> {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Paleta del chrome viene del Theme global. Derivamos los
        // 7 slots que las funciones render_* usan; las firmas
        // siguen tomando los colors individuales (Hsla / Background)
        // para preservar el shape del API interno.
        //
        // Si el caller no instaló un Theme, `Theme::global` panicea.
        // Convención: el binario shell instala el theme en main.
        let theme = Theme::global(cx).clone();
        let bg = theme.bg_app;
        let panel = theme.bg_panel;
        let border = theme.border;
        let text = theme.fg_text;
        let text_dim = theme.fg_muted;
        let accent = theme.accent;
        let accent_active = theme.accent_strong;

        let sidebar = self.render_sidebar(cx, panel, border, text, text_dim, accent_active);
        let main_panel = self.render_main(cx, panel, border, text, text_dim, accent);
        let confirm_banner = self.render_confirm_delete_banner(cx);
        let toast_div = self
            .toast
            .as_ref()
            .map(|t| banner_themed(cx, Banner::Success, t.clone()));
        let error_banner = self
            .load_error
            .as_ref()
            .map(|e| banner_themed(cx, Banner::Error, e.clone()));

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

impl<B: MetaBackend> MetaApp<B> {
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

        // Banner base usa los colors themed del kind Warning (amber
        // que sigue is_dark del theme actual). Los buttons confirm
        // (rojo) y cancel (gris) usan los colors themed del Error
        // y un derivative del border respectivamente.
        let theme = Theme::global(cx);
        let (banner_bg, banner_text) = themed_colors(Banner::Warning, theme);
        let (confirm_bg, confirm_text) = themed_colors(Banner::Error, theme);
        let cancel_bg: gpui::Background = theme.bg_panel_alt;
        let cancel_text = theme.fg_text;
        // Hover colors capturados antes de las closures para que el
        // move |d| d.bg(...) los cierre.
        let cancel_hover = theme.bg_button_hover();
        let confirm_hover = theme.bg_destructive_hover();
        let hint_color = theme.fg_muted;

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
                                .text_color(hint_color)
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
                        .hover(move |d| d.bg(cancel_hover))
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
                        .id(SharedString::from(format!("confirm-del-ok-{}", id_owned)))
                        .px(px(10.))
                        .py(px(4.))
                        .bg(confirm_bg)
                        .text_color(confirm_text)
                        .hover(move |d| d.bg(confirm_hover))
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
                                    this.toast =
                                        Some(append_compact_msg(base, outcome.post_status));
                                }
                                Err(e) => {
                                    this.toast =
                                        Some(SharedString::from(format!("error borrando: {e}")));
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
        panel: gpui::Background,
        border: gpui::Hsla,
        text: gpui::Hsla,
        text_dim: gpui::Hsla,
        accent_active: gpui::Hsla,
    ) -> gpui::Div {
        // Slots ornament del theme para los menu items de abajo.
        let theme = Theme::global(cx);
        let menu_active_bg = theme.bg_row_active;
        let menu_hover_bg = theme.bg_row_hover;
        let mut sidebar = div()
            .w(px(240.))
            .h_full()
            .bg(panel)
            .border_r_1()
            .border_color(border)
            .flex()
            .flex_col();

        // Sidebar header: título + theme switcher en una row.
        sidebar = sidebar.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .px(px(12.))
                .py(px(10.))
                .text_color(text)
                .text_size(px(13.))
                .child(div().flex_grow().child("Nakui"))
                .child(theme_switcher(cx)),
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
                        .when(is_active, move |d| d.bg(menu_active_bg).text_color(text))
                        .hover(move |d| d.bg(menu_hover_bg))
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
        panel: gpui::Background,
        border: gpui::Hsla,
        text: gpui::Hsla,
        text_dim: gpui::Hsla,
        accent: gpui::Hsla,
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
            View::List(lv) => {
                self.render_list(cx, main, &lv, mod_idx, border, text, text_dim, accent)
            }
            View::Form(fv) => {
                self.render_form(cx, main, &fv, mod_idx, border, text, text_dim, accent)
            }
            View::Detail(dv) => {
                self.render_detail(cx, main, &dv, mod_idx, border, text, text_dim, accent)
            }
            View::Dashboard(dv) => {
                self.render_dashboard(cx, main, &dv, border, text, text_dim, accent)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_list(
        &self,
        cx: &mut Context<Self>,
        mut main: gpui::Div,
        lv: &ListView,
        mod_idx: usize,
        border: gpui::Hsla,
        text: gpui::Hsla,
        text_dim: gpui::Hsla,
        accent: gpui::Hsla,
    ) -> gpui::Div {
        // Ornament secundarios del theme para hovers, row separators,
        // botones inline (edit ✎, delete ✕).
        let theme = Theme::global(cx);
        let row_separator = theme.bg_row_active;
        let action_bg = theme.bg_button();
        let action_hover = theme.bg_button_hover();
        let destructive_fg = theme.accent_destructive();
        let destructive_hover = theme.bg_destructive_hover();
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
                    .id(SharedString::from(format!("list-action-{mod_idx}-{idx}")))
                    .px(px(10.))
                    .py(px(4.))
                    .bg(action_bg)
                    .text_color(accent)
                    .text_size(px(11.))
                    .rounded(px(3.))
                    .hover(move |d| d.bg(action_hover))
                    .child(label)
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        this.apply_action(action_clone.clone(), cx);
                    })),
            );
        }
        // Botón de export CSV — disponible en toda lista.
        {
            let lv_for_csv = lv.clone();
            header = header.child(
                div()
                    .id(SharedString::from(format!("list-csv-{mod_idx}")))
                    .px(px(10.))
                    .py(px(4.))
                    .bg(action_bg)
                    .text_color(accent)
                    .text_size(px(11.))
                    .rounded(px(3.))
                    .hover(move |d| d.bg(action_hover))
                    .child("⬇ CSV")
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        this.export_csv(&lv_for_csv, cx);
                    })),
            );
        }
        main = main.child(header);

        // Caja de búsqueda (sólo si la lista declara `search_in`).
        if let Some(search) = &self.list_search {
            main = main.child(
                div()
                    .mb(px(8.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .child(div().text_color(text_dim).text_size(px(12.)).child("🔍"))
                    .child(search.clone()),
            );
        }

        let row_detail = lv.row_detail.clone();

        // Filas: buscar + ordenar (helper compartido con el export CSV),
        // luego paginar.
        let all_rows = self.list_filtered_sorted(lv, cx);
        let has_query = self
            .list_search
            .as_ref()
            .map(|i| !i.read(cx).text().trim().is_empty())
            .unwrap_or(false);
        let total = all_rows.len();
        let page_count = total.div_ceil(PAGE_SIZE).max(1);
        let page = self.list_page.min(page_count - 1);
        let start = page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(total);
        let rows: Vec<(Uuid, Value)> = all_rows[start..end].to_vec();

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
            // Indicador del orden activo + clic en el header para ordenar.
            let arrow = match &self.list_sort {
                Some((f, asc)) if f == &c.field => {
                    if *asc {
                        " ▲"
                    } else {
                        " ▼"
                    }
                }
                _ => "",
            };
            let field = c.field.clone();
            col_header = col_header.child(
                div()
                    .id(SharedString::from(format!("col-{mod_idx}-{}", c.field)))
                    .flex_grow()
                    .flex_basis(px(100. * frac))
                    .hover(move |d| d.bg(action_hover))
                    .child(format!("{}{arrow}", c.label))
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        this.toggle_sort(&field, cx);
                    })),
            );
        }
        col_header = col_header
            .child(div().w(px(80.)).text_color(text_dim).child("id"))
            .child(div().w(px(100.)).text_color(text_dim).child("acciones"));
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
                .border_color(row_separator)
                .text_color(text)
                .text_size(px(12.));
            for c in &lv.columns {
                let frac = c.weight / total_weight;
                let v = lookup_field(value, &c.field);
                row = row.child(
                    div()
                        .flex_grow()
                        .flex_basis(px(100. * frac))
                        .child(self.render_cell(c, v)),
                );
            }
            row = row.child(
                div()
                    .w(px(80.))
                    .text_color(text_dim)
                    .text_size(px(11.))
                    .child(short_uuid(id)),
            );
            // Acciones por fila: 👁 ver (si hay row_detail) + ✎ + ✕.
            let mut actions = div().w(px(100.)).flex().flex_row().gap(px(4.));
            if let Some(detail_view) = &row_detail {
                let dv_key = detail_view.clone();
                actions = actions.child(
                    div()
                        .id(SharedString::from(format!("row-view-{mod_idx}-{id_copy}")))
                        .px(px(6.))
                        .text_color(text_dim)
                        .text_size(px(13.))
                        .hover(move |d| d.bg(action_hover))
                        .child("👁")
                        .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                            this.select_detail(mod_idx, dv_key.clone(), id_copy, cx);
                        })),
                );
            }
            row = row.child(
                actions
                    .child(
                        div()
                            .id(SharedString::from(format!("row-edit-{mod_idx}-{id_copy}")))
                            .px(px(6.))
                            .text_color(accent)
                            .text_size(px(13.))
                            .hover(move |d| d.bg(action_hover))
                            .child("✎")
                            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                                this.open_edit(mod_idx, entity_for_edit.clone(), id_copy, cx);
                            })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("row-del-{mod_idx}-{id_copy}")))
                            .px(px(6.))
                            .text_color(destructive_fg)
                            .text_size(px(13.))
                            .hover(move |d| d.bg(destructive_hover))
                            .child("✕")
                            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                                // Marca para borrar en lugar de borrar
                                // directo: el modal de confirmación se
                                // renderea arriba en `render` y maneja
                                // confirm/cancel.
                                this.pending_delete = Some((entity_for_delete.clone(), id_copy));
                                this.toast = None;
                                cx.notify();
                            })),
                    ),
            );
            main = main.child(row);
        }

        if total == 0 {
            let msg = if has_query {
                "(sin resultados para la búsqueda)".to_string()
            } else {
                format!("(sin {})", lv.entity)
            };
            main = main.child(
                div()
                    .py(px(12.))
                    .text_color(text_dim)
                    .text_size(px(12.))
                    .child(msg),
            );
        } else {
            let mut footer = div()
                .mt(px(8.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .text_color(text_dim)
                .text_size(px(11.));
            if page_count > 1 {
                let last_page = page_count - 1;
                footer = footer.child(
                    div()
                        .id("list-prev")
                        .px(px(8.))
                        .py(px(2.))
                        .bg(action_bg)
                        .rounded(px(3.))
                        .text_color(if page == 0 { text_dim } else { accent })
                        .hover(move |d| d.bg(action_hover))
                        .child("◀")
                        .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                            this.list_page = this.list_page.saturating_sub(1);
                            cx.notify();
                        })),
                );
                footer = footer.child(div().child(format!("página {}/{}", page + 1, page_count)));
                footer = footer.child(
                    div()
                        .id("list-next")
                        .px(px(8.))
                        .py(px(2.))
                        .bg(action_bg)
                        .rounded(px(3.))
                        .text_color(if page >= last_page { text_dim } else { accent })
                        .hover(move |d| d.bg(action_hover))
                        .child("▶")
                        .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                            this.list_page = (this.list_page + 1).min(last_page);
                            cx.notify();
                        })),
                );
            }
            footer = footer.child(div().child(format!("{total} fila(s)")));
            main = main.child(footer);
        }

        main
    }

    /// Renderea el selector clickable de records existentes para un
    /// FieldSpec con kind=EntityRef. Lista compacta debajo del input;
    /// click en una opción setea el TextInput del field con el UUID
    /// seleccionado. El item del UUID actualmente seleccionado (si
    /// hay) se resalta con accent color.
    /// Renderea una ficha de detalle: campos del record + listas de
    /// records relacionados (back-references) + botones Volver/Editar.
    #[allow(clippy::too_many_arguments)]
    fn render_detail(
        &self,
        cx: &mut Context<Self>,
        mut main: gpui::Div,
        dv: &DetailView,
        mod_idx: usize,
        border: gpui::Hsla,
        text: gpui::Hsla,
        text_dim: gpui::Hsla,
        accent: gpui::Hsla,
    ) -> gpui::Div {
        let theme = Theme::global(cx);
        let action_bg = theme.bg_button();
        let action_hover = theme.bg_button_hover();
        let row_separator = theme.bg_row_active;

        let back_view = self.detail_return.clone();
        let mut header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .mb(px(12.))
            .child(
                div()
                    .text_color(text)
                    .text_size(px(18.))
                    .flex_grow()
                    .child(dv.title.clone()),
            )
            .child(
                div()
                    .id("detail-back")
                    .px(px(10.))
                    .py(px(4.))
                    .bg(action_bg)
                    .text_color(accent)
                    .text_size(px(11.))
                    .rounded(px(3.))
                    .hover(move |d| d.bg(action_hover))
                    .child("← Volver")
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        match back_view.clone() {
                            Some(v) => this.select_view(mod_idx, v, cx),
                            None => {
                                if let Some(first) = this
                                    .modules
                                    .get(mod_idx)
                                    .and_then(|m| m.menu.first().map(|i| i.view.clone()))
                                {
                                    this.select_view(mod_idx, first, cx);
                                }
                            }
                        }
                    })),
            );

        let Some(target_id) = self.detail_target else {
            return main.child(header).child(
                div()
                    .text_color(text_dim)
                    .child("Ningún record seleccionado."),
            );
        };

        let entity_for_edit = dv.entity.clone();
        header = header.child(
            div()
                .id("detail-edit")
                .px(px(10.))
                .py(px(4.))
                .bg(action_bg)
                .text_color(accent)
                .text_size(px(11.))
                .rounded(px(3.))
                .hover(move |d| d.bg(action_hover))
                .child("✎ Editar")
                .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                    this.open_edit(mod_idx, entity_for_edit.clone(), target_id, cx);
                })),
        );
        main = main.child(header);

        let Some(record) = self.backend.load_record(&dv.entity, target_id) else {
            return main.child(div().text_color(text_dim).child(format!(
                "El record {} ya no existe.",
                short_uuid(&target_id)
            )));
        };

        // Campos del record.
        let mut fields_box = div().flex().flex_col();
        for c in &dv.fields {
            let v = lookup_field(&record, &c.field);
            fields_box = fields_box.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(12.))
                    .py(px(4.))
                    .border_b_1()
                    .border_color(row_separator)
                    .child(
                        div()
                            .w(px(160.))
                            .flex_none()
                            .text_color(text_dim)
                            .text_size(px(12.))
                            .child(c.label.clone()),
                    )
                    .child(
                        div()
                            .flex_grow()
                            .text_color(text)
                            .text_size(px(12.))
                            .child(self.render_cell(c, v)),
                    ),
            );
        }
        main = main.child(fields_box);

        // Listas de records relacionados.
        for rl in &dv.related {
            main = main.child(self.render_related(rl, target_id, border, text, text_dim));
        }
        main
    }

    /// Renderea una lista de records relacionados (back-reference):
    /// los records de `rl.entity` cuyo `rl.via_field` apunta al record
    /// `target_id`.
    fn render_related(
        &self,
        rl: &RelatedList,
        target_id: Uuid,
        border: gpui::Hsla,
        text: gpui::Hsla,
        text_dim: gpui::Hsla,
    ) -> gpui::Div {
        let id_str = target_id.to_string();
        let rows: Vec<(Uuid, Value)> = self
            .backend
            .list_records(&rl.entity)
            .into_iter()
            .filter(|(_, v)| v.get(&rl.via_field).and_then(Value::as_str) == Some(id_str.as_str()))
            .collect();

        let mut section = div().flex().flex_col().mt(px(18.)).child(
            div()
                .text_color(text)
                .text_size(px(13.))
                .mb(px(4.))
                .child(format!("{} ({})", rl.title, rows.len())),
        );

        if rows.is_empty() {
            return section.child(
                div()
                    .py(px(4.))
                    .text_color(text_dim)
                    .text_size(px(11.))
                    .child("(ninguno)"),
            );
        }

        let total_weight: f32 = rl.columns.iter().map(|c| c.weight).sum::<f32>().max(0.01);
        let mut head = div()
            .flex()
            .flex_row()
            .py(px(4.))
            .border_b_1()
            .border_color(border)
            .text_color(text_dim)
            .text_size(px(10.));
        for c in &rl.columns {
            head = head.child(
                div()
                    .flex_grow()
                    .flex_basis(px(100. * c.weight / total_weight))
                    .child(c.label.clone()),
            );
        }
        section = section.child(head);

        for (_, v) in &rows {
            let mut row = div()
                .flex()
                .flex_row()
                .py(px(3.))
                .text_color(text)
                .text_size(px(11.));
            for c in &rl.columns {
                row = row.child(
                    div()
                        .flex_grow()
                        .flex_basis(px(100. * c.weight / total_weight))
                        .child(self.render_cell(c, lookup_field(v, &c.field))),
                );
            }
            section = section.child(row);
        }
        section
    }

    /// Renderea un tablero: una grilla de tarjetas de KPI, cada una con
    /// su agregado computado sobre los records de su entity.
    #[allow(clippy::too_many_arguments)]
    fn render_dashboard(
        &self,
        cx: &mut Context<Self>,
        mut main: gpui::Div,
        dv: &DashboardView,
        border: gpui::Hsla,
        text: gpui::Hsla,
        text_dim: gpui::Hsla,
        accent: gpui::Hsla,
    ) -> gpui::Div {
        let card_bg = Theme::global(cx).bg_panel_alt;
        main = main.child(
            div()
                .text_color(text)
                .text_size(px(18.))
                .mb(px(12.))
                .child(dv.title.clone()),
        );

        let mut grid = div().flex().flex_row().flex_wrap().gap(px(12.));
        for card in &dv.cards {
            let records = self.backend.list_records(&card.entity);
            let result = compute_metric(&card.metric, card.filter.as_ref(), &records);
            let mut card_box = div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(14.))
                .min_w(px(190.))
                .bg(card_bg)
                .border_1()
                .border_color(border)
                .rounded(px(8.))
                .child(
                    div()
                        .text_color(text_dim)
                        .text_size(px(11.))
                        .child(card.label.clone()),
                );
            match result {
                MetricResult::Scalar(s) => {
                    // Entero si no tiene parte decimal — `Count` y sumas
                    // de enteros se ven sin `.00`.
                    let value = if s.fract() == 0.0 {
                        Value::from(s as i64)
                    } else {
                        Value::from(s)
                    };
                    card_box = card_box.child(
                        div()
                            .text_color(accent)
                            .text_size(px(26.))
                            .child(format_value(Some(&value), &card.format)),
                    );
                }
                MetricResult::Breakdown(rows) => {
                    if rows.is_empty() {
                        card_box = card_box.child(
                            div()
                                .text_color(text_dim)
                                .text_size(px(11.))
                                .child("(sin datos)"),
                        );
                    }
                    let max = rows.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1);
                    for (key, n) in rows {
                        let bar = "█".repeat((n * 12 / max).max(1));
                        card_box = card_box.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(11.))
                                .child(div().w(px(96.)).flex_none().text_color(text).child(key))
                                .child(div().flex_grow().text_color(accent).child(bar))
                                .child(div().text_color(text_dim).child(n.to_string())),
                        );
                    }
                }
            }
            grid = grid.child(card_box);
        }
        main.child(grid)
    }

    /// Render del valor de una celda de lista. Una columna con
    /// `ref_entity` resuelve su UUID al label del record referido; el
    /// resto aplica el `ValueFormat` declarado en la columna.
    fn render_cell(&self, c: &Column, v: Option<&Value>) -> String {
        if let Some(ref_entity) = &c.ref_entity {
            return match v {
                Some(Value::String(s)) => match Uuid::parse_str(s) {
                    Ok(uuid) => self
                        .backend
                        .load_record(ref_entity, uuid)
                        .map(|rec| human_label_for_record(&rec, &uuid))
                        .unwrap_or_else(|| format!("(borrado · {})", short_uuid(&uuid))),
                    Err(_) => render_value(v),
                },
                _ => render_value(v),
            };
        }
        format_value(v, &c.format)
    }

    /// Label legible del record referenciado por un campo EntityRef.
    /// `(sin seleccionar)` si el campo está vacío.
    fn ref_label(&self, target: &str, current: &str) -> String {
        let current = current.trim();
        if current.is_empty() {
            return "(sin seleccionar)".to_string();
        }
        match Uuid::parse_str(current) {
            Ok(uuid) => self
                .backend
                .load_record(target, uuid)
                .map(|rec| human_label_for_record(&rec, &uuid))
                .unwrap_or_else(|| format!("(borrado · {})", short_uuid(&uuid))),
            Err(_) => current.to_string(),
        }
    }

    /// Chips clickables para un campo [`FieldKind::Select`]. El chip de
    /// la opción elegida se resalta con accent. Click setea el
    /// `TextInput` del field (de donde lee el submit), igual que el
    /// selector de EntityRef.
    fn render_select_chips(
        &self,
        cx: &mut Context<Self>,
        field_name: String,
        options: &[SelectOption],
        text_dim: gpui::Hsla,
        accent: gpui::Hsla,
    ) -> gpui::Div {
        let theme = Theme::global(cx);
        let row_active = theme.bg_row_active;
        let row_hover = theme.bg_row_hover;
        let border = theme.border;
        let current = self
            .form_inputs
            .get(&field_name)
            .map(|inp| inp.read(&*cx).text().to_string())
            .unwrap_or_default();

        let mut row = div().mt(px(2.)).flex().flex_row().flex_wrap().gap(px(4.));

        for opt in options {
            let value = opt.value.clone();
            let is_selected = current == value;
            let field_for_click = field_name.clone();
            let value_for_click = value.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("select-{field_name}-{value}")))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if is_selected { accent } else { border })
                    .text_size(px(11.))
                    .text_color(if is_selected { accent } else { text_dim })
                    .when(is_selected, move |d| d.bg(row_active))
                    .hover(move |d| d.bg(row_hover))
                    .child(opt.display().to_string())
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        if let Some(input) = this.form_inputs.get(&field_for_click) {
                            input.update(cx, |inp, cx| inp.set_text(value_for_click.clone(), cx));
                        }
                        cx.notify();
                    })),
            );
        }
        row
    }

    #[allow(clippy::too_many_arguments)]
    fn render_entity_ref_selector(
        &self,
        cx: &mut Context<Self>,
        field_name: String,
        target_entity: String,
        text: gpui::Hsla,
        text_dim: gpui::Hsla,
        accent: gpui::Hsla,
    ) -> gpui::Div {
        let _ = text;
        // Slots ornament para hover/selected del selector + border.
        let theme = Theme::global(cx);
        let row_active = theme.bg_row_active;
        let row_hover = theme.bg_row_hover;
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
            .border_color(theme.border)
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
                    .when(is_selected, move |d| d.bg(row_active))
                    .hover(move |d| d.bg(row_hover))
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
        border: gpui::Hsla,
        text: gpui::Hsla,
        text_dim: gpui::Hsla,
        accent: gpui::Hsla,
    ) -> gpui::Div {
        // Slots ornament para el botón submit + bg de fallback inputs.
        let theme = Theme::global(cx);
        let submit_bg = theme.bg_button();
        let submit_hover = theme.bg_button_hover();
        let input_bg = theme.bg_input();
        let destructive = theme.accent_destructive();
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
        let mut current_section: Option<&str> = None;
        for f in &fv.fields {
            // Encabezado al cambiar de sección (campos consecutivos con
            // la misma `section` se agrupan bajo un título).
            if f.section.as_deref() != current_section {
                if let Some(sec) = &f.section {
                    main = main.child(
                        div()
                            .mt(px(8.))
                            .mb(px(4.))
                            .pb(px(2.))
                            .border_b_1()
                            .border_color(border)
                            .text_color(accent)
                            .text_size(px(12.))
                            .child(sec.clone()),
                    );
                }
                current_section = f.section.as_deref();
            }

            let label = if f.required {
                format!("{} *", f.label)
            } else {
                f.label.clone()
            };
            // Si el campo tiene un error de validación, el label se
            // resalta en color destructivo.
            let has_error = self.form_errors.contains_key(&f.name);
            let label_color = if has_error { destructive } else { text_dim };

            let mut field_box = div().flex().flex_col().mb(px(10.)).child(
                div()
                    .text_color(label_color)
                    .text_size(px(11.))
                    .mb(px(2.))
                    .child(label),
            );

            // Render según el kind. AutoId y Select NO montan el
            // TextInput editable —aunque sí existe en `form_inputs`, de
            // donde lee el submit—; presentan su propio control.
            match f.kind {
                FieldKind::AutoId => {
                    // Read-only: el UUID autogenerado, sólo informativo.
                    let val = self
                        .form_inputs
                        .get(&f.name)
                        .map(|i| i.read(&*cx).text().to_string())
                        .unwrap_or_default();
                    field_box = field_box.child(
                        div()
                            .px(px(8.))
                            .py(px(6.))
                            .bg(input_bg)
                            .text_color(text_dim)
                            .text_size(px(11.))
                            .child(format!("{val}  ·  automático")),
                    );
                }
                FieldKind::Select => {
                    field_box = field_box.child(self.render_select_chips(
                        cx,
                        f.name.clone(),
                        &f.options,
                        text_dim,
                        accent,
                    ));
                }
                FieldKind::EntityRef => {
                    // Display read-only del record elegido (label, no
                    // el UUID crudo) + selector clickable debajo. El
                    // TextInput vive en `form_inputs` pero no se monta.
                    if let Some(target) = &f.ref_entity {
                        let current = self
                            .form_inputs
                            .get(&f.name)
                            .map(|i| i.read(&*cx).text().to_string())
                            .unwrap_or_default();
                        let is_empty = current.trim().is_empty();
                        field_box = field_box.child(
                            div()
                                .px(px(8.))
                                .py(px(6.))
                                .bg(input_bg)
                                .text_color(if is_empty { text_dim } else { text })
                                .text_size(px(11.))
                                .child(self.ref_label(target, &current)),
                        );
                        field_box = field_box.child(self.render_entity_ref_selector(
                            cx,
                            f.name.clone(),
                            target.clone(),
                            text,
                            text_dim,
                            accent,
                        ));
                    }
                    // Sin ref_entity es imposible: Module::validate lo
                    // rechaza al cargar el módulo.
                }
                _ => {
                    // Mount del TextInput vivo (creado en select_view).
                    if let Some(input) = self.form_inputs.get(&f.name) {
                        field_box = field_box.child(input.clone());
                    } else {
                        // No debería pasar — select_view crea inputs por
                        // cada field. Fallback estático por seguridad.
                        field_box = field_box.child(
                            div()
                                .px(px(8.))
                                .py(px(6.))
                                .bg(input_bg)
                                .text_color(text_dim)
                                .child("(input no inicializado)"),
                        );
                    }
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
            // Error de validación inline, debajo del campo.
            if let Some(err) = self.form_errors.get(&f.name) {
                field_box = field_box.child(
                    div()
                        .mt(px(2.))
                        .text_color(destructive)
                        .text_size(px(10.))
                        .child(err.clone()),
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
                    .bg(submit_bg)
                    .text_color(accent)
                    .text_size(px(12.))
                    .rounded(px(3.))
                    .hover(move |d| d.bg(submit_hover))
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
    //! Tests del widget. Funciones puras (sin GPUI cx) sólo. Los
    //! tests de backend impl viven en el binario que provee el
    //! backend (ej: nakui-ui). Los helpers movidos a
    //! nahual-meta-runtime se testean allí.
    use super::*;
    use serde_json::json;

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
    fn next_sort_cycles_asc_desc_off() {
        // Sin orden → ascendente.
        let s = next_sort(None, "monto");
        assert_eq!(s, Some(("monto".to_string(), true)));
        // Misma columna → descendente.
        let s = next_sort(s, "monto");
        assert_eq!(s, Some(("monto".to_string(), false)));
        // Misma columna otra vez → sin orden.
        let s = next_sort(s, "monto");
        assert_eq!(s, None);
        // Otra columna siempre arranca ascendente.
        let s = next_sort(Some(("monto".to_string(), false)), "etapa");
        assert_eq!(s, Some(("etapa".to_string(), true)));
    }

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

    #[test]
    fn format_seed_toast_distinguishes_create_update_no_change() {
        let id = Uuid::new_v4();
        let outcome_create = WriteOutcome {
            id: Some(id),
            changed: 1,
            post_status: None,
        };
        let toast = format_seed_toast("Customer", false, &outcome_create);
        assert!(toast.starts_with("creado Customer"));

        let outcome_update = WriteOutcome {
            id: Some(id),
            changed: 3,
            post_status: None,
        };
        let toast = format_seed_toast("Customer", true, &outcome_update);
        assert!(toast.starts_with("actualizado Customer"));
        assert!(toast.contains("(3 campo(s))"));

        let outcome_no_change = WriteOutcome::no_change(id);
        let toast = format_seed_toast("Customer", true, &outcome_no_change);
        assert!(toast.contains("sin cambios"));
    }
}
