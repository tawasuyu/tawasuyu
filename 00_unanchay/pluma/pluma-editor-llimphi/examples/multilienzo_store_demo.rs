//! Demo del multilienzo con persistencia: lo que generes una vez
//! sobrevive a cerrar el proceso.
//!
//! Comportamiento:
//!   - Primer arranque: cuerpo `es` sintético + traducción a `qu` vía
//!     LLM (transparente: el backend lo decide `pluma_llm::from_env`)
//!     + cuerpo `en` (resumen manual). Persiste todo en
//!     `~/.cache/gioser/pluma-multilienzo/`.
//!   - Arranques siguientes: lee la store, salta el LLM por completo.
//!     Lo que ves en pantalla son los mismos cuerpos y hebras de la
//!     primera vez.
//!   - `--reset` (o env `MULTILIENZO_RESET=1`) limpia el cache y
//!     fuerza una regeneración.
//!
//! ```bash
//! # Primera corrida: pega al LLM y guarda.
//! GEMINI_API_KEY=... PLUMA_LLM_BACKEND=gemini \
//!   cargo run -p pluma-editor-llimphi --example multilienzo_store_demo --release
//!
//! # Siguientes corridas: instantáneo, sin red.
//! cargo run -p pluma-editor-llimphi --example multilienzo_store_demo --release
//!
//! # Resetear cache:
//! MULTILIENZO_RESET=1 cargo run -p pluma-editor-llimphi \
//!   --example multilienzo_store_demo --release
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
use pluma_llm::from_env as llm_from_env;
use pluma_llm_core::ChatClient;
use pluma_store::PlumaStore;
use pluma_transform::{TipoTransformacion, Transformacion};
use pluma_transform_llm::EjecutorTraducirLlm;
use rimay_verbo_daemon::DaemonClient;
use rimay_verbo_mock::MockProvider;
use uuid::Uuid;

#[derive(Clone, Debug)]
enum Msg {}

struct Model {
    cuerpos: Vec<Cuerpo>,
    graph: NarrativeGraph,
    cartas: Vec<CartaHebras>,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · multilienzo store demo (persiste entre corridas)"
    }

    fn initial_size() -> (u32, u32) {
        (1280, 680)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let cache_dir = cache_dir();
        let reset = std::env::var("MULTILIENZO_RESET").ok().as_deref() == Some("1")
            || std::env::args().any(|a| a == "--reset");
        if reset {
            let _ = std::fs::remove_dir_all(&cache_dir);
            eprintln!("multilienzo_store_demo :: cache reseteado");
        }
        std::fs::create_dir_all(&cache_dir).expect("crear cache dir");
        let sled_path = cache_dir.join("pluma.sled");

        let store = PlumaStore::open(&sled_path).expect("abrir PlumaStore");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime tokio");

        // ¿La store tiene contenido? Si sí, cargamos. Si no, generamos.
        let tiene_contenido = store.cuerpos_len() >= 1;
        if tiene_contenido {
            eprintln!(
                "multilienzo_store_demo :: cargando de {} ({} cuerpos)",
                sled_path.display(),
                store.cuerpos_len()
            );
            return cargar_de_store(&store);
        }

        eprintln!(
            "multilienzo_store_demo :: cache vacía → generando vía LLM y persistiendo en {}",
            sled_path.display()
        );

        // -- Generar desde cero --
        let mut graph = NarrativeGraph::new();
        let mut es = Cuerpo::nuevo("es", "español (original)", Intencion::Original, 100);
        let textos_es = [
            "El cóndor cruzó el cielo del valle al amanecer.",
            "Las llamas pastaban entre los pastizales del altiplano.",
            "Una mujer joven tejía un telar bajo el alero.",
        ];
        let mut atoms_es: Vec<Uuid> = Vec::new();
        for t in textos_es {
            let atom = NarrativeAtom::new(t, "es");
            atoms_es.push(atom.id);
            es.agregar(atom.id, 101);
            graph.insert(atom);
        }

        // LLM transparente — si no hay key, MockChatClient pre-poblado.
        let chat = construir_chat();
        eprintln!("multilienzo_store_demo :: LLM = {}", chat.model_id());

        let ejecutor = EjecutorTraducirLlm::from_arc(chat, "qu");
        let t_qu = Transformacion::nueva(
            es.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "qu".into() },
            "demo",
            200,
        );

        let (qu, carta_es_qu) = rt.block_on(async {
            let idx = indice_atoms(&graph);
            let producto = ejecutor
                .aplicar_con_atoms(&t_qu, &es, &idx, 200)
                .await
                .expect("traducción");
            drop(idx);
            persistir_producto(&mut graph, producto)
        });

        // Cuerpo en (resumen manual, 2 párrafos hardcoded).
        let mut en = Cuerpo::nuevo(
            "en",
            "english (résumé)",
            Intencion::Resumen { palabras_objetivo: Some(40) },
            200,
        );
        for t in ["Dawn over the highlands.", "By dusk, clouds hid the sun."] {
            let atom = NarrativeAtom::new(t, "en");
            en.agregar(atom.id, 201);
            graph.insert(atom);
        }

        // Hebras qu↔en por embeddings.
        let socket = socket_verbo_default();
        let carta_qu_en = rt.block_on(async {
            let idx = indice_atoms(&graph);
            match conectar_daemon_si_existe(&socket).await {
                Some(daemon) => {
                    let params = ParamsAlineacion {
                        umbral_minimo: 0.5,
                        modo: ModoAlineacion::MejorParaCadaA,
                    };
                    alinear_por_embeddings(&qu, &en, &idx, &daemon, &params, 200)
                        .await
                        .expect("embeddings daemon")
                }
                None => {
                    let mock = MockProvider::default();
                    let params = ParamsAlineacion {
                        umbral_minimo: -1.0,
                        modo: ModoAlineacion::MejorParaCadaA,
                    };
                    alinear_por_embeddings(&qu, &en, &idx, &mock, &params, 200)
                        .await
                        .expect("embeddings mock")
                }
            }
        });

        // Persistir TODO.
        for atom in graph.atoms() {
            store.put_atom(atom).expect("persistir atom");
        }
        for c in [&es, &qu, &en] {
            store.put_cuerpo(c).expect("persistir cuerpo");
        }
        store.put_transformacion(&t_qu).expect("persistir transformación");
        store.put_carta(&carta_es_qu).expect("persistir carta es↔qu");
        store.put_carta(&carta_qu_en).expect("persistir carta qu↔en");
        store.flush().expect("flush");

        eprintln!(
            "multilienzo_store_demo :: persistido — atoms={} cuerpos={} cartas={} transformaciones={}",
            store.atoms_len(),
            store.cuerpos_len(),
            store.cartas_len(),
            store.iter_transformaciones().count(),
        );

        Model {
            cuerpos: vec![es, qu, en],
            graph,
            cartas: vec![carta_es_qu, carta_qu_en],
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

/// Carga cuerpos + grafo + cartas del store. Si la store está
/// inconsistente, el demo falla con mensaje claro — el caller resetea
/// con `--reset`.
fn cargar_de_store(store: &PlumaStore) -> Model {
    let mut graph = NarrativeGraph::new();
    for atom in store.iter_atoms() {
        graph.insert(atom.expect("leer atom"));
    }

    let mut cuerpos: Vec<Cuerpo> = store
        .iter_cuerpos()
        .map(|c| c.expect("leer cuerpo"))
        .collect();
    // Orden estable: Original primero, luego Traduccion, luego Resumen.
    // Más fino requeriría un campo de orden en MetaCuerpo; por ahora
    // este sort de tres categorías es suficiente para el demo.
    cuerpos.sort_by_key(|c| match c.metadatos.intencion {
        Intencion::Original => 0,
        Intencion::Traduccion => 1,
        Intencion::Resumen { .. } => 2,
        Intencion::Tono { .. } => 3,
        Intencion::Reescritura { .. } => 4,
        Intencion::Anotacion => 5,
        Intencion::Custom { .. } => 6,
    });

    // Cargar cartas en el orden de las columnas: cuerpos[i] ↔ cuerpos[i+1].
    let mut cartas: Vec<CartaHebras> = Vec::with_capacity(cuerpos.len().saturating_sub(1));
    for w in cuerpos.windows(2) {
        if let Some(carta) = store
            .get_carta_bidir(w[0].id, w[1].id)
            .expect("leer carta")
        {
            cartas.push(carta);
        }
    }

    Model {
        cuerpos,
        graph,
        cartas,
    }
}

/// Cache dir: `$XDG_CACHE_HOME/gioser/pluma-multilienzo` o
/// `$HOME/.cache/gioser/pluma-multilienzo`.
fn cache_dir() -> PathBuf {
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("gioser").join("pluma-multilienzo")
}

fn construir_chat() -> std::sync::Arc<dyn ChatClient> {
    // Si no hay key real, sustituimos por mock pre-poblado con
    // traducciones predecibles — el demo se ve "como si tradujera".
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
        let pares = [
            ("cóndor cruzó", "Kuntur wayqu hanaqpachata pacha paqarinpi pasarqa."),
            ("Las llamas pastaban", "Llamaqakuna qulla suyup q'achupinpi mikhusharqaku."),
            ("mujer joven tejía", "Sipas warmiq away wasiq hawanpi awayta ruwasharqa."),
        ];
        for (k, v) in pares {
            mock = mock.con_respuesta(k, v);
        }
        return std::sync::Arc::new(mock);
    }
    llm_from_env().expect("from_env LLM")
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
    llimphi_ui::run::<Demo>();
}
