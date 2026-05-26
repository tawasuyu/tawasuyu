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

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

use core::sync::atomic::{AtomicUsize, Ordering};

use spin::{Mutex, Once};

use crate::drivers::disco;

// El identificador y el objeto del grafo los define `format`; se reexportan
// para que el resto del kernel siga nombrandolos `almacen::Hash` y
// `almacen::Objeto`, sin enterarse de donde viven realmente.
pub use format::{Hash, Objeto};

/// El estado vivo del almacen: el cursor del log, la raiz, el manifiesto y el
/// indice en memoria que traduce cada hash al sector donde habita su registro.
struct Almacen {
    /// Primer sector del log activo. En un disco recien sembrado vale `1`; el
    /// compactador semantico lo desplaza al principio de un segmento limpio
    /// cada vez que aspira los nodos muertos del grafo.
    log_inicio: u64,
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

/// Contador de objetos GRABADOS al log desde la ultima compactacion. El
/// compositor lo consulta en su tic ocioso: cuando supera el umbral, lanza
/// una pasada del compactador semantico. Las deduplicaciones (cuando un
/// objeto ya existe y `almacenar` devuelve su hash sin escribir nada) NO
/// cuentan: el log no engordo, no hay basura nueva que aspirar.
static ESCRITURAS_DESDE_GC: AtomicUsize = AtomicUsize::new(0);

/// Umbral de escrituras tras el cual el compositor solicita una pasada del
/// compactador. Cada `sys_estado_guardar` engendra un nodo nuevo del estado
/// (y reescribe el manifiesto), de modo que 32 escrituras corresponde a una
/// docena larga de "guarda esto" del userspace: lo bastante para que valga
/// la pena la E/S sin estrangular el reactor con un GC casi-vacio.
const UMBRAL_GC: usize = 32;

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

    let (log_inicio, cursor, raiz, manifiesto, indice, formateado) =
        match format::SuperBloque::deserializar(&sector0) {
            // Disco de renaser, con la version corriente: adoptar su grafo.
            Ok(sb) if sb.magia == format::MAGIA && sb.version == format::VERSION_SUPERBLOQUE => {
                let indice = reconstruir_indice(sb.log_inicio, sb.cursor)?;
                (sb.log_inicio, sb.cursor, sb.raiz, sb.manifiesto, indice, false)
            }
            // Disco virgen, ajeno o de otra version: empezar de cero. El log
            // arranca en el sector 1, justo despues del superbloque.
            _ => (1, 1, None, None, BTreeMap::new(), true),
        };

    let objetos = indice.len();
    let tiene_raiz = raiz.is_some();
    let almacen = Almacen {
        log_inicio,
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

/// Recorre el log —de `inicio` al `cursor`— y reconstruye el indice
/// hash -> sector. Cada registro se rehashea: el indice se reconstruye, no se
/// confia. Un registro corrupto detiene el escaneo sin incendiar nada.
fn reconstruir_indice(inicio: u64, cursor: u64) -> Result<BTreeMap<Hash, u64>, &'static str> {
    let mut indice = BTreeMap::new();
    let mut sector: u64 = inicio;
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
        log_inicio: almacen.log_inicio,
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

    // El log gano un registro nuevo: contarlo para el compactador.
    ESCRITURAS_DESDE_GC.fetch_add(1, Ordering::Relaxed);

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

// =============================================================================
//  Compactador semantico (Fase 24) — Recolector de Basura del log inmutable
// -----------------------------------------------------------------------------
//  Cada cambio de estado del usuario engendra un objeto nuevo del grafo; los
//  objetos antiguos quedan, intactos, en sectores que ya nadie alcanza desde
//  las anclas (`raiz`, `manifiesto`). Si nadie los recoge, el log crece sin
//  techo y el disco se llena de bytes muertos.
//
//  El compactador NO desfragmenta sector por sector como un sistema de
//  archivos clasico — no le hace falta. Hace algo mas radical: parte de las
//  anclas, recorre el DAG en profundidad, copia el set ALCANZABLE a un
//  segmento limpio del disco, y reanca el superbloque al nuevo segmento. Las
//  fronteras del log se mueven; el grafo logico es el mismo. El espacio del
//  segmento viejo queda inaccesible y libre — la proxima compactacion lo
//  cubrira si hace falta.
//
//  ATOMICIDAD: la unica escritura que decide el resultado es la del
//  superbloque (sector 0). Si el kernel se cae mientras escribe la zona
//  nueva, el viejo superbloque sigue apuntando a la zona vieja: el grafo
//  esta intacto. Solo cuando el superbloque se graba con el nuevo
//  `log_inicio` la transicion es visible — y al ser una unica escritura,
//  virtio-blk la entrega entera o nada.
// =============================================================================

/// Estadisticas de una pasada del compactador. La traza serial las muestra
/// para que el operador vea, de un vistazo, cuanto se ha aspirado.
#[derive(Clone, Copy, Debug)]
pub struct EstadisticasCompacta {
    /// Numero de nodos alcanzables desde las anclas — el set VIVO.
    pub nodos_vivos: usize,
    /// Numero de nodos del indice que NO eran alcanzables — el set MUERTO.
    pub nodos_muertos: usize,
    /// Sectores que el log activo ocupaba antes de la compactacion.
    pub sectores_antes: u64,
    /// Sectores que ocupa el log activo despues. La diferencia es el espacio
    /// recuperado en disco (a partir del proximo arranque queda libre).
    pub sectores_despues: u64,
}

/// Numero de objetos grabados en el log desde la ultima compactacion. El
/// compositor lo consulta cada fotograma para decidir si despertar el GC.
pub fn escrituras_pendientes() -> usize {
    ESCRITURAS_DESDE_GC.load(Ordering::Relaxed)
}

/// ¿Conviene compactar AHORA? Decide por umbral fijo: tras [`UMBRAL_GC`]
/// escrituras al log, una pasada paga su coste — el log ha crecido y
/// probablemente hay nodos huerfanos que aspirar.
pub fn conviene_compactar() -> bool {
    escrituras_pendientes() >= UMBRAL_GC
}

/// Lanza una pasada de compactacion semantica. Marca los nodos alcanzables
/// desde `raiz` y `manifiesto`, escribe sus registros en un segmento limpio
/// del disco, y reanca el superbloque a ese segmento en una sola escritura.
///
/// Falla si:
///   * el segmento nuevo no cabe en lo que queda de disco (`Err("...lleno")`);
///   * un objeto alcanzable no se puede leer del log viejo (corrupcion).
///
/// El cerrojo del almacen se TOMA durante TODA la operacion. Quien llame ha
/// de hacerlo desde una tarea de baja prioridad y cuando el reactor esta
/// ocioso, no en mitad de un fotograma.
pub fn compactar() -> Result<EstadisticasCompacta, &'static str> {
    let mutex = ALMACEN.get().ok_or("almacen no inicializado")?;
    let mut almacen = mutex.lock();

    let sectores_antes = almacen.cursor.saturating_sub(almacen.log_inicio);
    let nodos_indice_antes = almacen.indice.len();

    // --- MARK :: DFS desde las anclas; alcanzables vivira como BTreeSet. ----
    // Pila lineal: arrancamos con los hashes que el superbloque sostiene.
    let mut alcanzables: BTreeSet<Hash> = BTreeSet::new();
    let mut pila: Vec<Hash> = Vec::new();
    if let Some(raiz) = almacen.raiz {
        pila.push(raiz);
    }
    if let Some(manifiesto) = almacen.manifiesto {
        pila.push(manifiesto);
    }
    while let Some(h) = pila.pop() {
        if !alcanzables.insert(h) {
            continue; // ya visitado
        }
        // Localizar el registro y leerlo. Si el objeto no esta en el indice,
        // es una arista a un nodo ausente: lo dejamos fuera del set vivo y
        // seguimos — el grafo puede tener referencias colgantes legitimas
        // (por ejemplo, un objeto no replicado aun via Akasha).
        let Some(&sector) = almacen.indice.get(&h) else {
            continue;
        };
        let payload = match leer_registro(sector)? {
            Some(p) => p,
            None => continue,
        };
        if format::hash(&payload) != h {
            // Registro corrupto: descartar este nodo. NO incendiar el GC.
            continue;
        }
        let objeto = Objeto::deserializar(&payload)?;
        for hijo in &objeto.hijos {
            if !alcanzables.contains(hijo) {
                pila.push(*hijo);
            }
        }
    }

    let nodos_vivos = alcanzables.len();
    let nodos_muertos = nodos_indice_antes.saturating_sub(nodos_vivos);

    // --- SWEEP :: copiar los registros vivos a un segmento limpio. ----------
    // El segmento nuevo arranca justo despues del log viejo. Asi, si el
    // kernel se cae antes de reescribir el superbloque, el log viejo queda
    // intacto y al proximo arranque se monta tal como estaba.
    let nuevo_inicio = almacen.cursor;
    let mut nuevo_cursor = nuevo_inicio;
    // Pre-calcular si cabra: si no, abortar SIN tocar el disco. Asi un
    // compactador que falle por espacio no deja basura escrita.
    let mut total_sectores: u64 = 0;
    for h in &alcanzables {
        let Some(&sector_viejo) = almacen.indice.get(h) else {
            continue;
        };
        let Some(payload) = leer_registro(sector_viejo)? else {
            continue;
        };
        total_sectores += format::sectores_registro(payload.len());
    }
    if nuevo_inicio + total_sectores > almacen.capacidad {
        return Err("compactar :: el set vivo no cabe en el espacio libre del disco");
    }

    // Construir el nuevo indice mientras escribimos. Si una escritura falla a
    // medio camino, el viejo indice sigue siendo coherente con el viejo
    // superbloque — no tocamos `almacen.indice` hasta el swap.
    let mut nuevo_indice: BTreeMap<Hash, u64> = BTreeMap::new();
    for h in &alcanzables {
        let Some(&sector_viejo) = almacen.indice.get(h) else {
            continue;
        };
        let Some(payload) = leer_registro(sector_viejo)? else {
            continue;
        };
        if format::hash(&payload) != *h {
            continue; // ya filtrado en mark; doble red de seguridad.
        }
        let n = format::sectores_registro(payload.len());
        let registro = format::componer_registro(&payload);
        disco::escribir_sectores(nuevo_cursor, &registro)?;
        nuevo_indice.insert(*h, nuevo_cursor);
        nuevo_cursor += n;
    }

    // --- SWAP :: una sola escritura del superbloque cambia la realidad. ----
    // Mutar el almacen vivo y persistir. Hasta esta linea, el grafo en disco
    // sigue siendo el viejo; en cuanto persistir() retorne Ok, es el nuevo.
    almacen.log_inicio = nuevo_inicio;
    almacen.cursor = nuevo_cursor;
    almacen.indice = nuevo_indice;
    persistir(&almacen)?;

    let sectores_despues = nuevo_cursor.saturating_sub(nuevo_inicio);
    // Reiniciar el contador: el log ya no tiene escrituras nuevas pendientes
    // de aspirar — todas viajaron al segmento nuevo o murieron como huerfanas.
    ESCRITURAS_DESDE_GC.store(0, Ordering::Relaxed);
    Ok(EstadisticasCompacta {
        nodos_vivos,
        nodos_muertos,
        sectores_antes,
        sectores_despues,
    })
}
