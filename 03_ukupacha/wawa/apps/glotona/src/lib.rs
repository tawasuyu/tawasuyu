// =============================================================================
//  renaser :: apps/glotona — Fase 6.0 :: el inquilino voraz del userspace
// -----------------------------------------------------------------------------
//  Esta aplicacion esta MAL a proposito, como su hermana `discola` — pero en la
//  otra dimension. Donde `discola` devora TIEMPO (un bucle sin fin), `glotona`
//  devora ESPACIO: su `tick` invoca `memory.grow` reclamando memoria lineal sin
//  freno alguno.
//
//  renaser le impone a cada modulo un techo de memoria. Cuando esta peticion lo
//  rebasa, el runtime `wasmi` —configurado para ello— lanza una trampa en vez
//  de devolver un discreto -1; el kernel la captura, desaloja a este modulo y
//  tiñe su region de amarillo palido. El heap del sistema jamas corre peligro.
// =============================================================================

#![no_std]

/// Sin sistema operativo bajo nosotros, un panico solo puede detenerse en seco.
#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Preparacion. Nada honrado que preparar: la app pasa por buena hasta su
/// primer `tick`, igual que su hermana `discola`.
#[no_mangle]
pub extern "C" fn init() {}

/// El fotograma voraz. Reclama 4096 paginas de memoria lineal de un golpe —
/// 256 MiB, muy por encima de cualquier techo razonable. El kernel `renaser`
/// denegara la expansion con una trampa y desalojara esta aplicacion.
#[no_mangle]
pub extern "C" fn tick() {
    // `memory_grow` sobre la memoria 0. Con el techo espacial activo y la
    // denegacion configurada como trampa, esta instruccion NO retorna: aborta.
    let _ = core::arch::wasm32::memory_grow(0, 4096);
    // Red de seguridad: si la denegacion no fuese una trampa, este bucle
    // cerrado garantiza el desalojo —por combustible— de todos modos.
    loop {}
}
