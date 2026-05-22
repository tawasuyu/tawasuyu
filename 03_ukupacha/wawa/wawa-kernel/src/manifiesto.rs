// =============================================================================
//  renaser :: kernel/src/manifiesto.rs — Fase 7 :: el Manifiesto de Génesis
// -----------------------------------------------------------------------------
//  Hasta la Fase 6, el userspace venia EMPOTRADO en el binario del kernel:
//  `include_bytes!` de cada `.wasm` y regiones escritas a mano. La Fase 7 lo
//  desterro: las aplicaciones son OBJETOS DEL GRAFO, y lo que arranca —con que
//  cuota, en que region— lo dicta este Manifiesto de Genesis, que tambien
//  habita el grafo. El superbloque guarda su hash en un ancla propia.
//
//  ESTADO: Fase 7b. El kernel ya NO empotra una sola app. La siembra de la
//  imagen —grabar los objetos de bytecode y el manifiesto en un disco virgen—
//  la hace por completo el constructor de imagen `boot`, en el anfitrion. Este
//  modulo se reduce a su esencia: LEER el manifiesto del grafo al arrancar.
//
//  Los tipos `Manifiesto` / `EntradaApp` y su (de)serializacion viven en la
//  crate `formato`, el nucleo `no_std` compartido con `boot`. Aqui solo queda
//  lo que es del kernel: recuperar el manifiesto del grafo y traducir las
//  regiones de su formato en disco (`u32`) a la `RegionPantalla` del kernel.
// =============================================================================

use crate::almacen;
use crate::grafico::RegionPantalla;

// Los tipos del manifiesto los define `formato`; se reexportan para que el
// resto del kernel los nombre `manifiesto::EntradaApp` / `manifiesto::Manifiesto`.
pub use formato::{EntradaApp, Manifiesto};

/// Traduce la sub-region de una `EntradaApp` —campos `u32` de ancho fijo, el
/// formato en disco— a la `RegionPantalla` que el kernel entiende (`usize`,
/// ancho de plataforma). El puente entre lo que el disco guarda y lo que el
/// compositor dibuja.
pub fn region(entrada: &EntradaApp) -> RegionPantalla {
    RegionPantalla {
        x: entrada.region_x as usize,
        y: entrada.region_y as usize,
        ancho: entrada.region_ancho as usize,
        alto: entrada.region_alto as usize,
    }
}

/// Lee el manifiesto del grafo: toma su hash del ancla del superbloque,
/// recupera el objeto y lo deserializa. `Ok(None)` si el disco no tiene
/// manifiesto anclado —un disco que `boot` no sembro—; el kernel se levanta
/// entonces sin userspace, pero se levanta.
pub fn cargar() -> Result<Option<Manifiesto>, &'static str> {
    let hash = match almacen::manifiesto() {
        Some(hash) => hash,
        None => return Ok(None),
    };
    // `recuperar` recomputa el hash del objeto y verifica su integridad: un
    // manifiesto corrupto se delata aqui.
    let objeto = almacen::recuperar(&hash)?
        .ok_or("manifiesto :: el objeto anclado no existe en el grafo")?;
    let manifiesto = Manifiesto::deserializar(&objeto.datos)?;
    Ok(Some(manifiesto))
}
