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
//  EXCLUSION DE INTERRUPCIONES. El `ESCRITORIO` lo tocan SOLO tareas
//  cooperativas (el `tick` de una app, la tarea del compositor): el manejador
//  de IRQ1 jamas lo bloquea. La IRQ se comunica con el mundo cooperativo por
//  un canal estrecho y a prueba de interbloqueos: dos atomicos —el foco y el
//  estado de Alt— y una cola lock-free de mandos. Ningun cerrojo que la IRQ
//  pudiera disputar a una tarea cooperativa.
// =============================================================================

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::vec;
use alloc::vec::Vec;

use crossbeam_queue::ArrayQueue;
use mirada_layout::{tile, LayoutMode, LayoutParams, Rect};
use spin::{Mutex, Once};

use crate::consola::{self, Capa, Contenido};
use crate::grafico::{Color, RegionPantalla};

/// Altura del strip superior reservado a la consola; las apps teselan debajo.
/// La consola conserva ahi su registro de arranque completo —seis lineas,
/// hasta la sonda asincrona de disco— legible sobre el teselado.
const FRANJA_CONSOLA: usize = 296;

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
}

/// Una ventana del escritorio: una app, su geometria y su ultimo fotograma.
struct Ventana {
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
}

/// El escritorio: el registro de todas las ventanas y el modo de teselado.
/// Lo tocan SOLO tareas cooperativas — nunca el manejador de IRQ1.
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

// =============================================================================
//  Fundacion y consulta — el arranque
// =============================================================================

/// Funda el escritorio: crea una ventana por app, con su marco teselado inicial
/// y su cache de respaldo ya reservada al tamaño natural. `naturales` da el
/// `(ancho, alto)` del lienzo de cada app, en el orden del manifiesto.
pub fn fundar(ancho: usize, alto: usize, naturales: &[(usize, usize)]) {
    MANDOS.call_once(|| ArrayQueue::new(CAPACIDAD_MANDOS));

    let mut ventanas = Vec::with_capacity(naturales.len());
    for &(nat_ancho, nat_alto) in naturales {
        ventanas.push(Ventana {
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
    let escritorio = escritorio.lock();
    recomponer(&escritorio);
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
        recomponer(&escritorio);
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
        ventana.baliza = Some(color);
    }

    if escritorio.flotantes.is_empty() {
        let marco = escritorio.ventanas[indice].marco;
        let enfocada = FOCO.load(Ordering::Relaxed) == indice;
        drop(escritorio);
        consola::pintar_desalojo(marco, color, enfocada);
    } else {
        recomponer(&escritorio);
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
    recomponer(&escritorio);
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
    // La ventana recien enfocada, si flota, al frente del orden-Z.
    alzar_si_flota(&mut escritorio, nuevo);
    recomponer(&escritorio);
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
        recomponer(&escritorio);
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
        recomponer(&escritorio);
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
        recomponer(&escritorio);
    } else if let Some(pos) = escritorio.flotantes.iter().position(|&v| v == foco) {
        // Flotante -> teselada: vuelve a la rejilla, al final del orden.
        escritorio.flotantes.remove(pos);
        escritorio.orden.push(foco);
        aplicar_teselado(&mut escritorio);
        recomponer(&escritorio);
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
fn recomponer(escritorio: &Escritorio) {
    let area = area_apps(escritorio.ancho, escritorio.alto);
    let foco = FOCO.load(Ordering::Relaxed);
    let mut capas: Vec<Capa> = Vec::with_capacity(escritorio.ventanas.len());
    for &indice in escritorio.orden.iter().chain(escritorio.flotantes.iter()) {
        let ventana = &escritorio.ventanas[indice];
        let contenido = match ventana.baliza {
            Some(color) => Contenido::Baliza(color),
            None if ventana.pintada => Contenido::Fotograma(&ventana.cache),
            None => Contenido::Panel,
        };
        capas.push(Capa {
            marco: ventana.marco,
            nat_ancho: ventana.natural_ancho,
            nat_alto: ventana.natural_alto,
            contenido,
            enfocada: indice == foco,
        });
    }
    consola::recomponer(area, &capas);
}

// =============================================================================
//  Teselado — la geometria pura de `mirada-layout`
// =============================================================================

/// El area de pantalla que el compositor tesela: toda la pantalla menos la
/// franja de la consola en la cima.
pub fn area_apps(ancho_pantalla: usize, alto_pantalla: usize) -> RegionPantalla {
    RegionPantalla {
        x: 0,
        y: FRANJA_CONSOLA.min(alto_pantalla),
        ancho: ancho_pantalla,
        alto: alto_pantalla.saturating_sub(FRANJA_CONSOLA),
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
