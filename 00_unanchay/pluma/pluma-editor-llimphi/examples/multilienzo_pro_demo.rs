//! `multilienzo_pro` — la cara profesional del multilienzo de pluma.
//!
//! Construido sobre `multilienzo_editor_view`: cada cuerpo es un
//! **text-editor real** (gutter numerado, secciones coloreadas, edición
//! viva con undo/clipboard), y entre columnas fluyen **haces** —cintas
//! Sankey rellenas, no líneas— que unen las secciones correspondientes,
//! con color por sección y tono atenuado cuando la carta queda stale.
//!
//! Encima de esa base agrega lo que la hace una app y no un demo:
//!   - **Toolbar gráfico** (`llimphi-icons` + `llimphi-widget-toolbar`)
//!     agrupado por familia de acción.
//!   - **Zoom de fuente** A−/A+ (también `Ctrl`+rueda): reescala los
//!     editores y, con ellos, los haces.
//!   - **Inclusión múltiple de `.docx`**: cada click en "incluir" trae el
//!     siguiente documento de `PLUMA_DOCX` como columna editable nueva.
//!   - **DOCX pareado**: "emparejar" reconstruye los haces posición-a-
//!     posición entre columnas consecutivas (original ↔ traducción).
//!   - **Exportar `.docx`** del cuerpo activo.
//!   - **Transformaciones LLM** (→qu / →en / tono / resumir) que derivan
//!     columnas nuevas con sus haces `Derivado`.
//!
//! Gestos: click = foco; teclas = editor activo; `Ctrl+S` guarda (al
//! tocar la madre los haces de sus hijas pasan a stale); `Ctrl+1..9`
//! cambia de columna; rueda = scroll vertical sincronizado.
//!
//! ```bash
//! cargo run -p pluma-editor-llimphi --example multilienzo_pro_demo --release
//!
//! PLUMA_DOCX="original.docx,traduccion.docx" \
//!   cargo run -p pluma-editor-llimphi --example multilienzo_pro_demo --release
//!
//! ANTHROPIC_API_KEY=... PLUMA_LLM_BACKEND=anthropic \
//!   cargo run -p pluma-editor-llimphi --example multilienzo_pro_demo --release
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, FlexDirection, Rect, Size, Style,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, View, WheelDelta};
use llimphi_icons::{icon_view, Icon};
use llimphi_widget_text_editor::{
    EditorMetrics, EditorPalette, Language, MemClipboard, PointerEvent,
};
use llimphi_widget_toolbar::{toolbar_view, ToolbarGroup, ToolbarItem, ToolbarPalette};

use pluma_align::{alinear_uno_a_uno, CartaHebras, OrigenAlineamiento};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_cuerpo::CambioAtom;
use pluma_editor_llimphi::cuerpo_ide::CuerpoIde;
use pluma_editor_llimphi::multilienzo::PaletaHebras;
use pluma_editor_llimphi::multilienzo_editor::{
    multilienzo_editor_view, sincronizar_scroll_desde_activo, ConfigMultilienzoEditor,
};
use pluma_editor_llimphi::Palette;
use pluma_llm::{from_env as llm_from_env, BackendKind};
use pluma_llm_core::ChatClient;
use pluma_transform::{TipoTransformacion, Transformacion};
use pluma_transform_llm::{EjecutorResumirLlm, EjecutorTonoLlm, EjecutorTraducirLlm};
use uuid::Uuid;

const VISIBLE_LINES: usize = 200;
const FONT_BASE: f32 = 13.0;

/// Metrics del editor para el zoom actual. El gutter queda en estilo
/// `Numbers` (default) → editores numerados.
fn metrics(font_size: f32) -> EditorMetrics {
    EditorMetrics::for_font_size(font_size.clamp(9.0, 30.0))
}

#[derive(Clone, Debug)]
enum Msg {
    EditorKey(KeyEvent),
    EditorPointer { cuerpo: usize, ev: PointerEvent },
    Guardar,
    CambiarActivo(usize),
    Scroll(i32),
    Zoom(f32),
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
    LlmListo {
        hija: Cuerpo,
        atoms_nuevos: Vec<NarrativeAtom>,
        carta: CartaHebras,
    },
    LlmError(String),
    IncluirSiguienteDocx,
    Emparejar,
    ExportarDocx,
}

struct Model {
    cuerpos: Vec<Cuerpo>,
    atoms: HashMap<Uuid, NarrativeAtom>,
    /// `cartas[i]` une `cuerpos[i]` con `cuerpos[i+1]`.
    cartas: Vec<CartaHebras>,
    ides: Vec<CuerpoIde>,
    activo: usize,
    clipboard: MemClipboard,
    drag_accum: Vec<(f32, f32)>,
    font_size: f32,
    chat: Arc<dyn ChatClient>,
    backend: BackendKind,
    en_curso: bool,
    estado: String,
    docx_pendientes: Vec<PathBuf>,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · multilienzo pro"
    }

    fn initial_size() -> (u32, u32) {
        (1460, 860)
    }

    fn on_key(_m: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        let ctrl = event.modifiers.ctrl || event.modifiers.meta;
        if ctrl {
            if let Key::Character(s) = &event.key {
                match s.as_str() {
                    "s" | "S" => return Some(Msg::Guardar),
                    "+" | "=" => return Some(Msg::Zoom(1.0)),
                    "-" | "_" => return Some(Msg::Zoom(-1.0)),
                    d if d.len() == 1 && d.chars().next().unwrap().is_ascii_digit() => {
                        let n = d.chars().next().unwrap().to_digit(10).unwrap() as usize;
                        if n >= 1 {
                            return Some(Msg::CambiarActivo(n - 1));
                        }
                    }
                    _ => {}
                }
            }
        }
        Some(Msg::EditorKey(event.clone()))
    }

    fn on_wheel(_m: &Model, delta: WheelDelta, _c: (f32, f32), modifiers: Modifiers) -> Option<Msg> {
        if modifiers.ctrl {
            return Some(Msg::Zoom(if delta.y > 0.0 { 1.0 } else { -1.0 }));
        }
        let lineas = (delta.y.abs().max(1.0) * 3.0).round() as i32;
        Some(Msg::Scroll(if delta.y > 0.0 { -lineas } else { lineas }))
    }

    fn init(_: &Handle<Msg>) -> Model {
        let (chat, backend) = construir_chat();
        let docx: Vec<PathBuf> = std::env::var("PLUMA_DOCX")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|p| PathBuf::from(p.trim()))
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let mut atoms: HashMap<Uuid, NarrativeAtom> = HashMap::new();
        let mut cuerpos: Vec<Cuerpo> = Vec::new();
        let mut pendientes: Vec<PathBuf> = Vec::new();

        if let Some((primero, resto)) = docx.split_first() {
            match cargar_docx(primero, &mut atoms) {
                Ok(c) => {
                    eprintln!("multilienzo_pro :: madre desde {}", primero.display());
                    cuerpos.push(c);
                    pendientes = resto.to_vec();
                }
                Err(e) => {
                    eprintln!("multilienzo_pro :: no se pudo abrir {}: {e} — sembrando demo", primero.display());
                    cuerpos.push(sembrar_madre(&mut atoms));
                }
            }
        } else {
            cuerpos.push(sembrar_madre(&mut atoms));
        }

        let idx = ref_idx(&atoms);
        let ides: Vec<CuerpoIde> = cuerpos.iter().map(|c| CuerpoIde::from_cuerpo(c, &idx)).collect();
        drop(idx);
        let n = cuerpos.len();

        Model {
            cuerpos,
            atoms,
            cartas: Vec::new(),
            ides,
            activo: 0,
            clipboard: MemClipboard::default(),
            drag_accum: vec![(0.0, 0.0); n],
            font_size: FONT_BASE,
            chat,
            backend,
            en_curso: false,
            estado: String::new(),
            docx_pendientes: pendientes,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let met = metrics(model.font_size);
        let mut model = match msg {
            Msg::EditorKey(ev) => {
                let mut m = model;
                let i = m.activo;
                let _ = m.ides[i].apply_key_with_clipboard(&ev, &mut m.clipboard);
                m
            }
            Msg::EditorPointer { cuerpo, ev } => {
                let mut m = model;
                if cuerpo >= m.cuerpos.len() {
                    return m;
                }
                if matches!(ev, PointerEvent::Click { .. }) && cuerpo != m.activo {
                    m.activo = cuerpo;
                }
                let scroll = m.ides[cuerpo].state.scroll_offset;
                match ev {
                    PointerEvent::Click { x, y } => {
                        m.drag_accum[cuerpo] = (0.0, 0.0);
                        let (line, col) = met.screen_to_pos(x, y, scroll);
                        m.ides[cuerpo].set_caret(line, col);
                    }
                    PointerEvent::Drag { initial_x, initial_y, dx, dy } => {
                        m.drag_accum[cuerpo].0 += dx;
                        m.drag_accum[cuerpo].1 += dy;
                        let cx = initial_x + m.drag_accum[cuerpo].0;
                        let cy = initial_y + m.drag_accum[cuerpo].1;
                        let (line, col) = met.screen_to_pos(cx, cy, scroll);
                        m.ides[cuerpo].state.extend_selection_to(line, col);
                    }
                }
                m
            }
            Msg::Guardar => guardar(model),
            Msg::CambiarActivo(i) => {
                let mut m = model;
                if i < m.cuerpos.len() {
                    m.activo = i;
                    if let Some(s) = m.drag_accum.get_mut(i) {
                        *s = (0.0, 0.0);
                    }
                    m.ides[i].state.ensure_caret_visible(VISIBLE_LINES);
                }
                m
            }
            Msg::Scroll(d) => {
                let mut m = model;
                let i = m.activo;
                let max = m.ides[i].state.line_count().saturating_sub(1);
                let cur = m.ides[i].state.scroll_offset as i64;
                m.ides[i].state.scroll_offset = (cur + d as i64).clamp(0, max as i64) as usize;
                m
            }
            Msg::Zoom(d) => {
                let mut m = model;
                m.font_size = (m.font_size + d).clamp(9.0, 30.0);
                m.estado = format!("fuente {:.0} px", m.font_size);
                m
            }
            Msg::Traducir(l) => return arrancar(model, handle, Trabajo::Traducir(l)),
            Msg::Tono(e) => return arrancar(model, handle, Trabajo::Tono(e)),
            Msg::Resumir(p) => return arrancar(model, handle, Trabajo::Resumir(p)),
            Msg::LlmListo { hija, atoms_nuevos, carta } => {
                let mut m = model;
                for a in atoms_nuevos {
                    m.atoms.insert(a.id, a);
                }
                let idx = ref_idx(&m.atoms);
                let ide = CuerpoIde::from_cuerpo(&hija, &idx);
                drop(idx);
                m.cuerpos.push(hija);
                m.ides.push(ide);
                m.cartas.push(carta);
                m.drag_accum.push((0.0, 0.0));
                m.en_curso = false;
                m.estado = format!("{} columnas", m.cuerpos.len());
                m
            }
            Msg::LlmError(e) => {
                let mut m = model;
                m.en_curso = false;
                m.estado = format!("⚠ {}", recorte(&e, 90));
                m
            }
            Msg::IncluirSiguienteDocx => {
                let mut m = model;
                if m.docx_pendientes.is_empty() {
                    m.estado = "no quedan .docx por incluir (PLUMA_DOCX)".into();
                    return m;
                }
                let path = m.docx_pendientes.remove(0);
                match cargar_docx(&path, &mut m.atoms) {
                    Ok(nuevo) => {
                        let idx = ref_idx(&m.atoms);
                        let ide = CuerpoIde::from_cuerpo(&nuevo, &idx);
                        drop(idx);
                        if let Some(prev) = m.cuerpos.last() {
                            m.cartas.push(alinear_uno_a_uno(
                                prev,
                                &nuevo,
                                OrigenAlineamiento::Manual { autor: "docx-pareado".into(), timestamp: ahora_unix() },
                            ));
                        }
                        m.cuerpos.push(nuevo);
                        m.ides.push(ide);
                        m.drag_accum.push((0.0, 0.0));
                        m.estado = format!(
                            "incluido {} · {} columnas · {} pendientes",
                            path.display(), m.cuerpos.len(), m.docx_pendientes.len()
                        );
                    }
                    Err(e) => m.estado = format!("⚠ {}: {e}", path.display()),
                }
                m
            }
            Msg::Emparejar => {
                let mut m = model;
                m.cartas.clear();
                for w in m.cuerpos.windows(2) {
                    m.cartas.push(alinear_uno_a_uno(
                        &w[0],
                        &w[1],
                        OrigenAlineamiento::Manual { autor: "emparejado".into(), timestamp: ahora_unix() },
                    ));
                }
                m.estado = format!("emparejados {} haz(ces) posicionales", m.cartas.len());
                m
            }
            Msg::ExportarDocx => {
                let mut m = model;
                let i = m.activo;
                let idx = ref_idx(&m.atoms);
                let salida = std::env::var("PLUMA_DOCX_OUT").map(PathBuf::from).unwrap_or_else(|_| {
                    let base = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                    PathBuf::from(base).join("pluma-export.docx")
                });
                match foreign_docx::write_docx_borrow(&m.cuerpos[i], &idx) {
                    Ok(bytes) => match std::fs::write(&salida, bytes) {
                        Ok(()) => m.estado = format!("exportado «{}» → {}", m.cuerpos[i].metadatos.nombre_legible, salida.display()),
                        Err(e) => m.estado = format!("⚠ escribir docx: {e}"),
                    },
                    Err(e) => m.estado = format!("⚠ generar docx: {e:?}"),
                }
                drop(idx);
                m
            }
        };
        sincronizar_scroll_desde_activo(&mut model.ides, model.activo);
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let palette_editor = EditorPalette::default();
        let palette_lienzo = Palette::default();
        let paleta_hebras = PaletaHebras::default();
        let cfg = ConfigMultilienzoEditor::default();
        let met = metrics(model.font_size);

        let ides_ref: Vec<&CuerpoIde> = model.ides.iter().collect();
        let cuerpos_ref: Vec<&Cuerpo> = model.cuerpos.iter().collect();
        let cartas_ref: Vec<Option<&CartaHebras>> = model.cartas.iter().map(Some).collect();
        let editores = multilienzo_editor_view::<Msg, _, _>(
            &ides_ref,
            &cuerpos_ref,
            &cartas_ref,
            model.activo,
            &palette_editor,
            &paleta_hebras,
            &palette_lienzo,
            &cfg,
            met,
            VISIBLE_LINES,
            Language::Plain,
            |cuerpo, ev| Msg::EditorPointer { cuerpo, ev },
            |_| None,
        );

        let area = View::new(Style {
            flex_direction: FlexDirection::Row,
            flex_grow: 1.0,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(palette_lienzo.bg_app)
        .children(vec![editores]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(palette_editor.bg)
        .children(vec![barra(model), area, status(model, &palette_lienzo)])
    }
}

/// Toolbar gráfico: grupos de botones-ícono.
fn barra(model: &Model) -> View<Msg> {
    let pal = ToolbarPalette::default();
    let trabajando = model.en_curso;
    let hay_pendientes = !model.docx_pendientes.is_empty();
    let multi = model.cuerpos.len() >= 2;

    let grupos = vec![
        ToolbarGroup::new(vec![
            ToolbarItem::new(|_, c| icon_view(Icon::Open, c, 1.7), Msg::IncluirSiguienteDocx)
                .with_label("incluir")
                .enabled(hay_pendientes),
            ToolbarItem::new(|_, c| icon_view(Icon::Link, c, 1.7), Msg::Emparejar)
                .with_label("emparejar")
                .enabled(multi),
            ToolbarItem::new(|_, c| icon_view(Icon::Save, c, 1.7), Msg::ExportarDocx)
                .with_label("exportar"),
        ]),
        ToolbarGroup::new(vec![
            ToolbarItem::new(|_, c| icon_view(Icon::Minus, c, 1.9), Msg::Zoom(-1.0))
                .enabled(model.font_size > 9.0),
            ToolbarItem::new(|_, c| icon_view(Icon::Plus, c, 1.9), Msg::Zoom(1.0))
                .with_label(&format!("{:.0}px", model.font_size))
                .enabled(model.font_size < 30.0),
        ]),
        ToolbarGroup::new(vec![
            ToolbarItem::new(|_, c| icon_view(Icon::Font, c, 1.7), Msg::Traducir("qu".into()))
                .with_label("→qu").enabled(!trabajando),
            ToolbarItem::new(|_, c| icon_view(Icon::Font, c, 1.7), Msg::Traducir("en".into()))
                .with_label("→en").enabled(!trabajando),
            ToolbarItem::new(|_, c| icon_view(Icon::Edit, c, 1.7), Msg::Tono("formal".into()))
                .with_label("tono").enabled(!trabajando),
            ToolbarItem::new(|_, c| icon_view(Icon::FileText, c, 1.7), Msg::Resumir(Some(30)))
                .with_label("resumir").enabled(!trabajando),
        ]),
    ];
    toolbar_view(grupos, 40.0, &pal)
}

fn status(model: &Model, palette: &Palette) -> View<Msg> {
    let activo = model.cuerpos.get(model.activo).map(|c| c.metadatos.nombre_legible.as_str()).unwrap_or("—");
    let texto = if model.en_curso {
        format!("⏳ LLM en curso · {} ({})", etiqueta_backend(model.backend), model.chat.model_id())
    } else if !model.estado.is_empty() {
        model.estado.clone()
    } else {
        format!(
            "activo «{activo}» · {} columnas · {} haces · click=foco · Ctrl+S guarda · Ctrl±/rueda+Ctrl zoom",
            model.cuerpos.len(), model.cartas.len(),
        )
    };
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0) },
        flex_shrink: 0.0,
        padding: Rect { left: length(12.0_f32), right: length(12.0_f32), top: length(4.0_f32), bottom: length(4.0_f32) },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .text_aligned(texto, 11.0, palette.fg_muted, Alignment::Start)
}

// ---------------------------------------------------------------------------
// Guardar / persistir (igual semántica que editor_unico_demo)
// ---------------------------------------------------------------------------

fn guardar(model: Model) -> Model {
    let mut model = model;
    let i = model.activo;
    let caret = model.ides[i].caret();
    let scroll = model.ides[i].state.scroll_offset;

    let idx = ref_idx(&model.atoms);
    let cambios = model.ides[i].diff(&idx);
    drop(idx);

    let toco = !cambios.is_empty();
    let resumen = persistir(&mut model, i, &cambios);

    // Tocar la madre (índice 0) ⇒ sus hijas derivadas quedan stale.
    if toco && i == 0 {
        for carta in &mut model.cartas {
            for h in &mut carta.hebras {
                h.fresco = false;
            }
        }
    }

    let cuerpo = model.cuerpos[i].clone();
    let idx2 = ref_idx(&model.atoms);
    model.ides[i].recargar(&cuerpo, &idx2);
    drop(idx2);
    model.ides[i].set_caret(caret.0, caret.1);
    model.ides[i].state.scroll_offset = scroll;
    model.ides[i].state.ensure_caret_visible(VISIBLE_LINES);
    model.estado = resumen;
    model
}

fn persistir(model: &mut Model, i: usize, cambios: &[CambioAtom]) -> String {
    if cambios.is_empty() {
        return "guardado: sin cambios".into();
    }
    let (mut mutados, mut eliminados) = (0usize, 0usize);
    let mut creados: Vec<Uuid> = Vec::new();
    let branch = model.cuerpos[i].branch_id.clone();
    for c in cambios {
        match c {
            CambioAtom::Mutar { id, texto_nuevo } => {
                if let Some(a) = model.atoms.get_mut(id) {
                    a.set_content(texto_nuevo.as_str());
                    mutados += 1;
                }
            }
            CambioAtom::Crear { texto, .. } => {
                let atom = NarrativeAtom::new(texto.as_str(), &branch);
                let id = atom.id;
                model.atoms.insert(id, atom);
                creados.push(id);
            }
            CambioAtom::Eliminar { id } => {
                model.atoms.remove(id);
                eliminados += 1;
            }
        }
    }
    model.ides[i].aplicar_cambios(cambios, &creados);
    let nuevo_orden: Vec<Uuid> = model.ides[i].editor_cuerpo.atom_ids.clone();
    let ahora = model.cuerpos[i].metadatos.modificado_en.saturating_add(1);
    let viejo: Vec<Uuid> = model.cuerpos[i].orden.clone();
    for id in &viejo {
        let _ = model.cuerpos[i].remover(*id, ahora);
    }
    for id in &nuevo_orden {
        model.cuerpos[i].agregar(*id, ahora);
    }
    format!(
        "guardado «{}»: {mutados} mutar · {} crear · {eliminados} borrar — {} átomos",
        model.cuerpos[i].metadatos.nombre_legible, creados.len(), nuevo_orden.len()
    )
}

// ---------------------------------------------------------------------------
// LLM
// ---------------------------------------------------------------------------

enum Trabajo {
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
}

fn arrancar(model: Model, handle: &Handle<Msg>, trabajo: Trabajo) -> Model {
    let mut m = model;
    if m.en_curso || m.cuerpos.is_empty() {
        return m;
    }
    m.en_curso = true;
    m.estado.clear();
    let madre = m.cuerpos[0].clone();
    let atoms_owned: Vec<NarrativeAtom> = m.atoms.values().cloned().collect();
    let chat = m.chat.clone();
    let ahora = ahora_unix();

    handle.spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => return Msg::LlmError(format!("runtime tokio: {e}")),
        };
        let idx: HashMap<Uuid, &NarrativeAtom> = atoms_owned.iter().map(|a| (a.id, a)).collect();
        let resultado = rt.block_on(async {
            match trabajo {
                Trabajo::Traducir(l) => {
                    let ej = EjecutorTraducirLlm::from_arc(chat, l.clone());
                    let t = Transformacion::nueva(madre.id, Uuid::new_v4(), TipoTransformacion::Traducir { lengua_destino: l }, "ui", ahora);
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora).await
                }
                Trabajo::Tono(e) => {
                    let ej = EjecutorTonoLlm::from_arc(chat, e.clone());
                    let t = Transformacion::nueva(madre.id, Uuid::new_v4(), TipoTransformacion::Tono { etiqueta: e }, "ui", ahora);
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora).await
                }
                Trabajo::Resumir(p) => {
                    let ej = EjecutorResumirLlm::from_arc(chat, p);
                    let t = Transformacion::nueva(madre.id, Uuid::new_v4(), TipoTransformacion::Resumir { palabras_objetivo: p }, "ui", ahora);
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora).await
                }
            }
        });
        match resultado {
            Ok(prod) => Msg::LlmListo { hija: prod.hija, atoms_nuevos: prod.atoms_nuevos, carta: prod.carta },
            Err(e) => Msg::LlmError(format!("{e:?}")),
        }
    });
    m
}

// ---------------------------------------------------------------------------
// docx + seed + helpers
// ---------------------------------------------------------------------------

fn ref_idx(atoms: &HashMap<Uuid, NarrativeAtom>) -> HashMap<Uuid, &NarrativeAtom> {
    atoms.iter().map(|(k, v)| (*k, v)).collect()
}

fn cargar_docx(path: &PathBuf, atoms: &mut HashMap<Uuid, NarrativeAtom>) -> Result<Cuerpo, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let nombre = path.file_stem().and_then(|s| s.to_str()).unwrap_or("docx").to_string();
    let imp = foreign_docx::parse_docx(&bytes, nombre.clone(), nombre, ahora_unix()).map_err(|e| format!("{e}"))?;
    for atom in imp.atoms {
        atoms.insert(atom.id, atom);
    }
    Ok(imp.cuerpo)
}

fn sembrar_madre(atoms: &mut HashMap<Uuid, NarrativeAtom>) -> Cuerpo {
    let mut es = Cuerpo::nuevo("es", "español (original)", Intencion::Original, 100);
    for t in [
        "El cóndor cruzó el cielo del valle al amanecer.",
        "Las llamas pastaban entre los pastizales del altiplano.",
        "Una mujer joven tejía un telar bajo el alero.",
        "El río bajaba turbio tras la lluvia de la noche.",
    ] {
        let atom = NarrativeAtom::new(t, "es");
        es.agregar(atom.id, 101);
        atoms.insert(atom.id, atom);
    }
    es
}

fn recorte(s: &str, n: usize) -> &str {
    let mut corte = s.len().min(n);
    while !s.is_char_boundary(corte) && corte > 0 {
        corte -= 1;
    }
    &s[..corte]
}

fn etiqueta_backend(b: BackendKind) -> &'static str {
    match b {
        BackendKind::Mock => "mock",
        BackendKind::Gemini => "gemini",
        BackendKind::Anthropic => "anthropic",
        BackendKind::DeepSeek => "deepseek",
        BackendKind::Cohere => "cohere",
        BackendKind::Ollama => "ollama",
    }
}

fn construir_chat() -> (Arc<dyn ChatClient>, BackendKind) {
    let hay_key = std::env::var("ANTHROPIC_API_KEY").is_ok()
        || std::env::var("GEMINI_API_KEY").is_ok()
        || std::env::var("GOOGLE_API_KEY").is_ok()
        || std::env::var("DEEPSEEK_API_KEY").is_ok()
        || std::env::var("COHERE_API_KEY").is_ok()
        || std::env::var("PLUMA_LLM_BACKEND").map(|s| s.eq_ignore_ascii_case("ollama")).unwrap_or(false);
    if !hay_key {
        let mut mock = pluma_llm_mock::MockChatClient::default().con_model_id("mock-pro");
        for (k, v) in [
            ("cóndor cruzó", "Kuntur wayqu hanaqpachata pacha paqarinpi pasarqa."),
            ("Las llamas pastaban", "Llamaqakuna qulla suyup q'achupinpi mikhusharqaku."),
            ("mujer joven tejía", "Sipas warmiq away wasiq hawanpi awayta ruwasharqa."),
            ("río bajaba", "Mayu tuta paranmanta q'illu uraykamusharqa."),
        ] {
            mock = mock.con_respuesta(k, v);
        }
        return (Arc::new(mock), BackendKind::Mock);
    }
    let backend = std::env::var("PLUMA_LLM_BACKEND").ok().and_then(|s| BackendKind::parse(&s)).unwrap_or(BackendKind::Anthropic);
    (llm_from_env().expect("from_env"), backend)
}

fn ahora_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn main() {
    llimphi_ui::run::<Demo>();
}
