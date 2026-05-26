//! El demo "todo junto": toolbar LLM dinámica + persistencia automática.
//!
//! Cada transformación que disparás con un botón se persiste en
//! `~/.cache/gioser/pluma-multilienzo-completo/` ANTES de que veas la
//! columna nueva. Cerrá el demo, volvé a abrirlo: todo lo que generaste
//! sigue ahí, sin pegarle de nuevo al LLM.
//!
//! Esto es lo más cerca que tenemos a una "app" de pluma multilienzo:
//! abre, deriva cuerpos cuando hace falta, persiste sin avisar, y al
//! cierre garantiza que el sled está flusheado.
//!
//! ```bash
//! GEMINI_API_KEY=... PLUMA_LLM_BACKEND=gemini \
//!   cargo run -p pluma-editor-llimphi \
//!   --example multilienzo_completo_demo --release
//!
//! # Reset del cache:
//! MULTILIENZO_COMPLETO_RESET=1 cargo run -p pluma-editor-llimphi \
//!   --example multilienzo_completo_demo --release
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, FlexDirection, Position, Rect, Size, Style,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::llimphi_hal::winit::keyboard::{Key, NamedKey};
use llimphi_ui::{App, Handle, KeyEvent, KeyState, Modifiers, View, WheelDelta};
use llimphi_widget_button::{button_view, ButtonPalette};
use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::multilienzo::{
    multilienzo_view_resaltado, IndiceAtoms, MultilienzoConfig, PaletaHebras,
};
use pluma_editor_llimphi::Palette;
use pluma_graph::NarrativeGraph;
use pluma_llm::from_env as llm_from_env;
use pluma_llm_core::ChatClient;
use pluma_store::PlumaStore;
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
        transformacion: Transformacion,
    },
    LlmError(String),
    /// Delta de scroll horizontal en píxeles, positivo = derecha.
    ScrollHoriz(f32),
    /// Alterna entre mostrar solo el cuerpo madre y mostrar todos.
    ToggleSoloMadre,
    /// Agrega un carácter al final de la búsqueda transversal.
    BuscarAgregar(char),
    /// Borra el último carácter de la búsqueda.
    BuscarBorrar,
    /// Limpia la búsqueda completa.
    BuscarLimpiar,
}

struct Model {
    cuerpos: Vec<Cuerpo>,
    graph: NarrativeGraph,
    cartas: Vec<CartaHebras>,
    chat: Arc<dyn ChatClient>,
    store: Arc<PlumaStore>,
    en_curso: bool,
    ultimo_error: Option<String>,
    /// Desplazamiento horizontal acumulado del multilienzo, en píxeles.
    /// Wheel del mouse + Shift (o eje X de un touchpad) lo modifica.
    /// Se limita en `view` al ancho del contenido.
    scroll_x: f32,
    /// Si `true`, oculta todos los cuerpos excepto el primero (la madre).
    /// Toggleable con el botón "solo madre"/"todos".
    solo_madre: bool,
    /// Query de búsqueda transversal. Cualquier átomo (en cualquier
    /// cuerpo visible) cuyo `content` contenga este substring se
    /// resalta. Se acumula con `App::on_key` — el demo no usa widget
    /// de input, captura las teclas directas (alfanuméricas + espacio
    /// + Backspace + Escape).
    busqueda: String,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · multilienzo completo (LLM + persistencia)"
    }

    fn initial_size() -> (u32, u32) {
        (1400, 720)
    }

    /// Wheel del mouse → scroll horizontal. Convenciones:
    /// - touchpad con eje X (delta.x != 0) → horizontal directo.
    /// - Shift + wheel-Y vertical (común en Linux) → horizontal.
    /// - Wheel-Y sin Shift → vertical (no implementado todavía, ignorado).
    /// Multiplicador 30 px/línea coincide con el visor de texto de nahual.
    /// Captura de teclado para la búsqueda transversal sin widget de
    /// input. Cualquier `text` no-vacío del KeyEvent (lo que el sistema
    /// IME ya resolvió) suma su primer char a la búsqueda. Backspace
    /// borra el último; Escape limpia. Ctrl/Alt como modificador deja
    /// pasar la tecla (no captura — futuro: combos de la app).
    fn on_key(_model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        if event.modifiers.ctrl || event.modifiers.alt || event.modifiers.meta {
            return None;
        }
        if let Key::Named(NamedKey::Backspace) = event.key {
            return Some(Msg::BuscarBorrar);
        }
        if let Key::Named(NamedKey::Escape) = event.key {
            return Some(Msg::BuscarLimpiar);
        }
        // Texto producido (con IME e ortografía) — el primer char alfanum
        // o espacio entra a la búsqueda. Filtramos teclas de control
        // (Tab/Enter/etc.) por ser no-imprimibles.
        if let Some(text) = &event.text {
            if let Some(c) = text.chars().next() {
                if !c.is_control() {
                    return Some(Msg::BuscarAgregar(c));
                }
            }
        }
        None
    }

    fn on_wheel(
        _model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        modifiers: Modifiers,
    ) -> Option<Self::Msg> {
        const PX_POR_LINEA: f32 = 30.0;
        let dx_lineas = if delta.x.abs() > 0.0 {
            delta.x
        } else if modifiers.shift {
            // Shift convierte el eje Y de la rueda en horizontal.
            delta.y
        } else {
            return None;
        };
        Some(Msg::ScrollHoriz(-dx_lineas * PX_POR_LINEA))
    }

    fn init(_: &Handle<Msg>) -> Model {
        let cache_dir = cache_dir();
        let reset = std::env::var("MULTILIENZO_COMPLETO_RESET").ok().as_deref()
            == Some("1")
            || std::env::args().any(|a| a == "--reset");
        if reset {
            let _ = std::fs::remove_dir_all(&cache_dir);
            eprintln!("multilienzo_completo_demo :: cache reseteado");
        }
        std::fs::create_dir_all(&cache_dir).expect("crear cache dir");
        let sled_path = cache_dir.join("pluma.sled");
        let store = Arc::new(PlumaStore::open(&sled_path).expect("abrir PlumaStore"));

        let chat = construir_chat();
        eprintln!(
            "multilienzo_completo_demo :: store={} · LLM={}",
            sled_path.display(),
            chat.model_id()
        );

        // Cargar lo que haya en disco; si nada, sembrar madre es base.
        if store.cuerpos_len() >= 1 {
            eprintln!(
                "multilienzo_completo_demo :: cargando {} cuerpos de disco",
                store.cuerpos_len()
            );
            cargar_de_store(store.clone(), chat)
        } else {
            eprintln!("multilienzo_completo_demo :: sembrando madre es base");
            sembrar_madre_base(store.clone(), chat)
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
                lanzar_trabajo(&m, handle, TrabajoLlm::Traducir(lengua));
            }
            Msg::PedirTono(etiqueta) => {
                if m.en_curso || m.cuerpos.is_empty() {
                    return m;
                }
                m.en_curso = true;
                m.ultimo_error = None;
                lanzar_trabajo(&m, handle, TrabajoLlm::Tono(etiqueta));
            }
            Msg::PedirResumir(palabras) => {
                if m.en_curso || m.cuerpos.is_empty() {
                    return m;
                }
                m.en_curso = true;
                m.ultimo_error = None;
                lanzar_trabajo(&m, handle, TrabajoLlm::Resumir(palabras));
            }
            Msg::LlmListo {
                hija,
                atoms_nuevos,
                carta,
                transformacion,
            } => {
                // Persistir TODO antes de actualizar el modelo — si el
                // proceso muere entre los dos pasos, lo siguiente que
                // abra la store ya ve la transformación completa.
                for atom in &atoms_nuevos {
                    if let Err(e) = m.store.put_atom(atom) {
                        eprintln!("persistir atom falló: {e}");
                    }
                }
                if let Err(e) = m.store.put_cuerpo(&hija) {
                    eprintln!("persistir cuerpo falló: {e}");
                }
                if let Err(e) = m.store.put_transformacion(&transformacion) {
                    eprintln!("persistir transformación falló: {e}");
                }
                if let Err(e) = m.store.put_carta(&carta) {
                    eprintln!("persistir carta falló: {e}");
                }
                if let Err(e) = m.store.flush() {
                    eprintln!("flush falló: {e}");
                }
                // Actualizar el modelo de la app.
                for atom in atoms_nuevos {
                    m.graph.insert(atom);
                }
                m.cuerpos.push(hija);
                m.cartas.push(carta);
                m.en_curso = false;
            }
            Msg::LlmError(s) => {
                eprintln!("multilienzo_completo_demo :: error LLM: {s}");
                m.ultimo_error = Some(s);
                m.en_curso = false;
            }
            Msg::ScrollHoriz(dx) => {
                // El clamp duro lo aplica `view` (necesita medir el
                // ancho del contenido); aquí solo acumulamos y dejamos
                // que no se vaya negativo.
                m.scroll_x = (m.scroll_x + dx).max(0.0);
            }
            Msg::ToggleSoloMadre => {
                m.solo_madre = !m.solo_madre;
            }
            Msg::BuscarAgregar(c) => {
                m.busqueda.push(c);
            }
            Msg::BuscarBorrar => {
                m.busqueda.pop();
            }
            Msg::BuscarLimpiar => {
                m.busqueda.clear();
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let cfg = MultilienzoConfig::default();
        let paleta = PaletaHebras::default();
        let palette = Palette::default();
        let index: IndiceAtoms = model.graph.atoms().map(|a| (a.id, a)).collect();
        // Focus mode: si `solo_madre`, recortamos a la primera columna y
        // descartamos todas las cartas (no hay vecinos a la derecha).
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
            &paleta,
            &palette,
            &model.busqueda,
        );

        // Envoltorio scrollable: contenedor relative full-width que
        // recorta su contenido; el interior va position=Absolute con
        // left = -scroll_x. Sin clamp del lado del scroll (el update
        // ya impide negativo); el clip resuelve el desbordamiento
        // a la derecha visualmente.
        let cuerpos_view = View::new(Style {
            position: Position::Relative,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .clip(true)
        .children(vec![View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(-model.scroll_x),
                top: length(0.0_f32),
                right: auto(),
                bottom: auto(),
            },
            ..Default::default()
        })
        .children(vec![interior])]);

        let toolbar = toolbar_view::<Msg>(
            &palette,
            model.en_curso,
            &model.ultimo_error,
            model.cuerpos.len(),
            model.cartas.len(),
            model.solo_madre,
            &model.busqueda,
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
        .children(vec![toolbar, cuerpos_view])
    }
}

enum TrabajoLlm {
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
}

fn lanzar_trabajo(model: &Model, handle: &Handle<Msg>, trabajo: TrabajoLlm) {
    let madre = model.cuerpos[0].clone();
    let atoms_owned: Vec<NarrativeAtom> = model.graph.atoms().cloned().collect();
    let chat = model.chat.clone();
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
                    let producto = ej.aplicar_con_atoms(&t, &madre, &idx, ahora).await?;
                    Ok::<_, pluma_transform::ErrorEjecutor>((t, producto))
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
                    let producto = ej.aplicar_con_atoms(&t, &madre, &idx, ahora).await?;
                    Ok((t, producto))
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
                    let producto = ej.aplicar_con_atoms(&t, &madre, &idx, ahora).await?;
                    Ok((t, producto))
                }
            }
        });

        match resultado {
            Ok((transformacion, prod)) => Msg::LlmListo {
                hija: prod.hija,
                atoms_nuevos: prod.atoms_nuevos,
                carta: prod.carta,
                transformacion,
            },
            Err(e) => Msg::LlmError(format!("{e:?}")),
        }
    });
}

fn toolbar_view<Msg: Clone + 'static>(
    palette: &Palette,
    en_curso: bool,
    ultimo_error: &Option<String>,
    n_cuerpos: usize,
    n_cartas: usize,
    solo_madre: bool,
    busqueda: &str,
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

    // Focus mode siempre activo, no afectado por en_curso.
    let pal_focus = &p_activo;
    let label_focus = if solo_madre { "todos" } else { "solo madre" };

    let mut botones: Vec<View<Msg>> = vec![
        button_view::<Msg>("→ qu", pal, MsgUi::Traducir("qu".into()).into()),
        button_view::<Msg>("→ en", pal, MsgUi::Traducir("en".into()).into()),
        button_view::<Msg>("tono formal", pal, MsgUi::Tono("formal".into()).into()),
        button_view::<Msg>("resumir 30p", pal, MsgUi::Resumir(Some(30)).into()),
        button_view::<Msg>(label_focus, pal_focus, MsgUi::ToggleSoloMadre.into()),
    ];

    let busqueda_label = if busqueda.is_empty() {
        "🔍 (escribe para buscar · Esc limpia)".to_string()
    } else {
        format!("🔍 \"{busqueda}\"")
    };

    let status_text = if en_curso {
        format!("⏳ en curso… · {n_cuerpos} cuerpos, {n_cartas} cartas · {busqueda_label}")
    } else if let Some(e) = ultimo_error {
        format!("⚠ {}", &e[..e.len().min(80)])
    } else {
        format!(
            "{n_cuerpos} cuerpos · {n_cartas} cartas · {busqueda_label}"
        )
    };
    let status = View::new(Style {
        size: Size {
            width: length(450.0_f32),
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

#[derive(Clone, Debug)]
enum MsgUi {
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
    ToggleSoloMadre,
}

impl From<MsgUi> for Msg {
    fn from(u: MsgUi) -> Self {
        match u {
            MsgUi::Traducir(l) => Msg::PedirTraducir(l),
            MsgUi::Tono(e) => Msg::PedirTono(e),
            MsgUi::Resumir(p) => Msg::PedirResumir(p),
            MsgUi::ToggleSoloMadre => Msg::ToggleSoloMadre,
        }
    }
}

fn cargar_de_store(store: Arc<PlumaStore>, chat: Arc<dyn ChatClient>) -> Model {
    let mut graph = NarrativeGraph::new();
    for atom in store.iter_atoms() {
        graph.insert(atom.expect("leer atom"));
    }
    let mut cuerpos: Vec<Cuerpo> = store
        .iter_cuerpos()
        .map(|c| c.expect("leer cuerpo"))
        .collect();
    // Original al frente; el resto en orden de creación (modificado_en).
    cuerpos.sort_by_key(|c| {
        let prioridad = if matches!(c.metadatos.intencion, Intencion::Original) {
            0
        } else {
            1
        };
        (prioridad, c.metadatos.creado_en)
    });

    let mut cartas: Vec<CartaHebras> = Vec::new();
    for w in cuerpos.windows(2) {
        if let Some(c) = store
            .get_carta_bidir(w[0].id, w[1].id)
            .expect("leer carta")
        {
            cartas.push(c);
        }
    }
    Model {
        cuerpos,
        graph,
        cartas,
        chat,
        store,
        en_curso: false,
        ultimo_error: None,
        scroll_x: 0.0,
        solo_madre: false,
        busqueda: String::new(),
    }
}

fn sembrar_madre_base(store: Arc<PlumaStore>, chat: Arc<dyn ChatClient>) -> Model {
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
    // Persistir la madre base.
    for atom in graph.atoms() {
        store.put_atom(atom).expect("persistir atom");
    }
    store.put_cuerpo(&es).expect("persistir cuerpo");
    store.flush().expect("flush");

    Model {
        cuerpos: vec![es],
        graph,
        cartas: Vec::new(),
        chat,
        store,
        en_curso: false,
        ultimo_error: None,
        scroll_x: 0.0,
        solo_madre: false,
        busqueda: String::new(),
    }
}

fn cache_dir() -> PathBuf {
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("gioser").join("pluma-multilienzo-completo")
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
            .con_model_id("mock-completo");
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
    let _ = Path::new("/"); // silenciar import si no se usa
    llimphi_ui::run::<Demo>();
}
