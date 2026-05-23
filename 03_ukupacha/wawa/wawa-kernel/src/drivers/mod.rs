// =============================================================================
//  renaser :: kernel/src/drivers — Fase 6.1 :: el puente hacia el hardware real
// -----------------------------------------------------------------------------
//  Hasta aqui renaser solo tocaba el silicio que el firmware le servia en
//  bandeja: el framebuffer GOP, el temporizador, el teclado. Los `drivers`
//  abren la primera via hacia hardware que el kernel debe DESCUBRIR y reclamar
//  por si mismo:
//
//    * `pci`     — acceso al espacio de configuracion del bus PCI (0xCF8/0xCFC).
//    * `disco`   — el disco virtio-blk: asignador de marcos DMA, `Hal` y la
//                  lectura, por sondeo, de su primer sector.
//    * `altavoz` — la bocina del PC: el canal 2 del PIT como generador de tono
//                  (Fase 12).
//    * `raton`   — el raton PS/2: el dispositivo auxiliar del 8042 + IRQ12,
//                  paquetes de 3 bytes (Fase 13).
//    * `red`     — la tarjeta virtio-net sobre PCI: ethernet crudo,
//                  primer ARP al gateway de QEMU (Fase 18).
// =============================================================================

pub mod altavoz;
pub mod disco;
pub mod pci;
pub mod raton;
pub mod red;
