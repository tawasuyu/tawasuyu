//! `mirada-asistente` — el asistente conversacional del escritorio carmen.
//!
//! Una ventana Llimphi con un input de texto donde el operador escribe en
//! lenguaje natural ("foca la siguiente ventana", "manda esta a workspace 3",
//! "cierra el navegador"). El asistente:
//!
//!   1. Manda la petición a un LLM via `pluma-llm::from_env()` (Anthropic,
//!      Gemini, DeepSeek, Cohere u Ollama según las env vars del operador;
//!      Mock como fallback).
//!   2. Pide al modelo que responda con un JSON `{accion, args, explicacion}`
//!      donde `accion` es uno de los subcomandos de `mirada-ctl` y `args` un
//!      array de strings.
//!   3. Presenta la propuesta al operador. Hasta que pulse "Ejecutar", el
//!      asistente NO toca el compositor — el humano sigue siendo el portador
//!      de la autoridad. La IA propone; el humano firma.
//!   4. Si confirma, spawnea `mirada-ctl <accion> <args...>` y reporta el
//!      resultado (stdout/stderr, código de salida).
//!
//! Diseño deliberado: el asistente NO ejerce comandos directamente sobre el
//! socket de `mirada-brain` ni evade `mirada-ctl`. Pasar por la CLI dejada por
//! el proyecto significa que cualquier auditoría futura ve los mismos eventos
//! que vería un humano tipeando — la IA no inventa caminos nuevos al núcleo
//! del compositor.

use std::process::Command;
use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use pluma_llm_core::{ChatClient, ChatRequest};
use serde::Deserialize;

/// `app_id` con el que el compositor reconoce y compone el asistente.
const ASISTENTE_APP_ID: &str = "carmen.asistente";

/// El prompt de sistema: instruye al modelo a responder estrictamente con
/// JSON que mapea a un subcomando de `mirada-ctl`. Lista las acciones
/// disponibles tal como las imprime `mirada-ctl --help` — si la CLI gana
/// acciones nuevas, esta lista hay que extenderla a mano (deliberadamente:
/// queremos que el LLM jamás invente acciones).
const PROMPT_SISTEMA: &str = "Eres el asistente del compositor Wayland `mirada` (escritorio carmen). \
El usuario describe lo que quiere hacer y tú respondes EXCLUSIVAMENTE con un \
objeto JSON con esta forma exacta:\n\
\n\
  {\"accion\": \"focus-next\", \"args\": [], \"explicacion\": \"breve por qué\"}\n\
\n\
Si no entiendes la intención o no hay acción adecuada, responde:\n\
\n\
  {\"error\": \"razón breve\"}\n\
\n\
Acciones disponibles (subcomandos de `mirada-ctl`, los args son strings):\n\
  focus-next, focus-prev, focus-window <id>, move-forward, move-backward,\n\
  close-focused, toggle-float, toggle-fullscreen, send-to-scratchpad,\n\
  toggle-scratchpad, cycle-layout, layout <modo:master-stack|centered-master|\n\
  spiral|grid|columns|rows|monocle>, grow-master, shrink-master, inc-master,\n\
  dec-master, promote-to-master, workspace <n:1-9>, send-to-workspace <n:1-9>,\n\
  focus-output-next, quit.\n\
\n\
REGLAS: (1) responde SOLO con el JSON, sin prosa antes ni después. (2) NO \
inventes acciones que no estén en la lista. (3) Si la petición pide algo \
destructivo (quit, close-focused, kill), inclúyelo igual — el humano confirma \
antes de ejecutar.";

/// Cota de tokens de salida — el JSON resultante es chico (típicamente
/// <100 tokens). Mantenemos margen para `explicacion` algo prolija.
const MAX_TOKENS_RESPUESTA: u32 = 300;

/// La forma JSON que el modelo debe producir cuando entiende la petición.
#[derive(Debug, Clone, Deserialize)]
struct Propuesta {
    accion: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    explicacion: String,
}

/// La forma JSON que el modelo debe producir cuando NO entiende — un solo
/// campo `error` con la razón. Distinto de "error del transporte" (ChatError):
/// éste es el LLM diciendo "no sé qué hacer con tu pedido".
#[derive(Debug, Clone, Deserialize)]
struct Rechazo {
    error: String,
}

/// El cliente LLM compartible entre el hilo de UI y los workers de fondo.
type DynLlm = Arc<dyn ChatClient>;

fn main() {
    llimphi_ui::run::<Asistente>();
}

// ---------------------------------------------------------------------
// Modelo + mensajes
// ---------------------------------------------------------------------

enum Estado {
    /// Sin pedido en curso, esperando que el operador escriba.
    Idle,
    /// LLM corriendo; el spinner debería estar visible.
    Consultando,
    /// El LLM produjo una propuesta. El operador puede ejecutar o descartar.
    Propuesta(Propuesta),
    /// El último intento falló (error de transporte, JSON inválido, o LLM
    /// rechazando con `{error: ...}`). El operador puede reintentar.
    Error(String),
    /// La acción se ejecutó. El operador puede leer el resultado y empezar
    /// otra petición.
    Ejecutado {
        accion: String,
        salida: String,
        ok: bool,
    },
}

struct Model {
    llm: Option<DynLlm>,
    /// Razón por la que el LLM no pudo inicializarse (sin credenciales, etc.).
    /// Si `llm` es `None`, este campo dice POR QUÉ. La UI lo muestra arriba
    /// del input para que el operador sepa qué env var le falta.
    init_error: Option<String>,
    pregunta: TextInputState,
    estado: Estado,
}

#[derive(Clone)]
enum Msg {
    EditKey(KeyEvent),
    Submit,
    /// Resultado del LLM: la respuesta cruda, antes de parsear el JSON. El
    /// worker pre-formatea el error como string para no obligar a `ChatError`
    /// a ser `Clone` (que `thiserror` no provee).
    LlmDone(Result<String, String>),
    /// El operador pulsó "Ejecutar" en la propuesta vigente.
    EjecutarPropuesta,
    /// Resultado de spawnear `mirada-ctl`: (accion, salida, ok).
    EjecucionDone {
        accion: String,
        salida: String,
        ok: bool,
    },
    /// El operador pulsó "Descartar" o quiere empezar otra petición.
    Limpiar,
}

// ---------------------------------------------------------------------
// Bucle Elm
// ---------------------------------------------------------------------

struct Asistente;

impl App for Asistente {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "carmen · asistente"
    }

    fn app_id() -> Option<&'static str> {
        Some(ASISTENTE_APP_ID)
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        // `pluma_llm::from_env()` lee env vars (`ANTHROPIC_API_KEY`,
        // `GEMINI_API_KEY`, etc.) y cae a Mock si no encuentra ninguna. Esto
        // significa que el asistente arranca SIEMPRE — sin clave, devuelve
        // respuestas del Mock, lo que sirve para probar el flujo de UI.
        let (llm, init_error) = match pluma_llm::from_env() {
            Ok(client) => (Some(client), None),
            Err(e) => (None, Some(format!("LLM no disponible: {e}"))),
        };
        Model {
            llm,
            init_error,
            pregunta: TextInputState::new(),
            estado: Estado::Idle,
        }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Mientras el LLM trabaja, no consumimos teclas — evitamos que el
        // operador edite la pregunta a medio camino y se confunda con la
        // respuesta entrante.
        if matches!(model.estado, Estado::Consultando) {
            return None;
        }
        match &e.key {
            Key::Named(NamedKey::Enter) => Some(Msg::Submit),
            Key::Named(NamedKey::Escape) => Some(Msg::Limpiar),
            _ => Some(Msg::EditKey(e.clone())),
        }
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::EditKey(ev) => {
                m.pregunta.apply_key(&ev);
            }
            Msg::Submit => {
                let prompt = m.pregunta.text().trim().to_string();
                if prompt.is_empty() {
                    return m;
                }
                let Some(llm) = m.llm.clone() else {
                    m.estado = Estado::Error(
                        m.init_error
                            .clone()
                            .unwrap_or_else(|| "LLM no inicializado".into()),
                    );
                    return m;
                };
                m.estado = Estado::Consultando;
                let req = ChatRequest::una_vuelta(prompt, MAX_TOKENS_RESPUESTA)
                    .con_sistema(PROMPT_SISTEMA);
                handle.spawn(move || {
                    // Cada call abre su propio runtime de Tokio porque el
                    // worker de Llimphi es síncrono y vivir entre messages
                    // sin un runtime compartido es más simple que reusar uno.
                    // El costo (~ms) es invisible frente al RTT del LLM (~s).
                    let resp = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("tokio runtime")
                        .block_on(llm.complete(&req));
                    Msg::LlmDone(resp.map(|r| r.content).map_err(|e| e.to_string()))
                });
            }
            Msg::LlmDone(Ok(texto)) => {
                m.estado = parsear_respuesta(&texto);
            }
            Msg::LlmDone(Err(motivo)) => {
                m.estado = Estado::Error(format!("transporte: {motivo}"));
            }
            Msg::EjecutarPropuesta => {
                let Estado::Propuesta(p) = &m.estado else {
                    return m;
                };
                let accion = p.accion.clone();
                let args = p.args.clone();
                let etiqueta = accion.clone();
                m.estado = Estado::Consultando; // spinner mientras spawnea
                handle.spawn(move || {
                    let salida = ejecutar_mirada_ctl(&accion, &args);
                    Msg::EjecucionDone {
                        accion: etiqueta,
                        salida: salida.salida,
                        ok: salida.ok,
                    }
                });
            }
            Msg::EjecucionDone { accion, salida, ok } => {
                m.estado = Estado::Ejecutado { accion, salida, ok };
                m.pregunta.clear();
            }
            Msg::Limpiar => {
                m.estado = Estado::Idle;
                m.pregunta.clear();
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = Theme::dark();
        let input_palette = TextInputPalette::from_theme(&theme);

        let title = row(28.0, "carmen · asistente", 22.0, theme.fg_text);
        let sub = row(
            14.0,
            "describe lo que quieres hacer; el asistente propone, tú confirmas.",
            11.0,
            theme.fg_muted,
        );

        // Si el LLM no inicializó, un banner discreto explica qué falta.
        let banner = match &model.init_error {
            Some(err) => row(18.0, err, 12.0, theme.fg_destructive),
            None => row(0.0, "", 0.0, theme.fg_muted),
        };

        let input = text_input_view(
            &model.pregunta,
            "¿qué quieres hacer? (Enter para preguntar, Esc para limpiar)",
            true,
            &input_palette,
            Msg::Limpiar, // click en el input cuando NO está enfocado: limpiar
        );

        // El cuerpo varía con el estado.
        let cuerpo: Vec<View<Msg>> = match &model.estado {
            Estado::Idle => vec![],
            Estado::Consultando => {
                vec![row(20.0, "pensando…", 14.0, theme.fg_muted)]
            }
            Estado::Propuesta(p) => {
                let resumen = format!("mirada-ctl {} {}", p.accion, p.args.join(" "));
                vec![
                    row(22.0, &resumen, 16.0, theme.fg_text),
                    row(18.0, &p.explicacion, 12.0, theme.fg_muted),
                    botonera(&theme),
                ]
            }
            Estado::Error(e) => {
                vec![row(20.0, e, 13.0, theme.fg_destructive)]
            }
            Estado::Ejecutado { accion, salida, ok } => {
                let cabecera = if *ok {
                    format!("✓ {} ejecutado", accion)
                } else {
                    format!("✗ {} falló", accion)
                };
                let tinta = if *ok {
                    theme.fg_text
                } else {
                    theme.fg_destructive
                };
                vec![
                    row(20.0, &cabecera, 14.0, tinta),
                    row(18.0, salida, 12.0, theme.fg_muted),
                ]
            }
        };

        let mut hijos = vec![title, sub, banner, input];
        hijos.extend(cuerpo);

        let panel = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: length(520.0_f32),
                height: Dimension::auto(),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(10.0_f32),
            },
            padding: Rect {
                left: length(28.0_f32),
                right: length(28.0_f32),
                top: length(28.0_f32),
                bottom: length(28.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(12.0)
        .children(hijos);

        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![panel])
    }
}

// ---------------------------------------------------------------------
// Parseo de la respuesta del LLM
// ---------------------------------------------------------------------

/// Intenta interpretar `texto` como JSON producido por el LLM. La respuesta
/// puede venir limpia (el modelo siguió las instrucciones) o con basurilla
/// alrededor (markdown fences, prosa); buscamos el primer `{...}` balanceado
/// y parseamos eso. Si todo falla, dejamos un `Estado::Error` con la cadena
/// cruda — el operador puede leerla y reformular su pedido.
fn parsear_respuesta(texto: &str) -> Estado {
    let Some(json) = extraer_objeto_json(texto) else {
        return Estado::Error(format!("respuesta sin JSON: {texto}"));
    };
    if let Ok(propuesta) = serde_json::from_str::<Propuesta>(json) {
        // `accion` debe ser no vacía — si lo es, el JSON estaba mal formado
        // semánticamente aunque parseara.
        if propuesta.accion.is_empty() {
            return Estado::Error(format!("propuesta sin accion: {texto}"));
        }
        return Estado::Propuesta(propuesta);
    }
    if let Ok(rechazo) = serde_json::from_str::<Rechazo>(json) {
        return Estado::Error(rechazo.error);
    }
    Estado::Error(format!("JSON no reconocido: {texto}"))
}

/// Devuelve la primera sub-cadena de `texto` que es un objeto JSON
/// balanceado por `{` y `}`. Tolerante a markdown fences y prosa alrededor.
/// `None` si no encuentra nada balanceado.
fn extraer_objeto_json(texto: &str) -> Option<&str> {
    let bytes = texto.as_bytes();
    let inicio = texto.find('{')?;
    let mut prof: usize = 0;
    for (offset, &b) in bytes[inicio..].iter().enumerate() {
        match b {
            b'{' => prof += 1,
            b'}' => {
                prof -= 1;
                if prof == 0 {
                    return Some(&texto[inicio..=inicio + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------
// Ejecución de mirada-ctl
// ---------------------------------------------------------------------

struct SalidaCmd {
    salida: String,
    ok: bool,
}

/// Spawnea `mirada-ctl <accion> <args...>` y captura stdout+stderr. Si el
/// binario no está en PATH, devuelve un mensaje que lo dice — el operador
/// puede instalar/agregar el path.
fn ejecutar_mirada_ctl(accion: &str, args: &[String]) -> SalidaCmd {
    let mut cmd = Command::new("mirada-ctl");
    cmd.arg(accion);
    for a in args {
        cmd.arg(a);
    }
    match cmd.output() {
        Ok(out) => {
            let mut salida = String::from_utf8_lossy(&out.stdout).into_owned();
            if !out.stderr.is_empty() {
                if !salida.is_empty() {
                    salida.push('\n');
                }
                salida.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            if salida.is_empty() {
                salida = if out.status.success() {
                    "(sin salida)".into()
                } else {
                    format!("código {}", out.status.code().unwrap_or(-1))
                };
            }
            SalidaCmd {
                salida,
                ok: out.status.success(),
            }
        }
        Err(e) => SalidaCmd {
            salida: format!("spawn falló: {e} (¿está `mirada-ctl` en PATH?)"),
            ok: false,
        },
    }
}

// ---------------------------------------------------------------------
// Helpers de vista
// ---------------------------------------------------------------------

/// Fila de ancho completo con un texto a la izquierda.
fn row(height: f32, text: &str, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(height),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), size, color, Alignment::Start)
}

/// Botonera "Ejecutar | Descartar" para una propuesta vigente. Llimphi no
/// expone un widget de botón en este crate, así que tomamos prestada la
/// estética del greeter: dos cajas clicables con texto centrado.
fn botonera(theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(36.0_f32),
        },
        gap: Size {
            width: length(12.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        boton("Ejecutar", theme.fg_text, theme.bg_app, Msg::EjecutarPropuesta),
        boton("Descartar", theme.fg_muted, theme.bg_panel, Msg::Limpiar),
    ])
}

fn boton(label: &str, fg: Color, bg: Color, on_click: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(0.5_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(8.0)
    .text_aligned(label.to_string(), 14.0, fg, Alignment::Start)
    .on_click(on_click)
}
