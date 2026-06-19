//! Lógica de actualización del bucle Elm: el `match` central, las
//! mutaciones del modelo (abrir/crear/guardar/mover/regenerar), el
//! find-in-page, y el trabajo LLM lanzado en un thread aparte.

use std::collections::HashMap;
use std::sync::Arc;

use llimphi_motion::{animate, motion, Tween};
use llimphi_ui::{DragPhase, Handle, Key, NamedKey};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_text_editor::{EditorState, PointerEvent};
use llimphi_widget_text_input::TextInputState;
use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_deck_core::{Recorrido, Rect as DeckRect};
use pluma_deck_outline::recorrido_desde_cuerpo;
use pluma_editor_cuerpo::CambioAtom;
use pluma_llm::{build_client, LlmConfig};
use pluma_transform::{TipoTransformacion, Transformacion};
use pluma_transform_llm::{
    EjecutorReescribirLlm, EjecutorResumirLlm, EjecutorTonoLlm, EjecutorTraducirLlm,
};
use rimay_verbo_core::Provider;
use rimay_verbo_daemon::DaemonClient;
use uuid::Uuid;

use crate::model::{Filtro, Modo, Model, Msg, NodoFiltro, BACKENDS, METRICS, VISIBLE_LINES};
use crate::util::{ahora_unix, etiqueta_backend, expandir_ruta, extension_lower};
use crate::view::etiqueta_filtro;

pub fn actualizar(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
    match msg {
        Msg::EditorKey(ev) => {
            // Disparo reactivo: `Ctrl+Enter` (en cualquier lado) o `Enter` al
            // final del último párrafo regeneran el haz del activo. Ctrl+Enter
            // NO inserta salto de línea; el Enter-al-final sí (comportamiento
            // normal del editor) y además dispara.
            let es_enter = matches!(ev.key, Key::Named(NamedKey::Enter));
            let ctrl = ev.modifiers.ctrl || ev.modifiers.meta;
            let disparar = es_enter && (ctrl || caret_al_final(&model.ide.state));
            if !(disparar && ctrl) {
                let _ = model.ide.apply_key_with_clipboard(&ev, &mut model.clipboard);
            }
            if disparar {
                disparar_regen_reactivo(&mut model, handle);
            }
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
                    let (line, col) = METRICS.screen_to_pos(x, y, scroll);
                    // Click simple = caret; doble = palabra; triple = párrafo.
                    model.ide.state.register_click(line, col);
                }
                PointerEvent::Drag {
                    initial_x,
                    initial_y,
                    dx,
                    dy,
                } => {
                    // El widget ancla en el press y extiende; sin acumular acá.
                    model
                        .ide
                        .state
                        .pointer_drag(METRICS, (initial_x, initial_y), dx, dy);
                }
            }
        }
        Msg::AbrirDoc(id) => {
            cambiar_activo(&mut model, id);
        }
        Msg::ToggleSeleccion(id) => {
            toggle_seleccion(&mut model, id);
        }
        Msg::ReordenarLienzo(desde, hasta) => {
            let n = model.orden_lienzos.len();
            if desde < n && hasta < n && desde != hasta {
                let id = model.orden_lienzos.remove(desde);
                let ins = hasta.min(model.orden_lienzos.len());
                model.orden_lienzos.insert(ins, id);
            }
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
        Msg::CaretBlink => {
            // Sólo titila el caret del lienzo con foco; los read-only no.
            model.ide.state.blink_toggle();
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
        Msg::HijaEnLugar {
            vieja,
            hija,
            atoms_nuevos,
            carta,
            transformacion,
        } => {
            recibir_hija_en_lugar(&mut model, vieja, hija, atoms_nuevos, carta, transformacion);
            // Cascada: el próximo eslabón stale del haz (traducción → resumen…).
            avanzar_reactivo(&mut model, handle);
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

        // --- Diente Grafo: grafo semántico de filtros ---
        Msg::GrafoInputKey(ev) => {
            model.grafo_input.apply_key(&ev);
        }
        Msg::FocusGrafo => {
            model.grafo_input_focused = true;
        }
        Msg::DefocusGrafo => {
            model.grafo_input_focused = false;
        }
        Msg::GrafoAdd(filtro) => {
            // Pipeline vertical (cabe en el sidebar angosto): fuente arriba,
            // filtros apilados hacia abajo, sumidero al final.
            const PASO: f32 = 70.0;
            let i = model.grafo.len();
            let x = model.grafo_src.0;
            let y = model.grafo_src.1 + (i as f32 + 1.0) * PASO;
            model.grafo.push(NodoFiltro { filtro, x, y });
            let n = model.grafo.len();
            model.grafo_sink = (x, model.grafo_src.1 + (n as f32 + 1.0) * PASO);
            // Limpiar el input tras agregar (sobre todo para Concepto).
            model.grafo_input = TextInputState::new();
            model.grafo_input_focused = false;
        }
        Msg::GrafoDel(id) => {
            let idx = (id as usize).saturating_sub(1);
            if id >= 1 && idx < model.grafo.len() {
                model.grafo.remove(idx);
            }
        }
        Msg::GrafoDrag(id, phase, dx, dy) => {
            if matches!(phase, DragPhase::Move) {
                let n = model.grafo.len() as u32;
                if id == 0 {
                    model.grafo_src.0 += dx;
                    model.grafo_src.1 += dy;
                } else if id == n + 1 {
                    model.grafo_sink.0 += dx;
                    model.grafo_sink.1 += dy;
                } else if let Some(nf) = model.grafo.get_mut((id - 1) as usize) {
                    nf.x += dx;
                    nf.y += dy;
                }
            }
        }
        Msg::GrafoLimpiar => {
            model.grafo.clear();
            model.ultimo_status = "grafo vacío".into();
        }
        Msg::GenerarLinea => {
            generar_linea(&mut model, handle);
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

        // --- Unificación: modos Lienzos / Presentar / Plano ---
        Msg::CicloModo => {
            cerrar_edicion_lienzo(&mut model);
            model.modo = model.modo.siguiente();
            model.ultimo_status = format!("modo: {}", model.modo.etiqueta());
            if model.modo == Modo::Presentar {
                posicionar_presentar(&mut model);
            }
        }
        Msg::SetModo(m) => {
            cerrar_edicion_lienzo(&mut model);
            model.modo = m;
            model.ultimo_status = format!("modo: {}", model.modo.etiqueta());
            if model.modo == Modo::Presentar {
                posicionar_presentar(&mut model);
            }
        }
        Msg::LienzoSelect(atom) => {
            iniciar_edicion_lienzo(&mut model, atom);
        }
        Msg::LienzoEditKey(ev) => {
            if let Some((_, state)) = model.editando.as_mut() {
                state.apply_key(&ev);
                state.ensure_caret_visible(80);
            }
        }
        Msg::LienzoEditPointer(ev) => {
            if let Some((_, state)) = model.editando.as_mut() {
                let scroll = state.scroll_offset;
                match ev {
                    PointerEvent::Click { x, y } => {
                        let (l, c) = METRICS.screen_to_pos(x, y, scroll);
                        // Click simple = caret; doble = palabra; triple = párrafo.
                        state.register_click(l, c);
                    }
                    PointerEvent::Drag {
                        initial_x,
                        initial_y,
                        dx,
                        dy,
                    } => {
                        state.pointer_drag(METRICS, (initial_x, initial_y), dx, dy);
                    }
                }
            }
        }
        Msg::LienzoCommit => {
            cerrar_edicion_lienzo(&mut model);
        }
        Msg::PresSiguiente => {
            navegar_presentar(&mut model, handle, 1);
        }
        Msg::PresAnterior => {
            navegar_presentar(&mut model, handle, -1);
        }
        Msg::PresVistaGeneral => {
            let rec = recorrido_actual(&model);
            let panel = panel_presentar(&model);
            if model.recorrido_state.vista_general(&rec, panel) {
                arrancar_vuelo(handle);
            }
        }
        Msg::PresTick => {
            // Avanza la interpolación del vuelo de cámara (~60fps).
            model.recorrido_state.avanzar(0.016);
        }
        Msg::LienzosScroll(notches) => {
            const PX_POR_NOTCH: f32 = 60.0;
            model.lienzos_scroll_y =
                (model.lienzos_scroll_y - notches * PX_POR_NOTCH).max(0.0);
        }
        Msg::EjecutarLienzo(atom) => {
            ejecutar_celda(&mut model, handle, atom);
        }
        Msg::LienzoSalida { atom, texto } => {
            model.salidas.insert(atom, texto);
            model.en_curso = false;
            model.ultimo_status = "celda ejecutada".into();
        }
    }
    // Acota el scroll horizontal al contenido tras cualquier cambio (selección,
    // tamaño, panel…). Idempotente y barato.
    clamp_scroll(&mut model);
    // Nivela el scroll vertical de los lienzos read-only al del foco, para que
    // las secciones queden alineadas y no se pierdan de vista.
    nivelar_scroll(&mut model);
    // Cualquier cuerpo nuevo (creado/derivado/importado) entra al orden del tree.
    let faltan: Vec<Uuid> = model
        .cuerpos
        .iter()
        .map(|c| c.id)
        .filter(|id| !model.orden_lienzos.contains(id))
        .collect();
    model.orden_lienzos.extend(faltan);
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
            Menu::new("Vista")
                .item(MenuItem::new("Ciclar modo", "vista.ciclo").shortcut("Ctrl+M"))
                .item(
                    MenuItem::new(
                        if model.modo == Modo::Lienzos {
                            "● Lienzos (jerárquico)"
                        } else {
                            "Lienzos (jerárquico)"
                        },
                        "vista.lienzos",
                    )
                    .separated(),
                )
                .item(MenuItem::new(
                    if model.modo == Modo::Presentar {
                        "● Presentar (deck)"
                    } else {
                        "Presentar (deck)"
                    },
                    "vista.presentar",
                ))
                .item(MenuItem::new(
                    if model.modo == Modo::Plano {
                        "● Plano (editor clásico)"
                    } else {
                        "Plano (editor clásico)"
                    },
                    "vista.plano",
                )),
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
        "vista.ciclo" => Some(Msg::CicloModo),
        "vista.lienzos" => Some(Msg::SetModo(Modo::Lienzos)),
        "vista.presentar" => Some(Msg::SetModo(Modo::Presentar)),
        "vista.plano" => Some(Msg::SetModo(Modo::Plano)),
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

// ---------------------------------------------------------------------
// Unificación: edición in-situ (modo Lienzos) + navegación (modo Presentar)
// ---------------------------------------------------------------------

/// Empieza la edición in-situ del átomo `atom`: hace activo su cuerpo y abre un
/// editor cargado con su texto. Si ya había una edición abierta, la cierra
/// guardando primero.
fn iniciar_edicion_lienzo(model: &mut Model, atom: Uuid) {
    cerrar_edicion_lienzo(model);
    if let Some(c) = model.cuerpos.iter().find(|c| c.orden.contains(&atom)) {
        let id = c.id;
        cambiar_activo(model, id);
    }
    let texto = model
        .atoms
        .get(&atom)
        .map(|a| a.content.to_string())
        .unwrap_or_default();
    let mut state = EditorState::new();
    state.set_text(&texto);
    model.editando = Some((atom, state));
}

/// Cierra la edición in-situ guardando el texto en el átomo (y persistiendo).
/// La jerarquía se re-deriva sola en el próximo render: si el `#` cambió, el
/// átomo cambia de nivel y la caja se re-anida. No-op si no se editaba nada.
fn cerrar_edicion_lienzo(model: &mut Model) {
    let Some((atom_id, state)) = model.editando.take() else {
        return;
    };
    let nuevo = state.text();
    let mut cambio = false;
    if let Some(a) = model.atoms.get_mut(&atom_id) {
        if a.content.as_str() != nuevo.as_str() {
            a.set_content(nuevo.as_str());
            let _ = model.store.put_atom(a);
            let _ = model.store.flush();
            cambio = true;
        }
    }
    if cambio {
        refrescar_ides(model);
    }
}

/// Recarga el editor activo y los read-only desde los atoms actuales — para que
/// el modo Plano refleje un cambio hecho in-situ en modo Lienzos.
fn refrescar_ides(model: &mut Model) {
    if let Some(id) = model.activo {
        if let Some(cuerpo) = model.cuerpos.iter().find(|c| c.id == id).cloned() {
            let idx: HashMap<Uuid, &NarrativeAtom> =
                model.atoms.iter().map(|(k, v)| (*k, v)).collect();
            model.ide.recargar(&cuerpo, &idx);
        }
    }
    reconstruir_ides_ro(model);
}

/// Construye el recorrido (deck) del cuerpo activo a partir de su árbol de
/// secciones. Vacío si no hay activo.
fn recorrido_actual(model: &Model) -> Recorrido {
    match model.activo.and_then(|a| model.cuerpos.iter().find(|c| c.id == a)) {
        Some(c) => recorrido_desde_cuerpo(c, |id| {
            model.atoms.get(&id).map(|a| a.content.to_string())
        }),
        None => Recorrido::new(),
    }
}

/// El rectángulo del panel donde se pinta el recorrido. Usa el que registró el
/// último frame (`panel_actual`); si todavía no se pintó, lo aproxima desde el
/// viewport (panel del diente + rail a la izquierda, menubar+status arriba).
fn panel_presentar(model: &Model) -> DeckRect {
    if let Some(r) = pluma_deck_recorrido_llimphi::panel_actual() {
        return r;
    }
    let left = model.panel_w + crate::model::RAIL_W as f32;
    let top = 60.0_f32;
    let w = (model.viewport.0 - left).max(1.0);
    let h = (model.viewport.1 - top).max(1.0);
    DeckRect::new(left as f64, top as f64, w as f64, h as f64)
}

/// Vuela `dir` pasos en el modo Presentar (cámara animada vía `ir_a_paso` +
/// ticks `PresTick`).
fn navegar_presentar(model: &mut Model, handle: &Handle<Msg>, dir: i32) {
    let rec = recorrido_actual(model);
    let n = rec.n_pasos();
    if n == 0 {
        return;
    }
    let panel = panel_presentar(model);
    let actual = model.recorrido_state.paso as i64;
    let nuevo = (actual + dir as i64).clamp(0, n as i64 - 1) as usize;
    if nuevo == model.recorrido_state.paso && model.recorrido_state.paso == actual as usize {
        // Ya en el extremo: nada que volar.
        if (dir > 0 && actual as usize + 1 >= n) || (dir < 0 && actual == 0) {
            return;
        }
    }
    model.recorrido_state.ir_a_paso(&rec, nuevo, panel);
    arrancar_vuelo(handle);
}

/// Dispara los ticks que animan el vuelo de cámara durante ~la duración de un
/// paso del deck (DURACION_PASO_S ≈ 0.8 s).
fn arrancar_vuelo(handle: &Handle<Msg>) {
    animate(handle, std::time::Duration::from_millis(850), || Msg::PresTick);
}

/// Ejecuta un lienzo-celda (notebook embebido): según el lenguaje del fence
/// (` ```lang `) corre su cuerpo con el kernel correspondiente — `llm` sobre el
/// `model.chat` ya configurado (igual que las transformaciones), `python`/`py`
/// con RustPython, `wasm`/`wat` con wasmi — y guarda la salida. Async en thread.
fn ejecutar_celda(model: &mut Model, handle: &Handle<Msg>, atom: Uuid) {
    use pluma_editor_llimphi::lienzos::{celda, lang_soportado};
    if model.en_curso {
        return;
    }
    // Guardar cualquier edición in-situ para correr el texto más reciente.
    cerrar_edicion_lienzo(model);
    let texto = match model.atoms.get(&atom) {
        Some(a) => a.content.to_string(),
        None => return,
    };
    let Some((lang, body)) = celda(&texto) else {
        model.ultimo_status = "no es una celda ```lang".into();
        return;
    };
    if body.is_empty() {
        model.ultimo_status = "celda vacía — nada que ejecutar".into();
        return;
    }
    if !lang_soportado(&lang) {
        model.ultimo_status = format!("sin kernel para '{lang}' (llm/python/wasm)");
        return;
    }
    let chat = model.chat.clone();
    model.en_curso = true;
    model.ultimo_error = None;
    model.ultimo_status = format!("ejecutando celda {lang}…");
    handle.spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                return Msg::LienzoSalida {
                    atom,
                    texto: format!("error runtime: {e}"),
                }
            }
        };
        let texto = rt.block_on(async move {
            match lang.as_str() {
                "llm" => {
                    let req = pluma_llm_core::ChatRequest::una_vuelta(body, 512);
                    match chat.complete(&req).await {
                        Ok(r) => r.content,
                        Err(e) => format!("error: {e}"),
                    }
                }
                "python" | "py" => {
                    corre_kernel(pluma_notebook_kernel_python::PythonKernel::new(), &body, "python")
                        .await
                }
                "wasm" | "wat" => {
                    corre_kernel(pluma_notebook_kernel_wasm::WasmKernel::new(), &body, "wat").await
                }
                otro => format!("sin kernel para '{otro}'"),
            }
        });
        Msg::LienzoSalida { atom, texto }
    });
}

/// Corre una celda en un kernel y reduce su salida a texto (stdout, o el value).
async fn corre_kernel<K: pluma_notebook_exec::Kernel>(k: K, body: &str, lang: &str) -> String {
    match k.execute(body, lang).await {
        Ok(out) => {
            let stdout = out.stdout.trim();
            if !stdout.is_empty() {
                stdout.to_string()
            } else if let Some(v) = out.value {
                v
            } else {
                "(sin salida)".into()
            }
        }
        Err(e) => format!("error: {e}"),
    }
}

/// Encuadra el paso actual al entrar al modo Presentar.
fn posicionar_presentar(model: &mut Model) {
    let rec = recorrido_actual(model);
    let n = rec.n_pasos();
    if n == 0 {
        return;
    }
    let panel = panel_presentar(model);
    let paso = model.recorrido_state.paso.min(n - 1);
    model.recorrido_state.saltar_a_paso(&rec, paso, panel);
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

/// La próxima hija a regenerar de **todo el haz** del activo, en **orden
/// topológico** (madre antes que hija — el orden lo da [`ReactorHaz`]): la
/// primera que esté *stale* respecto de su propia madre. Devuelve
/// `(hija, madre, tipo)`. Que la madre sea la traducción (no el activo) es lo
/// que habilita las **cadenas**: el resumen-de-la-traducción se regenera
/// después de la traducción.
fn siguiente_stale_en_orden(model: &Model, raiz: Uuid) -> Option<(Uuid, Uuid, TipoTransformacion)> {
    let rh = crate::reactor::ReactorHaz::construir(&model.orden_lienzos, &model.transformaciones);
    for hija_id in rh.regenerar_en_orden(raiz) {
        let Some(t) = model.transformaciones.iter().find(|t| t.hija == hija_id) else {
            continue;
        };
        let Some(madre) = model.cuerpos.iter().find(|c| c.id == t.madre) else {
            continue;
        };
        let stale = model
            .cuerpos
            .iter()
            .find(|c| c.id == hija_id)
            .is_some_and(|h| h.es_derivado() && h.es_stale(madre.metadatos.modificado_en));
        if stale {
            return Some((hija_id, t.madre, t.tipo.clone()));
        }
    }
    None
}

/// Disparo de la **regeneración reactiva** (Ctrl+Enter / Enter al final del
/// último párrafo): persiste la edición, marca la madre como modificada (sus
/// hijas quedan *stale*) y arranca la cascada in-place. Con backend Mock corre
/// gratis y offline.
fn disparar_regen_reactivo(model: &mut Model, handle: &Handle<Msg>) {
    guardar_activo(model);
    tocar_madre(model);
    avanzar_reactivo(model, handle);
}

/// Da **un paso** de la cascada reactiva: busca la próxima hija stale del haz
/// (en orden) y la regenera **in-place** desde su propia madre. Al completar,
/// `Msg::HijaEnLugar` vuelve a llamar acá; como cada hija, al regenerarse,
/// queda fresca, la cascada termina sola (la edición de la madre propaga hacia
/// abajo: traducción → resumen-de-la-traducción → …).
fn avanzar_reactivo(model: &mut Model, handle: &Handle<Msg>) {
    if model.en_curso {
        return;
    }
    let Some(raiz) = model.activo else {
        return;
    };
    let Some((hija_id, madre_id, tipo)) = siguiente_stale_en_orden(model, raiz) else {
        return; // nada stale aguas abajo — cascada terminada
    };
    let Some(trabajo) = trabajo_de_tipo(&tipo) else {
        return;
    };
    lanzar_modo(model, handle, trabajo, Some(hija_id), Some(madre_id));
}

/// `true` si la carta conecta exactamente los cuerpos `a` y `b` (en cualquier
/// orden).
fn carta_conecta(c: &CartaHebras, a: Uuid, b: Uuid) -> bool {
    (c.cuerpo_a == Some(a) && c.cuerpo_b == Some(b))
        || (c.cuerpo_a == Some(b) && c.cuerpo_b == Some(a))
}

/// Actualiza la hija `hija_id` **in-place** en las colecciones, **preservando
/// su id** — así los nietos que derivan de ella siguen apuntando bien (clave
/// para las cadenas). Reemplaza su cuerpo, su carta-con-la-madre `madre_id` y
/// su transformación; deja intactas las cartas con sus propios nietos. El
/// caller ya forzó `nueva.id`, `transf.hija` y el lado-hija de `carta` a
/// `hija_id`. Pura sobre las colecciones → testeable.
#[allow(clippy::too_many_arguments)]
fn actualizar_hija_in_place(
    cuerpos: &mut [Cuerpo],
    atoms: &mut HashMap<Uuid, NarrativeAtom>,
    cartas: &mut Vec<CartaHebras>,
    transformaciones: &mut Vec<Transformacion>,
    hija_id: Uuid,
    madre_id: Uuid,
    nueva: Cuerpo,
    atoms_nuevos: Vec<NarrativeAtom>,
    carta: CartaHebras,
    transf: Transformacion,
) {
    if let Some(slot) = cuerpos.iter_mut().find(|c| c.id == hija_id) {
        *slot = nueva;
    }
    // Sólo la carta (madre, hija) se reemplaza; la del nieto (hija, nieto) queda.
    cartas.retain(|c| !carta_conecta(c, madre_id, hija_id));
    cartas.push(carta);
    transformaciones.retain(|t| t.hija != hija_id);
    transformaciones.push(transf);
    for a in atoms_nuevos {
        atoms.insert(a.id, a);
    }
}

/// `true` si el caret del editor está al final del buffer (última línea, última
/// columna) — el "Enter al final del último párrafo".
fn caret_al_final(state: &EditorState) -> bool {
    let caret = state.cursor.caret;
    let ultima = state.buffer.len_lines().saturating_sub(1);
    caret.line == ultima && caret.col == state.buffer.line_len_chars(ultima)
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

/// Recibe una regeneración **reactiva** (in-place): reemplaza la hija `vieja`
/// por la nueva en su mismo lugar — **sin** apilar una traducción nueva ni
/// mover el foco (seguís editando el original). Persiste y refresca los
/// editores read-only para que la columna muestre el texto nuevo.
fn recibir_hija_en_lugar(
    model: &mut Model,
    vieja: Uuid,
    mut hija: Cuerpo,
    atoms_nuevos: Vec<NarrativeAtom>,
    mut carta: CartaHebras,
    mut transformacion: Transformacion,
) {
    // Forzar la identidad de lo producido al id de la hija EXISTENTE: el
    // ejecutor crea un cuerpo con id nuevo, pero queremos preservar el id para
    // que los nietos que derivan de esta hija sigan apuntando bien (cadenas).
    let prod_id = hija.id;
    let madre_id = transformacion.madre;
    hija.id = vieja;
    transformacion.hija = vieja;
    if carta.cuerpo_a == Some(prod_id) {
        carta.cuerpo_a = Some(vieja);
    }
    if carta.cuerpo_b == Some(prod_id) {
        carta.cuerpo_b = Some(vieja);
    }

    for a in &atoms_nuevos {
        let _ = model.store.put_atom(a);
    }
    let _ = model.store.put_cuerpo(&hija);
    let _ = model.store.put_carta(&carta);
    let _ = model.store.put_transformacion(&transformacion);
    let _ = model.store.flush();
    let nombre = hija.metadatos.nombre_legible.clone();
    actualizar_hija_in_place(
        &mut model.cuerpos,
        &mut model.atoms,
        &mut model.cartas,
        &mut model.transformaciones,
        vieja,
        madre_id,
        hija,
        atoms_nuevos,
        carta,
        transformacion,
    );
    model.en_curso = false;
    // El activo NO cambia: seguís en el original. Refrescamos los editores
    // read-only para que la columna regenerada muestre el texto nuevo.
    reconstruir_ides_ro(model);
    model.ultimo_status = format!("«{nombre}» regenerada en su lugar");
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
    lanzar_modo(model, handle, trabajo, None, None);
}

/// Como [`lanzar`] pero con modo:
/// - `reemplazar = Some(hija)` → produce `Msg::HijaEnLugar` (regeneración
///   reactiva in-place) en vez de apilar una hija nueva.
/// - `madre_override = Some(m)` → transforma `m` en vez del activo (para
///   regenerar un nieto desde su madre real, p.ej. la traducción). Cuando es
///   `None` usa el activo y persiste su edición primero.
fn lanzar_modo(
    model: &mut Model,
    handle: &Handle<Msg>,
    trabajo: TrabajoLlm,
    reemplazar: Option<Uuid>,
    madre_override: Option<Uuid>,
) {
    if model.en_curso {
        return;
    }
    let activo_id = match madre_override {
        Some(m) => m,
        None => match model.activo {
            Some(a) => a,
            None => {
                model.ultimo_status = "sin doc activo".into();
                return;
            }
        },
    };
    // Sincronizar antes de transformar — si el usuario tipeó sin Ctrl+S,
    // queremos que el LLM vea el texto editado. Sólo cuando la madre ES el
    // activo (sin override): regenerar un nieto no debe tocar el editor activo.
    if madre_override.is_none() {
        guardar_activo(model);
    }

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
            Ok((prod, transformacion)) => match reemplazar {
                // Reactivo: reemplaza la hija vieja en su lugar.
                Some(vieja) => Msg::HijaEnLugar {
                    vieja,
                    hija: prod.hija,
                    atoms_nuevos: prod.atoms_nuevos,
                    carta: prod.carta,
                    transformacion,
                },
                // Clásico: apila una hija nueva.
                None => Msg::LlmListo {
                    hija: prod.hija,
                    atoms_nuevos: prod.atoms_nuevos,
                    carta: prod.carta,
                    transformacion,
                },
            },
            Err(e) => Msg::LlmError(format!("{e:?}")),
        }
    });
}

// ---------------------------------------------------------------------
// Diente Grafo: corre el pipeline de filtros y genera una línea de lienzo
// ---------------------------------------------------------------------

/// Índice por `Uuid` de un slice de átomos prestados — el formato que comen
/// los ejecutores LLM.
fn build_idx(atoms: &[NarrativeAtom]) -> HashMap<Uuid, &NarrativeAtom> {
    atoms.iter().map(|a| (a.id, a)).collect()
}

/// Margen de similitud coseno respecto del mejor átomo. El filtro Concepto
/// por embeddings conserva los átomos cuya cercanía al concepto cae dentro de
/// este margen del máximo: un criterio **relativo** (al átomo más on-topic del
/// lienzo), agnóstico del modelo y de su escala absoluta de coseno.
const CONCEPTO_MARGEN: f32 = 0.12;

/// Filtro semántico del diente Grafo. Con un `provider` de embeddings (el
/// verbo-daemon) puntúa cada átomo por similitud coseno al `term` y conserva
/// los más cercanos al concepto; sin provider —o si el embedding falla— cae al
/// MVP léxico (substring case-insensitive). En ambos casos comparte los `Uuid`
/// de los átomos retenidos (no crea átomos nuevos) y nunca devuelve una línea
/// vacía: si nada matchea, conserva el orden completo.
async fn filtrar_concepto(
    cuerpo: &Cuerpo,
    idx: &HashMap<Uuid, &NarrativeAtom>,
    term: &str,
    ahora: u64,
    provider: Option<&dyn Provider>,
) -> Cuerpo {
    let term_t = term.trim();
    let mut hija = Cuerpo::nuevo(
        format!("{}~c", cuerpo.branch_id),
        format!("{} · concepto", cuerpo.metadatos.nombre_legible),
        Intencion::Anotacion,
        ahora,
    );
    hija.metadatos.derivado_de = Some(cuerpo.id);

    let retener: Vec<Uuid> = if term_t.is_empty() {
        cuerpo.orden.clone()
    } else {
        let por_embeddings = match provider {
            Some(p) => retener_por_embeddings(cuerpo, idx, term_t, p).await,
            None => None,
        };
        por_embeddings.unwrap_or_else(|| retener_lexico(cuerpo, idx, term_t))
    };

    for id in &retener {
        hija.agregar(*id, ahora);
    }
    if hija.orden.is_empty() {
        for id in &cuerpo.orden {
            hija.agregar(*id, ahora);
        }
    }
    hija
}

/// Retención léxica (MVP): los átomos cuyo contenido contiene `term`
/// (case-insensitive), en el orden del cuerpo madre.
fn retener_lexico(
    cuerpo: &Cuerpo,
    idx: &HashMap<Uuid, &NarrativeAtom>,
    term: &str,
) -> Vec<Uuid> {
    let term_lc = term.to_lowercase();
    cuerpo
        .orden
        .iter()
        .copied()
        .filter(|id| {
            idx.get(id)
                .map(|a| a.content.to_lowercase().contains(&term_lc))
                .unwrap_or(false)
        })
        .collect()
}

/// Retención por embeddings: puntúa cada átomo por coseno al concepto y
/// conserva los que caen dentro de [`CONCEPTO_MARGEN`] del mejor. Devuelve
/// `None` ante cualquier fallo (sin átomos con contenido, o error de embedding/
/// coseno) para que el caller caiga al criterio léxico.
async fn retener_por_embeddings(
    cuerpo: &Cuerpo,
    idx: &HashMap<Uuid, &NarrativeAtom>,
    term: &str,
    provider: &dyn Provider,
) -> Option<Vec<Uuid>> {
    let ids: Vec<Uuid> = cuerpo
        .orden
        .iter()
        .copied()
        .filter(|id| idx.contains_key(id))
        .collect();
    if ids.is_empty() {
        return None;
    }
    let term_vec = provider.embed(term).await.ok()?;
    let textos: Vec<String> = ids.iter().map(|id| idx[id].content.to_string()).collect();
    let vecs = provider.embed_batch(&textos).await.ok()?;
    if vecs.len() != ids.len() {
        return None;
    }
    let mut sims: Vec<(Uuid, f32)> = Vec::with_capacity(ids.len());
    let mut top = f32::MIN;
    for (id, v) in ids.iter().zip(vecs.iter()) {
        let sim = v.cosine(&term_vec).ok()?;
        if sim > top {
            top = sim;
        }
        sims.push((*id, sim));
    }
    let umbral = top - CONCEPTO_MARGEN;
    Some(
        sims.into_iter()
            .filter(|(_, s)| *s >= umbral)
            .map(|(id, _)| id)
            .collect(),
    )
}

/// Ruta del socket del `verbo-daemon`, alineada con `rimay-verbo-daemon-bin`:
/// `$XDG_RUNTIME_DIR/verbo.sock`, con fallback a `/tmp/verbo-{uid}.sock`.
fn socket_verbo_default() -> std::path::PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return std::path::PathBuf::from(xdg).join("verbo.sock");
    }
    let uid = std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&u| u != u32::MAX)
        .unwrap_or(1000);
    std::path::PathBuf::from(format!("/tmp/verbo-{uid}.sock"))
}

/// Conecta al verbo-daemon si su socket existe. `None` si no hay socket o si
/// la conexión falla — el filtro Concepto cae entonces al criterio léxico.
async fn conectar_verbo() -> Option<Arc<dyn Provider>> {
    let path = socket_verbo_default();
    if !path.exists() {
        return None;
    }
    match DaemonClient::connect(&path).await {
        Ok(c) => Some(Arc::new(c) as Arc<dyn Provider>),
        Err(e) => {
            eprintln!("pluma grafo :: verbo-daemon en {} falló: {e}", path.display());
            None
        }
    }
}

/// Corre el grafo de filtros sobre el lienzo activo, encadenando cada etapa
/// (la salida de una alimenta la entrada de la siguiente), y emite la línea
/// resultante como un cuerpo derivado nuevo vía `Msg::LlmListo` (reusa el
/// mismo camino de alta de columna que las transformaciones del diente Modelo).
fn generar_linea(model: &mut Model, handle: &Handle<Msg>) {
    if model.en_curso {
        return;
    }
    if model.grafo.is_empty() {
        model.ultimo_status = "agregá filtros al grafo".into();
        return;
    }
    let Some(activo_id) = model.activo else {
        model.ultimo_status = "sin doc activo".into();
        return;
    };
    // Volcar ediciones sin guardar para que los filtros vean el texto vivo.
    guardar_activo(model);
    let madre = match model.cuerpos.iter().find(|c| c.id == activo_id) {
        Some(c) => c.clone(),
        None => {
            model.ultimo_error = Some("doc activo desapareció".into());
            return;
        }
    };
    if madre.orden.is_empty() {
        model.ultimo_status = "lienzo activo vacío".into();
        return;
    }

    let filtros: Vec<Filtro> = model.grafo.iter().map(|nf| nf.filtro.clone()).collect();
    let desc = filtros
        .iter()
        .map(etiqueta_filtro)
        .collect::<Vec<_>>()
        .join(" · ");
    let atoms_owned: Vec<NarrativeAtom> = model.atoms.values().cloned().collect();
    let chat = model.chat.clone();
    let ahora = ahora_unix();

    model.en_curso = true;
    model.ultimo_error = None;
    model.ultimo_status = format!("grafo » {desc}");

    let madre_para_carta = madre.clone();
    handle.spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => return Msg::LlmError(format!("runtime tokio: {e}")),
        };
        let mut atoms_owned = atoms_owned;
        let mut acumulados: Vec<NarrativeAtom> = Vec::new();
        let mut actual = madre.clone();

        let resultado: Result<Cuerpo, String> = rt.block_on(async {
            // Sólo intentamos conectar al verbo-daemon si hay algún filtro
            // Concepto que lo use (si no, ni tocamos el socket).
            let provider = if filtros.iter().any(|f| matches!(f, Filtro::Concepto(_))) {
                conectar_verbo().await
            } else {
                None
            };
            for filtro in &filtros {
                match filtro {
                    Filtro::Concepto(term) => {
                        let idx = build_idx(&atoms_owned);
                        actual =
                            filtrar_concepto(&actual, &idx, term, ahora, provider.as_deref()).await;
                    }
                    Filtro::Traducir(l) => {
                        let prod = {
                            let idx = build_idx(&atoms_owned);
                            let ej = EjecutorTraducirLlm::from_arc(chat.clone(), l.clone());
                            let t = Transformacion::nueva(
                                actual.id,
                                Uuid::new_v4(),
                                TipoTransformacion::Traducir { lengua_destino: l.clone() },
                                "grafo",
                                ahora,
                            );
                            ej.aplicar_con_atoms(&t, &actual, &idx, ahora)
                                .await
                                .map_err(|e| format!("{e:?}"))?
                        };
                        atoms_owned.extend(prod.atoms_nuevos.iter().cloned());
                        acumulados.extend(prod.atoms_nuevos);
                        actual = prod.hija;
                    }
                    Filtro::Tono(etiq) => {
                        let prod = {
                            let idx = build_idx(&atoms_owned);
                            let ej = EjecutorTonoLlm::from_arc(chat.clone(), etiq.clone());
                            let t = Transformacion::nueva(
                                actual.id,
                                Uuid::new_v4(),
                                TipoTransformacion::Tono { etiqueta: etiq.clone() },
                                "grafo",
                                ahora,
                            );
                            ej.aplicar_con_atoms(&t, &actual, &idx, ahora)
                                .await
                                .map_err(|e| format!("{e:?}"))?
                        };
                        atoms_owned.extend(prod.atoms_nuevos.iter().cloned());
                        acumulados.extend(prod.atoms_nuevos);
                        actual = prod.hija;
                    }
                    Filtro::Resumir(p) => {
                        let prod = {
                            let idx = build_idx(&atoms_owned);
                            let ej = EjecutorResumirLlm::from_arc(chat.clone(), *p);
                            let t = Transformacion::nueva(
                                actual.id,
                                Uuid::new_v4(),
                                TipoTransformacion::Resumir { palabras_objetivo: *p },
                                "grafo",
                                ahora,
                            );
                            ej.aplicar_con_atoms(&t, &actual, &idx, ahora)
                                .await
                                .map_err(|e| format!("{e:?}"))?
                        };
                        atoms_owned.extend(prod.atoms_nuevos.iter().cloned());
                        acumulados.extend(prod.atoms_nuevos);
                        actual = prod.hija;
                    }
                }
            }
            Ok(actual.clone())
        });

        match resultado {
            Ok(mut hija) => {
                hija.metadatos.derivado_de = Some(madre_para_carta.id);
                hija.metadatos.fresco_hasta = Some(ahora);
                hija.metadatos.nombre_legible = format!("línea: {desc}");
                hija.metadatos.intencion = Intencion::Custom { kind: "grafo".into() };
                let carta = pluma_align::alinear_uno_a_uno(
                    &madre_para_carta,
                    &hija,
                    pluma_align::OrigenAlineamiento::Derivado {
                        transformacion: Uuid::new_v4(),
                        timestamp: ahora,
                    },
                );
                let transformacion = Transformacion::nueva(
                    madre_para_carta.id,
                    hija.id,
                    TipoTransformacion::Custom {
                        kind: "grafo".into(),
                        rhai_script: desc.clone(),
                    },
                    "grafo",
                    ahora,
                );
                Msg::LlmListo {
                    hija,
                    atoms_nuevos: acumulados,
                    carta,
                    transformacion,
                }
            }
            Err(e) => Msg::LlmError(e),
        }
    });
}

#[cfg(test)]
mod tests_concepto {
    use super::*;
    use rimay_verbo_mock::MockProvider;

    fn cuerpo_con(atoms: &[NarrativeAtom]) -> Cuerpo {
        let mut c = Cuerpo::nuevo("main", "doc", Intencion::Anotacion, 0);
        for a in atoms {
            c.agregar(a.id, 0);
        }
        c
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn corre_kernel_python_evalua_de_verdad() {
        // El kernel real (RustPython) ejecuta el cuerpo de la celda.
        let out = block_on(corre_kernel(
            pluma_notebook_kernel_python::PythonKernel::new(),
            "print(6 * 7)",
            "python",
        ));
        assert!(out.contains("42"), "salida inesperada: {out}");
    }

    #[test]
    fn corre_kernel_wat_evalua_de_verdad() {
        // Un módulo WAT que exporta main devolviendo 99 → el kernel wasmi lo corre.
        let wat = "(module (func (export \"main\") (result i32) i32.const 99))";
        let out = block_on(corre_kernel(
            pluma_notebook_kernel_wasm::WasmKernel::new(),
            wat,
            "wat",
        ));
        assert!(out.contains("99"), "salida inesperada: {out}");
    }

    #[test]
    fn embeddings_retiene_el_atomo_del_concepto() {
        // El MockProvider embebe por texto de forma determinista: el átomo
        // cuyo contenido == término tiene coseno ≈ 1 (es el tope) y se retiene;
        // uno claramente ajeno cae bajo el umbral relativo.
        let on_topic = NarrativeAtom::new("batalla", "main");
        let off_topic = NarrativeAtom::new("un jardín tranquilo en primavera", "main");
        let atoms = vec![on_topic.clone(), off_topic.clone()];
        let idx = build_idx(&atoms);
        let cuerpo = cuerpo_con(&atoms);
        let p = MockProvider::default();
        let ids = block_on(retener_por_embeddings(&cuerpo, &idx, "batalla", &p)).unwrap();
        assert!(ids.contains(&on_topic.id), "el átomo del concepto debe quedar");
        assert!(!ids.contains(&off_topic.id), "el ajeno debe filtrarse");
    }

    #[test]
    fn cuerpo_sin_atomos_devuelve_none() {
        let atoms: Vec<NarrativeAtom> = vec![];
        let idx = build_idx(&atoms);
        let cuerpo = Cuerpo::nuevo("main", "doc", Intencion::Anotacion, 0);
        let p = MockProvider::default();
        assert!(block_on(retener_por_embeddings(&cuerpo, &idx, "x", &p)).is_none());
    }

    #[test]
    fn lexico_filtra_por_substring_case_insensitive() {
        let a1 = NarrativeAtom::new("la Batalla final", "main");
        let a2 = NarrativeAtom::new("paz y calma", "main");
        let atoms = vec![a1.clone(), a2.clone()];
        let idx = build_idx(&atoms);
        let cuerpo = cuerpo_con(&atoms);
        assert_eq!(retener_lexico(&cuerpo, &idx, "batalla"), vec![a1.id]);
    }

    #[test]
    fn sin_provider_filtrar_concepto_usa_lexico() {
        let a1 = NarrativeAtom::new("el dragón ataca", "main");
        let a2 = NarrativeAtom::new("merienda con té", "main");
        let atoms = vec![a1.clone(), a2.clone()];
        let idx = build_idx(&atoms);
        let cuerpo = cuerpo_con(&atoms);
        let hija = block_on(filtrar_concepto(&cuerpo, &idx, "dragón", 0, None));
        assert_eq!(hija.orden, vec![a1.id]);
    }
}

#[cfg(test)]
mod tests_reactividad {
    use super::*;
    use pluma_cuerpo::Intencion;

    fn traducir() -> TipoTransformacion {
        TipoTransformacion::Traducir { lengua_destino: "en".into() }
    }

    #[test]
    fn actualizar_in_place_preserva_id_y_la_carta_del_nieto() {
        let madre_id = Uuid::from_u128(1);
        let hija_id = Uuid::from_u128(2);
        let nieto_id = Uuid::from_u128(3);

        // hija existente (traducción) en el modelo.
        let mut hija = Cuerpo::nuevo("en", "en", Intencion::Traduccion, 100);
        hija.id = hija_id;
        hija.metadatos.derivado_de = Some(madre_id);
        let mut cuerpos = vec![hija];
        let mut atoms: HashMap<Uuid, NarrativeAtom> = HashMap::new();
        let mut cartas = vec![
            CartaHebras::nueva().con_par(madre_id, hija_id), // madre↔hija (se reemplaza)
            CartaHebras::nueva().con_par(hija_id, nieto_id), // hija↔nieto (DEBE quedar)
        ];
        let mut transformaciones =
            vec![Transformacion::nueva(madre_id, hija_id, traducir(), "x", 100)];

        // Lo regenerado (el caller ya forzó el id a `hija_id`).
        let mut nueva = Cuerpo::nuevo("en", "en", Intencion::Traduccion, 200);
        nueva.id = hija_id;
        nueva.metadatos.derivado_de = Some(madre_id);
        let carta_n = CartaHebras::nueva().con_par(madre_id, hija_id);
        let transf_n = Transformacion::nueva(madre_id, hija_id, traducir(), "x", 200);
        let atomo = NarrativeAtom::new("hello", "en");

        actualizar_hija_in_place(
            &mut cuerpos, &mut atoms, &mut cartas, &mut transformaciones,
            hija_id, madre_id, nueva, vec![atomo], carta_n, transf_n,
        );

        // id preservado, sin apilar; contenido nuevo.
        assert_eq!(cuerpos.len(), 1);
        assert_eq!(cuerpos[0].id, hija_id);
        assert_eq!(cuerpos[0].metadatos.modificado_en, 200);
        // La carta del nieto sobrevive; la (madre,hija) se reemplazó (sigue 1).
        assert!(cartas.iter().any(|c| carta_conecta(c, hija_id, nieto_id)));
        assert_eq!(
            cartas.iter().filter(|c| carta_conecta(c, madre_id, hija_id)).count(),
            1
        );
        // Una sola transformación para la hija.
        assert_eq!(transformaciones.iter().filter(|t| t.hija == hija_id).count(), 1);
        assert_eq!(transformaciones[0].madre, madre_id);
    }

    #[test]
    fn caret_al_final_detecta_fin_de_buffer() {
        let mut s = EditorState::new();
        s.set_text("hola\nmundo");
        s.set_caret_at(0, 0);
        assert!(!caret_al_final(&s));
        s.set_caret_at(1, 5); // fin de "mundo"
        assert!(caret_al_final(&s));
    }
}
