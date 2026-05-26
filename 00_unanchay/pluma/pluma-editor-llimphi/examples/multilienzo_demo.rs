//! Demo del multilienzo — flujo end-to-end real:
//!
//!   1. Cuerpo madre `es` con párrafos sintéticos.
//!   2. Cuerpo `qu` derivado por `EjecutorTraducirTabla` (Derivada 1↔1).
//!   3. Cuerpo `en` (resumen, 2 párrafos manuales).
//!   4. Hebras `es↔qu`: producto natural de la transformación (Derivada).
//!   5. Hebras `qu↔en`: calculadas por `alinear_por_embeddings` con
//!      MockProvider determinista (umbral muy bajo para mostrar que el
//!      pipeline funciona aun con vectores random — fuerzas variadas
//!      generan saturación visible).
//!
//! Una hebra es↔qu se marca stale a mano para mostrar el efecto visual.
//!
//! Corré con:
//!   cargo run -p pluma-editor-llimphi --example multilienzo_demo --release

use std::collections::HashMap;

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
use pluma_transform::{
    Ejecutor, TipoTransformacion, Transformacion,
};
use pluma_transform_tabla::EjecutorTraducirTabla;
use rimay_verbo_core::Provider;
use rimay_verbo_daemon::DaemonClient;
use rimay_verbo_mock::MockProvider;
use uuid::Uuid;

#[derive(Clone, Debug)]
enum Msg {}

struct Model {
    cuerpos: Vec<Cuerpo>,
    atoms: Vec<NarrativeAtom>,
    cartas: Vec<CartaHebras>,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · multilienzo demo (es → qu derivado · qu ↔ en embeddings)"
    }

    fn initial_size() -> (u32, u32) {
        (1280, 680)
    }

    fn init(_: &Handle<Msg>) -> Model {
        // Runtime tokio compartido para todos los `await` (aplicar de la
        // transformación + alineación por embeddings).
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime tokio");

        // -- 1. Cuerpo madre `es` ----------------------------------------------
        let textos_es = [
            "El cóndor cruzó el cielo del valle al amanecer.",
            "Las llamas pastaban entre los pastizales del altiplano.",
            "Una mujer joven tejía un telar bajo el alero.",
            "El río Apurímac descendía rugiente por las rocas.",
            "Al caer la tarde, las nubes cubrieron el sol.",
        ];
        let atoms_es: Vec<NarrativeAtom> = textos_es
            .iter()
            .map(|t| NarrativeAtom::new(*t, "es"))
            .collect();
        let mut es = Cuerpo::nuevo("es", "español (original)", Intencion::Original, 100);
        for a in &atoms_es {
            es.agregar(a.id, 101);
        }

        // -- 2. Cuerpo `qu` derivado por tabla --------------------------------
        let traducciones = [
            "Kuntur wayqu hanaqpachata pacha paqarinpi pasarqa.",
            "Llamaqakuna qulla suyup q'achupinpi mikhusharqaku.",
            "Sipas warmiq away wasiq hawanpi awayta ruwasharqa.",
            "Apurímac mayu rumikuna ukhumanta qhaparispa uraykurqa.",
            "Inti yaykuy pachapi puyukuna intita pakarqaku.",
        ];
        let mut tabla: HashMap<Uuid, String> = HashMap::new();
        for (atom, tr) in atoms_es.iter().zip(traducciones.iter()) {
            tabla.insert(atom.id, (*tr).to_string());
        }
        let ejecutor_traducir = EjecutorTraducirTabla::new(tabla, "qu");
        let t_qu = Transformacion::nueva(
            es.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir {
                lengua_destino: "qu".into(),
            },
            "ana",
            200,
        );
        let prod = rt
            .block_on(ejecutor_traducir.aplicar(&t_qu, &es, 200))
            .expect("traducción por tabla debería tener éxito");
        let qu = prod.hija;
        let atoms_qu = prod.atoms_nuevos;
        let mut carta_es_qu = prod.carta;

        // Marcar a mano la primera hebra como stale: la madre se editó después
        // de la regeneración (simulación del estado típico tras edición).
        if let Some(h) = carta_es_qu.hebras.get_mut(0) {
            h.fresco = false;
        }

        // -- 3. Cuerpo `en` (resumen, 2 párrafos manuales) --------------------
        let textos_en = [
            "Dawn over the highlands — condor, llamas, weaver.",
            "By dusk, the Apurímac roared and the clouds hid the sun.",
        ];
        let atoms_en: Vec<NarrativeAtom> = textos_en
            .iter()
            .map(|t| NarrativeAtom::new(*t, "en"))
            .collect();
        let mut en = Cuerpo::nuevo(
            "en",
            "english (résumé)",
            Intencion::Resumen {
                palabras_objetivo: Some(40),
            },
            200,
        );
        for a in &atoms_en {
            en.agregar(a.id, 201);
        }

        // -- 4. Hebras qu↔en por embeddings (MockProvider determinista) -------
        // Indice de atoms para que alinear_por_embeddings resuelva los textos.
        let mut atoms_all: Vec<NarrativeAtom> = atoms_es.clone();
        atoms_all.extend(atoms_qu.iter().cloned());
        atoms_all.extend(atoms_en.iter().cloned());
        let idx: HashMap<Uuid, &NarrativeAtom> =
            atoms_all.iter().map(|a| (a.id, a)).collect();

        // Provider: si hay un `verbo-daemon` corriendo, nos conectamos —
        // así las hebras llevan similitudes semánticas reales. Sin daemon,
        // fallback al MockProvider determinista (las fuerzas serán
        // dispersas; sirve para ver la geometría del pintado).
        let socket = socket_verbo_default();
        let (provider_label, carta_qu_en) = rt.block_on(async {
            match conectar_daemon_si_existe(&socket).await {
                Some(daemon) => {
                    eprintln!(
                        "multilienzo_demo :: usando verbo-daemon en {} ({})",
                        socket.display(),
                        daemon.model_id()
                    );
                    // Con modelo real, umbral 0.5 filtra ruido sin perder
                    // las correspondencias semánticas reales.
                    let params = ParamsAlineacion {
                        umbral_minimo: 0.5,
                        modo: ModoAlineacion::MejorParaCadaA,
                    };
                    let carta = alinear_por_embeddings(&qu, &en, &idx, &daemon, &params, 200)
                        .await
                        .expect("alineación por embeddings (daemon)");
                    ("verbo-daemon".to_string(), carta)
                }
                None => {
                    let mock = MockProvider::default();
                    eprintln!(
                        "multilienzo_demo :: sin verbo-daemon — usando MockProvider \
                         (las fuerzas Embeddings saldrán dispersas y geométricas, no semánticas; \
                         para hebras reales: `verbo-daemon --provider fastembed &`)"
                    );
                    // Umbral negativo: con mock random todas las "mejores"
                    // pasan — vemos la geometría aunque la fuerza sea ruido.
                    let params = ParamsAlineacion {
                        umbral_minimo: -1.0,
                        modo: ModoAlineacion::MejorParaCadaA,
                    };
                    let carta = alinear_por_embeddings(&qu, &en, &idx, &mock, &params, 200)
                        .await
                        .expect("alineación por embeddings (mock)");
                    ("mock".to_string(), carta)
                }
            }
        });
        eprintln!("multilienzo_demo :: hebras qu↔en con provider={provider_label}");

        Model {
            cuerpos: vec![es, qu, en],
            atoms: atoms_all,
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

        let index: IndiceAtoms = model.atoms.iter().map(|a| (a.id, a)).collect();
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

/// Ruta default del socket del `verbo-daemon`, alineada con la convención
/// que usa el bin (`rimay-verbo-daemon-bin`): `$XDG_RUNTIME_DIR/verbo.sock`
/// con fallback a `/tmp/verbo-{uid}.sock`.
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

/// Intenta conectar al daemon en `path`. Devuelve `None` si el socket no
/// existe o si la conexión falla — el caller decide el fallback. No
/// imprime; deja la decisión de logging al caller.
async fn conectar_daemon_si_existe(path: &std::path::Path) -> Option<DaemonClient> {
    if !path.exists() {
        return None;
    }
    match DaemonClient::connect(path).await {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!(
                "multilienzo_demo :: socket {} existe pero connect falló: {e}",
                path.display()
            );
            None
        }
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
