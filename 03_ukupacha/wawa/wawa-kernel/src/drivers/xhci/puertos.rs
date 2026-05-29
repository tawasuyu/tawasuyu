// =============================================================================
//  renaser :: kernel/src/drivers/xhci/puertos.rs — descubrimiento de puertos USB
// -----------------------------------------------------------------------------
//  Tras `EstructurasArranque::fundar` el controlador esta corriendo (USBCMD.RS=1,
//  HCHalted=0). Sus puertos fisicos (PORTSC[0..max_puertos]) reportan a partir
//  de ese momento si hay un dispositivo conectado (Current Connect Status) y
//  a que velocidad (Port Speed). Esta capa los enumera, dispara un Port Reset
//  por cada uno conectado y deja constancia visual de los hallazgos.
//
//  Lo que NO hace todavia (X2d-completo):
//    * Emitir Enable Slot TRB en el Command Ring y leer el evento de
//      completion para obtener el Slot ID.
//    * Allocar Device Context + Input Context y emitir Address Device.
//    * Leer Device/Configuration/Endpoint descriptors via control transfers
//      en el EP0 Transfer Ring.
//
//  Eso requiere el motor de TRBs+Eventos completo (rings module en su
//  version final), que se construye en la siguiente iteracion.
// =============================================================================

use core::fmt::Write;

use xhci::Registers;

use super::mapeo::MapeadorXhci;

/// Codigos de velocidad de puerto definidos por el xHCI Default Speed
/// Encoding (xHCI §7.2.2.1.1). Estos son los slots universales — si el
/// chipset extiende el encoding por Extended Capabilities, los habria que
/// resolver mirando esa tabla; QEMU y la mayoria de Intel usan los
/// defaults.
fn nombre_velocidad(codigo: u8) -> &'static str {
    match codigo {
        0 => "desconocido",
        1 => "Full-speed (12 Mbps, USB 1.1)",
        2 => "Low-speed (1.5 Mbps, USB 1.x)",
        3 => "High-speed (480 Mbps, USB 2.0)",
        4 => "SuperSpeed (5 Gbps, USB 3.0)",
        5 => "SuperSpeedPlus (10 Gbps, USB 3.1)",
        _ => "extendido",
    }
}

/// Enumera los `max_puertos` puertos del controlador. Por cada uno con
/// Current Connect Status=1, dispara un Port Reset, espera a que la
/// HW levante PortResetChange (PRC=1), y reporta velocidad detectada a
/// la traza serial. Devuelve cuantos puertos quedaron con dispositivo
/// conectado y reseteado correctamente.
///
/// Spinning con tope para el reset; un puerto colgado no tumba el resto
/// del kernel — solo se omite y se traza el problema.
pub fn enumerar(registros: &mut Registers<MapeadorXhci>, max_puertos: u8) -> usize {
    /// Tope generoso del spinning de reset. Spec dice que el reset USB 2.0
    /// tarda ~50 ms; con un loop tight de spin_loop cabe sobradamente.
    const MAX_INTENTOS_RESET: u32 = 50_000_000;
    /// Demora cruda por spin para power-good + debounce de conexion. No hay
    /// reloj util aqui (las interrupciones aun no se habilitan), asi que un
    /// spin acotado hace de espera (~cientos de ms a GHz). USB exige ~20 ms de
    /// power-good y ~100 ms de debounce; sobra.
    const ESPERA_POWER: u32 = 150_000_000;

    // PASO 0 :: ALIMENTAR los puertos (PORTSC.PP=1). EN METAL el xHCI arranca con
    // los puertos SIN energia: hasta que el driver los enciende, Current Connect
    // Status lee 0 aunque HAYA un dispositivo (es el «0 puertos con dispositivo»
    // que vimos). QEMU los auto-alimenta — por eso alli no hacia falta. Encender
    // PP=1 ahora es SEGURO: el puerto esta deshabilitado (PED=0) y sin bits de
    // cambio puestos, asi que el writeback de `update_volatile` no dispara ningun
    // RW1C. Solo tocamos los que NO esten ya alimentados (no perturbar USB3 ya
    // entrenados por el firmware).
    let mut alimentamos_alguno = false;
    for i in 0..max_puertos as usize {
        let portsc = registros.port_register_set.read_volatile_at(i);
        if !portsc.portsc.port_power() {
            alimentamos_alguno = true;
            registros.port_register_set.update_volatile_at(i, |p| {
                p.portsc.set_port_power();
            });
        }
    }
    if alimentamos_alguno {
        for _ in 0..ESPERA_POWER {
            core::hint::spin_loop();
        }
    }

    let mut activos: usize = 0;

    for i in 0..max_puertos as usize {
        let portsc = registros.port_register_set.read_volatile_at(i);
        let conectado = portsc.portsc.current_connect_status();
        if !conectado {
            continue;
        }
        // Velocidad antes del reset — para USB 3 ya viene definida (link
        // training); para USB 2 puede venir 0 hasta que termine el reset.
        let velocidad_pre = portsc.portsc.port_speed();

        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: puerto {} :: conectado :: velocidad_pre={} ({})",
            i,
            velocidad_pre,
            nombre_velocidad(velocidad_pre),
        );

        // Disparar Port Reset. SEGURO via update_volatile aqui: ANTES del reset
        // un puerto USB2 esta deshabilitado (PED=0), asi que el writeback no
        // deshabilita nada. (Un USB3 ya habilitado se saltaria; nuestro raton es
        // LS/FS.) Para USB 2 este reset explicito es obligatorio para habilitar.
        registros.port_register_set.update_volatile_at(i, |p| {
            p.portsc.set_port_reset();
        });

        // Esperar Port Reset Change (con tope; un puerto colgado no debe
        // freezar el arranque — se omite y se sigue con el resto).
        let mut intentos = 0;
        let mut reset_ok = true;
        loop {
            let estado = registros.port_register_set.read_volatile_at(i);
            if estado.portsc.port_reset_change() {
                break;
            }
            intentos += 1;
            if intentos >= MAX_INTENTOS_RESET {
                let _ = writeln!(
                    crate::baliza::Serie,
                    "xhci :: puerto {} :: reset no completo en tope",
                    i,
                );
                reset_ok = false;
                break;
            }
            core::hint::spin_loop();
        }
        if !reset_ok {
            continue;
        }

        // OJO :: NO limpiamos PRC aqui. Tras el reset el puerto queda HABILITADO
        // (PED=1), y como PED es RW1C, un `update_volatile` que reescriba el
        // valor leido (con PED=1) DESHABILITARIA el puerto recien habilitado —
        // dejando el dispositivo inutilizable. PRC quedando puesto es inofensivo
        // (a lo sumo un Port Status Change Event que el drenado de eventos
        // ignora) y NO bloquea el Address Device. Solo leemos la velocidad.
        let portsc = registros.port_register_set.read_volatile_at(i);
        let velocidad = portsc.portsc.port_speed();

        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: puerto {} :: reset OK :: velocidad={} ({})",
            i,
            velocidad,
            nombre_velocidad(velocidad),
        );
        activos += 1;
    }

    let _ = writeln!(
        crate::baliza::Serie,
        "xhci :: enumeracion puertos :: {} con dispositivo conectado de {} total",
        activos,
        max_puertos,
    );

    activos
}
