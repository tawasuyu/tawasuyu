//! Lógica de actualización del bucle Elm: el `match` central, las
//! mutaciones del modelo (abrir/crear/guardar/mover/regenerar), el
//! find-in-page, y el trabajo LLM lanzado en un thread aparte.

use std::collections::HashMap;

use llimphi_ui::Handle;
use llimphi_widget_text_editor::PointerEvent;
use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_cuerpo::CambioAtom;
use pluma_llm::{build_client, LlmConfig};
use pluma_transform::{TipoTransformacion, Transformacion};
use pluma_transform_llm::{EjecutorResumirLlm, EjecutorTonoLlm, EjecutorTraducirLlm};
use uuid::Uuid;

use crate::model::{Model, Msg, BACKENDS, METRICS, VISIBLE_LINES};
use crate::util::{ahora_unix, etiqueta_backend, expandir_ruta, extension_lower};

pub(crate) fn actualizar(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
    match msg {
        Msg::EditorKey(ev) => {
            let _ = model.ide.apply_key_with_clipboard(&ev, &mut model.clipboard);
        }
        Msg::EditorPointer(ev) => {
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
            model.diff_visible = !model.diff_visible;
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
        Msg::ResizeIzq(dx) => {
            model.side_izq_w = (model.side_izq_w + dx).clamp(160.0, 420.0);
        }
        Msg::ResizeDer(dx) => {
            // El divisor está del lado izquierdo de la columna derecha:
            // dx>0 = divisor a la derecha = panel der encoge.
            model.side_der_w = (model.side_der_w - dx).clamp(220.0, 520.0);
        }
    }
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
    let idx: HashMap<Uuid, &NarrativeAtom> =
        model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    model.ide.recargar(&cuerpo, &idx);
    model.ultimo_status = format!("doc: {}", cuerpo.metadatos.nombre_legible);
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
