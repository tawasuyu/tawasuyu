// =============================================================================
//  renaser :: kernel/src/drivers/xhci/hid.rs — Fase X3 :: el raton USB (HID)
// -----------------------------------------------------------------------------
//  Un raton USB es un dispositivo HID (Human Interface Device, clase 0x03) que
//  entrega sus movimientos por un endpoint de INTERRUPCION IN, en reportes de
//  3-4 bytes. En BOOT PROTOCOL ese reporte es fijo y no hay que parsear el
//  Report Descriptor:
//
//    byte 0 = botones  (bit0 izquierdo, bit1 derecho, bit2 central)
//    byte 1 = dx       (i8, relativo, + a la derecha)
//    byte 2 = dy       (i8, relativo, + hacia abajo)
//    byte 3 = rueda    (i8, opcional — lo ignoramos)
//
//  Sobre el controlador XHCI ya vivo (controlador.rs) y un dispositivo ya
//  addresado (Address Device + descriptores), este modulo:
//
//    1. SET_CONFIGURATION — activa la config del dispositivo.
//    2. CONFIGURE_ENDPOINT — añade el endpoint de interrupcion IN con su
//       Transfer Ring propio (DCI = 2*numero+1).
//    3. SET_PROTOCOL(boot) + SET_IDLE(0) — reporte estandar, solo-al-cambiar.
//    4. ARMA un Normal TRB en el ring de interrupcion y patea su doorbell.
//
//  Luego `atender()` —llamado cada fotograma desde el reactor— DRENA el Event
//  Ring de forma NO BLOQUEANTE: si hay un Transfer Event de nuestro endpoint,
//  lee el reporte, lo entrega a `drivers::raton` como delta relativo y RE-ARMA
//  el siguiente Normal TRB. Si no hay evento, vuelve enseguida — un raton
//  quieto no cuesta CPU. (El polling bloqueante de la enumeracion no sirve
//  aqui: colgaria el reactor hasta 1 s esperando un movimiento que no llega.)
// =============================================================================

use core::fmt::Write;

use xhci::Registers;

use super::comandos;
use super::contextos::ContextoDispositivo;
use super::descriptores::{Configuracion, Endpoint};
use super::dma::BuferDma;
use super::mapeo::MapeadorXhci;
use super::rings::{CommandRing, EventRing};

/// TRB Type del Normal TRB (transferencia de datos en un ring que no es de
/// control). Spec §6.4.1.1.
const TRB_NORMAL: u32 = 1;
/// TRB Type del Transfer Event. Spec §6.4.2.1.
const TRB_TRANSFER_EVENT: u32 = 32;
/// Completion codes que tratamos como exito en una transferencia de interrupt.
const CC_SUCCESS: u8 = 1;
const CC_SHORT_PACKET: u8 = 13;

/// El raton USB ya configurado y armado. Vive en `controlador::Estado` mientras
/// el dispositivo este conectado. `atender()` lo consume cada fotograma.
pub struct RatonHid {
    /// Slot asignado por el HC al dispositivo.
    slot_id: u8,
    /// Device Context Index del endpoint de interrupcion IN — tambien el
    /// target del doorbell para armarlo.
    dci: u8,
    /// Transfer Ring del endpoint de interrupcion. Reusa `CommandRing` (que
    /// es un ring generico de TRBs con Link de cierre y cycle bit).
    ring: CommandRing,
    /// Buffer DMA donde el HC deposita cada reporte. >= MPS bytes.
    bufer: BuferDma,
    /// Max Packet Size del endpoint — la longitud que pedimos por TRB.
    mps: u16,
    /// Direccion fisica del Normal TRB actualmente armado — el Transfer Event
    /// la referencia, asi sabemos que el reporte que esperabamos llego.
    armado: u64,
}

// SEGURIDAD: igual que el resto del estado XHCI — un solo nucleo, todo acceso
// pasa por el Mutex global del controlador. Send es legal.
unsafe impl Send for RatonHid {}

/// Codifica el campo Interval del EP Context (unidades log2 de 125 µs) a partir
/// de la velocidad del puerto y el `bInterval` del Endpoint Descriptor.
///   * High/Super Speed interrupt: bInterval ya es un exponente 2^(n-1)·125µs →
///     Interval = bInterval - 1.
///   * Full/Low Speed interrupt: bInterval esta en frames (1 ms = 2^3·125µs) →
///     Interval = 3 + floor(log2(bInterval)).
/// Se acota a 0..=15. Un valor algo distinto del ideal no rompe nada: el HC
/// poleara el endpoint a un ritmo cercano.
fn intervalo_xhci(velocidad: u8, b_interval: u8) -> u8 {
    match velocidad {
        // 3=High, 4/5=Super (mismo encoding que contextos::nuevo).
        3 | 4 | 5 => b_interval.saturating_sub(1).min(15),
        // 1=Full, 2=Low (y cualquier otro): bInterval en ms.
        _ => {
            let b = b_interval.max(1) as u32;
            let log2 = 31 - b.leading_zeros(); // floor(log2(b))
            (3 + log2).min(15) as u8
        }
    }
}

impl RatonHid {
    /// Configura un raton HID ya addresado y devuelve el driver listo y armado.
    /// `velocidad` es la del puerto raiz; `interface`/`in_ep` salen de
    /// `descriptores::buscar_raton_hid`.
    #[allow(clippy::too_many_arguments)]
    pub fn configurar(
        registros: &mut Registers<MapeadorXhci>,
        command_ring: &mut CommandRing,
        event_ring: &mut EventRing,
        ctx: &mut ContextoDispositivo,
        slot_id: u8,
        velocidad: u8,
        config: &Configuracion,
        interface: u8,
        in_ep: &Endpoint,
    ) -> Result<Self, &'static str> {
        // 1. SET_CONFIGURATION — sin esto los endpoints no-EP0 estan muertos.
        comandos::set_configuration(registros, event_ring, &mut ctx.ep0_ring, slot_id, config.valor)?;

        // 2. CONFIGURE_ENDPOINT — añadir el interrupt IN con su Transfer Ring.
        let dci = in_ep.dci();
        let mps = in_ep.max_packet_size;
        let ring = CommandRing::nuevo().ok_or("xhci/hid :: arena DMA exhausta para interrupt ring")?;
        let intervalo = intervalo_xhci(velocidad, in_ep.intervalo);
        let input_fisica = ctx.preparar_interrupt_in(dci, mps, ring.fisica(), intervalo);
        comandos::configure_endpoint(registros, command_ring, event_ring, slot_id, input_fisica)?;

        // 3. Boot protocol + idle infinito (solo-al-cambiar). Best-effort: si el
        //    raton no soporta estos class requests, seguimos — muchos reportan
        //    boot por defecto igual.
        if let Err(m) = comandos::set_protocolo(registros, event_ring, &mut ctx.ep0_ring, slot_id, interface, 0) {
            let _ = writeln!(crate::baliza::Serie, "xhci/hid :: SET_PROTOCOL(boot) no honrado :: {m}");
        }
        if let Err(m) = comandos::set_idle(registros, event_ring, &mut ctx.ep0_ring, slot_id, interface) {
            let _ = writeln!(crate::baliza::Serie, "xhci/hid :: SET_IDLE no honrado :: {m}");
        }

        // Buffer de reporte — al menos 8 bytes, redondea a MPS.
        let bytes = (mps as usize).max(8);
        let bufer = BuferDma::asignar_zero(bytes).ok_or("xhci/hid :: arena DMA exhausta para buffer de reporte")?;

        let mut raton = RatonHid {
            slot_id,
            dci,
            ring,
            bufer,
            mps,
            armado: 0,
        };

        // 4. Armar la primera transferencia.
        raton.armar(registros);

        let _ = writeln!(
            crate::baliza::Serie,
            "xhci/hid :: raton USB configurado :: slot={} dci={} mps={} intervalo={}",
            slot_id, dci, mps, intervalo,
        );
        Ok(raton)
    }

    /// Encola un Normal TRB que pide `mps` bytes al buffer de reporte y patea el
    /// doorbell del endpoint. IOC=1 (queremos el Transfer Event) e ISP=1
    /// (interrumpir tambien en short packet — el reporte suele ser < MPS).
    fn armar(&mut self, registros: &mut Registers<MapeadorXhci>) {
        let mut trb = [0u32; 4];
        trb[0] = (self.bufer.fisica & 0xFFFFFFFF) as u32;
        trb[1] = ((self.bufer.fisica >> 32) & 0xFFFFFFFF) as u32;
        // dword2 = TRB Transfer Length (bits 0..16) | Interrupter Target (22..31=0).
        trb[2] = self.mps as u32;
        // dword3 = (Type=1 << 10) | IOC(bit5) | ISP(bit2). El cycle lo pone el ring.
        trb[3] = (TRB_NORMAL << 10) | (1 << 5) | (1 << 2);
        self.armado = self.ring.encolar(trb);

        registros.doorbell.update_volatile_at(self.slot_id as usize, |db| {
            db.set_doorbell_target(self.dci);
            db.set_doorbell_stream_id(0);
        });
    }

    /// Drena el Event Ring SIN bloquear: por cada Transfer Event de nuestro
    /// endpoint, lee el reporte, lo entrega a `drivers::raton` como delta
    /// relativo y RE-ARMA. Eventos ajenos se consumen y descartan. Vuelve en
    /// cuanto el ring no tiene mas eventos pendientes.
    pub fn atender(
        &mut self,
        registros: &mut Registers<MapeadorXhci>,
        event_ring: &mut EventRing,
    ) {
        // Tope de seguridad por si el ring se llenara de eventos ajenos: no
        // queremos atascar un fotograma. 64 es holgado para un raton.
        for _ in 0..64 {
            let Some(dwords) = event_ring.leer() else {
                break;
            };
            let tipo = (dwords[3] >> 10) & 0x3F;
            let ref_trb = (dwords[0] as u64) | ((dwords[1] as u64) << 32);
            let completion_code = ((dwords[2] >> 24) & 0xFF) as u8;
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

            if tipo == TRB_TRANSFER_EVENT && ref_trb == self.armado {
                if completion_code == CC_SUCCESS || completion_code == CC_SHORT_PACKET {
                    self.procesar_reporte();
                }
                // Re-armar siempre, exito o no: el endpoint debe seguir vivo.
                self.armar(registros);
            }
            // Cualquier otro evento (Port Status Change, command completion
            // tardio) se descarta — ya avanzamos el dequeue.
        }
    }

    /// Lee el buffer de reporte (boot mouse) y entrega el delta a `raton`.
    fn procesar_reporte(&self) {
        let slice = unsafe {
            core::slice::from_raw_parts(self.bufer.virtual_.as_ptr(), self.bufer.bytes)
        };
        if slice.len() < 3 {
            return;
        }
        let botones = slice[0] & 0b0000_0111; // bit0 izq, bit1 der, bit2 central.
        let dx = slice[1] as i8 as i32;
        let dy = slice[2] as i8 as i32; // + hacia abajo, igual que la pantalla.
        crate::drivers::raton::aplicar_delta_relativo(dx, dy, botones);
    }
}
