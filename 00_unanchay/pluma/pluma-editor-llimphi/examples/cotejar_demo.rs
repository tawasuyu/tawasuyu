//! Demo **interactivo** del cotejo de pluma — comparar dos versiones de un
//! documento como lienzos paralelos, con la diferencia por sección en el medio
//! y el coloreado **verde** (coincide) → **rojo** (difiere).
//!
//! Teclas:
//!   - `i` — invierte izquierda↔derecha (los lienzos son intercambiables).
//!   - `r` — pide a la IA (pluma-llm) que redacte qué cambió en cada sección;
//!           sin backend configurado usa un mock sembrado con frases de ejemplo.
//!   - `Esc` — sale.
//!
//! Sin daemon ni API key arranca igual (mock). Con un backend real (export
//! `ANTHROPIC_API_KEY=…` u otro) la tecla `r` produce resúmenes de verdad.
//!
//! Corré con:
//!   cargo run -p pluma-editor-llimphi --example cotejar_demo --release

use std::collections::HashMap;
use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Rect, Size, Style};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};

use pluma_align::CartaHebras;
use pluma_cotejo::{
    cotejar, columna_diferencias, ParamsCotejo, ResumidorTextual, SeccionCotejo,
};
use pluma_cotejo_llm::{items_desde_secciones, resumir_diferencias};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::multilienzo::{
    multilienzo_cotejo_view_reorderable, IndiceAtoms, MultilienzoConfig, PaletaHebras,
};
use pluma_editor_llimphi::Palette;
use pluma_llm_core::ChatClient;
use pluma_llm_mock::MockChatClient;
use uuid::Uuid;

#[derive(Clone, Debug)]
enum Msg {
    /// Invierte izquierda↔derecha y recalcula.
    Invertir,
    /// Drag-to-swap: se soltó la columna `desde` sobre la `hasta`.
    Reordenar(usize, usize),
    /// Lanza el resumidor IA sobre las secciones cambiadas.
    Resumir,
    /// Resultado del resumidor: una línea por sección, en orden.
    ResumenListo(Vec<String>),
    /// Falló el resumidor — se conserva el resumen textual.
    ResumenError(String),
}

struct Model {
    /// Documentos fuente, en su orientación actual (se swappean al invertir).
    izq: Cuerpo,
    der: Cuerpo,
    /// Átomos de los fuentes (estables; el lienzo de diferencias se recalcula).
    base_atoms: HashMap<Uuid, NarrativeAtom>,
    // --- Derivado del cotejo (se recalcula con `recotejar`) ---
    dif: Cuerpo,
    atoms: HashMap<Uuid, NarrativeAtom>,
    carta_izq: CartaHebras,
    carta_der: CartaHebras,
    divergencias: HashMap<Uuid, f32>,
    secciones: Vec<SeccionCotejo>,
    conteo: String,
    /// Orden de display de las 3 columnas, índices del canónico `[izq, dif, der]`.
    /// El drag-to-swap lo permuta; `recotejar` lo resetea a `[0, 1, 2]`.
    orden: Vec<usize>,
    // --- IA ---
    chat: Arc<dyn ChatClient>,
    resumiendo: bool,
    status: String,
}

impl Model {
    /// Recalcula el cotejo desde `izq`/`der`/`base_atoms` y repuebla los campos
    /// derivados (lienzo de diferencias textual, cartas, divergencias).
    fn recotejar(&mut self) {
        let idx: IndiceAtoms = self
            .izq
            .orden
            .iter()
            .chain(self.der.orden.iter())
            .filter_map(|id| self.base_atoms.get(id).map(|a| (*id, a)))
            .collect();
        let cot = cotejar(&self.izq, &self.der, &idx, &ParamsCotejo::default(), 0);
        let col = columna_diferencias(&cot, &self.izq, &self.der, &idx, &ResumidorTextual, 0);

        let c = cot.conteos();
        self.conteo = format!(
            "{} idénticas · {} reformuladas · {} reescritas · {} agregadas · {} eliminadas",
            c.identicas, c.similares, c.divergentes, c.agregadas, c.eliminadas
        );

        // Átomos completos: fuentes + lienzo de diferencias.
        let mut atoms = self.base_atoms.clone();
        for a in &col.atoms {
            atoms.insert(a.id, a.clone());
        }
        let mut divergencias = cot.divergencias;
        divergencias.extend(col.divergencias);

        self.dif = col.cuerpo;
        self.atoms = atoms;
        self.carta_izq = col.carta_izq;
        self.carta_der = col.carta_der;
        self.divergencias = divergencias;
        self.secciones = cot.secciones;
        self.orden = vec![0, 1, 2];
    }
}

/// Busca en el pool la carta que conecta los cuerpos `a` y `b` (en cualquier
/// orden). `None` si ese par no tiene carta — caso normal tras reordenar.
fn carta_par<'a>(pool: &[&'a CartaHebras], a: Uuid, b: Uuid) -> Option<&'a CartaHebras> {
    pool.iter()
        .copied()
        .find(|c| {
            (c.cuerpo_a == Some(a) && c.cuerpo_b == Some(b))
                || (c.cuerpo_a == Some(b) && c.cuerpo_b == Some(a))
        })
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · cotejar — dos versiones, la diferencia por sección (i: invertir · r: resumen IA)"
    }

    fn initial_size() -> (u32, u32) {
        (1360, 820)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let original = [
            "Pluma es un editor de documentos como haz de cuerpos.",
            "Cada cuerpo es un lienzo del mismo material bajo otra mirada.",
            "Los párrafos se alinean uno a uno entre cuerpos.",
            "El motor gráfico se llamaba GPUI en las primeras versiones.",
            "La persistencia vive en una base sled embebida.",
        ];
        let editado = [
            "Pluma es un editor de documentos como haz de cuerpos.",
            "Cada cuerpo es un lienzo del mismo material visto desde otra intención.",
            "Los párrafos quedan alineados uno a uno entre los cuerpos del haz.",
            "Hoy todo lo gráfico corre sobre Llimphi con wgpu y vello.",
            "La persistencia vive en una base sled embebida.",
            "Un cotejo compara dos versiones sección por sección.",
        ];

        let mut base_atoms: HashMap<Uuid, NarrativeAtom> = HashMap::new();
        let izq = cuerpo_con(&mut base_atoms, "a", "original.md", Intencion::Original, &original);
        let der = cuerpo_con(
            &mut base_atoms,
            "b",
            "editado.md",
            Intencion::Custom { kind: "versión".into() },
            &editado,
        );

        let mut m = Model {
            izq,
            der,
            base_atoms,
            dif: Cuerpo::nuevo("dif", "diferencias", Intencion::Custom { kind: "cotejo".into() }, 0),
            atoms: HashMap::new(),
            carta_izq: CartaHebras::nueva(),
            carta_der: CartaHebras::nueva(),
            divergencias: HashMap::new(),
            secciones: Vec::new(),
            conteo: String::new(),
            orden: vec![0, 1, 2],
            chat: construir_chat(),
            resumiendo: false,
            status: "arrastrá una cabecera para reordenar · i: invertir · r: resumen IA".into(),
        };
        m.recotejar();
        m
    }

    fn on_key(_model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        match &event.key {
            Key::Named(NamedKey::Escape) => std::process::exit(0),
            Key::Character(s) if s.eq_ignore_ascii_case("i") => Some(Msg::Invertir),
            Key::Character(s) if s.eq_ignore_ascii_case("r") => Some(Msg::Resumir),
            _ => None,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Invertir => {
                std::mem::swap(&mut model.izq, &mut model.der);
                model.recotejar();
                model.status = format!(
                    "invertido — «{}» ↔ «{}»",
                    model.izq.metadatos.nombre_legible, model.der.metadatos.nombre_legible
                );
            }
            Msg::Reordenar(desde, hasta) => {
                let n = model.orden.len();
                if desde < n && hasta < n && desde != hasta {
                    model.orden.swap(desde, hasta);
                    model.status = "columnas reordenadas".into();
                }
            }
            Msg::Resumir => {
                if model.resumiendo {
                    return model;
                }
                let items = items_desde_secciones(&model.secciones, &model.atoms);
                let hay_cambios = items.iter().any(|it| {
                    matches!(
                        it.clase,
                        pluma_cotejo::ClaseCambio::Similar | pluma_cotejo::ClaseCambio::Divergente
                    )
                });
                if !hay_cambios {
                    model.status = "no hay diferencias que resumir".into();
                    return model;
                }
                let chat = model.chat.clone();
                model.resumiendo = true;
                model.status = format!("resumen IA en curso ({})…", model.chat.model_id());
                handle.spawn(move || {
                    let rt = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(e) => return Msg::ResumenError(format!("runtime: {e}")),
                    };
                    match rt.block_on(resumir_diferencias(&items, &*chat)) {
                        Ok(l) => Msg::ResumenListo(l),
                        Err(e) => Msg::ResumenError(e),
                    }
                });
            }
            Msg::ResumenListo(lineas) => {
                model.resumiendo = false;
                let ids: Vec<Uuid> = model.dif.orden.clone();
                for (id, linea) in ids.iter().zip(lineas.iter()) {
                    if let Some(a) = model.atoms.get_mut(id) {
                        a.set_content(linea.clone());
                    }
                }
                model.status = "resumen IA aplicado".into();
            }
            Msg::ResumenError(e) => {
                model.resumiendo = false;
                model.status = format!("resumen IA falló: {e}");
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let palette = Palette::default();
        let paleta_hebras = PaletaHebras::default();
        let cfg = MultilienzoConfig {
            altura_atom: 92.0,
            gap_atom: 14.0,
            ancho_cuerpo: 376.0,
            ancho_carril: 86.0,
            padding_top: 14.0,
            ..MultilienzoConfig::default()
        };

        let index: IndiceAtoms = model.atoms.iter().map(|(id, a)| (*id, a)).collect();
        // Canónico [izq, dif, der]; el display sigue `orden` (drag-to-swap).
        let canon: [&Cuerpo; 3] = [&model.izq, &model.dif, &model.der];
        let cuerpos_ref: Vec<&Cuerpo> = model.orden.iter().map(|&i| canon[i]).collect();
        // Carril de cada par adyacente = la carta que conecta esos dos cuerpos
        // (no por posición), así reordenar mueve las hebras con las columnas.
        let pool: [&CartaHebras; 2] = [&model.carta_izq, &model.carta_der];
        let cartas_ref: Vec<Option<&CartaHebras>> = cuerpos_ref
            .windows(2)
            .map(|w| carta_par(&pool, w[0].id, w[1].id))
            .collect();

        let interior = multilienzo_cotejo_view_reorderable::<Msg, _>(
            &cuerpos_ref,
            &index,
            &cartas_ref,
            &model.divergencias,
            &cfg,
            &paleta_hebras,
            &palette,
            "",
            |desde, hasta| Some(Msg::Reordenar(desde, hasta)),
        );

        let titulo = View::<Msg>::new(Style {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
            ..Default::default()
        })
        .children(vec![
            View::<Msg>::new(Style {
                size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
                ..Default::default()
            })
            .text_aligned(
                format!("Cotejo — {}", model.conteo),
                14.0,
                palette.fg_text,
                Alignment::Start,
            ),
            View::<Msg>::new(Style {
                size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
                ..Default::default()
            })
            .text_aligned(
                format!("verde = coincide · rojo = difiere   ·   {}", model.status),
                11.0,
                palette.fg_muted,
                Alignment::Start,
            ),
        ]);

        let header = View::<Msg>::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(48.0_f32) },
            padding: Rect {
                left: length(18.0_f32),
                right: length(18.0_f32),
                top: length(8.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        })
        .fill(palette.bg_panel)
        .children(vec![titulo]);

        let centro = View::<Msg>::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            justify_content: Some(taffy::JustifyContent::Center),
            align_items: Some(taffy::AlignItems::Start),
            ..Default::default()
        })
        .children(vec![interior]);

        View::<Msg>::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(palette.bg_app)
        .clip(true)
        .children(vec![header, centro])
    }
}

/// Crea un cuerpo con sus átomos, registrándolos en `atoms`.
fn cuerpo_con(
    atoms: &mut HashMap<Uuid, NarrativeAtom>,
    branch: &str,
    nombre: &str,
    intencion: Intencion,
    textos: &[&str],
) -> Cuerpo {
    let mut c = Cuerpo::nuevo(branch, nombre, intencion, 0);
    for t in textos {
        let a = NarrativeAtom::new(*t, branch);
        c.agregar(a.id, 0);
        atoms.insert(a.id, a);
    }
    c
}

/// Cliente LLM para la tecla `r`. Con un backend real configurado por env, lo
/// usa; si no, cae a un mock sembrado con frases de ejemplo para las secciones
/// que cambian en este demo — así `r` siempre muestra algo legible.
fn construir_chat() -> Arc<dyn ChatClient> {
    let tiene_backend = std::env::var("PLUMA_LLM_BACKEND").is_ok()
        || [
            "ANTHROPIC_API_KEY",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "DEEPSEEK_API_KEY",
            "COHERE_API_KEY",
        ]
        .iter()
        .any(|e| std::env::var(e).is_ok());
    if tiene_backend {
        if let Ok(c) = pluma_llm::from_env() {
            eprintln!("cotejar_demo :: usando backend LLM real ({})", c.model_id());
            return c;
        }
    }
    eprintln!(
        "cotejar_demo :: sin backend LLM — usando mock sembrado (export ANTHROPIC_API_KEY=… \
         para resúmenes reales)"
    );
    Arc::new(
        MockChatClient::default()
            .con_respuesta("intención", "cambia «mirada» por «intención»: mismo sentido, otra palabra")
            .con_respuesta("del haz", "precisa que la alineación es entre los cuerpos del haz")
            .con_respuesta("Llimphi", "reemplaza el motor: de GPUI a Llimphi con wgpu y vello"),
    )
}

fn main() {
    llimphi_ui::run::<Demo>();
}
