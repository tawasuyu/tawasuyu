//! `shuma-module-agente` — el panel de chat multi-agente de shuma.
//!
//! Frontend del núcleo [`shuma_agente`], al estilo de las apps web de IA:
//! sidebar con la lista de conversaciones + selector de agente, un hilo central
//! con los turnos (cada bloque del asistente pintado según su tipo) y un input
//! abajo. Las acciones de control aparecen como tarjetas con **aprobar /
//! rechazar** — nunca se ejecutan solas.
//!
//! Sigue el contrato estructural de los módulos shuma (como
//! `shuma-module-commandbar`): `State` + `Msg` + `update` puro + `view` + las
//! funciones de provisión que el chasis llama fuera del `update`
//! ([`State::set_agentes`], [`State::set_conversaciones`], [`State::fijar_reloj`]).
//!
//! ## Trabajo async (mismo patrón intent que el shell)
//!
//! El módulo **no habla con la red**: cuando el usuario manda un mensaje, deja
//! una [`Peticion`] en `pendiente`; el chasis la toma con [`State::take_request`],
//! corre `shuma-agente-host::responder` en un thread, y devuelve el resultado
//! como [`Msg::Respuesta`]. Igual con las acciones aprobadas
//! ([`State::take_ejecucion`]).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{
        length, percent, AlignItems, Dimension, FlexDirection, JustifyContent, LengthPercentage,
        Size, Style,
    },
    Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_theme::Theme;
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_scroll::{scroll_y, ScrollPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use shuma_agente::{Agente, BloqueSalida, Conversacion, EstadoAccion, Peligro};
use shuma_module::{ModuleContributions, Placement};

/// `id` canónico del módulo.
pub const ID: &str = "agente";

/// `Placement` por defecto: ocupa el área principal.
pub const DEFAULT_PLACEMENT: Placement = Placement::Main;

const SIDEBAR_W: f32 = 150.0;
const VISTA_ALTO_DEFAULT: f32 = 600.0;

/// Lo que el chasis debe cumplir: responder un turno con `pluma-llm`. El módulo
/// la deja servida; el chasis le inyecta el backend de fallback global.
#[derive(Debug, Clone)]
pub struct Peticion {
    /// La conversación con el último mensaje del usuario ya agregado.
    pub conv: Conversacion,
    /// El agente que la responde (con su backend propio).
    pub agente: Agente,
}

/// Backends que el editor de agentes ofrece (se ciclan con un click). El
/// primero, `claude-cli`, usa la suscripción de Claude Code sin API key.
const BACKENDS: &[&str] = &[
    "claude-cli",
    "anthropic",
    "gemini",
    "deepseek",
    "cohere",
    "ollama",
    "mock",
    "", // vacío = heredar el backend global del SO
];

/// Qué campo de texto del editor tiene el foco del teclado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Campo {
    Nombre,
    Modelo,
    Persona,
}

/// Formulario de alta/edición de un agente. Abierto = el panel muestra el
/// editor en vez del hilo.
#[derive(Debug, Clone)]
struct EditorAgente {
    /// Id del agente que se edita; `None` = uno nuevo.
    id: Option<String>,
    nombre: TextInputState,
    modelo: TextInputState,
    persona: TextInputState,
    /// Índice en [`BACKENDS`].
    backend_idx: usize,
    control: bool,
    foco: Campo,
}

impl EditorAgente {
    fn nuevo() -> Self {
        Self {
            id: None,
            nombre: TextInputState::new(),
            modelo: TextInputState::new(),
            persona: TextInputState::new(),
            backend_idx: 0,
            control: false,
            foco: Campo::Nombre,
        }
    }

    fn desde(a: &Agente) -> Self {
        let backend_idx = BACKENDS
            .iter()
            .position(|b| *b == a.backend.backend)
            .unwrap_or(BACKENDS.len() - 1);
        let mut nombre = TextInputState::new();
        nombre.set_text(&a.nombre);
        let mut modelo = TextInputState::new();
        modelo.set_text(&a.backend.model);
        let mut persona = TextInputState::new();
        persona.set_text(&a.system_prompt);
        Self {
            id: Some(a.id.clone()),
            nombre,
            modelo,
            persona,
            backend_idx,
            control: a.capacidades.control,
            foco: Campo::Nombre,
        }
    }

    fn campo_mut(&mut self) -> &mut TextInputState {
        match self.foco {
            Campo::Nombre => &mut self.nombre,
            Campo::Modelo => &mut self.modelo,
            Campo::Persona => &mut self.persona,
        }
    }

    /// Construye el `Agente` a guardar a partir del formulario. Conserva el `id`
    /// si se edita; uno nuevo si no.
    fn a_agente(&self) -> Agente {
        let mut a = match &self.id {
            Some(id) => {
                let mut a = Agente::nuevo(self.nombre.text());
                a.id = id.clone();
                a
            }
            None => Agente::nuevo(self.nombre.text()),
        };
        a.system_prompt = self.persona.text();
        a.backend = wawa_config::LlmSettings {
            backend: BACKENDS[self.backend_idx].to_string(),
            model: self.modelo.text(),
            ..Default::default()
        };
        a.capacidades.control = self.control;
        a
    }
}

/// Estado de la **escucha por voz**, para el indicador del micrófono. Lo fija el
/// chasis a partir de los `EventoEscucha` de `rimay-voz-host`; el panel sólo lo
/// pinta (halo del botón + glow del input).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EstadoEscucha {
    /// Micrófono apagado.
    #[default]
    Apagado,
    /// Encendido y armado, esperando la palabra de llamada.
    Esperando,
    /// El VAD detectó voz (alguien habla) — transitorio.
    Oyendo,
    /// Despertó con el llamado; listo para dictar.
    Despierto,
    /// Dictando: el texto fluye al input.
    Dictando,
}

impl EstadoEscucha {
    /// `true` si el micrófono está encendido (cualquier estado salvo apagado).
    pub fn activo(self) -> bool {
        !matches!(self, EstadoEscucha::Apagado)
    }
}

/// Estado del panel de chat. Las conversaciones y agentes los **provee el
/// chasis** desde el [`shuma_agente::Almacen`]; el módulo los edita en memoria y
/// el chasis persiste tras cada `update`.
#[derive(Debug, Clone)]
pub struct State {
    agentes: Vec<Agente>,
    /// Índice del agente activo dentro de `agentes`.
    agente_sel: usize,
    /// Conversaciones, más recientes primero (orden del sidebar).
    conversaciones: Vec<Conversacion>,
    /// Id de la conversación abierta (estable ante reordenamientos).
    conv_activa: Option<String>,
    input: TextInputState,
    focused: bool,
    scroll: f32,
    /// Reloj inyectado por el chasis (epoch ms) — el `update` no lee el reloj.
    reloj_ms: u64,
    /// Alto del viewport del hilo (px) — lo fija el chasis según el panel.
    vista_alto: f32,
    /// `true` mientras un turno está en vuelo (lo tomó el chasis).
    esperando: bool,
    /// Intent de responder un turno; `None` salvo entre el envío y su resultado.
    pendiente: Option<Peticion>,
    /// Intent de ejecutar una acción aprobada; lo corre el chasis (shell).
    ejecucion: Option<shuma_agente::AccionPropuesta>,
    /// Texto del turno del asistente que está llegando en streaming; `None`
    /// salvo entre el envío y la respuesta final. Se pinta como una burbuja viva.
    parcial: Option<String>,
    /// Editor de agente abierto; `None` = se muestra el hilo.
    editor: Option<EditorAgente>,
    /// Intent: agente a persistir (alta/edición); lo escribe el chasis al Almacen.
    persist_agente: Option<Agente>,
    /// Intent: id de agente a borrar; lo borra el chasis del Almacen.
    borrar_agente_id: Option<String>,
    /// Intent: id de conversación a borrar del Almacen.
    borrar_conv_id: Option<String>,
    /// Renombre en curso: `(id de conversación, input con el título)`. `None` =
    /// no se está renombrando.
    renombrando: Option<(String, TextInputState)>,
    /// Estado de la escucha por voz (lo fija el chasis con `fijar_escucha`).
    escucha: EstadoEscucha,
    /// Intent: el usuario pidió encender (`Some(true)`) o apagar (`Some(false)`)
    /// el micrófono; el chasis lo toma con [`State::tomar_mic_intent`] y arranca
    /// o para la captura de `rimay-voz-host`. `None` = nada pendiente.
    mic_intent: Option<bool>,
    /// Enrolamiento del wake-word en curso: `Some(n)` = ya se grabaron `n`
    /// muestras de «shuma» (de [`ENROL_OBJETIVO`]); `None` = no se está enrolando.
    enrolando: Option<u8>,
    /// Intent de enrolar (`Some(true)` arrancar, `Some(false)` cancelar); el
    /// chasis lo toma con [`State::tomar_enrol_intent`] y corre `rimay_voz_host::enrolar`.
    enrol_intent: Option<bool>,
    /// `true` si ya hay un wake-word enrolado (lo fija el chasis al cargar / tras
    /// enrolar). Sólo rotula la UI; la compuerta real la monta el chasis.
    wake_listo: bool,
}

/// Cuántas grabaciones de «shuma» pide el enrolamiento.
pub const ENROL_OBJETIVO: u8 = 3;

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            agentes: Vec::new(),
            agente_sel: 0,
            conversaciones: Vec::new(),
            conv_activa: None,
            input: TextInputState::new(),
            focused: false,
            scroll: 0.0,
            reloj_ms: 0,
            vista_alto: VISTA_ALTO_DEFAULT,
            esperando: false,
            pendiente: None,
            ejecucion: None,
            parcial: None,
            editor: None,
            persist_agente: None,
            borrar_agente_id: None,
            borrar_conv_id: None,
            renombrando: None,
            escucha: EstadoEscucha::Apagado,
            mic_intent: None,
            enrolando: None,
            enrol_intent: None,
            wake_listo: false,
        }
    }

    // ── Provisión por el chasis (fuera del update, como set_catalog) ────────

    /// Inyecta los agentes disponibles (del Almacen). Mantiene la selección en
    /// rango.
    pub fn set_agentes(&mut self, agentes: Vec<Agente>) {
        self.agentes = agentes;
        if self.agente_sel >= self.agentes.len() {
            self.agente_sel = 0;
        }
    }

    /// Fija el estado de la escucha por voz (lo llama el chasis al recibir un
    /// `EventoEscucha` de `rimay-voz-host`). Si la escucha se apagó por su cuenta
    /// (timeout, error), el indicador vuelve a apagado.
    pub fn fijar_escucha(&mut self, e: EstadoEscucha) {
        self.escucha = e;
    }

    /// Estado actual de la escucha (para el chasis / tests).
    pub fn escucha(&self) -> EstadoEscucha {
        self.escucha
    }

    /// Toma el intent de encender/apagar el micrófono y lo limpia. El chasis lo
    /// consulta tras cada `update` y arranca o para `rimay-voz-host`.
    pub fn tomar_mic_intent(&mut self) -> Option<bool> {
        self.mic_intent.take()
    }

    /// Toma el intent de enrolar (arrancar/cancelar) y lo limpia.
    pub fn tomar_enrol_intent(&mut self) -> Option<bool> {
        self.enrol_intent.take()
    }

    /// Progreso del enrolamiento (`Some(n)` grabadas, `None` si no enrola).
    pub fn enrolando(&self) -> Option<u8> {
        self.enrolando
    }

    /// El chasis avisa que grabó una muestra más de «shuma» (avanza el contador).
    pub fn enrol_capturado(&mut self) {
        if let Some(n) = self.enrolando.as_mut() {
            *n = n.saturating_add(1).min(ENROL_OBJETIVO);
        }
    }

    /// El chasis avisa que el enrolamiento terminó y el wake-word quedó listo.
    pub fn enrol_terminado(&mut self) {
        self.enrolando = None;
        self.wake_listo = true;
    }

    /// El chasis fija si ya hay un wake-word enrolado (al cargar la config).
    pub fn set_wake_listo(&mut self, listo: bool) {
        self.wake_listo = listo;
    }

    /// Inyecta las conversaciones (más recientes primero). Si la activa ya no
    /// existe, la deselecciona.
    pub fn set_conversaciones(&mut self, convs: Vec<Conversacion>) {
        if let Some(id) = &self.conv_activa {
            if !convs.iter().any(|c| &c.id == id) {
                self.conv_activa = None;
            }
        }
        self.conversaciones = convs;
    }

    /// Fija el reloj (epoch ms) que usa el `update` para estampar turnos.
    pub fn fijar_reloj(&mut self, ms: u64) {
        self.reloj_ms = ms;
    }

    /// Abre la conversación más reciente si no hay ninguna activa (al arrancar,
    /// para reanudar donde se dejó — como las apps web de IA). No-op si ya hay
    /// una abierta o no hay conversaciones.
    pub fn abrir_mas_reciente(&mut self) {
        if self.conv_activa.is_none() {
            if let Some(c) = self.conversaciones.first() {
                self.conv_activa = Some(c.id.clone());
            }
        }
    }

    /// Fija el alto del viewport del hilo (px).
    pub fn fijar_vista_alto(&mut self, h: f32) {
        self.vista_alto = h.max(120.0);
    }

    /// Marca el input como (des)enfocado. El chasis lo enfoca cuando el diente
    /// del chat está activo.
    pub fn set_focus(&mut self, f: bool) {
        self.focused = f;
    }

    /// `true` si hay una petición servida esperando que el chasis la corra.
    pub fn tiene_pendiente(&self) -> bool {
        self.pendiente.is_some()
    }

    /// El chasis toma la petición pendiente para correr `pluma-llm`. Marca el
    /// turno en vuelo para no re-dispararlo.
    pub fn take_request(&mut self) -> Option<Peticion> {
        self.pendiente.take()
    }

    /// El chasis toma una acción aprobada para ejecutarla (en el shell).
    pub fn take_ejecucion(&mut self) -> Option<shuma_agente::AccionPropuesta> {
        self.ejecucion.take()
    }

    /// El chasis toma un agente a persistir (alta/edición) para escribirlo al
    /// Almacen y re-proveer la lista con [`State::set_agentes`].
    pub fn take_persist_agente(&mut self) -> Option<Agente> {
        self.persist_agente.take()
    }

    /// El chasis toma el id de un agente a borrar del Almacen.
    pub fn take_borrar_agente(&mut self) -> Option<String> {
        self.borrar_agente_id.take()
    }

    /// El chasis toma el id de una conversación a borrar del Almacen.
    pub fn take_borrar_conversacion(&mut self) -> Option<String> {
        self.borrar_conv_id.take()
    }

    /// Las conversaciones actuales (para que el chasis persista tras un update).
    pub fn conversaciones(&self) -> &[Conversacion] {
        &self.conversaciones
    }

    /// La conversación abierta, si hay.
    pub fn conversacion_activa(&self) -> Option<&Conversacion> {
        let id = self.conv_activa.as_ref()?;
        self.conversaciones.iter().find(|c| &c.id == id)
    }

    fn conversacion_activa_mut(&mut self) -> Option<&mut Conversacion> {
        let id = self.conv_activa.clone()?;
        self.conversaciones.iter_mut().find(|c| c.id == id)
    }

    fn agente_activo(&self) -> Option<&Agente> {
        self.agentes.get(self.agente_sel)
    }
}

/// Mensajes del panel.
#[derive(Debug, Clone)]
pub enum Msg {
    /// Tecla desde el chasis (cuando el input tiene foco).
    Key(KeyEvent),
    /// Click en el input → toma foco.
    FocusInput,
    /// Enviar el texto del input como turno de usuario.
    Enviar,
    /// Empezar una conversación nueva con el agente activo.
    NuevaConversacion,
    /// Abrir la conversación en esa posición de la lista.
    AbrirConversacion(usize),
    /// Borrar la conversación en esa posición.
    BorrarConversacion(usize),
    /// Empezar a renombrar la conversación en esa posición.
    RenombrarConversacion(usize),
    /// Confirmar el renombre en curso.
    ConfirmarRenombre,
    /// Elegir el agente en esa posición.
    SeleccionarAgente(usize),
    /// Rueda/arrastre del hilo (delta en px).
    Scroll(f32),
    /// Aprobar la acción del bloque `bloque` del turno `turno`.
    Aprobar { turno: usize, bloque: usize },
    /// Rechazar esa acción.
    Rechazar { turno: usize, bloque: usize },
    /// Fragmento de texto en streaming (lo dispatcha el chasis token a token).
    Token { conv_id: String, delta: String },
    /// Resultado del turno (lo dispatcha el chasis tras correr pluma-llm).
    Respuesta {
        conv_id: String,
        bloques: Vec<BloqueSalida>,
        ok: bool,
        /// Tokens reportados por el backend (0 si no los expone).
        entrada: u32,
        salida: u32,
    },
    /// Abrir el editor para crear un agente nuevo.
    NuevoAgente,
    /// Abrir el editor del agente seleccionado.
    EditarAgente,
    /// Enfocar un campo de texto del editor.
    EditorFoco(Campo),
    /// Ciclar el backend del editor (claude-cli → anthropic → …).
    EditorCiclarBackend,
    /// Alternar la capacidad de control del editor.
    EditorToggleControl,
    /// Guardar el agente del editor (alta/edición).
    GuardarAgente,
    /// Borrar el agente que se está editando.
    BorrarAgente,
    /// Cerrar el editor sin guardar.
    CancelarEditor,
    /// Click en el micrófono: alterna encender/apagar la escucha por voz.
    ToggleMic,
    /// El chasis reporta un cambio de estado de la escucha por voz.
    EscuchaCambio(EstadoEscucha),
    /// Texto dictado por voz: se inserta en el input (no envía solo).
    Dictado(String),
    /// Empezar a enrolar la palabra de llamada (grabar «shuma» ×N).
    EnrolarWake,
    /// El chasis grabó una muestra más de «shuma» (avanza el contador).
    EnrolarCapturado,
    /// El chasis terminó: el wake-word quedó enrolado.
    EnrolarHecho,
    /// Cancelar el enrolamiento en curso.
    EnrolarCancelar,
}

/// Transición pura del estado.
pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::Key(ev) => {
            if ev.state != KeyState::Pressed {
                return s;
            }
            // Renombre en curso: teclas al input del título (Enter confirma,
            // Escape cancela).
            if let Some((_, input)) = s.renombrando.as_mut() {
                match &ev.key {
                    Key::Named(NamedKey::Escape) => s.renombrando = None,
                    Key::Named(NamedKey::Enter) => return update(s, Msg::ConfirmarRenombre),
                    _ => {
                        input.apply_key(&ev);
                    }
                }
                return s;
            }
            // Con el editor abierto, las teclas van al campo enfocado (Tab cicla;
            // Escape cancela). No se envía mensaje.
            if let Some(ed) = s.editor.as_mut() {
                match &ev.key {
                    Key::Named(NamedKey::Escape) => return update(s, Msg::CancelarEditor),
                    Key::Named(NamedKey::Tab) => {
                        ed.foco = match ed.foco {
                            Campo::Nombre => Campo::Modelo,
                            Campo::Modelo => Campo::Persona,
                            Campo::Persona => Campo::Nombre,
                        };
                    }
                    _ => {
                        ed.campo_mut().apply_key(&ev);
                    }
                }
                return s;
            }
            // Enter (sin Shift) envía; el resto lo consume el input.
            if let Key::Named(NamedKey::Enter) = ev.key {
                if !ev.modifiers.shift {
                    return update(s, Msg::Enviar);
                }
            }
            s.input.apply_key(&ev);
        }
        Msg::FocusInput => s.focused = true,
        Msg::Enviar => {
            if s.esperando {
                return s;
            }
            // Líneas `img:<ruta>` se cargan como imágenes (visión); el resto es texto.
            let (texto, imagenes) = cargar_imagenes(&s.input.text());
            if texto.is_empty() && imagenes.is_empty() {
                return s;
            }
            let Some(agente) = s.agente_activo().cloned() else {
                return s; // sin agentes provistos no hay a quién preguntar
            };
            // Asegurá una conversación abierta (si no, abrí una nueva).
            if s.conversacion_activa().is_none() {
                let conv = Conversacion::nueva(&agente.id, s.reloj_ms);
                s.conv_activa = Some(conv.id.clone());
                s.conversaciones.insert(0, conv);
            }
            let ms = s.reloj_ms;
            if let Some(conv) = s.conversacion_activa_mut() {
                if imagenes.is_empty() {
                    conv.agregar_usuario(texto, ms);
                } else {
                    conv.agregar_usuario_con_imagenes(texto, imagenes, ms);
                }
                let snap = conv.clone();
                s.pendiente = Some(Peticion { conv: snap, agente });
                s.esperando = true;
                s.parcial = Some(String::new()); // burbuja viva en streaming
            }
            s.input.set_text("");
            s.scroll = f32::MAX; // saltá al final
        }
        Msg::NuevaConversacion => {
            // Abrí un lienzo limpio: la Conversacion concreta nace al primer
            // envío (así no se acumulan vacías).
            s.conv_activa = None;
            s.input.set_text("");
            s.scroll = 0.0;
        }
        Msg::AbrirConversacion(i) => {
            if let Some(c) = s.conversaciones.get(i) {
                s.conv_activa = Some(c.id.clone());
                s.scroll = f32::MAX;
            }
        }
        Msg::BorrarConversacion(i) => {
            if i < s.conversaciones.len() {
                let c = s.conversaciones.remove(i);
                if s.conv_activa.as_deref() == Some(c.id.as_str()) {
                    s.conv_activa = None;
                }
                s.borrar_conv_id = Some(c.id);
            }
        }
        Msg::RenombrarConversacion(i) => {
            if let Some(c) = s.conversaciones.get(i) {
                let mut input = TextInputState::new();
                input.set_text(&c.titulo);
                s.renombrando = Some((c.id.clone(), input));
            }
        }
        Msg::ConfirmarRenombre => {
            if let Some((id, input)) = s.renombrando.take() {
                let nuevo = input.text().trim().to_string();
                if !nuevo.is_empty() {
                    if let Some(c) = s.conversaciones.iter_mut().find(|c| c.id == id) {
                        c.titulo = nuevo;
                    }
                }
            }
        }
        Msg::SeleccionarAgente(i) => {
            if i < s.agentes.len() {
                s.agente_sel = i;
            }
        }
        Msg::Scroll(delta) => {
            s.scroll = (s.scroll + delta).max(0.0);
        }
        Msg::Token { conv_id, delta } => {
            // Sólo acumulá si es para la conversación en vuelo.
            if s.esperando && s.conv_activa.as_deref() == Some(conv_id.as_str()) {
                s.parcial.get_or_insert_with(String::new).push_str(&delta);
                s.scroll = f32::MAX;
            }
        }
        Msg::Respuesta { conv_id, bloques, ok, entrada, salida } => {
            s.esperando = false;
            s.parcial = None;
            let ms = s.reloj_ms;
            if let Some(conv) = s.conversaciones.iter_mut().find(|c| c.id == conv_id) {
                let bloques = if ok {
                    bloques
                } else {
                    vec![BloqueSalida::Error(
                        bloques
                            .into_iter()
                            .find_map(|b| match b {
                                BloqueSalida::Error(e) => Some(e),
                                BloqueSalida::Texto(t) => Some(t),
                                _ => None,
                            })
                            .unwrap_or_else(|| "el modelo no respondió".to_string()),
                    )]
                };
                conv.agregar_asistente(bloques, ms, Some(shuma_agente::Uso { entrada, salida }));
            }
            s.scroll = f32::MAX;
        }
        Msg::Aprobar { turno, bloque } => {
            if let Some(accion) = marcar_accion(&mut s, turno, bloque, EstadoAccion::Aprobada) {
                s.ejecucion = Some(accion);
            }
        }
        Msg::Rechazar { turno, bloque } => {
            marcar_accion(&mut s, turno, bloque, EstadoAccion::Rechazada);
        }
        Msg::NuevoAgente => {
            s.editor = Some(EditorAgente::nuevo());
        }
        Msg::EditarAgente => {
            if let Some(a) = s.agentes.get(s.agente_sel) {
                s.editor = Some(EditorAgente::desde(a));
            }
        }
        Msg::EditorFoco(c) => {
            if let Some(ed) = s.editor.as_mut() {
                ed.foco = c;
            }
        }
        Msg::EditorCiclarBackend => {
            if let Some(ed) = s.editor.as_mut() {
                ed.backend_idx = (ed.backend_idx + 1) % BACKENDS.len();
            }
        }
        Msg::EditorToggleControl => {
            if let Some(ed) = s.editor.as_mut() {
                ed.control = !ed.control;
            }
        }
        Msg::GuardarAgente => {
            if let Some(ed) = s.editor.take() {
                if ed.nombre.text().trim().is_empty() {
                    // Sin nombre no se guarda; reabrí el editor para corregir.
                    s.editor = Some(ed);
                } else {
                    let agente = ed.a_agente();
                    // Reflejá el cambio en memoria (el chasis además lo persiste).
                    if let Some(pos) = s.agentes.iter().position(|a| a.id == agente.id) {
                        s.agentes[pos] = agente.clone();
                    } else {
                        s.agentes.push(agente.clone());
                    }
                    s.persist_agente = Some(agente);
                }
            }
        }
        Msg::BorrarAgente => {
            if let Some(ed) = s.editor.take() {
                if let Some(id) = ed.id {
                    s.agentes.retain(|a| a.id != id);
                    if s.agente_sel >= s.agentes.len() {
                        s.agente_sel = s.agentes.len().saturating_sub(1);
                    }
                    s.borrar_agente_id = Some(id);
                }
            }
        }
        Msg::CancelarEditor => {
            s.editor = None;
        }
        Msg::ToggleMic => {
            // Durante el enrolamiento el micrófono lo usa la grabación: ignorar.
            if s.enrolando.is_some() {
                return s;
            }
            // Alterna encender/apagar; deja el intent para que el chasis arranque
            // o pare la captura real (rimay-voz-host).
            if s.escucha.activo() {
                s.escucha = EstadoEscucha::Apagado;
                s.mic_intent = Some(false);
            } else {
                s.escucha = EstadoEscucha::Esperando;
                s.mic_intent = Some(true);
            }
        }
        Msg::EscuchaCambio(e) => {
            s.escucha = e;
        }
        Msg::Dictado(t) => {
            // El dictado se inserta en el input; NO se envía solo (el usuario
            // revisa y manda con Enter / el botón), salvo que diga el llamado de
            // envío — eso lo decide quien dispatcha, no acá.
            if !t.is_empty() {
                if !s.input.is_empty() && !s.input.text().ends_with(' ') {
                    s.input.push_str(" ");
                }
                s.input.push_str(&t);
                s.focused = true;
            }
        }
        Msg::EnrolarWake => {
            // No enrolar mientras se escucha (mutuamente excluyente).
            if s.enrolando.is_none() && !s.escucha.activo() {
                s.enrolando = Some(0);
                s.enrol_intent = Some(true);
            }
        }
        Msg::EnrolarCapturado => s.enrol_capturado(),
        Msg::EnrolarHecho => s.enrol_terminado(),
        Msg::EnrolarCancelar => {
            if s.enrolando.is_some() {
                s.enrolando = None;
                s.enrol_intent = Some(false);
            }
        }
    }
    s
}

/// Cambia el estado de la acción en `(turno, bloque)` de la conversación activa.
/// Devuelve la acción (clonada) si la encontró y era una acción.
fn marcar_accion(
    s: &mut State,
    turno: usize,
    bloque: usize,
    nuevo: EstadoAccion,
) -> Option<shuma_agente::AccionPropuesta> {
    let conv = s.conversacion_activa_mut()?;
    let b = conv.turnos.get_mut(turno)?.bloques.get_mut(bloque)?;
    if let BloqueSalida::Accion(a) = b {
        a.estado = nuevo;
        Some(a.clone())
    } else {
        None
    }
}

/// Aportes al chasis: el panel no contribuye monitores ni shortcuts.
pub fn contributions(_state: &State) -> ModuleContributions {
    ModuleContributions::empty()
}

// ─── Vista ──────────────────────────────────────────────────────────────────

/// Pinta el panel. `lift` sube los `Msg` del módulo al `Msg` del chasis.
pub fn view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Send + Sync + 'static + Clone,
) -> View<HostMsg> {
    let sidebar = sidebar_view(state, theme, lift.clone());
    let main = panel_view(state, theme, lift);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![sidebar, main])
}

fn sidebar_view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Send + Sync + 'static + Clone,
) -> View<HostMsg> {
    let mut hijos: Vec<View<HostMsg>> = Vec::new();

    // Selector de agentes.
    hijos.push(rotulo("AGENTES", theme));
    for (i, ag) in state.agentes.iter().enumerate() {
        let sel = i == state.agente_sel;
        hijos.push(
            fila_seleccionable(&ag.nombre, sel, theme)
                .on_click(lift(Msg::SeleccionarAgente(i))),
        );
    }
    // Acciones de agentes: nuevo / editar el seleccionado.
    let bp = ButtonPalette::from_theme(theme);
    hijos.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
            gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
            padding: rect_xy(8.0, 2.0),
            ..Default::default()
        })
        .children(vec![
            View::new(Style { flex_grow: 1.0, ..Default::default() })
                .children(vec![button_view("+ agente", &bp, lift(Msg::NuevoAgente))]),
            View::new(Style { flex_grow: 1.0, ..Default::default() })
                .children(vec![button_view("editar", &bp, lift(Msg::EditarAgente))]),
        ]),
    );

    // Botón nueva conversación.
    hijos.push(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(36.0_f32) },
            padding: rect_xy(8.0, 4.0),
            ..Default::default()
        })
        .children(vec![button_view("+ nueva conversación", &bp, lift(Msg::NuevaConversacion))]),
    );

    // Lista de conversaciones: título clickeable + «×» para borrar.
    hijos.push(rotulo("CONVERSACIONES", theme));
    let renombrando_id = state.renombrando.as_ref().map(|(id, _)| id.as_str());
    for (i, c) in state.conversaciones.iter().enumerate() {
        let activa = state.conv_activa.as_deref() == Some(c.id.as_str());
        let titulo = if c.titulo.trim().is_empty() { "(sin título)" } else { &c.titulo };
        let bg = if activa { theme.bg_selected } else { theme.bg_panel_alt };
        let fg = if activa { theme.fg_text } else { theme.fg_muted };

        // Renombrando esta conversación: input en vez del título.
        if renombrando_id == Some(c.id.as_str()) {
            if let Some((_, input)) = &state.renombrando {
                let tp = TextInputPalette::from_theme(theme);
                hijos.push(
                    View::new(Style {
                        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
                        padding: rect_xy(6.0, 0.0),
                        ..Default::default()
                    })
                    .children(vec![text_input_view(
                        input,
                        "nuevo título…",
                        true,
                        &tp,
                        lift(Msg::ConfirmarRenombre),
                    )]),
                );
                continue;
            }
        }

        let fila = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(bg)
        .children(vec![
            // Título: toma el ancho y abre la conversación.
            View::new(Style {
                flex_grow: 1.0,
                size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
                padding: rect_xy(10.0, 0.0),
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(titulo, 12.5, fg, Alignment::Start)
            .on_click(lift(Msg::AbrirConversacion(i))),
            // «✎»: renombrar.
            View::new(Style {
                size: Size { width: length(20.0_f32), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned("✎", 12.0, theme.fg_muted, Alignment::Center)
            .on_click(lift(Msg::RenombrarConversacion(i))),
            // «×»: borra la conversación.
            View::new(Style {
                size: Size { width: length(20.0_f32), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned("×", 14.0, theme.fg_muted, Alignment::Center)
            .on_click(lift(Msg::BorrarConversacion(i))),
        ]);
        hijos.push(fila);
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(SIDEBAR_W), height: percent(1.0_f32) },
        padding: rect_xy(0.0, 8.0),
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(hijos)
}

/// Parámetros de pintura del indicador de voz por estado:
/// `(anillos, periodo_ms, intensidad)`. El color sale del theme (accent).
fn params_escucha(e: EstadoEscucha) -> (u32, f64, f32) {
    match e {
        EstadoEscucha::Apagado => (0, 1600.0, 0.0),
        EstadoEscucha::Esperando => (2, 1600.0, 0.45),
        EstadoEscucha::Oyendo => (3, 1000.0, 0.70),
        EstadoEscucha::Despierto => (3, 800.0, 0.90),
        EstadoEscucha::Dictando => (3, 600.0, 1.0),
    }
}

/// Botón de micrófono con **halo animado** según el estado de escucha — el
/// efecto «cava»: anillos que emanan como ondas de sonido, más rápidos e
/// intensos cuanto más activa la escucha — más el glifo del micrófono teñido por
/// estado. El click alterna encender/apagar. La animación avanza con `reloj_ms`
/// (el chasis lo refresca mientras escucha).
fn boton_mic<HostMsg: Clone + 'static>(
    escucha: EstadoEscucha,
    enrolando: bool,
    reloj_ms: u64,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Send + Sync + 'static + Clone,
) -> View<HostMsg> {
    let accent = theme.accent;
    let apagado = theme.fg_muted;
    // Enrolando: halo «grabando» en rojo cálido, anillos rápidos e intensos.
    let grabando = llimphi_ui::llimphi_raster::peniko::Color::from_rgb8(0xE0, 0x5A, 0x5A);
    View::new(Style {
        size: Size { width: length(34.0_f32), height: length(34.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .on_click(lift(Msg::ToggleMic))
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{
            Affine, BezPath, Circle, Line, Point, RoundedRect, Stroke,
        };
        use llimphi_ui::llimphi_raster::peniko::Fill;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let lado = rect.w.min(rect.h) as f64;
        // Enrolando manda sobre el estado de escucha (color rojo, máxima onda).
        let (color, anillos, periodo, intensidad) = if enrolando {
            (grabando, 3u32, 600.0_f64, 1.0_f32)
        } else {
            let (a, p, i) = params_escucha(escucha);
            (accent, a, p, i)
        };

        // Halo: anillos que emanan (las «ondas» del efecto cava).
        let r0 = lado * 0.20;
        let spread = lado * 0.42;
        for k in 0..anillos {
            let fase = (((reloj_ms as f64) / periodo) + (k as f64) / (anillos as f64)).fract();
            let r = r0 + fase * spread;
            let a = ((1.0 - fase as f32) * intensidad).clamp(0.0, 1.0);
            scene.stroke(
                &Stroke::new(1.6),
                Affine::IDENTITY,
                color.with_alpha(a),
                None,
                &Circle::new((cx, cy), r),
            );
        }

        // Glifo del micrófono, teñido por estado.
        let gc = if enrolando || escucha.activo() { color } else { apagado };
        let bw = lado * 0.11;
        let bh = lado * 0.20;
        let cap = RoundedRect::new(cx - bw, cy - bh - 2.0, cx + bw, cy + bh - 2.0, bw);
        scene.fill(Fill::NonZero, Affine::IDENTITY, gc, None, &cap);
        let aw = bw + 2.5;
        let ay = cy + bh - 2.0;
        let mut u = BezPath::new();
        u.move_to(Point::new(cx - aw, cy - 2.0));
        u.quad_to(Point::new(cx - aw, ay + 1.5), Point::new(cx, ay + 1.5));
        u.quad_to(Point::new(cx + aw, ay + 1.5), Point::new(cx + aw, cy - 2.0));
        scene.stroke(&Stroke::new(1.4), Affine::IDENTITY, gc, None, &u);
        scene.stroke(
            &Stroke::new(1.4),
            Affine::IDENTITY,
            gc,
            None,
            &Line::new(Point::new(cx, ay + 1.5), Point::new(cx, ay + 4.5)),
        );
        scene.stroke(
            &Stroke::new(1.4),
            Affine::IDENTITY,
            gc,
            None,
            &Line::new(Point::new(cx - 3.0, ay + 4.5), Point::new(cx + 3.0, ay + 4.5)),
        );
    })
}

fn panel_view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Send + Sync + 'static + Clone,
) -> View<HostMsg> {
    // Con el editor abierto ocupa todo el panel.
    if let Some(ed) = &state.editor {
        return editor_view(ed, theme, lift);
    }
    // Hilo de turnos.
    let mut turnos: Vec<View<HostMsg>> = Vec::new();
    let mut alto_total = 0.0_f32;
    if let Some(conv) = state.conversacion_activa() {
        for (ti, t) in conv.turnos.iter().enumerate() {
            let (v, h) = turno_view(ti, t, theme, lift.clone());
            alto_total += h + 10.0;
            turnos.push(v);
        }
    } else {
        turnos.push(
            View::new(Style {
                padding: rect_xy(16.0, 16.0),
                ..Default::default()
            })
            .text(
                "Elegí un agente y escribí abajo para empezar una conversación.",
                13.0,
                theme.fg_muted,
            ),
        );
        alto_total = 60.0;
    }
    if state.esperando {
        // Burbuja viva: el texto que está llegando en streaming, o «…pensando»
        // mientras no llegó el primer token.
        let parcial = state.parcial.as_deref().unwrap_or("");
        if parcial.is_empty() {
            turnos.push(
                View::new(Style { padding: rect_xy(16.0, 6.0), ..Default::default() })
                    .text("…pensando", 13.0, theme.fg_muted),
            );
            alto_total += 30.0;
        } else {
            turnos.push(
                View::new(Style {
                    flex_direction: FlexDirection::Column,
                    size: Size { width: percent(1.0_f32), height: Dimension::auto() },
                    gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
                    padding: rect_xy(12.0, 8.0),
                    ..Default::default()
                })
                .fill(theme.bg_panel_alt)
                .children(vec![
                    View::new(Style { size: Size { width: percent(1.0_f32), height: length(16.0_f32) }, ..Default::default() })
                        .text("IA", 11.0, theme.fg_muted),
                    View::new(Style { size: Size { width: percent(1.0_f32), height: Dimension::auto() }, ..Default::default() })
                        .text(format!("{parcial}▌"), 13.0, theme.fg_text),
                ]),
            );
            alto_total += estimar_alto_texto(parcial, 13.0) + 30.0;
        }
    }

    let contenido = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
        padding: rect_xy(16.0, 12.0),
        ..Default::default()
    })
    .children(turnos);

    let sp = ScrollPalette::from_theme(theme);
    let lift_scroll = lift.clone();
    let hilo = scroll_y(
        state.scroll.min(alto_total),
        alto_total,
        state.vista_alto,
        contenido,
        move |d| lift_scroll(Msg::Scroll(-d)),
        &sp,
    );

    // Barra de input.
    let tp = TextInputPalette::from_theme(theme);
    // Glow del input mientras escucha: borde redondeado que respira (varias
    // pasadas con alpha decreciente para difuminar). Apagado = sin pintura.
    let escucha = state.escucha;
    let reloj = state.reloj_ms;
    let accent = theme.accent;
    let input = View::new(Style {
        flex_grow: 1.0,
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, RoundedRect, Stroke};
        if !escucha.activo() || rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let (_, periodo, intensidad) = params_escucha(escucha);
        // Respiración 0..1 (seno) sincronizada con el periodo del estado.
        let t = (reloj as f64) / periodo * std::f64::consts::TAU;
        let respira = 0.5 + 0.5 * (t.sin() as f32);
        let base = intensidad * (0.35 + 0.65 * respira);
        let (x0, y0) = (rect.x as f64 + 1.0, rect.y as f64 + 1.0);
        let (x1, y1) = ((rect.x + rect.w) as f64 - 1.0, (rect.y + rect.h) as f64 - 1.0);
        // Tres pasadas hacia afuera con alpha decreciente → halo difuso.
        for (i, ancho) in [1.4_f64, 2.6, 3.8].into_iter().enumerate() {
            let d = i as f64 * 1.3;
            let a = base * (1.0 - i as f32 * 0.32);
            let rr = RoundedRect::new(x0 - d, y0 - d, x1 + d, y1 + d, 8.0 + d);
            scene.stroke(
                &Stroke::new(ancho),
                Affine::IDENTITY,
                accent.with_alpha(a.clamp(0.0, 1.0)),
                None,
                &rr,
            );
        }
    });
    // Enrolando: el placeholder guía la grabación de «shuma».
    let placeholder = match state.enrolando {
        Some(n) => format!("🎙 Grabá «shuma» — {}/{} (cancelar →)", n, ENROL_OBJETIVO),
        None => "Escribí tu mensaje…  (Enter envía · img:/ruta para adjuntar · 🎙 dicta)".into(),
    };
    let input = input.children(vec![text_input_view(
        &state.input,
        &placeholder,
        state.focused,
        &tp,
        lift(Msg::FocusInput),
    )]);
    // Botón de micrófono con el indicador animado (escucha o enrolamiento).
    let mic = boton_mic(
        state.escucha,
        state.enrolando.is_some(),
        state.reloj_ms,
        theme,
        lift.clone(),
    );
    let bp = ButtonPalette::from_theme(theme);
    // En idle, un acceso a enrolar el wake-word; enrolando, a cancelar; si no, Enviar.
    let accion = if state.enrolando.is_some() {
        View::new(Style {
            size: Size { width: length(96.0_f32), height: Dimension::auto() },
            ..Default::default()
        })
        .children(vec![button_view("Cancelar", &bp, lift(Msg::EnrolarCancelar))])
    } else if !state.escucha.activo() {
        // Idle: ofrecé enrolar (o re-enrolar) la palabra de llamada.
        let etq = if state.wake_listo { "re-enrolar" } else { "enrolar voz" };
        View::new(Style {
            flex_direction: FlexDirection::Row,
            gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .children(vec![
            View::new(Style {
                size: Size { width: length(96.0_f32), height: Dimension::auto() },
                ..Default::default()
            })
            .children(vec![button_view(etq, &bp, lift(Msg::EnrolarWake))]),
            View::new(Style {
                size: Size { width: length(96.0_f32), height: Dimension::auto() },
                ..Default::default()
            })
            .children(vec![button_view("Enviar", &bp, lift(Msg::Enviar))]),
        ])
    } else {
        View::new(Style {
            size: Size { width: length(96.0_f32), height: Dimension::auto() },
            ..Default::default()
        })
        .children(vec![button_view("Enviar", &bp, lift(Msg::Enviar))])
    };
    let enviar = accion;

    let barra = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        padding: rect_xy(12.0, 6.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![input, mic, enviar]);

    let hilo_wrap = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: length(state.vista_alto) },
        ..Default::default()
    })
    .children(vec![hilo]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![hilo_wrap, barra])
}

/// Formulario de alta/edición de un agente.
fn editor_view<HostMsg: Clone + 'static>(
    ed: &EditorAgente,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Send + Sync + 'static + Clone,
) -> View<HostMsg> {
    let tp = TextInputPalette::from_theme(theme);
    let bp = ButtonPalette::from_theme(theme);

    let titulo = if ed.id.is_some() { "Editar agente" } else { "Nuevo agente" };

    // Campo de texto etiquetado.
    let campo = |etq: &str, st: &TextInputState, foco: bool, c: Campo| {
        let lift = lift.clone();
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: Dimension::auto() },
            gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
            ..Default::default()
        })
        .children(vec![
            View::new(Style { size: Size { width: percent(1.0_f32), height: length(14.0_f32) }, ..Default::default() })
                .text(etq, 10.0, theme.fg_muted),
            text_input_view(st, "", foco, &tp, lift(Msg::EditorFoco(c))),
        ])
    };

    let backend_lbl = {
        let b = BACKENDS[ed.backend_idx];
        let nombre = if b.is_empty() { "(global del SO)" } else { b };
        format!("backend: {nombre}")
    };
    let control_lbl = format!("control: {}", if ed.control { "sí" } else { "no" });

    let mut hijos = vec![
        View::new(Style { size: Size { width: percent(1.0_f32), height: length(22.0_f32) }, ..Default::default() })
            .text(titulo, 14.0, theme.fg_text),
        campo("Nombre", &ed.nombre, ed.foco == Campo::Nombre, Campo::Nombre),
        campo("Modelo (vacío = default del backend)", &ed.modelo, ed.foco == Campo::Modelo, Campo::Modelo),
        campo("Persona / system prompt", &ed.persona, ed.foco == Campo::Persona, Campo::Persona),
        // Backend (cicla) + control (toggle).
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
            gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .children(vec![
            View::new(Style { flex_grow: 2.0, ..Default::default() })
                .children(vec![button_view(backend_lbl, &bp, lift(Msg::EditorCiclarBackend))]),
            View::new(Style { flex_grow: 1.0, ..Default::default() })
                .children(vec![button_view(control_lbl, &bp, lift(Msg::EditorToggleControl))]),
        ]),
    ];

    // Botonera: guardar / cancelar (+ borrar si edita).
    let mut botones = vec![
        View::new(Style { flex_grow: 1.0, ..Default::default() })
            .children(vec![button_view("guardar", &bp, lift(Msg::GuardarAgente))]),
        View::new(Style { flex_grow: 1.0, ..Default::default() })
            .children(vec![button_view("cancelar", &bp, lift(Msg::CancelarEditor))]),
    ];
    if ed.id.is_some() {
        botones.push(
            View::new(Style { flex_grow: 1.0, ..Default::default() })
                .children(vec![button_view("borrar", &bp, lift(Msg::BorrarAgente))]),
        );
    }
    hijos.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
            gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .children(botones),
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
        padding: rect_xy(16.0, 14.0),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(hijos)
}

/// Pinta un turno; devuelve la vista y una estimación de su alto (px) para el
/// scroll (no hay medición exacta de texto en tiempo de view).
fn turno_view<HostMsg: Clone + 'static>(
    turno_idx: usize,
    turno: &shuma_agente::Turno,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Send + Sync + 'static + Clone,
) -> (View<HostMsg>, f32) {
    use shuma_agente::Rol;
    let es_usuario = turno.rol == Rol::Usuario;
    let prefijo = if es_usuario { "Vos" } else { "IA" };
    let color_pref = if es_usuario { theme.accent } else { theme.fg_muted };

    let mut hijos: Vec<View<HostMsg>> = vec![View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text(prefijo, 11.0, color_pref)];

    let mut alto = 20.0_f32;
    for (bi, b) in turno.bloques.iter().enumerate() {
        let (v, h) = bloque_view(turno_idx, bi, b, theme, lift.clone());
        alto += h;
        hijos.push(v);
    }

    // Conteo de tokens del turno (paridad con Claude CLI), si lo hay.
    if let Some(u) = turno.uso.filter(|u| u.hay()) {
        hijos.push(
            View::new(Style { size: Size { width: percent(1.0_f32), height: length(14.0_f32) }, ..Default::default() })
                .text(format!("↑{} ↓{} tokens", u.entrada, u.salida), 10.0, theme.fg_muted),
        );
        alto += 16.0;
    }

    let v = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        padding: rect_xy(12.0, 8.0),
        ..Default::default()
    })
    .fill(if es_usuario { theme.bg_panel } else { theme.bg_panel_alt })
    .children(hijos);
    (v, alto)
}

fn bloque_view<HostMsg: Clone + 'static>(
    turno: usize,
    bloque: usize,
    b: &BloqueSalida,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Send + Sync + 'static + Clone,
) -> (View<HostMsg>, f32) {
    match b {
        BloqueSalida::Texto(t) => {
            let alto = estimar_alto_texto(t, 13.0);
            (
                View::new(Style { size: Size { width: percent(1.0_f32), height: Dimension::auto() }, ..Default::default() })
                    .text(t, 13.0, theme.fg_text),
                alto,
            )
        }
        BloqueSalida::Codigo { lenguaje, codigo } => {
            let etiqueta = lenguaje.clone().unwrap_or_default();
            let alto = estimar_alto_texto(codigo, 12.5) + 16.0;
            let v = View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size { width: percent(1.0_f32), height: Dimension::auto() },
                padding: rect_xy(10.0, 8.0),
                ..Default::default()
            })
            .fill(theme.bg_app)
            .children(vec![
                View::new(Style { size: Size { width: percent(1.0_f32), height: length(12.0_f32) }, ..Default::default() })
                    .text(etiqueta, 10.0, theme.fg_muted),
                View::new(Style { size: Size { width: percent(1.0_f32), height: Dimension::auto() }, ..Default::default() })
                    .text(codigo, 12.5, theme.fg_text),
            ]);
            (v, alto)
        }
        BloqueSalida::Accion(a) => (accion_view(turno, bloque, a, theme, lift), 70.0),
        BloqueSalida::Imagen { data_base64, .. } => {
            let alto = 180.0;
            match decodificar_imagen(data_base64) {
                Some(img) => (
                    View::new(Style {
                        size: Size { width: length(240.0_f32), height: length(alto) },
                        ..Default::default()
                    })
                    .image(img),
                    alto + 6.0,
                ),
                None => (
                    View::new(Style { size: Size { width: percent(1.0_f32), height: length(18.0_f32) }, ..Default::default() })
                        .text("🖼 imagen (no se pudo mostrar)", 12.0, theme.fg_muted),
                    22.0,
                ),
            }
        }
        BloqueSalida::Error(e) => (
            View::new(Style { size: Size { width: percent(1.0_f32), height: Dimension::auto() }, ..Default::default() })
                .text(format!("⚠ {e}"), 12.5, theme.fg_destructive),
            estimar_alto_texto(e, 12.5),
        ),
    }
}

/// Tarjeta de acción de control: línea de comando + peligro + aprobar/rechazar.
fn accion_view<HostMsg: Clone + 'static>(
    turno: usize,
    bloque: usize,
    a: &shuma_agente::AccionPropuesta,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Send + Sync + 'static + Clone,
) -> View<HostMsg> {
    let acento = match a.peligro {
        Peligro::Seguro => theme.accent,
        Peligro::Reversible => theme.accent,
        Peligro::Disruptivo => theme.fg_destructive,
    };
    let cabecera = format!("⚡ {} · [{}]", a.id, a.peligro.etiqueta());

    let mut hijos = vec![
        View::new(Style { size: Size { width: percent(1.0_f32), height: length(16.0_f32) }, ..Default::default() })
            .text(cabecera, 11.0, acento),
        View::new(Style { size: Size { width: percent(1.0_f32), height: Dimension::auto() }, ..Default::default() })
            .text(&a.linea_comando, 12.5, theme.fg_text),
    ];

    // Botonera según estado.
    let bp = ButtonPalette::from_theme(theme);
    let fila = match a.estado {
        EstadoAccion::Propuesta => {
            let aprobar = View::new(Style { size: Size { width: length(96.0_f32), height: Dimension::auto() }, ..Default::default() })
                .children(vec![button_view("aprobar", &bp, lift.clone()(Msg::Aprobar { turno, bloque }))]);
            let rechazar = View::new(Style { size: Size { width: length(96.0_f32), height: Dimension::auto() }, ..Default::default() })
                .children(vec![button_view("rechazar", &bp, lift(Msg::Rechazar { turno, bloque }))]);
            View::new(Style {
                flex_direction: FlexDirection::Row,
                gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
                size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
                ..Default::default()
            })
            .children(vec![aprobar, rechazar])
        }
        estado => {
            let (txt, col) = match estado {
                EstadoAccion::Aprobada => ("✓ aprobada — ejecutando…", theme.accent),
                EstadoAccion::Ejecutada => ("✓ ejecutada", theme.accent),
                EstadoAccion::Rechazada => ("✗ rechazada", theme.fg_muted),
                EstadoAccion::Fallida => ("⚠ falló", theme.fg_destructive),
                EstadoAccion::Propuesta => ("", theme.fg_muted),
            };
            View::new(Style { size: Size { width: percent(1.0_f32), height: length(20.0_f32) }, ..Default::default() })
                .text(txt, 11.0, col)
        }
    };
    hijos.push(fila);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        padding: rect_xy(10.0, 8.0),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(hijos)
}

// ─── Helpers de vista ───────────────────────────────────────────────────────

fn rotulo<HostMsg: Clone + 'static>(txt: &str, theme: &Theme) -> View<HostMsg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        padding: rect_xy(10.0, 4.0),
        ..Default::default()
    })
    .text_aligned(txt, 10.0, theme.fg_muted, Alignment::Start)
}

fn fila_seleccionable<HostMsg: Clone + 'static>(
    texto: &str,
    sel: bool,
    theme: &Theme,
) -> View<HostMsg> {
    let bg = if sel { theme.bg_selected } else { theme.bg_panel_alt };
    let fg = if sel { theme.fg_text } else { theme.fg_muted };
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        padding: rect_xy(10.0, 0.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .text_aligned(texto, 12.5, fg, Alignment::Start)
}

fn rect_xy(x: f32, y: f32) -> Rect<LengthPercentage> {
    Rect { left: length(x), right: length(x), top: length(y), bottom: length(y) }
}

/// Tope de tamaño de imagen adjunta (5 MiB) — evita pegar archivos enormes.
const IMG_MAX_BYTES: usize = 5 * 1024 * 1024;

/// Separa el input en texto y adjuntos: las líneas `img:<ruta>` se leen del disco
/// y se devuelven como `(media_type, base64)`; el resto es el texto del mensaje.
/// Las que no se pueden leer se descartan en silencio (el mensaje igual sale).
fn cargar_imagenes(crudo: &str) -> (String, Vec<(String, String)>) {
    use base64::Engine as _;
    let mut texto: Vec<&str> = Vec::new();
    let mut imgs: Vec<(String, String)> = Vec::new();
    for linea in crudo.lines() {
        let t = linea.trim();
        if let Some(ruta) = t.strip_prefix("img:") {
            let ruta = ruta.trim();
            if let Ok(bytes) = std::fs::read(ruta) {
                if !bytes.is_empty() && bytes.len() <= IMG_MAX_BYTES {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    imgs.push((media_type_de(ruta), b64));
                }
            }
        } else {
            texto.push(linea);
        }
    }
    (texto.join("\n").trim().to_string(), imgs)
}

/// Adivina el `media_type` por la extensión del archivo (default PNG).
fn media_type_de(ruta: &str) -> String {
    let l = ruta.to_lowercase();
    if l.ends_with(".jpg") || l.ends_with(".jpeg") {
        "image/jpeg"
    } else if l.ends_with(".webp") {
        "image/webp"
    } else if l.ends_with(".gif") {
        "image/gif"
    } else {
        "image/png"
    }
    .to_string()
}

/// Decodifica base64 → bytes → imagen lista para `View::image`. `None` si falla.
fn decodificar_imagen(data_base64: &str) -> Option<llimphi_image::Image> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data_base64).ok()?;
    llimphi_image::decode_bytes(&bytes).ok()
}

/// Estimación grosera del alto de un texto (px): cuenta líneas reales y suma un
/// poco por wrap. No es exacto — sólo dimensiona el scroll.
fn estimar_alto_texto(t: &str, size: f32) -> f32 {
    let alto_linea = size * 1.4;
    let lineas: f32 = t
        .lines()
        .map(|l| (l.chars().count() as f32 / 64.0).ceil().max(1.0))
        .sum();
    (lineas.max(1.0)) * alto_linea + 4.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn estado_con_agente() -> State {
        let mut s = State::new();
        s.set_agentes(vec![Agente::nuevo("Asistente"), Agente::nuevo("Control").con_control()]);
        s.fijar_reloj(1000);
        s
    }

    #[test]
    fn toggle_mic_enciende_y_apaga_con_intent() {
        let mut s = State::new();
        assert_eq!(s.escucha(), EstadoEscucha::Apagado);
        // Encender.
        s = update(s, Msg::ToggleMic);
        assert_eq!(s.escucha(), EstadoEscucha::Esperando);
        assert_eq!(s.tomar_mic_intent(), Some(true));
        assert_eq!(s.tomar_mic_intent(), None); // se consume una sola vez
        // Apagar (desde cualquier estado activo).
        s.fijar_escucha(EstadoEscucha::Dictando);
        s = update(s, Msg::ToggleMic);
        assert_eq!(s.escucha(), EstadoEscucha::Apagado);
        assert_eq!(s.tomar_mic_intent(), Some(false));
    }

    #[test]
    fn escucha_cambio_fija_el_estado() {
        let mut s = State::new();
        s = update(s, Msg::EscuchaCambio(EstadoEscucha::Despierto));
        assert_eq!(s.escucha(), EstadoEscucha::Despierto);
        assert!(s.escucha().activo());
    }

    #[test]
    fn dictado_inserta_en_el_input_con_espacio() {
        let mut s = State::new();
        s = update(s, Msg::Dictado("hola".into()));
        assert_eq!(s.input.text(), "hola");
        assert!(s.focused);
        // Un segundo dictado se separa con un espacio.
        s = update(s, Msg::Dictado("mundo".into()));
        assert_eq!(s.input.text(), "hola mundo");
        // Vacío no cambia nada.
        s = update(s, Msg::Dictado(String::new()));
        assert_eq!(s.input.text(), "hola mundo");
    }

    #[test]
    fn enrolar_flujo_completo() {
        let mut s = State::new();
        // Arrancar: deja intent y contador en 0.
        s = update(s, Msg::EnrolarWake);
        assert_eq!(s.enrolando(), Some(0));
        assert_eq!(s.tomar_enrol_intent(), Some(true));
        // El chasis reporta las 3 capturas.
        for n in 1..=ENROL_OBJETIVO {
            s = update(s, Msg::EnrolarCapturado);
            assert_eq!(s.enrolando(), Some(n));
        }
        // Capturas de más no pasan del objetivo.
        s = update(s, Msg::EnrolarCapturado);
        assert_eq!(s.enrolando(), Some(ENROL_OBJETIVO));
        // Terminar: sale de enrolando y marca wake listo.
        s = update(s, Msg::EnrolarHecho);
        assert_eq!(s.enrolando(), None);
        assert!(s.wake_listo);
    }

    #[test]
    fn enrolar_cancelar_deja_intent_de_corte() {
        let mut s = State::new();
        s = update(s, Msg::EnrolarWake);
        let _ = s.tomar_enrol_intent();
        s = update(s, Msg::EnrolarCancelar);
        assert_eq!(s.enrolando(), None);
        assert_eq!(s.tomar_enrol_intent(), Some(false));
    }

    #[test]
    fn no_se_enrola_mientras_escucha_ni_se_escucha_enrolando() {
        let mut s = State::new();
        // Escuchando → EnrolarWake no arranca.
        s.fijar_escucha(EstadoEscucha::Despierto);
        s = update(s, Msg::EnrolarWake);
        assert_eq!(s.enrolando(), None);
        // Enrolando → ToggleMic no enciende el micrófono.
        let mut s = State::new();
        s = update(s, Msg::EnrolarWake);
        let _ = s.tomar_enrol_intent();
        s = update(s, Msg::ToggleMic);
        assert_eq!(s.escucha(), EstadoEscucha::Apagado);
        assert_eq!(s.tomar_mic_intent(), None);
    }

    #[test]
    fn enviar_crea_conversacion_y_pendiente() {
        let mut s = estado_con_agente();
        s.input.set_text("hola");
        s = update(s, Msg::Enviar);
        assert_eq!(s.conversaciones.len(), 1);
        assert!(s.esperando);
        let req = s.take_request().expect("debe haber petición");
        assert_eq!(req.conv.turnos.len(), 1);
        assert_eq!(req.agente.nombre, "Asistente");
        assert_eq!(s.input.text(), ""); // input limpio
    }

    #[test]
    fn enviar_vacio_no_hace_nada() {
        let mut s = estado_con_agente();
        s.input.set_text("   ");
        s = update(s, Msg::Enviar);
        assert!(s.conversaciones.is_empty());
        assert!(!s.esperando);
    }

    #[test]
    fn respuesta_agrega_turno_asistente() {
        let mut s = estado_con_agente();
        s.input.set_text("¿hora?");
        s = update(s, Msg::Enviar);
        let id = s.conversacion_activa().unwrap().id.clone();
        s = update(
            s,
            Msg::Respuesta {
                conv_id: id,
                bloques: vec![BloqueSalida::Texto("son las 3".into())],
                ok: true,
                entrada: 12,
                salida: 5,
            },
        );
        assert!(!s.esperando);
        let conv = s.conversacion_activa().unwrap();
        assert_eq!(conv.turnos.len(), 2);
        assert_eq!(conv.turnos[1].rol, shuma_agente::Rol::Asistente);
    }

    #[test]
    fn seleccionar_agente_de_control_y_responder_con_accion() {
        let mut s = estado_con_agente();
        s = update(s, Msg::SeleccionarAgente(1)); // Control
        s.input.set_text("subí el brillo");
        s = update(s, Msg::Enviar);
        let req = s.take_request().unwrap();
        assert!(req.agente.capacidades.control);
    }

    #[test]
    fn aprobar_accion_deja_ejecucion_y_marca_estado() {
        let mut s = estado_con_agente();
        s.input.set_text("x");
        s = update(s, Msg::Enviar);
        let id = s.conversacion_activa().unwrap().id.clone();
        let accion = shuma_agente::AccionPropuesta {
            id: "sistema.brillo".into(),
            linea_comando: "brightnessctl set 80".into(),
            peligro: Peligro::Reversible,
            estado: EstadoAccion::Propuesta,
        };
        s = update(
            s,
            Msg::Respuesta { conv_id: id, bloques: vec![BloqueSalida::Accion(accion)], ok: true, entrada: 0, salida: 0 },
        );
        // turno 1 = asistente, bloque 0 = acción.
        s = update(s, Msg::Aprobar { turno: 1, bloque: 0 });
        let ej = s.take_ejecucion().expect("acción aprobada se ejecuta");
        assert_eq!(ej.id, "sistema.brillo");
        let conv = s.conversacion_activa().unwrap();
        match &conv.turnos[1].bloques[0] {
            BloqueSalida::Accion(a) => assert_eq!(a.estado, EstadoAccion::Aprobada),
            _ => panic!("esperaba acción"),
        }
    }

    #[test]
    fn alta_de_agente_persiste_y_aparece() {
        let mut s = estado_con_agente();
        let antes = s.agentes.len();
        s = update(s, Msg::NuevoAgente);
        assert!(s.editor.is_some());
        // Escribí un nombre en el campo enfocado (Nombre).
        if let Some(ed) = s.editor.as_mut() {
            ed.nombre.set_text("Traductor");
        }
        s = update(s, Msg::EditorCiclarBackend); // claude-cli → anthropic
        s = update(s, Msg::GuardarAgente);
        assert!(s.editor.is_none());
        assert_eq!(s.agentes.len(), antes + 1);
        let ag = s.take_persist_agente().expect("debe pedir persistir");
        assert_eq!(ag.nombre, "Traductor");
        assert_eq!(ag.backend.backend, "anthropic");
    }

    #[test]
    fn alta_sin_nombre_no_guarda() {
        let mut s = estado_con_agente();
        s = update(s, Msg::NuevoAgente);
        s = update(s, Msg::GuardarAgente);
        assert!(s.editor.is_some()); // reabre para corregir
        assert!(s.persist_agente.is_none());
    }

    #[test]
    fn editar_agente_conserva_id() {
        let mut s = estado_con_agente();
        let id0 = s.agentes[0].id.clone();
        s = update(s, Msg::SeleccionarAgente(0));
        s = update(s, Msg::EditarAgente);
        if let Some(ed) = s.editor.as_mut() {
            ed.nombre.set_text("Asistente Pro");
        }
        s = update(s, Msg::GuardarAgente);
        let ag = s.take_persist_agente().unwrap();
        assert_eq!(ag.id, id0); // mismo id (edición, no alta)
        assert_eq!(ag.nombre, "Asistente Pro");
        assert_eq!(s.agentes[0].nombre, "Asistente Pro");
    }

    #[test]
    fn borrar_agente_lo_saca_y_pide_borrado() {
        let mut s = estado_con_agente();
        let id0 = s.agentes[0].id.clone();
        let antes = s.agentes.len();
        s = update(s, Msg::SeleccionarAgente(0));
        s = update(s, Msg::EditarAgente);
        s = update(s, Msg::BorrarAgente);
        assert_eq!(s.agentes.len(), antes - 1);
        assert_eq!(s.take_borrar_agente().as_deref(), Some(id0.as_str()));
    }

    #[test]
    fn cargar_imagenes_parsea_lineas_img() {
        let ruta = std::env::temp_dir().join("shuma-agente-test-img.png");
        std::fs::write(&ruta, b"\x89PNG fake bytes").unwrap();
        let entrada = format!("mirá esto\nimg:{}\ny decime", ruta.display());
        let (texto, imgs) = cargar_imagenes(&entrada);
        assert_eq!(texto, "mirá esto\ny decime");
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].0, "image/png");
        assert!(!imgs[0].1.is_empty()); // base64 no vacío
        // Una ruta inexistente se descarta sin romper.
        let (t2, i2) = cargar_imagenes("texto\nimg:/no/existe.png");
        assert_eq!(t2, "texto");
        assert!(i2.is_empty());
        let _ = std::fs::remove_file(&ruta);
    }

    #[test]
    fn borrar_conversacion_la_saca_y_pide_borrado() {
        let mut s = estado_con_agente();
        s.input.set_text("hola");
        s = update(s, Msg::Enviar);
        let id = s.conversacion_activa().unwrap().id.clone();
        assert_eq!(s.conversaciones.len(), 1);
        s = update(s, Msg::BorrarConversacion(0));
        assert!(s.conversaciones.is_empty());
        assert!(s.conv_activa.is_none()); // era la activa
        assert_eq!(s.take_borrar_conversacion().as_deref(), Some(id.as_str()));
    }

    #[test]
    fn renombrar_conversacion_cambia_el_titulo() {
        let mut s = estado_con_agente();
        s.input.set_text("hola mundo");
        s = update(s, Msg::Enviar);
        s = update(s, Msg::RenombrarConversacion(0));
        assert!(s.renombrando.is_some());
        if let Some((_, input)) = s.renombrando.as_mut() {
            input.set_text("Mi charla");
        }
        s = update(s, Msg::ConfirmarRenombre);
        assert!(s.renombrando.is_none());
        assert_eq!(s.conversaciones[0].titulo, "Mi charla");
    }

    #[test]
    fn nueva_conversacion_deselecciona() {
        let mut s = estado_con_agente();
        s.input.set_text("hola");
        s = update(s, Msg::Enviar);
        assert!(s.conv_activa.is_some());
        s = update(s, Msg::NuevaConversacion);
        assert!(s.conv_activa.is_none());
    }
}
