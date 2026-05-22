// =============================================================================
//  renaser :: kernel/src/interrupts.rs — Fase 2/3 :: la IDT y los reflejos
// -----------------------------------------------------------------------------
//  La Interrupt Descriptor Table es la tabla de reflejos del procesador. ante
//  cada excepcion o IRQ, la CPU abandona lo que hacia y salta a la entrada que
//  le corresponde. renaser distingue tres naturalezas de impulso:
//
//    * RECUPERABLE — el breakpoint (#BP): se atiende y la ejecucion prosigue.
//    * FATAL       — el resto de excepciones: se entregan al `panic!`.
//    * HARDWARE    — temporizador, teclado y disco: ya NO escriben estado
//                    rustico, sino que alimentan el reactor asincrono.
// =============================================================================

use core::sync::atomic::{AtomicU8, Ordering};

use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

use crate::{gdt, pic, CeldaSync};

/// La Interrupt Descriptor Table propia de renaser.
static IDT: CeldaSync<InterruptDescriptorTable> =
    CeldaSync::nueva(InterruptDescriptorTable::new());

/// Vector de la IDT asignado a la IRQ del disco. Vale 0 hasta que el montaje
/// del disco (Fase 6.2) descubra su linea de IRQ y la registre — ninguna IRQ
/// legitima del disco vive en el vector 0 (reservado a las excepciones).
static VECTOR_DISCO: AtomicU8 = AtomicU8::new(0);

/// Construye y activa la Interrupt Descriptor Table.
///
/// Debe invocarse una sola vez, durante el arranque, DESPUES de [`gdt::init`].
pub fn init() {
    // SEGURIDAD: `init` corre una sola vez, en arranque secuencial de un solo
    // hilo; nada mas referencia la IDT todavia.
    let idt: &'static mut InterruptDescriptorTable = unsafe { &mut *IDT.puntero() };

    // --- Excepciones de CPU ---
    idt.breakpoint.set_handler_fn(reflejo_breakpoint);
    idt.invalid_opcode.set_handler_fn(reflejo_opcode_invalido);
    idt.divide_error.set_handler_fn(reflejo_division);
    idt.general_protection_fault
        .set_handler_fn(reflejo_proteccion_general);
    idt.page_fault.set_handler_fn(reflejo_fallo_pagina);

    // El doble fallo se atiende SIEMPRE sobre el stack de emergencia del TSS:
    // ni un desbordamiento de la pila del kernel impedira su diagnostico.
    // SEGURIDAD: el indice IST referido fue armado previamente por `gdt::init`.
    unsafe {
        idt.double_fault
            .set_handler_fn(reflejo_doble_fallo)
            .set_stack_index(gdt::IST_DOBLE_FALLO);
    }

    // --- Interrupciones de hardware, ya remapeadas por el PIC ---
    idt[pic::VECTOR_TEMPORIZADOR].set_handler_fn(irq_temporizador);
    idt[pic::VECTOR_TECLADO].set_handler_fn(irq_teclado);

    let idt_estatica: &'static InterruptDescriptorTable = idt;
    idt_estatica.load();
}

/// Registra el manejador de la IRQ del disco virtio-blk en la entrada de la IDT
/// que corresponde a `irq` (Fase 6.2). Lo invoca el montaje del disco, una vez
/// descubierta la linea de IRQ que el firmware le asigno.
///
/// Se llama durante el arranque secuencial, con las interrupciones aun
/// desactivadas y la linea todavia enmascarada en el PIC: la entrada que se
/// escribe no puede dispararse mientras se escribe.
pub fn registrar_irq_disco(irq: u8) {
    let vector = pic::vector_irq(irq);
    VECTOR_DISCO.store(vector, Ordering::SeqCst);
    // SEGURIDAD: el arranque es secuencial y de un solo hilo; las interrupciones
    // estan desactivadas y la linea del disco, enmascarada. La IDT ya esta
    // cargada (`init` hizo `lidt`), pero la CPU relee cada entrada en cada
    // interrupcion: modificar esta entrada en memoria surte efecto de inmediato.
    let idt: &'static mut InterruptDescriptorTable = unsafe { &mut *IDT.puntero() };
    idt[vector].set_handler_fn(irq_disco);
}

// =============================================================================
//  REFLEJOS DE EXCEPCION — las rutinas a las que la CPU salta ante cada fallo
// =============================================================================

/// #BP — Breakpoint. Excepcion RECUPERABLE: se atiende sin ruido y, al
/// retornar, la CPU reanuda la ejecucion justo tras la instruccion `int3`.
extern "x86-interrupt" fn reflejo_breakpoint(_marco: InterruptStackFrame) {}

/// #UD — Opcode invalido. Fatal: la corriente de instrucciones es incoherente.
extern "x86-interrupt" fn reflejo_opcode_invalido(marco: InterruptStackFrame) {
    panic!("EXCEPCION FATAL :: opcode invalido (#UD)\n{marco:#?}");
}

/// #DE — Error de division. Fatal.
extern "x86-interrupt" fn reflejo_division(marco: InterruptStackFrame) {
    panic!("EXCEPCION FATAL :: error de division (#DE)\n{marco:#?}");
}

/// #GP — Fallo de proteccion general. Fatal.
extern "x86-interrupt" fn reflejo_proteccion_general(marco: InterruptStackFrame, codigo: u64) {
    panic!("EXCEPCION FATAL :: proteccion general (#GP) codigo={codigo:#x}\n{marco:#?}");
}

/// #PF — Fallo de pagina. Fatal en esta fase (sin memoria virtual dinamica).
extern "x86-interrupt" fn reflejo_fallo_pagina(
    marco: InterruptStackFrame,
    codigo: PageFaultErrorCode,
) {
    let direccion = x86_64::registers::control::Cr2::read_raw();
    panic!("EXCEPCION FATAL :: fallo de pagina (#PF) en {direccion:#x} {codigo:?}\n{marco:#?}");
}

/// #DF — Doble fallo. Fatal e irreversible: por definicion, diverge. Se ejecuta
/// sobre el stack de emergencia del TSS, nunca sobre la pila comprometida.
extern "x86-interrupt" fn reflejo_doble_fallo(marco: InterruptStackFrame, _codigo: u64) -> ! {
    panic!("EXCEPCION FATAL :: doble fallo (#DF)\n{marco:#?}");
}

// =============================================================================
//  IMPULSOS DE HARDWARE — productores de eventos para el reactor asincrono
// =============================================================================

/// IRQ0 — Temporizador. Desde la Fase 5 marca el compas del userspace: cada
/// pulso avanza el `reloj` y despierta a las tareas que aguardaban su fotograma.
extern "x86-interrupt" fn irq_temporizador(_marco: InterruptStackFrame) {
    crate::async_system::reloj::pulso();
    pic::fin_de_interrupcion(pic::VECTOR_TEMPORIZADOR);
}

/// IRQ1 — Teclado. Ya no escribe estado rustico: lee el scancode crudo y lo
/// entrega al reactor asincrono, que despertara a la tarea del teclado.
extern "x86-interrupt" fn irq_teclado(_marco: InterruptStackFrame) {
    // SEGURIDAD: 0x60 es el puerto de datos del controlador PS/2, fijo en la
    // arquitectura PC. Leerlo, ademas, libera al controlador para el siguiente.
    let scancode: u8 = unsafe { x86_64::instructions::port::Port::new(0x60).read() };
    crate::async_system::teclado::recibir_scancode(scancode);
    pic::fin_de_interrupcion(pic::VECTOR_TECLADO);
}

/// IRQ del disco — virtio-blk (Fase 6.2). El disco ha terminado una
/// transferencia: `atender_irq` reconoce la interrupcion en el dispositivo
/// —lo que libera su linea— y despierta a la tarea que aguardaba el bloque.
/// Asi la E/S de disco deja de bloquear el planificador por sondeo.
extern "x86-interrupt" fn irq_disco(_marco: InterruptStackFrame) {
    crate::drivers::disco::atender_irq();
    // El EOI se cierra DESPUES de reconocer al dispositivo: una IRQ legada de
    // PCI es de nivel — anunciar el fin sin haber bajado la linea la reavivaria.
    pic::fin_de_interrupcion(VECTOR_DISCO.load(Ordering::SeqCst));
}
