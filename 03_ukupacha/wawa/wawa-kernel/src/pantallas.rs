// =============================================================================
//  wawa :: kernel/src/pantallas.rs — Fase 59 :: el registro de outputs
// -----------------------------------------------------------------------------
//  Hasta la Fase 58 el kernel asumio UN solo output fisico — el framebuffer que
//  el bootloader entrega via `BootInfo.framebuffer`. Toda la geometria del
//  compositor (`area_apps`, `area_taskbar`, `region_launcher`) toma su ancho y
//  alto del `Escritorio`, que a su vez lo tomo del unico framebuffer.
//
//  Para que multi-monitor sea posible, este modulo introduce un primer ladrillo:
//  `Output` — una sub-region de espacio compuesto donde el compositor puede
//  pintar de forma independiente. Hoy hay UNO, con region cubriendo todo el
//  framebuffer. La estructura es `Vec<Output>` desde el inicio: cuando el
//  bootloader (o un driver GPU propio) exponga mas handles GOP, esos outputs
//  adicionales se enchufan aqui sin tocar al resto del kernel.
//
//  ESTADO (Fase 59 v1): N = 1. El teselado, la taskbar, el launcher y la
//  consola siguen operando sobre el output PRIMARIO. El refactor de
//  «ventana_a_output» + render por-output queda como v2 — la cota fisica
//  (un framebuffer del bootloader) no permite hoy validar N > 1 en QEMU sin
//  driver propio (ver WAWA.md §14.1.2).
// =============================================================================

use alloc::vec;
use alloc::vec::Vec;

use spin::{Mutex, Once};

use crate::grafico::RegionPantalla;

/// Un output del compositor: una sub-region rectangular del escritorio
/// compuesto donde se renderiza independientemente. En hardware tipico
/// corresponde a un monitor; en QEMU con un solo framebuffer, hay UNO que
/// cubre el framebuffer entero.
#[derive(Clone, Copy)]
pub struct Output {
    /// Identidad del output. `0` es el primario; los demas en orden de
    /// enumeracion del firmware. Hoy el unico consumidor de `id` seria un
    /// renderer multi-output que aun no existe — `#[allow(dead_code)]`
    /// reconoce que es API preparada, no codigo huerfano.
    #[allow(dead_code)]
    pub id: usize,
    /// Sub-region del espacio compuesto. En la version mono-output (N=1) la
    /// region cubre `0..ancho × 0..alto` del framebuffer.
    pub region: RegionPantalla,
}

/// El registro global de outputs. Se funda una sola vez, al arrancar, con el
/// output primario derivado del framebuffer que entrega el bootloader. La
/// puerta `Mutex` deja sitio a futuro: un driver GPU o un fork de
/// `bootloader_api` podra `push`ear outputs adicionales sin re-fundar.
static OUTPUTS: Once<Mutex<Vec<Output>>> = Once::new();

/// Funda el registro con el output PRIMARIO derivado del framebuffer. La
/// region cubre `0..ancho × 0..alto`. Idempotente: una segunda invocacion
/// no remplaza el registro existente — el bootloader entrega los framebuffers
/// una sola vez.
pub fn fundar(ancho: usize, alto: usize) {
    OUTPUTS.call_once(|| {
        Mutex::new(vec![Output {
            id: 0,
            region: RegionPantalla {
                x: 0,
                y: 0,
                ancho,
                alto,
            },
        }])
    });
}

/// FASE 64 :: funda el registro con VARIOS outputs de una vez — el primero es
/// el primario (debe estar en el origen `(0,0)` por convencion de
/// `mirada-layout::disponer`), el resto son secundarios con su region en el
/// espacio compuesto. Lo invoca el arranque multi-scanout con las regiones que
/// el driver virtio-gpu + `disponer` calcularon. Idempotente como `fundar`: una
/// segunda invocacion (p.ej. el `fundar` mono-output del arranque de userspace)
/// no remplaza el registro ya fundado. No-op si `regiones` viene vacio.
pub fn fundar_outputs(regiones: &[RegionPantalla]) {
    if regiones.is_empty() {
        return;
    }
    OUTPUTS.call_once(|| {
        let mut outputs = Vec::with_capacity(regiones.len());
        for (id, region) in regiones.iter().enumerate() {
            outputs.push(Output { id, region: *region });
        }
        Mutex::new(outputs)
    });
}

/// La region del output PRIMARIO. `None` si el registro aun no se fundo —
/// el kernel se levanta sin pantalla en ese caso, y el compositor jamas
/// arranca, asi que en la practica esta funcion devuelve `Some` siempre que
/// haya compositor. Util por simetria con el resto de `Once`-fundados.
pub fn primario() -> Option<RegionPantalla> {
    OUTPUTS.get()?.lock().first().map(|o| o.region)
}

/// Cuantos outputs hay registrados. Hoy siempre `1` tras `fundar`; el dia que
/// un driver agregue mas, este valor cambia y los consumidores que iteren
/// sobre outputs (un compositor multi-monitor venidero) los veran.
#[allow(dead_code)]
pub fn count() -> usize {
    OUTPUTS.get().map(|m| m.lock().len()).unwrap_or(0)
}

/// Una copia (clonada bajo el lock) de todos los outputs. Pensado para
/// consumidores que necesiten iterar — el lock se libera antes de que el
/// llamante use el `Vec`, asi nadie anida cerrojos. Si el registro no se
/// fundo, devuelve un `Vec` vacio.
#[allow(dead_code)]
pub fn todos() -> Vec<Output> {
    OUTPUTS
        .get()
        .map(|m| m.lock().clone())
        .unwrap_or_default()
}

/// Agrega un output adicional al registro. Pensado para drivers que enumeran
/// outputs en runtime (virtio-gpu, fork de `bootloader_api`). Devuelve el
/// `id` asignado — el orden de llegada—. No-op si el registro aun no se
/// fundo (`primario` tiene que existir antes que cualquier secundario).
#[allow(dead_code)]
pub fn registrar(region: RegionPantalla) -> Option<usize> {
    let mutex = OUTPUTS.get()?;
    let mut outputs = mutex.lock();
    let id = outputs.len();
    outputs.push(Output { id, region });
    Some(id)
}
