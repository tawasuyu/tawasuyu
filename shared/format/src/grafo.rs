use super::*;

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

// =============================================================================
//  Fase 66 :: Árbol/Blob — el monorepo como grafo
// -----------------------------------------------------------------------------
//  El grafo direccionado por contenido ES el modelo de objetos de git. Esta
//  capa lo hace explícito para que un árbol de directorios viva en el grafo:
//
//    * BLOB      :: el contenido de un archivo. Es un `Objeto { datos: bytes,
//                   hijos: [] }` — sin estructura, solo bytes direccionados por
//                   su hash. Archivos idénticos comparten un solo blob (dedup
//                   por contenido, gratis).
//    * ÁRBOL     :: el contenido de un directorio. Un `Objeto` cuyo `datos` es
//                   un `Arbol` postcard (la lista de entradas: nombre + modo +
//                   hash) y cuyos `hijos` son los hashes de esas entradas — así
//                   el MARK del GC del kernel alcanza todo el subárbol siguiendo
//                   `hijos`, SIN tener que entender el format `Arbol`.
//
//  Las entradas de un árbol van ORDENADAS por nombre: mismo contenido de
//  directorio => mismo árbol serializado => mismo hash. Determinismo total, la
//  base de la dedup y de la verificación. Un repositorio entero colapsa a UN
//  hash raíz; dos commits que solo tocan un archivo comparten todo el resto del
//  árbol (estructura compartida, como git).
// =============================================================================

/// Version del format de un `Arbol`.
pub const VERSION_ARBOL: u32 = 1;

/// Qué clase de objeto referencia una entrada de árbol. Espeja los modos de
/// git (archivo / archivo+x / symlink / directorio). Variantes AÑADIDAS AL
/// FINAL: los tags `postcard` se asignan por orden y mover una romperia árboles
/// ya serializados.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModoEntrada {
    /// El hash apunta a un archivo regular (blob plano o índice de trozos).
    Archivo,
    /// El hash apunta a otro ÁRBOL (subdirectorio).
    Directorio,
    /// Como `Archivo` pero con bit de ejecución (un script, un binario).
    Ejecutable,
    /// El hash apunta a un blob cuyo contenido es el DESTINO del enlace
    /// simbólico (la ruta a la que apunta), en UTF-8.
    Symlink,
}

impl ModoEntrada {
    /// `true` si el modo referencia CONTENIDO de archivo (blob/índice): un
    /// archivo regular o un ejecutable. `Symlink` y `Directorio` no.
    pub fn es_archivo(&self) -> bool {
        matches!(self, ModoEntrada::Archivo | ModoEntrada::Ejecutable)
    }
}

/// Una entrada de un árbol: un nombre dentro del directorio + el modo + el hash
/// del objeto que la realiza (un blob si `Archivo`, un árbol si `Directorio`).
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct EntradaArbol {
    /// Nombre del archivo/subdirectorio (sin separadores de ruta).
    pub nombre: String,
    /// Si el hash apunta a un blob o a un subárbol.
    pub modo: ModoEntrada,
    /// Hash del objeto (blob o árbol) que esta entrada referencia.
    pub hash: Hash,
}

/// Un árbol: el contenido de un directorio, como lista ordenada de entradas.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Arbol {
    /// Version del format — debe ser [`VERSION_ARBOL`].
    pub version: u32,
    /// Entradas ORDENADAS por nombre (invariante que `objeto_arbol` impone).
    pub entradas: Vec<EntradaArbol>,
}

impl Arbol {
    /// Serializa el árbol a su forma `postcard` —la carga útil del objeto que
    /// lo aloja—.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self).map_err(|_| "arbol :: serializacion fallida")
    }

    /// Reconstruye un árbol desde la carga útil de su objeto. Rechaza una
    /// version desconocida en lugar de malinterpretarla.
    pub fn deserializar(bytes: &[u8]) -> Result<Arbol, &'static str> {
        let (arbol, _) =
            postcard::take_from_bytes::<Arbol>(bytes).map_err(|_| "arbol :: deserializacion fallida")?;
        if arbol.version != VERSION_ARBOL {
            return Err("arbol :: version de format desconocida");
        }
        Ok(arbol)
    }
}

/// Construye el objeto BLOB de un archivo: bytes crudos, sin hijos. El hash de
/// este objeto (sobre su forma serializada) es la identidad del archivo en el
/// grafo. Dos archivos con idéntico contenido producen el MISMO blob.
pub fn objeto_blob(datos: Vec<u8>) -> Objeto {
    Objeto {
        datos,
        hijos: Vec::new(),
    }
}

/// Construye el objeto ÍNDICE de un archivo GRANDE partido en trozos: `datos`
/// VACÍO, `hijos` = los hashes de los blobs-trozo EN ORDEN. La convención de
/// lectura: una entrada de archivo (`Archivo`/`Ejecutable`) cuyo objeto tiene
/// `hijos` no vacío es un índice, y el contenido del archivo es la
/// concatenación de los `datos` de sus trozos; si `hijos` está vacío, el
/// objeto ES el contenido (blob plano). Así un archivo de cualquier tamaño se
/// referencia igual desde el árbol — el lector decide plano vs índice por la
/// forma del objeto, sin un modo aparte. Un archivo vacío es un blob plano
/// (`datos` vacío, `hijos` vacío), nunca un índice.
pub fn objeto_blob_indice(hijos: Vec<Hash>) -> Objeto {
    Objeto {
        datos: Vec::new(),
        hijos,
    }
}

/// Construye el objeto ÁRBOL de un directorio a partir de sus entradas. ORDENA
/// las entradas por nombre (determinismo: mismo directorio → mismo hash) y fija
/// `hijos` con los hashes de las entradas, en el MISMO orden, para que el GC
/// alcance el subárbol siguiendo `hijos` sin parsear el `Arbol`.
pub fn objeto_arbol(mut entradas: Vec<EntradaArbol>) -> Result<Objeto, &'static str> {
    entradas.sort_by(|a, b| a.nombre.cmp(&b.nombre));
    let hijos: Vec<Hash> = entradas.iter().map(|e| e.hash).collect();
    let arbol = Arbol {
        version: VERSION_ARBOL,
        entradas,
    };
    let datos = arbol.serializar()?;
    Ok(Objeto { datos, hijos })
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

/// Compone el mensaje canonico que un autor firma para CONCEDER capacidad a un
/// bytecode: `bytecode(32) || permisos_le(4)`. Es la unica verdad del payload
/// firmable de una [`ConcesionCapacidad`] —firmante y verificador lo componen
/// por aqui, jamas a mano—. Liga la firma al hash EXACTO del binario y al
/// bitfield EXACTO: una concesion para el bytecode X no vale para Y, y subir un
/// bit de permiso invalida la firma. Devuelve un arreglo de pila de 36 bytes:
/// zero-alloc, apto para el camino Ring 0 del kernel.
pub fn mensaje_capacidad(bytecode: &Hash, permisos: Permisos) -> [u8; 36] {
    let mut m = [0u8; 36];
    m[..32].copy_from_slice(bytecode);
    m[32..].copy_from_slice(&permisos.to_le_bytes());
    m
}

/// Dominio de separacion del mensaje de ROTACION de clave. Un byte canonico de
/// rotacion jamas colisiona con uno de revocacion ni con un claim del grafo.
pub const DOM_ROTACION_CLAVE: &[u8] = b"agora-key-rotation\x01";

/// Dominio de separacion del mensaje de REVOCACION de clave.
pub const DOM_REVOCACION_CLAVE: &[u8] = b"agora-revocation\x01";

/// Compone el mensaje canonico de una ROTACION de clave (handoff voluntario
/// vieja->nueva): `DOM || old(32) || new(32) || issued_at_le(8)`. Tamanos fijos,
/// sin prefijos de largo; el dominio lo separa de otros records. Es la unica
/// verdad del payload firmable de una rotacion — `agora-core::KeyRotation` lo
/// compone por aqui y el kernel lo espeja sobre estos mismos bytes (ver
/// `agora/SDD-rotacion-revocacion.md` §2.1).
pub fn mensaje_rotacion_clave(old_key: &[u8; 32], new_key: &[u8; 32], issued_at: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(DOM_ROTACION_CLAVE.len() + 72);
    out.extend_from_slice(DOM_ROTACION_CLAVE);
    out.extend_from_slice(old_key);
    out.extend_from_slice(new_key);
    out.extend_from_slice(&issued_at.to_le_bytes());
    out
}

/// Compone el mensaje canonico de una REVOCACION de clave:
/// `DOM || target(32) || [motivo] || issued_at_le(8) || tag || [expires_le(8)]`,
/// donde `tag` es `0` si `expires_at` es `None` y `1` si es `Some` (para que
/// `None` y `Some(0)` no colisionen). El `motivo` es el discriminante estable de
/// `agora-core::RevReason` (0=Compromised, 1=Retired, 2=Superseded) — entra en la
/// firma para que no se pueda "ascender" un retiro a compromiso sin re-firmar.
/// Unica verdad del payload firmable de una revocacion: `agora-core::Revocation`
/// lo compone por aqui y el kernel lo espeja en `claves::verificar_revocacion`
/// (ver `agora/SDD-rotacion-revocacion.md` §2.2 y §4).
pub fn mensaje_revocacion_clave(
    target_key: &[u8; 32],
    motivo: u8,
    issued_at: u64,
    expires_at: Option<u64>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(DOM_REVOCACION_CLAVE.len() + 50);
    out.extend_from_slice(DOM_REVOCACION_CLAVE);
    out.extend_from_slice(target_key);
    out.push(motivo);
    out.extend_from_slice(&issued_at.to_le_bytes());
    match expires_at {
        None => out.push(0),
        Some(t) => {
            out.push(1);
            out.extend_from_slice(&t.to_le_bytes());
        }
    }
    out
}

/// Permisos EFECTIVOS de una app: la INTERSECCION de lo que su `EntradaApp` del
/// manifiesto DECLARA y lo que una [`ConcesionCapacidad`] valida CONCEDE para su
/// bytecode. El manifiesto no puede escalar un binario mas alla de su concesion
/// firmada, y una concesion generosa no enciende permisos que el manifiesto no
/// pidio. Sin concesion valida, el llamante pasa `0` como `concedidos` y la app
/// corre sin capacidades gateadas (la matriz pasiva siempre esta). Es la regla
/// que el kernel aplica en el punto de carga —ver `SDD-capacidades.md`—.
pub const fn permisos_efectivos(declarados: Permisos, concedidos: Permisos) -> Permisos {
    declarados & concedidos
}

