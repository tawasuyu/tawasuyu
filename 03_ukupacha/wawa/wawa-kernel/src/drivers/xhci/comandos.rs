// =============================================================================
//  renaser :: kernel/src/drivers/xhci/comandos.rs — comandos XHCI + control xfers
// -----------------------------------------------------------------------------
//  Fase X2d-completo. Tras Enable Slot ya tenemos un slot_id; este modulo
//  emite el resto del flujo de enumeracion estandar:
//
//    * `address_device`  — programa el slot con el Input Context recien
//      preparado y sube el dispositivo al estado «Addressed».
//    * `get_descriptor`  — control transfer GET_DESCRIPTOR via tres TRBs
//      (Setup + Data + Status) en el EP0 Transfer Ring del dispositivo.
//    * `poll_evento`     — helper que avanza el Event Ring hasta hallar un
//      evento de un tipo dado (Command Completion vs. Transfer Event) que
//      referencie un TRB fisica determinado.
//
//  Todo polling. La conversion a IRQ del XHCI llega cuando el reactor sepa
//  esperar transferencias bulk de varios MB (X3 en adelante).
// =============================================================================

use core::fmt::Write;

use xhci::Registers;

use super::dma::BuferDma;
use super::mapeo::MapeadorXhci;
use super::rings::{CommandRing, EventRing};

/// Codigos de TRB Type — los que el X2d-completo necesita. Spec §6.4.
mod trb_tipo {
    pub const SETUP_STAGE: u32 = 2;
    pub const DATA_STAGE: u32 = 3;
    pub const STATUS_STAGE: u32 = 4;
    pub const ADDRESS_DEVICE_COMMAND: u32 = 11;
    pub const CONFIGURE_ENDPOINT_COMMAND: u32 = 12;
    pub const TRANSFER_EVENT: u32 = 32;
    pub const COMMAND_COMPLETION_EVENT: u32 = 33;
}

/// Codigos de Completion Code que tratamos especialmente. Tabla 6-90.
mod completion {
    pub const SUCCESS: u8 = 1;
    pub const SHORT_PACKET: u8 = 13;
}

const MAX_INTENTOS_EVENTO: u32 = 50_000_000;

/// Avanza el Event Ring hasta encontrar un evento de `tipo_esperado` que
/// referencie a `trb_fisica`. Eventos intermedios (Port Status Change,
/// etc) se consumen y descartan — sera trabajo posterior tratarlos.
/// Devuelve los 4 dwords del evento hallado.
fn poll_evento(
    registros: &mut Registers<MapeadorXhci>,
    event_ring: &mut EventRing,
    tipo_esperado: u32,
    trb_fisica: u64,
) -> Result<[u32; 4], &'static str> {
    let mut intentos = 0;
    loop {
        if let Some(dwords) = event_ring.leer() {
            let tipo = (dwords[3] >> 10) & 0x3F;
            let ref_trb = (dwords[0] as u64) | ((dwords[1] as u64) << 32);
            event_ring.avanzar();
            let dequeue = event_ring.dequeue_fisica();
            registros
                .interrupter_register_set
                .interrupter_mut(0)
                .erdp
                .update_volatile(|d| {
                    d.set_event_ring_dequeue_pointer(dequeue);
                    d.clear_event_handler_busy();
                });
            if tipo == tipo_esperado && ref_trb == trb_fisica {
                return Ok(dwords);
            }
            // Evento ajeno al que esperamos; logging diagnostico y seguir.
            let _ = writeln!(
                crate::baliza::Serie,
                "xhci :: poll_evento :: tipo={} ref_trb={:#x} (esperaba tipo={}) — descartado",
                tipo,
                ref_trb,
                tipo_esperado,
            );
            continue;
        }
        intentos += 1;
        if intentos >= MAX_INTENTOS_EVENTO {
            return Err("xhci :: poll_evento :: tope sin evento esperado");
        }
        core::hint::spin_loop();
    }
}

/// Address Device: programa el slot con el Input Context preparado y sube
/// al device a estado «Addressed». Tras esto el HC tiene el slot_id ya
/// vinculado al puerto + EP0 Transfer Ring listo para control transfers.
///
/// BSR=0 (Block Set Address Request = 0) envia el SET_ADDRESS estandar al
/// dispositivo via USB. BSR=1 saltearia el SET_ADDRESS pero deja el slot
/// «Default» — util para algunos quirks; aqui hacemos lo estandar.
pub fn address_device(
    registros: &mut Registers<MapeadorXhci>,
    command_ring: &mut CommandRing,
    event_ring: &mut EventRing,
    slot_id: u8,
    input_ctx_fisica: u64,
) -> Result<(), &'static str> {
    // Address Device TRB. Spec §6.4.3.4.
    //   dword[0..2] = Input Context Pointer (64-bit fisica).
    //   dword[3] = (slot_id << 24) | (BSR << 9) | (Type=11 << 10) | cycle.
    let mut trb = [0u32; 4];
    trb[0] = (input_ctx_fisica & 0xFFFFFFFF) as u32;
    trb[1] = ((input_ctx_fisica >> 32) & 0xFFFFFFFF) as u32;
    trb[3] = ((slot_id as u32) << 24) | (trb_tipo::ADDRESS_DEVICE_COMMAND << 10);
    let trb_fisica = command_ring.encolar(trb);

    // Ring doorbell 0 (Command Ring).
    registros.doorbell.update_volatile_at(0, |db| {
        db.set_doorbell_target(0);
        db.set_doorbell_stream_id(0);
    });

    let evento = poll_evento(
        registros,
        event_ring,
        trb_tipo::COMMAND_COMPLETION_EVENT,
        trb_fisica,
    )?;
    let completion_code = ((evento[2] >> 24) & 0xFF) as u8;
    if completion_code != completion::SUCCESS {
        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: Address Device fallido :: code={}",
            completion_code,
        );
        return Err("xhci :: Address Device completion != SUCCESS");
    }
    Ok(())
}

/// Layout en memoria del Setup Packet de USB (8 bytes).
/// Lo empotramos inline en el Setup Stage TRB (IDT=1).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct SetupPacket {
    bm_request_type: u8,
    b_request: u8,
    w_value: u16,
    w_index: u16,
    w_length: u16,
}

/// Tipo de transferencia del Setup Stage TRB. Spec tabla 6-26.
/// `OutData` aun no se usa (no emitimos control OUT con datos todavia) pero
/// completa el espacio del campo TRT.
#[allow(dead_code)]
#[repr(u32)]
enum TipoTransferencia {
    NoData = 0,
    OutData = 2,
    InData = 3,
}

/// GET_DESCRIPTOR(tipo, indice, longitud). Hace un control transfer en EP0
/// del slot. Devuelve los `longitud` bytes leidos.
///
/// Sequence:
///   1. Setup Stage TRB con SetupPacket inline (IDT=1), TRT=InData.
///   2. Data Stage TRB con buffer fisica + len + DIR=1 (IN).
///   3. Status Stage TRB con DIR=0 (OUT para IN data), IOC=1.
///   4. Ring doorbell del slot con target=1 (EP0 DCI).
///   5. Poll Transfer Event referenciando el Status Stage TRB.
pub fn get_descriptor(
    registros: &mut Registers<MapeadorXhci>,
    event_ring: &mut EventRing,
    ep0_ring: &mut CommandRing,
    slot_id: u8,
    tipo_descriptor: u8,
    indice: u8,
    longitud: u16,
) -> Result<alloc::vec::Vec<u8>, &'static str> {
    let bufer = BuferDma::asignar_zero(longitud as usize)
        .ok_or("xhci :: arena DMA exhausta para GET_DESCRIPTOR buffer")?;

    let setup = SetupPacket {
        bm_request_type: 0x80, // Device to Host, Standard, Device.
        b_request: 6,          // GET_DESCRIPTOR.
        w_value: ((tipo_descriptor as u16) << 8) | (indice as u16),
        w_index: 0,
        w_length: longitud,
    };
    // SEGURIDAD: SetupPacket es repr(C, packed), su layout coincide con los
    // primeros 8 bytes que el HC espera leer del Setup Stage TRB cuando
    // IDT=1.
    let setup_bytes: [u8; 8] = unsafe { core::mem::transmute(setup) };

    // 1. Setup Stage TRB.
    let mut s = [0u32; 4];
    s[0] = u32::from_le_bytes([setup_bytes[0], setup_bytes[1], setup_bytes[2], setup_bytes[3]]);
    s[1] = u32::from_le_bytes([setup_bytes[4], setup_bytes[5], setup_bytes[6], setup_bytes[7]]);
    s[2] = 8; // TRB Transfer Length = 8 (size of setup packet).
    // dword[3] = (TRT << 16) | (Type=2 << 10) | (IDT=1 << 6)
    s[3] = ((TipoTransferencia::InData as u32) << 16)
        | (trb_tipo::SETUP_STAGE << 10)
        | (1 << 6);
    let _setup_fisica = ep0_ring.encolar(s);

    // 2. Data Stage TRB.
    let mut d = [0u32; 4];
    d[0] = (bufer.fisica & 0xFFFFFFFF) as u32;
    d[1] = ((bufer.fisica >> 32) & 0xFFFFFFFF) as u32;
    d[2] = longitud as u32;
    // dword[3] = (DIR=1 << 16) | (Type=3 << 10)
    d[3] = (1u32 << 16) | (trb_tipo::DATA_STAGE << 10);
    let _data_fisica = ep0_ring.encolar(d);

    // 3. Status Stage TRB. DIR=0 para IN data (handshake en sentido inverso).
    //    IOC=1 para que el HC publique un Transfer Event al completar.
    let mut st = [0u32; 4];
    st[3] = (trb_tipo::STATUS_STAGE << 10) | (1u32 << 5); // IOC bit.
    let status_fisica = ep0_ring.encolar(st);

    // 4. Ring doorbell del slot, target=1 (EP0 DCI).
    registros.doorbell.update_volatile_at(slot_id as usize, |db| {
        db.set_doorbell_target(1);
        db.set_doorbell_stream_id(0);
    });

    // 5. Poll Transfer Event.
    let evento = poll_evento(
        registros,
        event_ring,
        trb_tipo::TRANSFER_EVENT,
        status_fisica,
    )?;
    let completion_code = ((evento[2] >> 24) & 0xFF) as u8;
    if completion_code != completion::SUCCESS && completion_code != completion::SHORT_PACKET {
        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: GET_DESCRIPTOR fallido :: code={}",
            completion_code,
        );
        return Err("xhci :: GET_DESCRIPTOR completion != SUCCESS");
    }

    // Copiar los bytes del buffer DMA a una Vec poseida.
    let mut salida = alloc::vec![0u8; longitud as usize];
    let slice = unsafe {
        core::slice::from_raw_parts(bufer.virtual_.as_ptr(), longitud as usize)
    };
    salida.copy_from_slice(slice);
    Ok(salida)
}

/// Control transfer SIN etapa de datos: solo Setup + Status. Cubre los pedidos
/// de configuracion que no devuelven payload —SET_CONFIGURATION, y los class
/// requests del HID SET_PROTOCOL / SET_IDLE—. wLength va a 0.
///
/// Para un control sin datos la etapa de Status va SIEMPRE en sentido IN
/// (DIR=1) — la spec lo fija asi cuando no hubo Data Stage.
pub fn control_sin_datos(
    registros: &mut Registers<MapeadorXhci>,
    event_ring: &mut EventRing,
    ep0_ring: &mut CommandRing,
    slot_id: u8,
    bm_request_type: u8,
    b_request: u8,
    w_value: u16,
    w_index: u16,
) -> Result<(), &'static str> {
    let setup = SetupPacket {
        bm_request_type,
        b_request,
        w_value,
        w_index,
        w_length: 0,
    };
    // SEGURIDAD: SetupPacket es repr(C, packed); su layout coincide con los 8
    // bytes inline que el HC lee del Setup Stage TRB cuando IDT=1.
    let setup_bytes: [u8; 8] = unsafe { core::mem::transmute(setup) };

    // 1. Setup Stage TRB. TRT = NoData (sin Data Stage).
    let mut s = [0u32; 4];
    s[0] = u32::from_le_bytes([setup_bytes[0], setup_bytes[1], setup_bytes[2], setup_bytes[3]]);
    s[1] = u32::from_le_bytes([setup_bytes[4], setup_bytes[5], setup_bytes[6], setup_bytes[7]]);
    s[2] = 8; // TRB Transfer Length = 8 (setup packet).
    s[3] = ((TipoTransferencia::NoData as u32) << 16)
        | (trb_tipo::SETUP_STAGE << 10)
        | (1 << 6); // IDT=1.
    let _setup_fisica = ep0_ring.encolar(s);

    // 2. Status Stage TRB. DIR=1 (IN) porque no hubo Data Stage. IOC=1.
    let mut st = [0u32; 4];
    st[3] = (trb_tipo::STATUS_STAGE << 10) | (1u32 << 16) | (1u32 << 5);
    let status_fisica = ep0_ring.encolar(st);

    // 3. Ring doorbell del slot, target=1 (EP0 DCI).
    registros.doorbell.update_volatile_at(slot_id as usize, |db| {
        db.set_doorbell_target(1);
        db.set_doorbell_stream_id(0);
    });

    // 4. Poll Transfer Event del Status Stage.
    let evento = poll_evento(registros, event_ring, trb_tipo::TRANSFER_EVENT, status_fisica)?;
    let completion_code = ((evento[2] >> 24) & 0xFF) as u8;
    if completion_code != completion::SUCCESS && completion_code != completion::SHORT_PACKET {
        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: control_sin_datos req={:#x} fallido :: code={}",
            b_request,
            completion_code,
        );
        return Err("xhci :: control sin datos completion != SUCCESS");
    }
    Ok(())
}

/// SET_CONFIGURATION estandar (bRequest=9, bmRequestType=0x00). Activa la
/// configuracion `valor` del dispositivo — prerrequisito para que sus endpoints
/// no-EP0 funcionen.
pub fn set_configuration(
    registros: &mut Registers<MapeadorXhci>,
    event_ring: &mut EventRing,
    ep0_ring: &mut CommandRing,
    slot_id: u8,
    valor: u8,
) -> Result<(), &'static str> {
    control_sin_datos(registros, event_ring, ep0_ring, slot_id, 0x00, 9, valor as u16, 0)
}

/// SET_PROTOCOL del HID (class request, bmRequestType=0x21, bRequest=0x0B).
/// `protocolo` = 0 (Boot) o 1 (Report). Pedimos Boot para que el raton emita el
/// reporte estandar de 3-4 bytes (botones, dx, dy[, rueda]) sin tener que
/// parsear su Report Descriptor.
pub fn set_protocolo(
    registros: &mut Registers<MapeadorXhci>,
    event_ring: &mut EventRing,
    ep0_ring: &mut CommandRing,
    slot_id: u8,
    interface: u8,
    protocolo: u16,
) -> Result<(), &'static str> {
    control_sin_datos(registros, event_ring, ep0_ring, slot_id, 0x21, 0x0B, protocolo, interface as u16)
}

/// SET_IDLE del HID (bRequest=0x0A). duracion=0 => el dispositivo solo reporta
/// cuando hay CAMBIO (no NAK-spam en reposo). Es opcional; su fallo no es fatal.
pub fn set_idle(
    registros: &mut Registers<MapeadorXhci>,
    event_ring: &mut EventRing,
    ep0_ring: &mut CommandRing,
    slot_id: u8,
    interface: u8,
) -> Result<(), &'static str> {
    // wValue = (duracion << 8) | report_id; duracion=0, report_id=0.
    control_sin_datos(registros, event_ring, ep0_ring, slot_id, 0x21, 0x0A, 0, interface as u16)
}

/// Configure Endpoint Command (TRB type 12). Le dice al HC que active los
/// endpoints descritos en el Input Context `input_ctx_fisica` (add-context
/// flags) sobre `slot_id`. Tras esto el endpoint de interrupcion del raton
/// tiene su Transfer Ring vivo y acepta Normal TRBs.
pub fn configure_endpoint(
    registros: &mut Registers<MapeadorXhci>,
    command_ring: &mut CommandRing,
    event_ring: &mut EventRing,
    slot_id: u8,
    input_ctx_fisica: u64,
) -> Result<(), &'static str> {
    let mut trb = [0u32; 4];
    trb[0] = (input_ctx_fisica & 0xFFFFFFFF) as u32;
    trb[1] = ((input_ctx_fisica >> 32) & 0xFFFFFFFF) as u32;
    // dword3 = (slot_id << 24) | (Type=12 << 10) | cycle. DC=0 (no deconfig).
    trb[3] = ((slot_id as u32) << 24) | (trb_tipo::CONFIGURE_ENDPOINT_COMMAND << 10);
    let trb_fisica = command_ring.encolar(trb);

    registros.doorbell.update_volatile_at(0, |db| {
        db.set_doorbell_target(0);
        db.set_doorbell_stream_id(0);
    });

    let evento = poll_evento(
        registros,
        event_ring,
        trb_tipo::COMMAND_COMPLETION_EVENT,
        trb_fisica,
    )?;
    let completion_code = ((evento[2] >> 24) & 0xFF) as u8;
    if completion_code != completion::SUCCESS {
        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: Configure Endpoint fallido :: code={}",
            completion_code,
        );
        return Err("xhci :: Configure Endpoint completion != SUCCESS");
    }
    Ok(())
}
