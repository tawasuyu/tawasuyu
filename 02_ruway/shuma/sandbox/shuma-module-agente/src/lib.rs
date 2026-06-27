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
    prelude::{length, percent, AlignItems, Dimension, FlexDirection, LengthPercentage, Size, Style},
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
}

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
}

/// Transición pura del estado.
pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::Key(ev) => {
            if ev.state != KeyState::Pressed {
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
            let texto = s.input.text().trim().to_string();
            if texto.is_empty() || s.esperando {
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
                conv.agregar_usuario(texto, ms);
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
        size: Size { width: percent(1.0), height: percent(1.0) },
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
            size: Size { width: percent(1.0), height: length(30.0) },
            gap: Size { width: length(6.0), height: length(0.0) },
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
            size: Size { width: percent(1.0), height: length(36.0) },
            padding: rect_xy(8.0, 4.0),
            ..Default::default()
        })
        .children(vec![button_view("+ nueva conversación", &bp, lift(Msg::NuevaConversacion))]),
    );

    // Lista de conversaciones: título clickeable + «×» para borrar.
    hijos.push(rotulo("CONVERSACIONES", theme));
    for (i, c) in state.conversaciones.iter().enumerate() {
        let activa = state.conv_activa.as_deref() == Some(c.id.as_str());
        let titulo = if c.titulo.trim().is_empty() { "(sin título)" } else { &c.titulo };
        let bg = if activa { theme.bg_selected } else { theme.bg_panel_alt };
        let fg = if activa { theme.fg_text } else { theme.fg_muted };
        let fila = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0), height: length(26.0) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(bg)
        .children(vec![
            // Título: toma el ancho y abre la conversación.
            View::new(Style {
                flex_grow: 1.0,
                size: Size { width: Dimension::auto(), height: percent(1.0) },
                padding: rect_xy(10.0, 0.0),
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(titulo, 12.5, fg, Alignment::Start)
            .on_click(lift(Msg::AbrirConversacion(i))),
            // «×»: borra la conversación.
            View::new(Style {
                size: Size { width: length(22.0), height: percent(1.0) },
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
        size: Size { width: length(SIDEBAR_W), height: percent(1.0) },
        padding: rect_xy(0.0, 8.0),
        gap: Size { width: length(0.0), height: length(2.0) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(hijos)
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
                    size: Size { width: percent(1.0), height: Dimension::auto() },
                    gap: Size { width: length(0.0), height: length(4.0) },
                    padding: rect_xy(12.0, 8.0),
                    ..Default::default()
                })
                .fill(theme.bg_panel_alt)
                .children(vec![
                    View::new(Style { size: Size { width: percent(1.0), height: length(16.0) }, ..Default::default() })
                        .text("IA", 11.0, theme.fg_muted),
                    View::new(Style { size: Size { width: percent(1.0), height: Dimension::auto() }, ..Default::default() })
                        .text(format!("{parcial}▌"), 13.0, theme.fg_text),
                ]),
            );
            alto_total += estimar_alto_texto(parcial, 13.0) + 30.0;
        }
    }

    let contenido = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0), height: Dimension::auto() },
        gap: Size { width: length(0.0), height: length(10.0) },
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
    let input = View::new(Style {
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![text_input_view(
        &state.input,
        "Escribí tu mensaje…  (Enter envía)",
        state.focused,
        &tp,
        lift(Msg::FocusInput),
    )]);
    let bp = ButtonPalette::from_theme(theme);
    let enviar = View::new(Style {
        size: Size { width: length(96.0), height: Dimension::auto() },
        ..Default::default()
    })
    .children(vec![button_view("Enviar", &bp, lift(Msg::Enviar))]);

    let barra = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0), height: length(40.0) },
        gap: Size { width: length(8.0), height: length(0.0) },
        padding: rect_xy(12.0, 6.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![input, enviar]);

    let hilo_wrap = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0), height: length(state.vista_alto) },
        ..Default::default()
    })
    .children(vec![hilo]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: Dimension::auto(), height: percent(1.0) },
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
            size: Size { width: percent(1.0), height: Dimension::auto() },
            gap: Size { width: length(0.0), height: length(2.0) },
            ..Default::default()
        })
        .children(vec![
            View::new(Style { size: Size { width: percent(1.0), height: length(14.0) }, ..Default::default() })
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
        View::new(Style { size: Size { width: percent(1.0), height: length(22.0) }, ..Default::default() })
            .text(titulo, 14.0, theme.fg_text),
        campo("Nombre", &ed.nombre, ed.foco == Campo::Nombre, Campo::Nombre),
        campo("Modelo (vacío = default del backend)", &ed.modelo, ed.foco == Campo::Modelo, Campo::Modelo),
        campo("Persona / system prompt", &ed.persona, ed.foco == Campo::Persona, Campo::Persona),
        // Backend (cicla) + control (toggle).
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0), height: length(34.0) },
            gap: Size { width: length(8.0), height: length(0.0) },
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
            size: Size { width: percent(1.0), height: length(34.0) },
            gap: Size { width: length(8.0), height: length(0.0) },
            ..Default::default()
        })
        .children(botones),
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: Dimension::auto(), height: percent(1.0) },
        gap: Size { width: length(0.0), height: length(10.0) },
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
        size: Size { width: percent(1.0), height: length(16.0) },
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
            View::new(Style { size: Size { width: percent(1.0), height: length(14.0) }, ..Default::default() })
                .text(format!("↑{} ↓{} tokens", u.entrada, u.salida), 10.0, theme.fg_muted),
        );
        alto += 16.0;
    }

    let v = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0), height: Dimension::auto() },
        gap: Size { width: length(0.0), height: length(4.0) },
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
                View::new(Style { size: Size { width: percent(1.0), height: Dimension::auto() }, ..Default::default() })
                    .text(t, 13.0, theme.fg_text),
                alto,
            )
        }
        BloqueSalida::Codigo { lenguaje, codigo } => {
            let etiqueta = lenguaje.clone().unwrap_or_default();
            let alto = estimar_alto_texto(codigo, 12.5) + 16.0;
            let v = View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size { width: percent(1.0), height: Dimension::auto() },
                padding: rect_xy(10.0, 8.0),
                ..Default::default()
            })
            .fill(theme.bg_app)
            .children(vec![
                View::new(Style { size: Size { width: percent(1.0), height: length(12.0) }, ..Default::default() })
                    .text(etiqueta, 10.0, theme.fg_muted),
                View::new(Style { size: Size { width: percent(1.0), height: Dimension::auto() }, ..Default::default() })
                    .text(codigo, 12.5, theme.fg_text),
            ]);
            (v, alto)
        }
        BloqueSalida::Accion(a) => (accion_view(turno, bloque, a, theme, lift), 70.0),
        BloqueSalida::Error(e) => (
            View::new(Style { size: Size { width: percent(1.0), height: Dimension::auto() }, ..Default::default() })
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
        View::new(Style { size: Size { width: percent(1.0), height: length(16.0) }, ..Default::default() })
            .text(cabecera, 11.0, acento),
        View::new(Style { size: Size { width: percent(1.0), height: Dimension::auto() }, ..Default::default() })
            .text(&a.linea_comando, 12.5, theme.fg_text),
    ];

    // Botonera según estado.
    let bp = ButtonPalette::from_theme(theme);
    let fila = match a.estado {
        EstadoAccion::Propuesta => {
            let aprobar = View::new(Style { size: Size { width: length(96.0), height: Dimension::auto() }, ..Default::default() })
                .children(vec![button_view("aprobar", &bp, lift.clone()(Msg::Aprobar { turno, bloque }))]);
            let rechazar = View::new(Style { size: Size { width: length(96.0), height: Dimension::auto() }, ..Default::default() })
                .children(vec![button_view("rechazar", &bp, lift(Msg::Rechazar { turno, bloque }))]);
            View::new(Style {
                flex_direction: FlexDirection::Row,
                gap: Size { width: length(8.0), height: length(0.0) },
                size: Size { width: percent(1.0), height: length(34.0) },
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
            View::new(Style { size: Size { width: percent(1.0), height: length(20.0) }, ..Default::default() })
                .text(txt, 11.0, col)
        }
    };
    hijos.push(fila);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0), height: Dimension::auto() },
        gap: Size { width: length(0.0), height: length(6.0) },
        padding: rect_xy(10.0, 8.0),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(hijos)
}

// ─── Helpers de vista ───────────────────────────────────────────────────────

fn rotulo<HostMsg: Clone + 'static>(txt: &str, theme: &Theme) -> View<HostMsg> {
    View::new(Style {
        size: Size { width: percent(1.0), height: length(20.0) },
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
        size: Size { width: percent(1.0), height: length(26.0) },
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
    fn nueva_conversacion_deselecciona() {
        let mut s = estado_con_agente();
        s.input.set_text("hola");
        s = update(s, Msg::Enviar);
        assert!(s.conv_activa.is_some());
        s = update(s, Msg::NuevaConversacion);
        assert!(s.conv_activa.is_none());
    }
}
