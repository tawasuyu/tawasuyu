// =============================================================================
//  renaser :: kernel/src/manifiesto.rs — Fase 7 :: el Manifiesto de Génesis
// -----------------------------------------------------------------------------
//  Hasta la Fase 6, el userspace venia EMPOTRADO en el binario del kernel:
//  `include_bytes!` de cada `.wasm` y regiones escritas a mano. La Fase 7 lo
//  destierra: las aplicaciones pasan a ser OBJETOS DEL GRAFO, y lo que arranca
//  —con que cuota, en que region— lo dicta este Manifiesto de Genesis, que
//  tambien habita el grafo. El superbloque guarda su hash en un ancla propia.
//
//  El kernel, al despertar: lee el ancla del superbloque, recupera el objeto
//  del manifiesto, lo deserializa, y por cada `EntradaApp` recupera el objeto
//  de bytecode —verificado por su hash— y lo inyecta en `wasmi`.
//
//  ESTADO: Fase 7a. Tipos, (de)serializacion, carga desde el grafo y siembra
//  de la genesis, implementados. La siembra es TRANSITORIA — el bytecode aun
//  viaja empotrado (`include_bytes!`, abajo); la Fase 7b lo movera al
//  constructor de imagen `boot` y el kernel dejara de empotrar una sola app.
//  Ver `FASE7.md` para el plan completo.
// =============================================================================

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::almacen::Hash;
use crate::grafico::RegionPantalla;

/// Version del formato del manifiesto serializado. Independiente de la
/// version del superbloque (`almacen::VERSION`): el manifiesto es un objeto
/// del grafo, no una estructura de disco.
pub const VERSION_MANIFIESTO: u32 = 1;

/// El Manifiesto de Genesis: la lista de aplicaciones que el kernel instancia
/// al arrancar. Vive como un objeto del grafo de objetos; el superbloque
/// guarda su hash en el campo `manifiesto`.
#[derive(Serialize, Deserialize, Clone)]
pub struct Manifiesto {
    /// Version del formato — debe ser [`VERSION_MANIFIESTO`].
    pub version: u32,
    /// Las aplicaciones del userspace, en orden de arranque.
    pub apps: Vec<EntradaApp>,
}

/// Una entrada del manifiesto: una aplicacion del userspace y todo lo que el
/// kernel necesita para darle vida — su bytecode, su ventana, su cuota de
/// memoria y, si lo tuviera, su ultimo estado persistido.
#[derive(Serialize, Deserialize, Clone)]
pub struct EntradaApp {
    /// Nombre legible — para los rotulos de la consola y la baliza.
    pub nombre: String,
    /// Hash del objeto del grafo que contiene el bytecode WASM de la app.
    pub bytecode: Hash,
    /// Sub-region del framebuffer asignada a la app. Campos de ancho fijo
    /// `u32` A PROPOSITO: esto es un formato EN DISCO. `RegionPantalla` usa
    /// `usize` (ancho dependiente de plataforma) y no sirve para serializar.
    pub region_x: u32,
    pub region_y: u32,
    pub region_ancho: u32,
    pub region_alto: u32,
    /// Techo de memoria lineal de la app, en bytes. Sustituye a la constante
    /// global `wasm::TECHO_MEMORIA` — cada app lleva su cuota.
    pub techo_memoria: u32,
    /// Hash del ultimo estado persistido de la app (Fase 7c). `None` hasta
    /// que la app guarde estado por primera vez.
    pub estado: Option<Hash>,
}

impl EntradaApp {
    /// Construye la `RegionPantalla` que el kernel entiende a partir de los
    /// campos de ancho fijo del manifiesto.
    pub fn region(&self) -> RegionPantalla {
        RegionPantalla {
            x: self.region_x as usize,
            y: self.region_y as usize,
            ancho: self.region_ancho as usize,
            alto: self.region_alto as usize,
        }
    }
}

impl Manifiesto {
    /// Serializa el manifiesto a su forma binaria `postcard` — la carga util
    /// del objeto del grafo que lo aloja.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self).map_err(|_| "manifiesto :: serializacion fallida")
    }

    /// Reconstruye un manifiesto desde la carga util de su objeto. Rechaza
    /// un formato de version desconocida en lugar de malinterpretarlo.
    pub fn deserializar(bytes: &[u8]) -> Result<Manifiesto, &'static str> {
        let (manifiesto, _) = postcard::take_from_bytes::<Manifiesto>(bytes)
            .map_err(|_| "manifiesto :: deserializacion fallida")?;
        if manifiesto.version != VERSION_MANIFIESTO {
            return Err("manifiesto :: version de formato desconocida");
        }
        Ok(manifiesto)
    }
}

/// Lee el manifiesto del grafo: toma su hash del ancla del superbloque,
/// recupera el objeto y lo deserializa. `Ok(None)` si el disco aun no tiene
/// manifiesto anclado — el caller debe entonces sembrar la genesis.
pub fn cargar() -> Result<Option<Manifiesto>, &'static str> {
    let hash = match crate::almacen::manifiesto() {
        Some(hash) => hash,
        None => return Ok(None),
    };
    // `recuperar` recomputa el hash del objeto y verifica su integridad: un
    // manifiesto corrupto se delata aqui.
    let objeto = crate::almacen::recuperar(&hash)?
        .ok_or("manifiesto :: el objeto anclado no existe en el grafo")?;
    let manifiesto = Manifiesto::deserializar(&objeto.datos)?;
    Ok(Some(manifiesto))
}

// =============================================================================
//  La genesis — la semilla transitoria de la Fase 7a
// -----------------------------------------------------------------------------
//  El bytecode de las apps de genesis viaja, POR AHORA, empotrado en el kernel.
//  Es el unico `include_bytes!` que sobrevive a la Fase 7a — y solo como
//  semilla: en un disco virgen, `sembrar_genesis` lo graba en el grafo una vez.
//  La Fase 7b lo movera al constructor de imagen `boot` y este bloque morira.
// =============================================================================

static APP_WASM: &[u8] = include_bytes!("../assets/app.wasm");
static DISCOLA_WASM: &[u8] = include_bytes!("../assets/discola.wasm");
static GLOTONA_WASM: &[u8] = include_bytes!("../assets/glotona.wasm");
static CRONISTA_WASM: &[u8] = include_bytes!("../assets/cronista.wasm");

/// Descriptor de una app de genesis: lo que el kernel sabe de ella ANTES de
/// que exista en el grafo. `region` es `(x, y, ancho, alto)` en pixeles.
struct AppGenesis {
    nombre: &'static str,
    bytecode: &'static [u8],
    region: (u32, u32, u32, u32),
    techo_memoria: u32,
}

/// El userspace de genesis: las cinco aplicaciones que pueblan un disco
/// virgen, con las regiones de la Fase 6.2. `app.wasm` aparece dos veces
/// —dos instancias del mismo bytecode—; el grafo, direccionado por contenido,
/// lo guarda una sola vez.
fn genesis() -> [AppGenesis; 5] {
    let techo = crate::wasm::TECHO_MEMORIA as u32;
    [
        AppGenesis {
            nombre: "hola-izq",
            bytecode: APP_WASM,
            region: (100, 120, 480, 560),
            techo_memoria: techo,
        },
        AppGenesis {
            nombre: "hola-der",
            bytecode: APP_WASM,
            region: (700, 120, 480, 560),
            techo_memoria: techo,
        },
        AppGenesis {
            nombre: "discola",
            bytecode: DISCOLA_WASM,
            region: (60, 700, 360, 80),
            techo_memoria: techo,
        },
        AppGenesis {
            nombre: "glotona",
            bytecode: GLOTONA_WASM,
            region: (460, 700, 360, 80),
            techo_memoria: techo,
        },
        AppGenesis {
            nombre: "cronista",
            bytecode: CRONISTA_WASM,
            region: (860, 700, 360, 80),
            techo_memoria: techo,
        },
    ]
}

/// Siembra el grafo en un disco sin manifiesto: graba el bytecode de cada app
/// de genesis como un objeto, compone un `Manifiesto` con sus regiones y
/// cuotas, lo graba —con las aristas hacia los objetos de bytecode— y lo
/// ancla en el superbloque. Devuelve el hash del manifiesto recien anclado.
pub fn sembrar_genesis() -> Result<Hash, &'static str> {
    let mut apps: Vec<EntradaApp> = Vec::new();
    let mut hijos: Vec<Hash> = Vec::new();

    for app in genesis() {
        // Grabar el bytecode como objeto del grafo. Idempotente: dos
        // instancias de la misma app comparten un unico objeto.
        let bytecode = crate::almacen::almacenar(app.bytecode.to_vec(), Vec::new())?;
        if !hijos.contains(&bytecode) {
            hijos.push(bytecode);
        }
        let (x, y, ancho, alto) = app.region;
        apps.push(EntradaApp {
            nombre: String::from(app.nombre),
            bytecode,
            region_x: x,
            region_y: y,
            region_ancho: ancho,
            region_alto: alto,
            techo_memoria: app.techo_memoria,
            estado: None,
        });
    }

    // El objeto del manifiesto: sus `hijos` son los objetos de bytecode, de
    // modo que el grafo lo lea como el nodo padre del userspace.
    let manifiesto = Manifiesto {
        version: VERSION_MANIFIESTO,
        apps,
    };
    let bytes = manifiesto.serializar()?;
    let hash = crate::almacen::almacenar(bytes, hijos)?;
    crate::almacen::fijar_manifiesto(hash)?;
    Ok(hash)
}
