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
//  ESTADO: andamiaje de la Fase 7a. Los tipos y la (de)serializacion estan
//  completos; `cargar` y `sembrar_genesis` son esbozos — se implementan al
//  abordar la 7a, cuando el superbloque gane su campo `manifiesto`. Ver
//  `FASE7.md` para el plan completo.
// =============================================================================

// Fase 7a en construccion: el modulo aun no se cablea a `kernel_main`. El
// `allow` cae en cuanto `cargar`/`sembrar_genesis` tengan llamador real.
#![allow(dead_code)]

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
///
/// ANDAMIAJE (Fase 7a-4): depende de `almacen::manifiesto()` —el nuevo ancla
/// del superbloque— todavia por implementar (tarea 7a-2).
pub fn cargar() -> Result<Option<Manifiesto>, &'static str> {
    todo!("Fase 7a-4: leer almacen::manifiesto(), recuperar el objeto y deserializar")
}

/// Siembra el grafo en un disco sin manifiesto: graba el bytecode de las
/// aplicaciones de genesis, compone un `Manifiesto` por defecto con sus
/// regiones y cuotas, lo graba y lo ancla en el superbloque. Devuelve el
/// hash del manifiesto recien anclado.
///
/// ANDAMIAJE (Fase 7a-3): la semilla TRANSITORIA — en la 7a el bytecode aun
/// llega vacia `include_bytes!`; la 7b mueve la siembra al constructor de
/// imagen `boot` y elimina el empotrado del kernel.
pub fn sembrar_genesis() -> Result<Hash, &'static str> {
    todo!("Fase 7a-3: grabar los bytecodes de genesis + el manifiesto por defecto, y anclarlo")
}
