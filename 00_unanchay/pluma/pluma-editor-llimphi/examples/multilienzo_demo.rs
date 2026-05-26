//! Demo del multilienzo: tres cuerpos (es / qu / en) con hebras
//! mostrando los cuatro estados posibles — derivada fresca, embeddings
//! con fuerza modulada, manual, y stale punteada.
//!
//! Corré con:
//!   cargo run -p pluma-editor-llimphi --example multilienzo_demo --release

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, FlexDirection, Size, Style};
use llimphi_ui::{App, Handle, View};
use pluma_align::{
    alinear_explicito, alinear_uno_a_uno, Alineamiento, CartaHebras, OrigenAlineamiento,
};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::multilienzo::{
    multilienzo_view, IndiceAtoms, MultilienzoConfig, PaletaHebras,
};
use pluma_editor_llimphi::Palette;
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
        "pluma · multilienzo demo"
    }

    fn initial_size() -> (u32, u32) {
        (1280, 640)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let (es, atoms_es) = cuerpo(
            "es",
            "español (original)",
            Intencion::Original,
            &[
                "El cóndor cruzó el cielo del valle al amanecer.",
                "Las llamas pastaban entre los pastizales del altiplano.",
                "Una mujer joven tejía un telar bajo el aleros.",
                "El río Apurímac descendía rugiente por las rocas.",
                "Al caer la tarde, las nubes cubrieron el sol.",
            ],
        );

        let (qu, atoms_qu) = cuerpo(
            "qu",
            "runa simi (traducción)",
            Intencion::Traduccion,
            &[
                "Kuntur wayqu hanaqpachata pacha paqarinpi pasarqa.",
                "Llamaqakuna qulla suyup q'achupinpi mikhusharqaku.",
                "Sipas warmiq away wasiq hawanpi awayta ruwasharqa.",
                "Apurímac mayu rumikuna ukhumanta qhaparispa uraykurqa.",
                "Inti yaykuy pachapi puyukuna intita pakarqaku.",
            ],
        );

        let (en, atoms_en) = cuerpo(
            "en",
            "english (résumé)",
            Intencion::Resumen {
                palabras_objetivo: Some(40),
            },
            &[
                "Dawn over the highlands — condor, llamas, weaver.",
                "By dusk, the Apurímac roared and the clouds hid the sun.",
            ],
        );

        // Carta es↔qu: 1↔1 derivada fresca.
        let mut carta_es_qu = alinear_uno_a_uno(
            &es,
            &qu,
            OrigenAlineamiento::Derivado {
                transformacion: Uuid::new_v4(),
                timestamp: 1_000,
            },
        );
        // La hebra del tercer párrafo viene de Embeddings con fuerza baja —
        // se verá visualmente más tenue.
        if let Some(h) = carta_es_qu.hebras.get_mut(2) {
            h.origen = OrigenAlineamiento::Embeddings {
                modelo: "iniy-1".into(),
                timestamp: 1_000,
            };
            h.fuerza = 0.35;
        }
        // La hebra del último es Manual (autoría humana — color ámbar).
        if let Some(h) = carta_es_qu.hebras.get_mut(4) {
            h.origen = OrigenAlineamiento::Manual {
                autor: "ana".into(),
                timestamp: 1_000,
            };
        }
        // La hebra del primero quedó stale: la madre se editó después.
        if let Some(h) = carta_es_qu.hebras.get_mut(0) {
            h.fresco = false;
        }

        // Carta qu↔en: 5→2 manual (resumen condensa varios párrafos).
        let pares: Vec<(Uuid, Uuid, f32)> = vec![
            (atoms_qu[0].id, atoms_en[0].id, 0.9),
            (atoms_qu[1].id, atoms_en[0].id, 0.85),
            (atoms_qu[2].id, atoms_en[0].id, 0.6),
            (atoms_qu[3].id, atoms_en[1].id, 0.9),
            (atoms_qu[4].id, atoms_en[1].id, 0.8),
        ];
        let carta_qu_en = alinear_explicito(
            &qu,
            &en,
            &pares,
            OrigenAlineamiento::Embeddings {
                modelo: "iniy-1".into(),
                timestamp: 1_000,
            },
        );

        let mut atoms = atoms_es;
        atoms.extend(atoms_qu);
        atoms.extend(atoms_en);

        Model {
            cuerpos: vec![es, qu, en],
            atoms,
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

        // Envoltura full-window con clip — el multilienzo ya devuelve un nodo
        // del tamaño exacto del contenido; el padre lo recorta al viewport.
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

fn cuerpo(
    branch: &str,
    nombre: &str,
    intencion: Intencion,
    textos: &[&str],
) -> (Cuerpo, Vec<NarrativeAtom>) {
    let mut c = Cuerpo::nuevo(branch, nombre, intencion, 100);
    let atoms: Vec<NarrativeAtom> = textos
        .iter()
        .map(|t| NarrativeAtom::new(*t, branch))
        .collect();
    for a in &atoms {
        c.agregar(a.id, 101);
    }
    (c, atoms)
}

fn main() {
    llimphi_ui::run::<Demo>();
}

// Re-exported types referenced only via type alias chains.
#[allow(dead_code)]
fn _silence_alineamiento(_: Alineamiento) {}
