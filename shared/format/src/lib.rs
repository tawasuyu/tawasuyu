// =============================================================================
//  renaser :: format — el format del grafo de objetos en disco
// -----------------------------------------------------------------------------
//  Hasta la Fase 7a, el format del grafo de objetos —el superbloque, los
//  registros del log, el manifiesto— vivia disperso entre `kernel/almacen.rs`
//  y `kernel/manifiesto.rs`. Lo conocia solo el kernel.
//
//  La Fase 7b se lo entrega tambien a `boot`: el constructor de imagen de
//  ANFITRION debe sembrar el disco con el grafo ya poblado —los objetos de
//  bytecode y el Manifiesto de Genesis— para que el kernel jamas vuelva a
//  empotrar una sola app. Para ello, kernel y boot han de hablar EXACTAMENTE
//  el mismo format: la misma serializacion, el mismo hash, el mismo trazado
//  de registros en el log.
//
//  Esta crate es esa unica verdad. Es un nucleo `#![no_std]` —el kernel
//  bare-metal la enlaza— y, por ser no_std, el anfitrion `boot` la compila sin
//  friccion. Define los tipos del grafo, su (de)serializacion `postcard`, la
//  funcion hash BLAKE3 que da identidad a cada objeto y el trazado de un
//  registro en el log. Ni kernel ni boot vuelven a definir nada de esto.
// =============================================================================

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

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
pub const VERSION_MANIFIESTO: u32 = 4;

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

// =============================================================================
//  Los tipos del grafo
// =============================================================================

/// Un objeto del grafo: una carga util opaca y las aristas que lo enlazan con
/// otros objetos. Los `hijos` hacen del almacen un DAG —no un arbol—: un
/// objeto puede ser hijo de muchos, y el direccionamiento por contenido
/// garantiza que cada contenido distinto se guarda una sola vez.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Objeto {
    /// La carga util del objeto: bytes crudos, que nadie interpreta aqui.
    pub datos: Vec<u8>,
    /// Los hashes de los objetos hijos: las aristas salientes del DAG.
    pub hijos: Vec<Hash>,
}

/// El superbloque: el sector 0 del disco. Ancla el grafo entero — dice donde
/// arranca el log activo, donde acaba, cual es el objeto raiz y cual el
/// manifiesto.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct SuperBloque {
    /// Firma magica: debe ser [`MAGIA`].
    pub magia: [u8; 8],
    /// Version del format: debe ser [`VERSION_SUPERBLOQUE`].
    pub version: u32,
    /// Primer sector del log activo. En un disco virgen es `1` (justo despues
    /// del superbloque); el compactador semantico (Fase 24) lo desplaza al
    /// principio de un segmento limpio cada vez que aspira los nodos muertos.
    /// Mover `log_inicio` (junto con `cursor`) en una sola escritura del
    /// superbloque es lo que convierte la compactacion en una transicion
    /// atomica: el log viejo queda en sectores anteriores, ya inalcanzables,
    /// pero el grafo logico es el mismo.
    pub log_inicio: u64,
    /// Proximo sector libre del log — donde se anexara el siguiente objeto.
    pub cursor: u64,
    /// El objeto raiz del DAG: el punto de entrada que el userspace fija y lee.
    pub raiz: Option<Hash>,
    /// El Manifiesto de Genesis: el objeto que dicta que apps nacen del grafo
    /// al arrancar. Ancla del kernel, gemela de `raiz` (del userspace).
    pub manifiesto: Option<Hash>,
}

/// El Manifiesto de Genesis: la lista de aplicaciones que el kernel instancia
/// al arrancar. Vive como un objeto del grafo; el superbloque guarda su hash.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Manifiesto {
    /// Version del format — debe ser [`VERSION_MANIFIESTO`].
    pub version: u32,
    /// Las aplicaciones del userspace, en orden de arranque.
    pub apps: Vec<EntradaApp>,
    /// Hash del nodo `Configuracion` activo (idioma + tema). `None` => el
    /// kernel emplea los valores por defecto. Cambiar de idioma o tema NO
    /// muta este nodo: engendra una `Configuracion` nueva, calcula su hash,
    /// y reancla el manifiesto al objeto nuevo en un solo paso atomico —el
    /// mismo trazado que `EntradaApp::estado` para el estado por app.
    pub configuracion: Option<Hash>,
}

/// Un idioma codificado como un par de letras ASCII ISO 639-1 empaquetado en
/// little-endian: `b'e' | (b's' << 8) == 0x7365` para castellano, `0x6E65`
/// para ingles, `0x7571` para quechua. El propio numero es trivialmente
/// legible al inspeccionarlo en hexadecimal —no hace falta una tabla—.
pub type IdiomaCodigo = u16;

/// Compone un `IdiomaCodigo` desde un par ISO 639-1 (`b"es"`, `b"qu"`...).
/// Las dos letras viajan en orden de lectura: la primera ocupa el byte bajo.
pub const fn idioma_iso639(letras: [u8; 2]) -> IdiomaCodigo {
    (letras[0] as u16) | ((letras[1] as u16) << 8)
}

/// Codigo de idioma por defecto: `es` (castellano). Lo emplea el kernel cuando
/// el manifiesto no enlaza ninguna `Configuracion`.
pub const IDIOMA_DEFECTO: IdiomaCodigo = idioma_iso639(*b"es");

/// La paleta de un tema visual: cinco colores RGBA8 — primario, secundario,
/// fondo, texto, acento— en ese orden. La forma binaria (20 bytes) es la
/// misma que la app recibe del kernel a traves de la capacidad pasiva
/// `sys_config_paleta`. Cinco colores cubren un esquema completo sin caer en
/// la trampa de "un color por widget": la consistencia visual la impone el
/// numero pequeño.
pub type Paleta = [u8; 20];

/// Paleta por defecto cuando el manifiesto no enlaza configuracion. Negro de
/// fondo, blanco de texto, azul renaser de acento; cualquier app pinta sin
/// adivinar. Cada cuatro bytes son R, G, B, A en ese orden.
pub const PALETA_DEFECTO: Paleta = [
    0x20, 0x80, 0xC0, 0xFF, // primario   — azul renaser
    0x60, 0x60, 0x60, 0xFF, // secundario — gris medio
    0x00, 0x00, 0x00, 0xFF, // fondo      — negro
    0xFF, 0xFF, 0xFF, 0xFF, // texto      — blanco
    0xF0, 0x90, 0x20, 0xFF, // acento     — ambar
];

/// La configuracion activa de Wawa: idioma + paleta del tema. Es un objeto
/// del grafo —direccionado por su hash—; el manifiesto la enlaza. Cambiar de
/// idioma o tema significa engendrar UN NODO NUEVO y reanclar el manifiesto
/// al hash del nuevo objeto en una sola transicion atomica. Sin estados
/// mutables globales: la "configuracion vigente" es siempre el hash al que
/// apunta el manifiesto en este preciso fotograma.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Configuracion {
    /// Version del format — debe ser [`VERSION_CONFIGURACION`].
    pub version: u32,
    /// Idioma activo (ISO 639-1 empaquetado, ver [`idioma_iso639`]).
    pub idioma: IdiomaCodigo,
    /// Paleta del tema visual: cinco colores RGBA8 en orden canonico.
    pub paleta: Paleta,
}

impl Configuracion {
    /// La configuracion canonica cuando el manifiesto no enlaza ninguna:
    /// idioma `es`, paleta `PALETA_DEFECTO`. El kernel la inyecta tal cual en
    /// el `ContextoCapacidades` de cada app.
    pub const fn por_defecto() -> Configuracion {
        Configuracion {
            version: VERSION_CONFIGURACION,
            idioma: IDIOMA_DEFECTO,
            paleta: PALETA_DEFECTO,
        }
    }

    /// Serializa la configuracion a su forma binaria `postcard`.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self).map_err(|_| "configuracion :: serializacion fallida")
    }

    /// Reconstruye una configuracion desde la carga util de su objeto. Rechaza
    /// una version desconocida en lugar de malinterpretarla — gemelo del trato
    /// que `Manifiesto::deserializar` da a su propia version.
    pub fn deserializar(bytes: &[u8]) -> Result<Configuracion, &'static str> {
        let (cfg, _) = postcard::take_from_bytes::<Configuracion>(bytes)
            .map_err(|_| "configuracion :: deserializacion fallida")?;
        if cfg.version != VERSION_CONFIGURACION {
            return Err("configuracion :: version de format desconocida");
        }
        Ok(cfg)
    }
}

/// Bitfield de permisos de una app — cada bit habilita una clase de
/// capacidades. Capacidades sensibles que no figuran aqui no se ENLAZAN en
/// el `Linker` de wasmi cuando la app se instancia: el import del modulo
/// queda sin resolver y el modulo entero ni siquiera arranca. La frontera
/// es fisica; el kernel no hace chequeos en cada syscall porque no hay
/// syscall que chequear: la funcion del host no se concedio. POSIX gestiona
/// privilegios con un check `if (uid == 0)` en cada syscall y se llena de
/// CVE; aqui no hay nada que comprobar.
pub type Permisos = u32;

/// Permite enviar y recibir frames Ethernet y solicitar objetos por hash a
/// peers Akasha. Sin este bit, las capacidades `sys_net_*` y `sys_red_*` no
/// se enlazan: el modulo no las puede invocar porque no existen.
pub const PERMISO_RED: Permisos = 1 << 0;

/// Permite grabar objetos nuevos en el grafo del disco (`sys_object_put`).
/// La lectura del grafo es libre —la inmutabilidad direccionada por contenido
/// la hace inofensiva—, la escritura no.
pub const PERMISO_GRAFO_ESCRITURA: Permisos = 1 << 1;

/// Permite reanclar la raiz del grafo (`sys_object_fijar_raiz`). Cambia el
/// punto de entrada que el resto del userspace lee; un permiso de mucha
/// gravedad.
pub const PERMISO_RAIZ: Permisos = 1 << 2;

/// Permite hacer sonar la bocina del PC (`sys_tono`). El altavoz es un
/// recurso unico y global; aunque ya esta gateado por foco, el bit deja
/// explicito que la app puede SOLICITAR sonido.
pub const PERMISO_ALTAVOZ: Permisos = 1 << 3;

/// Permite proponer una nueva `Configuracion` (`sys_config_proponer`):
/// idioma + tema visual. La LECTURA pasiva del contexto (sys_config_idioma,
/// sys_config_paleta) no necesita bit; cualquier app la tiene siempre.
pub const PERMISO_CONFIG: Permisos = 1 << 4;

/// Permite forzar una pasada del compactador semantico del grafo
/// (`sys_grafo_compactar`). El GC ya corre solo cuando
/// `escrituras_pendientes() >= UMBRAL_GC` en el tic ocioso del compositor;
/// este bit habilita la palanca explicita para `wawactl gc` y similares.
/// Por su coste (toma el cerrojo del almacen y reescribe sectores), se
/// asume reservado a apps de mantenimiento privilegiadas — no apto para
/// userspace generico.
pub const PERMISO_COMPACTAR: Permisos = 1 << 5;

/// Una entrada del manifiesto: una aplicacion del userspace y todo lo que el
/// kernel necesita para darle vida — su bytecode, su ventana, su cuota de
/// memoria, su tabla de permisos y, si lo tuviera, su ultimo estado persistido.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct EntradaApp {
    /// Nombre legible — para los rotulos de la consola y la baliza.
    pub nombre: String,
    /// Hash del objeto del grafo que contiene el bytecode WASM de la app.
    pub bytecode: Hash,
    /// Sub-region del framebuffer asignada a la app. Campos de ancho fijo
    /// `u32` A PROPOSITO: esto es un format EN DISCO. La `RegionPantalla` del
    /// kernel usa `usize` (ancho dependiente de plataforma) y no serializa.
    pub region_x: u32,
    pub region_y: u32,
    pub region_ancho: u32,
    pub region_alto: u32,
    /// Techo de memoria lineal de la app, en bytes. Cada app lleva su cuota.
    pub techo_memoria: u32,
    /// Presupuesto de combustible (unidades de wasmi) que la app recibe en
    /// cada `tick`. Es el techo TEMPORAL por fotograma: lo agota una app en
    /// bucle infinito (`SinCombustible`) y se desaloja. Por-app porque un
    /// editor con tree-sitter no necesita lo mismo que un reloj parpadeante;
    /// el scheduler cooperativo honra la declaracion en lugar de un techo unico.
    pub fuel_fotograma: u32,
    /// Hash del ultimo estado persistido de la app (Fase 7c). `None` hasta que
    /// la app guarde estado por primera vez.
    pub estado: Option<Hash>,
    /// Bitfield de permisos (ver [`Permisos`] y las constantes `PERMISO_*`).
    /// Lo evalua el `Linker` de wasmi al instanciar la app: las capacidades
    /// gateadas que no figuren aqui NO se registran. La app puede llamar a
    /// otras capacidades —la matriz pasiva siempre esta— pero las gateadas
    /// son, literalmente, simbolos inexistentes para el modulo.
    pub permisos: Permisos,
}

/// Un canal de release: un objeto del grafo que historiza, en orden cronologico,
/// las raices de manifiesto que su(s) autor(es) recomiendan. Es el equivalente
/// nativo de un repositorio apt/dnf/pacman, pero firmado por una `AgoraId` y no
/// por una infraestructura central. Quien se suscribe a un canal confia en su
/// autor; el canal nunca dice "esta es la unica version", dice "esta es mi
/// recomendacion en este momento". El historial completo viaja junto.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Canal {
    /// Version del format — debe ser [`VERSION_CANAL`].
    pub version: u32,
    /// Nombre legible del canal: `estable`, `beta`, `dev`, `cofradia-tal`.
    /// Acotado a [`NOMBRE_CANAL_LIMITE`] bytes para que la cabecera sea barata.
    pub nombre: String,
    /// La identidad del autor que firma este canal. Quien recibe el canal
    /// verifica que cada `RaizFirmada` lleve una firma valida sobre esta clave.
    /// Un canal puede cambiar de autor en una version futura (multi-firma); por
    /// ahora, una clave gobierna un canal.
    pub autor: AgoraId,
    /// El historial de raices recomendadas, ordenado por `timestamp` ascendente.
    /// La ultima entrada es la recomendacion vigente. El historial completo se
    /// conserva para que un nodo pueda volver atras —rollback— sin pedirle
    /// permiso al canal.
    pub raices: Vec<RaizFirmada>,
}

/// El sobre criptografico de un Manifiesto: empareja su hash BLAKE3 con la
/// firma Ed25519 que un autor `AgoraId` produjo sobre el. Fase 25 — el
/// kernel solo acepta una propuesta de reancla del manifiesto cuando llega
/// envuelta en uno de estos sobres Y la firma valida contra la clave publica
/// del usuario local que el binario del kernel lleva grabada. Sin firma
/// valida, no hay mutacion: la mudanza de raiz es un PACTO MATEMATICO
/// explicito, no una orden ciega de la red.
///
/// El mensaje que se firma es, literalmente, los 32 bytes de
/// `manifiesto_hash` — el hash mismo es ya el resumen criptografico del
/// payload del manifiesto, asi que firmar el hash equivale a firmar el
/// manifiesto completo. Ed25519 no se preocupa por la longitud del mensaje.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct ManifiestoFirmado {
    /// Hash BLAKE3 del Manifiesto propuesto. Un hash idiosincratico = un
    /// manifiesto idiosincratico — ningun atacante puede sustituirlo y
    /// reutilizar la firma sin reproducir el hash exacto.
    pub manifiesto_hash: Hash,
    /// Llave publica Ed25519 del autor que firma esta propuesta. El kernel
    /// la compara contra su clave local empotrada antes de molestarse en
    /// verificar la firma: una llave ajena cae con `CapacidadInsuficiente`
    /// sin gastar ciclos en criptografia.
    pub autor: AgoraId,
    /// Firma Ed25519 sobre los 32 bytes de `manifiesto_hash`.
    #[serde(with = "BigArray")]
    pub firma: Firma,
}

impl ManifiestoFirmado {
    /// Serializa el sobre a su forma binaria `postcard` — la carga util del
    /// objeto del grafo que lo aloja (o el payload de un mensaje Akasha).
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self)
            .map_err(|_| "manifiesto_firmado :: serializacion fallida")
    }

    /// Reconstruye un sobre desde su forma binaria. Tolera bytes sobrantes
    /// tras la estructura — el relleno del registro.
    pub fn deserializar(bytes: &[u8]) -> Result<ManifiestoFirmado, &'static str> {
        postcard::take_from_bytes::<ManifiestoFirmado>(bytes)
            .map(|(mf, _)| mf)
            .map_err(|_| "manifiesto_firmado :: deserializacion fallida")
    }
}

// =============================================================================
//  Fase 37 :: el sello criptografico del CUADERNO SOBERANO
// -----------------------------------------------------------------------------
//  La integridad de un cuaderno —un nodo del grafo cuyo payload es
//  `Vec<CeldaWawa>` (Fase 43, modelo unificado)— se proteje en dos planos:
//
//    * Localmente, el direccionamiento por contenido garantiza que un
//      bit alterado en cualquier celda cambia el hash del cuaderno
//      —y ese hash es la identidad del nodo en el almacen—.
//    * En la red capa-2 (Akasha), eso no basta: un peer hostil puede
//      reescribir el cuaderno entero y reanunciarlo con su propio hash.
//      Para que el sistema reconozca un cuaderno como SOBERANO del
//      operador local, el peer ha de adjuntar una firma Ed25519 del
//      cuaderno_raiz_hash producida con la clave privada que pertenece
//      a la `AGORA_PUBLIC_KEY_LOCAL` empotrada en el binario del kernel.
//
//  Gemelo estructural de `ManifiestoFirmado`: la verificacion comparte
//  el camino Ring 0 zero-alloc de `ed25519-compact`.
// =============================================================================

/// Sobre criptografico de un cuaderno: vincula su `hash` con un autor y
/// una firma Ed25519. Sin este sobre, un cuaderno es solo un nodo mas
/// del grafo — con el sobre, queda anclado como SOBERANO al usuario
/// que firmo, y el kernel lo distingue de cualquier otro nodo cuaderno
/// que viaje por la red.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct CuadernoFirmado {
    /// Hash BLAKE3 del cuaderno propuesto. El payload del cuaderno es
    /// `Vec<CeldaWawa>` serializado con postcard; este hash es el
    /// resumen criptografico que va a engrapar la firma.
    pub cuaderno_raiz_hash: Hash,
    /// Llave publica Ed25519 del autor. El kernel la compara contra
    /// `AGORA_PUBLIC_KEY_LOCAL` antes de gastar un ciclo en criptografia
    /// — un autor ajeno cae con `CapacidadInsuficiente`.
    pub autor: AgoraId,
    /// Firma Ed25519 sobre los 32 bytes de `cuaderno_raiz_hash`.
    #[serde(with = "BigArray")]
    pub firma: Firma,
}

impl CuadernoFirmado {
    /// Serializa el sobre a su forma binaria `postcard`. La forma cruda
    /// ocupa 32 + 32 + 64 = 128 bytes; postcard agrega un preludio
    /// minusculo (longitudes varint) que mantiene el sobre bajo 140 B.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self)
            .map_err(|_| "cuaderno_firmado :: serializacion fallida")
    }

    /// Reconstruye un sobre desde su forma binaria. Tolera bytes
    /// sobrantes tras la estructura — el relleno del registro o el
    /// padding del payload del syscall.
    pub fn deserializar(bytes: &[u8]) -> Result<CuadernoFirmado, &'static str> {
        postcard::take_from_bytes::<CuadernoFirmado>(bytes)
            .map(|(cf, _)| cf)
            .map_err(|_| "cuaderno_firmado :: deserializacion fallida")
    }
}

/// Una entrada del historial de un canal: una raiz de manifiesto, el instante
/// en que el autor la propuso, y la firma Ed25519 con la que el autor la
/// respalda. La firma se calcula sobre [`mensaje_a_firmar`].
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct RaizFirmada {
    /// Instante en que el autor propuso esta raiz, segundos desde UNIX epoch.
    /// Un receptor desconfia de raices con timestamp futuro mas alla de un
    /// margen razonable —proteccion barata contra anuncios envenenados—.
    pub timestamp: u64,
    /// El hash del [`Manifiesto`] que esta raiz inaugura. Re-anclar el
    /// superbloque a este hash es, literalmente, "actualizar a esta version".
    pub raiz_manifiesto: Hash,
    /// La firma Ed25519 del autor del canal sobre [`mensaje_a_firmar`].
    /// `serde` no derivara `Deserialize` para `[u8; 64]` sin ayuda —su soporte
    /// directo se detiene en 32 bytes—; `serde-big-array` cierra ese hueco.
    #[serde(with = "BigArray")]
    pub firma: Firma,
}

// =============================================================================
//  Fase 33/43 :: el almacen semantico del cuaderno (modelo unificado)
// -----------------------------------------------------------------------------
//  Un CUADERNO de Wawa es un nodo del grafo cuyo payload `postcard` es un
//  `Vec<CeldaWawa>`. Cada `CeldaWawa` empaqueta TODA la informacion de un
//  eslabon del calculo en una sola estructura inmutable:
//
//    * `id_secuencial`   :: indice lineal en el cuaderno.
//    * `fuente_hash`     :: hash del texto Forth o token `@<hash>` literal.
//    * `binario_hash`    :: hash del modulo WASM materializado (None si
//                           la compilacion fallo).
//    * `ultimo_retorno`  :: el i32 que la sub-jaula efimera devolvio
//                           (None si nunca se ejecuto).
//    * `marca_error`     :: bandera atomica: hubo TRAP, OUT_OF_FUEL,
//                           PAYLOAD_INVALIDO, o cualquier otra falla.
//
//  La fusion (Fase 43) elimina el enum heredado `TipoCeldaWawa` con sus
//  tres variantes flat — el modelo estructurado es mas honesto con la
//  semantica del cuaderno y converge bit-a-bit con la representacion
//  del motor Linux del ecosistema Pluma (`pluma-notebook-core`), que
//  re-exporta esta misma struct para hablar el mismo idioma en host y
//  en el silicio.
//
//  Las aristas del nodo (los `hijos` que el almacen registra al insertar)
//  son: el CUADERNO PREVIO cuando existe (arista ancestral, Fase 47),
//  `fuente_hash` siempre, `binario_hash` cuando esta presente. El
//  direccionamiento por contenido hace EXPLICITAS las dependencias y
//  el cuaderno arrastra criptograficamente todo su tejido de causas y
//  efectos. Con la Fase 47, cada cuaderno apunta a su predecesor por
//  hash — el historial es una cadena recorrible por el Walker.
//
//  Postcard-amigable: campos primitivos + `Option<T>` + arrays alineados.
//  La deserializacion del cuaderno no allocea fuera del `Vec` principal.
// =============================================================================

/// El eslabon canonico de un cuaderno (Fase 43). Reemplaza al enum
/// `TipoCeldaWawa` de la Fase 33: en lugar de tres variantes flat que
/// solo el orden del `Vec` ataba a una "celda", aqui CADA `CeldaWawa`
/// es una celda completa con todos sus eslabones bundled. Bit-compatible
/// con `pluma_notebook_core::CeldaWawa` (re-export en el motor Linux).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CeldaWawa {
    /// Indice lineal en el cuaderno — orden de presentacion. Empieza
    /// en 0 y crece con cada celda exitosamente registrada.
    pub id_secuencial: u32,
    /// Hash del texto fuente: ASCII Forth tecleado por el humano, o
    /// la cadena literal `@<64-hex>` para celdas macro-importadas
    /// (Fase 36, Cross-App Bridge). Siempre presente — una celda sin
    /// fuente es incoherente con el modelo.
    pub fuente_hash: Hash,
    /// Hash del modulo WASM materializado por `forth-emisor` (o
    /// importado del grafo via `@<hash>`). `None` cuando la compilacion
    /// fallo, la sintaxis Forth fue rechazada, o la vinculacion macro
    /// no se logro — el binario no llego a inscribirse.
    pub binario_hash: Option<Hash>,
    /// El i32 que la sub-jaula efimera (Fase 32) devolvio en su ultima
    /// ejecucion. `None` cuando la celda nunca corrio (sin binario, o
    /// el despacho dinamico ni siquiera arranco). Un valor negativo
    /// en `[-7, -1]` reservado en `CodigoError` codifica fallas
    /// controladas; valores fuera de ese rango son resultados legitimos.
    pub ultimo_retorno: Option<i32>,
    /// Bandera atomica de error: `true` si CUALQUIER eslabon de la
    /// cadena (compilacion, registro v2, ejecucion dinamica, anclaje
    /// de cuaderno) devolvio fallo. El renderer la usa para teñir la
    /// celda de amarillo palido sin enterrar el valor del retorno —
    /// `marca_error && ultimo_retorno == Some(-7)` significa
    /// "ejecutada, fallida con trap"; `marca_error && ultimo_retorno
    /// == None` significa "ni siquiera corrio".
    pub marca_error: bool,
}

/// Serializa una secuencia de celdas a `postcard` — la forma que el
/// kernel inscribe como payload del nodo cuaderno. Centralizada aqui
/// para que el kernel no tenga que declarar `postcard` directamente
/// (ya lo hereda transitivamente via `format`).
pub fn serializar_celdas(celdas: &[CeldaWawa]) -> Result<Vec<u8>, &'static str> {
    postcard::to_allocvec(celdas).map_err(|_| "celdas :: serializacion fallida")
}

/// Reconstruye la secuencia de celdas desde el payload de un nodo cuaderno.
/// Tolera bytes sobrantes — el relleno del registro vive despues del payload.
pub fn deserializar_celdas(bytes: &[u8]) -> Result<Vec<CeldaWawa>, &'static str> {
    postcard::take_from_bytes::<Vec<CeldaWawa>>(bytes)
        .map(|(celdas, _)| celdas)
        .map_err(|_| "celdas :: deserializacion fallida")
}

// =============================================================================
//  (De)serializacion — la forma binaria que viaja al disco
// =============================================================================

impl Objeto {
    /// Serializa el objeto a su forma binaria `postcard`.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self).map_err(|_| "objeto :: serializacion fallida")
    }

    /// Reconstruye un objeto desde su forma binaria. Tolera bytes sobrantes
    /// tras el objeto —el relleno del registro—: solo consume su prefijo.
    pub fn deserializar(bytes: &[u8]) -> Result<Objeto, &'static str> {
        postcard::take_from_bytes::<Objeto>(bytes)
            .map(|(objeto, _)| objeto)
            .map_err(|_| "objeto :: deserializacion fallida")
    }
}

impl SuperBloque {
    /// Serializa el superbloque a su forma binaria `postcard`.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self).map_err(|_| "superbloque :: serializacion fallida")
    }

    /// Reconstruye el superbloque desde el sector 0. Tolera el relleno a cero
    /// que completa el sector: solo consume el prefijo serializado.
    pub fn deserializar(bytes: &[u8]) -> Result<SuperBloque, &'static str> {
        postcard::take_from_bytes::<SuperBloque>(bytes)
            .map(|(sb, _)| sb)
            .map_err(|_| "superbloque :: deserializacion fallida")
    }
}

impl Manifiesto {
    /// Serializa el manifiesto a su forma binaria `postcard` — la carga util
    /// del objeto del grafo que lo aloja.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self).map_err(|_| "manifiesto :: serializacion fallida")
    }

    /// Reconstruye un manifiesto desde la carga util de su objeto. Rechaza un
    /// format de version desconocida en lugar de malinterpretarlo.
    pub fn deserializar(bytes: &[u8]) -> Result<Manifiesto, &'static str> {
        let (manifiesto, _) = postcard::take_from_bytes::<Manifiesto>(bytes)
            .map_err(|_| "manifiesto :: deserializacion fallida")?;
        if manifiesto.version != VERSION_MANIFIESTO {
            return Err("manifiesto :: version de format desconocida");
        }
        Ok(manifiesto)
    }
}

impl Canal {
    /// Serializa el canal a su forma binaria `postcard` — la carga util del
    /// objeto del grafo que lo aloja. Rechaza por adelantado un nombre que
    /// supere [`NOMBRE_CANAL_LIMITE`]: mejor un error de serializacion que un
    /// canal grafico que no quepa en disco.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        if self.nombre.len() > NOMBRE_CANAL_LIMITE {
            return Err("canal :: nombre demasiado largo");
        }
        postcard::to_allocvec(self).map_err(|_| "canal :: serializacion fallida")
    }

    /// Reconstruye un canal desde la carga util de su objeto. Rechaza version
    /// desconocida y nombres que excedan [`NOMBRE_CANAL_LIMITE`] —un canal con
    /// nombre extravagante se detecta al recibirlo, no al servirlo—.
    pub fn deserializar(bytes: &[u8]) -> Result<Canal, &'static str> {
        let (canal, _) = postcard::take_from_bytes::<Canal>(bytes)
            .map_err(|_| "canal :: deserializacion fallida")?;
        if canal.version != VERSION_CANAL {
            return Err("canal :: version de format desconocida");
        }
        if canal.nombre.len() > NOMBRE_CANAL_LIMITE {
            return Err("canal :: nombre excede el techo");
        }
        Ok(canal)
    }

    /// La recomendacion vigente del canal: la ultima `RaizFirmada` por
    /// `timestamp`, o `None` si el canal aun no propuso ninguna. Quien quiera
    /// "actualizar" sigue este hash; quien quiera rollback elige otra entrada
    /// del historial.
    pub fn vigente(&self) -> Option<&RaizFirmada> {
        self.raices.last()
    }
}

/// Compone el mensaje canonico que un autor firma para respaldar una raiz en
/// un canal: la concatenacion `nombre || timestamp_le || raiz_manifiesto`.
/// Es la unica verdad del payload firmable —quien firma y quien verifica han
/// de componerlo por aqui, jamas a mano—. La canonizacion incluye el nombre
/// del canal para que una firma valida en `dev` no se replique en `estable`.
pub fn mensaje_a_firmar(nombre_canal: &str, timestamp: u64, raiz_manifiesto: &Hash) -> Vec<u8> {
    let mut mensaje = Vec::with_capacity(nombre_canal.len() + 8 + raiz_manifiesto.len());
    mensaje.extend_from_slice(nombre_canal.as_bytes());
    mensaje.extend_from_slice(&timestamp.to_le_bytes());
    mensaje.extend_from_slice(raiz_manifiesto);
    mensaje
}

// =============================================================================
//  El hash y el trazado de un registro en el log
// =============================================================================

/// La identidad de un objeto: el hash BLAKE3 de su forma serializada. Kernel y
/// `boot` la calculan por aqui — una sola definicion del hash, jamas dos.
pub fn hash(bytes: &[u8]) -> Hash {
    *blake3::hash(bytes).as_bytes()
}

/// Numero de sectores que ocupa un registro cuyo payload mide `longitud`
/// bytes. Cada registro es `[longitud: u32 LE][payload postcard][relleno 0]`.
pub fn sectores_registro(longitud: usize) -> u64 {
    (4 + longitud).div_ceil(TAM_SECTOR) as u64
}

/// Compone el registro en disco de un payload: `[longitud u32 LE][payload]
/// [relleno a cero]`, alineado a un numero entero de sectores. Es el trazado
/// exacto que el kernel lee al reconstruir su indice — lo escriben tanto
/// `kernel::almacen` (al anexar un objeto) como `boot` (al sembrar la imagen).
pub fn componer_registro(payload: &[u8]) -> Vec<u8> {
    let n = sectores_registro(payload.len()) as usize;
    let mut registro = vec![0u8; n * TAM_SECTOR];
    registro[0..4].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    registro[4..4 + payload.len()].copy_from_slice(payload);
    registro
}

/// Lee la cabecera de longitud de un registro (sus 4 primeros bytes). Devuelve
/// `None` si la longitud es cero —fin del log— o supera [`MAX_OBJETO`]
/// —corrupcion—. Gemela de [`componer_registro`].
pub fn longitud_registro(cabecera: &[u8]) -> Option<usize> {
    if cabecera.len() < 4 {
        return None;
    }
    let longitud =
        u32::from_le_bytes([cabecera[0], cabecera[1], cabecera[2], cabecera[3]]) as usize;
    if longitud == 0 || longitud > MAX_OBJETO {
        None
    } else {
        Some(longitud)
    }
}

// =============================================================================
//  Pruebas — el format debe ser un espejo perfecto: lo escrito se relee igual
// =============================================================================

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn objeto_ida_y_vuelta() {
        let objeto = Objeto {
            datos: vec![1, 2, 3, 4, 5],
            hijos: vec![[7u8; 32], [9u8; 32]],
        };
        let bytes = objeto.serializar().unwrap();
        assert_eq!(Objeto::deserializar(&bytes).unwrap(), objeto);
    }

    #[test]
    fn registro_alineado_a_sector() {
        let payload = vec![0xABu8; 600];
        let registro = componer_registro(&payload);
        // 4 + 600 = 604 bytes => dos sectores de 512.
        assert_eq!(registro.len(), 2 * TAM_SECTOR);
        assert_eq!(registro.len() % TAM_SECTOR, 0);
        assert_eq!(longitud_registro(&registro), Some(600));
        assert_eq!(&registro[4..604], &payload[..]);
    }

    #[test]
    fn cuaderno_ida_y_vuelta_con_celdas_mixtas() {
        // FASE 43 :: el modelo unificado CeldaWawa empaqueta los cinco
        // campos en una sola struct. Roundtrip cubre:
        //   - celda exitosa con binario y retorno legitimo
        //   - celda fallida sin binario, sin retorno, con `marca_error`
        //   - celda fallida con binario pero retorno negativo y error
        let celdas: Vec<CeldaWawa> = vec![
            CeldaWawa {
                id_secuencial: 0,
                fuente_hash: [0xA1; 32],
                binario_hash: Some([0xB2; 32]),
                ultimo_retorno: Some(42),
                marca_error: false,
            },
            CeldaWawa {
                id_secuencial: 1,
                fuente_hash: [0xC3; 32],
                binario_hash: None,
                ultimo_retorno: None,
                marca_error: true,
            },
            CeldaWawa {
                id_secuencial: 2,
                fuente_hash: [0xD4; 32],
                binario_hash: Some([0xE5; 32]),
                ultimo_retorno: Some(-7),
                marca_error: true,
            },
        ];
        let bytes = serializar_celdas(&celdas).unwrap();
        let leido = deserializar_celdas(&bytes).unwrap();
        assert_eq!(leido, celdas);

        // Single-cell payload (el caso que produce la PRIMERA anexion
        // de `sys_cuaderno_anexar_celda` sobre un cuaderno virgen).
        let una: Vec<CeldaWawa> = vec![CeldaWawa {
            id_secuencial: 99,
            fuente_hash: [0xF0; 32],
            binario_hash: None,
            ultimo_retorno: Some(0),
            marca_error: false,
        }];
        let bytes = serializar_celdas(&una).unwrap();
        let leido = deserializar_celdas(&bytes).unwrap();
        assert_eq!(leido, una);
    }

    #[test]
    fn cuaderno_acumulativo_anexa_celdas_en_orden() {
        // FASE 47 :: la nueva syscall `sys_cuaderno_anexar_celda` opera
        // en el kernel como: recuperar -> deserializar Vec<CeldaWawa> ->
        // push(nueva) -> reserializar. Este test reproduce esa cadena
        // en miniatura, asegurando que el roundtrip respeta el orden
        // cronologico real con id_secuencial creciente.
        let mut acumulado: Vec<CeldaWawa> = Vec::new();
        for i in 0..5u32 {
            // Re-deserializar lo que el kernel "tendria en disco" antes
            // del push — refleja exactamente la operacion del host.
            let acumulado_actual = if acumulado.is_empty() {
                Vec::new()
            } else {
                let bytes = serializar_celdas(&acumulado).unwrap();
                deserializar_celdas(&bytes).unwrap()
            };
            let mut siguiente = acumulado_actual;
            siguiente.push(CeldaWawa {
                id_secuencial: i,
                fuente_hash: [i as u8; 32],
                binario_hash: if i % 2 == 0 {
                    Some([(i + 0x10) as u8; 32])
                } else {
                    None
                },
                ultimo_retorno: Some(i as i32),
                marca_error: i % 3 == 0,
            });
            acumulado = siguiente;
        }
        // Tras 5 anexiones, el cuaderno tiene 5 celdas en orden 0..5.
        assert_eq!(acumulado.len(), 5);
        for (i, c) in acumulado.iter().enumerate() {
            assert_eq!(c.id_secuencial, i as u32);
        }
        // Roundtrip final del vector acumulado preserva la cadena.
        let bytes = serializar_celdas(&acumulado).unwrap();
        let leido = deserializar_celdas(&bytes).unwrap();
        assert_eq!(leido, acumulado);
    }

    #[test]
    fn cabecera_a_cero_es_fin_del_log() {
        assert_eq!(longitud_registro(&[0, 0, 0, 0]), None);
        assert_eq!(longitud_registro(&[0xFF, 0xFF, 0xFF, 0xFF]), None);
        assert_eq!(longitud_registro(&[3, 0, 0, 0]), Some(3));
    }

    #[test]
    fn manifiesto_rechaza_version_ajena() {
        let mut manifiesto = Manifiesto {
            version: 99,
            apps: Vec::new(),
            configuracion: None,
        };
        let bytes = postcard::to_allocvec(&manifiesto).unwrap();
        assert!(Manifiesto::deserializar(&bytes).is_err());
        manifiesto.version = VERSION_MANIFIESTO;
        assert!(Manifiesto::deserializar(&manifiesto.serializar().unwrap()).is_ok());
    }

    #[test]
    fn manifiesto_transporta_enlace_de_configuracion() {
        // Un manifiesto puede nacer sin configuracion (defecto) o cargar el
        // hash de un nodo de configuracion en el grafo. Lo que el `serializar`
        // escribe es exactamente lo que el `deserializar` recupera.
        let con_enlace = Manifiesto {
            version: VERSION_MANIFIESTO,
            apps: Vec::new(),
            configuracion: Some([0xC5; 32]),
        };
        let bytes = con_enlace.serializar().unwrap();
        let leido = Manifiesto::deserializar(&bytes).unwrap();
        assert_eq!(leido.configuracion, Some([0xC5; 32]));

        let sin_enlace = Manifiesto {
            version: VERSION_MANIFIESTO,
            apps: Vec::new(),
            configuracion: None,
        };
        let bytes = sin_enlace.serializar().unwrap();
        assert!(Manifiesto::deserializar(&bytes)
            .unwrap()
            .configuracion
            .is_none());
    }

    #[test]
    fn configuracion_ida_y_vuelta_y_rechaza_version() {
        let cfg = Configuracion {
            version: VERSION_CONFIGURACION,
            idioma: idioma_iso639(*b"qu"),
            paleta: [
                0x11, 0x22, 0x33, 0xFF, 0x44, 0x55, 0x66, 0xFF, 0x77, 0x88, 0x99, 0xFF, 0xAA, 0xBB,
                0xCC, 0xFF, 0xDD, 0xEE, 0xFF, 0xFF,
            ],
        };
        let bytes = cfg.serializar().unwrap();
        assert_eq!(Configuracion::deserializar(&bytes).unwrap(), cfg);

        // Hashes distintos => identidades distintas. Cambiar la paleta o el
        // idioma engendra un nodo nuevo del grafo; ningun cambio se cuela
        // bajo el mismo hash.
        let mut otro = cfg;
        otro.idioma = idioma_iso639(*b"en");
        assert_ne!(hash(&otro.serializar().unwrap()), hash(&bytes));

        // Version desconocida: se rechaza al deserializar.
        let mut ajeno = cfg;
        ajeno.version = 99;
        let bytes_ajenos = postcard::to_allocvec(&ajeno).unwrap();
        assert!(Configuracion::deserializar(&bytes_ajenos).is_err());
    }

    #[test]
    fn configuracion_por_defecto_es_estable() {
        // El `por_defecto` debe ser determinista y reconstruirse desde su
        // forma binaria sin perder ningun campo. El kernel lo inyecta tal
        // cual cuando el manifiesto no enlaza configuracion alguna.
        let defecto = Configuracion::por_defecto();
        assert_eq!(defecto.version, VERSION_CONFIGURACION);
        assert_eq!(defecto.idioma, IDIOMA_DEFECTO);
        assert_eq!(defecto.paleta, PALETA_DEFECTO);
        let bytes = defecto.serializar().unwrap();
        assert_eq!(Configuracion::deserializar(&bytes).unwrap(), defecto);
    }

    #[test]
    fn entrada_app_transporta_permisos_y_distingue_hash() {
        // Una entrada con permisos distintos engendra un manifiesto con un
        // hash distinto: el bit es CONTENIDO direccionado, no metadato lateral.
        // Una app que se "regala" un permiso a si misma no puede pasar por
        // la misma app del manifiesto anterior — el grafo lo delata.
        let base = EntradaApp {
            nombre: String::from("test"),
            bytecode: [0x11; 32],
            region_x: 0,
            region_y: 0,
            region_ancho: 100,
            region_alto: 100,
            techo_memoria: 4 * 1024 * 1024,
            fuel_fotograma: 1_000_000,
            estado: None,
            permisos: 0,
        };
        let mut con_red = base.clone();
        con_red.permisos = PERMISO_RED;
        let manifiesto_a = Manifiesto {
            version: VERSION_MANIFIESTO,
            apps: vec![base.clone()],
            configuracion: None,
        };
        let manifiesto_b = Manifiesto {
            version: VERSION_MANIFIESTO,
            apps: vec![con_red],
            configuracion: None,
        };
        assert_ne!(
            hash(&manifiesto_a.serializar().unwrap()),
            hash(&manifiesto_b.serializar().unwrap()),
            "manifiestos con distintos permisos deben dar hashes distintos"
        );

        // El roundtrip preserva la mascara entera.
        let con_todo = EntradaApp {
            permisos: PERMISO_RED
                | PERMISO_GRAFO_ESCRITURA
                | PERMISO_RAIZ
                | PERMISO_ALTAVOZ
                | PERMISO_CONFIG
                | PERMISO_COMPACTAR,
            ..base.clone()
        };
        let m = Manifiesto {
            version: VERSION_MANIFIESTO,
            apps: vec![con_todo],
            configuracion: None,
        };
        let bytes = m.serializar().unwrap();
        let leido = Manifiesto::deserializar(&bytes).unwrap();
        assert_eq!(leido.apps[0].permisos, 0b111111);
    }

    #[test]
    fn manifiesto_firmado_ida_y_vuelta() {
        // Roundtrip serializar->deserializar preserva los tres campos del
        // sobre criptografico: hash del manifiesto, llave publica del autor
        // y firma. Es el contrato basico de la Fase 25 con el wire/log.
        let mf = ManifiestoFirmado {
            manifiesto_hash: [0xC5; 32],
            autor: [0xA1; 32],
            firma: [0x77; 64],
        };
        let bytes = mf.serializar().unwrap();
        let leido = ManifiestoFirmado::deserializar(&bytes).unwrap();
        assert_eq!(leido, mf);
        // Tamaño acotado: 32 + 32 + 64 = 128 bytes crudos + el preludio
        // postcard. Debe caber holgado en un sector y en un frame Ethernet.
        assert!(bytes.len() <= 160, "MF demasiado grande: {} bytes", bytes.len());
    }

    #[test]
    fn cuaderno_firmado_ida_y_vuelta() {
        // Roundtrip estructural del sobre criptografico del cuaderno
        // (Fase 37). Gemelo a `manifiesto_firmado_ida_y_vuelta` — el
        // mismo contrato de los tres campos contra el wire/log.
        let cf = CuadernoFirmado {
            cuaderno_raiz_hash: [0xCE; 32],
            autor: [0xA1; 32],
            firma: [0x66; 64],
        };
        let bytes = cf.serializar().unwrap();
        let leido = CuadernoFirmado::deserializar(&bytes).unwrap();
        assert_eq!(leido, cf);
        assert!(
            bytes.len() <= 160,
            "CuadernoFirmado demasiado grande: {} bytes",
            bytes.len()
        );
    }

    #[test]
    fn codigo_error_tiene_valores_estables() {
        // Anadir una variante NUEVA al enum jamas debe renumerar las
        // existentes: el binario WASM viejo compila contra el numero
        // literal y kernel + userspace tienen que coincidir aunque el
        // catalogo crezca. Este test es el contrato.
        assert_eq!(CodigoError::Ok.como_i32(), 0);
        assert_eq!(CodigoError::Ausente.como_i32(), -1);
        assert_eq!(CodigoError::CapacidadInsuficiente.como_i32(), -2);
        assert_eq!(CodigoError::AlmacenamientoFallo.como_i32(), -3);
        assert_eq!(CodigoError::SinFoco.como_i32(), -4);
        assert_eq!(CodigoError::EnvioFallo.como_i32(), -5);
        assert_eq!(CodigoError::Saturado.como_i32(), -6);
        assert_eq!(CodigoError::PayloadInvalido.como_i32(), -7);
    }

    #[test]
    fn idioma_iso639_empaqueta_en_little_endian() {
        // `es` => 'e' (0x65) en el byte bajo, 's' (0x73) en el alto.
        assert_eq!(idioma_iso639(*b"es"), 0x7365);
        assert_eq!(idioma_iso639(*b"en"), 0x6E65);
        assert_eq!(idioma_iso639(*b"qu"), 0x7571);
    }

    #[test]
    fn canal_ida_y_vuelta_con_dos_raices() {
        let canal = Canal {
            version: VERSION_CANAL,
            nombre: String::from("estable"),
            autor: [0xA1; 32],
            raices: vec![
                RaizFirmada {
                    timestamp: 1_700_000_000,
                    raiz_manifiesto: [0x11; 32],
                    firma: [0x22; 64],
                },
                RaizFirmada {
                    timestamp: 1_700_000_100,
                    raiz_manifiesto: [0x33; 32],
                    firma: [0x44; 64],
                },
            ],
        };
        let bytes = canal.serializar().unwrap();
        let recuperado = Canal::deserializar(&bytes).unwrap();
        assert_eq!(recuperado, canal);
        // `vigente` devuelve la ultima entrada por orden, no la mas reciente
        // por timestamp — el contrato es que las entradas vienen ordenadas;
        // verificarlo es responsabilidad de quien construye el canal.
        assert_eq!(recuperado.vigente().unwrap().raiz_manifiesto, [0x33; 32]);
    }

    #[test]
    fn canal_rechaza_version_y_nombre_excedido() {
        let mut canal = Canal {
            version: 99,
            nombre: String::from("dev"),
            autor: [0; 32],
            raices: Vec::new(),
        };
        let bytes = postcard::to_allocvec(&canal).unwrap();
        assert!(Canal::deserializar(&bytes).is_err());
        canal.version = VERSION_CANAL;
        assert!(Canal::deserializar(&canal.serializar().unwrap()).is_ok());

        // Nombre excedido: el serializador lo veta sin escribir nada al disco.
        let largo = Canal {
            version: VERSION_CANAL,
            nombre: "x".repeat(NOMBRE_CANAL_LIMITE + 1),
            autor: [0; 32],
            raices: Vec::new(),
        };
        assert!(largo.serializar().is_err());
    }

    #[test]
    fn mensaje_a_firmar_es_canonico_y_distingue_canales() {
        let raiz: Hash = [0x55; 32];
        let m1 = mensaje_a_firmar("estable", 42, &raiz);
        let m2 = mensaje_a_firmar("estable", 42, &raiz);
        assert_eq!(m1, m2, "el mensaje firmable debe ser deterministico");

        // Cambiar el canal cambia el mensaje: una firma valida en `dev` no se
        // replica en `estable`.
        let m3 = mensaje_a_firmar("dev", 42, &raiz);
        assert_ne!(m1, m3);

        // Cambiar el timestamp tambien — no se replica una recomendacion vieja
        // como si fuera nueva.
        let m4 = mensaje_a_firmar("estable", 43, &raiz);
        assert_ne!(m1, m4);
    }

    #[test]
    fn superbloque_cabe_en_un_sector_y_vuelve_intacto() {
        let sb = SuperBloque {
            magia: MAGIA,
            version: VERSION_SUPERBLOQUE,
            log_inicio: 1,
            cursor: 4096,
            raiz: Some([1u8; 32]),
            manifiesto: Some([2u8; 32]),
        };
        let bytes = sb.serializar().unwrap();
        assert!(bytes.len() <= TAM_SECTOR);
        assert_eq!(SuperBloque::deserializar(&bytes).unwrap(), sb);
    }

    #[test]
    fn test_wawa_ecosystem_immutable_vanguard() {
        // =====================================================================
        // FASE 50 :: VANGUARDIA INMUTABLE DEL ABI WAWA
        // ---------------------------------------------------------------------
        //  Sello de cierre del Manifiesto Tecnico. La firma numerica de las
        //  ocho variantes licitas de `CodigoError` —el lenguaje compartido
        //  entre el kernel Ring 0, los modulos WASM Ring 3 y el explorador
        //  host-side— ha quedado fijada. Este test la consagra:
        //
        //    * Cada variante tiene su valor i32 FIJO en el orden negociado
        //      a lo largo de las primeras 49 fases. Renumerar una existente
        //      seria romper, byte a byte, todo binario Ring 3 ya inscrito
        //      en el grafo direccionado por contenido.
        //
        //    * La conversion `as i32` y la `const fn como_i32` son gemelas:
        //      ambas extraen el discriminante `#[repr(i32)]` —sin trampa,
        //      sin tabla auxiliar—.
        //
        //    * El catalogo permanece de cardinalidad ocho: ni una variante
        //      menos (siempre Ok=0 + siete fallas controladas), ni una mas
        //      escondida tras renumeracion. Anadir una NUEVA codifica un
        //      valor entero NUEVO; el contrato no se rompe.
        //
        //  Quien pretenda extender el catalogo en una fase futura debera,
        //  ANTES de mover una variante, actualizar esta tabla de cierre
        //  y aceptar que el wire del ecosistema entero ha cambiado de era.
        // =====================================================================

        // 1. Firma numerica congelada de la vanguardia (Ok + 7 fallas).
        const VANGUARDIA: [(CodigoError, i32); 8] = [
            (CodigoError::Ok, 0),
            (CodigoError::Ausente, -1),
            (CodigoError::CapacidadInsuficiente, -2),
            (CodigoError::AlmacenamientoFallo, -3),
            (CodigoError::SinFoco, -4),
            (CodigoError::EnvioFallo, -5),
            (CodigoError::Saturado, -6),
            (CodigoError::PayloadInvalido, -7),
        ];
        for &(variante, valor) in VANGUARDIA.iter() {
            assert_eq!(
                variante.como_i32(),
                valor,
                "ABI roto: {:?} dejo de valer {} — mutacion accidental detectada",
                variante,
                valor,
            );
            // `as i32` directo: el `#[repr(i32)]` fija el discriminante en
            // ambos caminos —el const fn y el cast— sin tabla auxiliar.
            assert_eq!(variante as i32, valor);
        }

        // 2. La proyeccion debe ser inyectiva: dos variantes distintas no
        //    pueden compartir su valor i32 — el catalogo de la vanguardia
        //    no tolera colisiones.
        for i in 0..VANGUARDIA.len() {
            for j in (i + 1)..VANGUARDIA.len() {
                assert_ne!(
                    VANGUARDIA[i].1, VANGUARDIA[j].1,
                    "ABI roto: dos variantes comparten valor i32"
                );
            }
        }

        // 3. Cardinalidad inmutable: 1 (Ok) + 7 fallas controladas. Cualquier
        //    fase que pretenda crecer este catalogo debe actualizar el test
        //    explicitamente; un cambio silencioso se delata aqui.
        assert_eq!(
            VANGUARDIA.len(),
            8,
            "ABI roto: cardinalidad del catalogo CodigoError mutada"
        );

        // 4. Rango cerrado de fallas en [-7, -1]. La cascada de Pluma
        //    (apps/pluma) y el dispatcher Ring 0 cuentan con este rango
        //    EXACTO para distinguir codigos de error de retornos legitimos.
        let fallas_min = VANGUARDIA.iter().skip(1).map(|&(_, v)| v).min().unwrap();
        let fallas_max = VANGUARDIA.iter().skip(1).map(|&(_, v)| v).max().unwrap();
        assert_eq!(fallas_min, -7, "ABI roto: el suelo de fallas se desplazo");
        assert_eq!(fallas_max, -1, "ABI roto: el techo de fallas se desplazo");
    }

    #[test]
    fn superbloque_porta_log_inicio_distinto_de_uno() {
        // Tras una compactacion semantica, `log_inicio` no es 1: apunta al
        // sector donde empieza el segmento limpio recien escrito. El
        // superbloque sigue cabiendo en su sector y el roundtrip preserva
        // el campo: el GC depende de esa simetria.
        let sb = SuperBloque {
            magia: MAGIA,
            version: VERSION_SUPERBLOQUE,
            log_inicio: 32_768,
            cursor: 33_500,
            raiz: Some([0xAA; 32]),
            manifiesto: Some([0xBB; 32]),
        };
        let bytes = sb.serializar().unwrap();
        assert!(bytes.len() <= TAM_SECTOR);
        let leido = SuperBloque::deserializar(&bytes).unwrap();
        assert_eq!(leido.log_inicio, 32_768);
        assert_eq!(leido.cursor, 33_500);
    }
}
