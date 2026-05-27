// =============================================================================
//  forth-emisor :: Fase 30 :: compilador Forth -> WASM 1.0 aislado y verificado
// -----------------------------------------------------------------------------
//  Esta crate compila expresiones de pila simples ("5 10 +", "3 4 5 * +") al
//  format binario WebAssembly 1.0. Se diseño para que la app `apps/ide` pueda
//  emitir modulos `.wasm` en caliente desde la jaula —pero la logica vive
//  AQUI para que el toolchain pueda verificarla con tests nativos en el host.
//
//  CONTRATO:
//    * Entrada: `&str` ASCII (digitos, espacios, `+ - *`). Cualquier caracter
//      fuera de ese dominio cae con `None`. No hay panico, no hay heap.
//    * Salida: numero de bytes escritos en `out_modulo`, o `None` si el
//      compilador no pudo construir un modulo legitimo (token ajeno, pila
//      descuadrada, buffer destino corto).
//    * El modulo resultante exporta una funcion `"run"` con firma `() -> i32`.
// =============================================================================

#![no_std]

/// Opcodes WASM 1.0 que el emisor produce. Pequeño catalogo expuesto solo
/// para que los tests puedan verificarlos por nombre.
pub mod opcodes {
    pub const I32_CONST: u8 = 0x41;
    pub const I32_ADD: u8 = 0x6A;
    pub const I32_SUB: u8 = 0x6B;
    pub const I32_MUL: u8 = 0x6C;
    pub const END: u8 = 0x0B;
}

/// Bytes magicos de un modulo WASM 1.0: `\0asm`.
pub const WASM_MAGIA: [u8; 4] = [0x00, 0x61, 0x73, 0x6D];

/// Bytes de version (1.0 little-endian) del modulo WASM.
pub const WASM_VERSION: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

/// El compilador Forth. Es un namespace sin estado — la API es una sola
/// funcion estatica `compilar` que opera sobre buffers en pila. Una
/// instancia por compilacion equivaldria a sobrediseñar: el motor no tiene
/// estado vivo entre llamadas.
pub struct ForthCompiler;

impl ForthCompiler {
    /// Compila `fuente` (texto Forth ASCII) al modulo WASM `out_modulo`.
    /// Devuelve `Some(len)` con la longitud final, o `None` si el codigo
    /// es invalido o el buffer destino es demasiado corto.
    ///
    /// Garantia: ninguna ruta de esta funcion alocha en el heap ni
    /// dispara `panic!`. Todo escenario corrupto baja por `None`.
    pub fn compilar(fuente: &str, out_modulo: &mut [u8]) -> Option<usize> {
        Self::compilar_bytes(fuente.as_bytes(), out_modulo)
    }

    /// Igual que `compilar` pero acepta bytes crudos. La app del IDE
    /// trabaja sobre un `[u8; 256]` ASCII, asi que esta variante le
    /// evita revalidar UTF-8 cuando ya sabe que sus tokens son ASCII.
    pub fn compilar_bytes(fuente: &[u8], out_modulo: &mut [u8]) -> Option<usize> {
        // --- 1. Tokenizar y construir el CUERPO de la funcion en un scratch
        //        local en pila. La cota acota el bytecode emitido a algo
        //        razonable para una expresion humana (~384 B). ---
        let mut body = [0u8; 384];
        let mut body_len = 0usize;

        // Locals declarations: 0 grupos (la funcion no tiene locales).
        push_byte(&mut body, &mut body_len, 0x00)?;

        let mut profundidad: i32 = 0;
        let mut i = 0usize;
        while i < fuente.len() {
            let c = fuente[i];
            // Espacios en blanco y bytes nulos del relleno: saltar.
            if c == b' ' || c == b'\n' || c == b'\t' || c == b'\r' || c == 0 {
                i += 1;
                continue;
            }
            // Numero ASCII (solo positivos por simplicidad —el Forth
            // canonico maneja negativos con `negate`, no con literales
            // signed-prefijo—).
            if c.is_ascii_digit() {
                let mut valor: u64 = 0;
                while i < fuente.len() && fuente[i].is_ascii_digit() {
                    valor = valor.checked_mul(10)?.checked_add((fuente[i] - b'0') as u64)?;
                    if valor > i32::MAX as u64 {
                        return None;
                    }
                    i += 1;
                }
                push_byte(&mut body, &mut body_len, opcodes::I32_CONST)?;
                emit_leb128_i32(valor as i32, &mut body, &mut body_len)?;
                profundidad = profundidad.checked_add(1)?;
                continue;
            }
            // Operador binario.
            if matches!(c, b'+' | b'-' | b'*') {
                if profundidad < 2 {
                    return None;
                }
                let op = match c {
                    b'+' => opcodes::I32_ADD,
                    b'-' => opcodes::I32_SUB,
                    _ => opcodes::I32_MUL,
                };
                push_byte(&mut body, &mut body_len, op)?;
                profundidad -= 1;
                i += 1;
                continue;
            }
            // Cualquier otro caracter es lexico ajeno; rechazar sin tocar
            // mas el grafo. El validador es hermetico.
            return None;
        }

        // La firma `() -> i32` exige EXACTAMENTE un valor en la pila al
        // cerrar. Sobrante = pila desbordada; ausente = pila vacia.
        if profundidad != 1 {
            return None;
        }
        // Cierre de la funcion: opcode `end`.
        push_byte(&mut body, &mut body_len, opcodes::END)?;

        // --- 2. Ensamblar el modulo completo en `out_modulo`. ----------------
        let mut out = 0usize;

        // Cabecera (`\0asm` + version 1.0 LE).
        push_slice(out_modulo, &mut out, &WASM_MAGIA)?;
        push_slice(out_modulo, &mut out, &WASM_VERSION)?;

        // Type Section (0x01): UN functype `() -> i32`.
        //   count=1, 0x60 (functype), n_params=0, n_results=1, 0x7F (i32)
        let type_payload = [0x01, 0x60, 0x00, 0x01, 0x7F];
        emit_section(0x01, &type_payload, out_modulo, &mut out)?;

        // Function Section (0x03): UNA funcion que usa el type 0.
        let func_payload = [0x01, 0x00];
        emit_section(0x03, &func_payload, out_modulo, &mut out)?;

        // Export Section (0x07): nombre "run", kind=func (0x00), idx=0.
        let export_payload = [0x01, 0x03, b'r', b'u', b'n', 0x00, 0x00];
        emit_section(0x07, &export_payload, out_modulo, &mut out)?;

        // Code Section (0x0A): count=1, body_size LEB128, body.
        // Lo armamos en un scratch local porque la longitud del payload
        // pre-cabecera depende de la longitud del cuerpo.
        let mut code_payload = [0u8; 400];
        let mut cp = 0usize;
        push_byte(&mut code_payload, &mut cp, 0x01)?;
        emit_leb128_u32(body_len as u32, &mut code_payload, &mut cp)?;
        if cp + body_len > code_payload.len() {
            return None;
        }
        code_payload[cp..cp + body_len].copy_from_slice(&body[..body_len]);
        cp += body_len;
        emit_section(0x0A, &code_payload[..cp], out_modulo, &mut out)?;

        Some(out)
    }
}

// =============================================================================
//  Helpers — todas devuelven Option<()> para propagar "buffer corto" sin panic
// =============================================================================

fn push_byte(buf: &mut [u8], cursor: &mut usize, byte: u8) -> Option<()> {
    if *cursor >= buf.len() {
        return None;
    }
    buf[*cursor] = byte;
    *cursor += 1;
    Some(())
}

fn push_slice(buf: &mut [u8], cursor: &mut usize, datos: &[u8]) -> Option<()> {
    if *cursor + datos.len() > buf.len() {
        return None;
    }
    buf[*cursor..*cursor + datos.len()].copy_from_slice(datos);
    *cursor += datos.len();
    Some(())
}

/// Emite una seccion WASM: `id` + LEB128(payload.len()) + payload.
fn emit_section(id: u8, payload: &[u8], destino: &mut [u8], cursor: &mut usize) -> Option<()> {
    push_byte(destino, cursor, id)?;
    emit_leb128_u32(payload.len() as u32, destino, cursor)?;
    push_slice(destino, cursor, payload)
}

/// LEB128 unsigned 32 bits.
fn emit_leb128_u32(mut v: u32, out: &mut [u8], cursor: &mut usize) -> Option<()> {
    loop {
        let mut byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
            push_byte(out, cursor, byte)?;
        } else {
            push_byte(out, cursor, byte)?;
            return Some(());
        }
    }
}

/// LEB128 signed 32 bits — el format que `i32.const` espera.
fn emit_leb128_i32(mut v: i32, out: &mut [u8], cursor: &mut usize) -> Option<()> {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        let continuar = !((v == 0 && byte & 0x40 == 0) || (v == -1 && byte & 0x40 != 0));
        if continuar {
            push_byte(out, cursor, byte | 0x80)?;
        } else {
            push_byte(out, cursor, byte)?;
            return Some(());
        }
    }
}

// =============================================================================
//  Suite de verificacion formal — tests nativos en el host
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Compilacion lineal feliz: "5 10 +" debe producir un modulo con la
    /// firma WASM y los opcodes esperados.
    #[test]
    fn test_compilacion_lineal_feliz() {
        let mut salida = [0u8; 256];
        let n = ForthCompiler::compilar("5 10 +", &mut salida).expect("ok");

        // Cabecera magica.
        assert_eq!(&salida[..4], &WASM_MAGIA);
        assert_eq!(&salida[4..8], &WASM_VERSION);

        // Buscar la secuencia esperada en el cuerpo: 0x41 0x05 0x41 0x0A 0x6A 0x0B
        let cuerpo_esperado = [0x41u8, 0x05, 0x41, 0x0A, opcodes::I32_ADD, opcodes::END];
        let modulo = &salida[..n];
        let pos = modulo
            .windows(cuerpo_esperado.len())
            .position(|w| w == cuerpo_esperado)
            .expect("el cuerpo de la funcion esta en el modulo");
        assert!(pos > 0, "el cuerpo no aparece en la cabecera");

        // Tamaño acotado: una expresion de tres tokens cabe en < 64 B.
        assert!(n > 20 && n < 64, "longitud inesperada: {n}");
    }

    /// La codificacion LEB128 signed debe usar UN byte para valores en
    /// [0, 63], y bytes adicionales con bit-7 = 1 para valores mayores
    /// que no caben en 7 bits con sign-extension.
    #[test]
    fn test_leb128_signed_constants() {
        // Constante = 5: 0x41 0x05 (un byte de payload, alto bit limpio).
        let mut salida = [0u8; 256];
        let n = ForthCompiler::compilar("5 5 +", &mut salida).expect("ok");
        let modulo = &salida[..n];
        // Debe contener 0x41 0x05 al menos dos veces.
        assert!(modulo.windows(2).filter(|w| *w == [0x41, 0x05]).count() >= 2);

        // Constante = 64: necesita dos bytes 0xC0 0x00 (porque 64 = 0b1000000
        // sin el byte de continuacion seria interpretado como -64 al
        // sign-extend desde el bit 6).
        let mut salida = [0u8; 256];
        let n = ForthCompiler::compilar("64 1 +", &mut salida).expect("ok");
        let modulo = &salida[..n];
        // La secuencia 0x41 0xC0 0x00 aparece (i32.const 64).
        assert!(
            modulo.windows(3).any(|w| w == [0x41, 0xC0, 0x00]),
            "no encontre la codificacion LEB128 de 64"
        );

        // Constante = 127: 0xFF 0x00 (bit 6 set => signo, continuar).
        let mut salida = [0u8; 256];
        let n = ForthCompiler::compilar("127 1 +", &mut salida).expect("ok");
        let modulo = &salida[..n];
        assert!(
            modulo.windows(3).any(|w| w == [0x41, 0xFF, 0x00]),
            "no encontre la codificacion LEB128 de 127"
        );

        // Constante = 128: 0x80 0x01.
        let mut salida = [0u8; 256];
        let n = ForthCompiler::compilar("128 1 +", &mut salida).expect("ok");
        let modulo = &salida[..n];
        assert!(
            modulo.windows(3).any(|w| w == [0x41, 0x80, 0x01]),
            "no encontre la codificacion LEB128 de 128"
        );
    }

    /// El compilador debe rechazar entradas que desbalancean la pila o
    /// usan tokens fuera de su gramatica, devolviendo None de forma segura.
    #[test]
    fn test_rechazo_desviacion_pila() {
        let mut salida = [0u8; 256];

        // Operador sin operandos (pila vacia).
        assert!(ForthCompiler::compilar("+", &mut salida).is_none());

        // Un solo operando, ningun operador: queda 1 valor — wait, eso SI
        // balancea (firma `() -> i32` espera exactamente 1). Verifiquemos:
        assert!(ForthCompiler::compilar("5", &mut salida).is_some());

        // Dos operandos sin operador: quedan 2 valores -> rechazo.
        assert!(ForthCompiler::compilar("5 10", &mut salida).is_none());

        // Tres operandos con un operador: quedan 2 valores -> rechazo.
        assert!(ForthCompiler::compilar("5 10 15 +", &mut salida).is_none());

        // Operador con un solo operando: 1 -> rechazo.
        assert!(ForthCompiler::compilar("5 +", &mut salida).is_none());

        // Token alfabetico ajeno.
        assert!(ForthCompiler::compilar("5 banana", &mut salida).is_none());
        assert!(ForthCompiler::compilar("hola mundo", &mut salida).is_none());

        // Vacio (sin tokens) — pila queda en 0 -> rechazo.
        assert!(ForthCompiler::compilar("", &mut salida).is_none());
        assert!(ForthCompiler::compilar("   ", &mut salida).is_none());
    }

    /// El buffer de salida demasiado corto debe propagarse como `None`
    /// SIN panic ni overflow.
    #[test]
    fn test_buffer_destino_corto_es_none() {
        let mut salida_pequena = [0u8; 16];
        // Una expresion legitima pero el destino no cabe.
        assert!(ForthCompiler::compilar("5 10 +", &mut salida_pequena).is_none());
    }

    /// El cero como constante: 0x41 0x00.
    #[test]
    fn test_constante_cero() {
        let mut salida = [0u8; 256];
        let n = ForthCompiler::compilar("0 1 +", &mut salida).expect("ok");
        let modulo = &salida[..n];
        assert!(modulo.windows(2).any(|w| w == [0x41, 0x00]));
    }

    /// Operadores SUB y MUL — 0x6B y 0x6C respectivamente.
    #[test]
    fn test_sub_y_mul_emiten_opcodes() {
        let mut salida = [0u8; 256];
        let n = ForthCompiler::compilar("10 3 -", &mut salida).expect("ok");
        assert!(salida[..n].iter().any(|&b| b == opcodes::I32_SUB));

        let mut salida = [0u8; 256];
        let n = ForthCompiler::compilar("4 5 *", &mut salida).expect("ok");
        assert!(salida[..n].iter().any(|&b| b == opcodes::I32_MUL));
    }

    /// Expresion compuesta: "2 3 + 4 *" = (2+3)*4 = 20.
    #[test]
    fn test_expresion_compuesta() {
        let mut salida = [0u8; 256];
        let n = ForthCompiler::compilar("2 3 + 4 *", &mut salida).expect("ok");
        // Debe contener i32.add seguido en algun punto por i32.mul.
        let modulo = &salida[..n];
        let pos_add = modulo
            .iter()
            .position(|&b| b == opcodes::I32_ADD)
            .expect("add");
        let pos_mul = modulo
            .iter()
            .position(|&b| b == opcodes::I32_MUL)
            .expect("mul");
        assert!(pos_add < pos_mul, "add debe preceder a mul");
    }

    /// Numero desbordado (mayor que i32::MAX): rechazo seguro.
    #[test]
    fn test_numero_desbordado_es_none() {
        let mut salida = [0u8; 256];
        // i32::MAX = 2_147_483_647. Pasamos un valor que lo supera holgado.
        assert!(ForthCompiler::compilar("9999999999999 1 +", &mut salida).is_none());
    }
}
