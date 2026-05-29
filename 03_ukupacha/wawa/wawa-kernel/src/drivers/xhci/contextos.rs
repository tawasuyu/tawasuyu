// =============================================================================
//  renaser :: kernel/src/drivers/xhci/contextos.rs — Slot / EP / Input Context
// -----------------------------------------------------------------------------
//  Address Device necesita TRES estructuras DMA por slot:
//
//    * Device Context (Device<N>): el HC lo escribe; el driver lo lee tras
//      Address Device para conocer el estado «running» del dispositivo.
//      Su direccion fisica se publica en DCBAA[slot_id].
//
//    * Input Context (Input<N>): el driver lo escribe ANTES de Address
//      Device para decirle al HC «este es el slot, esta es la velocidad,
//      este es el EP0 con su Transfer Ring». El HC lo lee, valida y copia
//      los campos relevantes al Device Context.
//
//    * EP0 Transfer Ring: la cola de TRBs sobre la que viajan los control
//      transfers de enumeracion (GET_DESCRIPTOR, SET_CONFIGURATION). Su
//      direccion fisica se programa en el EP0 Context del Input Context.
//
//  N=8 (32-byte contexts) o N=16 (64-byte contexts), segun `HCCPARAMS1.CSZ`.
//  QEMU xHCI y la mayoria de Intel modernos usan N=8. Este modulo asume N=8
//  por simplicidad — si en metal nos topamos con un controller que pida
//  N=16, ramificar aqui es trabajo de un dia.
// =============================================================================

use core::fmt::Write;

use xhci::context::{EndpointType, Input32Byte, InputHandler};

use super::dma::BuferDma;
use super::rings::CommandRing;

/// Tamano de un Device Context de 32-byte slots: 1024 bytes (32 slots × 32 B).
const TAM_DEVICE_CTX: usize = 1024;
/// Tamano de un Input Context de 32-byte slots: 1056 bytes (Input Control 32 B
/// + Device Context 1024 B).
const TAM_INPUT_CTX: usize = 1056;

/// Conjunto de estructuras DMA que pertenecen a UN dispositivo USB ya
/// addresado. Se llena en `preparar_para_address_device` y se conserva por
/// el driver mientras el dispositivo este conectado.
pub struct ContextoDispositivo {
    /// Buffer DMA del Device Context. Su fisica va en DCBAA[slot_id].
    pub device_ctx: BuferDma,
    /// Buffer DMA del Input Context. Se entrega al HC en Address Device y se
    /// reutiliza para Configure Endpoint.
    pub input_ctx: BuferDma,
    /// EP0 Transfer Ring: control transfers de enumeracion.
    pub ep0_ring: CommandRing,
    /// Indice del puerto raiz (0-based) y velocidad — guardados para rellenar
    /// el Slot Context tambien en Configure Endpoint, no solo en Address Device.
    puerto_idx: usize,
    velocidad: u8,
}

impl ContextoDispositivo {
    /// Asigna las tres estructuras DMA y rellena el Input Context con los
    /// campos minimos que Address Device necesita: A0=1 (Slot), A1=1 (EP0),
    /// Slot Context con Route String=0, Speed, Context Entries=1, Root Hub
    /// Port=puerto+1; EP0 Context como Control con MPS por defecto y
    /// Dequeue Pointer = EP0 Ring fisica + DCS=1.
    pub fn nuevo(
        puerto_idx: usize,
        velocidad_puerto: u8,
    ) -> Result<Self, &'static str> {
        let device_ctx = BuferDma::asignar_zero(TAM_DEVICE_CTX)
            .ok_or("xhci :: arena DMA exhausta para device context")?;
        let mut input_ctx = BuferDma::asignar_zero(TAM_INPUT_CTX)
            .ok_or("xhci :: arena DMA exhausta para input context")?;
        let ep0_ring =
            CommandRing::nuevo().ok_or("xhci :: arena DMA exhausta para EP0 ring")?;

        // Construir un Input32Byte en stack — la crate xhci nos da una API
        // tipada para sus campos. Luego lo copiamos byte-a-byte al BuferDma
        // (que es la memoria que el HC realmente lee).
        let mut input = Input32Byte::new_32byte();
        input.control_mut().set_add_context_flag(0); // A0 = Slot Context
        input.control_mut().set_add_context_flag(1); // A1 = EP0 Endpoint Context

        // Slot Context.
        let slot = input.device_mut().slot_mut();
        slot.set_route_string(0);
        slot.set_speed(velocidad_puerto);
        slot.set_context_entries(1); // solo EP0 por ahora
        slot.set_root_hub_port_number((puerto_idx + 1) as u8);

        // EP0 Context. Max Packet Size depende de la velocidad:
        //   Low Speed: 8
        //   Full Speed: 8 (sera reajustado tras leer Device Descriptor)
        //   High Speed: 64
        //   Super Speed / Plus: 512
        let mps = match velocidad_puerto {
            1 => 64, // Full Speed
            2 => 8,  // Low Speed
            3 => 64, // High Speed
            4 | 5 => 512, // SuperSpeed / Plus
            _ => 64,
        };
        let ep0 = input.device_mut().endpoint_mut(1); // DCI 1 = EP0
        ep0.set_endpoint_type(EndpointType::Control);
        ep0.set_max_packet_size(mps);
        ep0.set_tr_dequeue_pointer(ep0_ring.fisica());
        ep0.set_dequeue_cycle_state();
        ep0.set_error_count(3); // Standard default per xHCI spec.

        // Volcar al BuferDma. `Input32Byte` es `repr(C)` de [u32; 8] × 33 =
        // 1056 bytes; transmute seguro a [u8].
        let input_bytes: [u8; TAM_INPUT_CTX] = unsafe { core::mem::transmute(input) };
        let slice = input_ctx.como_slice_mut();
        slice[..TAM_INPUT_CTX].copy_from_slice(&input_bytes);

        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: contexto dispositivo :: device={:#x} input={:#x} ep0_ring={:#x} mps={}",
            device_ctx.fisica,
            input_ctx.fisica,
            ep0_ring.fisica(),
            mps,
        );

        Ok(ContextoDispositivo {
            device_ctx,
            input_ctx,
            ep0_ring,
            puerto_idx,
            velocidad: velocidad_puerto,
        })
    }

    /// Reescribe el Input Context para un Configure Endpoint Command que AÑADE
    /// un endpoint de INTERRUPCION IN (el del raton HID). Devuelve la fisica del
    /// Input Context, lista para `comandos::configure_endpoint`.
    ///
    /// Flags de add-context: A0 (Slot) + A_dci (el endpoint nuevo). El Slot
    /// Context se rellena de nuevo (ruta, velocidad, puerto) con Context Entries
    /// = dci, porque el HC valida el slot completo. El EP Context queda como
    /// Interrupt IN con su Transfer Ring, MPS, intervalo y error count.
    ///
    /// `dci` = 2*numero+1 para un IN. `ring_fisica` = base del Transfer Ring del
    /// endpoint (con DCS=1). `intervalo_xhci` ya codificado en unidades de
    /// 125 µs (campo Interval del EP Context), calculado por el caller segun la
    /// velocidad y el bInterval del descriptor.
    pub fn preparar_interrupt_in(
        &mut self,
        dci: u8,
        max_packet_size: u16,
        ring_fisica: u64,
        intervalo_xhci: u8,
    ) -> u64 {
        let mut input = Input32Byte::new_32byte();
        input.control_mut().set_add_context_flag(0); // A0 = Slot Context.
        input.control_mut().set_add_context_flag(dci as usize); // A_dci = EP.

        let slot = input.device_mut().slot_mut();
        slot.set_route_string(0);
        slot.set_speed(self.velocidad);
        slot.set_context_entries(dci); // el DCI mas alto configurado.
        slot.set_root_hub_port_number((self.puerto_idx + 1) as u8);

        let ep = input.device_mut().endpoint_mut(dci as usize);
        ep.set_endpoint_type(EndpointType::InterruptIn);
        ep.set_max_packet_size(max_packet_size);
        ep.set_max_burst_size(0);
        ep.set_tr_dequeue_pointer(ring_fisica);
        ep.set_dequeue_cycle_state();
        ep.set_interval(intervalo_xhci);
        ep.set_error_count(3);
        ep.set_average_trb_length(8); // reportes HID son chicos.

        let input_bytes: [u8; TAM_INPUT_CTX] = unsafe { core::mem::transmute(input) };
        let slice = self.input_ctx.como_slice_mut();
        slice[..TAM_INPUT_CTX].copy_from_slice(&input_bytes);

        self.input_ctx.fisica
    }
}

/// Programa DCBAA[slot_id] = device_ctx_fisica. La DCBAA vive en el
/// `EstructurasArranque.dcbaa`.
pub fn registrar_en_dcbaa(
    dcbaa: &mut super::rings::Dcbaa,
    slot_id: u8,
    device_ctx_fisica: u64,
) {
    let off = (slot_id as usize) * 8;
    let slice = dcbaa.bufer_mut();
    slice[off..off + 8].copy_from_slice(&device_ctx_fisica.to_le_bytes());
}
