// =============================================================================
//  renaser :: kernel/src/wasm/env.rs — Fase 4/5/6 :: la matriz de capacidades
// -----------------------------------------------------------------------------
//  El aislamiento de renaser no descansa en `int 0x80` ni en `sysenter`: no hay
//  vectores de syscall. Una aplicacion WASM solo puede hacer aquello para lo
//  que el kernel le haya inyectado una FUNCION DEL HOST. Esta matriz concede:
//
//    * sys_render_frame      — componer un fotograma en su region de pantalla;
//    * sys_get_scancode      — consultar su canal de teclado;
//    * sys_object_put        — grabar un objeto en el grafo (Fase 6.1c);
//    * sys_object_datos      — leer la carga util de un objeto;
//    * sys_object_hijo       — recorrer las aristas del DAG;
//    * sys_object_raiz       — leer la raiz del grafo;
//    * sys_object_fijar_raiz — coronar un objeto como raiz;
//    * sys_estado_cargar     — leer el estado persistido de la app (Fase 7c);
//    * sys_estado_guardar    — anclar el estado persistido de la app (Fase 7c);
//    * sys_tiempo_mono       — leer el reloj monotono del sistema (Fase 11);
//    * sys_tono              — hacer sonar la bocina del PC (Fase 12);
//    * sys_net_mac           — leer la MAC de la tarjeta de red (Fase 19);
//    * sys_net_enviar        — enviar un frame Ethernet crudo (Fase 19);
//    * sys_net_recibir       — leer el siguiente frame recibido (Fase 19;
//                              desde la Fase 20, los frames Akasha se filtran
//                              en el kernel y no llegan al userspace).
//    * sys_red_solicitar     — pedir un objeto por hash a peers de capa-2; el
//                              demultiplexer absorbe la respuesta async al
//                              almacen local (Fase 21, AoE bajo demanda).
//    * sys_subsistema_registrar_ejecutable    — registrar un modulo WASM (v1).
//    * sys_subsistema_registrar_ejecutable_v2 — registrar un modulo WASM CON
//                              el hash de su codigo fuente como PRIMER HIJO:
//                              el binario arrastra su causa criptografica
//                              (Fase 31, La Arista Causal).
//    * sys_subsistema_ejecutar_dinamico       — instanciar y correr UNA SOLA
//                              VEZ un binario emitido por una app (Fase 32,
//                              El Cargador en Vivo): sub-jaula efimera, fuel
//                              acotado, retorno de `"run"` propagado al
//                              llamante. Toca el grafo? No durante el calculo
//                              (Linker vacio); solo para recuperar el binario.
//    * sys_cuaderno_anexar_celda              — anexar una celda al cuaderno
//                              acumulativo (Fase 47, Notebook DAG
//                              Accumulator). Lee el cuaderno previo del
//                              grafo, deserializa su `Vec<CeldaWawa>`,
//                              hace `push` de la nueva celda y reinscribe
//                              el cuaderno como un nodo nuevo cuyos hijos
//                              son la arista ancestral (cuaderno previo),
//                              la fuente y, si existe, el binario. Cuando
//                              `cuaderno_previo_hash` es `[0; 32]` arranca
//                              un cuaderno virgen.
//    * sys_cuaderno_leer_celda                — deserializar un nodo cuaderno
//                              del grafo y devolver UNA CeldaWawa por
//                              indice (Fase 44, Notebook Walker): habilita
//                              la persistencia entre reboots — el cold
//                              boot reconstruye el lienzo desde disco.
//                              Tras la Fase 47 el walker recorre TODAS
//                              las celdas acumuladas, no solo la ultima.
//    * sys_subsistema_vincular_macro          — inspeccionar un binario del
//                              grafo sin instanciarlo (Fase 36, Cross-App
//                              Semantic Bridge): valida magia + export
//                              `"run"` y devuelve un dictamen de 4 B
//                              listo para que la app dispare la macro
//                              con `sys_subsistema_ejecutar_dinamico`.
//    * sys_cuaderno_firmar_y_anclar           — verificar Ed25519 + anclar
//                              un cuaderno soberano (Fase 37, Firma del
//                              Tejido Celular): copia el sobre a pila,
//                              verifica autor + firma contra
//                              AGORA_PUBLIC_KEY_LOCAL, y fija el cuaderno
//                              como raiz del grafo. Zero-alloc, sin
//                              panicos posibles desde la app.
//    * sys_cuaderno_solicitar_firma_host      — emitir un hash por COM1 y
//                              leer 64 bytes de firma del operador
//                              externo (Fase 38, Host Signer Injection):
//                              el cordon umbilical limpio entre Ring 0
//                              y `wawactl`, conservando la ley "el kernel
//                              jamas firma; solo verifica".
//    * sys_subsistema_ejecutar_dinamico_v2    — evolucion del ABI dinamico
//                              con despacho polimorfico (Fase 40, Cascade
//                              Injection): inyecta un i32 a binarios cuya
//                              firma es `(i32) -> i32`, degrada elegante
//                              a `() -> i32` para modulos legacy.
//
//  GUARDARRAIL: el kernel valida MATEMATICAMENTE todo puntero que el modulo le
//  entrega contra los limites reales de su memoria lineal. No se confia en que
//  el runtime lo haga; se verifica aqui, antes de leer o escribir un solo byte.
//
//  DOS CLASES DE FALLO. Un puntero fuera de limites es CULPA DE LA APP: se
//  devuelve un `Error` que la ABORTA (el kernel la captura y la desaloja). Un
//  fallo del almacenamiento —disco, objeto inexistente— NO es culpa de la app:
//  se le devuelve un codigo de error negativo, y la app decide que hacer.
// =============================================================================

use wasmi::{Caller, Error, Extern, Linker, Memory, StoreLimits};

use crate::almacen::Hash;
use crate::async_system::puntero::CanalPuntero;
use crate::async_system::teclado::CanalTeclado;
use format::{
    CodigoError, IdiomaCodigo, Paleta, Permisos, PERMISO_ALTAVOZ, PERMISO_COMPACTAR,
    PERMISO_CONFIG, PERMISO_GRAFO_ESCRITURA, PERMISO_RAIZ, PERMISO_RED, PERMISO_TINKUY,
};

/// Cuota de paginas DMA en vuelo simultaneas por aplicacion (Fase 26).
/// Cuatro paginas de 4 KiB = 16 KiB de buffers DMA en uso a la vez por una
/// sola jaula. Una app que necesite mas ha de cooperar: pide su escritura,
/// devuelve el control con un `tick`, y al siguiente fotograma el contador
/// se ha reiniciado y puede seguir. El bus virtio-blk y virtio-net comparten
/// la arena DMA del kernel; sin este techo, un app adversaria con
/// PERMISO_GRAFO_ESCRITURA podria agotar los descriptores en segundos y
/// dejar mudo al resto del sistema.
const MAX_PAGINAS_DMA_PER_APP: u32 = 4;

/// Techo del vector acumulado en `sys_cuaderno_anexar_celda` (Fase 47).
/// Cada `CeldaWawa` mide < 80 B serializada; doce celdas caben holgadas
/// en < 1 KiB sobre la pila de Ring 0 y dejan a la app un MVP interactivo
/// generoso sin transformar la syscall en una palanca para inflar el log.
/// Anexar sobre un cuaderno lleno cortocircuita con `Saturado(-6)`.
const MAX_CELDAS_ACUMULADAS: usize = 12;

/// El estado del host adscrito al `Store` de una aplicacion: cuanto necesita
/// una capacidad para servir a ESA app y a ninguna otra — su region de pantalla,
/// su canal de teclado y sus cuotas de recursos. Dos apps jamas comparten nada.
pub(crate) struct ContextoCapacidades {
    /// El tamaño natural del lienzo de la app, en pixeles. El fotograma que
    /// entrega `sys_render_frame` mide exactamente `natural_ancho × natural_alto`;
    /// el compositor lo cachea y lo compone, sin deformarlo, en el marco que el
    /// teselado le asigno.
    pub(crate) natural_ancho: usize,
    pub(crate) natural_alto: usize,
    /// El canal de teclado propio de la aplicacion.
    pub(crate) canal: CanalTeclado,
    /// El canal del puntero propio de la aplicacion. Eventos ya traducidos
    /// al lienzo natural: la app jamas ve coordenadas absolutas ni eventos
    /// que caigan fuera de su propio perimetro.
    pub(crate) canal_puntero: CanalPuntero,
    /// El techo de recursos de la aplicacion — hoy, su memoria lineal maxima.
    /// `wasmi` lo consulta en cada `memory.grow` via `Store::limiter`.
    pub(crate) limites: StoreLimits,
    /// El indice de esta app — su identidad. La usan las capacidades de estado
    /// (Fase 7c) para hallar su `EntradaApp` del manifiesto, y el compositor
    /// (Fase 8) para hallar su ventana en el escritorio: jamas la de otra.
    pub(crate) indice_app: usize,
    /// El idioma activo, copiado del nodo `Configuracion` que enlaza el
    /// manifiesto en este fotograma. La app lo lee con `sys_config_idioma`,
    /// sin sondear nada: el kernel ya se lo dejo aqui antes de cederle el
    /// `tick`. Inyeccion UNIDIRECCIONAL: la app es ciega al origen.
    pub(crate) idioma: IdiomaCodigo,
    /// La paleta del tema activo (20 bytes, cinco RGBA8). Misma disciplina
    /// que `idioma`: refrescada por el kernel al inicio de cada `tick`, la
    /// app la lee pasivamente con `sys_config_paleta`. El cambio visual
    /// ocurre en sincronia con el refresco del fotograma — sin sondeo,
    /// sin "preguntar al kernel".
    pub(crate) paleta: Paleta,
    /// EL TIEMPO CONGELADO POR FOTOGRAMA. Snapshot de los milisegundos
    /// monotonos en el instante en que el kernel le cede la CPU a esta app.
    /// Permanece INMUTABLE durante toda la rafaga cooperativa: si la app
    /// graba tres nodos en el grafo dentro del mismo `tick`, los tres
    /// llevaran exactamente el mismo indice temporal. El reloj fisico
    /// sigue corriendo en el host; la app lo ve quieto. POSIX permite que
    /// gettimeofday devuelva tres valores distintos en tres lineas
    /// adyacentes; renaser no.
    pub(crate) tiempo_ms_fotograma: u64,
    /// PAGINAS DE INTERCAMBIO DMA en vuelo para ESTA app, en este fotograma
    /// (Fase 26). Cada escritura al grafo (`sys_object_put`) cuenta como una
    /// pagina de 4 KiB —cota generosa para acomodar payloads tipicos—. Cuando
    /// el contador supera `MAX_PAGINAS_DMA_PER_APP`, el kernel devuelve
    /// `CodigoError::Saturado` ANTES de despachar al driver: back-pressure
    /// cooperativa. La cuota se reinicia al inicio de cada `tick` —cuando
    /// el reactor le cede el control y la IRQ del disco ha tenido un
    /// fotograma para liberar descriptores—. Asi un app que abuse del bus
    /// se auto-regula sin tumbar al driver de red.
    pub(crate) paginas_dma_en_vuelo: u32,
}

/// Recupera la memoria lineal exportada por el modulo. Que no la exporte es un
/// modulo mal formado: se aborta.
fn obtener_memoria(caller: &Caller<'_, ContextoCapacidades>) -> Result<Memory, Error> {
    caller
        .get_export("memory")
        .and_then(Extern::into_memory)
        .ok_or_else(|| Error::new("WASM :: el modulo no exporta su memoria lineal"))
}

/// VALIDACION INFRANQUEABLE DE LIMITES. Comprueba que `[ptr, ptr + len)` cae
/// entera dentro de la memoria lineal `m` y devuelve ese sub-slice. Un rango
/// que se desborde aborta la app — el `Error` se traduce en una trampa de WASM.
fn rango<'a>(m: &'a [u8], ptr: u32, len: usize, fallo: &'static str) -> Result<&'a [u8], Error> {
    let inicio = ptr as usize;
    match inicio.checked_add(len) {
        Some(fin) if fin <= m.len() => Ok(&m[inicio..fin]),
        _ => Err(Error::new(fallo)),
    }
}

/// Lee un hash de 32 bytes de la memoria lineal, con sus limites verificados.
fn leer_hash(m: &[u8], ptr: u32, fallo: &'static str) -> Result<Hash, Error> {
    let bytes = rango(m, ptr, 32, fallo)?;
    let mut hash = [0u8; 32];
    hash.copy_from_slice(bytes);
    Ok(hash)
}

/// Inyecta en el enlazador la matriz de capacidades del modulo WASM. Todo lo
/// que no se defina aqui le queda, al modulo, fisicamente fuera de alcance.
///
/// `permisos` es el bitfield de [`Permisos`] que el manifiesto declaro para
/// esta app. Las capacidades GATEADAS (red, raiz, altavoz, configuracion,
/// escritura del grafo) solo se registran si el bit correspondiente esta
/// puesto; si no, su import queda sin resolver y wasmi rechazara el modulo
/// que intente importarla — la frontera es fisica, no chequeada en runtime.
/// No hay escalada porque no hay tabla que escalar.
///
/// SWAP SEMANTICO: las capacidades `sys_object_put`, `sys_object_datos`,
/// `sys_estado_guardar` y `sys_estado_cargar` SON el swap del sistema.
/// Cuando una app necesita liberar espacio en su jaula de 4 MiB, serializa
/// sus estructuras intermedias con postcard, las graba en el grafo (un
/// hash) y limpia su propia memoria lineal. Cuando vuelve a necesitarlas,
/// las trae de vuelta por hash. Es una decision CONSCIENTE del userspace,
/// no un paginado ciego del kernel: el coste de E/S esta a la vista de
/// quien lo paga, y nada se mueve a sus espaldas. POSIX hace swap a ojos
/// cerrados y destroza el rendimiento; renaser entrega las herramientas y
/// confia en que la app sepa lo que hace con sus 4 MiB.
///
/// Devuelve `Err` si una capacidad no se pudo enlazar — un fallo del kernel,
/// no de la app; aun asi se propaga como `Result` para no incendiar nada.
pub(crate) fn enlazar_capacidades(
    enlazador: &mut Linker<ContextoCapacidades>,
    permisos: Permisos,
) -> Result<(), Error> {
    // Una familia de capacidades por función; los gates de permiso viven
    // dentro de cada una (per-syscall, intactos). Este despachador es el
    // mapa legible de qué superficie expone el kernel al userspace WASM.
    enlazar_presentacion(enlazador)?;
    enlazar_grafo(enlazador, permisos)?;
    enlazar_objeto(enlazador)?;
    enlazar_raiz_canal(enlazador, permisos)?;
    enlazar_estado_dispositivos(enlazador, permisos)?;
    enlazar_red(enlazador, permisos)?;
    enlazar_config(enlazador, permisos)?;
    enlazar_anuncio_tinkuy(enlazador, permisos)?;
    Ok(())
}


// --- Familias de capacidades, una por submódulo. Cada una abre con
// `use super::*` y ve helpers/consts/ContextoCapacidades de aquí por
// la regla de visibilidad descendiente. ---
mod anuncio_tinkuy;
mod config;
mod dispositivos;
mod grafo;
mod objeto;
mod presentacion;
mod raiz_canal;
mod red;

use anuncio_tinkuy::*;
use config::*;
use dispositivos::*;
use grafo::*;
use objeto::*;
use presentacion::*;
use raiz_canal::*;
use red::*;
