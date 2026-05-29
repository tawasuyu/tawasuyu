// =============================================================================
//  renaser :: kernel/src/drivers/xhci — Fase X2 :: el controlador host USB 3.x
// -----------------------------------------------------------------------------
//  XHCI (eXtensible Host Controller Interface) es el estandar que rige a TODOS
//  los controladores USB modernos —USB 3.x nativo, retrocompatible con USB 2.0
//  y 1.x—. Esta capa lo descubre via PCI (clase 0x0C/0x03/0x30), mapea su BAR
//  de registros y le da vida. Sobre ella se montan despues:
//
//    * USB-MS (Bulk-Only Transport + SCSI) — almacenamiento, para que el
//      kernel pueda leer/escribir un USB stick real en metal sin depender
//      del ramdisk del bootloader (capa R).
//    * USB HID — raton (y luego teclado) para maquinas cuyo UEFI NO emula el
//      raton USB sobre el i8042. La del usuario es de esas: su trackpad PS/2
//      anda, pero el raton USB no llega por la via legacy — necesita este
//      driver HID nativo (Fase X3, `hid`).
//
//  Submodulos:
//    * `mapeo` — implementacion del `accessor::Mapper` que la crate `xhci`
//      consume para mapear lazy el MMIO del BAR0.
//    * `controlador` — punto de entrada: descubrir XHCI via PCI, mapear el
//      BAR, instanciar `xhci::Registers`, leer capacidades, reset, DCBAA,
//      Command/Event rings, enumeracion de puertos y dispositivos.
//    * `hid` — driver del raton USB sobre el endpoint de interrupcion (X3).
//
//  Estado: X3 — controlador vivo + enumeracion + raton HID (boot protocol)
//  entregando deltas a `drivers::raton`. Pendiente: USB-MS, teclado HID, IRQ
//  del XHCI (hoy el raton se polea por fotograma desde el reactor).
// =============================================================================

pub mod comandos;
pub mod contextos;
pub mod controlador;
pub mod descriptores;
pub mod dma;
pub mod hid;
pub mod mapeo;
pub mod puertos;
pub mod rings;
