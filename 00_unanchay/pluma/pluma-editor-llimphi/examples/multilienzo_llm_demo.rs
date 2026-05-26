//! Demo end-to-end del multilienzo con LLM real + persistencia en grafo.
//!
//! Cuatro piezas en línea:
//!
//!   1. **`pluma_llm::from_env`** elige backend transparente —
//!      Anthropic/Gemini/DeepSeek/Ollama/Mock según `PLUMA_LLM_BACKEND` o
//!      la primera env key disponible; fallback a Mock para que el demo
//!      arranque sin nada configurado.
//!   2. **`EjecutorTraducirLlm::from_arc`** + Anthropic system-cached
//!      genera la traducción es→qu párrafo por párrafo.
//!   3. **`pluma_graph_transform::persistir_producto`** mete los atoms
//!      nuevos en el `NarrativeGraph`. Sin esto, la hija sería un cuerpo
//!      con Uuids huérfanos.
//!   4. **`pluma_align_embeddings::alinear_por_embeddings`** + `verbo-daemon`
//!      si está corriendo, sino MockProvider — calcula hebras qu↔en.
//!
//! Cómo correrlo:
//!
//! ```bash
//! # Mock (sin red, sin keys, eco predecible — siempre funciona):
//! cargo run -p pluma-editor-llimphi --example multilienzo_llm_demo --release
//!
//! # Anthropic real (necesita ANTHROPIC_API_KEY):
//! ANTHROPIC_API_KEY=sk-ant-... \
//!   cargo run -p pluma-editor-llimphi --example multilienzo_llm_demo --release
//!
//! # Gemini:
//! GEMINI_API_KEY=... PLUMA_LLM_BACKEND=gemini \
//!   cargo run -p pluma-editor-llimphi --example multilienzo_llm_demo --release
//!
//! # DeepSeek:
//! DEEPSEEK_API_KEY=... PLUMA_LLM_BACKEND=deepseek \
//!   cargo run -p pluma-editor-llimphi --example multilienzo_llm_demo --release
//!
//! # Ollama 100% local (necesita `ollama serve` y `ollama pull llama3.1`):
//! PLUMA_LLM_BACKEND=ollama PLUMA_LLM_MODEL=llama3.1 \
//!   cargo run -p pluma-editor-llimphi --example multilienzo_llm_demo --release
//!
//! # Embeddings reales en lugar de mock:
//! verbo-daemon --provider fastembed &
//! # ...y volver a lanzar el demo
//! ```

use std::path::{Path, PathBuf};

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, FlexDirection, Size, Style};
use llimphi_ui::{App, Handle, View};
use pluma_align::CartaHebras;
use pluma_align_embeddings::{alinear_por_embeddings, ModoAlineacion, ParamsAlineacion};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::multilienzo::{
    multilienzo_view, IndiceAtoms, MultilienzoConfig, PaletaHebras,
};
use pluma_editor_llimphi::Palette;
use pluma_graph::NarrativeGraph;
use pluma_graph_transform::{indice_atoms, persistir_producto};
use pluma_llm::{from_env as llm_from_env, BackendKind, LlmConfig};
use pluma_llm_core::ChatClient;
use pluma_transform::{TipoTransformacion, Transformacion};
use pluma_transform_llm::EjecutorTraducirLlm;
use rimay_verbo_core::Provider;
use rimay_verbo_daemon::DaemonClient;
use rimay_verbo_mock::MockProvider;
use uuid::Uuid;

#[derive(Clone, Debug)]
enum Msg {}

struct Model {
    cuerpos: Vec<Cuerpo>,
    graph: NarrativeGraph,
    cartas: Vec<CartaHebras>,
    label_llm: String,
    label_provider: String,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · multilienzo llm demo (es → qu via LLM real)"
    }

    fn initial_size() -> (u32, u32) {
        (1280, 680)
    }

    fn init(_: &Handle<Msg>) -> Model {
        // -- 0. Runtime async compartido ---------------------------------------
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime tokio");

        // -- 1. Madre es + grafo -----------------------------------------------
        let textos_es = [
            "El cóndor cruzó el cielo del valle al amanecer.",
            "Las llamas pastaban entre los pastizales del altiplano.",
            "Una mujer joven tejía un telar bajo el alero.",
            "El río Apurímac descendía rugiente por las rocas.",
            "Al caer la tarde, las nubes cubrieron el sol.",
        ];
        let mut graph = NarrativeGraph::new();
        let mut es = Cuerpo::nuevo("es", "español (original)", Intencion::Original, 100);
        for t in textos_es {
            let atom = NarrativeAtom::new(t, "es");
            es.agregar(atom.id, 101);
            graph.insert(atom);
        }

        // -- 2. Chat client transparente — quien sea el backend de turno ------
        // `from_env` cae a Mock si nada está configurado. Para mock con
        // respuestas razonables, sustituyo por un mock pre-poblado con
        // traducciones predecibles. Eso permite ver el demo "como si"
        // hubiera traducido sin red.
        let (chat, label_llm) = construir_chat_para_demo(&textos_es);
        eprintln!("multilienzo_llm_demo :: LLM = {label_llm}");

        // -- 3. Traducir es → qu vía LLM, persistir en grafo -----------------
        let ejecutor = EjecutorTraducirLlm::from_arc(chat, "qu");
        let t_qu = Transformacion::nueva(
            es.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "qu".into() },
            "demo",
            200,
        );
        let (qu, mut carta_es_qu) = rt.block_on(async {
            let idx = indice_atoms(&graph);
            let producto = ejecutor
                .aplicar_con_atoms(&t_qu, &es, &idx, 200)
                .await
                .expect("traducción LLM");
            drop(idx);
            persistir_producto(&mut graph, producto)
        });
        // Una hebra marcada stale para mostrar el efecto visual.
        if let Some(h) = carta_es_qu.hebras.get_mut(0) {
            h.fresco = false;
        }

        // -- 4. Cuerpo en (resumen, 2 párrafos manuales) ---------------------
        let textos_en = [
            "Dawn over the highlands — condor, llamas, weaver.",
            "By dusk, the Apurímac roared and the clouds hid the sun.",
        ];
        let mut en = Cuerpo::nuevo(
            "en",
            "english (résumé)",
            Intencion::Resumen { palabras_objetivo: Some(40) },
            200,
        );
        for t in textos_en {
            let atom = NarrativeAtom::new(t, "en");
            en.agregar(atom.id, 201);
            graph.insert(atom);
        }

        // -- 5. Hebras qu↔en por embeddings — daemon si existe, mock si no ---
        let socket = socket_verbo_default();
        let (carta_qu_en, label_provider) = rt.block_on(async {
            let idx = indice_atoms(&graph);
            match conectar_daemon_si_existe(&socket).await {
                Some(daemon) => {
                    let label = format!(
                        "verbo-daemon @ {} ({})",
                        socket.display(),
                        daemon.model_id()
                    );
                    let params = ParamsAlineacion {
                        umbral_minimo: 0.5,
                        modo: ModoAlineacion::MejorParaCadaA,
                    };
                    let carta = alinear_por_embeddings(&qu, &en, &idx, &daemon, &params, 200)
                        .await
                        .expect("embeddings (daemon)");
                    (carta, label)
                }
                None => {
                    let mock = MockProvider::default();
                    let params = ParamsAlineacion {
                        umbral_minimo: -1.0,
                        modo: ModoAlineacion::MejorParaCadaA,
                    };
                    let carta = alinear_por_embeddings(&qu, &en, &idx, &mock, &params, 200)
                        .await
                        .expect("embeddings (mock)");
                    (carta, "MockProvider".to_string())
                }
            }
        });
        eprintln!("multilienzo_llm_demo :: embeddings = {label_provider}");

        Model {
            cuerpos: vec![es, qu, en],
            graph,
            cartas: vec![carta_es_qu, carta_qu_en],
            label_llm,
            label_provider,
        }
    }

    fn update(model: Model, _msg: Msg, _: &Handle<Msg>) -> Model {
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let cfg = MultilienzoConfig::default();
        let paleta = PaletaHebras::default();
        let palette = Palette::default();

        let index: IndiceAtoms = model.graph.atoms().map(|a| (a.id, a)).collect();
        let cuerpos_ref: Vec<&Cuerpo> = model.cuerpos.iter().collect();
        let cartas_ref: Vec<Option<&CartaHebras>> = model.cartas.iter().map(Some).collect();

        let interior = multilienzo_view::<Msg>(
            &cuerpos_ref,
            &index,
            &cartas_ref,
            &cfg,
            &paleta,
            &palette,
        );
        let _ = &model.label_llm;
        let _ = &model.label_provider;

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
        .children(vec![interior])
    }
}

/// Construye el chat client para el demo. Si `pluma_llm::from_env` cae a
/// Mock (porque no hay credenciales), lo sustituimos por uno pre-poblado
/// con traducciones predecibles — así el demo se ve "como si tradujera"
/// aun sin red. Cuando hay credenciales reales, devuelve el cliente que
/// el factory produjo y la IA traduce de verdad.
fn construir_chat_para_demo(
    textos: &[&'static str],
) -> (std::sync::Arc<dyn ChatClient>, String) {
    // Detectamos si vamos a caer a Mock antes de construir, mirando las
    // mismas env vars que el factory.
    let usa_mock = std::env::var("PLUMA_LLM_BACKEND")
        .ok()
        .and_then(|s| BackendKind::parse(&s))
        .map(|k| k == BackendKind::Mock)
        .unwrap_or_else(|| {
            std::env::var("ANTHROPIC_API_KEY").is_err()
                && std::env::var("GEMINI_API_KEY").is_err()
                && std::env::var("GOOGLE_API_KEY").is_err()
                && std::env::var("DEEPSEEK_API_KEY").is_err()
                && std::env::var("PLUMA_LLM_BACKEND")
                    .map(|s| BackendKind::parse(&s) != Some(BackendKind::Ollama))
                    .unwrap_or(true)
        });

    if usa_mock {
        // Mock pre-poblado: cada texto en español tiene su traducción al qu
        // hardcoded en una tabla. Demuestra el flujo aún sin LLM real.
        let traducciones = [
            ("cóndor cruzó", "Kuntur wayqu hanaqpachata pacha paqarinpi pasarqa."),
            ("Las llamas pastaban", "Llamaqakuna qulla suyup q'achupinpi mikhusharqaku."),
            ("mujer joven tejía", "Sipas warmiq away wasiq hawanpi awayta ruwasharqa."),
            ("Apurímac", "Apurímac mayu rumikuna ukhumanta qhaparispa uraykurqa."),
            ("nubes cubrieron", "Inti yaykuy pachapi puyukuna intita pakarqaku."),
        ];
        let mut mock = pluma_llm_mock::MockChatClient::default()
            .con_model_id("mock-demo");
        for (k, v) in traducciones {
            mock = mock.con_respuesta(k, v);
        }
        let _ = textos;
        return (std::sync::Arc::new(mock), "mock-demo (sin red)".to_string());
    }

    match llm_from_env() {
        Ok(cli) => {
            let label = format!("backend real: {}", cli.model_id());
            (cli, label)
        }
        Err(e) => {
            eprintln!(
                "multilienzo_llm_demo :: fallo el factory ({e}); caigo a mock-demo"
            );
            let mock = pluma_llm_mock::MockChatClient::default()
                .con_model_id("mock-demo");
            (std::sync::Arc::new(mock), "mock-demo (fallback)".to_string())
        }
    }
}

fn socket_verbo_default() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(xdg).join("verbo.sock");
    }
    let uid = std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&u| u != u32::MAX)
        .unwrap_or(1000);
    PathBuf::from(format!("/tmp/verbo-{uid}.sock"))
}

async fn conectar_daemon_si_existe(path: &Path) -> Option<DaemonClient> {
    if !path.exists() {
        return None;
    }
    DaemonClient::connect(path).await.ok()
}

fn main() {
    // Silenciar wrns de campos no leídos en `Model` para esta demo —
    // labels existen para debugging, no para pintar (todavía).
    let _ = LlmConfig::default();
    llimphi_ui::run::<Demo>();
}
