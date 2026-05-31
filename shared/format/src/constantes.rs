// =============================================================================
//  Constantes del format en disco
// =============================================================================

/// Firma magica del superbloque — «RENASer GRaFo». Distingue un disco de
/// renaser de uno virgen o ajeno.
pub const MAGIA: [u8; 8] = *b"RENASGRF";

/// Version del format del superbloque en disco. Un disco con otra version se
/// reformatea al arrancar. v3 (Fase 24) — el superbloque porta `log_inicio`:
/// el sector donde arranca el log activo. El compactador semantico copia el
/// set alcanzable a una zona limpia del disco y reanca el superbloque a un
/// nuevo `log_inicio` en una sola escritura atomica. v2 (Fase 7) ya portaba
/// el ancla `manifiesto`, gemela de `raiz`.
pub const VERSION_SUPERBLOQUE: u32 = 3;

/// Version del format del manifiesto serializado. Independiente de la del
/// superbloque: el manifiesto es un objeto del grafo, no una estructura fija
/// del disco. v4 — cada `EntradaApp` declara su `permisos: u32`: un bitfield
/// que dicta QUE capacidades el kernel enlaza en su `Linker` de wasmi. Las
/// capacidades sensibles (red, raiz, altavoz, configuracion, escritura del
/// grafo) no se REGISTRAN si el bit no esta puesto: la frontera es fisica,
/// no chequeada en runtime. No hay escalada porque no hay tabla que escalar.
///
/// v5 (Fase 67 / WAWA §14.1.3) — cada `EntradaApp` gana `concesion:
/// Option<Hash>`: el hash de una [`ConcesionCapacidad`] firmada por el
/// `AGORA_AUTH_RING` sobre `(bytecode, permisos)`. Cuando una app la declara,
/// el kernel toma la INTERSECCION [`permisos_efectivos`]`(declarados,
/// concedidos)` — un manifiesto re-firmado ya no puede escalar un binario mas
/// alla de lo que su concesion, atada a su hash, autoriza. Si `concesion` es
/// `None` no hay techo per-bytecode: gobierna la firma del manifiesto (camino
/// legacy, rollout escalonado — ver `SDD-capacidades.md` §3.6).
///
/// CORTE DE FORMATO: `postcard` NO es autodescriptivo, asi que cada campo nuevo
/// rompe el wire de la version previa. Un disco viejo NO deserializa — el guardia
/// de version (`Manifiesto::deserializar` exige `version == VERSION_MANIFIESTO`)
/// lo rechaza y exige re-sembrar el genesis. En la practica el operador re-forja
/// la imagen en cada `cargo run -p boot`, asi que la genesis nace limpia.
/// v5→v6 (2026-05-30): agrega `overlay_revocacion: Option<Hash>` para el plano de
/// control del SDD-rotacion-revocacion §4.
pub const VERSION_MANIFIESTO: u32 = 6;

/// Version del format de la `Configuracion` serializada. La configuracion es
/// otro objeto del grafo (idioma + paleta); el manifiesto la enlaza por hash.
/// v1 inaugura el modelo: cambiarla es engendrar un nodo nuevo y reanclar.
pub const VERSION_CONFIGURACION: u32 = 1;

/// Version del format del canal de release serializado. Independiente del
/// manifiesto: un canal es otro objeto del grafo, con su propia historia de
/// raices recomendadas. v1 inaugura el modelo de distribucion.
pub const VERSION_CANAL: u32 = 1;

/// Techo del nombre de un canal, en bytes. Acota la cabecera serializada y
/// fuerza a que los canales se nombren cortos —`estable`, `beta`, `dev`,
/// `cofradia-tal`—. Quien intente registrar un canal con un nombre mas largo
/// se topa con un error de deserializacion.
pub const NOMBRE_CANAL_LIMITE: usize = 64;

/// Techo del tamaño de un objeto serializado: 1 MiB. Acota los buferes de E/S
/// y permite descartar un registro corrupto sin leer un disparate.
pub const MAX_OBJETO: usize = 1024 * 1024;

/// Tamaño de un sector del disco, en bytes. El log se traza en multiplos de
/// esta unidad — la misma que expone el transporte virtio-blk.
pub const TAM_SECTOR: usize = 512;

/// El identificador de un objeto: el hash BLAKE3 de su forma serializada. En
/// un almacen direccionado por contenido, la identidad ES el contenido.
pub type Hash = [u8; 32];

// =============================================================================
//  CodigoError — el lenguaje de los retornos de syscall, sin alucinaciones
// -----------------------------------------------------------------------------
//  Los retornos negativos de las capacidades `sys_*` no son enteros opacos:
//  son variantes nombradas, fuertemente tipadas, con un valor i32 estable.
//  El kernel emite `CodigoError::X as i32`; el userspace compara contra el
//  mismo numero. Anadir una variante NUEVA es engendrar un valor nuevo (las
//  existentes jamas se renumeran), de modo que un binario viejo y un kernel
//  nuevo siguen hablando el mismo idioma para los codigos que ambos conocen.
//
//  Los retornos POSITIVOS de algunas capacidades son cuentas de bytes copiados
//  —no errores—; por eso `Ok = 0` y todos los errores caen en negativos. La
//  comparacion habitual del userspace queda intacta: `< 0` ya es "fallo", y
//  el codigo concreto lo describe.
// =============================================================================

/// El catalogo de retornos negativos de las capacidades del kernel. Un solo
/// nombre por causa: nadie ha de inventarse una semantica nueva para el -1.
///
/// Mantenido en `format` porque viaja por TRES fronteras: el kernel lo emite,
/// el explorador (host-side) lo lee de las trazas, y los modulos WASM lo
/// reciben. La crate ya es no_std y la traen ambos lados sin friccion.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CodigoError {
    /// Operacion completada sin novedad. Las capacidades que devuelven un
    /// conteo de bytes usan tambien `0` para "no habia nada que entregar"
    /// (lectura sin frame, sin evento, sin estado previo); el contexto del
    /// retorno positivo distingue ambos casos.
    Ok = 0,
    /// El recurso solicitado no esta presente: un objeto que no esta en el
    /// grafo, la tarjeta de red sin montar, una app sin estado previo, una
    /// cola del puntero o el teclado vacia. Tambien lo emite un guardar
    /// que no encontro su ranura.
    Ausente = -1,
    /// La capacidad recibida en `salida` no cubre los datos a copiar. La app
    /// debe llamar con un bufer mas amplio; el kernel no escribio nada en el
    /// destino.
    CapacidadInsuficiente = -2,
    /// El subsistema de almacenamiento (virtio-blk, log de objetos, censo del
    /// manifiesto) fallo al servir o anclar el objeto. NO es culpa del modulo,
    /// pero la operacion no pudo completarse.
    AlmacenamientoFallo = -3,
    /// La app no tiene el FOCO del compositor en este fotograma y la capacidad
    /// solo se honra para la ventana enfocada — por ejemplo, cambiar la
    /// `Configuracion` del escritorio. Reintentar cuando la app sea la
    /// destinataria del teclado.
    SinFoco = -4,
    /// El envio al dispositivo (driver de red, altavoz) fracaso. Lo emite el
    /// driver y la capacidad lo propaga: no hay rastro de bytes residuales en
    /// el hardware.
    EnvioFallo = -5,
    /// Cuota de recurso saturada para esta app en este fotograma: hay un
    /// limite blando que protege un recurso fisico (DMA, descriptores de un
    /// anillo virtio) y la app lo alcanzo. El kernel NO entrega la
    /// operacion ni avanza el contador; la app ha de retirarse y volver a
    /// intentar en su proximo `tick` —cuando la IRQ del hardware haya
    /// liberado los descriptores que tenia retenidos—. Es BACK-PRESSURE
    /// cooperativa: el equivalente de un `Poll::Pending` que cabe en un
    /// codigo de retorno entero. Distingue a una autodefensa del kernel
    /// frente al codigo de la app de un fallo del propio almacenamiento.
    Saturado = -6,
    /// El payload que la app entrego al kernel decodifica pero esta FUERA
    /// del dominio que la capacidad acepta — un codigo de idioma que no es
    /// letras ASCII, una paleta cuyos canales suman cero, un campo
    /// inconsistente con su contexto. Distinto de `Ausente` (recurso
    /// inexistente) y `CapacidadInsuficiente` (bufer corto): aqui los
    /// bytes llegaron pero su SIGNIFICADO los descalifica. La app ha
    /// de reconstruir su entrada con valores legitimos antes de reintentar.
    PayloadInvalido = -7,
}

impl CodigoError {
    /// Convierte el codigo a su forma de cable i32 — la unica que el userspace
    /// recibe. `as i32` directo, sin trampa: el `#[repr(i32)]` fija el valor.
    pub const fn como_i32(self) -> i32 {
        self as i32
    }
}

/// La identidad de un autor agora: una clave publica Ed25519, 32 bytes. Quien
/// firma una raiz de canal se identifica con esto. `format` no valida la
/// firma —no enlaza ninguna primitiva criptografica—; solo declara su forma.
/// La verificacion vive en `agora` (o en `firma`), donde corresponde.
pub type AgoraId = [u8; 32];

/// Una firma Ed25519, 64 bytes. La produce `agora` sobre el mensaje canonico
/// que devuelve [`mensaje_a_firmar`]. `format` la transporta y la deja a quien
/// pueda verificarla.
pub type Firma = [u8; 64];

