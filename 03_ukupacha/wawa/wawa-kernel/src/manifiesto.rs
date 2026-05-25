// =============================================================================
//  renaser :: kernel/src/manifiesto.rs — Fase 7 :: el Manifiesto de Génesis
// -----------------------------------------------------------------------------
//  Hasta la Fase 6, el userspace venia EMPOTRADO en el binario del kernel:
//  `include_bytes!` de cada `.wasm` y regiones escritas a mano. La Fase 7 lo
//  desterro: las aplicaciones son OBJETOS DEL GRAFO, y lo que arranca —con que
//  cuota, en que region— lo dicta este Manifiesto de Genesis, que tambien
//  habita el grafo. El superbloque guarda su hash en un ancla propia.
//
//  ESTADO: Fase 7c. El manifiesto deja de ser de solo lectura. El kernel
//  conserva una copia VIVA —`VIVO`, un `Mutex<Manifiesto>`—; cuando una app
//  persiste su estado (`sys_estado_guardar`), el kernel actualiza la ranura
//  `estado` de su `EntradaApp`, re-graba el manifiesto en el grafo y lo
//  re-ancla. Asi el estado de cada app sobrevive, por separado, a un reinicio.
//
//  Los tipos `Manifiesto` / `EntradaApp` y su (de)serializacion viven en la
//  crate `format`, el nucleo `no_std` compartido con `boot`. Aqui solo queda
//  lo que es del kernel: cargar el manifiesto, traducir regiones, custodiar la
//  copia viva y mutarla cuando una app graba su estado.
// =============================================================================

use alloc::vec::Vec;

use spin::{Mutex, Once};

use format::Hash;

use crate::almacen;
use crate::grafico::RegionPantalla;

// Los tipos del manifiesto los define `format`; se reexportan para que el
// resto del kernel los nombre `manifiesto::EntradaApp` / `manifiesto::Manifiesto`.
pub use format::{EntradaApp, Manifiesto};

/// El manifiesto VIVO del kernel: la copia en memoria, mutable, del Manifiesto
/// de Genesis. Las apps la actualizan al persistir su estado (Fase 7c); el
/// kernel la re-graba en el grafo y la re-ancla en el superbloque. Se instala
/// una sola vez, en el arranque, con [`instalar`].
static VIVO: Once<Mutex<Manifiesto>> = Once::new();

/// Traduce la sub-region de una `EntradaApp` —campos `u32` de ancho fijo, el
/// format en disco— a la `RegionPantalla` que el kernel entiende (`usize`,
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

/// Instala el manifiesto recien cargado como el manifiesto VIVO del kernel. Se
/// invoca una sola vez, en el arranque, ANTES de instanciar las apps — el
/// `init` de cada app ya consulta su estado persistido, y eso lee de aqui.
pub fn instalar(manifiesto: Manifiesto) {
    VIVO.call_once(|| Mutex::new(manifiesto));
}

/// El hash del estado persistido de la app `indice`, si tiene uno anclado. Lo
/// consulta la capacidad `sys_estado_cargar` cuando una app despierta.
pub fn estado_de(indice: usize) -> Option<Hash> {
    VIVO.get()
        .and_then(|vivo| vivo.lock().apps.get(indice).and_then(|app| app.estado))
}

/// Registra `hash` como el nuevo estado persistido de la app `indice`: muta el
/// manifiesto vivo, lo re-serializa, lo graba como un objeto NUEVO del grafo y
/// lo re-ancla en el superbloque. Desde esta llamada, el estado de esa app
/// sobrevive a un reinicio. La invoca la capacidad `sys_estado_guardar`.
pub fn fijar_estado(indice: usize, hash: Hash) -> Result<(), &'static str> {
    let vivo = VIVO.get().ok_or("manifiesto :: no hay manifiesto vivo")?;
    let mut manifiesto = vivo.lock();
    let entrada = manifiesto
        .apps
        .get_mut(indice)
        .ok_or("manifiesto :: indice de app fuera de rango")?;
    entrada.estado = Some(hash);

    // Re-grabar el manifiesto mutado. Sus `hijos` son, como en la siembra de
    // `boot`, los objetos de bytecode deduplicados: el grafo lo sigue leyendo
    // como el nodo padre del userspace. El objeto nuevo se ancla en el
    // superbloque; el viejo queda en el log, inerte e inofensivo.
    let bytes = manifiesto.serializar()?;
    let mut hijos: Vec<Hash> = Vec::new();
    for app in &manifiesto.apps {
        if !hijos.contains(&app.bytecode) {
            hijos.push(app.bytecode);
        }
    }
    let nuevo = almacen::almacenar(bytes, hijos)?;
    almacen::fijar_manifiesto(nuevo)
}
