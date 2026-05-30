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
pub use format::{Configuracion, EntradaApp, Manifiesto};

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

/// Aplica el overlay de revocación que el manifiesto vivo ancla, si ancla uno.
/// Lo lee FRESH del grafo, lo deserializa y delega en
/// [`crate::claves::aplicar_overlay_revocacion`], que enciende los slots del
/// `AGORA_AUTH_RING` revocados por quórum — desde ahí `autor_en_anillo` deniega
/// esas claves (SDD-rotacion-revocacion §4). Devuelve cuántos slots quedaron
/// revocados (`0` también si no hay overlay anclado).
///
/// Se invoca UNA vez en el arranque, DESPUÉS de [`instalar`] y ANTES de aceptar
/// propuesta soberana alguna. Una falla al leer/deserializar el overlay NO
/// incendia el arranque: se trata como "sin revocaciones" (`0`) — un overlay
/// corrupto no debe dejar al sistema sin userspace; el operador lo re-ancla.
/// (Fail-safe en disponibilidad; el gate de autoridad sigue intacto: ninguna
/// clave se ACEPTA por error, sólo deja de denegarse una que el overlay no pudo
/// confirmar revocada.)
pub fn aplicar_overlay() -> u32 {
    let Some(vivo) = VIVO.get() else {
        return 0;
    };
    let hash = match vivo.lock().overlay_revocacion {
        Some(hash) => hash,
        None => return 0,
    };
    let objeto = match almacen::recuperar(&hash) {
        Ok(Some(objeto)) => objeto,
        _ => return 0, // overlay anclado pero ausente/ilegible: sin revocaciones
    };
    match format::OverlayRevocacion::deserializar(&objeto.datos) {
        Ok(overlay) => crate::claves::aplicar_overlay_revocacion(&overlay),
        Err(_) => 0,
    }
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
    regrabar_y_reanclar(&manifiesto)
}

/// El hash de la `Configuracion` activa, si el manifiesto enlaza una. `None`
/// significa que el kernel emplea [`Configuracion::por_defecto`].
pub fn configuracion_activa() -> Option<Hash> {
    VIVO.get().and_then(|vivo| vivo.lock().configuracion)
}

/// Lee la `Configuracion` que el manifiesto enlaza ahora, ya deserializada. Si
/// el manifiesto no enlaza ninguna —o el objeto no se halla en el grafo, o no
/// se deserializa—, devuelve [`Configuracion::por_defecto`]. El kernel jamas
/// se queda sin configuracion: el "no hay" se rellena con el defecto, no con
/// un error que detenga el fotograma.
pub fn cargar_configuracion() -> Configuracion {
    let Some(hash) = configuracion_activa() else {
        return Configuracion::por_defecto();
    };
    match almacen::recuperar(&hash) {
        Ok(Some(objeto)) => {
            Configuracion::deserializar(&objeto.datos).unwrap_or_else(|_| Configuracion::por_defecto())
        }
        _ => Configuracion::por_defecto(),
    }
}

/// Engendra un nodo nuevo `Configuracion` en el grafo, reancla el manifiesto al
/// hash recien creado y lo re-graba en disco — todo en un solo paso atomico
/// desde el punto de vista del proximo fotograma. Es la unica via para mutar
/// la configuracion activa: nadie escribe en una "variable global"; cada
/// cambio engendra un nodo nuevo, hashable e inmutable, y solo el puntero del
/// manifiesto se mueve.
///
/// La hermeticidad de la red descansa en esto: aunque Akasha absorba un objeto
/// `Configuracion` ajeno al grafo local, ese objeto NO altera la configuracion
/// vigente; solo este camino —y solo cuando el usuario local lo invoca con un
/// hash que ya esta en el grafo local— mueve el puntero del manifiesto vivo.
pub fn fijar_configuracion(configuracion: Configuracion) -> Result<Hash, &'static str> {
    let bytes = configuracion.serializar()?;
    let hash = almacen::almacenar(bytes, Vec::new())?;
    enlazar_configuracion(hash)?;
    Ok(hash)
}

/// Reancla el manifiesto vivo a un `hash` de configuracion que ya existe en el
/// grafo local. NO copia datos del exterior: si el objeto no esta en el grafo,
/// se devuelve error. Esta es la frontera que separa "recibir un objeto por
/// red" de "aplicar una configuracion": la red puede ingestar el objeto en el
/// grafo (Akasha lo hace si el rehash cuadra), pero solo este enlace —invocado
/// por un camino local que confia en quien lo invoca— lo aplica.
pub fn enlazar_configuracion(hash: Hash) -> Result<(), &'static str> {
    let objeto = almacen::recuperar(&hash)?
        .ok_or("configuracion :: el objeto no esta en el grafo local")?;
    // Defensa: verificar que el objeto se deserialice como una Configuracion
    // bien formada antes de reanclar el manifiesto. Un puntero a un objeto
    // arbitrario reanclado se delata aqui, no en el proximo fotograma.
    Configuracion::deserializar(&objeto.datos)?;

    let vivo = VIVO.get().ok_or("manifiesto :: no hay manifiesto vivo")?;
    let mut manifiesto = vivo.lock();
    manifiesto.configuracion = Some(hash);
    regrabar_y_reanclar(&manifiesto)
}

/// Re-serializa el manifiesto vivo, lo graba como un objeto NUEVO del grafo y
/// lo re-ancla en el superbloque. Sus `hijos` son los bytecodes de las apps
/// deduplicados —el grafo lo sigue leyendo como el nodo padre del userspace—.
/// El objeto previo queda en el log, inerte e inofensivo: la atomicidad del
/// cambio descansa en el reanclaje del superbloque, no en la mutacion en sitio.
fn regrabar_y_reanclar(manifiesto: &Manifiesto) -> Result<(), &'static str> {
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
