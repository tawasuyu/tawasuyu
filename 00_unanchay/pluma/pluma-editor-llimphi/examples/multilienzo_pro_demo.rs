//! `multilienzo_pro` — la cara profesional del multilienzo de pluma.
//!
//! Reúne, sobre el mismo lienzo de cuerpos paralelos:
//!   - **Toolbar gráfico** (iconos vectoriales `llimphi-icons` +
//!     `llimphi-widget-toolbar`), agrupado por familia de acción.
//!   - **Conectores con efectos**: las hebras entre párrafos se pintan como
//!     curvas en S con halo, grosor modulado por confianza y nodos en los
//!     extremos (ver `multilienzo::carril_hebras`).
//!   - **Zoom de fuente**: A−/A+ reescalan todo el multilienzo (bloques,
//!     columnas, carriles, tipografía) de forma proporcional.
//!   - **Inclusión múltiple de `.docx`**: cada click en "incluir" agrega el
//!     siguiente documento de la lista `PLUMA_DOCX` como una columna nueva.
//!   - **DOCX pareado**: "emparejar" reconstruye las hebras posición-a-
//!     posición entre columnas consecutivas — el caso clásico de dos `.docx`
//!     paralelos (original ↔ traducción) que querés alinear y ver juntos.
//!   - **Exportar `.docx`**: vuelca la última columna a Office Open XML.
//!   - **Transformaciones LLM** (→qu / →en / tono / resumir) que derivan
//!     columnas nuevas con sus hebras `Derivado`.
//!
//! ```bash
//! # Standalone (mock, columnas sembradas):
//! cargo run -p pluma-editor-llimphi --example multilienzo_pro_demo --release
//!
//! # Con documentos reales — el primero es la madre, el resto se incluyen
//! # con el botón "incluir" (uno por click):
//! PLUMA_DOCX="original.docx,traduccion.docx,resumen.docx" \
//!   cargo run -p pluma-editor-llimphi --example multilienzo_pro_demo --release
//!
//! # Con LLM real:
//! ANTHROPIC_API_KEY=... PLUMA_LLM_BACKEND=anthropic \
//!   cargo run -p pluma-editor-llimphi --example multilienzo_pro_demo --release
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, FlexDirection, Position, Rect, Size, Style,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Modifiers, View, WheelDelta};
use llimphi_icons::{icon_view, Icon};
use llimphi_widget_toolbar::{toolbar_view, ToolbarGroup, ToolbarItem, ToolbarPalette};

use pluma_align::{alinear_uno_a_uno, CartaHebras, OrigenAlineamiento};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::multilienzo::{
    multilienzo_view_resaltado, IndiceAtoms, MultilienzoConfig, PaletaHebras,
};
use pluma_editor_llimphi::Palette;
use pluma_graph::NarrativeGraph;
use pluma_llm::{from_env as llm_from_env, BackendKind};
use pluma_llm_core::ChatClient;
use pluma_transform::{TipoTransformacion, Transformacion};
use pluma_transform_llm::{EjecutorResumirLlm, EjecutorTonoLlm, EjecutorTraducirLlm};
use uuid::Uuid;

#[derive(Clone, Debug)]
enum Msg {
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
    LlmListo {
        hija: Cuerpo,
        atoms_nuevos: Vec<NarrativeAtom>,
        carta: CartaHebras,
    },
    LlmError(String),
    /// Incluye el siguiente `.docx` pendiente de `PLUMA_DOCX` como columna.
    IncluirSiguienteDocx,
    /// Reconstruye TODAS las cartas posición-a-posición entre columnas
    /// consecutivas — el "pareado" de documentos importados.
    Emparejar,
    /// Exporta la última columna a un `.docx`.
    ExportarDocx,
    /// Zoom de fuente: delta sobre la escala (clampeada en update).
    Zoom(f32),
    ToggleSoloMadre,
    Scroll(f32, f32),
}

struct Model {
    cuerpos: Vec<Cuerpo>,
    graph: NarrativeGraph,
    /// `cartas[i]` une `cuerpos[i]` con `cuerpos[i+1]`.
    cartas: Vec<CartaHebras>,
    chat: Arc<dyn ChatClient>,
    backend: BackendKind,
    en_curso: bool,
    estado: String,
    /// `.docx` aún sin incluir (de `PLUMA_DOCX`, en orden).
    docx_pendientes: Vec<PathBuf>,
    /// Escala de zoom; 1.0 = default. Reescala todo el multilienzo.
    escala: f32,
    solo_madre: bool,
    scroll_x: f32,
    scroll_y: f32,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · multilienzo pro"
    }

    fn initial_size() -> (u32, u32) {
        (1460, 820)
    }

    fn on_wheel(
        _m: &Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        modifiers: Modifiers,
    ) -> Option<Msg> {
        const PX: f32 = 32.0;
        // Ctrl + rueda = zoom; rueda normal = scroll vertical; Shift = horiz.
        if modifiers.ctrl {
            let paso = if delta.y > 0.0 { 0.1 } else { -0.1 };
            return Some(Msg::Zoom(paso));
        }
        if delta.x.abs() > 0.0 {
            return Some(Msg::Scroll(-delta.x * PX, 0.0));
        }
        if modifiers.shift {
            return Some(Msg::Scroll(-delta.y * PX, 0.0));
        }
        Some(Msg::Scroll(0.0, -delta.y * PX))
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

        let mut graph = NarrativeGraph::new();
        let mut cuerpos: Vec<Cuerpo> = Vec::new();
        let mut pendientes: Vec<PathBuf> = Vec::new();

        if let Some((primero, resto)) = docx.split_first() {
            match cargar_docx(primero, &mut graph) {
                Ok(c) => {
                    eprintln!("multilienzo_pro :: madre desde {}", primero.display());
                    cuerpos.push(c);
                    pendientes = resto.to_vec();
                }
                Err(e) => {
                    eprintln!("multilienzo_pro :: no se pudo abrir {}: {e} — sembrando demo", primero.display());
                    cuerpos.push(sembrar_madre(&mut graph));
                }
            }
        } else {
            cuerpos.push(sembrar_madre(&mut graph));
        }

        Model {
            cuerpos,
            graph,
            cartas: Vec::new(),
            chat,
            backend,
            en_curso: false,
            estado: String::new(),
            docx_pendientes: pendientes,
            escala: 1.0,
            solo_madre: false,
            scroll_x: 0.0,
            scroll_y: 0.0,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Traducir(l) => arrancar(&mut m, handle, Trabajo::Traducir(l)),
            Msg::Tono(e) => arrancar(&mut m, handle, Trabajo::Tono(e)),
            Msg::Resumir(p) => arrancar(&mut m, handle, Trabajo::Resumir(p)),
            Msg::LlmListo { hija, atoms_nuevos, carta } => {
                for a in atoms_nuevos {
                    m.graph.insert(a);
                }
                m.cuerpos.push(hija);
                m.cartas.push(carta);
                m.en_curso = false;
                m.estado = format!("{} columnas", m.cuerpos.len());
            }
            Msg::LlmError(e) => {
                m.en_curso = false;
                m.estado = format!("⚠ {}", &e[..e.len().min(90)]);
            }
            Msg::IncluirSiguienteDocx => {
                if m.docx_pendientes.is_empty() {
                    m.estado = "no quedan .docx por incluir (PLUMA_DOCX)".into();
                    return m;
                }
                let path = m.docx_pendientes.remove(0);
                match cargar_docx(&path, &mut m.graph) {
                    Ok(nuevo) => {
                        // Empareja posición-a-posición con la última columna.
                        if let Some(prev) = m.cuerpos.last() {
                            let carta = alinear_uno_a_uno(
                                prev,
                                &nuevo,
                                OrigenAlineamiento::Manual {
                                    autor: "docx-pareado".into(),
                                    timestamp: ahora_unix(),
                                },
                            );
                            m.cartas.push(carta);
                        }
                        m.cuerpos.push(nuevo);
                        m.estado = format!(
                            "incluido {} · {} columnas · {} pendientes",
                            path.display(),
                            m.cuerpos.len(),
                            m.docx_pendientes.len()
                        );
                    }
                    Err(e) => m.estado = format!("⚠ {}: {e}", path.display()),
                }
            }
            Msg::Emparejar => {
                m.cartas.clear();
                for w in m.cuerpos.windows(2) {
                    m.cartas.push(alinear_uno_a_uno(
                        &w[0],
                        &w[1],
                        OrigenAlineamiento::Manual {
                            autor: "emparejado".into(),
                            timestamp: ahora_unix(),
                        },
                    ));
                }
                m.estado = format!("emparejadas {} carta(s) posicionales", m.cartas.len());
            }
            Msg::ExportarDocx => {
                let Some(cuerpo) = m.cuerpos.last() else {
                    return m;
                };
                let idx: HashMap<Uuid, &NarrativeAtom> =
                    m.graph.atoms().map(|a| (a.id, a)).collect();
                let salida = std::env::var("PLUMA_DOCX_OUT")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| {
                        let base = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                        PathBuf::from(base).join("pluma-export.docx")
                    });
                match foreign_docx::write_docx_borrow(cuerpo, &idx) {
                    Ok(bytes) => match std::fs::write(&salida, bytes) {
                        Ok(()) => m.estado = format!("exportado → {}", salida.display()),
                        Err(e) => m.estado = format!("⚠ escribir docx: {e}"),
                    },
                    Err(e) => m.estado = format!("⚠ generar docx: {e:?}"),
                }
            }
            Msg::Zoom(d) => {
                m.escala = (m.escala + d).clamp(0.6, 2.4);
                m.estado = format!("zoom {:.0}%", m.escala * 100.0);
            }
            Msg::ToggleSoloMadre => m.solo_madre = !m.solo_madre,
            Msg::Scroll(dx, dy) => {
                m.scroll_x = (m.scroll_x + dx).max(0.0);
                m.scroll_y = (m.scroll_y + dy).max(0.0);
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let palette = Palette::default();
        let cfg = MultilienzoConfig::con_escala(model.escala);
        let paleta_h = PaletaHebras::default();
        let index: IndiceAtoms = model.graph.atoms().map(|a| (a.id, a)).collect();

        let cuerpos_ref: Vec<&Cuerpo> = if model.solo_madre {
            model.cuerpos.iter().take(1).collect()
        } else {
            model.cuerpos.iter().collect()
        };
        let cartas_ref: Vec<Option<&CartaHebras>> = if model.solo_madre {
            Vec::new()
        } else {
            model.cartas.iter().map(Some).collect()
        };

        let interior = multilienzo_view_resaltado::<Msg>(
            &cuerpos_ref,
            &index,
            &cartas_ref,
            &cfg,
            &paleta_h,
            &palette,
            "",
        );

        let lienzo = View::new(Style {
            position: Position::Relative,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .children(vec![View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(-model.scroll_x),
                top: length(-model.scroll_y),
                right: auto(),
                bottom: auto(),
            },
            ..Default::default()
        })
        .children(vec![interior])]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(palette.bg_app)
        .clip(true)
        .children(vec![barra(model), lienzo, status(model, &palette)])
    }
}

/// Toolbar gráfico: grupos de botones-ícono.
fn barra(model: &Model) -> View<Msg> {
    let pal = ToolbarPalette::default();
    let trabajando = model.en_curso;
    let hay_pendientes = !model.docx_pendientes.is_empty();
    let multi = model.cuerpos.len() >= 2;

    let grupos = vec![
        // Documentos: incluir / emparejar / exportar.
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
        // Zoom de fuente.
        ToolbarGroup::new(vec![
            ToolbarItem::new(|_, c| icon_view(Icon::Minus, c, 1.9), Msg::Zoom(-0.15))
                .enabled(model.escala > 0.6),
            ToolbarItem::new(|_, c| icon_view(Icon::Plus, c, 1.9), Msg::Zoom(0.15))
                .with_label(&format!("{:.0}%", model.escala * 100.0))
                .enabled(model.escala < 2.4),
        ]),
        // Vista.
        ToolbarGroup::new(vec![
            ToolbarItem::new(|_, c| icon_view(Icon::Rows, c, 1.7), Msg::ToggleSoloMadre)
                .with_label("foco")
                .active(model.solo_madre),
        ]),
        // Transformaciones LLM.
        ToolbarGroup::new(vec![
            ToolbarItem::new(|_, c| icon_view(Icon::Font, c, 1.7), Msg::Traducir("qu".into()))
                .with_label("→qu")
                .enabled(!trabajando),
            ToolbarItem::new(|_, c| icon_view(Icon::Font, c, 1.7), Msg::Traducir("en".into()))
                .with_label("→en")
                .enabled(!trabajando),
            ToolbarItem::new(|_, c| icon_view(Icon::Edit, c, 1.7), Msg::Tono("formal".into()))
                .with_label("tono")
                .enabled(!trabajando),
            ToolbarItem::new(|_, c| icon_view(Icon::FileText, c, 1.7), Msg::Resumir(Some(30)))
                .with_label("resumir")
                .enabled(!trabajando),
        ]),
    ];

    toolbar_view(grupos, 40.0, &pal)
}

/// Barra de estado inferior: contexto + último mensaje de la app.
fn status(model: &Model, palette: &Palette) -> View<Msg> {
    let texto = if model.en_curso {
        format!("⏳ LLM en curso · {} ({})", etiqueta_backend(model.backend), model.chat.model_id())
    } else if model.estado.is_empty() {
        format!(
            "{} columnas · {} cartas · modelo {} · Ctrl+rueda = zoom",
            model.cuerpos.len(),
            model.cartas.len(),
            etiqueta_backend(model.backend),
        )
    } else {
        model.estado.clone()
    };
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0) },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .text_aligned(texto, 11.5, palette.fg_muted, Alignment::Start)
}

// ---------------------------------------------------------------------------
// LLM
// ---------------------------------------------------------------------------

enum Trabajo {
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
}

fn arrancar(m: &mut Model, handle: &Handle<Msg>, trabajo: Trabajo) {
    if m.en_curso || m.cuerpos.is_empty() {
        return;
    }
    m.en_curso = true;
    m.estado.clear();
    let madre = m.cuerpos[0].clone();
    let atoms_owned: Vec<NarrativeAtom> = m.graph.atoms().cloned().collect();
    let chat = m.chat.clone();
    let ahora = ahora_unix();

    handle.spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => return Msg::LlmError(format!("runtime tokio: {e}")),
        };
        let idx: HashMap<Uuid, &NarrativeAtom> =
            atoms_owned.iter().map(|a| (a.id, a)).collect();

        let resultado = rt.block_on(async {
            match trabajo {
                Trabajo::Traducir(l) => {
                    let ej = EjecutorTraducirLlm::from_arc(chat, l.clone());
                    let t = Transformacion::nueva(
                        madre.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Traducir { lengua_destino: l },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora).await
                }
                Trabajo::Tono(e) => {
                    let ej = EjecutorTonoLlm::from_arc(chat, e.clone());
                    let t = Transformacion::nueva(
                        madre.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Tono { etiqueta: e },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora).await
                }
                Trabajo::Resumir(p) => {
                    let ej = EjecutorResumirLlm::from_arc(chat, p);
                    let t = Transformacion::nueva(
                        madre.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Resumir { palabras_objetivo: p },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora).await
                }
            }
        });

        match resultado {
            Ok(prod) => Msg::LlmListo {
                hija: prod.hija,
                atoms_nuevos: prod.atoms_nuevos,
                carta: prod.carta,
            },
            Err(e) => Msg::LlmError(format!("{e:?}")),
        }
    });
}

// ---------------------------------------------------------------------------
// docx + seed + chat
// ---------------------------------------------------------------------------

fn cargar_docx(path: &PathBuf, graph: &mut NarrativeGraph) -> Result<Cuerpo, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let nombre = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("docx")
        .to_string();
    let imp = foreign_docx::parse_docx(&bytes, nombre.clone(), nombre, ahora_unix())
        .map_err(|e| format!("{e}"))?;
    for atom in imp.atoms {
        graph.insert(atom);
    }
    Ok(imp.cuerpo)
}

fn sembrar_madre(graph: &mut NarrativeGraph) -> Cuerpo {
    let mut es = Cuerpo::nuevo("es", "español (original)", Intencion::Original, 100);
    for t in [
        "El cóndor cruzó el cielo del valle al amanecer.",
        "Las llamas pastaban entre los pastizales del altiplano.",
        "Una mujer joven tejía un telar bajo el alero.",
        "El río bajaba turbio tras la lluvia de la noche.",
    ] {
        let atom = NarrativeAtom::new(t, "es");
        es.agregar(atom.id, 101);
        graph.insert(atom);
    }
    es
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
    let backend = std::env::var("PLUMA_LLM_BACKEND")
        .ok()
        .and_then(|s| BackendKind::parse(&s))
        .unwrap_or(BackendKind::Anthropic);
    (llm_from_env().expect("from_env"), backend)
}

fn ahora_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn main() {
    llimphi_ui::run::<Demo>();
}
