// =============================================================================
//  renaser :: kernel/src/compositor.rs — Fase 8 :: el compositor teselante
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
}

/// Una ventana del escritorio: una app, su geometria y su ultimo fotograma.
struct Ventana {
    /// Tamaño natural del lienzo de la app — lo que sabe pintar, fijo.
    natural_ancho: usize,
    natural_alto: usize,
    /// El marco teselado actual — donde la app vive en pantalla. Cambia con
    /// cada re-teselado.
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
    /// ocupa esa celda del teselado. Una permutacion de `0..ventanas.len()`.
    /// Separar el orden de la identidad permite promover y reordenar ventanas
    /// sin tocar su `indice_app` —su canal de teclado, su ranura de estado—.
    orden: Vec<usize>,
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
    // celda `i`. Los mandos del teclado lo permutaran.
    let orden = (0..ventanas.len()).collect();
    let mut escritorio = Escritorio {
        modo: MODO_INICIAL,
        ancho,
        alto,
        ventanas,
        orden,
    };
    aplicar_teselado(&mut escritorio);

    ESCRITORIO.call_once(|| Mutex::new(escritorio));
}

/// Recalcula el teselado y asigna a cada ventana su marco. La celda `slot` del
/// teselado va a la ventana `orden[slot]`: manda el orden, no la identidad.
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

/// Pinta el escenario inicial del compositor: el area de apps y sus marcos
/// teselados. Se invoca una vez, tras `fundar`, antes de encender las apps.
pub fn componer_escenario() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let escritorio = escritorio.lock();
    let area = area_apps(escritorio.ancho, escritorio.alto);
    let marcos: Vec<RegionPantalla> = escritorio.ventanas.iter().map(|v| v.marco).collect();
    crate::consola::pintar_escenario(area, &marcos);
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
/// kernel asume la persistencia visual— y lo compone, centrado, en su marco.
/// Lo invoca la capacidad `sys_render_frame` desde el `tick` cooperativo.
pub fn presentar_fotograma(indice: usize, datos: &[u8]) {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let (marco, nat_ancho, nat_alto) = {
        let mut escritorio = escritorio.lock();
        let Some(ventana) = escritorio.ventanas.get_mut(indice) else {
            return;
        };
        // Cachear el fotograma. El destino esta acotado al lienzo natural; se
        // copia el minimo de ambas longitudes — jamas se desborda la cache.
        let n = ventana.cache.len().min(datos.len());
        ventana.cache[..n].copy_from_slice(&datos[..n]);
        ventana.pintada = true;
        (ventana.marco, ventana.natural_ancho, ventana.natural_alto)
    };
    let enfocada = FOCO.load(Ordering::Relaxed) == indice;
    crate::consola::volcar_marco(marco, nat_ancho, nat_alto, datos, enfocada);
}

/// Marca la ventana `indice` como desalojada y tatua su marco con la baliza.
/// Desde aqui queda excluida del foco — el teclado la salta.
pub fn desalojar(indice: usize, color: Color) {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let marco = {
        let mut escritorio = escritorio.lock();
        let Some(ventana) = escritorio.ventanas.get_mut(indice) else {
            return;
        };
        ventana.baliza = Some(color);
        ventana.marco
    };
    let enfocada = FOCO.load(Ordering::Relaxed) == indice;
    crate::consola::pintar_desalojo(marco, color, enfocada);
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
        }
    }
}

/// Cicla al siguiente modo de teselado: recalcula los marcos de todas las
/// ventanas y recompone el escritorio entero desde las caches de respaldo.
fn ciclar_layout() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    escritorio.modo = escritorio.modo.next();
    aplicar_teselado(&mut escritorio);
    redibujar_todo(&escritorio);
}

/// Mueve el foco a la siguiente ventana VIVA, recorriendo el ORDEN de teselado
/// —de modo que el foco salte entre ventanas visualmente contiguas— y saltando
/// las desalojadas. Redibuja la ventana que pierde el foco y la que lo gana.
fn mover_foco(adelante: bool) {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let escritorio = escritorio.lock();
    let n = escritorio.orden.len();
    if n == 0 {
        return;
    }
    let anterior = FOCO.load(Ordering::Relaxed);
    // La posicion del foco en el orden de teselado — el punto de partida.
    let pos = escritorio
        .orden
        .iter()
        .position(|&v| v == anterior)
        .unwrap_or(0);

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
        let candidata = escritorio.orden[nueva_pos];
        if escritorio.ventanas[candidata].baliza.is_none() {
            nuevo = candidata;
            break;
        }
    }
    FOCO.store(nuevo, Ordering::Relaxed);

    if let Some(ventana) = escritorio.ventanas.get(anterior) {
        redibujar_ventana(ventana, false);
    }
    redibujar_ventana(&escritorio.ventanas[nuevo], true);
}

/// Promueve la ventana enfocada a la posicion maestra —la celda 0— del
/// teselado: la saca de su lugar en el orden y la inserta al frente. Las demas
/// se desplazan una posicion. El escritorio entero se recompone.
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
        redibujar_todo(&escritorio);
    }
}

/// Mueve la ventana enfocada una posicion en el orden de teselado,
/// intercambiandola con su vecina. El foco viaja con la ventana — el indice no
/// cambia, solo su lugar en el orden.
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
        redibujar_todo(&escritorio);
    }
}

/// Recompone el escritorio entero: repinta el escenario —area y paneles— con
/// los marcos nuevos y, sobre el, cada ventana desde su cache de respaldo.
fn redibujar_todo(escritorio: &Escritorio) {
    let area = area_apps(escritorio.ancho, escritorio.alto);
    let marcos: Vec<RegionPantalla> = escritorio.ventanas.iter().map(|v| v.marco).collect();
    crate::consola::pintar_escenario(area, &marcos);

    let foco = FOCO.load(Ordering::Relaxed);
    for (i, ventana) in escritorio.ventanas.iter().enumerate() {
        redibujar_ventana(ventana, i == foco);
    }
}

/// Redibuja UNA ventana en su marco actual: si fue desalojada, su baliza; si ya
/// pinto, su ultimo fotograma desde la cache; si aun no pinto, nada —el panel
/// del escenario ya esta puesto—.
fn redibujar_ventana(ventana: &Ventana, enfocada: bool) {
    match ventana.baliza {
        Some(color) => crate::consola::pintar_desalojo(ventana.marco, color, enfocada),
        None => {
            if ventana.pintada {
                crate::consola::volcar_marco(
                    ventana.marco,
                    ventana.natural_ancho,
                    ventana.natural_alto,
                    &ventana.cache,
                    enfocada,
                );
            }
        }
    }
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
/// tiene exactamente `n` elementos, en el orden de las apps del manifiesto.
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
