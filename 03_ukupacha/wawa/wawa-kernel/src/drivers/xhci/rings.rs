// =============================================================================
//  renaser :: kernel/src/drivers/xhci/rings.rs — Command + Event + ERST
// -----------------------------------------------------------------------------
//  XHCI usa tres clases de anillos de TRBs (Transfer Request Blocks, 16 B
//  cada uno):
//
//    * Command Ring (driver → HC): cola de comandos como Enable Slot,
//      Address Device, Configure Endpoint. El driver enqueua TRBs y toca
//      el doorbell #0; el HC los consume y publica Command Completion
//      Events en el Event Ring.
//    * Event Ring (HC → driver): el HC publica eventos (transfer completion,
//      port change, command completion). Un solo segmento de TRBs apuntado
//      por la Event Ring Segment Table (ERST).
//    * Transfer Rings (driver → HC, uno por endpoint): viven con cada device.
//      Los allocamos en X2d, no aqui.
//
//  Cada TRB tiene un bit «Cycle» que el productor invierte al dar la vuelta
//  por el ring. El HC consume mientras el cycle bit coincida con el suyo.
//  Cycle bit inicial: 1 (Producer Cycle State) en Command/Transfer Rings;
//  el HC arranca con su Consumer Cycle State tambien en 1.
//
//  Tamanos: 256 TRBs × 16 B = 4 KiB exactos por ring. Cabe en un marco.
// =============================================================================

use core::fmt::Write;

use alloc::vec::Vec;
use spin::Mutex;
use xhci::Registers;

use super::dma::BuferDma;
use super::mapeo::MapeadorXhci;

/// Numero de TRBs por anillo. Multiplo de 4 KiB / 16 — 256 TRBs encajan
/// exactos en una pagina. La spec exige al menos un Link TRB de cierre,
/// pero el HC tambien soporta rings circulares sin Link si el segmento no
/// se desborda. Por simplicidad usamos Link en el slot final del Command
/// Ring; el Event Ring no usa Link (la ERST lo cubre).
pub const TRBS_POR_ANILLO: usize = 256;
const TAM_TRB: usize = 16;

/// Anillo de comandos. El driver enqueua TRBs y patea el doorbell 0; el HC
/// publica el resultado en el Event Ring. Single-producer, no necesita
/// sincronizacion mas alla del Mutex global del controlador.
pub struct CommandRing {
    bufer: BuferDma,
    /// Indice del proximo slot libre para encolar. X2d lo consume al
    /// poner el primer Enable Slot TRB.
    #[allow(dead_code)]
    enqueue: usize,
    /// Producer Cycle State. Se invierte cada vez que el ring da la vuelta.
    cycle: bool,
}

impl CommandRing {
    /// Asigna un nuevo Command Ring, deja el Link TRB de cierre apuntando al
    /// principio (para que el HC sepa volver), y devuelve el anillo con
    /// cursor en 0 y cycle=1.
    pub fn nuevo() -> Option<Self> {
        let mut bufer = BuferDma::asignar_zero(TRBS_POR_ANILLO * TAM_TRB)?;
        // Programar el TRB final como Link TRB que apunta al inicio del
        // mismo segmento (ring circular). Esto NO es estrictamente
        // necesario si nunca llegamos al ultimo slot, pero es la forma
        // canonica de cerrar el anillo segun la spec.
        let inicio_fisica = bufer.fisica;
        // SEGURIDAD: `como_slice_mut` da acceso exclusivo a la pagina; el
        // slot final cae en bytes (TRBS_POR_ANILLO-1)*16 .. *16+16.
        let slice = bufer.como_slice_mut();
        let off = (TRBS_POR_ANILLO - 1) * TAM_TRB;
        // Link TRB layout (16 bytes):
        //   bytes 0..8  = ring segment pointer (low 32 + high 32)
        //   bytes 8..12 = reserved (low) + Interrupter Target (high 22..31)
        //   bytes 12..14 = control: bit0 cycle, bit1 TC=1 (Toggle Cycle),
        //                  bit10..15 = type (6 = Link TRB)
        slice[off..off + 8].copy_from_slice(&inicio_fisica.to_le_bytes());
        let control: u32 = (1 << 1) | (6 << 10); // TC=1, type=Link(6)
        slice[off + 12..off + 16].copy_from_slice(&control.to_le_bytes());
        // El cycle bit del Link queda en 0 — el HC se lo conseguira al pasar
        // la primera vez. En la siguiente vuelta nuestro driver lo invertira.

        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: command ring fisica={:#x} TRBs={}",
            bufer.fisica,
            TRBS_POR_ANILLO,
        );
        Some(CommandRing {
            bufer,
            enqueue: 0,
            cycle: true,
        })
    }

    /// Direccion fisica del primer TRB. La que se programa en CRCR.
    pub fn fisica(&self) -> u64 {
        self.bufer.fisica
    }

    /// Producer Cycle State actual — para programar CRCR.RCS al arrancar.
    pub fn cycle(&self) -> bool {
        self.cycle
    }

    /// Encola un TRB en el slot del enqueue actual. El TRB se pasa como
    /// `[u32; 4]` sin cycle bit — el ring lo aplica segun su Producer
    /// Cycle State y avanza el cursor. Devuelve la direccion fisica del
    /// TRB encolado (util para encontrar el evento de completion).
    ///
    /// Cycle bit: bit 0 del dword[3]. Lo aplicamos OR-ing tras la copia.
    /// Si llegamos al Link TRB del final, el HC nos lleva de vuelta al
    /// principio y nuestro cycle bit se invierte.
    pub fn encolar(&mut self, mut trb: [u32; 4]) -> u64 {
        // Aplicar el cycle bit.
        if self.cycle {
            trb[3] |= 1;
        } else {
            trb[3] &= !1;
        }
        let off = self.enqueue * TAM_TRB;
        let slice = self.bufer.como_slice_mut();
        for (i, &dword) in trb.iter().enumerate() {
            slice[off + i * 4..off + i * 4 + 4].copy_from_slice(&dword.to_le_bytes());
        }
        let trb_fisica = self.bufer.fisica + off as u64;
        // Avanzar el cursor. Si el siguiente slot es el Link TRB del
        // final, dar la vuelta al principio e invertir el cycle.
        self.enqueue += 1;
        if self.enqueue == TRBS_POR_ANILLO - 1 {
            // El Link TRB ya tiene TC=1 — al cruzarlo el HC invierte su
            // Consumer Cycle State y el nuestro debe seguirlo.
            self.enqueue = 0;
            self.cycle = !self.cycle;
        }
        trb_fisica
    }
}

/// Event Ring + ERST (Event Ring Segment Table). Un solo segmento, lo mas
/// simple posible. ERST con una entrada de 16 bytes que apunta al Event
/// Ring de 256 TRBs.
pub struct EventRing {
    erst: BuferDma,
    ring: BuferDma,
    /// Indice del proximo slot a consumir. El driver lo avanza al leer
    /// eventos; el HC mantiene su productor por dentro.
    #[allow(dead_code)]
    dequeue: usize,
    /// Consumer Cycle State del driver. Empieza en 1; se invierte al dar
    /// la vuelta.
    #[allow(dead_code)]
    cycle: bool,
}

impl EventRing {
    /// Asigna ERST de 1 entrada + Event Ring de 256 TRBs, programa la ERST
    /// para que apunte al ring, devuelve la estructura. Despues hay que
    /// programar ERSTSZ=1, ERSTBA con la fisica de la ERST y ERDP con la
    /// fisica del primer TRB del ring.
    pub fn nuevo() -> Option<Self> {
        let mut erst = BuferDma::asignar_zero(64)?; // 1 segmento * 16 B alineado a 64.
        let ring = BuferDma::asignar_zero(TRBS_POR_ANILLO * TAM_TRB)?;

        // ERST entry layout (16 bytes):
        //   bytes 0..8  = ring segment base address (64-bit)
        //   bytes 8..10 = ring segment size (TRBs en este segmento)
        //   bytes 10..16 = reserved
        let slice = erst.como_slice_mut();
        slice[0..8].copy_from_slice(&ring.fisica.to_le_bytes());
        let size: u16 = TRBS_POR_ANILLO as u16;
        slice[8..10].copy_from_slice(&size.to_le_bytes());

        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: event ring fisica={:#x} ERST fisica={:#x} TRBs={}",
            ring.fisica,
            erst.fisica,
            TRBS_POR_ANILLO,
        );

        Some(EventRing {
            erst,
            ring,
            dequeue: 0,
            cycle: true,
        })
    }

    pub fn erst_fisica(&self) -> u64 {
        self.erst.fisica
    }

    pub fn ring_fisica(&self) -> u64 {
        self.ring.fisica
    }

    /// Devuelve la direccion fisica del TRB en el dequeue actual. Util
    /// para programar ERDP tras consumir un evento.
    pub fn dequeue_fisica(&self) -> u64 {
        self.ring.fisica + (self.dequeue * TAM_TRB) as u64
    }

    /// Lee el TRB en el dequeue actual y devuelve sus 4 dwords si el cycle
    /// bit coincide con nuestro Consumer Cycle State (= evento valido).
    /// `None` si todavia no hay evento — el HC no lo escribio.
    ///
    /// IMPORTANTE: la responsabilidad de avanzar dequeue y actualizar ERDP
    /// vive en el caller — algunos esquemas leen varios eventos antes de
    /// reprogramar ERDP en bloque.
    pub fn leer(&self) -> Option<[u32; 4]> {
        let off = self.dequeue * TAM_TRB;
        let slice = unsafe {
            core::slice::from_raw_parts(self.ring.virtual_.as_ptr(), self.ring.bytes)
        };
        let mut dwords = [0u32; 4];
        for i in 0..4 {
            dwords[i] = u32::from_le_bytes([
                slice[off + i * 4],
                slice[off + i * 4 + 1],
                slice[off + i * 4 + 2],
                slice[off + i * 4 + 3],
            ]);
        }
        // El cycle bit del TRB es bit 0 del dword[3]. Si coincide con el
        // nuestro, el HC lo publico y es valido.
        let trb_cycle = (dwords[3] & 1) == 1;
        if trb_cycle == self.cycle {
            Some(dwords)
        } else {
            None
        }
    }

    /// Avanza el dequeue al siguiente slot, dando la vuelta + invirtiendo
    /// el cycle si llegamos al final del segmento. El Event Ring no usa
    /// Link TRBs — la spec exige un solo segmento sin Link, y la vuelta
    /// la hace la propia mecanica del consumer.
    pub fn avanzar(&mut self) {
        self.dequeue += 1;
        if self.dequeue == TRBS_POR_ANILLO {
            self.dequeue = 0;
            self.cycle = !self.cycle;
        }
    }
}

/// Device Context Base Address Array. Una entrada por slot (mas slot 0 si
/// hay scratchpad). Cada entrada apunta a un Device Context. Las entradas
/// se llenan en X2d cuando se asignan slots; al arrancar todas son ceros
/// (slot inutilizado).
pub struct Dcbaa {
    bufer: BuferDma,
    /// Numero de slots habilitados (CONFIG.MaxSlotsEn). Incluye slot 0
    /// reservado, asi que el array tiene `max_slots + 1` entradas.
    #[allow(dead_code)]
    max_slots: u8,
}

impl Dcbaa {
    /// Asigna el DCBAA dimensionado a `max_slots + 1` entradas de 8 bytes.
    /// El slot 0 se reserva para el Scratchpad Buffer Array si el
    /// controlador exige scratchpad (HCSPARAMS2.Max_Scratchpad_Buffers > 0,
    /// caso comun en chipsets Intel modernos). X2c basica deja el slot 0 en
    /// cero — si el controlador realmente necesita scratchpad lo tratamos
    /// en una iteracion posterior.
    pub fn nuevo(max_slots: u8) -> Option<Self> {
        let bytes = ((max_slots as usize) + 1) * 8;
        let bufer = BuferDma::asignar_zero(bytes)?;
        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: DCBAA fisica={:#x} slots={}",
            bufer.fisica,
            max_slots,
        );
        Some(Dcbaa { bufer, max_slots })
    }

    pub fn fisica(&self) -> u64 {
        self.bufer.fisica
    }

    /// Acceso mutable al slice del DCBAA — para que `contextos::registrar_en_dcbaa`
    /// pueda escribir el pointer al Device Context de cada slot enumerado.
    pub fn bufer_mut(&mut self) -> &mut [u8] {
        self.bufer.como_slice_mut()
    }
}

/// Conjunto de estructuras DMA que el controlador necesita para arrancar.
/// Vive detras del Mutex global del driver xHCI; lo construye `montar` en
/// X2c y lo consumen las fases X2d (port enumeration) en adelante.
#[allow(dead_code)]
pub struct EstructurasArranque {
    pub command_ring: CommandRing,
    pub event_ring: EventRing,
    pub dcbaa: Dcbaa,
    /// Buferes de scratchpad que el HC exige (HCSPARAMS2.MaxScratchpadBufs).
    /// Incluye el Scratchpad Buffer Array y los N marcos que apunta. Se guardan
    /// SOLO para mantenerlos vivos —el HC los usa como almacen privado—; el
    /// driver no los toca tras programar DCBAA[0]. Vacio si el HC no pide
    /// scratchpad (caso de QEMU).
    scratchpad: Vec<BuferDma>,
}

/// Reserva los buferes de scratchpad que el controlador exige y deja
/// DCBAA[0] = Scratchpad Buffer Array. Muchos xHCI reales (Intel/AMD) NO
/// arrancan tras USBCMD.RS=1 si este array no esta puesto; QEMU pide 0 y
/// devuelve Vec vacio. Spec xHCI §4.20.
///
/// El array es N punteros fisicos de 64 bits a N marcos de 4 KiB. Devuelve
/// todos los `BuferDma` (los N marcos + el array) para que el caller los
/// conserve vivos lo que viva el controlador.
fn fundar_scratchpad(
    registros: &mut Registers<MapeadorXhci>,
    dcbaa: &mut Dcbaa,
) -> Vec<BuferDma> {
    let n = registros
        .capability
        .hcsparams2
        .read_volatile()
        .max_scratchpad_buffers();
    let mut bufers: Vec<BuferDma> = Vec::new();
    if n == 0 {
        return bufers;
    }

    let mut array = match BuferDma::asignar_zero(n as usize * 8) {
        Some(b) => b,
        None => {
            let _ = writeln!(crate::baliza::Serie, "xhci :: scratchpad array sin DMA");
            return bufers;
        }
    };
    for i in 0..n as usize {
        let buf = match BuferDma::asignar_zero(4096) {
            Some(b) => b,
            None => {
                let _ = writeln!(crate::baliza::Serie, "xhci :: scratchpad buf {i} sin DMA");
                break;
            }
        };
        let off = i * 8;
        array.como_slice_mut()[off..off + 8].copy_from_slice(&buf.fisica.to_le_bytes());
        bufers.push(buf);
    }
    // DCBAA[0] apunta al Scratchpad Buffer Array.
    dcbaa.bufer_mut()[0..8].copy_from_slice(&array.fisica.to_le_bytes());
    let _ = writeln!(
        crate::baliza::Serie,
        "xhci :: scratchpad :: {} buferes, array fisica={:#x}",
        n,
        array.fisica,
    );
    bufers.push(array);
    bufers
}

impl EstructurasArranque {
    /// Aloca todo, programa los registros del controlador para que apunten a
    /// estas estructuras, deja el Interrupter 0 activo y enciende USBCMD.RS=1.
    /// Devuelve la coleccion para que el driver la conserve viva.
    pub fn fundar(
        registros: &mut Registers<MapeadorXhci>,
        max_slots: u8,
    ) -> Result<Self, &'static str> {
        let command_ring =
            CommandRing::nuevo().ok_or("xhci :: arena DMA exhausta para command ring")?;
        let event_ring =
            EventRing::nuevo().ok_or("xhci :: arena DMA exhausta para event ring")?;
        let mut dcbaa = Dcbaa::nuevo(max_slots).ok_or("xhci :: arena DMA exhausta para DCBAA")?;

        // Scratchpad ANTES de programar DCBAAP/RS: en metal el HC puede no
        // arrancar sin DCBAA[0] = Scratchpad Buffer Array.
        let scratchpad = fundar_scratchpad(registros, &mut dcbaa);

        // CONFIG.MaxSlotsEn — habilitar los slots que el HC soporta.
        registros.operational.config.update_volatile(|c| {
            c.set_max_device_slots_enabled(max_slots);
        });

        // DCBAAP — fisica del DCBAA. La spec exige 64-byte alignment;
        // nuestros marcos son 4 KiB-aligned, asi que sobra.
        registros.operational.dcbaap.update_volatile(|p| {
            p.set(dcbaa.fisica());
        });

        // CRCR — base del Command Ring + Ring Cycle State + Command Stop=0.
        registros.operational.crcr.update_volatile(|c| {
            c.set_command_ring_pointer(command_ring.fisica());
            if command_ring.cycle() {
                c.set_ring_cycle_state();
            } else {
                c.clear_ring_cycle_state();
            }
        });

        // Interrupter 0 :: ERSTSZ + ERSTBA + ERDP. Programar en este orden
        // — la spec es estricta: ERSTBA es la trigger que activa el Event
        // Ring, asi que debe venir DESPUES de ERSTSZ y ERDP.
        let mut interrupter = registros.interrupter_register_set.interrupter_mut(0);
        interrupter.erstsz.update_volatile(|s| {
            s.set(1); // un solo segmento.
        });
        interrupter.erdp.update_volatile(|d| {
            d.set_event_ring_dequeue_pointer(event_ring.ring_fisica());
        });
        interrupter.erstba.update_volatile(|b| {
            b.set(event_ring.erst_fisica());
        });
        // Habilitar interrupts del interrupter 0. Aun no enrutamos su IRQ
        // a la IDT (eso es X2d), pero IMAN.IE=1 es prerrequisito para que
        // el HC tan siquiera arme la condicion.
        interrupter.iman.update_volatile(|i| {
            i.set_interrupt_enable();
        });

        // USBCMD.RS=1 — arrancar el controlador. La spec exige verificar
        // que USBSTS.HCHalted baje despues.
        registros.operational.usbcmd.update_volatile(|u| {
            u.set_run_stop();
        });

        const MAX_INTENTOS: u32 = 100_000_000;
        let mut intentos = 0;
        while registros.operational.usbsts.read_volatile().hc_halted() {
            intentos += 1;
            if intentos >= MAX_INTENTOS {
                return Err("xhci :: HCHalted no bajo tras USBCMD.RS=1");
            }
            core::hint::spin_loop();
        }

        let _ = writeln!(
            crate::baliza::Serie,
            "xhci :: corriendo (USBCMD.RS=1, HCHalted=0)",
        );

        Ok(EstructurasArranque {
            command_ring,
            event_ring,
            dcbaa,
            scratchpad,
        })
    }
}

// SEGURIDAD: las estructuras DMA viven en marcos exclusivos del kernel; el
// Mutex global del driver xHCI serializa el acceso. Send es legal.
unsafe impl Send for EstructurasArranque {}

#[allow(dead_code)]
pub type EstructurasMutex = Mutex<EstructurasArranque>;
