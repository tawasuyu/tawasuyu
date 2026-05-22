// =============================================================================
//  renaser :: kernel/src/drivers/altavoz.rs — Fase 12 :: la bocina del PC
// -----------------------------------------------------------------------------
//  La bocina del PC es el instrumento mas humilde del hardware: un solo bit que,
//  conmutado a la frecuencia justa, hace vibrar una membrana. No hay PCM, ni
//  DMA, ni mezclador — solo el canal 2 del PIT generando una onda cuadrada y
//  una compuerta en el puerto 0x61 que la deja pasar al altavoz, o no.
//
//  El canal 0 del PIT es el latido del kernel (ver `pic`); el canal 2 es de la
//  bocina y de nadie mas — programarlo aqui no perturba el temporizador—. Esta
//  es la unica via del kernel hacia el sonido; la capacidad `sys_tono` la
//  ofrece al userspace, gobernada por el foco del compositor.
// =============================================================================

use x86_64::instructions::port::Port;

/// Frecuencia del cristal del PIT, en Hz — el divisor se calcula contra ella.
const PIT_BASE_HZ: u32 = 1_193_182;

/// Puerto de comando del PIT.
const PIT_COMANDO: u16 = 0x43;
/// Puerto de datos del canal 2 del PIT — el de la bocina.
const PIT_CANAL2: u16 = 0x42;
/// Puerto de control de la bocina (bits 0 y 1: compuerta del PIT y dato).
const CONTROL_BOCINA: u16 = 0x61;

/// Pone la bocina a sonar a `frecuencia_hz`. Un `0` —o una frecuencia que un
/// divisor de 16 bits no pueda representar (por debajo de ~19 Hz)— la SILENCIA.
/// Es la unica via del kernel hacia el sonido.
pub fn tono(frecuencia_hz: u32) {
    if frecuencia_hz == 0 || PIT_BASE_HZ / frecuencia_hz > 0xFFFF {
        silenciar();
        return;
    }
    // El divisor cabe en 16 bits; un `.max(1)` lo protege de una frecuencia
    // disparatadamente alta que lo dejara en cero.
    let divisor = (PIT_BASE_HZ / frecuencia_hz).max(1) as u16;

    // SEGURIDAD: 0x43 y 0x42 son los puertos del PIT en la arquitectura PC;
    // 0xB6 selecciona el canal 2, acceso lobyte+hibyte, modo 3 (onda cuadrada).
    // El canal 2 es exclusivo de la bocina: no perturba el latido del kernel.
    unsafe {
        let mut comando = Port::<u8>::new(PIT_COMANDO);
        let mut canal2 = Port::<u8>::new(PIT_CANAL2);
        comando.write(0xB6u8);
        canal2.write((divisor & 0xFF) as u8);
        canal2.write((divisor >> 8) as u8);
    }
    abrir_compuerta();
}

/// Abre la compuerta del puerto 0x61: deja pasar la onda del canal 2 al altavoz.
fn abrir_compuerta() {
    // SEGURIDAD: 0x61 es el puerto de control de la bocina; sus bits 0 y 1
    // —compuerta del PIT y dato del altavoz— se tocan con leer-modificar-
    // escribir para no perturbar los demas bits del chipset.
    unsafe {
        let mut control = Port::<u8>::new(CONTROL_BOCINA);
        let estado = control.read();
        control.write(estado | 0b11);
    }
}

/// Silencia la bocina: cierra la compuerta del puerto 0x61. La onda del canal 2
/// sigue generandose, pero ya no alcanza la membrana.
fn silenciar() {
    // SEGURIDAD: ver `abrir_compuerta`. Limpiar los bits 0 y 1 corta el sonido.
    unsafe {
        let mut control = Port::<u8>::new(CONTROL_BOCINA);
        let estado = control.read();
        control.write(estado & !0b11);
    }
}
