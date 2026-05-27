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

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

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
    /// Cerrar la aplicacion enfocada — una baja limpia, en vivo (Fase 10).
    Cerrar,
    /// Lanzar una aplicacion nueva — un alta en vivo (Fase 10).
    Lanzar,
}

/// Un arrastre EN CURSO (Fase 13): el indice de la ventana flotante asida con
/// el raton y el desfase con que se asio —para que la ventana no salte al
/// agarrarla, sino que siga al puntero como si lo llevara cogido por ahi—.
#[derive(Clone, Copy)]
struct Arrastre {
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

struct Escritorio {
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
}

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

/// El ultimo segundo del reloj monotono que la barra de tareas ha mostrado.
/// `tick_reloj` lo compara con el actual: si difiere, recompone para pintar el
/// nuevo. Centinela `u64::MAX` para garantizar que el primer tick fuerza un
/// repintado y la barra arranca con su reloj a 0:00.
static ULTIMO_SEGUNDO: AtomicU64 = AtomicU64::new(u64::MAX);

// =============================================================================
//  Fundacion y consulta — el arranque
// =============================================================================

/// Funda el escritorio: crea una ventana por app, con su marco teselado inicial
/// y su cache de respaldo ya reservada al tamaño natural. `naturales` da el
/// `(ancho, alto)` del lienzo de cada app, en el orden del manifiesto.
pub fn fundar(ancho: usize, alto: usize, naturales: &[(usize, usize, &str)]) {
    MANDOS.call_once(|| ArrayQueue::new(CAPACIDAD_MANDOS));

    let mut ventanas = Vec::with_capacity(naturales.len());
    for &(nat_ancho, nat_alto, nombre) in naturales {
        ventanas.push(Ventana {
            nombre: nombre.to_string(),
            natural_ancho: nat_ancho,
            natural_alto: nat_alto,
            // Marco provisional; `aplicar_teselado` lo fija enseguida.
            marco: RegionPantalla {
                x: 0,
                y: 0,
                ancho: 0,
                alto: 0,
            },
            // La cache: reservada UNA vez, acotada al lienzo natural.
            cache: vec![0u8; nat_ancho.saturating_mul(nat_alto).saturating_mul(4)],
            pintada: false,
            baliza: None,
            cerrada: false,
        });
    }

    // El orden de teselado arranca como la identidad: la ventana `i` ocupa la
    // celda `i`. Ninguna ventana flota al nacer — el escritorio es puro
    // teselado hasta que el teclado lo decida (`Alt+F`).
    let orden = (0..ventanas.len()).collect();
    let mut escritorio = Escritorio {
        modo: MODO_INICIAL,
        ancho,
        alto,
        ventanas,
        orden,
        flotantes: Vec::new(),
        raton_izq: false,
        arrastre: None,
        // Buffers de recomposicion: reservados UNA SOLA VEZ al fundar el
        // escritorio; `recomponer` los reusa con `clear() + push()` sin
        // tocar al asignador. La cota es MAX_VENTANAS — apps por encima
        // se omiten silenciosamente del repintado, no se aloca tras este punto.
        capas_buf: Vec::with_capacity(MAX_VENTANAS),
        celdas_buf: Vec::with_capacity(MAX_VENTANAS),
    };
    aplicar_teselado(&mut escritorio);

    ESCRITORIO.call_once(|| Mutex::new(escritorio));
}

/// Recalcula el teselado y asigna a cada ventana TESELADA su marco. La celda
/// `slot` del teselado va a la ventana `orden[slot]`: manda el orden, no la
/// identidad. Las ventanas flotantes no estan en `orden` y conservan su marco.
fn aplicar_teselado(escritorio: &mut Escritorio) {
    let marcos = teselar(
        escritorio.orden.len(),
        escritorio.ancho,
        escritorio.alto,
        escritorio.modo,
    );
    for (slot, marco) in marcos.into_iter().enumerate() {
        let ventana = escritorio.orden[slot];
        escritorio.ventanas[ventana].marco = marco;
    }
}

/// Pinta el escenario inicial del compositor. Se invoca una vez, tras `fundar`,
/// antes de encender las apps: recompone el escritorio con todas las ventanas
/// aun sin pintar — el teselado se ve como una rejilla de paneles.
pub fn componer_escenario() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    recomponer(&mut escritorio);
}

/// El indice de la ventana enfocada. Lo LEE el manejador de IRQ1 para enrutar
/// cada tecla — por eso es una simple lectura atomica, sin cerrojo alguno.
pub fn foco() -> usize {
    FOCO.load(Ordering::Relaxed)
}

/// Encola un mando del teclado. Lo invoca el manejador de IRQ1: empujar a una
/// cola lock-free es seguro en contexto de interrupcion.
pub fn solicitar(mando: Mando) {
    if let Some(mandos) = MANDOS.get() {
        // Si la cola se desborda, el mando se pierde en silencio: mas vale
        // perder una pulsacion que arriesgar un panico dentro de una IRQ.
        let _ = mandos.push(mando);
    }
}

// =============================================================================
//  El fotograma de una app — cache y composicion
// =============================================================================

/// Recibe el fotograma de la app `indice`: lo copia a su CACHE de respaldo —el
/// kernel asume la persistencia visual— y lo lleva a pantalla. Sin ventanas
/// flotantes ninguna ventana solapa a otra: basta repintar la que cambio —el
/// camino RAPIDO—. Con flotantes vivas el solapamiento obliga a RECOMPONER el
/// escritorio entero, respetando el orden-Z. Lo invoca la capacidad
/// `sys_render_frame` desde el `tick` cooperativo.
pub fn presentar_fotograma(indice: usize, datos: &[u8]) {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    {
        let Some(ventana) = escritorio.ventanas.get_mut(indice) else {
            return;
        };
        // Una ventana cerrada (Fase 10) ya no se pinta: su app pudo emitir un
        // ultimo fotograma antes de que su tarea advirtiera la baja.
        if ventana.cerrada {
            return;
        }
        // Cachear el fotograma. El destino esta acotado al lienzo natural; se
        // copia el minimo de ambas longitudes — jamas se desborda la cache.
        let n = ventana.cache.len().min(datos.len());
        ventana.cache[..n].copy_from_slice(&datos[..n]);
        ventana.pintada = true;
    }

    if escritorio.flotantes.is_empty() {
        // Camino RAPIDO: sin flotantes el escritorio es puro teselado y la app
        // pinta directamente en su marco, como en la Fase 8.
        let ventana = &escritorio.ventanas[indice];
        let marco = ventana.marco;
        let nat_ancho = ventana.natural_ancho;
        let nat_alto = ventana.natural_alto;
        let enfocada = FOCO.load(Ordering::Relaxed) == indice;
        drop(escritorio);
        consola::volcar_marco(marco, nat_ancho, nat_alto, datos, enfocada);
    } else {
        // Hay ventanas flotantes: el solapamiento obliga a recomponer.
        recomponer(&mut escritorio);
    }
}

/// Marca la ventana `indice` como desalojada y tatua su marco con la baliza.
/// Desde aqui queda excluida del foco — el teclado la salta.
pub fn desalojar(indice: usize, color: Color) {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    {
        let Some(ventana) = escritorio.ventanas.get_mut(indice) else {
            return;
        };
        // Una ventana ya cerrada (Fase 10) no recibe baliza: la baja limpia
        // gana a un desalojo que llegue tarde, en la misma vuelta.
        if ventana.cerrada {
            return;
        }
        ventana.baliza = Some(color);
    }
    // Fase 15: la voz del kernel anuncia el desalojo.
    crate::drivers::altavoz::agendar(&crate::drivers::altavoz::VOZ_DESALOJO);

    if escritorio.flotantes.is_empty() {
        let marco = escritorio.ventanas[indice].marco;
        let enfocada = FOCO.load(Ordering::Relaxed) == indice;
        drop(escritorio);
        consola::pintar_desalojo(marco, color, enfocada);
    } else {
        recomponer(&mut escritorio);
    }
}

// =============================================================================
//  Los mandos del teclado — el escritorio interactivo
// =============================================================================

/// Atiende los mandos pendientes del teclado. La invoca la tarea del compositor
/// en cada fotograma, desde el reactor cooperativo — el unico contexto donde es
/// seguro bloquear el `ESCRITORIO` y la consola.
pub fn atender_mandos() {
    let Some(mandos) = MANDOS.get() else {
        return;
    };
    while let Some(mando) = mandos.pop() {
        match mando {
            Mando::CiclarLayout => ciclar_layout(),
            Mando::FocoSiguiente => mover_foco(true),
            Mando::FocoAnterior => mover_foco(false),
            Mando::Promover => promover(),
            Mando::MoverAdelante => mover_ventana(true),
            Mando::MoverAtras => mover_ventana(false),
            Mando::Flotar => flotar(),
            Mando::Cerrar => cerrar(),
            // El alta de una app necesita instanciar un WASM — algo que el
            // compositor no sabe hacer—. Solo se cuenta la peticion; el
            // orquestador del kernel la atendera (ver `partos_pendientes`).
            Mando::Lanzar => {
                PARTOS.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Cicla al siguiente modo de teselado: recalcula los marcos de las ventanas
/// teseladas y recompone el escritorio entero desde las caches de respaldo.
fn ciclar_layout() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    escritorio.modo = escritorio.modo.next();
    aplicar_teselado(&mut escritorio);
    recomponer(&mut escritorio);
}

/// Mueve el foco a la siguiente ventana VIVA. El recorrido abarca TODAS las
/// ventanas —las teseladas y, tras ellas, las flotantes— saltando las
/// desalojadas. Si la ventana recien enfocada flota, sube al frente del
/// orden-Z: la flotante con el foco esta SIEMPRE delante.
fn mover_foco(adelante: bool) {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    // El recorrido del foco: las teseladas, luego las flotantes — un orden
    // estable y visualmente coherente.
    let recorrido: Vec<usize> = escritorio
        .orden
        .iter()
        .chain(escritorio.flotantes.iter())
        .copied()
        .collect();
    let n = recorrido.len();
    if n == 0 {
        return;
    }
    let anterior = FOCO.load(Ordering::Relaxed);
    let pos = recorrido.iter().position(|&v| v == anterior).unwrap_or(0);

    // Avanzar saltando las ventanas desalojadas. Si no hay ninguna viva, tras
    // `n` pasos se vuelve al punto de partida y el foco no cambia.
    let mut nueva_pos = pos;
    let mut nuevo = anterior;
    for _ in 0..n {
        nueva_pos = if adelante {
            (nueva_pos + 1) % n
        } else {
            (nueva_pos + n - 1) % n
        };
        let candidata = recorrido[nueva_pos];
        if escritorio.ventanas[candidata].baliza.is_none() {
            nuevo = candidata;
            break;
        }
    }
    FOCO.store(nuevo, Ordering::Relaxed);
    // La bocina pertenece a la ventana enfocada (Fase 12): al cambiar el foco,
    // callar — la nueva dueña la reclamara en su proximo fotograma si quiere.
    crate::drivers::altavoz::tono(0);
    // La ventana recien enfocada, si flota, al frente del orden-Z.
    alzar_si_flota(&mut escritorio, nuevo);
    recomponer(&mut escritorio);
}

/// Promueve la ventana enfocada a la posicion maestra —la celda 0— del
/// teselado. Si la ventana enfocada flota, no esta en el orden de teselado y
/// el mando no hace nada — promover es una operacion del teselado.
fn promover() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    let foco = FOCO.load(Ordering::Relaxed);
    if let Some(pos) = escritorio.orden.iter().position(|&v| v == foco) {
        let ventana = escritorio.orden.remove(pos);
        escritorio.orden.insert(0, ventana);
        aplicar_teselado(&mut escritorio);
        recomponer(&mut escritorio);
    }
}

/// Mueve la ventana enfocada una posicion en el orden de teselado,
/// intercambiandola con su vecina. Una ventana flotante no esta en el orden:
/// el mando no la afecta.
fn mover_ventana(adelante: bool) {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    let n = escritorio.orden.len();
    if n < 2 {
        return;
    }
    let foco = FOCO.load(Ordering::Relaxed);
    if let Some(pos) = escritorio.orden.iter().position(|&v| v == foco) {
        let destino = if adelante {
            (pos + 1) % n
        } else {
            (pos + n - 1) % n
        };
        escritorio.orden.swap(pos, destino);
        aplicar_teselado(&mut escritorio);
        recomponer(&mut escritorio);
    }
}

// =============================================================================
//  FASE 9 — orden-Z y ventanas flotantes
// =============================================================================

/// Alterna la ventana enfocada entre TESELADA y FLOTANTE. Al flotar, la ventana
/// abandona el teselado —que se recalcula para las que quedan—, recibe un marco
/// propio en cascada y sube al frente del orden-Z. Al volver al teselado, se
/// reincorpora al final del orden. El foco no cambia: viaja con la ventana.
fn flotar() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    let foco = FOCO.load(Ordering::Relaxed);

    if let Some(pos) = escritorio.orden.iter().position(|&v| v == foco) {
        // Teselada -> flotante: se desliga del teselado, recibe su marco
        // propio en cascada y sube al frente del orden-Z.
        escritorio.orden.remove(pos);
        let marco = marco_flotante(&escritorio, foco);
        escritorio.ventanas[foco].marco = marco;
        escritorio.flotantes.push(foco);
        aplicar_teselado(&mut escritorio);
        recomponer(&mut escritorio);
    } else if let Some(pos) = escritorio.flotantes.iter().position(|&v| v == foco) {
        // Flotante -> teselada: vuelve a la rejilla, al final del orden.
        escritorio.flotantes.remove(pos);
        escritorio.orden.push(foco);
        aplicar_teselado(&mut escritorio);
        recomponer(&mut escritorio);
    }
}

/// Si la ventana `indice` es flotante, la lleva al frente del orden-Z —al final
/// de `flotantes`—. Si esta teselada, no hace nada.
fn alzar_si_flota(escritorio: &mut Escritorio, indice: usize) {
    if let Some(pos) = escritorio.flotantes.iter().position(|&v| v == indice) {
        let ventana = escritorio.flotantes.remove(pos);
        escritorio.flotantes.push(ventana);
    }
}

/// El marco de una ventana recien hecha flotante: su lienzo natural mas un
/// reborde de cromo, colocado en cascada —para que varias flotantes no se
/// tapen del todo— y acotado al area de apps. Se invoca ANTES de inscribir la
/// ventana en `flotantes`: su longitud da el escalon de la cascada.
fn marco_flotante(escritorio: &Escritorio, indice: usize) -> RegionPantalla {
    let area = area_apps(escritorio.ancho, escritorio.alto);
    let ventana = &escritorio.ventanas[indice];
    let ancho = (ventana.natural_ancho + 2 * CROMO_FLOTANTE).min(area.ancho);
    let alto = (ventana.natural_alto + 2 * CROMO_FLOTANTE).min(area.alto);

    // La cascada: un escalon por cada flotante ya existente.
    let escalon = escritorio.flotantes.len().saturating_mul(PASO_CASCADA);
    let mut x = area.x + 48 + escalon;
    let mut y = area.y + 40 + escalon;
    // Acotar: la ventana entera ha de caber dentro del area de apps.
    if x + ancho > area.x + area.ancho {
        x = area.x + area.ancho.saturating_sub(ancho);
    }
    if y + alto > area.y + area.alto {
        y = area.y + area.alto.saturating_sub(alto);
    }
    RegionPantalla {
        x,
        y,
        ancho,
        alto,
    }
}

/// Recompone el escritorio entero respetando el orden-Z. Arma la lista de capas
/// —primero las ventanas TESELADAS, la capa de fondo; despues las FLOTANTES, de
/// atras hacia adelante— y se la entrega a la consola, que las funde en ese
/// orden de una sola pasada. El solapamiento se resuelve por el orden del
/// pintado. La invocan los mandos del teclado y `presentar_fotograma` cuando
/// hay flotantes vivas. El llamante sostiene ya el cerrojo del `ESCRITORIO`.
fn recomponer(escritorio: &mut Escritorio) {
    let area = area_apps(escritorio.ancho, escritorio.alto);
    let foco = FOCO.load(Ordering::Relaxed);

    // --- BUFFER de capas reusado. clear() no libera capacidad; push() abajo
    //     se mantiene dentro de la capacity reservada al fundar (no toca al
    //     asignador). Si MAX_VENTANAS se queda corto, el `take` lo acota sin
    //     panico — las apps extras quedan sin recomponer este fotograma. ---
    escritorio.capas_buf.clear();
    for &indice in escritorio
        .orden
        .iter()
        .chain(escritorio.flotantes.iter())
        .take(MAX_VENTANAS)
    {
        let ventana = &escritorio.ventanas[indice];
        let contenido = match ventana.baliza {
            Some(color) => consola::ContenidoSlot::Baliza(color),
            None if ventana.pintada => consola::ContenidoSlot::Fotograma(indice),
            None => consola::ContenidoSlot::Panel,
        };
        escritorio.capas_buf.push(consola::CapaSlot {
            marco: ventana.marco,
            nat_ancho: ventana.natural_ancho,
            nat_alto: ventana.natural_alto,
            contenido,
            enfocada: indice == foco,
        });
    }

    // --- FASE 14/16 :: armar la barra de tareas. El mismo trato: clear() +
    //     push() sobre el buffer pre-alocado de celdas. ---
    let area_bar = area_taskbar(escritorio.ancho, escritorio.alto);
    let cy = area_bar.y + 4;
    let calto = area_bar.alto.saturating_sub(8);
    let launcher = RegionPantalla {
        x: area_bar.x + CELDA_TASKBAR_MARGEN,
        y: cy,
        ancho: LAUNCHER_ANCHO,
        alto: calto,
    };
    let cells_x0 = launcher.x + launcher.ancho + LAUNCHER_HUECO;
    let cells_x_max =
        area_bar.x + area_bar.ancho - CELDA_TASKBAR_MARGEN - RELOJ_ANCHO - RELOJ_HUECO;
    escritorio.celdas_buf.clear();
    let mut cx = cells_x0;
    for (indice, ventana) in escritorio
        .ventanas
        .iter()
        .enumerate()
        .take(MAX_VENTANAS)
    {
        if ventana.cerrada {
            continue;
        }
        if cx + CELDA_TASKBAR_ANCHO > cells_x_max {
            break;
        }
        let fondo = match ventana.baliza {
            Some(color) => color,
            None if indice == foco => Color::FOCO,
            None => Color::PANEL,
        };
        escritorio.celdas_buf.push(consola::CeldaTaskbarSlot {
            region: RegionPantalla {
                x: cx,
                y: cy,
                ancho: CELDA_TASKBAR_ANCHO,
                alto: calto,
            },
            ventana: indice,
            fondo,
            tinta: tinta_para(fondo),
        });
        cx += CELDA_TASKBAR_ANCHO + CELDA_TASKBAR_HUECO;
    }

    // --- Reloj formateado SOBRE PILA. Reemplaza `alloc::format!("{}:{:02}", ...)`
    //     por escritura en un `[u8; 8]` mediante un `core::fmt::Write` minimo.
    //     Cero allocaciones. El segundero cubre hasta 99:59 (~6000 segundos);
    //     a partir de ahi se acota a "99:59" — el escritorio se reinicia
    //     antes en cualquier escenario realista. ---
    let ms = crate::async_system::reloj::milisegundos();
    let segs = ms / 1000;
    let mut reloj_buf = [0u8; RELOJ_BUFFER_LEN];
    let reloj_len = formatear_reloj(&mut reloj_buf, segs);
    // SEGURIDAD: `formatear_reloj` escribe SOLO ASCII (digitos y ':'),
    // garantizando un &str valido sin pasar por `from_utf8`.
    let reloj_texto =
        unsafe { core::str::from_utf8_unchecked(&reloj_buf[..reloj_len]) };

    let reloj_region = RegionPantalla {
        x: area_bar.x + area_bar.ancho - CELDA_TASKBAR_MARGEN - RELOJ_ANCHO,
        y: cy,
        ancho: RELOJ_ANCHO,
        alto: calto,
    };
    let taskbar = consola::TaskbarSlot {
        area: area_bar,
        launcher,
        celdas: &escritorio.celdas_buf,
        reloj: reloj_texto,
        reloj_region,
    };
    let resolver = ResolverEscritorio {
        ventanas: &escritorio.ventanas,
    };
    consola::recomponer(area, &escritorio.capas_buf, &taskbar, &resolver);
    // Recordar el segundo recien mostrado: `tick_reloj` evita repintar de mas
    // mientras dure este mismo segundo.
    ULTIMO_SEGUNDO.store(segs, Ordering::Relaxed);
}

/// Resolver concreto del compositor para la consola: traduce un indice de
/// ventana a su cache de fotograma y a su nombre legible. Se construye en
/// la pila justo antes de invocar `consola::recomponer` — su lifetime no
/// se extiende mas alla del cerrojo del escritorio.
struct ResolverEscritorio<'a> {
    ventanas: &'a [Ventana],
}

impl<'a> consola::Resolver for ResolverEscritorio<'a> {
    fn cache(&self, indice: usize) -> &[u8] {
        &self.ventanas[indice].cache
    }
    fn nombre(&self, indice: usize) -> &str {
        &self.ventanas[indice].nombre
    }
}

/// Anchura del buffer del reloj. Cubre "MM:SS" con M de hasta dos digitos
/// y un margen — formato fijo "99:59" cuando los segundos saturan.
const RELOJ_BUFFER_LEN: usize = 8;

/// Escribe "M:SS" o "MM:SS" en `dst` y devuelve la longitud. Sin alocacion,
/// sin `core::fmt::Write`: un formateador ad-hoc para el reloj de la
/// taskbar. Acota los minutos a 99 — un disclaimer barato para no
/// engordar el formato y evitar tener que reanclar buffer en runtime.
fn formatear_reloj(dst: &mut [u8; RELOJ_BUFFER_LEN], segs: u64) -> usize {
    let mut min = segs / 60;
    let sec = (segs % 60) as u8;
    if min > 99 {
        min = 99;
    }
    let min = min as u8;
    let mut n = 0usize;
    if min >= 10 {
        dst[n] = b'0' + (min / 10);
        n += 1;
    }
    dst[n] = b'0' + (min % 10);
    n += 1;
    dst[n] = b':';
    n += 1;
    dst[n] = b'0' + (sec / 10);
    n += 1;
    dst[n] = b'0' + (sec % 10);
    n += 1;
    n
}

/// Localiza la celda de la barra de tareas bajo la coordenada x: itera las
/// ventanas vivas en orden de creacion y devuelve la N-esima donde la N es la
/// posicion en la barra. `None` si el clic cae en el lanzador, en el reloj, en
/// un hueco entre celdas, o fuera del rango de las pestañas.
fn celda_taskbar_en(escritorio: &Escritorio, x: usize) -> Option<usize> {
    let area_bar = area_taskbar(escritorio.ancho, escritorio.alto);
    // Las pestañas empiezan despues del lanzador.
    let cells_x0 = area_bar.x + CELDA_TASKBAR_MARGEN + LAUNCHER_ANCHO + LAUNCHER_HUECO;
    let cells_x_max =
        area_bar.x + area_bar.ancho - CELDA_TASKBAR_MARGEN - RELOJ_ANCHO - RELOJ_HUECO;
    if x < cells_x0 || x >= cells_x_max {
        return None;
    }
    let rel = x - cells_x0;
    let paso = CELDA_TASKBAR_ANCHO + CELDA_TASKBAR_HUECO;
    let posicion = rel / paso;
    let offset = rel % paso;
    if offset >= CELDA_TASKBAR_ANCHO {
        return None;
    }
    let mut k = 0;
    for (indice, ventana) in escritorio.ventanas.iter().enumerate() {
        if ventana.cerrada {
            continue;
        }
        if k == posicion {
            return Some(indice);
        }
        k += 1;
    }
    None
}

/// ¿Cae la coordenada x en el boton lanzador («+»)?
fn clic_en_launcher(escritorio: &Escritorio, x: usize) -> bool {
    let area_bar = area_taskbar(escritorio.ancho, escritorio.alto);
    let x0 = area_bar.x + CELDA_TASKBAR_MARGEN;
    let x1 = x0 + LAUNCHER_ANCHO;
    x >= x0 && x < x1
}

// =============================================================================
//  FASE 10 — alta y baja de aplicaciones en vivo
// =============================================================================

/// Cierra la aplicacion enfocada (`Alt+Q`): una baja LIMPIA, distinta del
/// desalojo por falla. Marca la ventana como cerrada, libera su cache de
/// respaldo, la saca del teselado y del orden-Z, y traslada el foco a una
/// ventana viva contigua. La app, en su tarea, advertira la baja y concluira.
fn cerrar() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    let foco = FOCO.load(Ordering::Relaxed);
    // Solo se cierra una ventana viva. El foco jamas se posa en una muerta o
    // cerrada, pero la guarda lo deja explicito.
    match escritorio.ventanas.get(foco) {
        Some(v) if v.baliza.is_none() && !v.cerrada => {}
        _ => return,
    }
    // Fase 15: el kernel se despide de la app con un repique descendente.
    crate::drivers::altavoz::agendar(&crate::drivers::altavoz::VOZ_CERRAR);
    // Marcar la baja y liberar el respaldo: la cache de un fotograma puede
    // pesar un megabyte — no tiene sentido retenerla en una ranura inerte.
    let ventana = &mut escritorio.ventanas[foco];
    ventana.cerrada = true;
    ventana.pintada = false;
    ventana.cache = Vec::new();
    // Sacarla del teselado y del orden-Z. El censo conserva la ranura —los
    // indices son la identidad, jamas se reciclan—, pero ya nadie la dibuja.
    escritorio.orden.retain(|&v| v != foco);
    escritorio.flotantes.retain(|&v| v != foco);
    // Si la estabamos arrastrando con el raton (Fase 13), soltarla.
    if escritorio.arrastre.map(|a| a.ventana) == Some(foco) {
        escritorio.arrastre = None;
    }
    // El foco salta a la primera ventana viva que quede; si no queda ninguna,
    // se queda donde estaba —inofensivo: no hay a quien enrutar el teclado—.
    let nuevo = escritorio
        .orden
        .iter()
        .chain(escritorio.flotantes.iter())
        .copied()
        .find(|&v| {
            let w = &escritorio.ventanas[v];
            w.baliza.is_none() && !w.cerrada
        })
        .unwrap_or(foco);
    FOCO.store(nuevo, Ordering::Relaxed);
    // El foco cambia: callar la bocina (Fase 12) — ver `mover_foco`.
    crate::drivers::altavoz::tono(0);
    alzar_si_flota(&mut escritorio, nuevo);
    aplicar_teselado(&mut escritorio);
    recomponer(&mut escritorio);
}

/// Da de alta una ventana NUEVA y devuelve su indice —su identidad—. La crea
/// con su cache de respaldo al tamaño natural, la añade al final del orden de
/// teselado, recalcula el teselado y recompone. La invoca el orquestador del
/// kernel justo antes de instanciar el WASM de la app, que necesita ese indice.
pub fn nacer_ventana(nat_ancho: usize, nat_alto: usize, nombre: &str) -> usize {
    let Some(escritorio) = ESCRITORIO.get() else {
        return 0;
    };
    let mut escritorio = escritorio.lock();
    let indice = escritorio.ventanas.len();
    escritorio.ventanas.push(Ventana {
        nombre: nombre.to_string(),
        natural_ancho: nat_ancho,
        natural_alto: nat_alto,
        marco: RegionPantalla {
            x: 0,
            y: 0,
            ancho: 0,
            alto: 0,
        },
        cache: vec![0u8; nat_ancho.saturating_mul(nat_alto).saturating_mul(4)],
        pintada: false,
        baliza: None,
        cerrada: false,
    });
    escritorio.orden.push(indice);
    aplicar_teselado(&mut escritorio);
    recomponer(&mut escritorio);
    // Fase 15: el kernel saluda al nacimiento con un repique ascendente.
    crate::drivers::altavoz::agendar(&crate::drivers::altavoz::VOZ_LANZAR);
    indice
}

/// ¿Se ha pedido cerrar la ventana `indice`? Cada app la consulta en su tarea,
/// fotograma a fotograma: cuando es `true`, concluye su tarea y se libera. Una
/// ventana inexistente cuenta como cerrada.
pub fn ventana_cerrada(indice: usize) -> bool {
    let Some(escritorio) = ESCRITORIO.get() else {
        return false;
    };
    escritorio
        .lock()
        .ventanas
        .get(indice)
        .map(|ventana| ventana.cerrada)
        .unwrap_or(true)
}

/// Cuantas aplicaciones nuevas se han pedido lanzar desde la ultima consulta —y
/// pone el contador a cero—. La invoca el orquestador del kernel —el unico que
/// sabe instanciar un WASM— en cada fotograma de la tarea del compositor.
pub fn partos_pendientes() -> usize {
    PARTOS.swap(0, Ordering::Relaxed)
}

/// Avanza el reloj de la barra de tareas (Fase 16): si el segundo del reloj
/// monotono cambio respecto al ultimo mostrado, recompone para refrescar la
/// pantalla. Si el segundo es el mismo, vuelve sin hacer nada — un fotograma
/// barato—. La invoca la tarea del compositor cada fotograma.
pub fn tick_reloj() {
    let actual = crate::async_system::reloj::milisegundos() / 1000;
    if ULTIMO_SEGUNDO.load(Ordering::Relaxed) == actual {
        return;
    }
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    recomponer(&mut escritorio);
}

// =============================================================================
//  FASE 13 — raton, puntero y arrastre de ventanas flotantes
// =============================================================================

/// La ultima posicion del puntero que el compositor REFRESCO. Si la posicion
/// actual del raton coincide con esta, no hay nada nuevo que estampar; si
/// difiere, la consola debe volver a presentar. Empacada como `y * 65536 + x`,
/// con `usize::MAX` como centinela de «aun no he visto al raton».
static PUNTERO_REFRESCADO: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Drena los eventos del raton y los aplica: clic enfoca la ventana bajo el
/// puntero (y, si flota, inicia un arrastre); el boton sostenido la arrastra;
/// soltarlo termina el gesto. La invoca la tarea del compositor en cada
/// fotograma, desde el reactor cooperativo.
pub fn atender_raton() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    let mut cambio = false;
    while let Some(evento) = crate::drivers::raton::siguiente_evento() {
        let izq = evento.botones & 0b001 != 0;
        let x = evento.x as usize;
        let y = evento.y as usize;
        let izq_antes = escritorio.raton_izq;
        if izq && !izq_antes {
            // Boton bajó: un CLIC. Si cae en la barra de tareas, enfocar la
            // pestaña pulsada SIN iniciar arrastre. Si no, comportamiento
            // habitual: enfocar la ventana topmost bajo el puntero.
            let area_bar = area_taskbar(escritorio.ancho, escritorio.alto);
            if y >= area_bar.y && y < area_bar.y + area_bar.alto {
                if clic_en_launcher(&escritorio, x) {
                    // El boton «+» equivale a `Alt+N`: solicita un parto. La
                    // tarea del compositor lo recogera en su proxima vuelta.
                    PARTOS.fetch_add(1, Ordering::Relaxed);
                } else if let Some(v) = celda_taskbar_en(&escritorio, x) {
                    let viva = {
                        let w = &escritorio.ventanas[v];
                        w.baliza.is_none() && !w.cerrada
                    };
                    if viva {
                        FOCO.store(v, Ordering::Relaxed);
                        crate::drivers::altavoz::tono(0);
                        alzar_si_flota(&mut escritorio, v);
                        cambio = true;
                    }
                }
            } else if let Some(v) = ventana_en(&escritorio, x, y) {
                let viva = {
                    let w = &escritorio.ventanas[v];
                    w.baliza.is_none() && !w.cerrada
                };
                if viva {
                    // Enfocar como hace `mover_foco`: foco + bocina muda + alza
                    // si flota.
                    FOCO.store(v, Ordering::Relaxed);
                    crate::drivers::altavoz::tono(0);
                    alzar_si_flota(&mut escritorio, v);
                    // Si la ventana flota, empezar a arrastrarla.
                    if escritorio.flotantes.contains(&v) {
                        let marco = escritorio.ventanas[v].marco;
                        escritorio.arrastre = Some(Arrastre {
                            ventana: v,
                            agarre_dx: x.saturating_sub(marco.x),
                            agarre_dy: y.saturating_sub(marco.y),
                        });
                    }
                    cambio = true;
                }
            }
        } else if izq && izq_antes {
            // Boton sostenido: arrastrar la ventana asida, si la hay.
            if let Some(arr) = escritorio.arrastre {
                mover_arrastrada(&mut escritorio, arr, x, y);
                cambio = true;
            }
        } else if !izq && izq_antes {
            // Boton subió: fin del arrastre.
            escritorio.arrastre = None;
        }
        escritorio.raton_izq = izq;

        // ENRUTADO AL USERSPACE. Despues de aplicar foco y arrastre, entregar
        // el evento ya TRADUCIDO al canal del puntero de la ventana enfocada.
        // El kernel hace toda la matematica: la app no ve coordenadas globales
        // ni eventos que caigan fuera de su lienzo natural. Si el cursor esta
        // sobre el cromo de su propia ventana o sobre otras ventanas, el
        // evento se descarta en silencio dentro de `puntero::enrutar`.
        let foco = FOCO.load(Ordering::Relaxed);
        if let Some(ventana) = escritorio.ventanas.get(foco) {
            if !ventana.cerrada && ventana.baliza.is_none() {
                let marco = ventana.marco;
                let nat_ancho = ventana.natural_ancho;
                let nat_alto = ventana.natural_alto;
                crate::async_system::puntero::enrutar(
                    foco,
                    x,
                    y,
                    evento.botones,
                    marco.x,
                    marco.y,
                    marco.ancho,
                    marco.alto,
                    nat_ancho,
                    nat_alto,
                );
            }
        }
    }
    if cambio {
        recomponer(&mut escritorio);
        // El recomponer ya presento; sincronizar el centinela para no presentar
        // dos veces en la misma vuelta.
        PUNTERO_REFRESCADO.store(empacar_puntero(), Ordering::Relaxed);
    }
}

/// La ventana topmost que contiene el punto (x, y), si la hay. Recorre el
/// orden-Z de delante hacia atras: primero las flotantes (la ultima es la
/// frontal), despues las teseladas.
fn ventana_en(escritorio: &Escritorio, x: usize, y: usize) -> Option<usize> {
    for &v in escritorio.flotantes.iter().rev() {
        if contiene(escritorio.ventanas[v].marco, x, y) {
            return Some(v);
        }
    }
    for &v in escritorio.orden.iter().rev() {
        if contiene(escritorio.ventanas[v].marco, x, y) {
            return Some(v);
        }
    }
    None
}

/// ¿Contiene el marco al punto (x, y)?
fn contiene(marco: RegionPantalla, x: usize, y: usize) -> bool {
    x >= marco.x && x < marco.x + marco.ancho && y >= marco.y && y < marco.y + marco.alto
}

/// Mueve la ventana arrastrada de modo que el punto del puntero —la asa— siga
/// estando, dentro de la ventana, donde se asio. La ventana queda acotada al
/// area de apps.
fn mover_arrastrada(escritorio: &mut Escritorio, arr: Arrastre, x: usize, y: usize) {
    let area = area_apps(escritorio.ancho, escritorio.alto);
    let Some(ventana) = escritorio.ventanas.get_mut(arr.ventana) else {
        return;
    };
    let ancho = ventana.marco.ancho;
    let alto = ventana.marco.alto;
    let mut nx = x.saturating_sub(arr.agarre_dx);
    let mut ny = y.saturating_sub(arr.agarre_dy);
    // Acotar al area de apps: la ventana entera ha de caber dentro.
    if nx + ancho > area.x + area.ancho {
        nx = (area.x + area.ancho).saturating_sub(ancho);
    }
    if ny + alto > area.y + area.alto {
        ny = (area.y + area.alto).saturating_sub(alto);
    }
    nx = nx.max(area.x);
    ny = ny.max(area.y);
    ventana.marco.x = nx;
    ventana.marco.y = ny;
}

/// Empaca la posicion actual del puntero en un solo `usize` —`y * 65536 + x`—
/// para compararla atomicamente con la ultima refrescada. `usize::MAX` indica
/// «el raton no esta vivo».
fn empacar_puntero() -> usize {
    match crate::drivers::raton::posicion() {
        Some((x, y)) => (y << 16) | (x & 0xFFFF),
        None => usize::MAX,
    }
}

/// Si el puntero se ha movido desde la ultima presentacion del compositor, le
/// pide a la consola un volcado fresco —para reestampar el puntero en su
/// nuevo lugar—. La invoca la tarea del compositor cada fotograma.
pub fn refrescar_puntero() {
    let actual = empacar_puntero();
    if actual == usize::MAX {
        return;
    }
    if PUNTERO_REFRESCADO.swap(actual, Ordering::Relaxed) != actual {
        crate::consola::refrescar();
    }
}

// =============================================================================
//  Teselado — la geometria pura de `mirada-layout`
// =============================================================================

/// El area de pantalla que el compositor tesela: toda la pantalla menos la
/// franja de la consola en la cima y la barra de tareas al pie.
pub fn area_apps(ancho_pantalla: usize, alto_pantalla: usize) -> RegionPantalla {
    let cabeza = FRANJA_CONSOLA.min(alto_pantalla);
    let pie = FRANJA_TASKBAR.min(alto_pantalla.saturating_sub(cabeza));
    RegionPantalla {
        x: 0,
        y: cabeza,
        ancho: ancho_pantalla,
        alto: alto_pantalla.saturating_sub(cabeza).saturating_sub(pie),
    }
}

/// El area de la barra de tareas: una franja al pie de la pantalla.
fn area_taskbar(ancho_pantalla: usize, alto_pantalla: usize) -> RegionPantalla {
    let pie = FRANJA_TASKBAR.min(alto_pantalla);
    RegionPantalla {
        x: 0,
        y: alto_pantalla.saturating_sub(pie),
        ancho: ancho_pantalla,
        alto: pie,
    }
}

/// El color de tinta —oscuro o claro— que da contraste legible sobre `fondo`.
/// Sin esto, la pestaña amarilla palida del desalojo por memoria quedaba con
/// texto blanco sobre crema: ilegible. La regla de luminancia ITU-R BT.601 fija
/// el umbral: fondos claros llevan tinta oscura, fondos oscuros la clara.
fn tinta_para(fondo: Color) -> Color {
    let brillo =
        (fondo.r as u32 * 299 + fondo.g as u32 * 587 + fondo.b as u32 * 114) / 1000;
    if brillo > 160 {
        // Fondo claro: tinta del reposo del lienzo, casi negra.
        Color::LIENZO_EN_REPOSO
    } else {
        Color::TEXTO
    }
}

/// Tesela el area de apps en `n` marcos con el modo dado. El vector resultante
/// tiene exactamente `n` elementos, en el orden de las celdas del teselado.
fn teselar(n: usize, ancho: usize, alto: usize, modo: LayoutMode) -> Vec<RegionPantalla> {
    let area = area_apps(ancho, alto);
    let pantalla = Rect::new(
        area.x as i32,
        area.y as i32,
        area.ancho as i32,
        area.alto as i32,
    );
    let params = LayoutParams {
        mode: modo,
        gap: MARGEN,
        ..LayoutParams::default()
    };
    tile(pantalla, n, &params)
        .into_iter()
        .map(rect_a_region)
        .collect()
}

/// Traduce un `Rect` de `mirada-layout` (`i32`) a la `RegionPantalla` del
/// kernel (`usize`). Un rectangulo degenerado queda en cero.
fn rect_a_region(r: Rect) -> RegionPantalla {
    RegionPantalla {
        x: r.x.max(0) as usize,
        y: r.y.max(0) as usize,
        ancho: r.w.max(0) as usize,
        alto: r.h.max(0) as usize,
    }
}
