// =============================================================================
//  renaser :: kernel/src/almacen.rs — Fase 6.1c :: el grafo de objetos
// -----------------------------------------------------------------------------
//  renaser rompe con POSIX tambien en el almacenamiento: aqui no hay un sistema
//  de archivos plano —rutas, directorios, inodos—. Hay un GRAFO DIRIGIDO ACICLICO
//  de objetos DIRECCIONADOS POR CONTENIDO.
//
//  Un objeto es una carga util de bytes y una lista de aristas hacia otros
//  objetos. Su IDENTIDAD no es un nombre ni un numero: es el hash BLAKE3 de su
//  forma serializada. De ello se siguen dos propiedades que un FS jamas regala:
//
//    * INTEGRIDAD — el hash verifica el contenido; un objeto corrupto se delata.
//    * DEDUPLICACION — contenido identico produce hash identico; se almacena
//      una sola vez, aunque mil aristas apunten a el.
//
//  El disco se organiza como un LOG: el sector 0 es el superbloque —el ancla
//  del grafo—, y tras el se anexan los registros de objetos, uno tras otro. Un
//  indice en memoria (hash -> sector) se reconstruye al arrancar recorriendo el
//  log.
//
//  El FORMATO en disco —los tipos `Objeto`/`SuperBloque`, su (de)serializacion
//  `postcard`, el hash BLAKE3 y el trazado de cada registro— ya no vive aqui:
//  habita la crate `format` (Fase 7b), un nucleo `no_std` COMPARTIDO con el
//  constructor de imagen `boot`. Este modulo es solo el almacen VIVO: el
//  cursor, el indice y la E/S contra el disco virtio-blk.
// =============================================================================

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use spin::{Mutex, Once};

use crate::drivers::disco;

// El identificador y el objeto del grafo los define `format`; se reexportan
// para que el resto del kernel siga nombrandolos `almacen::Hash` y
// `almacen::Objeto`, sin enterarse de donde viven realmente.
pub use format::{Hash, Objeto};

/// El estado vivo del almacen: el cursor del log, la raiz, el manifiesto y el
/// indice en memoria que traduce cada hash al sector donde habita su registro.
struct Almacen {
    /// Proximo sector libre del log.
    cursor: u64,
    /// El objeto raiz del DAG.
    raiz: Option<Hash>,
    /// El objeto del Manifiesto de Genesis (Fase 7).
    manifiesto: Option<Hash>,
    /// Indice hash -> sector del registro. Se reconstruye al arrancar.
    indice: BTreeMap<Hash, u64>,
    /// Capacidad del disco, en sectores.
    capacidad: u64,
}

/// El almacen global de renaser. Se funde una sola vez, en `init`.
static ALMACEN: Once<Mutex<Almacen>> = Once::new();

/// El fruto de fundar el almacen — para que el arranque deje constancia visual.
pub struct Resumen {
    /// Capacidad del disco, en sectores.
    pub capacidad: u64,
    /// Numero de objetos hallados en el grafo.
    pub objetos: usize,
    /// ¿Tiene el grafo un objeto raiz?
    pub raiz: bool,
    /// ¿Se reformateo el disco (estaba virgen o era ajeno)?
    pub formateado: bool,
}

/// Funda el almacen de objetos: monta el disco, lee el superbloque y, si el
/// disco ya es de renaser, reconstruye el indice recorriendo el log; si es
/// virgen o ajeno, lo formatea. Toda falla se devuelve como `Err`.
pub fn init() -> Result<Resumen, &'static str> {
    let capacidad = disco::montar()?;
    if capacidad < 2 {
        return Err("el disco es demasiado pequeño para un grafo");
    }

    // Leer el sector 0 e intentar interpretarlo como superbloque de renaser.
    let mut sector0 = [0u8; format::TAM_SECTOR];
    disco::leer_sectores(0, &mut sector0)?;

    let (cursor, raiz, manifiesto, indice, formateado) =
        match format::SuperBloque::deserializar(&sector0) {
            // Disco de renaser, con la version corriente: adoptar su grafo.
            Ok(sb) if sb.magia == format::MAGIA && sb.version == format::VERSION_SUPERBLOQUE => {
                let indice = reconstruir_indice(sb.cursor)?;
                (sb.cursor, sb.raiz, sb.manifiesto, indice, false)
            }
            // Disco virgen, ajeno o de otra version: empezar de cero. El log
            // arranca en el sector 1, justo despues del superbloque.
            _ => (1, None, None, BTreeMap::new(), true),
        };

    let objetos = indice.len();
    let tiene_raiz = raiz.is_some();
    let almacen = Almacen {
        cursor,
        raiz,
        manifiesto,
        indice,
        capacidad,
    };

    // Un disco recien formateado necesita su superbloque grabado de inmediato.
    if formateado {
        persistir(&almacen)?;
    }
    ALMACEN.call_once(|| Mutex::new(almacen));

    Ok(Resumen {
        capacidad,
        objetos,
        raiz: tiene_raiz,
        formateado,
    })
}

/// Recorre el log —del sector 1 al `cursor`— y reconstruye el indice
/// hash -> sector. Cada registro se rehashea: el indice se reconstruye, no se
/// confia. Un registro corrupto detiene el escaneo sin incendiar nada.
fn reconstruir_indice(cursor: u64) -> Result<BTreeMap<Hash, u64>, &'static str> {
    let mut indice = BTreeMap::new();
    let mut sector: u64 = 1;
    while sector < cursor {
        match leer_registro(sector)? {
            // Un payload valido: hashearlo e indexarlo.
            Some(payload) => {
                let n = format::sectores_registro(payload.len());
                indice.insert(format::hash(&payload), sector);
                sector += n;
            }
            // Cabecera a cero o longitud imposible: fin (o corrupcion) del log.
            None => break,
        }
    }
    Ok(indice)
}

/// Lee el registro que arranca en `sector` y devuelve su payload postcard
/// (sin la cabecera de longitud ni el relleno). `None` si la cabecera dice
/// longitud cero —fin del log— o una longitud imposible —corrupcion—.
fn leer_registro(sector: u64) -> Result<Option<Vec<u8>>, &'static str> {
    let mut cabecera = [0u8; format::TAM_SECTOR];
    disco::leer_sectores(sector, &mut cabecera)?;
    let longitud = match format::longitud_registro(&cabecera) {
        Some(longitud) => longitud,
        None => return Ok(None),
    };
    let n = format::sectores_registro(longitud) as usize;
    // Si el registro cabe en el sector ya leido, evitar una segunda lectura.
    let payload = if n == 1 {
        cabecera[4..4 + longitud].to_vec()
    } else {
        let mut buf = vec![0u8; n * format::TAM_SECTOR];
        disco::leer_sectores(sector, &mut buf)?;
        buf[4..4 + longitud].to_vec()
    };
    Ok(Some(payload))
}

/// Graba el superbloque —el ancla del grafo— en el sector 0.
fn persistir(almacen: &Almacen) -> Result<(), &'static str> {
    let sb = format::SuperBloque {
        magia: format::MAGIA,
        version: format::VERSION_SUPERBLOQUE,
        cursor: almacen.cursor,
        raiz: almacen.raiz,
        manifiesto: almacen.manifiesto,
    };
    let bytes = sb.serializar()?;
    if bytes.len() > format::TAM_SECTOR {
        return Err("el superbloque no cabe en un sector");
    }
    let mut sector0 = [0u8; format::TAM_SECTOR];
    sector0[..bytes.len()].copy_from_slice(&bytes);
    disco::escribir_sectores(0, &sector0)
}

/// Almacena un objeto y devuelve su hash. Direccionamiento por contenido en
/// estado puro: si un objeto de contenido identico ya existe, NO se reescribe —
/// se devuelve el hash que ya tenia. El grafo nunca guarda dos veces lo mismo.
pub fn almacenar(datos: Vec<u8>, hijos: Vec<Hash>) -> Result<Hash, &'static str> {
    let objeto = Objeto { datos, hijos };
    let bytes = objeto.serializar()?;
    if bytes.is_empty() || bytes.len() > format::MAX_OBJETO {
        return Err("el objeto tiene un tamaño invalido");
    }
    // La identidad del objeto: el hash de su forma serializada.
    let hash = format::hash(&bytes);

    let mutex = ALMACEN.get().ok_or("almacen no inicializado")?;
    let mut almacen = mutex.lock();

    // ¿Ya esta en el grafo? Entonces no hay nada que grabar.
    if almacen.indice.contains_key(&hash) {
        return Ok(hash);
    }

    // Reservar los sectores del registro al final del log.
    let n = format::sectores_registro(bytes.len());
    if almacen.cursor + n > almacen.capacidad {
        return Err("el grafo de objetos esta lleno");
    }
    let sector = almacen.cursor;

    // Componer el registro —[longitud][payload][relleno]— y grabarlo.
    let registro = format::componer_registro(&bytes);
    disco::escribir_sectores(sector, &registro)?;

    // El objeto ya esta en disco: avanzar el cursor, indexarlo y RE-anclar el
    // superbloque. El orden importa — el superbloque se graba el ultimo, de
    // modo que jamas apunte a un registro a medio escribir.
    almacen.cursor += n;
    almacen.indice.insert(hash, sector);
    persistir(&almacen)?;

    Ok(hash)
}

/// Recupera un objeto por su hash. `Ok(None)` si el hash no esta en el grafo.
pub fn recuperar(hash: &Hash) -> Result<Option<Objeto>, &'static str> {
    let mutex = ALMACEN.get().ok_or("almacen no inicializado")?;
    // Soltar el cerrojo del almacen ANTES de la E/S de disco —lenta, por
    // sondeo—: el indice ya entrego el sector, y nada mas reclama el cerrojo.
    let sector = match mutex.lock().indice.get(hash) {
        Some(&s) => s,
        None => return Ok(None),
    };
    let payload = leer_registro(sector)?.ok_or("registro de objeto corrupto")?;
    // Verificacion de integridad: el contenido leido DEBE rehashear al hash
    // pedido. Si no, el disco ha mentido — y se delata.
    if format::hash(&payload) != *hash {
        return Err("el objeto no supero la verificacion de integridad");
    }
    Ok(Some(Objeto::deserializar(&payload)?))
}

/// El hash del objeto raiz del grafo, si lo hay.
pub fn raiz() -> Option<Hash> {
    ALMACEN.get().and_then(|mutex| mutex.lock().raiz)
}

/// Corona un objeto como raiz del grafo y ancla el cambio en el superbloque.
pub fn fijar_raiz(hash: Hash) -> Result<(), &'static str> {
    let mutex = ALMACEN.get().ok_or("almacen no inicializado")?;
    let mut almacen = mutex.lock();
    almacen.raiz = Some(hash);
    persistir(&almacen)
}

/// El hash del objeto del Manifiesto de Genesis, si el disco tiene uno
/// anclado. Gemelo de [`raiz`], pero del lado del kernel: lo lee la Fase 7
/// para descubrir que apps poblar al arrancar.
pub fn manifiesto() -> Option<Hash> {
    ALMACEN.get().and_then(|mutex| mutex.lock().manifiesto)
}

/// Ancla un objeto como el Manifiesto de Genesis y graba el cambio en el
/// superbloque. Gemelo de [`fijar_raiz`]. La Fase 7c lo invoca cada vez que una
/// app persiste su estado y el manifiesto debe re-anclarse.
pub fn fijar_manifiesto(hash: Hash) -> Result<(), &'static str> {
    let mutex = ALMACEN.get().ok_or("almacen no inicializado")?;
    let mut almacen = mutex.lock();
    almacen.manifiesto = Some(hash);
    persistir(&almacen)
}
