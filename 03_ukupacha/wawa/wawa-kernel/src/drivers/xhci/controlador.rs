// =============================================================================
//  renaser :: kernel/src/drivers/xhci/controlador.rs — descubrimiento + capacidad
// -----------------------------------------------------------------------------
//  Fase X2a :: localizar el primer controlador xHCI del bus PCI, leer su BAR0,
//  mapearlo a virtual y dejarse interrogar por sus capability registers
//  (CAPLENGTH, HCIVERSION, HCSPARAMS1..3, HCCPARAMS1, DBOFF, RTSOFF). Eso
//  responde tres preguntas que toda la fase X2 necesita:
//
//    * Cuantos `Device Slots` soporta este controlador (Max Slots).
//    * Cuantos puertos fisicos expone (Max Ports).
//    * Cual es la version de xHCI que implementa (0x100 = xHCI 1.0,
//      0x110 = xHCI 1.1, 0x120 = xHCI 1.2).
//
//  Esta fase NO arranca el controlador (USBCMD.RS=1 viene en X2b/c despues
//  de programar DCBAA, Command Ring y Event Ring). Solo descubre + lee +
//  deja constancia. Es el equivalente a leer el chipset antes de hablarle.
// =============================================================================

use core::fmt::Write;

use alloc::{format, string::String, vec::Vec};
use spin::{Mutex, Once};
use virtio_drivers::transport::pci::bus::DeviceFunction;

use xhci::Registers;

use super::mapeo::MapeadorXhci;
use super::rings::EstructurasArranque;
use crate::drivers::pci;

/// Registros del controlador xHCI ya mapeados — `xhci::Registers` envuelve
/// los volatiles de los cuatro grupos (capability/operational/runtime/
/// doorbell). Reside detras de un `Mutex` por la disciplina del kernel —
/// renaser es de un solo nucleo, pero el cerrojo deja un punto canonico
/// para sincronizar entre tareas y la IRQ del XHCI cuando llegue.
struct Estado {
    /// El `DeviceFunction` PCI de donde sale el controlador. Util para
    /// volver al espacio de configuracion (Bus Master enable, IRQ line)
    /// durante las fases siguientes.
    pci: DeviceFunction,
    /// Direccion fisica del BAR0 — base del MMIO del controlador.
    bar0_fisica: u64,
    /// Registros volatiles, listos para leer/escribir.
    registros: Registers<MapeadorXhci>,
    /// Estructuras DMA que dan vida al controlador (DCBAA, Command Ring,
    /// Event Ring). X2d (port enumeration) las consume.
    estructuras: EstructurasArranque,
    /// El raton USB HID si se hallo y configuro uno durante la enumeracion
    /// (X3). `atender_raton_hid` lo polea por fotograma. `None` si no hay
    /// raton USB —se cae al PS/2/tableta como hasta X2—.
    raton_hid: Option<super::hid::RatonHid>,
    /// Lineas de diagnostico legibles —que clase tiene cada dispositivo USB
    /// visto, si se monto raton— para volcarlas A PANTALLA desde main.rs. En
    /// metal sin COM1 esta es la UNICA forma de saber que paso con el USB.
    resumen: Vec<String>,
}

// SEGURIDAD: `Registers` contiene punteros crudos al MMIO. renaser es de un
// solo nucleo y todo acceso pasa por el cerrojo de `ESTADO`; jamas se
// comparte entre hilos reales. La marca `Send` la exige el `Mutex`.
unsafe impl Send for Estado {}

/// El estado global del controlador xHCI, vivo una sola vez por arranque.
static ESTADO: Once<Mutex<Estado>> = Once::new();

/// Lee el registro BAR0 del espacio de configuracion PCI y devuelve su
/// direccion fisica de 64 bits, descartando los bits bajos de tipo. El BAR
/// del xHCI siempre es memoria mapeada y, en xHCI moderno, 64 bits — se
/// lee como dos dwords contiguos (offset 0x10 + 0x14).
fn leer_bar0(device_function: DeviceFunction) -> u64 {
    let bajo = read_dword(device_function, 0x10);
    let alto = read_dword(device_function, 0x14);
    // Bits 0..=3 del BAR codifican el tipo (memoria/IO, 32/64, prefetch).
    // Los descartamos: el BAR alineado a 4 KiB es el campo de bits 4..=31.
    let bajo_addr = (bajo & 0xFFFF_FFF0) as u64;
    let alto_addr = (alto as u64) << 32;
    alto_addr | bajo_addr
}

/// Lee un dword del espacio de configuracion PCI. Sirve para los BAR; las
/// primitivas del modulo `pci` son privadas — replicamos las dos lineas
/// para no exponerlas innecesariamente.
fn read_dword(device_function: DeviceFunction, offset: u8) -> u32 {
    use x86_64::instructions::port::Port;
    const CONFIG_ADDRESS: u16 = 0xCF8;
    const CONFIG_DATA: u16 = 0xCFC;
    let direccion = 0x8000_0000u32
        | ((device_function.bus as u32) << 16)
        | ((device_function.device as u32) << 11)
        | ((device_function.function as u32) << 8)
        | ((offset as u32) & 0xFC);
    unsafe {
        Port::<u32>::new(CONFIG_ADDRESS).write(direccion);
        Port::<u32>::new(CONFIG_DATA).read()
    }
}

/// Activa Bus Master + Memory Space en el comando PCI del dispositivo. xHCI
/// no inicia DMA sin BUS_MASTER, y el firmware UEFI lo deja apagado tras
/// `ExitBootServices` para no pelear con el kernel cargado. Lo encendemos
/// nosotros.
fn habilitar_bus_master(device_function: DeviceFunction) {
    use x86_64::instructions::port::Port;
    const CONFIG_ADDRESS: u16 = 0xCF8;
    const CONFIG_DATA: u16 = 0xCFC;
    const COMMAND_OFFSET: u8 = 0x04;
    let direccion = 0x8000_0000u32
        | ((device_function.bus as u32) << 16)
        | ((device_function.device as u32) << 11)
        | ((device_function.function as u32) << 8)
        | ((COMMAND_OFFSET as u32) & 0xFC);
    unsafe {
        Port::<u32>::new(CONFIG_ADDRESS).write(direccion);
        let actual = Port::<u32>::new(CONFIG_DATA).read();
        // Bit 1 = MEMORY_SPACE, bit 2 = BUS_MASTER. Mantener el resto.
        let nuevo = actual | 0b110;
        Port::<u32>::new(CONFIG_ADDRESS).write(direccion);
        Port::<u32>::new(CONFIG_DATA).write(nuevo);
    }
}

/// Resumen legible de las capacidades fijas del controlador. Se imprime al
/// montar para que el operador vea con que se enfrenta. Los campos vienen
/// del grupo de capability registers del xHCI (offset 0..CAPLENGTH del BAR).
///
/// `#[allow(dead_code)]` en los campos hasta que X2c los consuma — el DCBAA
/// se dimensiona segun `contexts_64`, los buferes DMA se acotan por
/// `tam_pagina`, etc.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct ResumenCapacidades {
    /// Version del estandar xHCI (BCD): 0x0100=1.0, 0x0110=1.1, 0x0120=1.2.
    pub version: u16,
    /// Numero maximo de Device Slots soportados (1..=255).
    pub max_slots: u8,
    /// Numero maximo de puertos fisicos del controlador (1..=255).
    pub max_puertos: u8,
    /// Numero de Interrupters disponibles para el Event Ring. >=1 siempre.
    pub max_interrupters: u16,
    /// Tamano de la pagina que el controlador exige para DCBAA y demas
    /// estructuras DMA. Casi siempre 4 KiB; bit n = pagina 2^(n+12) bytes.
    pub tam_pagina: u32,
    /// Si el controlador es de 64-bit Addressing Capable (AC64).
    pub ac64: bool,
    /// Si el controlador requiere Contexts de 64 bytes (CSZ=1) en lugar
    /// de los 32 bytes por defecto. Importante para X2c (DCBAA).
    pub contexts_64: bool,
}

/// Descubre el primer controlador xHCI del bus PCI, mapea su BAR0 y deja sus
/// registros vivos en `ESTADO`. Devuelve un `Err` legible si no hay XHCI en
/// el bus —caso comun en QEMU sin `-device qemu-xhci`, y caso definitorio en
/// portatiles sin USB 3 (que ya no existen)—. La fase es NO-DESTRUCTIVA: no
/// resetea el controlador ni levanta USBCMD.RS; solo lee.
pub fn montar() -> Result<ResumenCapacidades, &'static str> {
    let hallados = pci::enumerar_por_clase(pci::clases::USB_XHCI);
    let num_controladores = hallados.len();
    if num_controladores == 0 {
        return Err("xhci :: no se hallo controlador en el bus PCI");
    }

    let mut resumen_global: Vec<String> = Vec::new();
    resumen_global.push(format!("usb :: {num_controladores} controlador(es) XHCI"));

    // Inicializar CADA controlador XHCI y quedarse con el PRIMERO que tenga un
    // raton USB. Muchas maquinas tienen varios controladores —o el raton cuelga
    // de uno distinto al primero—, asi que mirar solo el primero no basta. Si
    // ninguno tiene raton, se guarda el primero que haya inicializado para que
    // el resto del kernel sepa que hay XHCI. Los controladores no elegidos
    // quedan corriendo pero sin servir — inofensivo.
    let mut elegido: Option<ControladorListo> = None;
    let mut total_disp = 0usize;

    for (idx, info) in hallados.into_iter().enumerate() {
        match intentar_controlador(info) {
            Ok(mut listo) => {
                let tiene_raton = listo.raton_hid.is_some();
                total_disp += listo.conectados;
                resumen_global.push(format!(
                    "usb ctrl{idx}: {} puertos, {} con disp, raton={}",
                    listo.caps.max_puertos,
                    listo.conectados,
                    if tiene_raton { "SI" } else { "NO" },
                ));
                resumen_global.append(&mut listo.diag);
                if tiene_raton {
                    elegido = Some(listo);
                    break; // hallado el del raton; no inicializar mas.
                }
                if elegido.is_none() {
                    elegido = Some(listo);
                }
            }
            Err(motivo) => {
                resumen_global.push(format!("usb ctrl{idx}: fallo: {motivo}"));
            }
        }
    }

    let elegido = elegido.ok_or("xhci :: ningun controlador XHCI inicializo")?;
    let caps = elegido.caps;
    // Linea-resumen DECISIVA: cuantos dispositivos hay en los puertos raiz de
    // todos los controladores. >0 sin raton = hay algo (probable HUB con el
    // raton detras, que aun no atravesamos). =0 = no se detecta nada conectado.
    resumen_global.push(format!(
        "usb RESUMEN: {num_controladores} ctrls, {total_disp} disp en puertos raiz, raton={}",
        if elegido.raton_hid.is_some() { "SI" } else { "NO" },
    ));

    ESTADO.call_once(move || {
        Mutex::new(Estado {
            pci: elegido.pci,
            bar0_fisica: elegido.bar0_fisica,
            registros: elegido.registros,
            estructuras: elegido.estructuras,
            raton_hid: elegido.raton_hid,
            resumen: resumen_global,
        })
    });

    Ok(caps)
}

/// Todo lo que aporta UN controlador XHCI ya inicializado. `montar` colecciona
/// uno por controlador del bus y se queda con el que tenga raton.
struct ControladorListo {
    pci: DeviceFunction,
    bar0_fisica: u64,
    registros: Registers<MapeadorXhci>,
    estructuras: EstructurasArranque,
    raton_hid: Option<super::hid::RatonHid>,
    caps: ResumenCapacidades,
    conectados: usize,
    diag: Vec<String>,
}

/// Inicializa UN controlador XHCI end-to-end: bus master, mapear BAR, leer
/// capacidades, reset (X2b), estructuras DMA + USBCMD.RS=1 (X2c), enumerar
/// puertos (X2d) y dispositivos (raton HID, X3). Devuelve todo lo necesario
/// para guardarlo, o un Err legible si un paso critico falla —ese controlador
/// se omite y se prueba el siguiente—.
fn intentar_controlador(info: pci::InfoPci) -> Result<ControladorListo, &'static str> {
    let pci_df = info.device_function();

    // Habilitar Bus Master + Memory Space antes de tocar el BAR.
    habilitar_bus_master(pci_df);

    let bar0_fisica = leer_bar0(pci_df);
    if bar0_fisica == 0 {
        return Err("BAR0 vale cero");
    }

    // SEGURIDAD: `Registers::new` toma la base FISICA del BAR0 y un Mapper que
    // sabe llevarla a virtual (`MapeadorXhci` sobre `memory::mmio`).
    let mut registros = unsafe { Registers::new(bar0_fisica as usize, MapeadorXhci) };

    let cap = &registros.capability;
    let version = cap.hciversion.read_volatile().get();
    let hcsparams1 = cap.hcsparams1.read_volatile();
    let hccparams1 = cap.hccparams1.read_volatile();
    let max_slots = hcsparams1.number_of_device_slots();
    let max_puertos = hcsparams1.number_of_ports();
    let max_interrupters = hcsparams1.number_of_interrupts();
    let ac64 = hccparams1.addressing_capability();
    let contexts_64 = hccparams1.context_size();
    let tam_pagina = registros.operational.pagesize.read_volatile().get() as u32;

    let caps = ResumenCapacidades {
        version,
        max_slots,
        max_puertos,
        max_interrupters,
        tam_pagina,
        ac64,
        contexts_64,
    };

    let _ = writeln!(
        crate::baliza::Serie,
        "xhci :: PCI {}:{}.{} vendor={:#06x} device={:#06x} BAR0={:#x} ver={:#06x} slots={} puertos={}",
        pci_df.bus, pci_df.device, pci_df.function,
        info.vendor_id, info.device_id, bar0_fisica, version, max_slots, max_puertos,
    );

    resetear_controlador(&mut registros)?;
    let mut estructuras = EstructurasArranque::fundar(&mut registros, max_slots)?;
    let conectados = super::puertos::enumerar(&mut registros, max_puertos);

    let (raton_hid, diag) = if conectados > 0 {
        match enumerar_dispositivos(&mut registros, &mut estructuras, max_puertos) {
            Ok(r) => r,
            Err(motivo) => {
                let mut d: Vec<String> = Vec::new();
                d.push(format!("usb enum abortada: {motivo}"));
                (None, d)
            }
        }
    } else {
        (None, Vec::new())
    };

    Ok(ControladorListo {
        pci: pci_df,
        bar0_fisica,
        registros,
        estructuras,
        raton_hid,
        caps,
        conectados,
        diag,
    })
}

/// Bajar el controlador a HCHalted, dispararle HCRST=1, esperar a que la HW
/// lo baje sola y finalmente esperar CNR=0. Tras esto el controlador esta
/// en estado «default» — todas las estructuras DMA (DCBAA, Command Ring,
/// Event Ring) hay que (re)programarlas en X2c antes de USBCMD.RS=1.
///
/// Spinning con tope: la spec no garantiza un tiempo maximo de reset; un
/// controlador roto podria colgarse. Acotamos en `MAX_INTENTOS` lecturas y
/// devolvemos Err con traza para que el resto del kernel siga vivo —xHCI
/// muerto no debe tumbar wawa, solo dejarla sin USB.
fn resetear_controlador(registros: &mut Registers<MapeadorXhci>) -> Result<(), &'static str> {
    /// Aproximadamente 1 segundo a 1 GHz si cada iteracion son ~10 ciclos.
    /// Mas que suficiente para el reset y sin riesgo de freeze permanente.
    const MAX_INTENTOS: u32 = 100_000_000;

    let op = &mut registros.operational;

    // 1. Si el controlador esta corriendo, pararlo limpiamente.
    if !op.usbsts.read_volatile().hc_halted() {
        op.usbcmd.update_volatile(|u| {
            u.clear_run_stop();
        });
        let mut intentos = 0;
        while !op.usbsts.read_volatile().hc_halted() {
            intentos += 1;
            if intentos >= MAX_INTENTOS {
                return Err("xhci :: no llego a HCHalted tras RS=0");
            }
            core::hint::spin_loop();
        }
    }

    // 2. Disparar HCRST. La HW lo baja sola al completar.
    op.usbcmd.update_volatile(|u| {
        u.set_host_controller_reset();
    });

    // 3. Esperar HCRST=0 (reset completo).
    let mut intentos = 0;
    while op.usbcmd.read_volatile().host_controller_reset() {
        intentos += 1;
        if intentos >= MAX_INTENTOS {
            return Err("xhci :: HCRST no bajo tras reset");
        }
        core::hint::spin_loop();
    }

    // 4. Esperar Controller Not Ready=0. Hasta que CNR=0 el controlador
    //    rechaza programar DCBAAP/CRCR/ERSTBA — la spec lo exige.
    let mut intentos = 0;
    while op.usbsts.read_volatile().controller_not_ready() {
        intentos += 1;
        if intentos >= MAX_INTENTOS {
            return Err("xhci :: CNR no bajo tras reset");
        }
        core::hint::spin_loop();
    }

    let _ = writeln!(
        crate::baliza::Serie,
        "xhci :: reset OK (HCRST/CNR limpios)",
    );

    Ok(())
}

/// `true` si `montar` descubrio y mapeo un controlador. Util para que los
/// drivers que se apoyan en xHCI (USB-MS, USB-HID) sepan si tienen base.
#[allow(dead_code)]
pub fn esta_vivo() -> bool {
    ESTADO.get().is_some()
}

/// Lineas de diagnostico del USB para volcar A PANTALLA. Vacio si el
/// controlador no se monto (no hay XHCI en el bus). En metal sin COM1 esta es
/// la unica via para saber que dispositivos USB vio el kernel.
pub fn resumen_usb() -> Vec<String> {
    match ESTADO.get() {
        Some(estado) => estado.lock().resumen.clone(),
        None => Vec::new(),
    }
}

/// X3 :: polea el raton USB HID una vez. Lo llama el reactor (tarea_compositor)
/// cada fotograma. No-op si el controlador no se monto o no hay raton USB —se
/// cae al PS/2/tableta—. Drena el Event Ring SIN bloquear y entrega los deltas
/// del reporte a `drivers::raton`, que mueve y redibuja el puntero.
pub fn atender_raton_hid() {
    let Some(estado) = ESTADO.get() else {
        return;
    };
    let mut guard = estado.lock();
    let estado = &mut *guard;
    if let Some(hid) = estado.raton_hid.as_mut() {
        hid.atender(&mut estado.registros, &mut estado.estructuras.event_ring);
    }
}

/// Devuelve el `DeviceFunction` PCI del controlador, si esta vivo. La fase
/// X2c lo consume para descubrir la linea de IRQ asignada por el firmware.
#[allow(dead_code)]
pub fn pci_device_function() -> Option<DeviceFunction> {
    ESTADO.get().map(|m| m.lock().pci)
}

/// Devuelve la direccion fisica del BAR0 del controlador, si esta vivo.
/// Diagnostico para la traza serial — no se consume desde codigo del kernel.
#[allow(dead_code)]
pub fn bar0_fisica() -> Option<u64> {
    ESTADO.get().map(|m| m.lock().bar0_fisica)
}

/// Codigos de TRB Type relevantes para X2d. Vienen del xHCI spec tabla 6-91.
mod trb_tipo {
    pub const ENABLE_SLOT_COMMAND: u32 = 9;
    pub const COMMAND_COMPLETION_EVENT: u32 = 33;
}

/// Codigos de Completion Code del Command Completion Event. Tabla 6-90.
mod completion {
    pub const SUCCESS: u8 = 1;
}

/// Enumera todos los puertos conectados y para cada uno: Enable Slot →
/// preparar Input/Device Context → Address Device → GET_DESCRIPTOR. Si una
/// etapa falla, se traza y se intenta el siguiente puerto — un dispositivo
/// roto no debe abortar la enumeracion del resto.
fn enumerar_dispositivos(
    registros: &mut Registers<MapeadorXhci>,
    estructuras: &mut super::rings::EstructurasArranque,
    max_puertos: u8,
) -> Result<(Option<super::hid::RatonHid>, Vec<String>), &'static str> {
    use super::comandos;
    use super::contextos::{registrar_en_dcbaa, ContextoDispositivo};

    // El primer raton USB HID hallado se configura y se devuelve para que el
    // reactor lo polee. Los demas dispositivos solo se trazan (X3 cubre raton).
    let mut raton: Option<super::hid::RatonHid> = None;
    // Lineas legibles para volcar a pantalla (sin COM1 es lo unico visible).
    let mut diag: Vec<String> = Vec::new();

    for puerto in 0..max_puertos as usize {
        let portsc = registros.port_register_set.read_volatile_at(puerto);
        if !portsc.portsc.current_connect_status() {
            continue;
        }
        let velocidad = portsc.portsc.port_speed();
        diag.push(format!("usb p{} conectado (vel {})", puerto, velocidad));

        // 1. Enable Slot.
        let slot_id = match emitir_enable_slot(
            registros,
            &mut estructuras.command_ring,
            &mut estructuras.event_ring,
        ) {
            Ok(id) => id,
            Err(motivo) => {
                let _ = writeln!(
                    crate::baliza::Serie,
                    "xhci :: puerto {} :: Enable Slot fallido :: {motivo}",
                    puerto,
                );
                continue;
            }
        };
        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: puerto {} :: slot_id={}",
            puerto,
            slot_id,
        );

        // 2. Preparar Device Context + Input Context + EP0 Ring.
        let mut ctx = match ContextoDispositivo::nuevo(puerto, velocidad) {
            Ok(c) => c,
            Err(motivo) => {
                let _ = writeln!(crate::baliza::Serie, "xhci :: contexto fallido :: {motivo}");
                continue;
            }
        };
        // Apuntar DCBAA[slot_id] al Device Context recien creado.
        registrar_en_dcbaa(&mut estructuras.dcbaa, slot_id, ctx.device_ctx.fisica);

        // 3. Address Device.
        if let Err(motivo) = comandos::address_device(
            registros,
            &mut estructuras.command_ring,
            &mut estructuras.event_ring,
            slot_id,
            ctx.input_ctx.fisica,
        ) {
            let _ = writeln!(
                crate::baliza::Serie,
                "xhci :: puerto {} :: Address Device :: {motivo}",
                puerto,
            );
            continue;
        }
        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: puerto {} :: Address Device OK",
            puerto,
        );

        // 4. GET_DESCRIPTOR(Device, 0, 18) — el descriptor de dispositivo
        //    completo. El primer GET_DESCRIPTOR pide solo 8 bytes para
        //    ajustar MPS, pero nuestro Input Context ya configuro MPS por
        //    velocidad — leemos los 18 bytes directamente. Si el chipset
        //    se queja en metal, partir esto en dos pasos.
        let descriptor =
            match comandos::get_descriptor(registros, &mut estructuras.event_ring, &mut ctx.ep0_ring, slot_id, 1, 0, 18)
        {
            Ok(d) => d,
            Err(motivo) => {
                let _ = writeln!(
                    crate::baliza::Serie,
                    "xhci :: puerto {} :: GET_DESCRIPTOR(Device) :: {motivo}",
                    puerto,
                );
                continue;
            }
        };
        if descriptor.len() >= 18 {
            let id_vendor = u16::from_le_bytes([descriptor[8], descriptor[9]]);
            let id_product = u16::from_le_bytes([descriptor[10], descriptor[11]]);
            let class = descriptor[4];
            let subclass = descriptor[5];
            let protocol = descriptor[6];
            let _ = writeln!(
                crate::baliza::Serie,
                "xhci :: puerto {} :: Device Descriptor :: vendor={:#06x} product={:#06x} class={:#x}/{:#x}/{:#x}",
                puerto,
                id_vendor,
                id_product,
                class,
                subclass,
                protocol,
            );
        }

        // 5. GET_DESCRIPTOR(Configuration, 0, 9) — leer Configuration
        //    Descriptor de cabecera. Bytes 2-3 = wTotalLength.
        let cfg_head = match comandos::get_descriptor(
            registros,
            &mut estructuras.event_ring,
            &mut ctx.ep0_ring,
            slot_id,
            2,
            0,
            9,
        ) {
            Ok(d) => d,
            Err(motivo) => {
                let _ = writeln!(
                    crate::baliza::Serie,
                    "xhci :: puerto {} :: GET_DESCRIPTOR(Config head) :: {motivo}",
                    puerto,
                );
                continue;
            }
        };
        if cfg_head.len() < 9 {
            continue;
        }
        let total_length = u16::from_le_bytes([cfg_head[2], cfg_head[3]]);

        // 6. GET_DESCRIPTOR(Configuration, 0, total_length) — leer el
        //    descriptor completo con todas sus interfaces y endpoints.
        let cfg_blob = match comandos::get_descriptor(
            registros,
            &mut estructuras.event_ring,
            &mut ctx.ep0_ring,
            slot_id,
            2,
            0,
            total_length,
        ) {
            Ok(d) => d,
            Err(motivo) => {
                let _ = writeln!(
                    crate::baliza::Serie,
                    "xhci :: puerto {} :: GET_DESCRIPTOR(Config full) :: {motivo}",
                    puerto,
                );
                continue;
            }
        };
        let config = match super::descriptores::parsear(&cfg_blob) {
            Some(c) => c,
            None => {
                let _ = writeln!(
                    crate::baliza::Serie,
                    "xhci :: puerto {} :: Config descriptor invalido",
                    puerto,
                );
                continue;
            }
        };
        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: puerto {} :: Config descriptor :: valor={} interfaces={}",
            puerto,
            config.valor,
            config.interfaces.len(),
        );
        for iface in &config.interfaces {
            diag.push(format!(
                "usb p{} cls={:#04x}/{:#04x}/{:#04x} eps={}",
                puerto, iface.clase, iface.subclase, iface.protocolo, iface.endpoints.len(),
            ));
            let _ = writeln!(
                crate::baliza::Serie,
                "xhci :: puerto {} :: iface {} alt {} class={:#x}/{:#x}/{:#x} eps={}",
                puerto,
                iface.numero,
                iface.alt_setting,
                iface.clase,
                iface.subclase,
                iface.protocolo,
                iface.endpoints.len(),
            );
            for ep in &iface.endpoints {
                let _ = writeln!(
                    crate::baliza::Serie,
                    "xhci :: puerto {} :: iface {} :: ep {:?} #{} type={} mps={}",
                    puerto,
                    iface.numero,
                    ep.direccion,
                    ep.numero,
                    ep.tipo_transferencia,
                    ep.max_packet_size,
                );
            }
        }
        if let Some((iface, in_ep, out_ep)) = super::descriptores::buscar_usb_ms(&config) {
            let _ = writeln!(
                crate::baliza::Serie,
                "xhci :: puerto {} :: USB-MS detectado :: iface={} bulk_in_ep={} bulk_out_ep={} mps_in={} mps_out={}",
                puerto,
                iface.numero,
                in_ep.numero,
                out_ep.numero,
                in_ep.max_packet_size,
                out_ep.max_packet_size,
            );
        }

        // X3 :: RATON USB HID. Si aun no configuramos uno y esta interface es un
        // raton boot, montarlo: SET_CONFIGURATION → CONFIGURE_ENDPOINT del
        // interrupt IN → SET_PROTOCOL(boot) → armar. El driver vivo se devuelve
        // para que el reactor lo polee por fotograma.
        if raton.is_none() {
            if let Some((iface, in_ep)) = super::descriptores::buscar_raton_hid(&config) {
                let iface_num = iface.numero;
                match super::hid::RatonHid::configurar(
                    registros,
                    &mut estructuras.command_ring,
                    &mut estructuras.event_ring,
                    &mut ctx,
                    slot_id,
                    velocidad,
                    &config,
                    iface_num,
                    &in_ep,
                ) {
                    Ok(r) => {
                        diag.push(format!("usb p{} RATON HID montado", puerto));
                        let _ = writeln!(
                            crate::baliza::Serie,
                            "xhci :: puerto {} :: raton USB HID montado",
                            puerto,
                        );
                        raton = Some(r);
                    }
                    Err(m) => {
                        diag.push(format!("usb p{} raton HID FALLO: {m}", puerto));
                        let _ = writeln!(
                            crate::baliza::Serie,
                            "xhci :: puerto {} :: raton HID fallo :: {m}",
                            puerto,
                        );
                    }
                }
            }
        }
    }
    Ok((raton, diag))
}

/// Emite un Enable Slot Command en el Command Ring, toca el Doorbell 0 y
/// polea el Event Ring buscando el Command Completion Event correspondiente.
/// Devuelve el Slot ID asignado por el HC (1..=max_slots) o un error legible.
///
/// IMPORTANTE: bloquea por polling. En X2d-completo / X3 se reemplazara por
/// un esquema asincrono cuando el reactor sepa esperar la IRQ del XHCI; hoy
/// es la opcion mas corta hacia la validacion end-to-end.
fn emitir_enable_slot(
    registros: &mut Registers<MapeadorXhci>,
    command_ring: &mut super::rings::CommandRing,
    event_ring: &mut super::rings::EventRing,
) -> Result<u8, &'static str> {
    /// Tope del polling — generoso. QEMU responde en microsegundos.
    const MAX_INTENTOS: u32 = 50_000_000;

    // Construir el TRB del Enable Slot Command. Layout xHCI §6.4.3.2:
    //   dwords 0..3 = 0 (reservados / sin parametros).
    //   dword 3 bits 10..15 = TRB Type = 9.
    // El cycle bit se aplica dentro de `CommandRing::encolar`.
    let mut trb = [0u32; 4];
    trb[3] = trb_tipo::ENABLE_SLOT_COMMAND << 10;
    let trb_fisica = command_ring.encolar(trb);

    // Ring Doorbell 0 con target=0 (Command Ring) y stream_id=0. El valor
    // crudo de la spec es el u32 con esos campos; aqui basta escribir cero.
    registros.doorbell.update_volatile_at(0, |db| {
        db.set_doorbell_target(0);
        db.set_doorbell_stream_id(0);
    });

    // Poll del Event Ring. Buscamos un Command Completion Event que
    // referencia nuestro TRB por su direccion fisica (dwords 0..8).
    let mut intentos = 0;
    let slot_id = loop {
        if let Some(dwords) = event_ring.leer() {
            let tipo = (dwords[3] >> 10) & 0x3F;
            let trb_pointer = (dwords[0] as u64) | ((dwords[1] as u64) << 32);
            let completion_code = ((dwords[2] >> 24) & 0xFF) as u8;
            let slot_id_evento = ((dwords[3] >> 24) & 0xFF) as u8;
            event_ring.avanzar();
            // Reprogramar ERDP al nuevo dequeue + EHB=1 (clear pendings).
            let dequeue = event_ring.dequeue_fisica();
            registros
                .interrupter_register_set
                .interrupter_mut(0)
                .erdp
                .update_volatile(|d| {
                    d.set_event_ring_dequeue_pointer(dequeue);
                    d.clear_event_handler_busy();
                });
            if tipo == trb_tipo::COMMAND_COMPLETION_EVENT && trb_pointer == trb_fisica {
                if completion_code != completion::SUCCESS {
                    let _ = writeln!(
                        crate::baliza::Serie,
                        "xhci :: Enable Slot fallido :: completion_code={}",
                        completion_code,
                    );
                    return Err("xhci :: Enable Slot completion code != SUCCESS");
                }
                break slot_id_evento;
            }
            // Evento distinto al esperado (p. ej. Port Status Change): lo
            // saltamos. Una iteracion futura procesara estos eventos en
            // lugar de descartarlos — hoy basta con seguir buscando.
            continue;
        }
        intentos += 1;
        if intentos >= MAX_INTENTOS {
            return Err("xhci :: Enable Slot sin evento de completion en tope");
        }
        core::hint::spin_loop();
    };

    Ok(slot_id)
}
