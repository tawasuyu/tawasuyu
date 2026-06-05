//! Lógica de actualización del bucle Elm: el `match` central, las
//! mutaciones del modelo (abrir/crear/guardar/mover/regenerar), el
//! find-in-page, y el trabajo LLM lanzado en un thread aparte.

use std::collections::HashMap;

use llimphi_motion::{animate, motion, Tween};
use llimphi_ui::Handle;
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_text_editor::PointerEvent;
use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_cuerpo::CambioAtom;
use pluma_llm::{build_client, LlmConfig};
use pluma_transform::{TipoTransformacion, Transformacion};
use pluma_transform_llm::{
    EjecutorReescribirLlm, EjecutorResumirLlm, EjecutorTonoLlm, EjecutorTraducirLlm,
};
use uuid::Uuid;

use crate::model::{Model, Msg, BACKENDS, METRICS, VISIBLE_LINES};
use crate::util::{ahora_unix, etiqueta_backend, expandir_ruta, extension_lower};

pub(crate) fn actualizar(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
    match msg {
        Msg::EditorKey(ev) => {
            let _ = model.ide.apply_key_with_clipboard(&ev, &mut model.clipboard);
        }
        Msg::MultiPointer(id, ev) => {
            // Click en una columna que no es el activo → primero le da el foco
            // (a partir de acá el teclado va a ese cuerpo, vía model.ide).
            if model.activo != Some(id) {
                cambiar_activo(&mut model, id);
            }
            let scroll = model.ide.state.scroll_offset;
            match ev {
                PointerEvent::Click { x, y } => {
                    model.drag_accum = (0.0, 0.0);
                    let (line, col) = METRICS.screen_to_pos(x, y, scroll);
                    model.ide.set_caret(line, col);
                }
                PointerEvent::Drag {
                    initial_x,
                    initial_y,
                    dx,
                    dy,
                } => {
                    model.drag_accum.0 += dx;
                    model.drag_accum.1 += dy;
                    let cx = initial_x + model.drag_accum.0;
                    let cy = initial_y + model.drag_accum.1;
                    let (line, col) = METRICS.screen_to_pos(cx, cy, scroll);
                    model.ide.state.extend_selection_to(line, col);
                }
            }
        }
        Msg::AbrirDoc(id) => {
            cambiar_activo(&mut model, id);
        }
        Msg::ToggleSeleccion(id) => {
            toggle_seleccion(&mut model, id);
        }
        Msg::SelectDiente(i) => {
            model.diente_activo = i;
        }
        Msg::FocoSiguiente => {
            mover_foco(&mut model, 1);
        }
        Msg::FocoAnterior => {
            mover_foco(&mut model, -1);
        }
        Msg::ToggleFocoHover => {
            model.foco_por_hover = !model.foco_por_hover;
            model.ultimo_status = if model.foco_por_hover {
                "foco por hover: ON".into()
            } else {
                "foco por hover: off".into()
            };
        }
        Msg::ScrollHoriz(dx) => {
            model.scroll_x += dx; // el clamp final acota a [0, max]
        }
        Msg::ScrollVert(dy) => {
            // Rueda arriba (dy>0) → ver líneas anteriores (offset baja). 3
            // líneas por notch. El nivelado al resto lo hace el final del update.
            let lineas = (dy * 3.0).round() as i64;
            let max = model.ide.state.line_count().saturating_sub(1) as i64;
            let actual = model.ide.state.scroll_offset as i64;
            model.ide.state.scroll_offset = (actual - lineas).clamp(0, max) as usize;
        }
        Msg::Resized(w, h) => {
            model.viewport = (w, h);
        }
        Msg::NuevoDoc => {
            crear_doc_nuevo(&mut model);
        }
        Msg::Guardar => {
            guardar_activo(&mut model);
        }
        Msg::PathInputKey(ev) => {
            model.path_input.apply_key(&ev);
        }
        Msg::FocusPath => {
            model.path_focused = true;
        }
        Msg::DefocusPath => {
            model.path_focused = false;
        }
        Msg::AbrirArchivo => {
            model.path_focused = false;
            abrir_archivo(&mut model);
        }
        Msg::ExportarMd => {
            model.path_focused = false;
            exportar_md(&mut model);
        }
        Msg::FindToggle => {
            model.find_visible = !model.find_visible;
            if model.find_visible {
                recomputar_matches(&mut model);
                if !model.find_matches.is_empty() {
                    saltar_a_match(&mut model);
                }
            }
        }
        Msg::FindKey(ev) => {
            model.find_input.apply_key(&ev);
            recomputar_matches(&mut model);
            if !model.find_matches.is_empty() {
                saltar_a_match(&mut model);
            }
        }
        Msg::FindSiguiente => {
            if model.find_matches.is_empty() {
                return model;
            }
            model.find_idx = (model.find_idx + 1) % model.find_matches.len();
            saltar_a_match(&mut model);
        }
        Msg::FindAnterior => {
            if model.find_matches.is_empty() {
                return model;
            }
            let n = model.find_matches.len();
            model.find_idx = (model.find_idx + n - 1) % n;
            saltar_a_match(&mut model);
        }
        Msg::FindClose => {
            model.find_visible = false;
        }
        Msg::DiffToggle => {
            model.solo_activo = !model.solo_activo;
        }
        // Rail hospedado: pata reenvió un diente → selecciona ese diente.
        Msg::HostActivate(id) => {
            model.diente_activo = id as usize;
        }
        Msg::MoverAtomArriba => {
            mover_atom_caret(&mut model, -1);
        }
        Msg::MoverAtomAbajo => {
            mover_atom_caret(&mut model, 1);
        }
        Msg::TocarMadre => {
            tocar_madre(&mut model);
        }
        Msg::RegenerarStale => {
            regenerar_siguiente_stale(&mut model, handle);
        }
        Msg::ToglearFusion => {
            if let Some(idx) = model.ide.junction_antes_del_caret() {
                model.ide.togglear_junction(idx);
            }
        }
        Msg::ZonaSiguiente => {
            model.ide.ir_a_zona_siguiente();
            model.ide.state.ensure_caret_visible(VISIBLE_LINES);
        }
        Msg::ZonaAnterior => {
            model.ide.ir_a_zona_anterior();
            model.ide.state.ensure_caret_visible(VISIBLE_LINES);
        }
        Msg::CicloBackend => {
            cycle_backend(&mut model);
        }
        Msg::PedirTraducir(lengua) => {
            lanzar(&mut model, handle, TrabajoLlm::Traducir(lengua));
        }
        Msg::PedirTono(etiqueta) => {
            lanzar(&mut model, handle, TrabajoLlm::Tono(etiqueta));
        }
        Msg::PedirResumir(palabras) => {
            lanzar(&mut model, handle, TrabajoLlm::Resumir(palabras));
        }
        Msg::LlmListo {
            hija,
            atoms_nuevos,
            carta,
            transformacion,
        } => {
            recibir_hija(&mut model, hija, atoms_nuevos, carta, transformacion);
        }
        Msg::LlmError(s) => {
            eprintln!("pluma-app :: error LLM: {s}");
            model.ultimo_error = Some(s);
            model.en_curso = false;
        }
        Msg::ResizePanel(dx) => {
            model.panel_w = (model.panel_w + dx).clamp(180.0, 460.0);
        }

        // --- Diente Derivar-IA ---
        Msg::PresetInputKey(ev) => {
            model.preset_input.apply_key(&ev);
        }
        Msg::FocusPreset => {
            model.preset_focused = true;
        }
        Msg::DefocusPreset => {
            model.preset_focused = false;
        }
        Msg::CrearAlterno => {
            let prompt = model.preset_input.text().trim().to_string();
            if !prompt.is_empty() {
                lanzar(&mut model, handle, TrabajoLlm::Reescribir(prompt));
            } else {
                model.ultimo_status = "escribí un prompt para derivar".into();
            }
        }
        Msg::GuardarPreset => {
            let prompt = model.preset_input.text().trim().to_string();
            if !prompt.is_empty() && !model.presets.contains(&prompt) {
                model.presets.push(prompt);
                crate::util::guardar_presets(&model.presets);
                model.ultimo_status = format!("preset guardado ({})", model.presets.len());
            }
        }
        Msg::UsarPreset(i) => {
            if let Some(prompt) = model.presets.get(i).cloned() {
                lanzar(&mut model, handle, TrabajoLlm::Reescribir(prompt));
            }
        }
        Msg::BorrarPreset(i) => {
            if i < model.presets.len() {
                model.presets.remove(i);
                crate::util::guardar_presets(&model.presets);
            }
        }

        // --- Menú principal + menú de edición contextual ---
        Msg::MenuOpen(idx) => {
            model.menu_open = idx;
            model.menu_active = usize::MAX;
            model.edit_menu = None;
            if idx.is_some() {
                model.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(handle, motion::FAST, || Msg::MenuTick);
            }
        }
        Msg::CloseMenus => {
            model.menu_open = None;
            model.menu_active = usize::MAX;
            model.edit_menu = None;
            model.edit_active = usize::MAX;
        }
        Msg::EditMenuOpen(x, y) => {
            model.edit_menu = Some((x, y));
            model.edit_active = usize::MAX;
            model.menu_open = None;
            model.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
            animate(handle, motion::FAST, || Msg::MenuTick);
        }
        Msg::EditMenuAction(action) => {
            return aplicar_edit_menu(model, action);
        }
        Msg::MenuCommand(cmd) => {
            return ejecutar_menu_command(model, cmd, handle);
        }
        Msg::MenuNav(dir) => {
            if let Some(mi) = model.menu_open {
                let menu = menu_principal(&model);
                model.menu_active =
                    llimphi_widget_menubar::menubar_nav(&menu, mi, model.menu_active, dir);
            }
        }
        Msg::MenuActivate => {
            if let Some(mi) = model.menu_open {
                let menu = menu_principal(&model);
                if let Some(cmd) =
                    llimphi_widget_menubar::menubar_command_at(&menu, mi, model.menu_active)
                {
                    return ejecutar_menu_command(model, cmd, handle);
                }
            }
        }
        Msg::MenuTick => {}
        Msg::EditNav(dir) => {
            let flags = EditFlags::from_editor(&model.ide.state, false);
            model.edit_active = editmenu::edit_menu_step(flags, model.edit_active, dir);
        }
        Msg::EditActivate => {
            let flags = EditFlags::from_editor(&model.ide.state, false);
            if let Some(action) = editmenu::edit_menu_action_at(flags, model.edit_active) {
                return aplicar_edit_menu(model, action);
            }
        }
    }
    // Acota el scroll horizontal al contenido tras cualquier cambio (selección,
    // tamaño, panel…). Idempotente y barato.
    clamp_scroll(&mut model);
    // Nivela el scroll vertical de los lienzos read-only al del foco, para que
    // las secciones queden alineadas y no se pierdan de vista.
    nivelar_scroll(&mut model);
    model
}

/// Copia el `scroll_offset` vertical del lienzo con foco (`model.ide`) a todos
/// los read-only, clampeado a la última línea de cada uno (si es más corto,
/// queda topado). Mantiene las secciones alineadas entre columnas.
/// Mueve el foco al lienzo siguiente (`dir=1`) o anterior (`dir=-1`) de la
/// selección visible, ciclando. Sin selección o con una sola columna, no-op.
fn mover_foco(model: &mut Model, dir: i32) {
    let n = model.seleccionados.len();
    if n < 2 {
        return;
    }
    let actual = model
        .activo
        .and_then(|a| model.seleccionados.iter().position(|x| *x == a))
        .unwrap_or(0);
    let siguiente = (actual as i32 + dir).rem_euclid(n as i32) as usize;
    let id = model.seleccionados[siguiente];
    cambiar_activo(model, id);
}

fn nivelar_scroll(model: &mut Model) {
    let s = model.ide.state.scroll_offset;
    for ro in model.ides_ro.values_mut() {
        let max = ro.state.line_count().saturating_sub(1);
        ro.state.scroll_offset = s.min(max);
    }
}

/// Acota `scroll_x` a `[0, ancho_contenido - ancho_centro]`. Con ≤1 columna o
/// contenido que cabe entero, queda en 0.
fn clamp_scroll(model: &mut Model) {
    let n = if model.solo_activo {
        model.activo.iter().count()
    } else {
        model.seleccionados.len()
    };
    let contenido = crate::model::ancho_contenido(n);
    let centro = (model.viewport.0 - model.panel_w - crate::model::RAIL_W).max(0.0);
    let max = (contenido - centro).max(0.0);
    model.scroll_x = model.scroll_x.clamp(0.0, max);
}

/// Construye el menú principal de pluma reflejando el estado actual: los
/// ítems de Editar quedan grises cuando no hay selección (Cortar/Copiar) o
/// historial (Deshacer/Rehacer). El editor focuseado es el `cuerpo_ide`
/// central (único editor de texto rico de la app).
pub(crate) fn menu_principal(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};

    let ed = &model.ide.state;
    let has_sel = ed.has_selection();
    let can_undo = ed.can_undo();
    let can_redo = ed.can_redo();

    let mut undo = MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z");
    if !can_undo {
        undo = undo.disabled();
    }
    let mut redo = MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Y");
    if !can_redo {
        redo = redo.disabled();
    }
    let mut cut = MenuItem::new("Cortar", "edit.cut").shortcut("Ctrl+X").separated();
    let mut copy = MenuItem::new("Copiar", "edit.copy").shortcut("Ctrl+C");
    if !has_sel {
        cut = cut.disabled();
        copy = copy.disabled();
    }
    let paste = MenuItem::new("Pegar", "edit.paste").shortcut("Ctrl+V");
    let sel_all = MenuItem::new("Seleccionar todo", "edit.selectall")
        .shortcut("Ctrl+A")
        .separated();

    // El botón de regenerar stale sólo tiene sentido si hay alguna hija
    // stale del activo — lo grisamos cuando no.
    let mut regen = MenuItem::new("Regenerar stale", "llm.regen");
    if contar_stale_del_activo(model) == 0 {
        regen = regen.disabled();
    }

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Nuevo documento", "file.nuevo").shortcut("Ctrl+N"))
                .item(MenuItem::new("Guardar", "file.guardar").shortcut("Ctrl+S"))
                .item(MenuItem::new("Abrir archivo (ruta)", "file.abrir").separated())
                .item(MenuItem::new("Exportar (md/docx)", "file.exportar")),
        )
        .menu(
            Menu::new("Editar")
                .item(undo)
                .item(redo)
                .item(cut)
                .item(copy)
                .item(paste)
                .item(sel_all),
        )
        .menu(
            Menu::new("Buscar")
                .item(MenuItem::new("Buscar en documento", "search.find").shortcut("Ctrl+F")),
        )
        .menu(
            Menu::new("Multilienzo")
                .item(MenuItem::new("Sólo activo / todos", "mult.diff").shortcut("Ctrl+D"))
                .item(MenuItem::new(
                    if model.foco_por_hover {
                        "Foco por hover: ON"
                    } else {
                        "Foco por hover: off"
                    },
                    "mult.hover",
                ))
                .item(MenuItem::new("Foco siguiente", "mult.foco_sig").shortcut("Ctrl+Tab"))
                .item(MenuItem::new("Togglear fusión (zona)", "mult.fusion").shortcut("Ctrl+J"))
                .item(MenuItem::new("Zona siguiente", "mult.zona_sig").separated())
                .item(MenuItem::new("Zona anterior", "mult.zona_ant")),
        )
        .menu(
            Menu::new("LLM")
                .item(MenuItem::new("Ciclar backend", "llm.backend"))
                .item(MenuItem::new("Traducir → qu", "llm.trad_qu"))
                .item(MenuItem::new("Traducir → en", "llm.trad_en"))
                .item(MenuItem::new("Tono formal", "llm.tono"))
                .item(MenuItem::new("Resumir 30p", "llm.resumir"))
                .item(MenuItem::new("Tocar madre", "llm.tocar").separated())
                .item(regen),
        )
        .menu(
            Menu::new("Ayuda")
                .item(MenuItem::new("pluma · editor multilienzo", "help.about").disabled()),
        )
}

/// Traduce el `command` string del menú principal al `Msg` real de la app
/// y lo aplica. Cierra el dropdown antes de actuar. Los comandos `edit.*`
/// se enrutan al menú de edición sobre el `cuerpo_ide`.
fn ejecutar_menu_command(mut model: Model, command: String, handle: &Handle<Msg>) -> Model {
    model.menu_open = None;
    let target = match command.as_str() {
        "file.nuevo" => Some(Msg::NuevoDoc),
        "file.guardar" => Some(Msg::Guardar),
        "file.abrir" => Some(Msg::AbrirArchivo),
        "file.exportar" => Some(Msg::ExportarMd),
        "edit.undo" => Some(Msg::EditMenuAction(EditAction::Undo)),
        "edit.redo" => Some(Msg::EditMenuAction(EditAction::Redo)),
        "edit.cut" => Some(Msg::EditMenuAction(EditAction::Cut)),
        "edit.copy" => Some(Msg::EditMenuAction(EditAction::Copy)),
        "edit.paste" => Some(Msg::EditMenuAction(EditAction::Paste)),
        "edit.selectall" => Some(Msg::EditMenuAction(EditAction::SelectAll)),
        "search.find" => Some(Msg::FindToggle),
        "mult.diff" => Some(Msg::DiffToggle),
        "mult.hover" => Some(Msg::ToggleFocoHover),
        "mult.foco_sig" => Some(Msg::FocoSiguiente),
        "mult.fusion" => Some(Msg::ToglearFusion),
        "mult.zona_sig" => Some(Msg::ZonaSiguiente),
        "mult.zona_ant" => Some(Msg::ZonaAnterior),
        "llm.backend" => Some(Msg::CicloBackend),
        "llm.trad_qu" => Some(Msg::PedirTraducir("qu".into())),
        "llm.trad_en" => Some(Msg::PedirTraducir("en".into())),
        "llm.tono" => Some(Msg::PedirTono("formal".into())),
        "llm.resumir" => Some(Msg::PedirResumir(Some(30))),
        "llm.tocar" => Some(Msg::TocarMadre),
        "llm.regen" => Some(Msg::RegenerarStale),
        _ => None,
    };
    match target {
        Some(msg) => actualizar(model, msg, handle),
        None => model,
    }
}

/// Aplica una acción del menú de edición al `EditorState` del cuerpo_ide,
/// reusando `editmenu::apply` (mismo camino que las teclas de edición).
/// Cierra el menú. Como `apply_key_with_clipboard`, no necesita marcar
/// dirty manual: el `CuerpoIde` deriva el pendiente_sync de su `edit_seq`.
fn aplicar_edit_menu(mut model: Model, action: EditAction) -> Model {
    model.edit_menu = None;
    let _ = llimphi_widget_edit_menu::apply(&mut model.ide.state, action, &mut model.clipboard);
    model.ide.state.ensure_caret_visible(VISIBLE_LINES);
    model
}

fn cambiar_activo(model: &mut Model, id: Uuid) {
    if model.activo == Some(id) {
        return;
    }
    let cuerpo = match model.cuerpos.iter().find(|c| c.id == id) {
        Some(c) => c.clone(),
        None => return,
    };
    model.activo = Some(id);
    // El activo siempre está en la selección visible del multilienzo.
    if !model.seleccionados.contains(&id) {
        model.seleccionados.push(id);
    }
    let idx: HashMap<Uuid, &NarrativeAtom> =
        model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    model.ide.recargar(&cuerpo, &idx);
    model.ultimo_status = format!("doc: {}", cuerpo.metadatos.nombre_legible);
    reconstruir_ides_ro(model);
}

/// Agrega/saca un cuerpo de la selección visible. Nunca deja la selección
/// vacía ni saca el activo sin reasignarlo: al sacar el activo, pasa el
/// foco al primer cuerpo que quede.
fn toggle_seleccion(model: &mut Model, id: Uuid) {
    if let Some(pos) = model.seleccionados.iter().position(|x| *x == id) {
        if model.seleccionados.len() == 1 {
            return; // no dejar el multilienzo sin columnas
        }
        model.seleccionados.remove(pos);
        if model.activo == Some(id) {
            // Reasignar foco al primer cuerpo restante.
            if let Some(&otro) = model.seleccionados.first() {
                model.activo = None; // forzar recarga en cambiar_activo
                cambiar_activo(model, otro);
                return;
            }
        }
    } else if model.cuerpos.iter().any(|c| c.id == id) {
        model.seleccionados.push(id);
    }
    reconstruir_ides_ro(model);
}

/// Reconstruye los editores read-only de los cuerpos seleccionados que no
/// son el activo (el activo vive en `model.ide`, editable). Descarta los
/// que ya no están seleccionados.
fn reconstruir_ides_ro(model: &mut Model) {
    use pluma_editor_llimphi::cuerpo_ide::CuerpoIde;
    let idx: HashMap<Uuid, &NarrativeAtom> =
        model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    let mut nuevos: HashMap<Uuid, CuerpoIde> = HashMap::new();
    for &id in &model.seleccionados {
        if model.activo == Some(id) {
            continue;
        }
        if let Some(cuerpo) = model.cuerpos.iter().find(|c| c.id == id) {
            nuevos.insert(id, CuerpoIde::from_cuerpo(cuerpo, &idx));
        }
    }
    model.ides_ro = nuevos;
}

fn crear_doc_nuevo(model: &mut Model) {
    let ahora = ahora_unix();
    let n = model
        .cuerpos
        .iter()
        .filter(|c| !c.metadatos.intencion.es_derivada())
        .count()
        + 1;
    let atom = NarrativeAtom::new("Empieza a escribir aquí…", "es");
    let mut cuerpo = Cuerpo::nuevo(
        format!("es-{n}"),
        format!("doc #{n} sin título"),
        Intencion::Original,
        ahora,
    );
    cuerpo.agregar(atom.id, ahora);
    let _ = model.store.put_atom(&atom);
    let _ = model.store.put_cuerpo(&cuerpo);
    let _ = model.store.flush();
    let id = cuerpo.id;
    model.atoms.insert(atom.id, atom);
    model.cuerpos.push(cuerpo);
    cambiar_activo(model, id);
    model.ultimo_status = format!("doc #{n} creado");
}

fn guardar_activo(model: &mut Model) {
    let Some(activo_id) = model.activo else {
        model.ultimo_status = "sin doc activo".into();
        return;
    };
    let idx: HashMap<Uuid, &NarrativeAtom> =
        model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    let cambios = model.ide.diff(&idx);
    drop(idx);

    if cambios.is_empty() {
        model.ultimo_status = "sin cambios".into();
        return;
    }

    let mut creados: Vec<Uuid> = Vec::new();
    let branch_id = model
        .cuerpos
        .iter()
        .find(|c| c.id == activo_id)
        .map(|c| c.branch_id.clone())
        .unwrap_or_else(|| "es".to_string());

    for c in &cambios {
        match c {
            CambioAtom::Mutar { id, texto_nuevo } => {
                if let Some(a) = model.atoms.get_mut(id) {
                    a.set_content(texto_nuevo.as_str());
                    let _ = model.store.put_atom(a);
                }
            }
            CambioAtom::Crear { texto, posicion: _ } => {
                let atom = NarrativeAtom::new(texto.as_str(), &branch_id);
                let id = atom.id;
                let _ = model.store.put_atom(&atom);
                model.atoms.insert(id, atom);
                creados.push(id);
            }
            CambioAtom::Eliminar { id } => {
                model.atoms.remove(id);
                // El sled mantiene el atom histórico — no lo borramos
                // del backend porque hijas/cartas pueden seguir apuntando
                // a él. La memoria local sí lo descarta.
            }
        }
    }

    model.ide.aplicar_cambios(&cambios, &creados);

    // Reconstruir `cuerpo.orden` con el orden nuevo del IDE.
    let nuevo_orden: Vec<Uuid> = model.ide.editor_cuerpo.atom_ids.clone();
    if let Some(c) = model.cuerpos.iter_mut().find(|c| c.id == activo_id) {
        let ahora = c.metadatos.modificado_en.saturating_add(1);
        let viejo: Vec<Uuid> = c.orden.clone();
        for id in &viejo {
            let _ = c.remover(*id, ahora);
        }
        for id in &nuevo_orden {
            c.agregar(*id, ahora);
        }
        let _ = model.store.put_cuerpo(c);
    }
    let _ = model.store.flush();

    let n_mut = cambios
        .iter()
        .filter(|c| matches!(c, CambioAtom::Mutar { .. }))
        .count();
    let n_new = creados.len();
    let n_del = cambios
        .iter()
        .filter(|c| matches!(c, CambioAtom::Eliminar { .. }))
        .count();
    model.ultimo_status = format!("guardado: {n_mut} mut · {n_new} crear · {n_del} del");
}

/// Recalcula las posiciones (línea, col) donde aparece el query en el
/// buffer actual. Búsqueda case-insensitive, substring. Llamarlo cada
/// vez que el query o el texto cambian. Reset de `find_idx` al primer
/// match cuando hay alguno; lo deja en 0 si no hay (consistente con
/// "0 de 0"), pero la UI no salta si está vacío.
fn recomputar_matches(model: &mut Model) {
    let query = model.find_input.text();
    if query.is_empty() {
        model.find_matches.clear();
        model.find_idx = 0;
        return;
    }
    let q_lower = query.to_lowercase();
    let mut matches: Vec<(usize, usize)> = Vec::new();
    let texto = model.ide.texto_buffer();
    for (line_idx, linea) in texto.lines().enumerate() {
        let l_lower = linea.to_lowercase();
        let mut start = 0;
        while let Some(pos) = l_lower[start..].find(&q_lower) {
            let col = start + pos;
            matches.push((line_idx, col));
            start = col + q_lower.len().max(1);
            if start >= l_lower.len() {
                break;
            }
        }
    }
    model.find_matches = matches;
    if model.find_idx >= model.find_matches.len() {
        model.find_idx = 0;
    }
}

fn saltar_a_match(model: &mut Model) {
    let Some(&(line, col)) = model.find_matches.get(model.find_idx) else {
        return;
    };
    model.ide.set_caret(line, col);
    model.ide.state.ensure_caret_visible(VISIBLE_LINES);
}

/// Avanza `modificado_en` del cuerpo activo a la hora actual. Cualquier
/// hija derivada cuyo `regenerada_en` sea anterior se vuelve `es_stale`
/// y aparece en el botón «regenerar stale (N)». Caso de uso típico:
/// editaste la madre sin querer recordar todos los detalles y querés
/// invalidar las derivadas para que vuelvan a salir del LLM.
fn tocar_madre(model: &mut Model) {
    let Some(activo_id) = model.activo else {
        model.ultimo_status = "sin doc activo".into();
        return;
    };
    let ahora = ahora_unix();
    if let Some(c) = model.cuerpos.iter_mut().find(|c| c.id == activo_id) {
        c.metadatos.modificado_en = ahora;
        let _ = model.store.put_cuerpo(c);
    }
    let _ = model.store.flush();
    let n = contar_stale_del_activo(model);
    model.ultimo_status = format!("madre tocada — {n} hija(s) ahora stale");
    model.ultimo_error = None;
}

pub(crate) fn contar_stale_del_activo(model: &Model) -> usize {
    let Some(activo_id) = model.activo else {
        return 0;
    };
    let Some(madre) = model.cuerpos.iter().find(|c| c.id == activo_id) else {
        return 0;
    };
    let modif = madre.metadatos.modificado_en;
    model
        .cuerpos
        .iter()
        .filter(|c| {
            c.metadatos.derivado_de == Some(activo_id) && c.es_derivado() && c.es_stale(modif)
        })
        .count()
}

/// Encuentra la primera hija del activo que sea stale, busca la
/// `Transformacion` original registrada (madre==activo, hija==hija_id),
/// traduce su `TipoTransformacion` a un `TrabajoLlm`, y lo lanza con la
/// madre actualizada — el ejecutor produce una hija nueva fresca; la
/// vieja queda en el modelo (sigue visible por si querés diff). Lo
/// hacemos hija-por-hija (no batch) para que el progreso sea visible
/// y un error no aborte todas.
fn regenerar_siguiente_stale(model: &mut Model, handle: &Handle<Msg>) {
    if model.en_curso {
        model.ultimo_status = "LLM ocupado — esperá".into();
        return;
    }
    let Some(activo_id) = model.activo else {
        model.ultimo_status = "sin doc activo".into();
        return;
    };
    let madre_modif = match model.cuerpos.iter().find(|c| c.id == activo_id) {
        Some(c) => c.metadatos.modificado_en,
        None => return,
    };
    let hija_id_opt = model
        .cuerpos
        .iter()
        .find(|c| {
            c.metadatos.derivado_de == Some(activo_id)
                && c.es_derivado()
                && c.es_stale(madre_modif)
        })
        .map(|c| c.id);
    let Some(hija_id) = hija_id_opt else {
        model.ultimo_status = "no hay hijas stale — tocar madre primero".into();
        return;
    };
    // Buscar la Transformacion original. Prioridad: la del store (en
    // memoria está cargada al iniciar; el sled es la fuente de verdad).
    let tipo = model
        .transformaciones
        .iter()
        .find(|t| t.madre == activo_id && t.hija == hija_id)
        .map(|t| t.tipo.clone());
    let Some(tipo) = tipo else {
        model.ultimo_status = format!(
            "no se halló transformación para regenerar {hija_id} — falta historial"
        );
        return;
    };
    let Some(trabajo) = trabajo_de_tipo(&tipo) else {
        model.ultimo_status = format!("tipo {tipo:?} no es regenerable automáticamente");
        return;
    };
    lanzar(model, handle, trabajo);
}

/// Traduce un `TipoTransformacion` persistido al `TrabajoLlm` que
/// `lanzar` sabe correr. `Identidad`/`Reescribir`/`Custom` no son
/// auto-regenerables — Reescribir necesita prompt humano, Custom Rhai,
/// Identidad no aporta nada nuevo.
fn trabajo_de_tipo(t: &TipoTransformacion) -> Option<TrabajoLlm> {
    match t {
        TipoTransformacion::Traducir { lengua_destino } => {
            Some(TrabajoLlm::Traducir(lengua_destino.clone()))
        }
        TipoTransformacion::Tono { etiqueta } => Some(TrabajoLlm::Tono(etiqueta.clone())),
        TipoTransformacion::Resumir { palabras_objetivo } => {
            Some(TrabajoLlm::Resumir(*palabras_objetivo))
        }
        _ => None,
    }
}

/// Mueve el átomo donde está el caret una posición arriba (`delta=-1`)
/// o abajo (`delta=1`). Sincroniza el buffer al modelo antes de
/// reordenar (para no perder ediciones pendientes), muta `cuerpo.orden`,
/// persiste, y recarga el IDE — junctions resetean a separadores (es
/// el costo del reorder; el usuario las re-fusiona si las quería).
/// El caret queda en la primera línea del átomo movido.
fn mover_atom_caret(model: &mut Model, delta: i32) {
    let Some(activo_id) = model.activo else {
        return;
    };
    // Sincroniza pendientes para no perderlos al recargar.
    guardar_activo(model);

    let (caret_line, _) = model.ide.caret();
    let Some(atom_id) = model.ide.atom_id_en_linea(caret_line) else {
        return;
    };
    let cuerpo = match model.cuerpos.iter_mut().find(|c| c.id == activo_id) {
        Some(c) => c,
        None => return,
    };
    let n = cuerpo.orden.len();
    if n < 2 {
        return;
    }
    let i = match cuerpo.orden.iter().position(|x| *x == atom_id) {
        Some(i) => i,
        None => return,
    };
    let j = if delta < 0 {
        if i == 0 {
            return;
        }
        i - 1
    } else {
        if i + 1 >= n {
            return;
        }
        i + 1
    };
    cuerpo.orden.swap(i, j);
    cuerpo.metadatos.modificado_en = cuerpo.metadatos.modificado_en.saturating_add(1);
    let _ = model.store.put_cuerpo(cuerpo);
    let _ = model.store.flush();

    // Recargar el IDE con el orden nuevo. Snapshot la cuerpo data
    // primero para evitar el borrow simultáneo del index.
    let cuerpo_clon = cuerpo.clone();
    // Liberamos el préstamo mutable de `model.cuerpos` antes de
    // tomar uno inmutable de `model.atoms` para construir el índice.
    let _ = cuerpo;
    let idx: HashMap<Uuid, &NarrativeAtom> =
        model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    model.ide.recargar(&cuerpo_clon, &idx);
    drop(idx);

    // Posicionar el caret al inicio del átomo movido. Su nuevo idx es
    // `j`; sumamos lineas anteriores (cada atom = 1 + atoms_extra_lineas
    // + separador). Más simple: usar posicion_de_atom.
    if let Some((line, col)) = model.ide.posicion_de_atom(atom_id) {
        model.ide.set_caret(line, col);
        model.ide.state.ensure_caret_visible(VISIBLE_LINES);
    }

    model.ultimo_status = format!(
        "atom movido {}",
        if delta < 0 { "↑" } else { "↓" }
    );
    model.ultimo_error = None;
}

fn abrir_archivo(model: &mut Model) {
    let path_raw = model.path_input.text().trim().to_string();
    if path_raw.is_empty() {
        model.ultimo_error = Some("ruta vacía".into());
        return;
    }
    let path = expandir_ruta(&path_raw);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            model.ultimo_error = Some(format!("leyendo {path:?}: {e}"));
            return;
        }
    };
    let nombre = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archivo".to_string());
    let ahora = ahora_unix();

    let importado = if extension_lower(&path) == Some("docx".to_string()) {
        match foreign_docx::parse_docx(&bytes, "es", nombre.clone(), ahora) {
            Ok(imp) => (imp.cuerpo, imp.atoms),
            Err(e) => {
                model.ultimo_error = Some(format!("parse_docx {nombre}: {e:?}"));
                return;
            }
        }
    } else if extension_lower(&path) == Some("md".to_string())
        || extension_lower(&path) == Some("markdown".to_string())
        || extension_lower(&path) == Some("txt".to_string())
    {
        let texto = match std::str::from_utf8(&bytes) {
            Ok(s) => s.to_string(),
            Err(e) => {
                model.ultimo_error = Some(format!("{nombre} no es UTF-8: {e}"));
                return;
            }
        };
        let imp = pluma_md::parse_md(&texto, "es", nombre.clone(), ahora);
        (imp.cuerpo, imp.atoms)
    } else {
        model.ultimo_error = Some(format!(
            "extensión no soportada en {nombre} — usá .md o .docx"
        ));
        return;
    };

    let (cuerpo, atoms_nuevos) = importado;
    if atoms_nuevos.is_empty() {
        model.ultimo_error = Some(format!("{nombre} no produjo átomos"));
        return;
    }
    for a in &atoms_nuevos {
        let _ = model.store.put_atom(a);
        model.atoms.insert(a.id, a.clone());
    }
    let _ = model.store.put_cuerpo(&cuerpo);
    let _ = model.store.flush();
    let id = cuerpo.id;
    let n = atoms_nuevos.len();
    model.cuerpos.push(cuerpo);
    model.ultimo_status = format!("abierto «{nombre}»: {n} átomos");
    model.ultimo_error = None;
    cambiar_activo(model, id);
}

fn exportar_md(model: &mut Model) {
    let Some(activo_id) = model.activo else {
        model.ultimo_error = Some("sin doc activo".into());
        return;
    };
    let path_raw = model.path_input.text().trim().to_string();
    if path_raw.is_empty() {
        model.ultimo_error = Some("ruta vacía".into());
        return;
    }
    let path = expandir_ruta(&path_raw);
    let Some(cuerpo) = model.cuerpos.iter().find(|c| c.id == activo_id) else {
        model.ultimo_error = Some("doc activo desapareció".into());
        return;
    };

    let ext = extension_lower(&path).unwrap_or_default();
    let bytes: Vec<u8> = if ext == "docx" {
        match foreign_docx::write_docx(cuerpo, &model.atoms) {
            Ok(b) => b,
            Err(e) => {
                model.ultimo_error = Some(format!("write_docx: {e}"));
                return;
            }
        }
    } else if ext.is_empty() || ext == "md" || ext == "markdown" || ext == "txt" {
        let md = pluma_md::to_md(cuerpo, &model.atoms);
        if md.is_empty() {
            model.ultimo_error = Some("doc vacío — nada que exportar".into());
            return;
        }
        md.into_bytes()
    } else {
        model.ultimo_error = Some(format!(
            "extensión .{ext} no soportada — usá .md o .docx"
        ));
        return;
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, &bytes) {
        Ok(()) => {
            model.ultimo_status = format!(
                "exportado «{}» a {} ({} bytes)",
                cuerpo.metadatos.nombre_legible,
                path.display(),
                bytes.len(),
            );
            model.ultimo_error = None;
        }
        Err(e) => {
            model.ultimo_error = Some(format!("escribiendo {path:?}: {e}"));
        }
    }
}

fn cycle_backend(model: &mut Model) {
    let total = BACKENDS.len();
    for step in 1..=total {
        let try_idx = (model.backend_idx + step) % total;
        let kind = BACKENDS[try_idx];
        match build_client(&LlmConfig {
            kind,
            ..Default::default()
        }) {
            Ok(c) => {
                model.chat = c;
                model.backend_idx = try_idx;
                model.ultimo_status = format!("backend → {}", etiqueta_backend(kind));
                model.ultimo_error = None;
                return;
            }
            Err(e) => {
                model.ultimo_error = Some(format!("backend {kind:?}: {e}"));
            }
        }
    }
    // Si todos fallaron (no debería: Mock siempre funciona), no-op.
}

fn recibir_hija(
    model: &mut Model,
    hija: Cuerpo,
    atoms_nuevos: Vec<NarrativeAtom>,
    carta: CartaHebras,
    transformacion: Transformacion,
) {
    for a in &atoms_nuevos {
        let _ = model.store.put_atom(a);
        model.atoms.insert(a.id, a.clone());
    }
    let _ = model.store.put_cuerpo(&hija);
    let _ = model.store.put_carta(&carta);
    let _ = model.store.put_transformacion(&transformacion);
    let _ = model.store.flush();
    let hija_id = hija.id;
    let nombre = hija.metadatos.nombre_legible.clone();
    model.cuerpos.push(hija);
    model.cartas.push(carta);
    model.transformaciones.push(transformacion);
    model.en_curso = false;
    model.ultimo_status = format!("hija «{nombre}» derivada");
    cambiar_activo(model, hija_id);
}

// ---------------------------------------------------------------------
// Trabajo LLM
// ---------------------------------------------------------------------

pub(crate) enum TrabajoLlm {
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
    /// Reescritura libre dictada por un prompt humano (diente Derivar-IA).
    Reescribir(String),
}

fn lanzar(model: &mut Model, handle: &Handle<Msg>, trabajo: TrabajoLlm) {
    if model.en_curso {
        return;
    }
    let Some(activo_id) = model.activo else {
        model.ultimo_status = "sin doc activo".into();
        return;
    };
    // Sincronizar antes de transformar — si el usuario tipeó sin Ctrl+S,
    // queremos que el LLM vea el texto editado.
    guardar_activo(model);

    let madre = match model.cuerpos.iter().find(|c| c.id == activo_id) {
        Some(c) => c.clone(),
        None => {
            model.ultimo_error = Some("doc activo desapareció".into());
            return;
        }
    };
    if madre.orden.is_empty() {
        model.ultimo_status = "madre vacía — nada que transformar".into();
        return;
    }

    let atoms_owned: Vec<NarrativeAtom> = model.atoms.values().cloned().collect();
    let chat = model.chat.clone();
    let h = handle.clone();
    let ahora = ahora_unix();

    model.en_curso = true;
    model.ultimo_error = None;
    model.ultimo_status = format!("LLM en curso ({} backend)", etiqueta_backend(BACKENDS[model.backend_idx]));

    handle.spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => return Msg::LlmError(format!("runtime tokio: {e}")),
        };
        let idx: HashMap<Uuid, &NarrativeAtom> =
            atoms_owned.iter().map(|a| (a.id, a)).collect();

        let resultado = rt.block_on(async {
            match trabajo {
                TrabajoLlm::Traducir(lengua) => {
                    let ej = EjecutorTraducirLlm::from_arc(chat, lengua.clone());
                    let t = Transformacion::nueva(
                        madre.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Traducir {
                            lengua_destino: lengua,
                        },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora)
                        .await
                        .map(|p| (p, t))
                }
                TrabajoLlm::Tono(etiq) => {
                    let ej = EjecutorTonoLlm::from_arc(chat, etiq.clone());
                    let t = Transformacion::nueva(
                        madre.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Tono { etiqueta: etiq },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora)
                        .await
                        .map(|p| (p, t))
                }
                TrabajoLlm::Resumir(palabras) => {
                    let ej = EjecutorResumirLlm::from_arc(chat, palabras);
                    let t = Transformacion::nueva(
                        madre.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Resumir {
                            palabras_objetivo: palabras,
                        },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora)
                        .await
                        .map(|p| (p, t))
                }
                TrabajoLlm::Reescribir(prompt) => {
                    let ej = EjecutorReescribirLlm::from_arc(chat, prompt.clone());
                    let t = Transformacion::nueva(
                        madre.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Reescribir { prompt },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora)
                        .await
                        .map(|p| (p, t))
                }
            }
        });

        let _ = h;
        match resultado {
            Ok((prod, transformacion)) => Msg::LlmListo {
                hija: prod.hija,
                atoms_nuevos: prod.atoms_nuevos,
                carta: prod.carta,
                transformacion,
            },
            Err(e) => Msg::LlmError(format!("{e:?}")),
        }
    });
}
