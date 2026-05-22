// =============================================================================
//  renaser :: apps/discola — Fase 5 :: el inquilino discolo del userspace
// -----------------------------------------------------------------------------
//  Esta aplicacion esta MAL a proposito. Su `tick` no hace un fotograma de
//  trabajo y retorna —como manda el ABI cooperativo—: cae en un bucle cerrado
//  y jamas devuelve el control. En un sistema cooperativo ingenuo, eso colgaria
//  la maquina entera.
//
//  Pero renaser ejecuta cada `tick` con un presupuesto estricto de COMBUSTIBLE.
//  Cuando este bucle lo agota, el runtime `wasmi` lanza una trampa, el kernel
//  recupera el mando y desaloja a este modulo — tatuando su region de purpura.
//  El resto del userspace ni se entera. Eso es lo que esta app demuestra.
// =============================================================================

#![no_std]

/// Sin sistema operativo bajo nosotros, un panico solo puede detenerse en seco.
#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Sumidero de las escrituras del bucle: obliga al compilador a CONSERVAR cada
/// iteracion —y, con ella, su consumo de combustible— en lugar de elidirla.
static mut SUMIDERO: u64 = 0;

/// Preparacion. No hay nada honrado que preparar: el kernel la invoca, retorna
/// sin incidentes, y la app pasa por buena... hasta su primer `tick`.
#[no_mangle]
pub extern "C" fn init() {}

/// El fotograma que nunca termina. Un bucle cerrado, deliberado: jamas retorna.
/// El kernel `renaser` lo cortara por agotamiento de combustible y desalojara
/// esta aplicacion sin que el sistema sufra un solo sobresalto.
#[no_mangle]
pub extern "C" fn tick() {
    let mut contador: u64 = 0;
    loop {
        contador = contador.wrapping_add(1);
        // SEGURIDAD: escritura volatil a un escalar estatico; su unico fin es
        // que el optimizador no pueda vaciar el bucle. No se crea referencia.
        unsafe {
            core::ptr::write_volatile(core::ptr::addr_of_mut!(SUMIDERO), contador);
        }
    }
}
