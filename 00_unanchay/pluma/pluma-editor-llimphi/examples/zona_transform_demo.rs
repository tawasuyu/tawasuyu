//! Demo: transformaciones LLM sobre la **zona del caret**, no sobre el
//! cuerpo entero. Es la unión del `cuerpo_ide_demo` (edición con
//! junctions togglables) y `multilienzo_dinamico_demo` (botones LLM),
//! pero el ejecutor recibe un sub-`Cuerpo` con SOLO los atoms de la zona
//! activa.
//!
//! Flujo:
//!
//!   1. cuerpo_ide a la izquierda con 6 párrafos sintéticos.
//!   2. Edita con normalidad (multi-cursor, undo, clipboard). `Ctrl+J`
//!      togglea la junction anterior al caret → fusiona/desfusiona zonas.
//!      `Ctrl+Shift+]/[` saltan entre zonas.
//!   3. Click en cualquiera de los cuatro botones (`→ qu`, `→ en`,
//!      `tono formal`, `resumir 30p`): toma la zona del caret, hace un
//!      `guardar()` implícito (sync cuerpo_ide → atoms), arma un
//!      sub-`Cuerpo` con `orden = atom_ids_de_zona(zona)`, lanza el
//!      ejecutor LLM correspondiente, y al volver agrega una card en el
//!      panel derecho con la hija producida.
//!
//! Sin keys: usa `pluma_llm_mock` con respuestas pre-pobladas. Con keys
//! (`GEMINI_API_KEY`, `ANTHROPIC_API_KEY`…) usa `pluma_llm::from_env`.
//!
//! ```bash
//! cargo run -p pluma-editor-llimphi --example zona_transform_demo --release
//!
//! GEMINI_API_KEY=... PLUMA_LLM_BACKEND=gemini \
//!   cargo run -p pluma-editor-llimphi --example zona_transform_demo --release
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, FlexDirection, Rect, Size, Style,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_text_editor::{
    EditorMetrics, EditorPalette, Language, MemClipboard, PointerEvent,
};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_cuerpo::CambioAtom;
use pluma_editor_llimphi::cuerpo_ide::{cuerpo_ide_view, CuerpoIde};
use pluma_llm::from_env as llm_from_env;
use pluma_llm_core::ChatClient;
use pluma_transform::{TipoTransformacion, Transformacion};
use pluma_transform_llm::{
    EjecutorResumirLlm, EjecutorTonoLlm, EjecutorTraducirLlm,
};
use uuid::Uuid;

const METRICS: EditorMetrics = EditorMetrics::for_font_size(13.0);
const VISIBLE_LINES: usize = 200;

#[derive(Clone, Debug)]
enum Msg {
    EditorKey(KeyEvent),
    EditorPointer(PointerEvent),
    ToglearFusion,
    ZonaSiguiente,
    ZonaAnterior,
    PedirTraducir(String),
    PedirTono(String),
    PedirResumir(Option<u32>),
    LlmListo {
        zona: usize,
        etiqueta: String,
        branch: String,
        atoms_nuevos: Vec<NarrativeAtom>,
        orden: Vec<Uuid>,
    },
    LlmError(String),
}

/// Una derivación por zona ya materializada. El panel derecho pinta una
/// `card` por entrada — vivimos con un `Vec<HijaZona>` plano para que no
/// haya magia: cada click suma una entrada al final.
struct HijaZona {
    zona: usize,
    etiqueta: String,
    branch: String,
    atoms: HashMap<Uuid, NarrativeAtom>,
    orden: Vec<Uuid>,
}

struct Model {
    cuerpo: Cuerpo,
    atoms: HashMap<Uuid, NarrativeAtom>,
    ide: CuerpoIde,
    clipboard: MemClipboard,
    drag_accum: (f32, f32),
    chat: Arc<dyn ChatClient>,
    en_curso: bool,
    ultimo_error: Option<String>,
    hijas: Vec<HijaZona>,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · transform sobre zona (haz que crece por zona)"
    }

    fn initial_size() -> (u32, u32) {
        (1500, 760)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let textos = [
            "El cóndor cruzó el cielo del valle al amanecer.",
            "Las llamas pastaban entre los pastizales del altiplano.",
            "Una mujer joven tejía un telar bajo el alero.",
            "El río Apurímac descendía rugiente por las rocas.",
            "Al caer la tarde, las nubes cubrieron el sol.",
            "Los kuntures alzaron vuelo hacia los nevados.",
        ];
        let atoms_vec: Vec<NarrativeAtom> = textos
            .iter()
            .map(|t| NarrativeAtom::new(*t, "es"))
            .collect();
        let mut cuerpo = Cuerpo::nuevo("es", "español (original)", Intencion::Original, 0);
        for a in &atoms_vec {
            cuerpo.agregar(a.id, 0);
        }
        let atoms: HashMap<Uuid, NarrativeAtom> =
            atoms_vec.into_iter().map(|a| (a.id, a)).collect();

        let idx: HashMap<Uuid, &NarrativeAtom> = atoms.iter().map(|(k, v)| (*k, v)).collect();
        let ide = CuerpoIde::from_cuerpo(&cuerpo, &idx);

        Model {
            cuerpo,
            atoms,
            ide,
            clipboard: MemClipboard::default(),
            drag_accum: (0.0, 0.0),
            chat: construir_chat(),
            en_curso: false,
            ultimo_error: None,
            hijas: Vec::new(),
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::EditorKey(ev) => {
                let mut model = model;
                let _ = model.ide.apply_key_with_clipboard(&ev, &mut model.clipboard);
                model
            }
            Msg::EditorPointer(ev) => {
                let mut model = model;
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
                model
            }
            Msg::ToglearFusion => {
                let mut model = model;
                if let Some(idx) = model.ide.junction_antes_del_caret() {
                    model.ide.togglear_junction(idx);
                }
                model
            }
            Msg::ZonaSiguiente => {
                let mut model = model;
                model.ide.ir_a_zona_siguiente();
                model.ide.state.ensure_caret_visible(VISIBLE_LINES);
                model
            }
            Msg::ZonaAnterior => {
                let mut model = model;
                model.ide.ir_a_zona_anterior();
                model.ide.state.ensure_caret_visible(VISIBLE_LINES);
                model
            }
            Msg::PedirTraducir(lengua) => lanzar(model, handle, TrabajoLlm::Traducir(lengua)),
            Msg::PedirTono(etiqueta) => lanzar(model, handle, TrabajoLlm::Tono(etiqueta)),
            Msg::PedirResumir(palabras) => lanzar(model, handle, TrabajoLlm::Resumir(palabras)),
            Msg::LlmListo {
                zona,
                etiqueta,
                branch,
                atoms_nuevos,
                orden,
            } => {
                let mut model = model;
                let atoms_hash: HashMap<Uuid, NarrativeAtom> =
                    atoms_nuevos.into_iter().map(|a| (a.id, a)).collect();
                model.hijas.push(HijaZona {
                    zona,
                    etiqueta,
                    branch,
                    atoms: atoms_hash,
                    orden,
                });
                model.en_curso = false;
                model
            }
            Msg::LlmError(s) => {
                let mut model = model;
                eprintln!("zona_transform_demo :: error LLM: {s}");
                model.ultimo_error = Some(s);
                model.en_curso = false;
                model
            }
        }
    }

    fn on_key(_model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        let ctrl = event.modifiers.ctrl || event.modifiers.meta;
        let shift = event.modifiers.shift;
        if ctrl {
            if let Key::Character(s) = &event.key {
                if shift && (s == "}" || s == "]") {
                    return Some(Msg::ZonaSiguiente);
                }
                if shift && (s == "{" || s == "[") {
                    return Some(Msg::ZonaAnterior);
                }
                if s.eq_ignore_ascii_case("j") {
                    return Some(Msg::ToglearFusion);
                }
            }
        }
        Some(Msg::EditorKey(event.clone()))
    }

    fn view(model: &Model) -> View<Msg> {
        let palette_editor = EditorPalette::default();
        let bg_app = palette_editor.bg;
        let fg_text = palette_editor.fg_text;
        let fg_muted = palette_editor.fg_line_number;

        let zona_caret = model.ide.zona_del_caret();
        let n_zonas = model.ide.n_zonas();
        let header_text = format!(
            "haz por zona  ·  {} átomos  ·  {} zonas  ·  caret en zona {}  ·  {}  ·  Ctrl+J fundir · Ctrl+Shift+]/[ navegar zonas",
            model.cuerpo.orden.len(),
            n_zonas,
            zona_caret,
            if model.en_curso {
                "⏳ LLM en curso…"
            } else if model.ultimo_error.is_some() {
                "⚠ error (ver footer)"
            } else {
                "listo"
            },
        );
        let header = chip(
            header_text,
            28.0,
            12.0,
            Color::from_rgba8(40, 44, 52, 255),
            fg_text,
        );

        let toolbar = toolbar_view(model.en_curso, zona_caret);

        let editor = cuerpo_ide_view::<Msg>(
            &model.ide,
            &palette_editor,
            METRICS,
            VISIBLE_LINES,
            Language::Plain,
            |ev| Some(Msg::EditorPointer(ev)),
        );

        let columna_izq = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(0.58_f32),
                height: percent(1.0_f32),
            },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(8.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        })
        .fill(bg_app)
        .children(vec![editor]);

        let panel_der = panel_hijas_view(&model.hijas, fg_text, fg_muted);

        let centro = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .fill(bg_app)
        .children(vec![columna_izq, panel_der]);

        let footer_text = model
            .ultimo_error
            .clone()
            .unwrap_or_else(|| {
                if model.hijas.is_empty() {
                    "(sin derivaciones todavía — click en un botón para transformar la zona del caret)"
                        .to_string()
                } else {
                    format!("{} hijas derivadas", model.hijas.len())
                }
            });
        let footer = chip(
            footer_text,
            24.0,
            11.0,
            Color::from_rgba8(33, 36, 42, 255),
            fg_muted,
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(bg_app)
        .children(vec![header, toolbar, centro, footer])
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

fn toolbar_view(en_curso: bool, zona_caret: usize) -> View<Msg> {
    let p_activo = ButtonPalette {
        bg: Color::from_rgba8(60, 70, 88, 255),
        bg_hover: Color::from_rgba8(85, 100, 130, 255),
        fg: Color::from_rgba8(235, 235, 245, 255),
        radius: 5.0,
    };
    let p_off = ButtonPalette {
        bg: Color::from_rgba8(60, 60, 60, 255),
        bg_hover: Color::from_rgba8(60, 60, 60, 255),
        fg: Color::from_rgba8(140, 140, 140, 255),
        radius: 5.0,
    };
    let pal = if en_curso { &p_off } else { &p_activo };

    let mk = |label: &str, m: Msg| button_view::<Msg>(label, pal, m);

    let label_zona = format!("derivar zona {}:", zona_caret);

    let etiqueta = View::new(Style {
        size: Size {
            width: length(150.0_f32),
            height: length(30.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        label_zona,
        12.0,
        Color::from_rgba8(220, 220, 220, 255),
        Alignment::Start,
    );

    let botones: Vec<View<Msg>> = vec![
        etiqueta,
        mk("→ qu", Msg::PedirTraducir("qu".into())),
        mk("→ en", Msg::PedirTraducir("en".into())),
        mk("tono formal", Msg::PedirTono("formal".into())),
        mk("resumir 30p", Msg::PedirResumir(Some(30))),
    ];

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(48.0_f32),
        },
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(28, 32, 40, 255))
    .children(botones)
}

fn panel_hijas_view(hijas: &[HijaZona], fg_text: Color, fg_muted: Color) -> View<Msg> {
    let mut cards: Vec<View<Msg>> = Vec::new();
    if hijas.is_empty() {
        cards.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(60.0_f32),
                },
                padding: Rect {
                    left: length(12.0_f32),
                    right: length(12.0_f32),
                    top: length(20.0_f32),
                    bottom: length(8.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(
                "panel hijas vacío".to_string(),
                12.0,
                fg_muted,
                Alignment::Start,
            ),
        );
    } else {
        // Las cards más recientes arriba — usuario las ve primero.
        for h in hijas.iter().rev() {
            cards.push(card_hija(h, fg_text, fg_muted));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(0.42_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(22, 26, 32, 255))
    .clip(true)
    .children(cards)
}

fn card_hija(h: &HijaZona, fg_text: Color, fg_muted: Color) -> View<Msg> {
    let head = format!("zona {} · {} · branch {}", h.zona, h.etiqueta, h.branch);
    let cuerpo: String = h
        .orden
        .iter()
        .filter_map(|id| h.atoms.get(id).map(|a| a.content.as_str()))
        .collect::<Vec<_>>()
        .join("\n\n");

    let head_view = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(4.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(head, 11.0, fg_muted, Alignment::Start);

    // Estimar alto del cuerpo en función de líneas. 16px por línea es
    // generoso pero evita scrollbars internos.
    let n_lineas = (cuerpo.matches('\n').count() + 1).max(1) as f32;
    let alto_cuerpo = (n_lineas * 16.0 + 12.0).max(40.0);

    let body_view = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(alto_cuerpo),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(2.0_f32),
            bottom: length(6.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(cuerpo, 12.0, fg_text, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(alto_cuerpo + 26.0),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(40, 46, 56, 255))
    .children(vec![head_view, body_view])
}

enum TrabajoLlm {
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
}

impl TrabajoLlm {
    fn etiqueta(&self) -> String {
        match self {
            TrabajoLlm::Traducir(l) => format!("traducir → {l}"),
            TrabajoLlm::Tono(t) => format!("tono → {t}"),
            TrabajoLlm::Resumir(Some(n)) => format!("resumir ≈{n}p"),
            TrabajoLlm::Resumir(None) => "resumir".to_string(),
        }
    }
}

/// `guardar` lite — sincroniza el buffer del IDE contra los atoms para
/// que la transformación use el texto que el usuario VE (no el original
/// de init). Igual que el `guardar` de `cuerpo_ide_demo` pero sin
/// rearmar el IDE: ese refresh confunde si lo hacemos antes de lanzar el
/// trabajo. Reflejamos cambios en `atoms` y `cuerpo.orden`, y dejamos el
/// IDE coherente vía `aplicar_cambios`.
fn sincronizar(model: &mut Model) {
    let idx: HashMap<Uuid, &NarrativeAtom> =
        model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    let cambios = model.ide.diff(&idx);
    drop(idx);
    if cambios.is_empty() {
        return;
    }
    let mut creados: Vec<Uuid> = Vec::new();
    for c in &cambios {
        match c {
            CambioAtom::Mutar { id, texto_nuevo } => {
                if let Some(a) = model.atoms.get_mut(id) {
                    a.set_content(texto_nuevo.as_str());
                }
            }
            CambioAtom::Crear { texto, posicion: _ } => {
                let atom = NarrativeAtom::new(texto.as_str(), &model.cuerpo.branch_id);
                let id = atom.id;
                model.atoms.insert(id, atom);
                creados.push(id);
            }
            CambioAtom::Eliminar { id } => {
                model.atoms.remove(id);
            }
        }
    }
    model.ide.aplicar_cambios(&cambios, &creados);
    let nuevo_orden: Vec<Uuid> = model.ide.editor_cuerpo.atom_ids.clone();
    let ahora = model.cuerpo.metadatos.modificado_en.saturating_add(1);
    let viejo: Vec<Uuid> = model.cuerpo.orden.clone();
    for id in &viejo {
        let _ = model.cuerpo.remover(*id, ahora);
    }
    for id in &nuevo_orden {
        model.cuerpo.agregar(*id, ahora);
    }
}

fn lanzar(mut model: Model, handle: &Handle<Msg>, trabajo: TrabajoLlm) -> Model {
    if model.en_curso {
        return model;
    }
    sincronizar(&mut model);

    let zona = model.ide.zona_del_caret();
    let atom_ids = match model.ide.atom_ids_de_zona(zona) {
        Some(v) if !v.is_empty() => v,
        _ => {
            model.ultimo_error = Some(format!("zona {zona} sin atoms — nada que transformar"));
            return model;
        }
    };

    // Sub-cuerpo: clonamos la madre y le reemplazamos el `orden` con los
    // atoms de la zona. Los atoms reales viven en `model.atoms` —
    // intactos.
    let mut subcuerpo = model.cuerpo.clone();
    subcuerpo.orden = atom_ids;

    let etiqueta = trabajo.etiqueta();
    let atoms_owned: Vec<NarrativeAtom> = model.atoms.values().cloned().collect();
    let chat = model.chat.clone();
    let h = handle.clone();
    let ahora = ahora_unix();

    model.en_curso = true;
    model.ultimo_error = None;

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
                        subcuerpo.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Traducir {
                            lengua_destino: lengua,
                        },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &subcuerpo, &idx, ahora).await
                }
                TrabajoLlm::Tono(etiq) => {
                    let ej = EjecutorTonoLlm::from_arc(chat, etiq.clone());
                    let t = Transformacion::nueva(
                        subcuerpo.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Tono { etiqueta: etiq },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &subcuerpo, &idx, ahora).await
                }
                TrabajoLlm::Resumir(palabras) => {
                    let ej = EjecutorResumirLlm::from_arc(chat, palabras);
                    let t = Transformacion::nueva(
                        subcuerpo.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Resumir {
                            palabras_objetivo: palabras,
                        },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &subcuerpo, &idx, ahora).await
                }
            }
        });

        let _ = h;
        match resultado {
            Ok(prod) => Msg::LlmListo {
                zona,
                etiqueta,
                branch: prod.hija.branch_id.clone(),
                atoms_nuevos: prod.atoms_nuevos,
                orden: prod.hija.orden,
            },
            Err(e) => Msg::LlmError(format!("{e:?}")),
        }
    });

    model
}

fn construir_chat() -> Arc<dyn ChatClient> {
    let usa_mock = std::env::var("ANTHROPIC_API_KEY").is_err()
        && std::env::var("GEMINI_API_KEY").is_err()
        && std::env::var("GOOGLE_API_KEY").is_err()
        && std::env::var("DEEPSEEK_API_KEY").is_err()
        && std::env::var("COHERE_API_KEY").is_err()
        && std::env::var("PLUMA_LLM_BACKEND")
            .map(|s| s.to_lowercase() != "ollama")
            .unwrap_or(true);
    if usa_mock {
        let mut mock = pluma_llm_mock::MockChatClient::default().con_model_id("mock-zona");
        // Respuestas pre-pobladas — substring → respuesta. Si la zona no
        // contiene ninguna de estas, el mock responde con un placeholder
        // genérico para que el demo igual muestre algo.
        for (k, v) in [
            ("cóndor cruzó", "Kuntur wayqu hanaqpachata pacha paqarinpi pasarqa."),
            ("Las llamas pastaban", "Llamakuna qulla suyup q'achunpi mikhusharqaku."),
            ("mujer joven tejía", "Sipas warmi away wasiq hawanpi awayta ruwasharqa."),
            ("río Apurímac", "Apurímac mayu rumikunaq ukhunpita uraykachisharqa."),
            ("Al caer la tarde", "Inti waykuyninpi phuyukuna intita pakarqa."),
            ("kuntures alzaron", "Kunturkunaqa riti urqukunaman phawarqaku."),
        ] {
            mock = mock.con_respuesta(k, v);
        }
        return Arc::new(mock);
    }
    llm_from_env().expect("from_env")
}

fn ahora_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn main() {
    llimphi_ui::run::<Demo>();
}
