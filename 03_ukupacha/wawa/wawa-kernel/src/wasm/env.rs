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
    CodigoError, IdiomaCodigo, Paleta, Permisos, PERMISO_ALTAVOZ, PERMISO_CONFIG,
    PERMISO_GRAFO_ESCRITURA, PERMISO_RAIZ, PERMISO_RED,
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
    // --- CAPACIDAD 1 :: sys_render_frame(ptr, len) ---
    // El modulo entrega (ptr, len) hacia su PROPIA memoria lineal; el kernel
    // valida esos limites y, solo entonces, compone el fotograma DENTRO de la
    // region asignada a la app.
    enlazador.func_wrap(
        "renaser",
        "sys_render_frame",
        |caller: Caller<'_, ContextoCapacidades>, ptr: u32, len: u32| -> Result<(), Error> {
            let indice = caller.data().indice_app;
            let nat_ancho = caller.data().natural_ancho;
            let nat_alto = caller.data().natural_alto;

            // El fotograma debe medir EXACTAMENTE el lienzo natural de la app.
            // Un tamaño distinto delata a una app que pinta fuera de su lienzo:
            // se aborta antes de tocar un byte.
            let esperado = nat_ancho * nat_alto * 4;
            if len as usize != esperado {
                return Err(Error::new(
                    "WASM :: sys_render_frame con un fotograma ajeno al lienzo natural",
                ));
            }

            let memoria = obtener_memoria(&caller)?;
            let datos: &[u8] = memoria.data(&caller);

            // VALIDACION INFRANQUEABLE: si (ptr, len) se sale de la memoria
            // lineal del modulo, se aborta la app —no el kernel—.
            let fotograma = rango(
                datos,
                ptr,
                len as usize,
                "WASM :: sys_render_frame desbordo la memoria lineal del modulo",
            )?;

            // Limites verificados: el compositor cachea el fotograma —para
            // poder recomponerlo si el escritorio se re-tesela— y lo compone,
            // centrado, en el marco que el teselado asigno a esta app.
            crate::compositor::presentar_fotograma(indice, fotograma);
            Ok(())
        },
    )?;

    // --- CAPACIDAD 2 :: sys_get_scancode() -> u32 ---
    // Expone, sin bloquear, el siguiente scancode del canal PROPIO de la app.
    enlazador.func_wrap(
        "renaser",
        "sys_get_scancode",
        |caller: Caller<'_, ContextoCapacidades>| -> u32 {
            caller.data().canal.pop().unwrap_or(0) as u32
        },
    )?;

    // --- CAPACIDAD 2b :: sys_puntero(salida) -> i32 ---
    // Saca el siguiente evento del puntero del canal PROPIO de la app, ya
    // TRADUCIDO al lienzo natural por el compositor. Escribe cinco bytes en
    // `salida`: local_x (u16 LE), local_y (u16 LE), botones (u8). Devuelve
    // 5 si habia evento, 0 si la cola esta vacia.
    //
    // INYECCION UNIDIRECCIONAL y GEOMETRICA. La app jamas conoce la posicion
    // absoluta del puntero: el kernel solo deposita eventos cuyo (x, y)
    // ABSOLUTO cae dentro del propio lienzo natural de la app. Clics sobre
    // otras ventanas, sobre el cromo de la propia ventana o sobre la
    // taskbar nunca llegan aqui. Es la matematica de mirada-layout decidiendo,
    // no un chequeo de la app: la geometria del marco no es opcional.
    enlazador.func_wrap(
        "renaser",
        "sys_puntero",
        |mut caller: Caller<'_, ContextoCapacidades>, salida: u32| -> Result<i32, Error> {
            let evento = match caller.data().canal_puntero.pop() {
                Some(e) => e,
                None => return Ok(CodigoError::Ok.como_i32()),
            };
            let memoria = obtener_memoria(&caller)?;
            {
                let m = memoria.data(&caller);
                rango(m, salida, 5, "WASM :: sys_puntero desbordo la memoria lineal")?;
            }
            let m = memoria.data_mut(&mut caller);
            let off = salida as usize;
            m[off..off + 2].copy_from_slice(&evento.local_x.to_le_bytes());
            m[off + 2..off + 4].copy_from_slice(&evento.local_y.to_le_bytes());
            m[off + 4] = evento.botones;
            Ok(5)
        },
    )?;

    // --- CAPACIDAD 3 :: sys_object_put(datos, datos_len, hijos, hijos_cnt, salida) -> i32 ---
    // Graba un objeto en el grafo. El modulo entrega, en su memoria lineal, la
    // carga util y un arreglo de `hijos_cnt` hashes de 32 bytes (las aristas).
    // El kernel escribe el hash resultante —la identidad del objeto— en
    // `salida`. Devuelve 0 si el objeto se grabo (o ya existia), -1 si el
    // almacenamiento fallo.
    //
    // GATEADA por PERMISO_GRAFO_ESCRITURA: si la app no lo declaro en su
    // EntradaApp, este import NO se registra y el modulo no la puede
    // invocar — el simbolo, sencillamente, no existe.
    if permisos & PERMISO_GRAFO_ESCRITURA != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_object_put",
        |mut caller: Caller<'_, ContextoCapacidades>,
         datos_ptr: u32,
         datos_len: u32,
         hijos_ptr: u32,
         hijos_cnt: u32,
         salida: u32|
         -> Result<i32, Error> {
            // BACK-PRESSURE DMA (Fase 26). Si la app ha grabado ya su techo
            // en este `tick`, devolvemos `Saturado` SIN despachar al driver
            // —el unico camino legitimo es retirarse y reintentar en el
            // proximo fotograma—. La cuota se reinicia al inicio de cada
            // `tick` (ver `AplicacionWasm::tick`).
            if caller.data().paginas_dma_en_vuelo >= MAX_PAGINAS_DMA_PER_APP {
                return Ok(CodigoError::Saturado.como_i32());
            }
            // Reservar la pagina ANTES de tocar el disco. Si el almacen
            // falla y devuelve error, la decrementaremos en la rama de
            // fallo (ver mas abajo); asi una rafaga de fallos no se queda
            // pegada con paginas "ocupadas" ficticiamente.
            caller.data_mut().paginas_dma_en_vuelo += 1;
            let memoria = obtener_memoria(&caller)?;

            // --- Leer las entradas de la memoria lineal, con limites firmes. ---
            let (datos, hijos) = {
                let m = memoria.data(&caller);

                let datos = rango(
                    m,
                    datos_ptr,
                    datos_len as usize,
                    "WASM :: sys_object_put desbordo la memoria lineal (datos)",
                )?
                .to_vec();

                // El arreglo de hijos: `hijos_cnt` hashes contiguos de 32 bytes.
                let bytes_hijos = (hijos_cnt as usize).checked_mul(32).ok_or_else(|| {
                    Error::new("WASM :: sys_object_put con un conteo de hijos imposible")
                })?;
                let crudo = rango(
                    m,
                    hijos_ptr,
                    bytes_hijos,
                    "WASM :: sys_object_put desbordo la memoria lineal (hijos)",
                )?;
                let mut hijos: alloc::vec::Vec<Hash> =
                    alloc::vec::Vec::with_capacity(hijos_cnt as usize);
                for trozo in crudo.chunks_exact(32) {
                    let mut h = [0u8; 32];
                    h.copy_from_slice(trozo);
                    hijos.push(h);
                }

                // Verificar que el hash de salida cabe ANTES de tocar el disco.
                rango(
                    m,
                    salida,
                    32,
                    "WASM :: sys_object_put desbordo la memoria lineal (salida)",
                )?;

                (datos, hijos)
            };

            // --- Grabar. Un fallo del almacen NO es culpa de la app. ---
            let resultado = match crate::almacen::almacenar(datos, hijos) {
                Ok(hash) => {
                    let m = memoria.data_mut(&mut caller);
                    m[salida as usize..salida as usize + 32].copy_from_slice(&hash);
                    CodigoError::Ok
                }
                Err(_) => CodigoError::AlmacenamientoFallo,
            };
            // Devolver la pagina al pozo: la operacion termino (con exito o
            // con fallo) y los descriptores virtio quedaron liberados por
            // el camino sincrono del driver. Si en el futuro `almacenar`
            // se vuelve async, este decremento migrara al despertar del
            // waker que arme la IRQ del disco — el contrato con la app no
            // cambia.
            caller.data_mut().paginas_dma_en_vuelo -= 1;
            Ok(resultado.como_i32())
        },
    )?;
    } // PERMISO_GRAFO_ESCRITURA

    // --- CAPACIDAD 4 :: sys_object_datos(hash, salida, capacidad) -> i32 ---
    // Copia la carga util del objeto `hash` en `salida`. Devuelve el numero de
    // bytes copiados, o -1 si el objeto no existe, -2 si `capacidad` no basta,
    // -3 si el almacenamiento fallo.
    enlazador.func_wrap(
        "renaser",
        "sys_object_datos",
        |mut caller: Caller<'_, ContextoCapacidades>,
         hash_ptr: u32,
         salida: u32,
         capacidad: u32|
         -> Result<i32, Error> {
            let memoria = obtener_memoria(&caller)?;

            let hash = {
                let m = memoria.data(&caller);
                leer_hash(
                    m,
                    hash_ptr,
                    "WASM :: sys_object_datos desbordo la memoria lineal (hash)",
                )?
            };

            let objeto = match crate::almacen::recuperar(&hash) {
                Ok(Some(objeto)) => objeto,
                Ok(None) => return Ok(CodigoError::Ausente.como_i32()),
                Err(_) => return Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            };
            if objeto.datos.len() > capacidad as usize {
                return Ok(CodigoError::CapacidadInsuficiente.como_i32());
            }

            // Verificar que el destino cabe, y solo entonces copiar.
            {
                let m = memoria.data(&caller);
                rango(
                    m,
                    salida,
                    objeto.datos.len(),
                    "WASM :: sys_object_datos desbordo la memoria lineal (salida)",
                )?;
            }
            let n = objeto.datos.len();
            let m = memoria.data_mut(&mut caller);
            m[salida as usize..salida as usize + n].copy_from_slice(&objeto.datos);
            Ok(n as i32)
        },
    )?;

    // --- CAPACIDAD 5 :: sys_object_hijo(hash, indice, salida) -> i32 ---
    // Recorre las aristas del DAG. Devuelve el NUMERO de hijos del objeto
    // `hash`; si `indice` es valido, ademas escribe el hash de ese hijo en
    // `salida`. CodigoError::Ausente si el objeto no existe,
    // CodigoError::AlmacenamientoFallo si el almacen fallo.
    enlazador.func_wrap(
        "renaser",
        "sys_object_hijo",
        |mut caller: Caller<'_, ContextoCapacidades>,
         hash_ptr: u32,
         indice: u32,
         salida: u32|
         -> Result<i32, Error> {
            let memoria = obtener_memoria(&caller)?;

            let hash = {
                let m = memoria.data(&caller);
                leer_hash(
                    m,
                    hash_ptr,
                    "WASM :: sys_object_hijo desbordo la memoria lineal (hash)",
                )?
            };

            let objeto = match crate::almacen::recuperar(&hash) {
                Ok(Some(objeto)) => objeto,
                Ok(None) => return Ok(CodigoError::Ausente.como_i32()),
                Err(_) => return Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            };
            let total = objeto.hijos.len();

            // Si el indice apunta a un hijo real, entregar su hash.
            if let Some(hijo) = objeto.hijos.get(indice as usize) {
                {
                    let m = memoria.data(&caller);
                    rango(
                        m,
                        salida,
                        32,
                        "WASM :: sys_object_hijo desbordo la memoria lineal (salida)",
                    )?;
                }
                let m = memoria.data_mut(&mut caller);
                m[salida as usize..salida as usize + 32].copy_from_slice(hijo);
            }
            Ok(total as i32)
        },
    )?;

    // --- CAPACIDAD 6 :: sys_object_raiz(salida) -> i32 ---
    // Escribe en `salida` el hash de la raiz del grafo. Devuelve 1 si hay
    // raiz, 0 si el grafo aun no tiene ninguna.
    enlazador.func_wrap(
        "renaser",
        "sys_object_raiz",
        |mut caller: Caller<'_, ContextoCapacidades>, salida: u32| -> Result<i32, Error> {
            let memoria = obtener_memoria(&caller)?;
            match crate::almacen::raiz() {
                Some(hash) => {
                    {
                        let m = memoria.data(&caller);
                        rango(
                            m,
                            salida,
                            32,
                            "WASM :: sys_object_raiz desbordo la memoria lineal (salida)",
                        )?;
                    }
                    let m = memoria.data_mut(&mut caller);
                    m[salida as usize..salida as usize + 32].copy_from_slice(&hash);
                    Ok(1)
                }
                None => Ok(CodigoError::Ok.como_i32()),
            }
        },
    )?;

    // --- CAPACIDAD 7 :: sys_object_fijar_raiz(hash) -> i32 ---
    // Corona el objeto `hash` como raiz del grafo. CodigoError::Ok si se logro,
    // CodigoError::AlmacenamientoFallo si el almacenamiento fallo.
    //
    // GATEADA por PERMISO_RAIZ: cambiar la raiz del grafo mueve el punto
    // de entrada que el resto del userspace lee. Solo apps explicitamente
    // habilitadas en el manifiesto pueden hacerlo; el resto, ni la ve.
    if permisos & PERMISO_RAIZ != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_object_fijar_raiz",
        |caller: Caller<'_, ContextoCapacidades>, hash_ptr: u32| -> Result<i32, Error> {
            let memoria = obtener_memoria(&caller)?;
            let hash = {
                let m = memoria.data(&caller);
                leer_hash(
                    m,
                    hash_ptr,
                    "WASM :: sys_object_fijar_raiz desbordo la memoria lineal (hash)",
                )?
            };
            match crate::almacen::fijar_raiz(hash) {
                Ok(()) => Ok(CodigoError::Ok.como_i32()),
                Err(_) => Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            }
        },
    )?;
    } // PERMISO_RAIZ

    // --- CAPACIDAD 7b :: sys_manifiesto_proponer(mf_ptr, mf_len) -> i32 ---
    // Reancla del MANIFIESTO con guardarrail criptografico (Fase 25). La app
    // entrega en su memoria lineal la forma `postcard` de un sobre
    // `ManifiestoFirmado` (manifiesto_hash + autor Ed25519 + firma). El kernel:
    //
    //   1. Decodifica el sobre — un payload truncado o ajeno cae con
    //      `CodigoError::Ausente` (no es un error de almacenamiento).
    //   2. Verifica la firma contra `claves::AGORA_PUBLIC_KEY_LOCAL`. Una
    //      llave ajena, un payload tampered o una firma forjada caen sin
    //      mover un solo byte del manifiesto.
    //   3. Comprueba que el `manifiesto_hash` referenciado YA existe en el
    //      grafo local — la red puede traer la propuesta, pero el manifiesto
    //      real ha de estar ingestado (via Akasha) antes de reanclar.
    //   4. Reanca el manifiesto vivo del kernel — una sola escritura del
    //      superbloque, atomica como cualquier `fijar_manifiesto`.
    //
    // GATEADA por PERMISO_RAIZ: misma autoridad que mueve la raiz del grafo.
    // Una app sin este permiso no puede ni nombrar la capacidad: el linker
    // ni siquiera registra el simbolo.
    //
    // CERO ALOCACION ADICIONAL: la verificacion `ed25519-compact` corre sobre
    // la pila; el sobre se deserializa con `take_from_bytes` que NO copia.
    if permisos & PERMISO_RAIZ != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_manifiesto_proponer",
        |caller: Caller<'_, ContextoCapacidades>,
         mf_ptr: u32,
         mf_len: u32|
         -> Result<i32, Error> {
            // Cota dura del sobre: 32 + 32 + 64 + preludio postcard < 256 B.
            // Acota tambien una llamada adversaria con mf_len absurdo, que
            // intentaria desbordar el rango.
            const MAX_MF: usize = 256;
            if (mf_len as usize) > MAX_MF {
                return Ok(CodigoError::CapacidadInsuficiente.como_i32());
            }
            let memoria = obtener_memoria(&caller)?;
            // Copiar el sobre a una pila local — sin tocar al asignador.
            let mut buf = [0u8; MAX_MF];
            let n = mf_len as usize;
            {
                let m = memoria.data(&caller);
                let crudo = rango(
                    m,
                    mf_ptr,
                    n,
                    "WASM :: sys_manifiesto_proponer desbordo la memoria lineal",
                )?;
                buf[..n].copy_from_slice(crudo);
            }
            let mf = match format::ManifiestoFirmado::deserializar(&buf[..n]) {
                Ok(mf) => mf,
                Err(_) => return Ok(CodigoError::Ausente.como_i32()),
            };
            // Verificacion criptografica. Sin firma valida, no hay reancla.
            if let Err(err) = crate::claves::verificar_manifiesto_firmado(&mf) {
                return Ok(err.como_i32());
            }
            // El manifiesto referenciado tiene que estar ingestado localmente.
            // Si la red trajo el sobre pero no el Manifiesto en si, mudanza
            // ha de pedirlo via sys_red_solicitar y reintentar este syscall
            // cuando el demuxer lo haya absorbido al grafo.
            match crate::almacen::recuperar(&mf.manifiesto_hash) {
                Ok(Some(_)) => {}
                Ok(None) => return Ok(CodigoError::Ausente.como_i32()),
                Err(_) => return Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            }
            // Reancla atomica del manifiesto: el superbloque queda apuntando
            // a la propuesta verificada. El proximo fotograma —y todo
            // arranque ulterior— veran el nuevo userspace.
            match crate::almacen::fijar_manifiesto(mf.manifiesto_hash) {
                Ok(()) => Ok(CodigoError::Ok.como_i32()),
                Err(_) => Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            }
        },
    )?;
    } // PERMISO_RAIZ

    // --- CAPACIDAD 8 :: sys_estado_cargar(salida, capacidad) -> i32 ---
    // Copia el estado persistido de ESTA app —el objeto que su `EntradaApp` del
    // manifiesto tiene anclado— en `salida`. Devuelve el numero de bytes
    // copiados, 0 si la app no tiene estado previo, -1 si el objeto anclado no
    // existe, -2 si `capacidad` no basta, -3 si el almacenamiento fallo.
    enlazador.func_wrap(
        "renaser",
        "sys_estado_cargar",
        |mut caller: Caller<'_, ContextoCapacidades>,
         salida: u32,
         capacidad: u32|
         -> Result<i32, Error> {
            let indice = caller.data().indice_app;
            // El hash del estado de esta app, segun el manifiesto vivo.
            let hash = match crate::manifiesto::estado_de(indice) {
                Some(hash) => hash,
                None => return Ok(CodigoError::Ok.como_i32()), // Sin estado previo.
            };
            let objeto = match crate::almacen::recuperar(&hash) {
                Ok(Some(objeto)) => objeto,
                Ok(None) => return Ok(CodigoError::Ausente.como_i32()),
                Err(_) => return Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            };
            if objeto.datos.len() > capacidad as usize {
                return Ok(CodigoError::CapacidadInsuficiente.como_i32());
            }

            let memoria = obtener_memoria(&caller)?;
            // Verificar que el destino cabe, y solo entonces copiar.
            {
                let m = memoria.data(&caller);
                rango(
                    m,
                    salida,
                    objeto.datos.len(),
                    "WASM :: sys_estado_cargar desbordo la memoria lineal (salida)",
                )?;
            }
            let n = objeto.datos.len();
            let m = memoria.data_mut(&mut caller);
            m[salida as usize..salida as usize + n].copy_from_slice(&objeto.datos);
            Ok(n as i32)
        },
    )?;

    // --- CAPACIDAD 9 :: sys_estado_guardar(datos, datos_len) -> i32 ---
    // Graba `datos` como el estado persistido de ESTA app: el kernel lo
    // almacena como un objeto del grafo y ancla su hash en la `EntradaApp` de
    // la app, re-grabando y re-anclando el manifiesto. El estado sobrevivira al
    // reinicio. Devuelve 0 si se logro, -3 si el almacenamiento fallo.
    enlazador.func_wrap(
        "renaser",
        "sys_estado_guardar",
        |caller: Caller<'_, ContextoCapacidades>,
         datos_ptr: u32,
         datos_len: u32|
         -> Result<i32, Error> {
            let indice = caller.data().indice_app;
            let memoria = obtener_memoria(&caller)?;
            // Leer el estado de la memoria lineal, con limites firmes.
            let datos = {
                let m = memoria.data(&caller);
                rango(
                    m,
                    datos_ptr,
                    datos_len as usize,
                    "WASM :: sys_estado_guardar desbordo la memoria lineal (datos)",
                )?
                .to_vec()
            };
            // Grabar el objeto de estado. Un fallo del almacen NO es culpa de
            // la app: se le devuelve CodigoError::AlmacenamientoFallo.
            let hash = match crate::almacen::almacenar(datos, alloc::vec::Vec::new()) {
                Ok(hash) => hash,
                Err(_) => return Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            };
            // Anclarlo: muta el manifiesto vivo, lo re-graba y lo re-ancla.
            match crate::manifiesto::fijar_estado(indice, hash) {
                Ok(()) => Ok(CodigoError::Ok.como_i32()),
                Err(_) => Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            }
        },
    )?;

    // --- CAPACIDAD 10 :: sys_tiempo_mono() -> u64 ---
    // El reloj MONOTONO del sistema, CONGELADO POR FOTOGRAMA. El kernel
    // tomo un snapshot de los milisegundos justo antes de cederle a esta
    // app su `tick`; cada llamada dentro del fotograma devuelve EL MISMO
    // valor. Si la app graba tres nodos del grafo en un `tick`, los tres
    // llevan el mismo indice temporal — determinismo total a la vista del
    // userspace. El reloj sigue corriendo en el host, pero la app no lo
    // ve correr: lo ve como una fotografia. POSIX permite que dos lineas
    // adyacentes de `gettimeofday` devuelvan valores distintos; aqui no.
    enlazador.func_wrap(
        "renaser",
        "sys_tiempo_mono",
        |caller: Caller<'_, ContextoCapacidades>| -> u64 {
            caller.data().tiempo_ms_fotograma
        },
    )?;

    // --- CAPACIDAD 11 :: sys_tono(frecuencia_hz) ---
    // Hace sonar la bocina del PC a `frecuencia_hz` (un 0 la silencia). La
    // bocina es un recurso UNICO y global: para que dos apps no se la disputen,
    // pertenece —como el teclado desde la Fase 8c— a la ventana ENFOCADA. Una
    // app sin foco puede pedir un tono; sencillamente, no se oye. Y cuando el
    // foco cambia, el compositor calla la bocina: la nueva dueña la reclamara
    // en su proximo fotograma si quiere sonar.
    //
    // GATEADA por PERMISO_ALTAVOZ: aunque la bocina ya esta gateada por
    // foco, el bit deja EXPLICITO que la app puede solicitar sonido.
    if permisos & PERMISO_ALTAVOZ != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_tono",
        |caller: Caller<'_, ContextoCapacidades>, frecuencia_hz: u32| {
            // Prioridad del kernel: mientras suena una nota agendada por el
            // sistema (acorde de bienvenida, repique al lanzar o cerrar una
            // app, bajo de desalojo), las llamadas de los apps se ignoran. El
            // kernel no se interrumpe a si mismo en mitad de su voz propia.
            if crate::drivers::altavoz::kernel_sonando() {
                return;
            }
            if crate::compositor::foco() == caller.data().indice_app {
                crate::drivers::altavoz::tono(frecuencia_hz);
            }
        },
    )?;
    } // PERMISO_ALTAVOZ

    // --- CAPACIDADES 12-15 (gateadas por PERMISO_RED) ---
    // Las cuatro capacidades de red (`sys_net_mac`, `sys_net_enviar`,
    // `sys_net_recibir`, `sys_red_solicitar`) viajan juntas: una app que
    // no declaro PERMISO_RED en su manifiesto NO ve ninguna de ellas. Sin
    // tabla que escalar; si necesitas tres y declaras una, no aprovecharas
    // un import — los cuatro simbolos quedan ausentes a la vez.
    if permisos & PERMISO_RED != 0 {

    // --- CAPACIDAD 12 :: sys_net_mac(salida) -> i32 ---
    // Copia los 6 bytes de la MAC de la tarjeta de red en `salida`.
    // CodigoError::Ok si la red esta montada; CodigoError::Ausente si no hay
    // tarjeta o aun no se monto.
    enlazador.func_wrap(
        "renaser",
        "sys_net_mac",
        |mut caller: Caller<'_, ContextoCapacidades>, salida: u32| -> Result<i32, Error> {
            let Some(mac) = crate::drivers::red::mac() else {
                return Ok(CodigoError::Ausente.como_i32());
            };
            let memoria = obtener_memoria(&caller)?;
            {
                let m = memoria.data(&caller);
                rango(m, salida, 6, "WASM :: sys_net_mac desbordo la memoria lineal")?;
            }
            let m = memoria.data_mut(&mut caller);
            m[salida as usize..salida as usize + 6].copy_from_slice(&mac);
            Ok(CodigoError::Ok.como_i32())
        },
    )?;

    // --- CAPACIDAD 13 :: sys_net_enviar(ptr, len) -> i32 ---
    // Envia un frame Ethernet crudo (cabecera + payload, sin CRC). El app
    // construye el frame entero en su memoria lineal. CodigoError::Ok si el
    // envio se entrego al dispositivo; CodigoError::EnvioFallo si fallo.
    enlazador.func_wrap(
        "renaser",
        "sys_net_enviar",
        |caller: Caller<'_, ContextoCapacidades>, ptr: u32, len: u32| -> Result<i32, Error> {
            let memoria = obtener_memoria(&caller)?;
            let datos = memoria.data(&caller);
            let frame = rango(
                datos,
                ptr,
                len as usize,
                "WASM :: sys_net_enviar desbordo la memoria lineal",
            )?;
            match crate::drivers::red::enviar(frame) {
                Ok(()) => Ok(CodigoError::Ok.como_i32()),
                Err(_) => Ok(CodigoError::EnvioFallo.como_i32()),
            }
        },
    )?;

    // --- CAPACIDAD 14 :: sys_net_recibir(salida, capacidad) -> i32 ---
    // Saca el siguiente frame de la cola del USUARIO y lo copia en `salida`.
    // Desde la Fase 20, esa cola la rellena el demultiplexor del kernel
    // (`akasha::drenar_y_demultiplexar`): los frames Akasha (`0x88B5` con
    // payload valido) se procesan en el nucleo y NO llegan aqui; el resto
    // del trafico —ARP, IPv4 de QEMU, futuros protocolos— si. Devuelve los
    // bytes copiados (>0), 0 si no hay frame pendiente, o -1 si no hay red
    // montada. La cola se vacia FIFO; si un app no llama nunca, los frames
    // mas antiguos se descartan al desbordar (ver `akasha::COLA_USUARIO`).
    enlazador.func_wrap(
        "renaser",
        "sys_net_recibir",
        |mut caller: Caller<'_, ContextoCapacidades>,
         salida: u32,
         capacidad: u32|
         -> Result<i32, Error> {
            if crate::drivers::red::mac().is_none() {
                return Ok(CodigoError::Ausente.como_i32());
            }
            // Bufer kernel-side donde la cola del usuario vuelca el frame antes
            // de copiarlo a la memoria lineal de la app. PRE-ALOCADO EN PILA y
            // de tamaño fijo: una rafaga RX de 60 fps que llamaba a `sys_net_recibir`
            // engendraba un `Vec` nuevo en el heap del kernel por fotograma —
            // ahora la operacion entera no toca al asignador.
            //
            // El techo se elige sobre el MTU clasico de Ethernet (1500 payload +
            // 18 cabecera/FCS) con un margen para frames marcadamente cortos;
            // un app que pida mas que esto recibe CapacidadInsuficiente — el
            // protocolo no acomoda jumbo frames y un buffer ilimitado abriria
            // la puerta a una alocacion adversaria desde el userspace.
            const MAX_FRAME_USERSPACE: usize = 2048;
            if (capacidad as usize) > MAX_FRAME_USERSPACE {
                return Ok(CodigoError::CapacidadInsuficiente.como_i32());
            }
            let memoria = obtener_memoria(&caller)?;
            // Verificar que el destino cabe ANTES de tocar la cola.
            {
                let m = memoria.data(&caller);
                rango(
                    m,
                    salida,
                    capacidad as usize,
                    "WASM :: sys_net_recibir desbordo la memoria lineal",
                )?;
            }
            let mut buf = [0u8; MAX_FRAME_USERSPACE];
            let cap = capacidad as usize;
            let n = crate::akasha::pop_usuario(&mut buf[..cap]);
            if n == 0 {
                return Ok(CodigoError::Ok.como_i32());
            }
            let m = memoria.data_mut(&mut caller);
            m[salida as usize..salida as usize + n].copy_from_slice(&buf[..n]);
            Ok(n as i32)
        },
    )?;

    // --- CAPACIDAD 15 :: sys_red_solicitar(hash_ptr) -> i32 ---
    // Difunde a la red `MensajeAkasha::SolicitarObjeto(hash)`. Si un par tiene
    // el objeto y responde, el demultiplexer del kernel lo absorbe al almacen
    // local async — el siguiente `sys_object_datos(hash, ...)` del app lo
    // encontrara. Patron tipico:
    //
    //   let n = sys_object_datos(&h, buf, BUF);
    //   if n == -1 { sys_red_solicitar(&h); /* reintentar en siguiente tick */ }
    //
    // Devuelve 0 si el frame se entrego al driver; -1 si no hay red montada o
    // el envio fallo. NO bloquea esperando respuesta — la espera la decide la
    // app entre fotogramas, no el kernel dentro del syscall.
    enlazador.func_wrap(
        "renaser",
        "sys_red_solicitar",
        |caller: Caller<'_, ContextoCapacidades>, hash_ptr: u32| -> Result<i32, Error> {
            let memoria = obtener_memoria(&caller)?;
            let hash = {
                let m = memoria.data(&caller);
                leer_hash(
                    m,
                    hash_ptr,
                    "WASM :: sys_red_solicitar desbordo la memoria lineal (hash)",
                )?
            };
            match crate::akasha::difundir_solicitud(hash) {
                Ok(()) => Ok(CodigoError::Ok.como_i32()),
                Err(()) => Ok(CodigoError::EnvioFallo.como_i32()),
            }
        },
    )?;

    } // PERMISO_RED

    // --- CAPACIDAD 16 :: sys_config_idioma() -> u32 ---
    // Lectura PASIVA del idioma activo: el kernel ya copio el valor en el
    // `ContextoCapacidades` antes de cederle el `tick` a la app. No hay sondeo
    // ni bloqueo; es leer un `u16` que ya esta en el contexto. La app es
    // ciega a la procedencia del numero —el manifiesto, el grafo, el defecto—:
    // solo sabe que en este fotograma renderiza con este idioma.
    enlazador.func_wrap(
        "renaser",
        "sys_config_idioma",
        |caller: Caller<'_, ContextoCapacidades>| -> u32 { caller.data().idioma as u32 },
    )?;

    // --- CAPACIDAD 17 :: sys_config_proponer(idioma, paleta_ptr) -> i32 ---
    // El UNICO camino para mutar la configuracion activa desde una app: la app
    // entrega `idioma` (un `u32` cuyo byte bajo es el codigo ISO 639-1
    // empaquetado) y un puntero a 20 bytes de paleta en su propia memoria
    // lineal. El kernel construye un `Configuracion` bien tipada, la graba
    // como un nodo NUEVO del grafo, calcula su hash, y reancla el manifiesto
    // al objeto recien creado en una sola transicion atomica. El SIGUIENTE
    // `tick` —de esta app y de todas las demas— pinta ya con la paleta nueva
    // y rotula con el idioma nuevo: frame-lock perfecto, sin estados mutables
    // globales: el "ahora" es siempre el hash al que apunta el manifiesto.
    //
    // Devuelve 0 al aplicar, -1 si el almacenamiento o el reancla fallaron,
    // -2 si la app no esta enfocada (la configuracion la gobierna el usuario,
    // y el usuario interactua con la ventana enfocada; una app sin foco no
    // se apropia de la experiencia del escritorio).
    //
    // GATEADA por PERMISO_CONFIG. La LECTURA del contexto (idioma + paleta)
    // siempre esta; cambiar la configuracion, no. Solo el "tonalero" y
    // futuras apps panel-de-control llevan ese bit en su manifiesto.
    if permisos & PERMISO_CONFIG != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_config_proponer",
        |caller: Caller<'_, ContextoCapacidades>,
         idioma: u32,
         paleta_ptr: u32|
         -> Result<i32, Error> {
            // Frontera de confianza local: solo la ventana enfocada gobierna
            // la experiencia. Una app en segundo plano recibe SinFoco; el
            // kernel no toca nada.
            if crate::compositor::foco() != caller.data().indice_app {
                return Ok(CodigoError::SinFoco.como_i32());
            }
            // Defensa-en-profundidad N.1 (Fase 27): validar que el codigo de
            // idioma sea un par ISO 639-1 lexico — dos letras ASCII. Un
            // codigo como `0x4040` (`@@`) cae con `PayloadInvalido` aqui
            // antes de que toque el grafo. El kernel jamas anclaria una
            // configuracion cuyo idioma fuera un sinsentido lexico.
            let idioma_lo = (idioma & 0xFF) as u8;
            let idioma_hi = ((idioma >> 8) & 0xFF) as u8;
            let es_letra = |b: u8| b.is_ascii_uppercase() || b.is_ascii_lowercase();
            if !(es_letra(idioma_lo) && es_letra(idioma_hi)) {
                return Ok(CodigoError::PayloadInvalido.como_i32());
            }
            let memoria = obtener_memoria(&caller)?;
            let datos = memoria.data(&caller);
            let paleta_bytes = rango(
                datos,
                paleta_ptr,
                20,
                "WASM :: sys_config_proponer desbordo la memoria lineal (paleta)",
            )?;
            let mut paleta = [0u8; 20];
            paleta.copy_from_slice(paleta_bytes);
            let nueva = format::Configuracion {
                version: format::VERSION_CONFIGURACION,
                idioma: idioma as u16,
                paleta,
            };
            match crate::manifiesto::fijar_configuracion(nueva) {
                Ok(_hash) => Ok(CodigoError::Ok.como_i32()),
                Err(_) => Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            }
        },
    )?;
    } // PERMISO_CONFIG

    // --- CAPACIDAD 18 :: sys_config_paleta(salida) -> i32 ---
    // Copia los 20 bytes de la paleta activa (cinco colores RGBA8) en la
    // memoria lineal de la app, en la direccion `salida`. La paleta vive en
    // el contexto (la inyecto el kernel al iniciar el `tick`): copiar veinte
    // bytes es la operacion entera, sin sondeos ni cuotas adicionales. Devuelve
    // 0 al copiar; abortar la app si el destino se sale de su memoria lineal —
    // la culpa es del modulo, como en cualquier otra capacidad de escritura.
    enlazador.func_wrap(
        "renaser",
        "sys_config_paleta",
        |mut caller: Caller<'_, ContextoCapacidades>, salida: u32| -> Result<i32, Error> {
            let paleta = caller.data().paleta;
            let memoria = obtener_memoria(&caller)?;
            {
                let m = memoria.data(&caller);
                rango(
                    m,
                    salida,
                    paleta.len(),
                    "WASM :: sys_config_paleta desbordo la memoria lineal",
                )?;
            }
            let m = memoria.data_mut(&mut caller);
            m[salida as usize..salida as usize + paleta.len()].copy_from_slice(&paleta);
            Ok(CodigoError::Ok.como_i32())
        },
    )?;

    Ok(())
}
