// =============================================================================
//  renaser :: apps/pluma — Fase 33/34/35 :: notebook bare-metal de Pluma
// -----------------------------------------------------------------------------
//  Reflejo bare-metal del ecosistema PLUMA del host (`00_unanchay/pluma/`).
//  En Linux, Pluma se renderiza con `pluma-notebook-llimphi` /
//  `pluma-editor-llimphi` (wgpu + ropey + tree-sitter); aqui, el mismo
//  concepto vive dentro de una jaula WASM de Wawa OS sobre un framebuffer
//  480x400. La capa de TIPOS converge: `pluma-notebook-core` ya es
//  `#![no_std] + alloc` (Fase 35) y puede importarse desde la pila
//  bare-metal — el dia que el modelo Forth minimal converja con el rico
//  (markdown/embed/table/image), la `Cell`/`CellKind` del host sera la
//  MISMA estructura que la del bare-metal.
//
//  Un cuaderno (en el sentido del grafo: un NODO con payload
//  `Vec<TipoCeldaWawa>` + aristas a fuente y binario) lo construye la
//  syscall `sys_cuaderno_registrar_celda`. Cada CELDA enlaza una FUENTE
//  Forth, un BINARIO WASM emitido por forth-emisor y el RETORNO de su
//  ultima ejecucion. Las tres piezas se inscriben en el grafo
//  direccionado por contenido del kernel — el cuaderno deja de ser un
//  buffer volatil y se vuelve un nodo inmutable que sobrevive al
//  apagado, al panico y a la mudanza.
//
//  FASE 34 :: AUDITORIA DE CRATES y FLUJO CELULAR ENCADENADO
//
//  Auditoria conceptual de parsers `no_std` antes de codificar:
//    * `nom` (default-features=false): viable bajo wasm32 + alloc. Sin
//      embargo, exige el `alloc` global y construye combinatores con
//      closures alocadas en heap — viola la zero-alloc strict policy
//      que nuestro emisor mantiene hoy en su buffer en pila.
//    * `winnow` (fork moderno de nom): mismo veredicto. Util si el dia
//      de mañana exponemos Forth con definiciones anidadas y necesitamos
//      backtracking real; hoy el lenguaje es tan plano que el coste
//      no compensa.
//    * `logos`: lexer por proc-macro. Genera codigo no_std-friendly pero
//      su API natural devuelve `Token`s en un iterador heap-backed.
//    * `chumsky`: depende de `std` (BTreeMap, etc.). Descartado.
//
//  CONCLUSION: `forth-emisor` casero (zero-alloc, ~400 LOC, 8 tests verde)
//  es ESTRICTAMENTE mas puro que cualquier crate externa al fragmento
//  actual del lenguaje. La auditoria queda registrada aqui para que la
//  proxima Fase que extienda Forth (definiciones, condicionales, words)
//  recoja `winnow` como primera opcion sin recorrer este sendero otra vez.
//
//  EJECUCION EN CASCADA :: la Celda N hereda el `UltimoRetorno` de la
//  Celda N-1 (si fue exitoso) y lo inyecta como prefijo en el stack
//  Forth de la celda actual. La cadena es estricta: una celda con
//  fallo ROMPE la cascada y la leyenda del pie pasa a rotular
//  "ERROR  CADENA DE EJECUCION ROTA" hasta que una celda exitosa la
//  restaure. La transferencia entre celdas es zero-alloc — el valor
//  viaja en la pila como `i32` y se serializa a ASCII decimal en un
//  buffer estatico al concatenar con la fuente del editor.
//
//  El usuario teclea en el PANEL EDITOR (parte alta del lienzo natural);
//  F5 dispara una rafaga de syscalls que:
//    1. Compila la fuente con forth-emisor.
//    2. Inscribe la FUENTE en el grafo (`sys_object_put`).
//    3. Inscribe el BINARIO con la arista causal de la Fase 31
//       (`sys_subsistema_registrar_ejecutable_v2`).
//    4. Ejecuta el binario en una sub-jaula efimera de la Fase 32
//       (`sys_subsistema_ejecutar_dinamico`) y captura el i32 retornado.
//    5. Consolida la celda en el cuaderno via la nueva syscall de la
//       Fase 33 (`sys_cuaderno_registrar_celda`).
//
//  Si CUALQUIER paso de la cadena falla (trap, fuel agotado, almacenamiento),
//  la celda se inscribe igual — con `retorno` negativo del rango reservado de
//  `CodigoError`— y se pinta de AMARILLO PALIDO. Las celdas vecinas siguen
//  ofreciendo sus retornos en verde, el compositor jamas pierde un fotograma.
//
//  Alt+Enter, mencionado en la directiva original, lo intercepta el kernel
//  como mando del compositor (promueve la ventana enfocada a maestra). El
//  cuaderno escogio F5 (scancode 0x3F) como hotkey pragmatica de ejecucion
//  celular — alineada con la convencion de los IDEs notebook clasicos sin
//  colisionar con la matriz de mandos del WM.
// =============================================================================

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_get_scancode() -> u32;
    fn sys_config_paleta(salida: u32) -> i32;
    fn sys_object_put(
        datos_ptr: u32,
        datos_len: u32,
        hijos_ptr: u32,
        hijos_cnt: u32,
        salida: u32,
    ) -> i32;
    fn sys_subsistema_registrar_ejecutable_v2(
        ptr: u32,
        len: u32,
        padre_hash_ptr: u32,
        salida_hash_ptr: u32,
    ) -> i32;
    fn sys_subsistema_ejecutar_dinamico(binario_hash_ptr: u32) -> i32;
    fn sys_cuaderno_registrar_celda(
        fuente_hash_ptr: u32,
        binario_hash_ptr: u32,
        retorno: i32,
        salida_cuaderno_hash_ptr: u32,
    ) -> i32;
    /// FASE 36 :: Cross-App Semantic Bridge. Inspecciona un binario en el
    /// grafo (sin instanciar) y devuelve un dictamen de 4 B en
    /// `salida_info_ptr`:
    ///   byte 0 = 0x01 si el nodo existe + tiene magia WASM + expone
    ///            export `"run"`; 0x00 en cualquier otro caso.
    ///   bytes 1..4 = tamaño en bloques de 256 B (LE u24).
    /// El retorno i32 es 0 (Ok) si la inspeccion corrio; codigos
    /// negativos reservados (Ausente, Saturado, etc.) reportan
    /// fallas del syscall en si.
    fn sys_subsistema_vincular_macro(
        binario_hash_ptr: u32,
        salida_info_ptr: u32,
    ) -> i32;
    /// FASE 37 :: Firma del Tejido Celular. La app entrega un sobre
    /// `CuadernoFirmado` (cuaderno_raiz_hash + autor Ed25519 + firma)
    /// serializado con postcard en su memoria lineal. El kernel verifica
    /// la firma contra `AGORA_PUBLIC_KEY_LOCAL` y, si es lícita, fija
    /// el cuaderno como raíz del grafo userspace. Códigos:
    ///   * 0  → CUADERNO SOBERANO ANCLADO.
    ///   * -1 → sobre malformado / cuaderno no ingestado.
    ///   * -2 → autor distinto al operador local.
    ///   * -3 → firma forjada o tampered.
    ///   * -4 → app sin foco.
    fn sys_cuaderno_firmar_y_anclar(cuaderno_firmado_ptr: u32) -> i32;
}

#[cfg(not(test))]
#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// =============================================================================
//  Geometria del lienzo natural (480 x 400)
// =============================================================================

const ANCHO: usize = 480;
const ALTO: usize = 400;

const EDITOR_Y: usize = 24;
const EDITOR_ALTO: usize = 60;
const CELDAS_Y: usize = EDITOR_Y + EDITOR_ALTO + 6;
const CELDAS_ALTO: usize = 270;
const LEYENDA_Y: usize = ALTO - 16;

const MAX_CELDAS: usize = 3;
const CELDA_ALTO: usize = CELDAS_ALTO / MAX_CELDAS;

// =============================================================================
//  Estado celular
// =============================================================================

/// Tope del buffer fuente (compartido por todas las celdas y el editor).
/// Capacidad del buffer fuente. La Fase 36 introduce el token `@hash` para
/// importar binarios pre-compilados: `@` + 64 hex = 65 bytes minimo. Subimos
/// a 96 para dejar holgura para el editor y la fuente efectiva concatenada.
const FUENTE_CAP: usize = 96;

/// Una celda del cuaderno en su forma viva en RAM. La forma serializada
/// canonica vive en el grafo: este struct es el cache de pintado.
#[derive(Clone, Copy)]
struct Celda {
    /// La fuente Forth tal y como la TECLEO el humano. La fuente real que
    /// se compila puede llevar prepended el retorno heredado de la celda
    /// anterior — esa version efectiva se inscribe en el grafo (via
    /// `hash_fuente`) para que la arista causal del binario apunte a la
    /// cadena exacta de caracteres que lo engendro.
    fuente: [u8; FUENTE_CAP],
    fuente_len: usize,
    hash_fuente: [u8; 32],
    hash_binario: [u8; 32],
    retorno: i32,
    /// `true` cuando la celda recorrio la cadena completa al menos una vez.
    /// Distingue celdas habitadas de las ranuras vacias del cuaderno.
    valida: bool,
    /// `true` cuando el retorno fue legitimo. `false` cuando la cadena
    /// fallo (cualquier eslabon devolvio un codigo de error). El pintado
    /// usa este bit para teñir la celda de amarillo palido sin enterrar
    /// el resultado.
    exito: bool,
    /// FASE 34 :: la celda HEREDO un valor de su predecesora exitosa y lo
    /// inyecto como cabeza de su pila Forth. El pintado dibuja un glifo
    /// `>` en el margen izquierdo cuando esto es cierto, y muestra
    /// `HER N` con `N` en `valor_heredado`. Una celda que ejecute en
    /// solitario (sin predecesora exitosa) deja este bit a `false`.
    heredado: bool,
    /// El i32 que se prepended a la fuente del editor para compilar esta
    /// celda. Solo es significativo cuando `heredado == true`.
    valor_heredado: i32,
    /// FASE 36 :: la celda no nacio de compilar Forth en el editor, sino
    /// de IMPORTAR un binario pre-existente del grafo via la sintaxis
    /// `@<64-hex>`. La cadena de compilacion se salta: el cuaderno solo
    /// inspecciona (`sys_subsistema_vincular_macro`), ejecuta
    /// (`sys_subsistema_ejecutar_dinamico`) y consolida la celda. El
    /// `hash_binario` apunta a un nodo que pinto OTRA APP en el grafo
    /// del disco; el cuaderno se vuelve cross-app por construccion.
    macro_importada: bool,
}

impl Celda {
    const fn vacia() -> Self {
        Celda {
            fuente: [0; FUENTE_CAP],
            fuente_len: 0,
            hash_fuente: [0; 32],
            hash_binario: [0; 32],
            retorno: 0,
            valida: false,
            exito: false,
            heredado: false,
            valor_heredado: 0,
            macro_importada: false,
        }
    }
}

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];
static mut PALETA: [u8; 20] = [0; 20];

static mut EDITOR: [u8; FUENTE_CAP] = [0; FUENTE_CAP];
static mut EDITOR_LEN: usize = 0;

static mut CELDAS: [Celda; MAX_CELDAS] = [Celda::vacia(); MAX_CELDAS];
static mut PROXIMA_CELDA: usize = 0;

/// Hash del cuaderno tras la ultima consolidacion exitosa. Es la "raiz"
/// movil del cuaderno: cada celda inscrita engendra un cuaderno nuevo y
/// este se reancla aqui. No se compara con el grafo del kernel — solo
/// rotula la cabecera del editor para que el humano vea que el cuaderno
/// quedo persistido (`HASH cuaderno: XX..YY`).
static mut HASH_CUADERNO: [u8; 32] = [0; 32];
static mut HASH_CUADERNO_VALIDO: bool = false;

/// FASE 34 :: el ESTADO FLUYENTE entre celdas. El ultimo retorno exitoso
/// queda disponible aqui para que la siguiente F5 lo prependa como cabeza
/// de pila Forth. Una celda exitosa lo refresca; una celda fallida lo
/// INVALIDA — el flujo se interrumpe hasta que otra celda exitosa
/// reanude la cascada.
static mut RETORNO_HEREDADO: i32 = 0;
static mut RETORNO_HEREDADO_VALIDO: bool = false;
/// Bandera de la leyenda al pie: cuando una celda que pretendia heredar
/// fallo (o cuando una celda fallida deja roto el flujo), el pintado
/// rota la leyenda a "ERROR  CADENA DE EJECUCION ROTA". Se reinicia en
/// `false` con la proxima celda exitosa.
static mut CADENA_ROTA: bool = false;

static mut F5_PREV: bool = false;
static mut F6_PREV: bool = false;
static mut SCAN_PREV: u32 = 0;

/// FASE 37 :: estado del anclaje soberano. Tras un F6 exitoso, el kernel
/// confirma que el cuaderno se firmo y anclo como raiz; la leyenda al
/// pie muestra "CUADERNO SOBERANO ANCLADO [OK]". Un fallo (firma rota,
/// autor ajeno, cuaderno no ingestado) viste el indicador del marco de
/// amarillo palido y rotula "FIRMA INVALIDA  INTEGRIDAD COMPROMETIDA".
static mut SOBERANIA_ANCLADA: bool = false;
static mut SOBERANIA_FIRMA_ROTA: bool = false;
static mut SOBERANIA_CODIGO: i32 = 0;

// =============================================================================
//  ABI obligatorio del userspace renaser
// =============================================================================

#[no_mangle]
pub extern "C" fn init() {
    refrescar_contexto();
    pintar();
    volcar();
}

#[no_mangle]
pub extern "C" fn tick() {
    refrescar_contexto();

    let scancode = unsafe { sys_get_scancode() };
    if scancode != 0 && scancode != unsafe { SCAN_PREV } {
        atender_scancode(scancode);
    }
    unsafe { SCAN_PREV = scancode };

    pintar();
    volcar();
}

fn refrescar_contexto() {
    let _ = unsafe { sys_config_paleta(core::ptr::addr_of_mut!(PALETA) as u32) };
}

// =============================================================================
//  Teclado
// =============================================================================

fn atender_scancode(scancode: u32) {
    if scancode == 0x3F {
        // F5 :: ejecutar celda. Solo dispara una vez por pulsacion.
        if !unsafe { F5_PREV } {
            ejecutar_celda_actual();
        }
        unsafe { F5_PREV = true };
        return;
    }
    if scancode == 0x40 {
        // FASE 37 :: F6 :: firmar y anclar el cuaderno soberano. Toma el
        // ultimo `HASH_CUADERNO` que el kernel devolvio en la consolidacion
        // de la cascada y lo pasa por `sys_cuaderno_firmar_y_anclar` con
        // un sobre criptografico. Sin clave privada en la jaula, el MVP
        // adjunta un placeholder de firma — el kernel correctamente lo
        // rechaza con AlmacenamientoFallo y la leyenda al pie indica
        // "FIRMA INVALIDA". El camino feliz requerira un signer externo
        // (wawactl) en la Fase 38+.
        if !unsafe { F6_PREV } {
            firmar_y_anclar_cuaderno();
        }
        unsafe { F6_PREV = true };
        return;
    }
    unsafe {
        F5_PREV = scancode == 0x3F;
        F6_PREV = scancode == 0x40;
    }

    if scancode == 0x0E {
        // Backspace.
        if unsafe { EDITOR_LEN } > 0 {
            unsafe { EDITOR_LEN -= 1 };
        }
        return;
    }
    let escribir = match scancode {
        0x39 => Some(b' '),
        _ => mapear_caracter(scancode as u8),
    };
    if let Some(b) = escribir {
        let len = unsafe { EDITOR_LEN };
        if len < FUENTE_CAP {
            unsafe {
                EDITOR[len] = b;
                EDITOR_LEN = len + 1;
            }
        }
    }
}

fn mapear_caracter(scan: u8) -> Option<u8> {
    Some(match scan {
        // FASE 36 :: la tecla backtick/grave (rara en Forth) produce `@`,
        // el prefijo del token de importacion de macros. Sin esto, la
        // sintaxis `@<hash>` seria intecleable: el kernel no decodifica
        // modificadores (Shift, AltGr), asi que `Shift+2` solo emite '2'.
        0x29 => b'@',
        0x02 => b'1',
        0x03 => b'2',
        0x04 => b'3',
        0x05 => b'4',
        0x06 => b'5',
        0x07 => b'6',
        0x08 => b'7',
        0x09 => b'8',
        0x0A => b'9',
        0x0B => b'0',
        0x0C => b'-',
        // Numpad: digitos y operadores Forth (sin Shift).
        0x47 => b'7',
        0x48 => b'8',
        0x49 => b'9',
        0x4B => b'4',
        0x4C => b'5',
        0x4D => b'6',
        0x4F => b'1',
        0x50 => b'2',
        0x51 => b'3',
        0x52 => b'0',
        0x37 => b'*',
        0x4A => b'-',
        0x4E => b'+',
        _ => return None,
    })
}

// =============================================================================
//  Cadena de ejecucion celular (F5)
// =============================================================================

/// Tope del buffer EFECTIVO de fuente: 64 (editor) + 12 (ASCII decimal de
/// un i32) + 1 (separador) + holgura. Vive en pila durante la rafaga F5
/// — el asignador del kernel nunca interviene en la cascada.
const FUENTE_EFECTIVA_CAP: usize = 96;

/// Recorre la cadena FUENTE -> BINARIO -> EJECUTAR -> CUADERNO sobre el
/// buffer del editor, EVENTUALMENTE prepended con el retorno heredado de
/// la celda anterior (Fase 34). Cualquier eslabon fallido se conserva
/// como celda con `exito = false` y rompe la cascada para las celdas
/// siguientes hasta que un nuevo F5 exitoso la restaure.
fn ejecutar_celda_actual() {
    let len = unsafe { EDITOR_LEN };
    if len == 0 {
        return;
    }

    // ----- FASE 36 :: rama de IMPORTACION DE MACRO (@<64-hex>) ---------------
    // Si el editor arranca con `@` y trae al menos 64 hex caracteres
    // despues, NO pasamos por forth-emisor: el binario ya esta en el
    // grafo (lo escribio otra app en otra pestaña, otro arranque, otro
    // mes). Pedimos al kernel que lo VINCULE como macro (inspeccion sin
    // instanciar) y, si el dictamen es valido, lo EJECUTAMOS via
    // `sys_subsistema_ejecutar_dinamico`. La celda se consolida en el
    // grafo igual que cualquier otra; el `hash_fuente` queda en cero
    // —no hay fuente Forth que enlazar— y el `hash_binario` es el del
    // nodo importado.
    if unsafe { EDITOR[0] } == b'@' && len >= 65 {
        ejecutar_celda_importada();
        return;
    }

    // ----- Construir la FUENTE EFECTIVA en un buffer en pila ----------------
    // Si la celda previa fue exitosa, su retorno encabeza la pila Forth de
    // esta celda. La concatenacion ocurre en ASCII decimal — forth-emisor
    // tokeniza digitos y operadores, no hay otro camino para introducir
    // una constante por la puerta delantera del lenguaje.
    let mut efectiva = [0u8; FUENTE_EFECTIVA_CAP];
    let mut efectiva_len = 0usize;
    let heredado = unsafe { RETORNO_HEREDADO_VALIDO };
    let valor_heredado = unsafe { RETORNO_HEREDADO };
    if heredado {
        let (dec, dlen) = formatear_i32(valor_heredado);
        if dlen + 1 + len > FUENTE_EFECTIVA_CAP {
            // El editor pegado al ASCII del retorno no cabria — escenario
            // hipotetico con un retorno de 11 digitos y un editor casi
            // pleno. Tratamos como cadena rota: una celda con texto que
            // no compila en este formato no debe abortar el cuaderno.
            unsafe {
                RETORNO_HEREDADO_VALIDO = false;
                CADENA_ROTA = true;
            }
            // Caemos al modo sin herencia: el editor compila como-as.
        } else {
            efectiva[..dlen].copy_from_slice(&dec[..dlen]);
            efectiva[dlen] = b' ';
            efectiva_len = dlen + 1;
        }
    }
    // Copiar el editor al final del buffer efectivo (con o sin prefijo).
    let editor_inicio = efectiva_len;
    let cap_rest = FUENTE_EFECTIVA_CAP - efectiva_len;
    let n_editor = len.min(cap_rest);
    efectiva[editor_inicio..editor_inicio + n_editor]
        .copy_from_slice(unsafe { &EDITOR[..n_editor] });
    efectiva_len += n_editor;
    let efectiva_uso = unsafe { RETORNO_HEREDADO_VALIDO } && heredado;

    let mut celda = Celda::vacia();
    // En `celda.fuente` guardamos la fuente EFECTIVA: es la cadena de
    // bytes que el grafo va a inscribir como TextoFuente y la que el
    // binario referencia como primer hijo. El pintado distingue el
    // tramo heredado del tramo tecleado via los campos
    // `heredado`/`valor_heredado`.
    let copy_n = efectiva_len.min(FUENTE_CAP);
    celda.fuente[..copy_n].copy_from_slice(&efectiva[..copy_n]);
    celda.fuente_len = copy_n;
    celda.valida = true;
    celda.heredado = efectiva_uso;
    celda.valor_heredado = if efectiva_uso { valor_heredado } else { 0 };

    // 1. Compilar Forth -> WASM en un buffer en pila.
    let mut binario = [0u8; 512];
    let bin_len = match forth_emisor::ForthCompiler::compilar_bytes(
        &efectiva[..efectiva_len],
        &mut binario,
    ) {
        Some(n) => n,
        None => {
            celda.retorno = -7; // PayloadInvalido (sintaxis Forth ajena).
            celda.exito = false;
            unsafe {
                RETORNO_HEREDADO_VALIDO = false;
                CADENA_ROTA = true;
            }
            registrar_celda_local(celda);
            return;
        }
    };

    // 2. Grabar la FUENTE EFECTIVA como objeto del grafo. Sin hijos: la
    //    fuente es una hoja del DAG. El hash que devuelve es el padre
    //    del binario que vamos a registrar a continuacion.
    let cod_fuente = unsafe {
        sys_object_put(
            efectiva.as_ptr() as u32,
            efectiva_len as u32,
            0u32,
            0u32,
            celda.hash_fuente.as_mut_ptr() as u32,
        )
    };
    if cod_fuente != 0 {
        celda.retorno = cod_fuente;
        celda.exito = false;
        unsafe {
            RETORNO_HEREDADO_VALIDO = false;
            CADENA_ROTA = true;
        }
        registrar_celda_local(celda);
        return;
    }

    // 3. Registrar el BINARIO con la arista causal hacia la FUENTE.
    let cod_bin = unsafe {
        sys_subsistema_registrar_ejecutable_v2(
            binario.as_ptr() as u32,
            bin_len as u32,
            celda.hash_fuente.as_ptr() as u32,
            celda.hash_binario.as_mut_ptr() as u32,
        )
    };
    if cod_bin != 0 {
        celda.retorno = cod_bin;
        celda.exito = false;
        unsafe {
            RETORNO_HEREDADO_VALIDO = false;
            CADENA_ROTA = true;
        }
        registrar_celda_local(celda);
        return;
    }

    // 4. Ejecutar el binario en sub-jaula efimera. El i32 retornado por
    //    `"run"` viaja tal cual; codigos negativos [-7..-1] reservados
    //    son fallas controladas (trap, fuel agotado, etc.).
    let retorno = unsafe { sys_subsistema_ejecutar_dinamico(celda.hash_binario.as_ptr() as u32) };
    let es_falla = retorno <= -1 && retorno >= -7;
    celda.retorno = retorno;
    celda.exito = !es_falla;

    // 5. Consolidar la celda como nodo cuaderno en el grafo.
    let cod_cuaderno = unsafe {
        sys_cuaderno_registrar_celda(
            celda.hash_fuente.as_ptr() as u32,
            celda.hash_binario.as_ptr() as u32,
            celda.retorno,
            core::ptr::addr_of_mut!(HASH_CUADERNO) as u32,
        )
    };
    unsafe { HASH_CUADERNO_VALIDO = cod_cuaderno == 0 };

    // 6. Refrescar el flujo cascada. Una celda exitosa propaga su retorno
    //    a la proxima F5 y restaura la leyenda al pie; una celda fallida
    //    rompe la cascada hasta que otra celda exitosa la reanude.
    unsafe {
        if celda.exito {
            RETORNO_HEREDADO = retorno;
            RETORNO_HEREDADO_VALIDO = true;
            CADENA_ROTA = false;
        } else {
            RETORNO_HEREDADO_VALIDO = false;
            CADENA_ROTA = true;
        }
    }

    registrar_celda_local(celda);

    // Si la ejecucion fue exitosa, dejamos el editor cargado para que el
    // humano pueda iterar; si fallo, tampoco se borra — el usuario corrige
    // y vuelve a F5. Vaciar el editor cada vez seria un anti-MVP.
}

/// FASE 36 :: parsea un caracter ASCII (0-9, a-f, A-F) a su nibble [0..15].
/// Devuelve `None` si el caracter no es hex valido.
fn nibble_de_hex(c: u8) -> Option<u8> {
    Some(match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => return None,
    })
}

/// FASE 36 :: ejecuta el camino de IMPORTACION DE MACRO. El editor empieza
/// con `@` seguido de exactamente 64 caracteres hex (32 bytes). Parsea el
/// hash, lo vincula con el kernel, lo ejecuta y consolida la celda.
fn ejecutar_celda_importada() {
    let mut celda = Celda::vacia();
    celda.valida = true;
    celda.macro_importada = true;

    // 1. Parsear los 64 hex chars tras el `@` a un [u8; 32]. Tomamos el
    //    puntero crudo del editor — Rust 2024 prohibe la referencia
    //    `&EDITOR` directa sobre un `static mut`, pero un `*const`
    //    elaborado con `addr_of!` lo sortea sin Undefined Behavior.
    let mut hash = [0u8; 32];
    let editor_ptr = core::ptr::addr_of!(EDITOR);
    let editor: &[u8; FUENTE_CAP] = unsafe { &*editor_ptr };
    for i in 0..32 {
        let hi = match nibble_de_hex(editor[1 + i * 2]) {
            Some(n) => n,
            None => {
                celda.retorno = -7; // PayloadInvalido: hash mal formado.
                celda.exito = false;
                unsafe {
                    RETORNO_HEREDADO_VALIDO = false;
                    CADENA_ROTA = true;
                }
                registrar_celda_local(celda);
                return;
            }
        };
        let lo = match nibble_de_hex(editor[1 + i * 2 + 1]) {
            Some(n) => n,
            None => {
                celda.retorno = -7;
                celda.exito = false;
                unsafe {
                    RETORNO_HEREDADO_VALIDO = false;
                    CADENA_ROTA = true;
                }
                registrar_celda_local(celda);
                return;
            }
        };
        hash[i] = (hi << 4) | lo;
    }
    celda.hash_binario = hash;
    // La "fuente" mostrada es el token literal `@<hash>` recortado.
    let cap = 64.min(FUENTE_CAP);
    celda.fuente[..cap].copy_from_slice(&editor[..cap]);
    celda.fuente_len = cap;

    // 2. Pedirle al kernel el dictamen de vinculacion (4 B).
    let mut dictamen = [0u8; 4];
    let cod_vinc = unsafe {
        sys_subsistema_vincular_macro(
            celda.hash_binario.as_ptr() as u32,
            dictamen.as_mut_ptr() as u32,
        )
    };
    if cod_vinc != 0 || dictamen[0] != 0x01 {
        // Codigos: -1 Ausente, 0 con dictamen[0]=0 → binario invalido o
        // no expone `run`. Cualquier otro negativo: falla del syscall.
        // Reportamos el codigo si es negativo, o -7 si la inspeccion
        // rechazo el binario.
        celda.retorno = if cod_vinc < 0 { cod_vinc } else { -7 };
        celda.exito = false;
        unsafe {
            RETORNO_HEREDADO_VALIDO = false;
            CADENA_ROTA = true;
        }
        registrar_celda_local(celda);
        return;
    }

    // 3. Ejecutar la macro. NOTA :: en este MVP la cascada NO inyecta el
    //    retorno heredado al sub-proceso — el binario importado fue
    //    compilado con su pila propia y la ABI actual de
    //    `sys_subsistema_ejecutar_dinamico` solo soporta `() -> i32`.
    //    Documentamos el HER del cuaderno solo como cortesia visual.
    //    Convergencia con `(i32) -> i32` queda para Fase 37+.
    let retorno = unsafe {
        sys_subsistema_ejecutar_dinamico(celda.hash_binario.as_ptr() as u32)
    };
    let es_falla = retorno <= -1 && retorno >= -7;
    celda.retorno = retorno;
    celda.exito = !es_falla;
    // El campo `heredado` queda en false para celdas importadas: aunque
    // exista un RETORNO_HEREDADO disponible, no se inyecta al binario;
    // el indicador visual del chevron `>` no tiene sentido aqui.
    celda.heredado = false;

    // 4. Consolidar la celda en el grafo. `hash_fuente` queda en cero
    //    (no hay fuente Forth nueva — la fuente vive en otra app, en
    //    otra celda, en otro cuaderno). El nodo cuaderno asi formado
    //    apunta solo a `hash_binario` como hijo.
    let cod_cuaderno = unsafe {
        sys_cuaderno_registrar_celda(
            celda.hash_fuente.as_ptr() as u32,
            celda.hash_binario.as_ptr() as u32,
            celda.retorno,
            core::ptr::addr_of_mut!(HASH_CUADERNO) as u32,
        )
    };
    unsafe { HASH_CUADERNO_VALIDO = cod_cuaderno == 0 };

    unsafe {
        if celda.exito {
            RETORNO_HEREDADO = retorno;
            RETORNO_HEREDADO_VALIDO = true;
            CADENA_ROTA = false;
        } else {
            RETORNO_HEREDADO_VALIDO = false;
            CADENA_ROTA = true;
        }
    }

    registrar_celda_local(celda);
}

/// FASE 37 :: Firma del Tejido Celular. Empaqueta un sobre `CuadernoFirmado`
/// con el `HASH_CUADERNO` mas reciente y se lo pasa al kernel. La clave
/// privada del operador no vive en la jaula (politica de la Fase 25:
/// solo el kernel VERIFICA, jamas FIRMA dentro de Ring 0), de modo que
/// el sobre lleva la `autor` pinneada a `AGORA_PUBLIC_KEY_LOCAL` y una
/// firma PLACEHOLDER. El kernel verificara matematicamente y la rechazara
/// con `AlmacenamientoFallo` — el camino visual rotula "FIRMA INVALIDA".
/// Cuando `wawactl` o una clave de sesion del kernel se conecten en la
/// Fase 38+, la `firma` real reemplazara el placeholder y el camino
/// feliz quedara disponible — la infraestructura de Ring 0 ya esta hoy.
fn firmar_y_anclar_cuaderno() {
    if !unsafe { HASH_CUADERNO_VALIDO } {
        // No hay cuaderno consolidado para firmar. Reportamos sin tocar
        // al kernel; la leyenda se queda igual.
        unsafe {
            SOBERANIA_FIRMA_ROTA = false;
            SOBERANIA_ANCLADA = false;
        }
        return;
    }

    // Llave publica del operador local — empotrada en el kernel desde
    // la Fase 25. La constante esta acordada con
    // `claves::AGORA_PUBLIC_KEY_LOCAL` y vive en este array para que el
    // sobre que enviamos sea estructuralmente legitimo (autor reconocido).
    const AGORA_PUBLIC_KEY_LOCAL: [u8; 32] = [
        0x1a, 0x4f, 0x7c, 0x91, 0xb6, 0x2d, 0x5e, 0xa8,
        0x33, 0xc7, 0x09, 0x84, 0xf1, 0x60, 0xb5, 0x52,
        0x6e, 0xae, 0x17, 0x40, 0x82, 0xfb, 0x99, 0xc1,
        0x2d, 0x55, 0xd6, 0x3a, 0xe4, 0x77, 0x1c, 0x80,
    ];

    // Sobre serializado MANUALMENTE en pila. Postcard, para una struct
    // con tres arrays de tamaño fijo, NO inserta headers de longitud:
    // cuaderno_raiz_hash (32) + autor (32) + firma (64) = 128 B EXACTOS.
    // El buffer se padea a 256 B con ceros para satisfacer la lectura
    // de 256 B que el kernel hace (cota dura MAX_CF). Pila pura,
    // zero-alloc. `postcard::take_from_bytes` consume solo el prefijo
    // de 128 B; los ceros del padding son inofensivos.
    let mut sobre = [0u8; 256];
    let hash_cuaderno = unsafe { HASH_CUADERNO };
    sobre[0..32].copy_from_slice(&hash_cuaderno);
    sobre[32..64].copy_from_slice(&AGORA_PUBLIC_KEY_LOCAL);
    // Firma PLACEHOLDER (ceros, ya inicializados arriba). El kernel
    // la rechazara matematicamente — Ed25519 sobre todo-ceros no verifica
    // para ninguna llave publica realista. La estructura del sobre es
    // legitima; la criptografia, no. Cuando un signer real este
    // disponible (wawactl o clave de sesion del kernel en Fase 38+),
    // sobre[64..128] llevara los 64 bytes de la firma autentica y el
    // kernel anclara con Ok(0).

    let codigo = unsafe { sys_cuaderno_firmar_y_anclar(sobre.as_ptr() as u32) };
    unsafe {
        SOBERANIA_CODIGO = codigo;
        if codigo == 0 {
            SOBERANIA_ANCLADA = true;
            SOBERANIA_FIRMA_ROTA = false;
        } else {
            SOBERANIA_ANCLADA = false;
            SOBERANIA_FIRMA_ROTA = true;
        }
    }
}

/// Inserta `celda` en el array circular. Cuando el cuaderno se llena, la
/// celda mas antigua se sobreescribe — el grafo guarda todas las
/// historias, pero el panel solo retiene las ultimas tres.
fn registrar_celda_local(celda: Celda) {
    unsafe {
        let slot = PROXIMA_CELDA;
        CELDAS[slot] = celda;
        PROXIMA_CELDA = (slot + 1) % MAX_CELDAS;
    }
}

// =============================================================================
//  Pintado
// =============================================================================

fn pintar() {
    let paleta = unsafe { PALETA };
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };

    let fondo = color_u32(paleta, 2);
    let tinta = color_u32(paleta, 3);
    let acento = color_u32(paleta, 4);
    let secundario = color_u32(paleta, 1);

    rellenar_rect(lienzo, 0, 0, ANCHO, ALTO, fondo);

    // Cabecera con el hash del cuaderno (si ya hubo una consolidacion).
    rellenar_rect(lienzo, 0, 0, ANCHO, EDITOR_Y - 4, secundario);
    dibujar_texto(lienzo, b"PLUMA  WAWA  F37", 8, 6, 1, tinta);
    if unsafe { HASH_CUADERNO_VALIDO } {
        let h = unsafe { HASH_CUADERNO };
        let mut etiqueta = [b' '; 8];
        etiqueta[0] = nibble_hex(h[0] >> 4);
        etiqueta[1] = nibble_hex(h[0] & 0x0F);
        etiqueta[2] = nibble_hex(h[1] >> 4);
        etiqueta[3] = nibble_hex(h[1] & 0x0F);
        etiqueta[4] = b'.';
        etiqueta[5] = b'.';
        etiqueta[6] = nibble_hex(h[31] >> 4);
        etiqueta[7] = nibble_hex(h[31] & 0x0F);
        dibujar_texto(lienzo, &etiqueta, ANCHO - 8 - 8 * 6, 6, 1, acento);
    }
    rellenar_rect(lienzo, 0, EDITOR_Y - 4, ANCHO, 2, acento);

    pintar_editor(lienzo, tinta, acento, secundario);
    pintar_celdas(lienzo, tinta, acento, secundario);

    // FASE 34 :: la leyenda rota a la version de cadena rota cuando la
    // ultima ejecucion fallo en mitad de un flujo encadenado. La proxima
    // celda exitosa restaura la leyenda normal automaticamente.
    // FASE 37 :: la soberania anclada tiene prioridad visual sobre la
    // leyenda comun. Una firma rechazada deja el rotulo de integridad
    // comprometida hasta que un nuevo F6 exitoso lo restaure.
    let leyenda: &[u8] = if unsafe { SOBERANIA_FIRMA_ROTA } {
        b"FIRMA INVALIDA  INTEGRIDAD COMPROMETIDA"
    } else if unsafe { SOBERANIA_ANCLADA } {
        b"CUADERNO SOBERANO ANCLADO  OK"
    } else if unsafe { CADENA_ROTA } {
        b"ERROR  CADENA DE EJECUCION ROTA"
    } else {
        b"F5 EJECUTA  F6 FIRMA  BS BORRA  @HASH MACRO"
    };
    dibujar_texto(lienzo, leyenda, 8, LEYENDA_Y, 1, acento);

    // FASE 37 :: marco vertical amarillo palido a lo largo del borde
    // izquierdo del lienzo cuando la integridad esta comprometida. Es
    // una pista perimetral inconfundible — no oculta el contenido pero
    // recuerda que el cuaderno no quedo anclado como soberano.
    if unsafe { SOBERANIA_FIRMA_ROTA } {
        rellenar_rect(lienzo, 0, 0, 3, ALTO, AMARILLO_PALIDO);
    }
}

fn pintar_editor(lienzo: &mut [u32], tinta: u32, acento: u32, secundario: u32) {
    dibujar_texto(lienzo, b"EDITOR  TECLEA FORTH", 8, EDITOR_Y, 1, acento);
    rellenar_rect(
        lienzo,
        8,
        EDITOR_Y + 12,
        ANCHO - 16,
        EDITOR_ALTO - 16,
        color_atenuar_u32(secundario, 0xC0),
    );

    let len = unsafe { EDITOR_LEN };
    let mut buf = [b' '; FUENTE_CAP];
    let mut n = 0;
    for i in 0..len {
        let c = unsafe { EDITOR[i] };
        if es_renderable(c) {
            buf[n] = c.to_ascii_uppercase();
            n += 1;
        }
    }
    dibujar_texto(lienzo, &buf[..n], 14, EDITOR_Y + 18, 2, tinta);

    // Cursor parpadeando — un rectanguillo al final del buffer renderizado.
    let cursor_x = 14 + n * 12;
    rellenar_rect(lienzo, cursor_x, EDITOR_Y + 18, 4, 14, acento);
}

fn pintar_celdas(lienzo: &mut [u32], tinta: u32, acento: u32, secundario: u32) {
    dibujar_texto(lienzo, b"CELDAS  GRAFO", 8, CELDAS_Y - 12, 1, acento);

    for i in 0..MAX_CELDAS {
        let y = CELDAS_Y + i * CELDA_ALTO;
        let celda = unsafe { CELDAS[i] };
        // Fondo de la celda: amarillo palido cuando la cadena fallo,
        // gris atenuado en exito o ranura vacia.
        let fondo_celda = if celda.valida && !celda.exito {
            AMARILLO_PALIDO
        } else {
            color_atenuar_u32(secundario, 0xA0)
        };
        rellenar_rect(lienzo, 8, y, ANCHO - 16, CELDA_ALTO - 4, fondo_celda);

        // FASE 36 :: indicador vertical de 5 px en el borde IZQUIERDO de
        // las celdas que importaron un binario via `@<hash>`. Tinte
        // acento (maestro) — el ojo lo distingue de las celdas que
        // nacieron compilando Forth en este mismo cuaderno.
        if celda.valida && celda.macro_importada {
            rellenar_rect(lienzo, 8, y, 5, CELDA_ALTO - 4, acento);
        }

        // Etiqueta de slot (CELDA 1/2/3).
        let etiqueta = [b'C', b'E', b'L', b'D', b'A', b' ', b'1' + i as u8];
        dibujar_texto(lienzo, &etiqueta, 14, y + 4, 1, acento);

        // FASE 36 :: rotulo "MACRO" en la esquina superior derecha si
        // la celda viene del puente inter-app. La etiqueta "HER N" no
        // aplica aqui (no hay cascada hacia el binario importado todavia).
        if celda.macro_importada {
            dibujar_texto(lienzo, b"MACRO", ANCHO - 14 - 5 * 6, y + 4, 1, acento);
        }

        // FASE 34 :: chevron `>` en el margen IZQUIERDO de las celdas que
        // heredaron un valor de su predecesora exitosa. El glifo se
        // dibuja en escala 2 para que sea inconfundible incluso a
        // distancia, y se acompaña de "HER N" rotulando el valor
        // exacto que vino de la cadena.
        if celda.heredado {
            dibujar_texto(lienzo, b">", 2, y + 18, 2, acento);
            let mut etiq_her = [b' '; 14];
            let prefijo = b"HER ";
            etiq_her[..prefijo.len()].copy_from_slice(prefijo);
            let (dec, dlen) = formatear_i32(celda.valor_heredado);
            let mut p = prefijo.len();
            for &c in &dec[..dlen] {
                if p < etiq_her.len() {
                    etiq_her[p] = c;
                    p += 1;
                }
            }
            dibujar_texto(lienzo, &etiq_her[..p], ANCHO - 14 - p * 6, y + 4, 1, acento);
        }

        if !celda.valida {
            dibujar_texto(lienzo, b"VACIA", ANCHO - 14 - 5 * 6, y + 4, 1, tinta);
            continue;
        }

        // Linea 1: fragmento de la fuente EFECTIVA (incluye el valor
        // heredado si la celda lo recibio). Hasta 24 chars.
        let mut linea_src = [b' '; 24];
        let cap = celda.fuente_len.min(linea_src.len());
        for k in 0..cap {
            let c = celda.fuente[k];
            linea_src[k] = if es_renderable(c) { c.to_ascii_uppercase() } else { b' ' };
        }
        dibujar_texto(lienzo, &linea_src, 14, y + 18, 1, tinta);

        // Linea 2: hash del binario, primeros 16 nibbles.
        let mut linea_hash = [b'-'; 16];
        for k in 0..8 {
            let b = celda.hash_binario[k];
            linea_hash[k * 2] = nibble_hex(b >> 4);
            linea_hash[k * 2 + 1] = nibble_hex(b & 0x0F);
        }
        dibujar_texto(lienzo, b"BIN ", 14, y + 32, 1, acento);
        dibujar_texto(lienzo, &linea_hash, 14 + 4 * 6, y + 32, 1, tinta);

        // Linea 3 (grande): OUT  <retorno> o OUT TRAP cuando fallo.
        let mut linea_out = [b' '; 24];
        let prefix = b"OUT  ";
        linea_out[..prefix.len()].copy_from_slice(prefix);
        if celda.exito {
            let (dec, dlen) = formatear_i32(celda.retorno);
            let mut p = prefix.len();
            for &c in &dec[..dlen] {
                if p < linea_out.len() {
                    linea_out[p] = c;
                    p += 1;
                }
            }
            dibujar_texto(lienzo, &linea_out[..prefix.len() + dlen], 14, y + 48, 2, tinta);
        } else {
            let etiqueta = match celda.retorno {
                -7 => b"TRAP    ".as_slice(),
                -6 => b"SATURADO".as_slice(),
                -2 => b"CAP INSF".as_slice(),
                -3 => b"DISCO   ".as_slice(),
                -1 => b"AUSENTE ".as_slice(),
                _ => b"ERROR   ".as_slice(),
            };
            let p = prefix.len();
            let cap = etiqueta.len().min(linea_out.len() - p);
            linea_out[p..p + cap].copy_from_slice(&etiqueta[..cap]);
            dibujar_texto(lienzo, &linea_out[..p + cap], 14, y + 48, 2, tinta);
        }
    }
}

fn volcar() {
    let lienzo: &[u32] = unsafe { &*core::ptr::addr_of!(LIENZO) };
    unsafe { sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32) };
}

// =============================================================================
//  Helpers de color y dibujo
// =============================================================================

const AMARILLO_PALIDO: u32 = 0x00FFEEA0;

fn color_u32(paleta: [u8; 20], n: usize) -> u32 {
    let base = n * 4;
    let r = paleta[base] as u32;
    let g = paleta[base + 1] as u32;
    let b = paleta[base + 2] as u32;
    b | (g << 8) | (r << 16)
}

fn color_atenuar_u32(color: u32, factor: u32) -> u32 {
    let b = (color & 0xFF) * factor >> 8;
    let g = ((color >> 8) & 0xFF) * factor >> 8;
    let r = ((color >> 16) & 0xFF) * factor >> 8;
    b | (g << 8) | (r << 16)
}

fn rellenar_rect(lienzo: &mut [u32], x: usize, y: usize, w: usize, h: usize, color: u32) {
    let x_fin = (x + w).min(ANCHO);
    let y_fin = (y + h).min(ALTO);
    for fila in y..y_fin {
        let base = fila * ANCHO;
        for col in x..x_fin {
            lienzo[base + col] = color;
        }
    }
}

fn es_renderable(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b' ' || c == b'+' || c == b'-' || c == b'*' || c == b'@'
}

fn nibble_hex(n: u8) -> u8 {
    if n < 10 {
        b'0' + n
    } else {
        b'A' + (n - 10)
    }
}

fn formatear_i32(n: i32) -> ([u8; 12], usize) {
    let mut buf = [0u8; 12];
    if n == 0 {
        buf[0] = b'0';
        return (buf, 1);
    }
    let (mut absoluto, negativo) = if n < 0 {
        (n.unsigned_abs(), true)
    } else {
        (n as u32, false)
    };
    let mut digits = [0u8; 11];
    let mut nd = 0;
    while absoluto > 0 && nd < digits.len() {
        digits[nd] = b'0' + (absoluto % 10) as u8;
        absoluto /= 10;
        nd += 1;
    }
    let mut out = 0;
    if negativo {
        buf[out] = b'-';
        out += 1;
    }
    while nd > 0 {
        nd -= 1;
        buf[out] = digits[nd];
        out += 1;
    }
    (buf, out)
}

// =============================================================================
//  Mini-tipografia 5x7 (copia local — el ABI del userspace renaser no expone
//  glifos compartidos por ahora; cada app trae su tabla).
// =============================================================================

const FA: usize = 5;
const FH: usize = 7;
const FAV: usize = 6;

fn glifo(c: u8) -> [u8; FH] {
    match c {
        b' ' => [0; 7],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        b':' => [0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00],
        b'+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        b'*' => [0x00, 0x0A, 0x04, 0x1F, 0x04, 0x0A, 0x00],
        // FASE 34 :: chevron derecho — indicador de herencia en cascada.
        b'>' => [0x10, 0x08, 0x04, 0x02, 0x04, 0x08, 0x10],
        // FASE 36 :: arroba — prefijo del token de importacion de macros.
        b'@' => [0x0E, 0x11, 0x17, 0x15, 0x17, 0x10, 0x0E],
        b'0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        b'1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        b'3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        b'4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        b'5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        b'6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        b'7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        b'8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        b'9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        b'A' => [0x0E, 0x11, 0x11, 0x11, 0x1F, 0x11, 0x11],
        b'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        b'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        b'D' => [0x1E, 0x09, 0x09, 0x09, 0x09, 0x09, 0x1E],
        b'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        b'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        b'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        b'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        b'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        b'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        b'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        b'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        b'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        b'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        b'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        b'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        b'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        b'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        b'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        b'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        b'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        b'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        _ => [0x1F; 7],
    }
}

fn dibujar_texto(lienzo: &mut [u32], texto: &[u8], x: usize, y: usize, escala: usize, color: u32) {
    let mut cursor_x = x;
    for &c in texto {
        let g = glifo(c);
        for (fila, bits) in g.iter().enumerate() {
            for col in 0..FA {
                if bits & (1 << (FA - 1 - col)) != 0 {
                    let px0 = cursor_x + col * escala;
                    let py0 = y + fila * escala;
                    rellenar_rect(lienzo, px0, py0, escala, escala, color);
                }
            }
        }
        cursor_x += FAV * escala;
    }
}
