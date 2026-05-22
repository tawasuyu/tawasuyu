// =============================================================================
//  renaser :: kernel/src/pic.rs — Fase 2.1 :: el par 8259 y el metronomo
// -----------------------------------------------------------------------------
//  El controlador de interrupciones 8259 (PIC) nace, por herencia historica,
//  con sus vectores solapados sobre los de las excepciones de CPU. Antes de
//  encender las interrupciones HAY que remapearlo: desplazar sus vectores
//  fuera del rango 0..31. Aqui tambien programamos el PIT, el temporizador
//  que dara a renaser su latido regular — el origen de su fluidez.
// =============================================================================

use x86_64::instructions::port::Port;

// --- Puertos del par de 8259 (maestro + esclavo en cascada). ---
const CMD_MAESTRO: u16 = 0x20;
const DATOS_MAESTRO: u16 = 0x21;
const CMD_ESCLAVO: u16 = 0xA0;
const DATOS_ESCLAVO: u16 = 0xA1;

// --- Puertos del temporizador de intervalos programable (PIT). ---
const PIT_COMANDO: u16 = 0x43;
const PIT_CANAL0: u16 = 0x40;
/// Frecuencia base del cristal del PIT, en Hz.
const PIT_BASE_HZ: u32 = 1_193_182;

/// Orden de «fin de interrupcion» que el PIC espera tras cada IRQ atendida.
const EOI: u8 = 0x20;

/// Vector base del PIC maestro tras el remapeo (justo encima de las 32
/// excepciones reservadas de la arquitectura).
const OFFSET_MAESTRO: u8 = 0x20;
/// Vector base del PIC esclavo tras el remapeo.
const OFFSET_ESCLAVO: u8 = 0x28;

/// Vector de la IRQ0 — el temporizador (PIT).
pub const VECTOR_TEMPORIZADOR: u8 = OFFSET_MAESTRO; // 0x20
/// Vector de la IRQ1 — el teclado.
pub const VECTOR_TECLADO: u8 = OFFSET_MAESTRO + 1; // 0x21

/// Remapea el par 8259 y programa el PIT a 100 Hz.
///
/// Debe invocarse una sola vez, tras cargar la IDT y ANTES de habilitar las
/// interrupciones con `sti`.
pub fn init() {
    remapear();
    configurar_temporizador(100);
}

/// Reprograma los vectores del 8259 fuera del rango de las excepciones de CPU.
fn remapear() {
    // SEGURIDAD: estos son los puertos de E/S estandar del 8259 en la
    // arquitectura PC; la secuencia ICW1..ICW4 es su protocolo de inicio.
    unsafe {
        let mut cmd_m = Port::<u8>::new(CMD_MAESTRO);
        let mut dat_m = Port::<u8>::new(DATOS_MAESTRO);
        let mut cmd_e = Port::<u8>::new(CMD_ESCLAVO);
        let mut dat_e = Port::<u8>::new(DATOS_ESCLAVO);
        // Escribir en un puerto inerte da al 8259 el respiro que necesita
        // entre palabras de control en hardware antiguo.
        let mut respiro = Port::<u8>::new(0x80);

        // ICW1 — iniciar la secuencia: en cascada, con ICW4 presente.
        cmd_m.write(0x11u8);
        respiro.write(0u8);
        cmd_e.write(0x11u8);
        respiro.write(0u8);
        // ICW2 — el remapeo en si: desplazar los vectores lejos de 0..31.
        dat_m.write(OFFSET_MAESTRO);
        respiro.write(0u8);
        dat_e.write(OFFSET_ESCLAVO);
        respiro.write(0u8);
        // ICW3 — declarar el cableado de la cascada (el esclavo en la IRQ2).
        dat_m.write(0b0000_0100u8);
        respiro.write(0u8);
        dat_e.write(0b0000_0010u8);
        respiro.write(0u8);
        // ICW4 — modo 8086.
        dat_m.write(0x01u8);
        respiro.write(0u8);
        dat_e.write(0x01u8);
        respiro.write(0u8);
        // Mascaras: el maestro deja pasar la IRQ0 (temporizador) y la IRQ1
        // (teclado); todo lo demas, en silencio hasta que renaser lo reclame.
        dat_m.write(0b1111_1100u8);
        dat_e.write(0b1111_1111u8);
    }
}

/// Programa el canal 0 del PIT para que emita la IRQ0 a `frecuencia_hz`.
fn configurar_temporizador(frecuencia_hz: u32) {
    let divisor = (PIT_BASE_HZ / frecuencia_hz) as u16;
    // SEGURIDAD: 0x43 y 0x40 son los puertos estandar del PIT en el PC.
    unsafe {
        let mut comando = Port::<u8>::new(PIT_COMANDO);
        let mut canal0 = Port::<u8>::new(PIT_CANAL0);
        // Canal 0, acceso lobyte+hibyte, modo 3 (generador de onda cuadrada).
        comando.write(0x36u8);
        canal0.write((divisor & 0xFF) as u8);
        canal0.write((divisor >> 8) as u8);
    }
}

/// Notifica al PIC el «fin de interrupcion» de la IRQ recien atendida. Sin
/// este aviso, el PIC jamas volveria a emitir esa interrupcion.
pub fn fin_de_interrupcion(vector: u8) {
    // SEGURIDAD: enviar EOI al puerto de comandos del PIC tras atender su IRQ
    // es el cierre obligatorio del protocolo del 8259.
    unsafe {
        // Si la IRQ provino del esclavo, ambos PIC deben recibir el EOI.
        if vector >= OFFSET_ESCLAVO {
            Port::<u8>::new(CMD_ESCLAVO).write(EOI);
        }
        Port::<u8>::new(CMD_MAESTRO).write(EOI);
    }
}

/// Vector de la IDT que corresponde a una linea de IRQ (0..15). El remapeo dejo
/// las 16 lineas del par 8259 en vectores contiguos desde `OFFSET_MAESTRO`: la
/// IRQ8 cae en `OFFSET_ESCLAVO`, que es justo `OFFSET_MAESTRO + 8`.
pub fn vector_irq(irq: u8) -> u8 {
    OFFSET_MAESTRO + irq
}

/// Levanta la mascara de una linea de IRQ — el PIC empezara a emitirla. Si la
/// linea vive en el PIC esclavo (8..15), abre tambien la cascada del maestro
/// (la IRQ2), sin la cual el esclavo jamas alcanzaria a la CPU.
///
/// Debe invocarse en el arranque, antes de habilitar las interrupciones.
pub fn desenmascarar(irq: u8) {
    // SEGURIDAD: 0x21 y 0xA1 son los puertos de mascara del par 8259 en la
    // arquitectura PC; leer-modificar-escribir es la via de tocar una linea
    // sola sin perturbar a las demas.
    unsafe {
        if irq < 8 {
            let mut datos = Port::<u8>::new(DATOS_MAESTRO);
            let mascara = datos.read();
            datos.write(mascara & !(1 << irq));
        } else {
            let mut datos_e = Port::<u8>::new(DATOS_ESCLAVO);
            let mascara_e = datos_e.read();
            datos_e.write(mascara_e & !(1 << (irq - 8)));
            // La cascada: el esclavo entrega sus IRQ al maestro por la IRQ2.
            let mut datos_m = Port::<u8>::new(DATOS_MAESTRO);
            let mascara_m = datos_m.read();
            datos_m.write(mascara_m & !(1 << 2));
        }
    }
}
