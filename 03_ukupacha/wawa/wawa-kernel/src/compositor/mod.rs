// =============================================================================
//  renaser :: kernel/src/compositor.rs — el compositor: teselado y flotantes
// -----------------------------------------------------------------------------
//  El kernel no coloca las ventanas a mano: las TESELA. El motor es
//  `mirada-layout` —el mismo nucleo `no_std` que ordena el compositor Wayland
//  de brahman—, enlazado por `path` cruzando la frontera de workspace.
//
//  FASE 8b/8c :: el compositor cobra vida. Mantiene un ESCRITORIO —el registro
//  de todas las ventanas— y, por cada una, una CACHE de respaldo con su ultimo
//  fotograma. Gracias a esa cache, el teclado puede re-teselar el escritorio en
//  caliente —o mover el foco— y el kernel recompone cada ventana en su marco
//  nuevo SIN despertar a las apps: una app que solo pinto en su `init` conserva
//  su imagen intacta a traves de cualquier reordenacion.
//
//  FASE 9 :: orden-Z y ventanas flotantes. Una ventana puede ABANDONAR el
//  teselado y FLOTAR —un marco propio, libre, que SOLAPA a las demas—. El
//  escritorio separa entonces dos capas: las TESELADAS, al fondo, sin
//  solapamiento entre si; y las FLOTANTES, encima, apiladas por un orden-Z
//  —`flotantes` ES esa pila, de atras hacia adelante; la ultima es la frontal—.
//  Con flotantes vivas el kernel deja de pintar cada ventana por separado:
//  RECOMPONE el escritorio entero, capa a capa, de modo que el solapamiento se
//  resuelva por el orden del pintado, sin recortes ni mascaras.
//
//  FASE 10 :: el escritorio deja de ser un censo fijo. Una ventana puede
//  CERRARSE en vivo (`Alt+Q`): se la marca, su app concluye su tarea por su
//  voluntad y el teselado reclama su espacio. Y puede NACER una ventana nueva
//  (`Alt+N`): `nacer_ventana` la añade al censo y devuelve su indice al
//  orquestador, que instancia su WASM y engendra su tarea. El censo de
//  ventanas solo crece —los indices son la IDENTIDAD, jamas se reciclan—; una
//  ventana cerrada queda como una ranura inerte, fuera del orden y del foco.
//
//  FASE 13 :: el raton entra en juego. Hay un PUNTERO en pantalla y el
//  compositor gana dos gestos: clic-para-enfocar (sobre cualquier ventana viva)
//  y ARRASTRAR una flotante con el boton izquierdo sostenido. Como el teclado
//  y la bocina, los eventos del raton vienen del manejador de IRQ12 por una
//  cola lock-free; `atender_raton` los drena cooperativamente y, al detectar
//  un boton que baja o un arrastre en curso, mueve el foco o el marco. Los
//  cuartos flotantes dejan, por fin, de estar clavados en su cascada.
//
//  EXCLUSION DE INTERRUPCIONES. El `ESCRITORIO` lo tocan SOLO tareas
//  cooperativas (el `tick` de una app, la tarea del compositor): el manejador
//  de IRQ1 jamas lo bloquea. La IRQ se comunica con el mundo cooperativo por
//  un canal estrecho y a prueba de interbloqueos: dos atomicos —el foco y el
//  estado de Alt— y una cola lock-free de mandos. Ningun cerrojo que la IRQ
//  pudiera disputar a una tarea cooperativa.
// =============================================================================

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crossbeam_queue::ArrayQueue;
use mirada_layout::{tile, LayoutMode, LayoutParams, Rect};
use spin::{Mutex, Once};

use crate::consola;
use crate::grafico::{Color, RegionPantalla};

/// Altura del strip superior reservado a la consola; las apps teselan debajo.
/// La consola conserva ahi su registro de arranque completo —seis lineas,
/// hasta la sonda asincrona de disco— legible sobre el teselado.
const FRANJA_CONSOLA: usize = 296;

/// Altura de la barra de tareas inferior (Fase 14): cada ventana viva tiene
/// ahi una pestaña con su nombre, que el clic enfoca.
const FRANJA_TASKBAR: usize = 40;

/// Anchura de cada celda de la barra de tareas, en pixeles. Dimensionada para
/// que las nueve apps de genesis (Fase 19 anexa `pregon`) + el lanzador + el
/// reloj caben holgados en una pantalla de 1280 px.
const CELDA_TASKBAR_ANCHO: usize = 116;
/// Hueco entre celdas adyacentes de la barra.
const CELDA_TASKBAR_HUECO: usize = 6;
/// Margen izquierdo y derecho de la barra de tareas.
const CELDA_TASKBAR_MARGEN: usize = 12;
/// Anchura del boton lanzador («+» a la izquierda de la barra, Fase 16).
const LAUNCHER_ANCHO: usize = 36;
/// Hueco entre el lanzador y la primera pestaña.
const LAUNCHER_HUECO: usize = 8;
/// Anchura reservada para el reloj a la derecha de la barra (Fase 16).
const RELOJ_ANCHO: usize = 80;
/// Hueco entre la ultima pestaña y el reloj.
const RELOJ_HUECO: usize = 8;

/// El modo de teselado con que arranca el escritorio. El teclado lo cicla.
const MODO_INICIAL: LayoutMode = LayoutMode::MasterStack;

/// Margen entre ventanas teseladas, en pixeles.
const MARGEN: i32 = 14;

/// Capacidad de la cola de mandos del compositor — holgada: nadie pulsa tanto.
const CAPACIDAD_MANDOS: usize = 32;

/// Reborde de cromo de una ventana flotante: el panel que rodea su lienzo
/// natural, donde se asienta el borde de foco sin tapar el dibujo de la app.
const CROMO_FLOTANTE: usize = 8;

/// Paso de la cascada con que se colocan las ventanas flotantes nuevas, en
/// pixeles. Cada flotante se desplaza un paso respecto a la anterior, de modo
/// que varias no se tapen por completo.
const PASO_CASCADA: usize = 44;

/// Un mando del compositor — lo emite el teclado desde el contexto de IRQ, lo
/// atiende la tarea del compositor desde el reactor cooperativo.
#[derive(Clone, Copy)]
pub enum Mando {
    /// Ciclar al siguiente modo de teselado de `mirada-layout`.
    CiclarLayout,
    /// Mover el foco a la siguiente ventana viva.
    FocoSiguiente,
    /// Mover el foco a la ventana viva anterior.
    FocoAnterior,
    /// Promover la ventana enfocada a la posicion maestra del teselado.
    Promover,
    /// Mover la ventana enfocada una posicion adelante en el orden de teselado.
    MoverAdelante,
    /// Mover la ventana enfocada una posicion atras en el orden de teselado.
    MoverAtras,
    /// Alternar la ventana enfocada entre teselada y flotante (Fase 9).
    Flotar,
    /// FASE 64 :: mover la ventana enfocada al SIGUIENTE output/monitor, en
    /// rotacion (`Alt+O`). Cambia su `Ventana::output` y retesela: el teselado
    /// por-output la reagrupa en la region del monitor destino. No-op con un
    /// solo monitor.
    MoverVentanaOutput,
    /// Cerrar la aplicacion enfocada — una baja limpia, en vivo (Fase 10).
    Cerrar,
    /// Lanzar una aplicacion nueva — un alta en vivo (Fase 10).
    Lanzar,
    /// Forzar una pasada del compactador semantico del grafo (Fase 57).
    /// Es la palanca operacional in-VM equivalente al futuro `wawactl gc`
    /// host-side: el operador pulsa `Alt+G` y el compositor invoca
    /// `almacen::compactar()` en su tic, emitiendo el resultado por la
    /// baliza serial. No interactua con el grafo de aplicaciones —es
    /// estrictamente mantenimiento del log direccionado por contenido—.
    CompactarGrafo,
    /// Abre o cierra el launcher grafico (Fase 58). Mientras esta abierto,
    /// `FocoSiguiente`/`FocoAnterior` mueven la seleccion DENTRO del overlay
    /// —no entre ventanas—, `Promover` lanza la app seleccionada y `Cerrar`
    /// cierra el overlay (sin matar a la ventana enfocada). Es el sustituto
    /// dirigido del ciclo ciego de `Alt+N`.
    ToggleLauncher,
    /// FASE 58 v3 :: una letra/cifra/espacio para la query del launcher, o el
    /// byte sentinela `0x08` para borrar el ultimo caracter. La empuja el
    /// teclado cuando ve un scancode imprimible mientras `LAUNCHER_ABIERTO`
    /// esta vivo (Alt sin pulsar). Cualquier otro byte es ASCII minuscula.
    TextoLauncher(u8),
    /// FASE 58 v8 :: lanzamiento rapido `Alt+1..9` sobre la fila VISIBLE
    /// `0..=8` del launcher. El indice esta en el rango `0..=8` (1-based en
    /// teclado, 0-based en el mando); el compositor le suma `launcher_scroll`
    /// para resolver al indice absoluto del filtrado y dispara la app si
    /// existe, en silencio si la fila visible esta vacia. Solo se emite
    /// cuando `LAUNCHER_ABIERTO` esta vivo.
    LanzarFila(usize),
}

/// Un arrastre EN CURSO (Fase 13): el indice de la ventana flotante asida con
/// el raton y el desfase con que se asio —para que la ventana no salte al
/// agarrarla, sino que siga al puntero como si lo llevara cogido por ahi—.
#[derive(Clone, Copy)]
pub(crate) struct Arrastre {
    ventana: usize,
    agarre_dx: usize,
    agarre_dy: usize,
}

/// Una ventana del escritorio: una app, su geometria y su ultimo fotograma.
struct Ventana {
    /// Nombre legible de la app — el que dicta su `EntradaApp` del manifiesto.
    /// Aparece en la pestaña de la barra de tareas (Fase 14).
    nombre: String,
    /// Tamaño natural del lienzo de la app — lo que sabe pintar, fijo.
    natural_ancho: usize,
    natural_alto: usize,
    /// El marco actual — donde la app vive en pantalla. Si la ventana esta
    /// teselada, lo fija el teselado; si flota, es un marco propio y libre.
    marco: RegionPantalla,
    /// CACHE de respaldo: el ultimo fotograma exitoso que la app envio. Su
    /// tamaño esta acotado al lienzo natural —`natural_ancho × natural_alto ×
    /// 4`— y se reserva UNA sola vez, al fundar el escritorio: jamas crece. El
    /// re-teselado recompone la ventana desde aqui, sin molestar a la app.
    cache: Vec<u8>,
    /// ¿Ha enviado la app al menos un fotograma? Hasta entonces, su cache es
    /// solo ceros y no se recompone.
    pintada: bool,
    /// Si el kernel desalojo la app, el color de su baliza. `None` mientras
    /// vive; `Some(color)` la marca como muerta y la excluye del foco.
    baliza: Option<Color>,
    /// ¿Se ha pedido cerrar esta ventana en vivo (Fase 10)? Una vez `true`, su
    /// app concluye su tarea, la ranura queda inerte —fuera del orden, del
    /// orden-Z y del foco— y el teselado reclama su espacio.
    cerrada: bool,
    /// FASE 59 v2 :: el output donde vive esta ventana. Hoy siempre `0` —el
    /// unico output existente—; cuando un driver enumere outputs adicionales
    /// (`pantallas::registrar`), las ventanas pueden distribuirse entre
    /// ellos. El teselado agrupa las ventanas por este indice y tesela cada
    /// grupo en su `Output::region` propia.
    output: usize,
}

/// El escritorio: el registro de todas las ventanas y el modo de teselado.
/// Lo tocan SOLO tareas cooperativas — nunca el manejador de IRQ1.
/// Cuota dura de ventanas concurrentes. Acota los buffers pre-alocados de
/// recomposicion (`capas_buf`, `celdas_buf`) para que `recomponer` no toque
/// jamas al asignador en el camino caliente del compositor. Treinta y dos
/// pestañas cubren con holgura el genesis y los `Alt+N` que un usuario suele
/// engendrar antes de cerrar algo; si alguien la rebasa, el escritorio
/// silenciosamente deja de listar las sobrantes en la barra de tareas, sin
/// alocar a sus espaldas.
const MAX_VENTANAS: usize = 32;

pub(crate) struct Escritorio {
    modo: LayoutMode,
    ancho: usize,
    alto: usize,
    /// Las ventanas, indexadas por `indice_app` — su IDENTIDAD, inmutable.
    ventanas: Vec<Ventana>,
    /// El ORDEN de teselado: `orden[slot]` es el `indice_app` de la ventana que
    /// ocupa esa celda del teselado. Contiene SOLO las ventanas teseladas —las
    /// flotantes salen de aqui—. Separar el orden de la identidad permite
    /// promover y reordenar ventanas sin tocar su `indice_app`.
    orden: Vec<usize>,
    /// Las ventanas FLOTANTES, en orden-Z (Fase 9): de atras hacia adelante.
    /// `flotantes.last()` es la ventana frontal. Una ventana esta en `orden` o
    /// en `flotantes`, jamas en ambos ni en ninguno: juntos son una particion
    /// de `0..ventanas.len()`.
    flotantes: Vec<usize>,
    /// ¿Estaba el boton izquierdo del raton pulsado en el evento anterior?
    /// Para detectar las transiciones —el momento exacto del clic o de soltar—.
    raton_izq: bool,
    /// Arrastre en curso, si lo hay (Fase 13).
    arrastre: Option<Arrastre>,
    /// Buffer pre-alocado de capas de recomposicion. Vive aqui, no en cada
    /// llamada: `recomponer` lo limpia con `clear()` (sin liberar capacidad)
    /// y lo rellena con `push()` dentro de su tope. CERO ALOCACIONES en el
    /// camino caliente del compositor. Capacidad `MAX_VENTANAS`.
    capas_buf: Vec<consola::CapaSlot>,
    /// Igual que `capas_buf` pero para las pestañas de la taskbar. Cubre
    /// como mucho una pestaña por ventana viva.
    celdas_buf: Vec<consola::CeldaTaskbarSlot>,
    /// FASE 58 :: ¿Esta el launcher grafico abierto? Si lo esta, los mandos
    /// del teclado se reinterpretan (foco mueve seleccion, Enter lanza la
    /// app seleccionada) y `recomponer` pinta el overlay sobre la taskbar.
    launcher_abierto: bool,
    /// FASE 58 :: indice de la app SELECCIONADA en el launcher, dentro del
    /// `catalogo`. Acotado en `[0, catalogo.len())` mientras este abierto.
    launcher_seleccion: usize,
    /// FASE 58 :: catalogo de apps lanzables (nombres). Lo fija `fijar_catalogo`
    /// tras armar las plantillas del manifiesto. El indice coincide con el de
    /// `PLANTILLAS` en `main.rs` — el orquestador lo recibe en la cola de
    /// partos por indice y resuelve la plantilla por esa posicion.
    catalogo: Vec<String>,
    /// FASE 58 v3 :: query incremental del launcher — ASCII minuscula, sin
    /// modificadores. Se acumula en `recibir_scancode` via `TextoLauncher`,
    /// se vacia al abrir/cerrar el launcher. Su capacidad esta acotada por
    /// `QUERY_MAX_LEN` para no degenerar.
    launcher_query: String,
    /// FASE 58 v3 :: indices del `catalogo` que matchean la query vigente,
    /// recalculado en cada keystroke. La seleccion del launcher (`Alt+J/K`,
    /// hover, clic) indexa ESTE vector, no el catalogo directamente — asi un
    /// "p" filtra a las apps con esa letra en su nombre y el lanzamiento
    /// resuelve al indice original de plantilla.
    launcher_filtrado: Vec<usize>,
    /// FASE 58 v6 :: mascara de chars matcheados por fila filtrada, paralela
    /// a `launcher_filtrado`. Bit `i` a 1 = el caracter `i` del nombre matcheo
    /// con la query (Spotlight-classic highlight). 64 bits cubren los nombres
    /// del manifiesto con holgura — caracteres mas alla del bit 63 no se
    /// resaltan (degradacion silenciosa, no panic).
    launcher_mascaras: Vec<u64>,
    /// FASE 58 v7 :: primer indice de `launcher_filtrado` visible en el
    /// overlay. La ventana visible es `[launcher_scroll, launcher_scroll +
    /// PICKER_MAX_FILAS)`; las filas fuera de esa ventana no se pintan. Se
    /// reajusta automaticamente cuando la seleccion sale del viewport
    /// (`ajustar_scroll_launcher`), de modo que `Alt+J`/`Alt+K` jamas dejan
    /// al cursor fuera de pantalla. Para 12 apps el scroll queda en 0 toda
    /// la vida util del launcher — es invisible hasta que el catalogo crece.
    launcher_scroll: usize,
}

/// FASE 58 v3 :: mirror atomico de `Escritorio::launcher_abierto` que el
/// manejador de IRQ1 LEE antes de enrutar una tecla — el unico camino para
/// detectar el modo del launcher sin tomar el cerrojo del escritorio en
/// contexto de interrupcion. Lo escribe `launcher_intercepta` cada vez que
/// el estado del overlay cambia, y `atender_raton` cuando el clic-fuera
/// cancela.
pub static LAUNCHER_ABIERTO: AtomicBool = AtomicBool::new(false);

/// FASE 58 v3 :: techo de la query del launcher. Treinta y dos caracteres
/// cubren cualquier nombre realista de app sin abrir la puerta a una query
/// patologicamente larga.
const QUERY_MAX_LEN: usize = 32;

/// El escritorio global. Se funda una sola vez, en el arranque.
static ESCRITORIO: Once<Mutex<Escritorio>> = Once::new();

/// El indice de la ventana ENFOCADA. Atomico —no un campo del `Escritorio`—
/// porque el manejador de IRQ1 lo LEE para enrutar el teclado, y un atomico no
/// se puede disputar: jamas hay interbloqueo entre la IRQ y una tarea.
static FOCO: AtomicUsize = AtomicUsize::new(0);

/// La cola de mandos: el manejador de IRQ1 deposita aqui las ordenes del
/// teclado (lock-free, segura en contexto de interrupcion); la tarea del
/// compositor las drena desde el reactor cooperativo.
static MANDOS: Once<ArrayQueue<Mando>> = Once::new();

/// Cuantos lanzamientos de aplicacion (Fase 10) aguardan. Lo incrementa
/// `atender_mandos` al recibir un `Mando::Lanzar`; lo drena `partos_pendientes`,
/// que lo lee el orquestador del kernel —el unico que sabe instanciar un WASM—.
/// Atomico: el compositor lo escribe, el orquestador lo lee y lo pone a cero.
static PARTOS: AtomicUsize = AtomicUsize::new(0);

/// FASE 58 :: cola de partos DIRIGIDOS — cada `usize` es el indice de la
/// plantilla a instanciar (la N-esima del manifiesto). La rellena el launcher
/// al cerrar con Alt+Enter; la drena el orquestador igual que `PARTOS`. Vive
/// en un `Mutex` y NO en una cola lock-free porque solo se toca desde el tic
/// cooperativo del compositor —jamas desde IRQ—.
static PARTOS_POR_INDICE: Once<Mutex<Vec<usize>>> = Once::new();

/// El ultimo segundo del reloj monotono que la barra de tareas ha mostrado.
/// `tick_reloj` lo compara con el actual: si difiere, recompone para pintar el
/// nuevo. Centinela `u64::MAX` para garantizar que el primer tick fuerza un
/// repintado y la barra arranca con su reloj a 0:00.
static ULTIMO_SEGUNDO: AtomicU64 = AtomicU64::new(u64::MAX);


/// Anchura del buffer del reloj. Cubre "MM:SS" con M de hasta dos digitos
/// y un margen — formato fijo "99:59" cuando los segundos saturan.
const RELOJ_BUFFER_LEN: usize = 8;

// --- Submódulos por cluster. Tipos, statics globales (ESCRITORIO/FOCO/...)
// y consts viven aquí en el root; cada submódulo los ve por la regla de
// visibilidad descendiente vía `use super::*`. Las free-fns públicas se
// re-exportan (API `compositor::X` intacta); las privadas pub(crate). ---
mod ciclo;
mod escenario;
mod geometria;
mod launcher;
mod mando;
pub(crate) mod pata_marco;
mod raton;

pub use ciclo::*;
pub use escenario::*;
pub(crate) use geometria::*;
pub(crate) use launcher::*;
pub(crate) use mando::*;
pub use raton::*;
