//! **Editor único** — la UX prometida en PLAN §11.
//!
//! Layout vertical:
//!
//!   ┌──────────────────────────────────────────────────────────────┐
//!   │ header: cuerpo activo + atajos                               │
//!   ├──────────────────────────────────────────────────────────────┤
//!   │ ┌─────────────────┬─────────┬─────────────────┐              │
//!   │ │                 │         │                 │              │
//!   │ │ CuerpoIde 0     │ hebras  │ CuerpoIde 1     │              │
//!   │ │ (text-editor)   │ (carril)│ (text-editor)   │              │
//!   │ │                 │         │                 │              │
//!   │ └─────────────────┴─────────┴─────────────────┘              │
//!   ├──────────────────────────────────────────────────────────────┤
//!   │ footer: último save                                          │
//!   └──────────────────────────────────────────────────────────────┘
//!
//! Tres cuerpos: `qu` (derivado) — `es` (original, **al centro**) —
//! `en` (derivado). Dos cartas: `qu ↔ es` y `es ↔ en`. Cada cuerpo
//! es un text-editor real (no readonly): escribís donde mirás. Las
//! hebras salen en curva S a ambos lados de la madre central — el
//! sentido visual del DAG es directo (madre arriba/centro, hijas
//! abajo/laterales) sin necesidad de etiquetas.
//!
//! **Scroll vertical sincronizado**: al final de cada `update`, el
//! scroll del cuerpo activo se copia a todos los demás (clampeado al
//! fin de buffer de cada uno). PageUp/PageDown, ensure_caret_visible
//! tras typing y set_caret tras click — cualquier cosa que mueva el
//! viewport del activo arrastra al resto. Las hebras nunca se
//! desalinean visualmente.
//!
//! Atajos y gestos:
//!   - **Click dentro de cualquier editor** → le da el foco (cuerpo
//!     activo) y posiciona el caret en la línea cliqueada.
//!   - `Ctrl+1` / `Ctrl+2` / `Ctrl+3` → cambiar cuerpo activo con
//!     teclado (qu / es / en respectivamente; preserva buffer, caret,
//!     undo — cada cuerpo tiene su propio `CuerpoIde`).
//!   - `Ctrl+S` → diff + persiste el cuerpo activo; si era la madre
//!     (`es`), marca **ambas** cartas como stale (hebras punteadas).
//!   - `Ctrl+]` → siguiente átomo del cuerpo activo.
//!   - `Ctrl+C/X/V` → clipboard en memoria.
//!
//! ```bash
//! cargo run -p pluma-editor-llimphi --example editor_unico_demo --release
//! ```

use std::collections::HashMap;

use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, FlexDirection, Rect, Size, Style,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, View};
use llimphi_widget_text_editor::{
    EditorMetrics, EditorPalette, Language, MemClipboard, PointerEvent,
};
use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_cuerpo::CambioAtom;
use pluma_editor_llimphi::cuerpo_ide::CuerpoIde;
use pluma_editor_llimphi::multilienzo::PaletaHebras;
use pluma_editor_llimphi::multilienzo_editor::{
    multilienzo_editor_view, sincronizar_scroll_desde_activo, ConfigMultilienzoEditor,
};
use pluma_editor_llimphi::Palette;
use pluma_transform::{Ejecutor, TipoTransformacion, Transformacion};
use pluma_transform_tabla::EjecutorTraducirTabla;
use uuid::Uuid;

const METRICS: EditorMetrics = EditorMetrics::for_font_size(13.0);
const VISIBLE_LINES: usize = 200;

#[derive(Clone, Debug)]
enum Msg {
    EditorKey(KeyEvent),
    /// Pointer event sobre el editor del cuerpo `cuerpo`. Al cliquear un
    /// editor que no es el activo, además del set_caret cambiamos el
    /// `activo` — el foco lo da el último click.
    EditorPointer { cuerpo: usize, ev: PointerEvent },
    Guardar,
    CambiarActivo(usize),
    SaltarAtomoSiguiente,
}

struct Model {
    cuerpos: Vec<Cuerpo>,
    atoms: HashMap<Uuid, NarrativeAtom>,
    /// `cartas[i]` conecta `cuerpos[i]` con `cuerpos[i+1]`.
    cartas: Vec<CartaHebras>,
    /// Un IDE por cuerpo — cambiar de cuerpo conserva el buffer de cada
    /// uno (caret, undo, ediciones sin guardar). Indexado por cuerpo.
    ides: Vec<CuerpoIde>,
    /// Índice en `cuerpos` y en `ides` del cuerpo con foco — el que
    /// recibe los `Msg::EditorKey` y se pinta con borde accent.
    activo: usize,
    clipboard: MemClipboard,
    ultimo_save: String,
    /// Acumulado de drag (x, y) por cuerpo — el `Drag` del widget pasa
    /// deltas y el caller acumula.
    drag_accum: Vec<(f32, f32)>,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · editor único (editores lado-a-lado + hebras)"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 820)
    }

    fn init(_: &Handle<Msg>) -> Model {
        // -- Cuerpo madre `es` -------------------------------------------
        let textos_es = [
            "El cóndor cruzó el cielo del valle al amanecer.",
            "Las llamas pastaban entre los pastizales del altiplano.",
            "Una mujer joven tejía un telar bajo el alero.",
            "El río Apurímac descendía rugiente por las rocas.",
        ];
        let atoms_es: Vec<NarrativeAtom> = textos_es
            .iter()
            .map(|t| NarrativeAtom::new(*t, "es"))
            .collect();
        let mut es = Cuerpo::nuevo("es", "español (original)", Intencion::Original, 100);
        for a in &atoms_es {
            es.agregar(a.id, 101);
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime tokio");

        // -- Cuerpo `qu` derivado por tabla -------------------------------
        let traducciones_qu = [
            "Kuntur wayqu hanaqpachata pacha paqarinpi pasarqa.",
            "Llamaqakuna qulla suyup q'achupinpi mikhusharqaku.",
            "Sipas warmiq away wasiq hawanpi awayta ruwasharqa.",
            "Apurímac mayu rumikuna ukhumanta qhaparispa uraykurqa.",
        ];
        let (qu, atoms_qu, carta_es_qu) = derivar_por_tabla(
            &rt, &es, &atoms_es, &traducciones_qu, "qu", 200,
        );

        // -- Cuerpo `en` derivado por tabla -------------------------------
        let traducciones_en = [
            "The condor crossed the valley sky at dawn.",
            "Llamas grazed among the highland grasslands.",
            "A young woman was weaving on a loom beneath the eaves.",
            "The Apurímac river descended roaring through the rocks.",
        ];
        let (en, atoms_en, carta_es_en) = derivar_por_tabla(
            &rt, &es, &atoms_es, &traducciones_en, "en", 300,
        );

        // -- Index global de atoms ----------------------------------------
        let mut atoms: HashMap<Uuid, NarrativeAtom> = HashMap::new();
        for a in atoms_es
            .iter()
            .chain(atoms_qu.iter())
            .chain(atoms_en.iter())
        {
            atoms.insert(a.id, a.clone());
        }

        // Orden visual: la madre (es) al centro, derivadas a los lados.
        // El multilienzo_editor pinta cartas entre cuerpos consecutivos,
        // así que cartas[0] = qu↔es y cartas[1] = es↔en. Las hebras
        // salen en S a ambos lados de la columna central — visualiza
        // cómo `es` es el ancla de las traducciones.
        let cuerpos = vec![qu, es, en];
        let cartas = vec![carta_es_qu, carta_es_en];
        let idx = ref_idx(&atoms);
        let ides: Vec<CuerpoIde> = cuerpos
            .iter()
            .map(|c| CuerpoIde::from_cuerpo(c, &idx))
            .collect();
        drop(idx);

        let n = cuerpos.len();
        Model {
            cuerpos,
            atoms,
            cartas,
            ides,
            // Arranca con `es` (la madre) activa — está al centro.
            activo: 1,
            clipboard: MemClipboard::default(),
            ultimo_save: String::new(),
            drag_accum: vec![(0.0, 0.0); n],
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        // Procesamos el msg sobre el cuerpo activo y, al final, sincronizamos
        // el scroll del activo a todos los demás editores. Las hebras
        // quedan así alineadas en todo momento sin importar qué cuerpo
        // disparó el cambio.
        let mut model: Model = match msg {
            Msg::EditorKey(ev) => {
                let mut model = model;
                let i = model.activo;
                let _ = model.ides[i].apply_key_with_clipboard(&ev, &mut model.clipboard);
                model
            }
            Msg::EditorPointer { cuerpo, ev } => {
                let mut model = model;
                if cuerpo >= model.cuerpos.len() {
                    return model;
                }
                // Cualquier click en un editor le da el foco. Drag sin
                // click previo no debería cambiar el activo (el press
                // que originó el drag ya lo cambió antes).
                if matches!(ev, PointerEvent::Click { .. }) && cuerpo != model.activo {
                    model.activo = cuerpo;
                }
                let scroll = model.ides[cuerpo].state.scroll_offset;
                match ev {
                    PointerEvent::Click { x, y } => {
                        model.drag_accum[cuerpo] = (0.0, 0.0);
                        let (line, col) = METRICS.screen_to_pos(x, y, scroll);
                        model.ides[cuerpo].set_caret(line, col);
                    }
                    PointerEvent::Drag {
                        initial_x,
                        initial_y,
                        dx,
                        dy,
                    } => {
                        model.drag_accum[cuerpo].0 += dx;
                        model.drag_accum[cuerpo].1 += dy;
                        let cx = initial_x + model.drag_accum[cuerpo].0;
                        let cy = initial_y + model.drag_accum[cuerpo].1;
                        let (line, col) = METRICS.screen_to_pos(cx, cy, scroll);
                        model.ides[cuerpo].state.extend_selection_to(line, col);
                    }
                }
                model
            }
            Msg::Guardar => guardar(model),
            Msg::CambiarActivo(i) => {
                let mut model = model;
                if i < model.cuerpos.len() {
                    model.activo = i;
                    if let Some(slot) = model.drag_accum.get_mut(i) {
                        *slot = (0.0, 0.0);
                    }
                    // Al cambiar activo con teclado, el caret del nuevo
                    // activo puede estar fuera del viewport común. Lo
                    // traemos a la vista — el scroll resultante se
                    // propaga al resto en la sincronización del final.
                    model.ides[i].state.ensure_caret_visible(VISIBLE_LINES);
                }
                model
            }
            Msg::SaltarAtomoSiguiente => {
                let mut model = model;
                let i = model.activo;
                if let Some(siguiente) = atom_siguiente(&model.ides[i]) {
                    if let Some((line, col)) = model.ides[i].posicion_de_atom(siguiente) {
                        model.ides[i].set_caret(line, col);
                        model.ides[i].state.ensure_caret_visible(VISIBLE_LINES);
                    }
                }
                model
            }
        };
        sincronizar_scroll_desde_activo(&mut model.ides, model.activo);
        model
    }

    fn on_key(_model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        let ctrl = event.modifiers.ctrl || event.modifiers.meta;
        if ctrl {
            if let Key::Character(s) = &event.key {
                match s.as_str() {
                    "s" | "S" => return Some(Msg::Guardar),
                    "]" => return Some(Msg::SaltarAtomoSiguiente),
                    "1" => return Some(Msg::CambiarActivo(0)),
                    "2" => return Some(Msg::CambiarActivo(1)),
                    "3" => return Some(Msg::CambiarActivo(2)),
                    _ => {}
                }
            }
        }
        Some(Msg::EditorKey(event.clone()))
    }

    fn view(model: &Model) -> View<Msg> {
        let palette_editor = EditorPalette::default();
        let palette_lienzo = Palette::default();
        let paleta_hebras = PaletaHebras::default();
        let cfg = ConfigMultilienzoEditor::default();

        let bg_app = palette_editor.bg;
        let fg_text = palette_editor.fg_text;
        let fg_muted = palette_editor.fg_line_number;

        let ide_activo = &model.ides[model.activo];
        let header_text = format!(
            "activo: «{}»  ·  {} átomos  ·  {} párrafos  ·  {}  ·  click = foco  ·  Ctrl+1/2/3 cambiar  ·  Ctrl+S guardar  ·  Ctrl+] siguiente",
            model.cuerpos[model.activo].metadatos.nombre_legible,
            model.cuerpos[model.activo].orden.len(),
            ide_activo.n_parrafos_buffer(),
            if ide_activo.pendiente_sync() {
                "● cambios sin guardar"
            } else {
                "○ sincronizado"
            },
        );
        let header = chip(header_text, 28.0, 12.0, Color::from_rgba8(40, 44, 52, 255), fg_text);

        // Cuerpo principal: N editores lado-a-lado con hebras entre
        // cada par consecutivo. Click en cualquiera → foco; teclas →
        // editor activo. Las hebras siguen al scroll de cada editor.
        let ides_ref: Vec<&CuerpoIde> = model.ides.iter().collect();
        let cuerpos_ref: Vec<&Cuerpo> = model.cuerpos.iter().collect();
        let cartas_ref: Vec<Option<&CartaHebras>> = model.cartas.iter().map(Some).collect();
        let editores = multilienzo_editor_view::<Msg, _>(
            &ides_ref,
            &cuerpos_ref,
            &cartas_ref,
            model.activo,
            &palette_editor,
            &paleta_hebras,
            &palette_lienzo,
            &cfg,
            METRICS,
            VISIBLE_LINES,
            Language::Plain,
            |cuerpo, ev| Msg::EditorPointer { cuerpo, ev },
        );
        let area_principal = View::new(Style {
            flex_direction: FlexDirection::Row,
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(palette_lienzo.bg_app)
        .children(vec![editores]);

        // Footer: estado de las hebras (fresh/total por carta) +
        // último save. Útil para ver de un vistazo qué pasó al
        // Ctrl+S — si editaste `es`, las dos cartas pasan de N/N a
        // 0/N y las hebras del multilienzo se pintan punteadas.
        let estado_hebras = formatear_estado_hebras(&model);
        let footer_text = if model.ultimo_save.is_empty() {
            format!(
                "{estado_hebras}  ·  editá y Ctrl+S; al tocar la madre las hebras pasan a stale"
            )
        } else {
            format!("{estado_hebras}  ·  {}", model.ultimo_save)
        };
        let footer = chip(footer_text, 24.0, 11.0, Color::from_rgba8(33, 36, 42, 255), fg_muted);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(bg_app)
        .children(vec![header, area_principal, footer])
    }
}

fn chip(texto: String, alto: f32, font_size: f32, fondo: Color, fg: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(alto),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(fondo)
    .text_aligned(texto, font_size, fg, Alignment::Start)
}

fn ref_idx(atoms: &HashMap<Uuid, NarrativeAtom>) -> HashMap<Uuid, &NarrativeAtom> {
    atoms.iter().map(|(k, v)| (*k, v)).collect()
}

/// Crea un cuerpo derivado de `madre` aplicando `EjecutorTraducirTabla`
/// con la tabla `madre_atom_id → traduccion[i]`. Devuelve la hija +
/// sus átomos nuevos + la carta de hebras derivadas.
///
/// El demo lo usa para generar `qu` y `en` desde la misma `es` sin
/// duplicar el ceremonial del runtime tokio en cada llamada.
/// Resumen textual del estado de cada carta en el formato
/// `cuerpoA↔cuerpoB: fresh/total`. El multilienzo_editor pinta `cartas[i]`
/// entre `cuerpos[i]` y `cuerpos[i+1]`, así que el rótulo refleja ese
/// par exacto. Hebras stale destacan con un `✗`, todas fresh con un `✓`.
fn formatear_estado_hebras(model: &Model) -> String {
    if model.cartas.is_empty() {
        return "(sin cartas)".to_string();
    }
    let mut partes: Vec<String> = Vec::with_capacity(model.cartas.len());
    for (i, carta) in model.cartas.iter().enumerate() {
        let total = carta.hebras.len();
        let fresh = carta.hebras.iter().filter(|h| h.fresco).count();
        let estado = if total == 0 {
            "—"
        } else if fresh == total {
            "✓"
        } else {
            "✗"
        };
        let a = model
            .cuerpos
            .get(i)
            .map(|c| c.branch_id.as_str())
            .unwrap_or("?");
        let b = model
            .cuerpos
            .get(i + 1)
            .map(|c| c.branch_id.as_str())
            .unwrap_or("?");
        partes.push(format!("{a}↔{b}: {fresh}/{total} {estado}"));
    }
    partes.join("  ·  ")
}

fn derivar_por_tabla(
    rt: &tokio::runtime::Runtime,
    madre: &Cuerpo,
    atoms_madre: &[NarrativeAtom],
    traducciones: &[&str],
    lengua_destino: &str,
    timestamp: u64,
) -> (Cuerpo, Vec<NarrativeAtom>, CartaHebras) {
    let mut tabla: HashMap<Uuid, String> = HashMap::new();
    for (atom, tr) in atoms_madre.iter().zip(traducciones.iter()) {
        tabla.insert(atom.id, (*tr).to_string());
    }
    let ejecutor = EjecutorTraducirTabla::new(tabla, lengua_destino);
    let t = Transformacion::nueva(
        madre.id,
        Uuid::new_v4(),
        TipoTransformacion::Traducir {
            lengua_destino: lengua_destino.into(),
        },
        "ana",
        timestamp,
    );
    let prod = rt
        .block_on(ejecutor.aplicar(&t, madre, timestamp))
        .expect("traducción por tabla");
    (prod.hija, prod.atoms_nuevos, prod.carta)
}

fn atom_siguiente(ide: &CuerpoIde) -> Option<Uuid> {
    if ide.editor_cuerpo.atom_ids.is_empty() {
        return None;
    }
    let (linea, _) = ide.caret();
    let actual = ide.atom_id_en_linea(linea);
    let pos_actual = actual
        .and_then(|id| ide.editor_cuerpo.atom_ids.iter().position(|x| *x == id))
        .unwrap_or(usize::MAX);
    let n = ide.editor_cuerpo.atom_ids.len();
    let siguiente = if pos_actual == usize::MAX {
        0
    } else {
        (pos_actual + 1) % n
    };
    ide.editor_cuerpo.atom_ids.get(siguiente).copied()
}

fn guardar(model: Model) -> Model {
    let mut model = model;
    let i = model.activo;
    let caret_antes = model.ides[i].caret();
    let scroll_antes = model.ides[i].state.scroll_offset;

    let idx = ref_idx(&model.atoms);
    let cambios = model.ides[i].diff(&idx);
    drop(idx);

    let toco_atomos = !cambios.is_empty();
    let resumen = persistir(&mut model, i, &cambios);

    // Si tocamos la madre (`es`, idx=1 en el orden [qu, es, en]), TODAS
    // las cartas quedan stale — ambas la usan como un extremo. Si
    // tocamos una hija (qu o en), las hebras se mantienen fresh: la
    // hija cambió por edición humana, no porque la madre haya
    // derivado. Conservador pero exacto para el demo.
    let edito_la_madre = i == 1;
    if toco_atomos && edito_la_madre {
        for carta in &mut model.cartas {
            for h in &mut carta.hebras {
                h.fresco = false;
            }
        }
    }

    // Refrescá el IDE del cuerpo modificado con el cuerpo + atoms nuevos.
    let cuerpo_clon = model.cuerpos[i].clone();
    let idx2 = ref_idx(&model.atoms);
    model.ides[i].recargar(&cuerpo_clon, &idx2);
    drop(idx2);
    model.ides[i].set_caret(caret_antes.0, caret_antes.1);
    model.ides[i].state.scroll_offset = scroll_antes;
    model.ides[i].state.ensure_caret_visible(VISIBLE_LINES);

    model.ultimo_save = resumen;
    model
}

fn persistir(model: &mut Model, i_cuerpo: usize, cambios: &[CambioAtom]) -> String {
    if cambios.is_empty() {
        return "guardado: sin cambios".to_string();
    }
    let mut mutados = 0usize;
    let mut creados: Vec<Uuid> = Vec::new();
    let mut eliminados = 0usize;

    let branch_id = model.cuerpos[i_cuerpo].branch_id.clone();
    for c in cambios {
        match c {
            CambioAtom::Mutar { id, texto_nuevo } => {
                if let Some(a) = model.atoms.get_mut(id) {
                    a.set_content(texto_nuevo.as_str());
                    mutados += 1;
                }
            }
            CambioAtom::Crear { texto, posicion: _ } => {
                let atom = NarrativeAtom::new(texto.as_str(), &branch_id);
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

    model.ides[i_cuerpo].aplicar_cambios(cambios, &creados);
    let nuevo_orden: Vec<Uuid> = model.ides[i_cuerpo].editor_cuerpo.atom_ids.clone();

    let ahora = model.cuerpos[i_cuerpo].metadatos.modificado_en.saturating_add(1);
    let viejo: Vec<Uuid> = model.cuerpos[i_cuerpo].orden.clone();
    for id in &viejo {
        let _ = model.cuerpos[i_cuerpo].remover(*id, ahora);
    }
    for id in &nuevo_orden {
        model.cuerpos[i_cuerpo].agregar(*id, ahora);
    }

    let nombre = &model.cuerpos[i_cuerpo].metadatos.nombre_legible;
    format!(
        "guardado en «{nombre}»: {mutados} mutar · {} crear · {eliminados} eliminar — {} átomos",
        creados.len(),
        nuevo_orden.len(),
    )
}

fn main() {
    llimphi_ui::run::<Demo>();
}
