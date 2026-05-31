use super::*;

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
    /// Hash del nodo [`OverlayRevocacion`] vigente, o `None` si el operador no
    /// ancló ninguno (el caso común — sin revocaciones de claves del anillo).
    /// El kernel lo lee FRESH en el arranque y deniega en `autor_en_anillo` toda
    /// clave del anillo revocada M-of-N: así una clave soberana filtrada se
    /// apaga ENTRE reflasheos, sin esperar al re-forjado del binario. Es la pieza
    /// del plano de CONTROL del SDD-rotacion-revocacion §4 — gemela de
    /// `configuracion`/`estado`: reanclar engendra un overlay nuevo y mueve el
    /// puntero del manifiesto, jamás muta en sitio.
    pub overlay_revocacion: Option<Hash>,
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

/// Permite llamar al motor `tinkuy` embebido en el kernel: una sub-jaula
/// `wasmi` aparte, con su propio Store y su propio fuel — la que carga
/// `assets/tinkuy.wasm` y expone los `tk_*`. La capa de capacidades
/// `sys_tinkuy_*` enlaza solo si el bit esta puesto. El motor tinkuy es
/// computo puro (sin red, sin grafo, sin altavoz): el bit lo SE PARA del
/// resto de capacidades, no porque sea privilegiado, sino porque tiene
/// memoria persistente entre `tick`s — una app que lo tenga puede
/// secuestrar slots de simulacion entre fotogramas y conviene que el
/// operador lo declare a sabiendas.
pub const PERMISO_TINKUY: Permisos = 1 << 6;

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
    ///
    /// Estos son los permisos DECLARADOS. Los EFECTIVOS (lo que el kernel
    /// enlaza de verdad) salen de [`permisos_efectivos`]`(permisos, concedidos)`
    /// donde `concedidos` viene de la [`ConcesionCapacidad`] referida por
    /// [`concesion`](Self::concesion). El manifiesto puede pedir menos, nunca mas.
    pub permisos: Permisos,
    /// Fase 67 / WAWA §14.1.3 — hash de la [`ConcesionCapacidad`] que firma el
    /// par `(bytecode, permisos)` de esta app, o `None`. La concesion vive como
    /// un objeto del grafo (direccionado por contenido); el kernel la recupera,
    /// verifica su firma contra el `AGORA_AUTH_RING` y toma la interseccion de
    /// sus permisos con los declarados aqui. `None` ⇒ sin techo per-bytecode:
    /// el kernel honra `permisos` tal cual (la integridad la da la firma del
    /// manifiesto). El binding "que binario puede que" queda asi INDEPENDIENTE
    /// del manifiesto: re-firmar un manifiesto no escala un binario por encima
    /// de su concesion.
    pub concesion: Option<Hash>,
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

