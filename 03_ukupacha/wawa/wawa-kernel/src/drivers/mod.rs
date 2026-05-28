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
//    * `gpu`     — el adaptador virtio-gpu sobre PCI: el kernel crea su propio
//                  recurso 2D y toma posesion del scanout (Fase 60).
//    * `tableta` — un virtio-input configurado como tableta: puntero ABSOLUTO
//                  que sigue 1:1 al cursor del host, complementa al PS/2 (Fase 61).
//    * `sonido`  — un virtio-sound sobre PCI: PCM real por DMA que reemplaza la
//                  bocina del PIT; sintetiza la onda desde la frecuencia (Fase 62).
// =============================================================================

pub mod altavoz;
pub mod disco;
pub mod gpu;
pub mod pci;
pub mod raton;
pub mod red;
pub mod sonido;
pub mod tableta;
// Fase 38 :: COM1 polling — canal LEGACY del firmador externo (wawactl).
// La Fase 49 corono el HAL bajo virtio-console; este modulo persiste como
// fallback para escenarios donde QEMU no expone un virtconsole.
pub mod serial;
// Fase 49 :: VirtIO Console — canal de alta velocidad sobre PCI moderno.
// Espeja la API de `serial` pero enrutado por el mismo transporte que
// gobierna virtio-blk y virtio-net.
pub mod consola_virtio;
