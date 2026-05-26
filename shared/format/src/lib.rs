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

// =============================================================================
//  Constantes del format en disco
// =============================================================================

/// Firma magica del superbloque — «RENASer GRaFo». Distingue un disco de
/// renaser de uno virgen o ajeno.
pub const MAGIA: [u8; 8] = *b"RENASGRF";

/// Version del format del superbloque en disco. Un disco con otra version se
/// reformatea al arrancar. v2 (Fase 7) — el superbloque porta el ancla
/// `manifiesto`, gemela de `raiz`.
pub const VERSION_SUPERBLOQUE: u32 = 2;

/// Version del format del manifiesto serializado. Independiente de la del
/// superbloque: el manifiesto es un objeto del grafo, no una estructura fija
/// del disco. v2 — cada `EntradaApp` declara su propio `fuel_fotograma`
/// (presupuesto cooperativo por `tick`); el kernel ya no impone un techo unico.
pub const VERSION_MANIFIESTO: u32 = 2;

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

/// El superbloque: el sector 0 del disco. Ancla el grafo entero — dice por
/// donde continua el log, cual es el objeto raiz y cual el manifiesto.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct SuperBloque {
    /// Firma magica: debe ser [`MAGIA`].
    pub magia: [u8; 8],
    /// Version del format: debe ser [`VERSION_SUPERBLOQUE`].
    pub version: u32,
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
}

/// Una entrada del manifiesto: una aplicacion del userspace y todo lo que el
/// kernel necesita para darle vida — su bytecode, su ventana, su cuota de
/// memoria y, si lo tuviera, su ultimo estado persistido.
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
        };
        let bytes = postcard::to_allocvec(&manifiesto).unwrap();
        assert!(Manifiesto::deserializar(&bytes).is_err());
        manifiesto.version = VERSION_MANIFIESTO;
        assert!(Manifiesto::deserializar(&manifiesto.serializar().unwrap()).is_ok());
    }

    #[test]
    fn superbloque_cabe_en_un_sector_y_vuelve_intacto() {
        let sb = SuperBloque {
            magia: MAGIA,
            version: VERSION_SUPERBLOQUE,
            cursor: 4096,
            raiz: Some([1u8; 32]),
            manifiesto: Some([2u8; 32]),
        };
        let bytes = sb.serializar().unwrap();
        assert!(bytes.len() <= TAM_SECTOR);
        assert_eq!(SuperBloque::deserializar(&bytes).unwrap(), sb);
    }
}
