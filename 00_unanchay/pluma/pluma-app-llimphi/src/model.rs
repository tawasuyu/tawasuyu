//! Modelo de la app y mensajes del bucle Elm.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use llimphi_ui::{DragPhase, KeyEvent};
use llimphi_widget_text_editor::{EditorMetrics, PointerEvent};
use llimphi_widget_text_input::TextInputState;
use llimphi_widget_toast::Toast;
use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;
use pluma_editor_llimphi::cuerpo_ide::CuerpoIde;
use pluma_estilo::{EstiloLienzo, EstiloTexto};
use pluma_llm::BackendKind;
use pluma_llm_core::ChatClient;
use pluma_proyecto::{DocId, Hash as ProyHash, Proyecto};
use pluma_transform::Transformacion;
use uuid::Uuid;

use crate::clipboard::ArboardClipboard;

pub(crate) const METRICS: EditorMetrics = EditorMetrics::for_font_size(13.0);
pub(crate) const VISIBLE_LINES: usize = 200;

/// Ancho del rail de dientes, en px.
pub(crate) const RAIL_W: f32 = 46.0;
/// Ancho fijo de cada columna del multilienzo cuando hay ≥2 lienzos.
pub(crate) const ANCHO_COL: f32 = 360.0;
/// Ancho del carril entre columnas (= `ConfigMultilienzoEditor::ancho_carril`).
pub(crate) const ANCHO_CARRIL: f32 = 56.0;

/// Métricas del editor con el **alto de línea adaptado** al mayor `size_px` en
/// uso entre los lienzos visibles. Uniforme (todas las columnas comparten el
/// mismo alto, así las hebras del multilienzo siguen alineadas y el
/// hit-testing por `screen_to_pos` queda consistente), pero crece lo suficiente
/// para que las fuentes grandes no se solapen con la línea siguiente. Debe
/// usarse TANTO al renderizar como al convertir clicks → posición.
pub(crate) fn metrics_efectivas(model: &Model) -> EditorMetrics {
    let mut m = METRICS;
    let ratio = METRICS.line_height / METRICS.font_size;
    let mut max_size = METRICS.font_size;
    let ids: Vec<Uuid> = if model.solo_activo {
        model.activo.into_iter().collect()
    } else {
        model.seleccionados.clone()
    };
    for id in ids {
        if let Some(e) = model.estilos.get(&id) {
            if let Some(s) = e.max_size_px() {
                max_size = max_size.max(s);
            }
        }
    }
    m.line_height = (max_size * ratio).max(METRICS.line_height);
    m
}

/// Ancho total del contenido del multilienzo para `n` columnas fijas, o `0`
/// si `n < 2` (con una sola columna es elástica, sin scroll).
pub(crate) fn ancho_contenido(n: usize) -> f32 {
    if n < 2 {
        0.0
    } else {
        n as f32 * ANCHO_COL + (n as f32 - 1.0) * ANCHO_CARRIL
    }
}

/// Un filtro del grafo semántico: una etapa que transforma o acota el lienzo
/// que recibe. Encadenados de la fuente (lienzo activo) al sumidero, generan
/// una **línea de lienzo** nueva. Los tres primeros son transformaciones LLM
/// (las mismas que el diente Modelo); `Concepto` es un filtro semántico que
/// retiene sólo los párrafos afines a un término — por similitud coseno de
/// embeddings vía el verbo-daemon (rimay-verbo), con fallback léxico (substring)
/// si el socket no está disponible.
#[derive(Clone, Debug)]
pub enum Filtro {
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
    Concepto(String),
}

/// Un nodo-filtro posicionado en el lienzo del grafo (canvas coords del
/// nodegraph). El orden en `Model::grafo` es el orden del pipeline.
#[derive(Clone, Debug)]
pub struct NodoFiltro {
    pub filtro: Filtro,
    pub x: f32,
    pub y: f32,
}

/// Modo del centro: las tres caras unificadas de pluma sobre el mismo
/// documento. `Lienzos` es la superficie por defecto (títulos como cajas
/// anidadas, editable in-situ, con tamaño de fuente por nivel); `Presentar`
/// vuela por las secciones con la cámara del deck; `Plano` es el editor
/// multilienzo clásico (text-editor por cuerpo).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modo {
    Lienzos,
    Presentar,
    Plano,
}

impl Modo {
    /// Cicla Lienzos → Presentar → Plano → Lienzos.
    pub(crate) fn siguiente(self) -> Modo {
        match self {
            Modo::Lienzos => Modo::Presentar,
            Modo::Presentar => Modo::Plano,
            Modo::Plano => Modo::Lienzos,
        }
    }

    pub(crate) fn etiqueta(self) -> &'static str {
        match self {
            Modo::Lienzos => "Lienzos",
            Modo::Presentar => "Presentar",
            Modo::Plano => "Plano",
        }
    }
}

/// A qué porción del lienzo apunta el panel de estilo: el lienzo entero, una
/// zona concreta (índice de zona de `CuerpoIde`), o la selección de texto
/// actual del editor activo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ObjetivoEstilo {
    #[default]
    Lienzo,
    Zona(usize),
    Seleccion,
}

impl ObjetivoEstilo {
    pub(crate) fn etiqueta(self) -> &'static str {
        match self {
            ObjetivoEstilo::Lienzo => "Lienzo",
            ObjetivoEstilo::Zona(_) => "Zona",
            ObjetivoEstilo::Seleccion => "Selección",
        }
    }
}

/// Qué control expandible del panel de estilo está abierto (combos de fuente y
/// tamaño = trigger + lista inline; selectores de color = color-picker inline).
/// Sólo uno a la vez.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EstiloExpand {
    Fuente,
    Tamano,
    ColorFg,
    ColorBg,
}

/// Tipo de transformación que define el wizard de "+": qué inteligencia se
/// aplica sobre el lienzo madre elegido.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WizardTipo {
    Traducir,
    Tono,
    Resumir,
    Reescribir,
    /// Script Rhai local: transforma cada párrafo (var `texto`).
    Custom,
}

impl WizardTipo {
    pub(crate) fn etiqueta(self) -> &'static str {
        match self {
            WizardTipo::Traducir => "Traducir",
            WizardTipo::Tono => "Tono",
            WizardTipo::Resumir => "Resumir",
            WizardTipo::Reescribir => "Reescribir",
            WizardTipo::Custom => "Rhai",
        }
    }

    /// Placeholder del campo de parámetro según el tipo.
    pub(crate) fn placeholder(self) -> &'static str {
        match self {
            WizardTipo::Traducir => "lengua destino (qu, en, fr…)",
            WizardTipo::Tono => "etiqueta de tono (formal, infantil…)",
            WizardTipo::Resumir => "palabras objetivo (ej. 30) — vacío = libre",
            WizardTipo::Reescribir => "prompt de reescritura…",
            WizardTipo::Custom => "script Rhai: usá `texto`, ej. texto.to_upper()",
        }
    }
}

/// Estado del wizard modal de transformación. La madre arranca en el lienzo
/// activo; el parámetro se teclea en `preset_input` (reusado), cuyo significado
/// depende de `tipo`.
#[derive(Clone, Debug)]
pub struct WizardEstado {
    pub madre: Option<Uuid>,
    pub tipo: WizardTipo,
}

impl Default for WizardEstado {
    fn default() -> Self {
        Self {
            madre: None,
            tipo: WizardTipo::Traducir,
        }
    }
}

/// Sub-pestaña del panel de un proyecto. `Historia` es el grafo de versiones;
/// las otras son las herramientas (antes dientes del rail) que ahora son
/// propiedades del proyecto activo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProyectoTab {
    #[default]
    Historia,
    Lienzos,
    Modelo,
    Grafo,
}

impl ProyectoTab {
    pub(crate) fn etiqueta(self) -> &'static str {
        match self {
            ProyectoTab::Historia => "Historia",
            ProyectoTab::Lienzos => "Lienzos",
            ProyectoTab::Modelo => "Modelo",
            ProyectoTab::Grafo => "Grafo",
        }
    }
}

/// Qué se está renombrando en el modal de renombrar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenombrarObjetivo {
    Proyecto,
    Documento(DocId),
}

/// Un proyecto abierto: el `Proyecto` (con su DAG de versiones), su ruta en
/// disco (`None` = nunca guardado), y qué documento del proyecto está activo.
pub struct ProyectoAbierto {
    pub proyecto: Proyecto,
    pub ruta: Option<PathBuf>,
    pub doc_activo: DocId,
}

impl ProyectoAbierto {
    /// Proyecto vacío con un documento (para arranque/headless).
    pub(crate) fn vacio(nombre: &str) -> Self {
        let mut proyecto = Proyecto::nuevo(nombre);
        let doc_activo = proyecto.nuevo_documento("documento 1");
        Self { proyecto, ruta: None, doc_activo }
    }
}

pub(crate) const BACKENDS: [BackendKind; 6] = [
    BackendKind::Mock,
    BackendKind::Gemini,
    BackendKind::Anthropic,
    BackendKind::DeepSeek,
    BackendKind::Cohere,
    BackendKind::Ollama,
];

#[derive(Clone, Debug)]
pub enum Msg {
    EditorKey(KeyEvent),
    /// Click/drag dentro de la columna del cuerpo `Uuid` del multilienzo. Si
    /// ese cuerpo no es el activo, primero le pasa el foco (se apropia del
    /// teclado). Se identifica por `Uuid` y no por índice porque la lista de
    /// columnas visibles puede no coincidir 1-1 con `seleccionados`.
    MultiPointer(Uuid, PointerEvent),
    /// Abre un cuerpo como activo (lo agrega a la selección si no estaba).
    AbrirDoc(Uuid),
    /// Agrega/saca un cuerpo de la selección visible del multilienzo.
    ToggleSeleccion(Uuid),
    /// Reordena el tree de lienzos: mueve el lienzo en la posición `desde` a la
    /// posición `hasta` de `orden_lienzos` (drag&drop de filas). El orden del
    /// tree manda el orden de las columnas.
    ReordenarLienzo(usize, usize),
    /// Selecciona el diente del rail (0=Archivo,1=Lienzos,2=Derivar,3=LLM).
    SelectDiente(usize),
    /// Ctrl+Tab / Ctrl+Shift+Tab: mueve el foco al lienzo siguiente/anterior
    /// de la selección (cicla).
    FocoSiguiente,
    FocoAnterior,
    /// Activa/desactiva el foco por hover (pasar el cursor cambia el lienzo
    /// activo).
    ToggleFocoHover,
    /// Scroll horizontal del multilienzo, en píxeles (positivo = derecha).
    ScrollHoriz(f32),
    /// Scroll vertical del lienzo con foco, en "notches" de rueda (positivo =
    /// rueda hacia arriba). Los demás lienzos se nivelan al del foco.
    ScrollVert(f32),
    /// La ventana cambió de tamaño (ancho, alto) — para clampear el scroll.
    Resized(f32, f32),
    /// Tick del parpadeo del caret (~530 ms) — alterna su fase visible.
    CaretBlink,
    /// Tick del fluido de los cauces (~33 Hz) — avanza `fase_flujo`.
    FlujoTick,
    NuevoDoc,
    Guardar,
    PathInputKey(KeyEvent),
    FocusPath,
    DefocusPath,
    AbrirArchivo,
    ExportarMd,
    FindToggle,
    FindKey(KeyEvent),
    FindSiguiente,
    FindAnterior,
    FindClose,
    /// Togglea el modo "sólo activo" (una columna) vs "todos los
    /// seleccionados" (multilienzo completo) — antes era Diff.
    DiffToggle,
    /// Rail hospedado: pata reenvió un clic en un diente prestado — mapea
    /// directo a `SelectDiente`.
    HostActivate(u32),
    MoverAtomArriba,
    MoverAtomAbajo,
    TocarMadre,
    RegenerarStale,
    ToglearFusion,
    ZonaSiguiente,
    ZonaAnterior,
    CicloBackend,
    PedirTraducir(String),
    PedirTono(String),
    PedirResumir(Option<u32>),
    // --- Diente Derivar-IA: lienzo alterno desde prompt + presets ---
    /// Teclas hacia el input de prompt del diente Derivar.
    PresetInputKey(KeyEvent),
    FocusPreset,
    DefocusPreset,
    /// Deriva un lienzo alterno reescribiendo el activo con el prompt del input.
    CrearAlterno,
    /// Guarda el prompt actual del input como preset reutilizable.
    GuardarPreset,
    /// Re-corre el preset `usize` (lo reescribe sobre el activo).
    UsarPreset(usize),
    /// Borra el preset `usize` de la lista.
    BorrarPreset(usize),
    LlmListo {
        hija: Cuerpo,
        atoms_nuevos: Vec<NarrativeAtom>,
        carta: CartaHebras,
        transformacion: Transformacion,
    },
    /// Como `LlmListo` pero **reemplaza** a la hija `vieja` en su mismo lugar
    /// (regeneración reactiva in-place): no apila una traducción nueva ni mueve
    /// el foco. La disparan `Ctrl+Enter` / `Enter` al final del último párrafo.
    HijaEnLugar {
        vieja: Uuid,
        hija: Cuerpo,
        atoms_nuevos: Vec<NarrativeAtom>,
        carta: CartaHebras,
        transformacion: Transformacion,
    },
    LlmError(String),

    // --- Diente Grafo: grafo semántico de filtros → línea de lienzo ---
    /// Agrega un nodo-filtro al final del pipeline.
    GrafoAdd(Filtro),
    /// Borra el nodo-filtro cuyo `NodeId` se pasa (right-click). La fuente
    /// (id 0) y el sumidero no se pueden borrar — se ignoran.
    GrafoDel(u32),
    /// Arrastra un nodo del grafo: `NodeId`, fase, delta (dx, dy).
    GrafoDrag(u32, DragPhase, f32, f32),
    /// Teclas hacia el input del término del filtro Concepto.
    GrafoInputKey(KeyEvent),
    FocusGrafo,
    DefocusGrafo,
    /// Corre el pipeline de filtros sobre el activo y agrega la línea generada.
    GenerarLinea,
    /// Vacía el grafo de filtros.
    GrafoLimpiar,
    /// Arrastra el divisor entre el panel del diente y el centro.
    ResizePanel(f32),

    // --- Menú principal + menú de edición contextual ---
    /// Abre/cierra un dropdown del menú principal (índice del menú raíz).
    MenuOpen(Option<usize>),
    /// Comando string del menú principal (rebota desde `on_command`).
    MenuCommand(String),
    /// Navegación por teclado en el menú principal (`+1` baja, `-1` sube).
    MenuNav(i32),
    /// Enter en el menú principal: ejecuta la fila activa.
    MenuActivate,
    /// Tick de animación de menús (sólo re-render).
    MenuTick,
    /// Navegación por teclado en el menú de edición.
    EditNav(i32),
    /// Enter en el menú de edición: ejecuta la fila activa.
    EditActivate,
    /// Right-click: abre el menú de edición anclado en (x, y) de ventana.
    EditMenuOpen(f32, f32),
    /// Acción elegida en el menú de edición contextual.
    EditMenuAction(llimphi_widget_edit_menu::EditAction),
    /// Cierra cualquier menú abierto (dropdown o edición).
    CloseMenus,

    // --- Unificación: modos (Lienzos / Presentar / Plano) ---
    /// Cicla el modo del centro.
    CicloModo,
    /// Fija un modo concreto del centro.
    SetModo(Modo),
    /// Click en una caja de lienzo (modo Lienzos): empieza la edición in-situ
    /// de ese átomo.
    LienzoSelect(Uuid),
    /// Tecla hacia el editor in-situ del átomo en edición.
    LienzoEditKey(KeyEvent),
    /// Click/drag dentro del editor in-situ (mover caret / selección).
    LienzoEditPointer(PointerEvent),
    /// Cierra la edición in-situ guardando el cambio del átomo (y re-deriva la
    /// jerarquía si el `#` cambió).
    LienzoCommit,
    /// Presentar: vuela al paso siguiente / anterior / vista general.
    PresSiguiente,
    PresAnterior,
    PresVistaGeneral,
    /// Tick de animación del vuelo de cámara (modo Presentar).
    PresTick,
    /// Scroll vertical del modo Lienzos, en "notches" de rueda (positivo = arriba).
    LienzosScroll(f32),
    /// Ejecuta el lienzo-celda `Uuid` (notebook embebido): corre su cuerpo como
    /// prompt LLM y guarda la salida.
    EjecutarLienzo(Uuid),
    /// Resultado de ejecutar una celda: `(átomo, texto de salida)`.
    LienzoSalida { atom: Uuid, texto: String },

    // --- Rail derecho de estilo (un diente por lienzo) ---
    /// Activa el diente de estilo del lienzo `Uuid` (toggle: re-click cierra el
    /// panel). Despliega su panel de estilo a la derecha.
    SelectDienteEstilo(Uuid),
    /// Cierra el panel de estilo (sin diente activo).
    CerrarPanelEstilo,
    /// Cambia el objetivo del panel de estilo (Lienzo / Zona(i) / Selección).
    SetObjetivoEstilo(ObjetivoEstilo),
    /// Mergea un delta de estilo (parcial) sobre el objetivo actual del lienzo
    /// del panel de estilo, y lo persiste.
    AplicarEstilo(EstiloTexto),
    /// Limpia el estilo del objetivo actual (vuelve al default).
    EstiloReset,
    /// Abre/cierra un control expandible del panel de estilo (combo o picker).
    ToggleEstiloExpand(EstiloExpand),
    /// Arrastra el divisor del panel de estilo (derecha).
    ResizePanelEstilo(f32),

    // --- Wizard "+" de transformación (reemplaza el diente Derivar) ---
    /// Abre el wizard modal de nueva transformación (madre = activo).
    AbrirWizard,
    /// Cierra el wizard sin crear nada.
    CerrarWizard,
    /// Elige el lienzo madre sobre el que correr la transformación.
    WizardMadre(Uuid),
    /// Elige el tipo de transformación del wizard.
    WizardTipoSel(WizardTipo),
    /// Confirma el wizard: arma y lanza la transformación sobre la madre.
    WizardConfirm,

    // --- Proyectos versionados (rail izquierdo) ---
    /// Crea un proyecto nuevo (en memoria) y lo abre.
    NuevoProyecto,
    /// Abre un proyecto `.pluma` desde la ruta del `path_input`.
    AbrirProyecto,
    /// Asigna la ruta del `path_input` al proyecto activo y lo guarda (.pluma).
    GuardarProyectoComo,
    /// Cierra (saca del rail) el proyecto `idx`.
    CerrarProyecto(usize),
    /// Activa el proyecto `idx` (carga su documento activo en la superficie).
    ActivarProyecto(usize),
    /// Cambia la sub-pestaña del panel de proyecto.
    SetProyectoTab(ProyectoTab),
    /// Selecciona/activa un documento dentro del proyecto activo.
    SelDocProyecto(DocId),
    /// Agrega un documento nuevo al proyecto activo.
    NuevoDocProyecto,
    /// Elimina un documento del proyecto activo (nunca deja 0).
    EliminarDoc(DocId),
    /// Borra una rama del proyecto activo.
    BorrarRama(String),
    /// Abre el modal de renombrar (proyecto o documento).
    AbrirRenombrar(RenombrarObjetivo),
    /// Confirma el renombre con el texto tecleado.
    ConfirmarRenombrar,
    /// Cierra el modal de renombrar.
    CerrarRenombrar,
    /// Abre el modal de push (mensaje del snapshot).
    AbrirPush,
    /// Confirma el push con el mensaje tecleado.
    ConfirmarPush,
    /// Cierra el modal de push.
    CerrarPush,
    /// Previsualiza un commit (solo lectura) — fija `commit_preview`.
    VerCommit(ProyHash),
    /// Cierra la previsualización de commit.
    CerrarPreview,
    /// Restaura un commit como estado de trabajo (checkout).
    RestaurarCommit(ProyHash),
    /// Crea una rama nueva desde el HEAD del proyecto activo.
    NuevaRama,
    /// Cambia a la rama `String` del proyecto activo.
    CambiarRama(String),
    /// Mergea la rama `String` en la rama actual.
    MergeRama(String),
    /// Compacta (GC) el store del proyecto activo: descarta objetos inalcanzables.
    CompactarProyecto,
    /// Descarta un toast del stack al clickearlo (los expirados se podan solos
    /// en `FlujoTick`).
    DescartarToast(u64),
    /// Cotejá dos documentos: toma los dos lienzos seleccionados (o, si no hay
    /// dos en la selección, los dos últimos abiertos), los compara con
    /// `pluma-cotejo` y abre el overlay de cotejo (verde = coincide, rojo =
    /// difiere) con el lienzo de diferencias en el medio.
    Cotejar,
    /// Cierra el overlay de cotejo.
    CerrarCotejo,
    /// Desplaza verticalmente el overlay de cotejo (delta de rueda).
    CotejoScroll(f32),
    /// Invierte izquierda↔derecha del cotejo y lo recalcula (los lienzos son
    /// intercambiables: ver el cambio desde la otra orilla).
    CotejoInvertir,
    /// Abre el diálogo de "cotejar dos archivos" (dos rutas).
    AbrirDialogoCotejo,
    /// Cierra el diálogo de cotejo sin comparar.
    CerrarDialogoCotejo,
    /// Da foco a un campo de ruta del diálogo de cotejo.
    CotejoDialogFoco(CotejoCampo),
    /// Teclea en el campo con foco del diálogo de cotejo.
    CotejoDialogKey(KeyEvent),
    /// Confirma el diálogo: carga ambos archivos (o cae a los documentos
    /// abiertos si las rutas están vacías) y abre el overlay de cotejo.
    ConfirmarCotejoArchivos,
    /// Pide al modelo (pluma-llm) que redacte el resumen de cada diferencia del
    /// cotejo abierto. Trabajo async; al volver despacha `CotejoResumenListo`.
    CotejoResumirIA,
    /// Resultado del resumidor IA: una línea por sección, en orden. Reemplaza
    /// el contenido de los átomos del lienzo de diferencias.
    CotejoResumenListo(Vec<String>),
    /// El resumidor IA falló — se conserva el resumen textual y se avisa.
    CotejoResumenError(String),
}

/// Estado del overlay de **cotejo**: dos documentos comparados como lienzos
/// paralelos, con un tercer lienzo de diferencias en el medio. Es autónomo —
/// clona los cuerpos y átomos involucrados para que el overlay no dependa de
/// ediciones posteriores del modelo. Lo arma `Msg::Cotejar` vía `pluma-cotejo`.
pub(crate) struct EstadoCotejo {
    /// Los tres cuerpos en orden de presentación: `[izq, diferencias, der]`.
    pub(crate) cuerpos: Vec<Cuerpo>,
    /// Índice de todos los átomos referenciados (izquierda + diferencias + derecha).
    pub(crate) atoms: HashMap<Uuid, NarrativeAtom>,
    /// Cartas entre columnas consecutivas: `[izq↔dif, dif↔der]`.
    pub(crate) cartas: Vec<CartaHebras>,
    /// Divergencia `[0,1]` de cada átomo — alimenta el coloreado verde→rojo.
    pub(crate) divergencias: HashMap<Uuid, f32>,
    /// Línea de conteo para la cabecera ("2 idénticas · 1 reescrita · …").
    pub(crate) conteo: String,
    /// Desplazamiento vertical del overlay (px), para documentos más altos que
    /// la ventana. La rueda lo ajusta; `0.0` = tope.
    pub(crate) scroll_y: f32,
    /// Filas del cuerpo más alto — para acotar el scroll a su contenido.
    pub(crate) filas_max: usize,
    /// Las secciones del cotejo (clase + textos por átomo) — necesarias para
    /// armar los ítems del resumidor IA sin recalcular el cotejo.
    pub(crate) secciones: Vec<pluma_cotejo::SeccionCotejo>,
    /// `true` mientras el resumidor IA está en curso (bloquea doble disparo y
    /// muestra el estado en la cabecera).
    pub(crate) resumiendo: bool,
}

/// Qué campo del diálogo de cotejo tiene el foco de teclado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CotejoCampo {
    A,
    B,
}

/// Diálogo para **cotejar dos archivos del disco**: dos rutas. Si ambas quedan
/// vacías al confirmar, el cotejo cae sobre los dos documentos ya abiertos
/// (seleccionados o los dos últimos) — así un mismo botón sirve para comparar
/// archivos sueltos o documentos del proyecto.
pub(crate) struct CotejoDialog {
    pub(crate) a: TextInputState,
    pub(crate) b: TextInputState,
    pub(crate) foco: CotejoCampo,
    pub(crate) error: Option<String>,
}

pub struct Model {
    pub(crate) cuerpos: Vec<Cuerpo>,
    pub(crate) atoms: HashMap<Uuid, NarrativeAtom>,
    pub(crate) cartas: Vec<CartaHebras>,
    pub(crate) transformaciones: Vec<Transformacion>,
    /// `id` del `Cuerpo` activo (el editable en vivo, `ide`). `None` sólo
    /// si la lista de cuerpos está vacía — el init siembra uno para evitarlo.
    pub(crate) activo: Option<Uuid>,
    pub(crate) ide: CuerpoIde,
    /// Modo del centro (Lienzos / Presentar / Plano). Ver [`Modo`].
    pub(crate) modo: Modo,
    /// Edición in-situ en modo Lienzos: `(átomo, estado del editor)`. `None`
    /// cuando no se está editando ninguna caja.
    pub(crate) editando: Option<(Uuid, llimphi_widget_text_editor::EditorState)>,
    /// Estado de la cámara del deck para el modo Presentar (paso + zoom/pan).
    pub(crate) recorrido_state: pluma_deck_core::RecorridoState,
    /// Salida de cada lienzo-celda ejecutado (notebook embebido): átomo → texto.
    pub(crate) salidas: HashMap<Uuid, String>,
    /// Desplazamiento vertical del modo Lienzos, en px (≥ 0).
    pub(crate) lienzos_scroll_y: f32,
    /// Fase `[0,1)` del fluido animado de los cauces Sankey (modo Plano).
    /// La avanza `Msg::FlujoTick` (~33 Hz).
    pub(crate) fase_flujo: f32,
    /// Conjunto de cuerpos visibles en el multilienzo (membresía). Siempre
    /// contiene al `activo`. El ORDEN de columnas lo da `orden_lienzos`, no
    /// este vector.
    pub(crate) seleccionados: Vec<Uuid>,
    /// Orden maestro de todos los cuerpos en el tree de lienzos (reordenable por
    /// drag). Manda tanto el orden del tree como el de las columnas (filtrado
    /// por `seleccionados`).
    pub(crate) orden_lienzos: Vec<Uuid>,
    /// Editores read-only de los cuerpos seleccionados que no son el activo.
    /// Se reconstruyen al cambiar selección/activo/atoms.
    pub(crate) ides_ro: HashMap<Uuid, CuerpoIde>,
    /// Si `true`, el centro muestra sólo el cuerpo activo (una columna);
    /// si `false`, todo el multilienzo de `seleccionados`. Togglea con Ctrl+D.
    pub(crate) solo_activo: bool,
    /// Desplazamiento horizontal del multilienzo, en píxeles. Clampeado a
    /// `[0, ancho_contenido - ancho_centro]`.
    pub(crate) scroll_x: f32,
    /// Tamaño actual de la ventana (ancho, alto) en px lógicos. Lo actualiza
    /// `on_resize`; arranca en `initial_size`.
    pub(crate) viewport: (f32, f32),
    /// Diente activo del rail: 0=Archivo · 1=Lienzos · 2=Derivar · 3=LLM.
    pub(crate) diente_activo: usize,
    /// Si `true`, pasar el cursor sobre una columna le pasa el foco (off por
    /// defecto — se togglea desde el menú Multilienzo).
    pub(crate) foco_por_hover: bool,
    /// Ancho del panel del diente activo, en px (resizable con el divisor).
    pub(crate) panel_w: f32,
    pub(crate) clipboard: ArboardClipboard,
    pub(crate) drag_accum: (f32, f32),

    // --- Diente Derivar-IA ---
    /// Input del prompt para derivar un lienzo alterno.
    pub(crate) preset_input: TextInputState,
    /// Si el input de prompt tiene foco (las teclas van ahí).
    pub(crate) preset_focused: bool,
    /// Prompts guardados reutilizables. Persisten en `presets.txt` junto al sled.
    pub(crate) presets: Vec<String>,

    // --- Diente Grafo ---
    /// Pipeline de filtros (orden = fuente → ... → sumidero).
    pub(crate) grafo: Vec<NodoFiltro>,
    /// Posición del nodo fuente en el canvas del grafo (arrastrable).
    pub(crate) grafo_src: (f32, f32),
    /// Posición del nodo sumidero "→ nueva línea".
    pub(crate) grafo_sink: (f32, f32),
    /// Input del término para el filtro Concepto.
    pub(crate) grafo_input: TextInputState,
    pub(crate) grafo_input_focused: bool,

    pub(crate) chat: Arc<dyn ChatClient>,
    pub(crate) backend_idx: usize,
    pub(crate) en_curso: bool,
    pub(crate) ultimo_error: Option<String>,
    pub(crate) ultimo_status: String,

    /// Ruta del archivo a abrir/exportar — input compartido.
    /// Se interpreta según qué botón clickea el usuario.
    pub(crate) path_input: TextInputState,
    /// Cuando es `true`, las teclas del usuario van al `path_input` en
    /// vez del editor. Click sobre el input lo enciende; Esc, o un
    /// click fuera (en realidad, sólo Esc) lo apaga.
    pub(crate) path_focused: bool,

    /// Find-in-page sobre el cuerpo activo. `Ctrl+F` muestra el overlay
    /// y lo enfoca; Esc lo cierra; Enter/Shift+Enter cyclan matches.
    pub(crate) find_input: TextInputState,
    pub(crate) find_visible: bool,
    pub(crate) find_matches: Vec<(usize, usize)>,
    pub(crate) find_idx: usize,

    /// Índice del menú raíz cuyo dropdown está abierto (`None` = cerrado).
    pub(crate) menu_open: Option<usize>,
    /// Fila resaltada por teclado en el menú principal (`usize::MAX` = ninguna).
    pub(crate) menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal (0→1).
    pub(crate) menu_anim: llimphi_motion::Tween<f32>,
    /// Ancla (x, y) en coords de ventana del menú de edición contextual,
    /// o `None` si no está abierto.
    pub(crate) edit_menu: Option<(f32, f32)>,
    /// Fila resaltada por teclado en el menú de edición (`usize::MAX` = ninguna).
    pub(crate) edit_active: usize,
    /// Animación de aparición del menú de edición (0→1).
    pub(crate) edit_anim: llimphi_motion::Tween<f32>,

    // --- Rail hospedado (dientes delegados a pata) ---
    /// `true` si pluma delega su rail a pata (`PLUMA_DELEGATE_SIDEBAR`): sus
    /// dientes aparecen en el rail de pata cuando tiene foco y pluma no dibuja
    /// su propio rail interno (sólo el panel del diente activo + el centro).
    pub(crate) delegated: bool,
    /// Cliente del rail hospedado; sólo se retiene (las activaciones llegan por
    /// callback). `_` evita el lint de campo sin leer.
    pub(crate) _host: Option<pata_host::HostClient>,
    /// Último diente activo reportado al rail hospedado de pata (`diente_activo`).
    /// Evita reenviar el mismo estado en cada `update`: sólo se manda `SetActive`
    /// cuando cambia. Inerte sin `_host` (no delegado).
    pub(crate) host_active_synced: Option<u32>,

    // --- Estilo por lienzo (rail derecho + panel) ---
    /// Estilo persistido de cada lienzo (cache en memoria; fuente de verdad en
    /// el store, árbol `estilos`). Ausente = lienzo sin estilo.
    pub(crate) estilos: HashMap<Uuid, EstiloLienzo>,
    /// Lienzo cuyo panel de estilo está abierto a la derecha (`None` = cerrado).
    pub(crate) diente_estilo_activo: Option<Uuid>,
    /// Ancho del panel de estilo, en px (resizable).
    pub(crate) panel_estilo_w: f32,
    /// Objetivo del panel de estilo: lienzo entero / zona / selección.
    pub(crate) objetivo_estilo: ObjetivoEstilo,
    /// Control expandible abierto del panel de estilo (combo/picker), si hay.
    pub(crate) estilo_expand: Option<EstiloExpand>,

    // --- Wizard de transformación ("+") ---
    /// Estado del wizard modal de nueva transformación (`None` = cerrado).
    pub(crate) wizard: Option<WizardEstado>,

    // --- Proyectos versionados ---
    /// Proyectos abiertos (uno por diente del rail izquierdo, además de Archivo).
    pub(crate) proyectos: Vec<ProyectoAbierto>,
    /// Índice del proyecto cuyo documento se está editando.
    pub(crate) proyecto_activo: usize,
    /// Sub-pestaña del panel del proyecto (Historia/Lienzos/Modelo/Grafo).
    pub(crate) proyecto_tab: ProyectoTab,
    /// Commit en previsualización (solo lectura), si lo hay.
    pub(crate) commit_preview: Option<ProyHash>,
    /// Modal de push abierto (mensaje en `path_input`).
    pub(crate) push_abierto: bool,
    /// Modal de renombrar abierto (texto nuevo en `preset_input`).
    pub(crate) renombrar: Option<RenombrarObjetivo>,
    /// Rutas de proyectos recientes (persistidas junto al sled).
    pub(crate) proyectos_recientes: Vec<PathBuf>,

    // --- Toasts efímeros ---
    /// Notificaciones efímeras vivas (guardar / transformar / error LLM). Las
    /// expiradas se podan en `Msg::FlujoTick`; el click las descarta.
    pub(crate) toasts: Vec<Toast>,
    /// Id incremental para correlacionar toast ↔ `Msg::DescartarToast`.
    pub(crate) next_toast: u64,

    // --- Cotejo (comparación de dos documentos) ---
    /// Overlay de cotejo activo, si lo hay. `Some` lo pinta a pantalla
    /// completa por encima de todo; `Esc` o el botón ✕ lo cierran.
    pub(crate) cotejo: Option<EstadoCotejo>,
    /// Diálogo "cotejar dos archivos" abierto, si lo hay.
    pub(crate) cotejo_dialog: Option<CotejoDialog>,
}
