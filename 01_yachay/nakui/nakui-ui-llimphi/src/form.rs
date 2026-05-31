use super::*;

/// Tras cambiar de módulo/menú: si la vista activa es un `Form`, abre el
/// form fresco (así clickear "Nuevo" en el menú muestra el formulario).
pub(crate) fn sync_form_to_menu(m: &mut Model) {
    let (Some(mod_idx), Some(menu_idx)) = (m.selected_module, m.selected_menu) else {
        return;
    };
    let Some(module) = m.modules.get(mod_idx) else {
        return;
    };
    let Some(item) = module.menu.get(menu_idx) else {
        return;
    };
    if let Some(ModuleView::Form(fv)) = module.views.get(&item.view) {
        m.form = Some(build_form(mod_idx, fv, None));
    }
}

/// Localiza el primer `Form` view de un módulo cuya entity coincide.
pub(crate) fn find_form_view<'a>(module: &'a Module, entity: &str) -> Option<&'a FormView> {
    module.views.values().find_map(|v| match v {
        ModuleView::Form(fv) if fv.entity == entity => Some(fv),
        _ => None,
    })
}

/// Construye un `FormState` desde un `FormView`. `editing` pre-rellena
/// los inputs desde un record existente; en alta, los `AutoId` se
/// rellenan con un UUID nuevo y el resto con su `default`.
pub(crate) fn build_form(module_idx: usize, fv: &FormView, editing: Option<(Uuid, Value)>) -> FormState {
    let fields = fv
        .fields
        .iter()
        .map(|fs| {
            let mut input = TextInputState::new();
            let raw = match &editing {
                Some((_, rec)) => rec
                    .get(&fs.name)
                    .map(value_to_raw)
                    .unwrap_or_default(),
                None => match fs.kind {
                    FieldKind::AutoId => Uuid::new_v4().to_string(),
                    FieldKind::Boolean => fs.default.clone().unwrap_or_else(|| "false".into()),
                    _ => fs.default.clone().unwrap_or_default(),
                },
            };
            input.set_text(raw);
            FieldRuntime {
                spec: fs.clone(),
                input,
            }
        })
        .collect();

    FormState {
        module_idx,
        entity: fv.entity.clone(),
        title: fv.title.clone(),
        on_submit: fv.on_submit.clone(),
        fields,
        editing: editing.as_ref().map(|(id, _)| *id),
        original: editing.map(|(_, v)| v),
        focused: None,
        error: None,
    }
}

/// Representación cruda (string) de un valor JSON para precargar un input.
pub(crate) fn value_to_raw(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub(crate) fn is_text_field(kind: FieldKind) -> bool {
    matches!(
        kind,
        FieldKind::Text | FieldKind::Multiline | FieldKind::Number | FieldKind::Date
    )
}

/// Ejecuta el submit del form activo contra el backend. Espeja
/// `commit_seed` / `commit_morphism` del meta-form GPUI borrado:
/// valida required, parsea por kind, valida `EntityRef`s, y ramifica en
/// edición (`update` con delta) vs alta (`seed`/`morphism`).
///
/// Saca el form del modelo con `take()` para no aliasar `m` mientras
/// tiene tomado el guard del backend; si algo falla, lo reinserta con el
/// error puesto para que la UI lo muestre.
pub(crate) fn submit_form(m: &mut Model) {
    let Some(mut form) = m.form.take() else {
        return;
    };

    // 1. Recolectar y parsear los fields.
    let mut obj = serde_json::Map::new();
    let mut to_clear: Vec<String> = Vec::new();
    let mut entity_refs: Vec<(String, String, Uuid)> = Vec::new();
    let mut by_name: BTreeMap<String, String> = BTreeMap::new();
    let mut parse_error: Option<String> = None;

    for fr in &form.fields {
        let raw = fr.raw();
        by_name.insert(fr.spec.name.clone(), raw.clone());

        if fr.spec.required && raw.trim().is_empty() && fr.spec.kind != FieldKind::AutoId {
            parse_error = Some(format!("campo '{}' es obligatorio", fr.spec.label));
            break;
        }
        if raw.is_empty() && !fr.spec.required {
            to_clear.push(fr.spec.name.clone());
            continue;
        }
        let value = match parse_field_value(fr.spec.kind, &raw) {
            Ok(v) => v,
            Err(e) => {
                parse_error = Some(format!("campo '{}': {e}", fr.spec.label));
                break;
            }
        };
        if fr.spec.kind == FieldKind::EntityRef {
            if let (Some(target), Some(uuid_str)) = (&fr.spec.ref_entity, value.as_str()) {
                if let Ok(id) = Uuid::parse_str(uuid_str) {
                    entity_refs.push((fr.spec.label.clone(), target.clone(), id));
                }
            }
        }
        obj.insert(fr.spec.name.clone(), value);
    }

    if let Some(e) = parse_error {
        form.error = Some(e);
        m.form = Some(form);
        return;
    }

    // 2. Datos derivados (sin tocar `form` durante el lock del backend).
    let module_id = m
        .modules
        .get(form.module_idx)
        .map(|md| md.id.clone())
        .unwrap_or_default();
    let entity = form.entity.clone();
    let editing = form.editing;
    let original = form.original.clone();
    let on_submit = form.on_submit.clone();
    let specs: BTreeMap<String, FieldSpec> = form
        .fields
        .iter()
        .map(|f| (f.spec.name.clone(), f.spec.clone()))
        .collect();

    // 3. Resolver contra el backend (lock una sola vez).
    let result: Result<WriteOutcome, String> = match m.backend.lock() {
        Ok(mut backend) => {
            let refs_ok: Result<(), String> = if entity_refs.is_empty() {
                Ok(())
            } else {
                validate_entity_refs(|e, id| backend.load_record(e, id), &entity_refs)
            };
            match refs_ok {
                Err(e) => Err(e),
                Ok(()) => {
                    if let Some(id) = editing {
                        let current = original.unwrap_or(Value::Null);
                        let set = compute_field_delta(&current, &obj);
                        let clear = compute_clear_fields(&current, &to_clear);
                        backend.update(&entity, id, set, clear)
                    } else {
                        match &on_submit {
                            Action::SeedEntity { entity: e, .. } => backend.seed(e, obj),
                            Action::Morphism {
                                name,
                                inputs,
                                params,
                                ..
                            } => commit_morphism(
                                &mut backend,
                                &module_id,
                                name,
                                inputs,
                                params,
                                &by_name,
                                &specs,
                            ),
                            Action::OpenView { .. } => {
                                Err("on_submit OpenView no crea ni edita records".into())
                            }
                        }
                    }
                }
            }
        }
        Err(_) => Err("backend lock envenenado".into()),
    };

    // 4. Toast + navegación.
    match result {
        Ok(outcome) => {
            let verb = if editing.is_some() { "guardado" } else { "creado" };
            let mut text = match outcome.changed {
                0 => format!("{entity}: sin cambios"),
                _ => format!("{entity} {verb} ✓"),
            };
            if let Some(post) = outcome.post_status {
                text = format!("{text} · {post}");
            }
            m.toast = Some(Toast {
                kind: BannerKind::Success,
                text,
            });
            // `form` queda consumido (no reinsertado): cerramos la sesión.
            navigate_next_view(m, &on_submit);
        }
        Err(e) => {
            form.error = Some(e);
            m.form = Some(form);
        }
    }
}

/// Resuelve inputs (role→field→UUID) y params (fields → JSON) y delega
/// al backend. Espejo de `commit_morphism` del widget GPUI.
pub(crate) fn commit_morphism(
    backend: &mut NakuiBackend,
    module_id: &str,
    name: &str,
    inputs_map: &BTreeMap<String, String>,
    params_fields: &[String],
    by_name: &BTreeMap<String, String>,
    specs: &BTreeMap<String, FieldSpec>,
) -> Result<WriteOutcome, String> {
    // Inputs: cada (role, field) → parsear el value del field como UUID.
    let mut inputs: BTreeMap<String, Uuid> = BTreeMap::new();
    for (role, field_name) in inputs_map {
        let raw = by_name
            .get(field_name)
            .ok_or_else(|| format!("input field '{field_name}' no existe en el form"))?;
        let id = Uuid::parse_str(raw.trim()).map_err(|_| {
            format!("input '{role}' (field '{field_name}'): '{raw}' no es UUID válido")
        })?;
        inputs.insert(role.clone(), id);
    }

    // Params: lista explícita, o todos los fields que no son inputs.
    let input_fields: BTreeSet<&String> = inputs_map.values().collect();
    let field_iter: Vec<String> = if params_fields.is_empty() {
        by_name
            .keys()
            .filter(|k| !input_fields.contains(*k))
            .cloned()
            .collect()
    } else {
        params_fields.to_vec()
    };

    let mut params_obj = serde_json::Map::new();
    for field_name in field_iter {
        let raw = by_name.get(&field_name).cloned().unwrap_or_default();
        let spec = specs.get(&field_name);
        let value = resolve_param_value(&field_name, &raw, spec)?;
        params_obj.insert(field_name, value);
    }

    backend.morphism(module_id, name, inputs, Value::Object(params_obj))
}

/// Tras un submit exitoso, salta al `next_view` declarado en la acción
/// (típicamente `"list"`), seleccionando ese ítem del menú del módulo.
pub(crate) fn navigate_next_view(m: &mut Model, action: &Action) {
    let next = match action {
        Action::SeedEntity { next_view, .. } => next_view.clone(),
        Action::Morphism { next_view, .. } => next_view.clone(),
        Action::OpenView { view, .. } => Some(view.clone()),
    };
    let Some(view_key) = next else {
        return;
    };
    let Some(mod_idx) = m.selected_module else {
        return;
    };
    if let Some(module) = m.modules.get(mod_idx) {
        if let Some(i) = module.menu.iter().position(|it| it.view == view_key) {
            m.selected_menu = Some(i);
        }
    }
}
