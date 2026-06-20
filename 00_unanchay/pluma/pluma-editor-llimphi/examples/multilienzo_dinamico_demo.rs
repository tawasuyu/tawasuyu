//! Demo del multilienzo con transformaciones disparadas desde la UI.
//!
//! Cuerpo `es` cargado al inicio. Toolbar arriba con cuatro botones:
//! `→ qu`, `→ en`, `tono formal`, `resumir`. Click → spawn thread que
//! corre el ejecutor LLM transparente → al volver, dispatch del
//! resultado al `update` → aparece una columna nueva con hebras
//! Derivadas.
//!
//! Mientras una transformación está en curso, los botones quedan
//! deshabilitados (un solo trabajo a la vez — evita que clicks
//! repetidos disparen N requests en paralelo).
//!
//! ```bash
//! # Con LLM real:
//! GEMINI_API_KEY=... PLUMA_LLM_BACKEND=gemini \
//!   cargo run -p pluma-editor-llimphi --example multilienzo_dinamico_demo --release
//!
//! # Sin keys: mock pre-poblado con respuestas predecibles.
//! cargo run -p pluma-editor-llimphi --example multilienzo_dinamico_demo --release
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, FlexDirection, Rect, Size, Style,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::multilienzo::{
    multilienzo_view, IndiceAtoms, MultilienzoConfig, PaletaHebras,
};
use pluma_editor_llimphi::Palette;
use pluma_graph::NarrativeGraph;
use pluma_llm::from_env as llm_from_env;
use pluma_llm_core::ChatClient;
use pluma_transform::{TipoTransformacion, Transformacion};
use pluma_transform_llm::{
    EjecutorResumirLlm, EjecutorTonoLlm, EjecutorTraducirLlm,
};
use uuid::Uuid;

#[derive(Clone, Debug)]
enum Msg {
    PedirTraducir(String),
    PedirTono(String),
    PedirResumir(Option<u32>),
    LlmListo {
        hija: Cuerpo,
        atoms_nuevos: Vec<NarrativeAtom>,
        carta: CartaHebras,
    },
    LlmError(String),
    /// Pulso de animación del flujo (~33 Hz). Avanza `fase_flujo` mientras
    /// haya algo que fluir (transformación en curso o burst de llegada).
    Tick,
}

struct Model {
    cuerpos: Vec<Cuerpo>,
    graph: NarrativeGraph,
    cartas: Vec<CartaHebras>,
    chat: Arc<dyn ChatClient>,
    en_curso: bool,
    ultimo_error: Option<String>,
    /// Fase del flujo en `[0, 1)`; corre mientras `en_curso` o `burst > 0`.
    fase_flujo: f32,
    /// Ticks restantes del "burst" de llegada: cuando una columna nueva
    /// aterriza, el flujo sigue corriendo unos frames más para que se vea
    /// la corriente surcar el haz recién nacido antes de apagarse.
    burst: u32,
    /// El ticker periódico se arma una sola vez (en el primer trabajo) y
    /// vive lo que resta del proceso; este flag evita re-armarlo.
    ticker_armado: bool,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · multilienzo dinámico (botones LLM)"
    }

    fn initial_size() -> (u32, u32) {
        (1400, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let mut graph = NarrativeGraph::new();
        let mut es = Cuerpo::nuevo("es", "español (original)", Intencion::Original, 100);
        for t in [
            "El cóndor cruzó el cielo del valle al amanecer.",
            "Las llamas pastaban entre los pastizales del altiplano.",
            "Una mujer joven tejía un telar bajo el alero.",
        ] {
            let atom = NarrativeAtom::new(t, "es");
            es.agregar(atom.id, 101);
            graph.insert(atom);
        }
        Model {
            cuerpos: vec![es],
            graph,
            cartas: Vec::new(),
            chat: construir_chat(),
            en_curso: false,
            ultimo_error: None,
            fase_flujo: 0.0,
            burst: 0,
            ticker_armado: false,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::PedirTraducir(lengua) => {
                if m.en_curso || m.cuerpos.is_empty() {
                    return m;
                }
                m.en_curso = true;
                m.ultimo_error = None;
                lanzar_trabajo(&mut m, handle, TrabajoLlm::Traducir(lengua));
            }
            Msg::PedirTono(etiqueta) => {
                if m.en_curso || m.cuerpos.is_empty() {
                    return m;
                }
                m.en_curso = true;
                m.ultimo_error = None;
                lanzar_trabajo(&mut m, handle, TrabajoLlm::Tono(etiqueta));
            }
            Msg::PedirResumir(palabras) => {
                if m.en_curso || m.cuerpos.is_empty() {
                    return m;
                }
                m.en_curso = true;
                m.ultimo_error = None;
                lanzar_trabajo(&mut m, handle, TrabajoLlm::Resumir(palabras));
            }
            Msg::LlmListo { hija, atoms_nuevos, carta } => {
                for atom in atoms_nuevos {
                    m.graph.insert(atom);
                }
                m.cuerpos.push(hija);
                m.cartas.push(carta);
                m.en_curso = false;
                // Burst de llegada: ~1.4 s de flujo extra para ver la
                // corriente surcar el haz recién nacido antes de apagarse.
                m.burst = 45;
            }
            Msg::LlmError(s) => {
                eprintln!("multilienzo_dinamico_demo :: error LLM: {s}");
                m.ultimo_error = Some(s);
                m.en_curso = false;
            }
            Msg::Tick => {
                if m.en_curso || m.burst > 0 {
                    // ~33 Hz · período de flujo ~2.4 s (0.0125 por tick).
                    m.fase_flujo = (m.fase_flujo + 0.0125).rem_euclid(1.0);
                    m.burst = m.burst.saturating_sub(1);
                }
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let cfg = MultilienzoConfig {
            // Flujo encendido mientras se transforma o durante el burst de
            // llegada; apagado en reposo (los haces quedan estáticos).
            mostrar_flujo: model.en_curso || model.burst > 0,
            fase_flujo: model.fase_flujo,
            ..MultilienzoConfig::default()
        };
        let paleta_hebras = PaletaHebras::default();
        let palette = Palette::default();

        let index: IndiceAtoms = model.graph.atoms().map(|a| (a.id, a)).collect();
        let cuerpos_ref: Vec<&Cuerpo> = model.cuerpos.iter().collect();
        let cartas_ref: Vec<Option<&CartaHebras>> = model.cartas.iter().map(Some).collect();

        let cuerpos_view = multilienzo_view::<Msg>(
            &cuerpos_ref,
            &index,
            &cartas_ref,
            &cfg,
            &paleta_hebras,
            &palette,
        );

        let toolbar = toolbar_view::<Msg>(&palette, model.en_curso, &model.ultimo_error);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(palette.bg_app)
        .clip(true)
        .children(vec![toolbar, cuerpos_view])
    }
}

enum TrabajoLlm {
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
}

/// Encola el trabajo LLM en un thread aparte. Capturamos un snapshot
/// owned de la madre + atoms; el handle dispatcha el resultado al
/// volver al hilo de UI.
fn lanzar_trabajo(model: &mut Model, handle: &Handle<Msg>, trabajo: TrabajoLlm) {
    // Arma el ticker del flujo la primera vez que se dispara un trabajo:
    // ~33 Hz, perpetuo (spawn_periodic no se cancela), pero el `update`
    // sólo avanza la fase cuando hay algo que fluir.
    if !model.ticker_armado {
        model.ticker_armado = true;
        handle.spawn_periodic(std::time::Duration::from_millis(30), || Msg::Tick);
    }
    // Madre = primer cuerpo (la "raíz" del haz en este demo).
    let madre = model.cuerpos[0].clone();
    // Snapshot owned de los atoms para sobrevivir al thread.
    let atoms_owned: Vec<NarrativeAtom> = model.graph.atoms().cloned().collect();
    let chat = model.chat.clone();
    let h = handle.clone();
    let ahora = ahora_unix();

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
                        TipoTransformacion::Traducir { lengua_destino: lengua },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora).await
                }
                TrabajoLlm::Tono(etiqueta) => {
                    let ej = EjecutorTonoLlm::from_arc(chat, etiqueta.clone());
                    let t = Transformacion::nueva(
                        madre.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Tono { etiqueta },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora).await
                }
                TrabajoLlm::Resumir(palabras) => {
                    let ej = EjecutorResumirLlm::from_arc(chat, palabras);
                    let t = Transformacion::nueva(
                        madre.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Resumir { palabras_objetivo: palabras },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora).await
                }
            }
        });

        let _ = h; // handle ya no se usa más; el Msg de retorno lo entrega el runtime.
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

fn toolbar_view<Msg: Clone + 'static>(
    palette: &Palette,
    en_curso: bool,
    ultimo_error: &Option<String>,
) -> View<Msg>
where
    Msg: From<MsgUi>,
{
    let p_activo = ButtonPalette {
        bg: palette.bg_panel,
        bg_hover: palette.border_strong,
        fg: palette.fg_text,
        radius: 5.0,
    };
    let p_desactivado = ButtonPalette {
        bg: Color::from_rgba8(60, 60, 60, 255),
        bg_hover: Color::from_rgba8(60, 60, 60, 255),
        fg: palette.fg_muted,
        radius: 5.0,
    };
    let pal = if en_curso { &p_desactivado } else { &p_activo };

    let mut botones: Vec<View<Msg>> = Vec::new();
    let mk = |label: &str, m: MsgUi| {
        button_view::<Msg>(label, pal, m.into())
    };
    botones.push(env(mk("→ qu", MsgUi::Traducir("qu".into()))));
    botones.push(env(mk("→ en", MsgUi::Traducir("en".into()))));
    botones.push(env(mk("tono formal", MsgUi::Tono("formal".into()))));
    botones.push(env(mk("resumir 30p", MsgUi::Resumir(Some(30)))));

    let status_text = if en_curso {
        "⏳ en curso…".to_string()
    } else if let Some(e) = ultimo_error {
        format!("⚠ {}", &e[..e.len().min(80)])
    } else {
        "listo — click para derivar un cuerpo nuevo".to_string()
    };
    let status = View::new(Style {
        size: Size {
            width: length(360.0_f32),
            height: length(30.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(status_text, 12.0, palette.fg_muted, Alignment::Start);
    botones.push(status);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(46.0_f32),
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
    .fill(palette.bg_panel)
    .children(botones)
}

/// Helper para que el genérico funcione: el toolbar es genérico sobre
/// `Msg: From<MsgUi>`, así reusable si algún día se monta dentro de una
/// app más grande. En este demo, `MsgUi == app::Msg`.
fn env<T>(v: T) -> T {
    v
}

#[derive(Clone, Debug)]
enum MsgUi {
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
}

impl From<MsgUi> for Msg {
    fn from(u: MsgUi) -> Self {
        match u {
            MsgUi::Traducir(l) => Msg::PedirTraducir(l),
            MsgUi::Tono(e) => Msg::PedirTono(e),
            MsgUi::Resumir(p) => Msg::PedirResumir(p),
        }
    }
}

fn construir_chat() -> Arc<dyn ChatClient> {
    let usa_mock = std::env::var("ANTHROPIC_API_KEY").is_err()
        && std::env::var("GEMINI_API_KEY").is_err()
        && std::env::var("GOOGLE_API_KEY").is_err()
        && std::env::var("DEEPSEEK_API_KEY").is_err()
        && std::env::var("PLUMA_LLM_BACKEND")
            .map(|s| s.to_lowercase() != "ollama")
            .unwrap_or(true);
    if usa_mock {
        let mut mock = pluma_llm_mock::MockChatClient::default()
            .con_model_id("mock-demo");
        // Mock pre-poblado por substring → respuesta. Suficiente para
        // mostrar el flujo aun sin red.
        for (k, v) in [
            ("cóndor cruzó", "Kuntur wayqu hanaqpachata pacha paqarinpi pasarqa."),
            ("Las llamas pastaban", "Llamaqakuna qulla suyup q'achupinpi mikhusharqaku."),
            ("mujer joven tejía", "Sipas warmiq away wasiq hawanpi awayta ruwasharqa."),
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
