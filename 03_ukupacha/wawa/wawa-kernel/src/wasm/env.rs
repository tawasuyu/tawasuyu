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

fn enlazar_presentacion(
    enlazador: &mut Linker<ContextoCapacidades>,
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
    Ok(())
}

fn enlazar_grafo(
    enlazador: &mut Linker<ContextoCapacidades>,
    permisos: Permisos,
) -> Result<(), Error> {
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
    } // PERMISO_GRAFO_ESCRITURA (sys_object_put)

    // --- CAPACIDAD 3b :: sys_subsistema_registrar_ejecutable -----------------
    // sys_subsistema_registrar_ejecutable(ptr, len, salida_hash_ptr) -> i32
    //
    // La via PRIVILEGIADA para que el IDE materialice un modulo WebAssembly
    // (Fase 28). Es un sys_object_put con validacion semantica: antes de
    // tocar el grafo, el kernel comprueba que los primeros cuatro bytes
    // del payload son la firma magica de WebAssembly (`\0asm`). Un payload
    // sin la firma cae con `PayloadInvalido` y el grafo NO crece.
    //
    // La idea es enchufar el Hito 8 (binding inmutable bytecode-permisos):
    // el dia que una app firmada por el operador local empareje el HASH de
    // un ejecutable con un set de permisos, esta syscall sera la unica via
    // que un userspace pueda usar para INSCRIBIR un binario reciclable.
    //
    // GATEADA por PERMISO_GRAFO_ESCRITURA (misma autoridad que cualquier
    // mutacion del grafo) Y consume del contador `paginas_dma_en_vuelo`
    // de la app — el bytecode pesa, el bus DMA no es gratis—. El payload
    // se acota a 1 MiB (`format::MAX_OBJETO`) por la propia almacen.
    if permisos & PERMISO_GRAFO_ESCRITURA != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_subsistema_registrar_ejecutable",
        |mut caller: Caller<'_, ContextoCapacidades>,
         ptr: u32,
         len: u32,
         salida_hash_ptr: u32|
         -> Result<i32, Error> {
            // Back-pressure DMA, gemela de sys_object_put.
            if caller.data().paginas_dma_en_vuelo >= MAX_PAGINAS_DMA_PER_APP {
                return Ok(CodigoError::Saturado.como_i32());
            }
            caller.data_mut().paginas_dma_en_vuelo += 1;

            let memoria = obtener_memoria(&caller)?;
            // Lectura del payload con limites firmes; copia a Vec —el `to_vec`
            // es inevitable porque `almacenar` toma propiedad—.
            let payload = {
                let m = memoria.data(&caller);
                let bytes = rango(
                    m,
                    ptr,
                    len as usize,
                    "WASM :: sys_subsistema_registrar_ejecutable desbordo memoria (payload)",
                )?;
                bytes.to_vec()
            };

            // Validacion semantica: cuatro bytes magicos `\0asm`. Sin esto,
            // el grafo se podria llenar de basura no-WebAssembly bajo una
            // capacidad de "ejecutable" que en realidad solo lee texto.
            const WASM_MAGIA: [u8; 4] = [0x00, 0x61, 0x73, 0x6D];
            if payload.len() < 8 || payload[..4] != WASM_MAGIA {
                caller.data_mut().paginas_dma_en_vuelo -= 1;
                return Ok(CodigoError::PayloadInvalido.como_i32());
            }

            // Verificar que el destino del hash cabe ANTES de tocar el disco.
            {
                let m = memoria.data(&caller);
                rango(
                    m,
                    salida_hash_ptr,
                    32,
                    "WASM :: sys_subsistema_registrar_ejecutable desbordo memoria (salida)",
                )?;
            }

            let resultado = match crate::almacen::almacenar(payload, alloc::vec::Vec::new()) {
                Ok(hash) => {
                    let m = memoria.data_mut(&mut caller);
                    m[salida_hash_ptr as usize..salida_hash_ptr as usize + 32]
                        .copy_from_slice(&hash);
                    CodigoError::Ok
                }
                Err(_) => CodigoError::AlmacenamientoFallo,
            };
            caller.data_mut().paginas_dma_en_vuelo -= 1;
            Ok(resultado.como_i32())
        },
    )?;
    } // PERMISO_GRAFO_ESCRITURA

    // --- CAPACIDAD 3c :: sys_subsistema_registrar_ejecutable_v2 -------------
    // sys_subsistema_registrar_ejecutable_v2(ptr, len, padre_hash_ptr,
    //                                        salida_hash_ptr) -> i32
    //
    // EVOLUCION del ABI sin romper compatibilidad regresiva (Fase 31). La
    // syscall `v1` (ver mas arriba) sigue VIVA e INTACTA: los modulos del
    // userspace que la importan no perciben este cambio. La `v2` anade un
    // PARAMETRO MAS — un puntero a 32 bytes que apuntan al HASH del CODIGO
    // FUENTE que engendro este binario—. El kernel entrelaza ambos en el
    // grafo: el HASH_FUENTE se inscribe como el PRIMER HIJO LICITO del
    // nodo ejecutable. El binario deja de ser huerfano: arrastra un
    // CORDON UMBILICAL criptografico hacia su propia causa.
    //
    // GATEADA por PERMISO_GRAFO_ESCRITURA. Hereda back-pressure DMA y
    // validacion semantica (firma WASM) de la `v1`.
    if permisos & PERMISO_GRAFO_ESCRITURA != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_subsistema_registrar_ejecutable_v2",
        |mut caller: Caller<'_, ContextoCapacidades>,
         ptr: u32,
         len: u32,
         padre_hash_ptr: u32,
         salida_hash_ptr: u32|
         -> Result<i32, Error> {
            // Back-pressure DMA: misma cota que la v1; el bytecode pesa.
            if caller.data().paginas_dma_en_vuelo >= MAX_PAGINAS_DMA_PER_APP {
                return Ok(CodigoError::Saturado.como_i32());
            }
            caller.data_mut().paginas_dma_en_vuelo += 1;

            let memoria = obtener_memoria(&caller)?;

            // Lectura del payload con limites firmes.
            let payload = {
                let m = memoria.data(&caller);
                let bytes = rango(
                    m,
                    ptr,
                    len as usize,
                    "WASM :: sys_subsistema_registrar_ejecutable_v2 desbordo memoria (payload)",
                )?;
                bytes.to_vec()
            };

            // Lectura del hash del padre (32 bytes) — la causa del binario.
            let padre_hash = {
                let m = memoria.data(&caller);
                match leer_hash(
                    m,
                    padre_hash_ptr,
                    "WASM :: sys_subsistema_registrar_ejecutable_v2 desbordo memoria (padre)",
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        caller.data_mut().paginas_dma_en_vuelo -= 1;
                        return Err(e);
                    }
                }
            };

            // Validacion semantica: cuatro bytes magicos `\0asm`.
            const WASM_MAGIA: [u8; 4] = [0x00, 0x61, 0x73, 0x6D];
            if payload.len() < 8 || payload[..4] != WASM_MAGIA {
                caller.data_mut().paginas_dma_en_vuelo -= 1;
                return Ok(CodigoError::PayloadInvalido.como_i32());
            }

            // Verificar que el destino del hash cabe ANTES de tocar el disco.
            {
                let m = memoria.data(&caller);
                rango(
                    m,
                    salida_hash_ptr,
                    32,
                    "WASM :: sys_subsistema_registrar_ejecutable_v2 desbordo memoria (salida)",
                )?;
            }

            // LA ARISTA CAUSAL: el HASH_FUENTE se inscribe como el PRIMER
            // HIJO del nodo binario. El grafo queda con dos nodos enlazados
            // de forma indisoluble: causa (fuente) -> efecto (binario).
            let mut hijos: alloc::vec::Vec<Hash> = alloc::vec::Vec::with_capacity(1);
            hijos.push(padre_hash);

            let resultado = match crate::almacen::almacenar(payload, hijos) {
                Ok(hash) => {
                    let m = memoria.data_mut(&mut caller);
                    m[salida_hash_ptr as usize..salida_hash_ptr as usize + 32]
                        .copy_from_slice(&hash);
                    CodigoError::Ok
                }
                Err(_) => CodigoError::AlmacenamientoFallo,
            };
            caller.data_mut().paginas_dma_en_vuelo -= 1;
            Ok(resultado.como_i32())
        },
    )?;
    } // PERMISO_GRAFO_ESCRITURA (v2)

    // --- CAPACIDAD 3d :: sys_subsistema_ejecutar_dinamico --------------------
    // sys_subsistema_ejecutar_dinamico(binario_hash_ptr) -> i32
    //
    // EL CIERRE DEL BUCLE (Fase 32). Lee 32 bytes del hash; recupera el
    // payload del grafo; instancia una sub-jaula EFIMERA; invoca su export
    // `"run"` UNA SOLA VEZ con un techo estricto de combustible
    // (`FUEL_DINAMICO`); destruye la jaula. El i32 que devuelve `"run"`
    // (positivo o negativo) se PROPAGA a la app llamante como el retorno
    // de la syscall. Los codigos negativos reservados de `CodigoError`
    // (-1 a -7) NO colisionan con valores Forth tipicos porque la app que
    // llama compara antes contra los enumerados conocidos —y al usuario
    // se le rotula la causa en el panel GAMMA—.
    //
    // GATEADA por PERMISO_GRAFO_ESCRITURA + foco: solo la ventana enfocada
    // puede pedir un despacho dinamico (igual disciplina que `sys_tono` y
    // `sys_config_proponer`). El bit ya autoriza al IDE a escribir el
    // grafo; ejecutar un binario que el mismo emitio cae naturalmente
    // bajo la misma autoridad. El foco evita que una app en segundo plano
    // despache calculos a espaldas del usuario.
    //
    // BACK-PRESSURE DMA: el almacen::recuperar hace E/S; cuenta como una
    // pagina (idem `sys_object_datos`, que tambien la consume).
    if permisos & PERMISO_GRAFO_ESCRITURA != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_subsistema_ejecutar_dinamico",
        |mut caller: Caller<'_, ContextoCapacidades>,
         binario_hash_ptr: u32|
         -> Result<i32, Error> {
            // Solo la ventana enfocada despacha calculos.
            if crate::compositor::foco() != caller.data().indice_app {
                return Ok(CodigoError::SinFoco.como_i32());
            }
            if caller.data().paginas_dma_en_vuelo >= MAX_PAGINAS_DMA_PER_APP {
                return Ok(CodigoError::Saturado.como_i32());
            }
            caller.data_mut().paginas_dma_en_vuelo += 1;

            let memoria = obtener_memoria(&caller)?;
            let hash = {
                let m = memoria.data(&caller);
                match leer_hash(
                    m,
                    binario_hash_ptr,
                    "WASM :: sys_subsistema_ejecutar_dinamico desbordo memoria (hash)",
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        caller.data_mut().paginas_dma_en_vuelo -= 1;
                        return Err(e);
                    }
                }
            };

            // Recuperar el bytecode del grafo direccionado por contenido.
            let objeto = match crate::almacen::recuperar(&hash) {
                Ok(Some(objeto)) => objeto,
                Ok(None) => {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Ok(CodigoError::Ausente.como_i32());
                }
                Err(_) => {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Ok(CodigoError::AlmacenamientoFallo.como_i32());
                }
            };

            // Liberar la pagina ANTES de la ejecucion: la operacion del bus
            // ya termino. La sub-jaula que sigue no usa DMA del kernel —el
            // motor de wasmi corre puramente en CPU—.
            caller.data_mut().paginas_dma_en_vuelo -= 1;

            // Despachar. El retorno entero del binario sube TAL CUAL al
            // usuario; las fallas se traducen a `CodigoError` negativos
            // — el cuadro de mando del IDE distingue las dos clases con
            // la etiqueta que pinta en GAMMA, no por el numero a secas.
            match crate::wasm::ejecutar_dinamico(&objeto.datos) {
                Ok(retorno) => Ok(retorno),
                Err(crate::wasm::FallaApp::SinCombustible) => Ok(CodigoError::Saturado.como_i32()),
                Err(crate::wasm::FallaApp::SinMemoria) => Ok(CodigoError::CapacidadInsuficiente.como_i32()),
                Err(_) => Ok(CodigoError::PayloadInvalido.como_i32()),
            }
        },
    )?;
    } // PERMISO_GRAFO_ESCRITURA (dinamico)

    // --- CAPACIDAD 3d_v2 :: sys_subsistema_ejecutar_dinamico_v2 ------------
    // sys_subsistema_ejecutar_dinamico_v2(binario_hash_ptr, valor_entrada) -> i32
    //
    // FASE 40 :: la EVOLUCION del ABI dinamico. Idem `ejecutar_dinamico` pero
    // con un parametro `i32` que el host inyecta al sub-proceso si su firma
    // de `"run"` es `(i32) -> i32`. Para binarios legacy que solo declaran
    // `() -> i32`, el kernel ignora el parametro y los corre como antes —
    // compatibilidad regresiva total.
    //
    // El despacho polimorfico vive en `wasm::ejecutar_dinamico_v2`. Esta
    // syscall solo agrega el bridge: leer el hash, recuperar el binario,
    // delegar en la fn de wasm.
    //
    // GATEADA igual que la v1: PERMISO_GRAFO_ESCRITURA + foco, BACK-PRESSURE
    // DMA. Mismo techo de FUEL_DINAMICO y 1 MiB de RAM en la sub-jaula.
    if permisos & PERMISO_GRAFO_ESCRITURA != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_subsistema_ejecutar_dinamico_v2",
        |mut caller: Caller<'_, ContextoCapacidades>,
         binario_hash_ptr: u32,
         valor_entrada: i32|
         -> Result<i32, Error> {
            if crate::compositor::foco() != caller.data().indice_app {
                return Ok(CodigoError::SinFoco.como_i32());
            }
            if caller.data().paginas_dma_en_vuelo >= MAX_PAGINAS_DMA_PER_APP {
                return Ok(CodigoError::Saturado.como_i32());
            }
            caller.data_mut().paginas_dma_en_vuelo += 1;

            let memoria = obtener_memoria(&caller)?;
            let hash = {
                let m = memoria.data(&caller);
                match leer_hash(
                    m,
                    binario_hash_ptr,
                    "WASM :: sys_subsistema_ejecutar_dinamico_v2 desbordo memoria (hash)",
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        caller.data_mut().paginas_dma_en_vuelo -= 1;
                        return Err(e);
                    }
                }
            };

            // FASE 41 :: CRL — el binario solicitado puede estar proscrito
            // por la lista de revocacion estatica del kernel. Aborto
            // inmediato, antes de tocar el disco o gastar criptografia.
            if crate::almacen::esta_revocado(&hash) {
                caller.data_mut().paginas_dma_en_vuelo -= 1;
                return Ok(CodigoError::PayloadInvalido.como_i32());
            }

            let objeto = match crate::almacen::recuperar(&hash) {
                Ok(Some(o)) => o,
                Ok(None) => {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Ok(CodigoError::Ausente.como_i32());
                }
                Err(_) => {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Ok(CodigoError::AlmacenamientoFallo.como_i32());
                }
            };
            caller.data_mut().paginas_dma_en_vuelo -= 1;

            match crate::wasm::ejecutar_dinamico_v2(&objeto.datos, valor_entrada) {
                Ok(retorno) => Ok(retorno),
                Err(crate::wasm::FallaApp::SinCombustible) => Ok(CodigoError::Saturado.como_i32()),
                Err(crate::wasm::FallaApp::SinMemoria) => {
                    Ok(CodigoError::CapacidadInsuficiente.como_i32())
                }
                Err(_) => Ok(CodigoError::PayloadInvalido.como_i32()),
            }
        },
    )?;
    } // PERMISO_GRAFO_ESCRITURA (dinamico_v2)

    // --- CAPACIDAD 3e :: sys_cuaderno_anexar_celda ---------------------------
    // sys_cuaderno_anexar_celda(cuaderno_previo_hash_ptr,
    //                           fuente_hash_ptr, binario_hash_ptr,
    //                           retorno: i32, error_flag: u32,
    //                           id_sec: u32, salida_hash_ptr) -> i32
    //
    // EL HISTORIAL ACUMULATIVO DEL CUADERNO (Fase 47, Notebook DAG
    // Accumulator). Evoluciona `sys_cuaderno_registrar_celda` de la
    // Fase 33: el kernel ya no emite cuadernos huerfanos de UNA
    // celda. En su lugar:
    //
    //   1. Si `cuaderno_previo_hash_ptr` apunta a `[0; 32]`, arranca
    //      un cuaderno virgen con vector vacio.
    //   2. Si apunta a un hash real, recupera el nodo previo,
    //      deserializa su `Vec<CeldaWawa>` y lo asume como base.
    //   3. Ensambla la nueva `CeldaWawa` con los campos planos
    //      provistos por la app, hace `push` y reinscribe el
    //      vector COMPLETO como un nodo nuevo. Los hijos del DAG
    //      son: el cuaderno previo (arista ancestral, si Some),
    //      la fuente y el binario (si Some).
    //
    // El cuaderno se vuelve una bitacora forense profundamente
    // enlazada: cada nodo apunta a su predecesor por hash, formando
    // una cadena recorrible por el Walker en orden cronologico real.
    //
    // CONVENCION DE CAMPOS OPCIONALES (zero-alloc en el lado app):
    //   * `binario_hash` lleva [0; 32] cuando la app no produjo binario
    //     (compilacion fallida). El kernel detecta el patron all-zero
    //     y lo traduce a `binario_hash: None`.
    //   * `error_flag != 0` marca `marca_error: true` en la struct.
    //   * `ultimo_retorno` siempre se inscribe como `Some(retorno)`.
    //
    // LIMITE DURO :: el vector acumulado se topa contra
    // `MAX_CELDAS_ACUMULADAS` antes del `push`. Superarlo cortocircuita
    // con `Saturado(-6)` sin tocar el disco — protege la pila del kernel
    // y mantiene el techo presupuestal del MVP interactivo.
    //
    // GATEADA por PERMISO_GRAFO_ESCRITURA. Back-pressure DMA.
    if permisos & PERMISO_GRAFO_ESCRITURA != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_cuaderno_anexar_celda",
        |mut caller: Caller<'_, ContextoCapacidades>,
         cuaderno_previo_hash_ptr: u32,
         fuente_hash_ptr: u32,
         binario_hash_ptr: u32,
         retorno: i32,
         error_flag: u32,
         id_sec: u32,
         salida_hash_ptr: u32|
         -> Result<i32, Error> {
            if caller.data().paginas_dma_en_vuelo >= MAX_PAGINAS_DMA_PER_APP {
                return Ok(CodigoError::Saturado.como_i32());
            }
            caller.data_mut().paginas_dma_en_vuelo += 1;

            let memoria = obtener_memoria(&caller)?;
            // Leer los TRES hashes de la memoria lineal en un solo borrow.
            let (cuaderno_previo_bytes, fuente_hash, binario_hash_bytes) = {
                let m = memoria.data(&caller);
                let p = match leer_hash(
                    m,
                    cuaderno_previo_hash_ptr,
                    "WASM :: sys_cuaderno_anexar_celda desbordo memoria (previo)",
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        caller.data_mut().paginas_dma_en_vuelo -= 1;
                        return Err(e);
                    }
                };
                let f = match leer_hash(
                    m,
                    fuente_hash_ptr,
                    "WASM :: sys_cuaderno_anexar_celda desbordo memoria (fuente)",
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        caller.data_mut().paginas_dma_en_vuelo -= 1;
                        return Err(e);
                    }
                };
                let b = match leer_hash(
                    m,
                    binario_hash_ptr,
                    "WASM :: sys_cuaderno_anexar_celda desbordo memoria (binario)",
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        caller.data_mut().paginas_dma_en_vuelo -= 1;
                        return Err(e);
                    }
                };
                (p, f, b)
            };
            // Verificar que el destino del hash del cuaderno cabe ANTES de
            // tocar el disco. Un puntero invalido aborta sin escribir.
            {
                let m = memoria.data(&caller);
                if let Err(e) = rango(
                    m,
                    salida_hash_ptr,
                    32,
                    "WASM :: sys_cuaderno_anexar_celda desbordo memoria (salida)",
                ) {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Err(e);
                }
            }

            // CONVENCION DEL HASH NULO: un binario `[0; 32]` significa
            // "la celda no produjo binario ejecutable" y colapsa a
            // `Option::None` en la struct.
            let binario_hash: Option<Hash> = if binario_hash_bytes == [0u8; 32] {
                None
            } else {
                Some(binario_hash_bytes)
            };
            // Cuaderno previo nulo = arranque virgen del historial.
            let cuaderno_previo: Option<Hash> = if cuaderno_previo_bytes == [0u8; 32] {
                None
            } else {
                Some(cuaderno_previo_bytes)
            };

            // Recuperar el vector acumulado del cuaderno previo. Si el
            // hash no existe en el grafo lo tratamos como virgen — la
            // app pudo haber perdido referencia, pero no rompemos la
            // anexion. Si el almacen falla, propagamos el error
            // controlado sin tocar el disco.
            let mut celdas: alloc::vec::Vec<format::CeldaWawa> = match cuaderno_previo {
                None => alloc::vec::Vec::new(),
                Some(h) => match crate::almacen::recuperar(&h) {
                    Ok(Some(objeto)) => match format::deserializar_celdas(&objeto.datos) {
                        Ok(v) => v,
                        Err(_) => {
                            caller.data_mut().paginas_dma_en_vuelo -= 1;
                            return Ok(CodigoError::PayloadInvalido.como_i32());
                        }
                    },
                    Ok(None) => alloc::vec::Vec::new(),
                    Err(_) => {
                        caller.data_mut().paginas_dma_en_vuelo -= 1;
                        return Ok(CodigoError::AlmacenamientoFallo.como_i32());
                    }
                },
            };

            // LIMITE DURO :: techo pre-alocado para que el vector
            // acumulado no agote la pila del kernel ni se transforme
            // en una palanca para inflar el log. Si la app necesita
            // mas historial, debera compactar (fase futura).
            if celdas.len() >= MAX_CELDAS_ACUMULADAS {
                caller.data_mut().paginas_dma_en_vuelo -= 1;
                return Ok(CodigoError::Saturado.como_i32());
            }

            celdas.push(format::CeldaWawa {
                id_secuencial: id_sec,
                fuente_hash,
                binario_hash,
                ultimo_retorno: Some(retorno),
                marca_error: error_flag != 0,
            });

            let payload = match format::serializar_celdas(&celdas) {
                Ok(bytes) => bytes,
                Err(_) => {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Ok(CodigoError::AlmacenamientoFallo.como_i32());
                }
            };

            // Hijos del DAG: arista ancestral (cuaderno previo, si
            // Some), fuente (siempre), binario (si Some). Cose el
            // tejido criptografico completo del historial.
            let mut hijos: alloc::vec::Vec<Hash> = alloc::vec::Vec::with_capacity(3);
            if let Some(p) = cuaderno_previo {
                hijos.push(p);
            }
            hijos.push(fuente_hash);
            if let Some(b) = binario_hash {
                hijos.push(b);
            }

            let resultado = match crate::almacen::almacenar(payload, hijos) {
                Ok(hash) => {
                    let m = memoria.data_mut(&mut caller);
                    m[salida_hash_ptr as usize..salida_hash_ptr as usize + 32]
                        .copy_from_slice(&hash);
                    CodigoError::Ok
                }
                Err(_) => CodigoError::AlmacenamientoFallo,
            };
            caller.data_mut().paginas_dma_en_vuelo -= 1;
            Ok(resultado.como_i32())
        },
    )?;
    } // PERMISO_GRAFO_ESCRITURA (cuaderno_anexar)

    // --- CAPACIDAD 3e_walker :: sys_cuaderno_leer_celda --------------------
    // sys_cuaderno_leer_celda(cuaderno_hash_ptr, indice_lineal, salida_celda_ptr) -> i32
    //
    // EL EXPLORADOR DEL GRAFO INMUTABLE (Fase 44 :: Notebook Walker). La
    // app entrega el hash de un nodo cuaderno y un indice lineal; el
    // kernel deserializa el `Vec<CeldaWawa>` del payload via
    // `format::deserializar_celdas`, busca la celda en ese indice y la
    // copia a la memoria lineal de la app en formato ABI plano de
    // 73 bytes (sin postcard del lado app — el modulo WASM puede leer
    // los campos por offset sin importar la crate `format`):
    //
    //   Offset Size Campo
    //   0      1    flags  (bit 0 = marca_error,
    //                       bit 1 = has_binario,
    //                       bit 2 = has_retorno)
    //   1      4    id_secuencial    (u32 LE)
    //   5      32   fuente_hash
    //   37     32   binario_hash     (ceros si !has_binario)
    //   69     4    ultimo_retorno   (i32 LE, 0 si !has_retorno)
    //
    // Indices fuera de rango devuelven `Ausente(-1)` — el walker rompe
    // su lazo limpiamente. Hash no encontrado en el grafo: `Ausente`.
    // Payload que no deserializa como `Vec<CeldaWawa>`: `PayloadInvalido`.
    //
    // GATEADA por PERMISO_GRAFO_ESCRITURA + foco. Back-pressure DMA.
    //
    // ZERO-ALLOC EN EL HOST: la deserializacion via postcard usa el
    // asignador del kernel para construir el Vec — eso ya existia en
    // la version anterior (registrar_celda). El WALK en si no agrega
    // alocaciones nuevas; lee y libera el Vec en el mismo stack frame.
    if permisos & PERMISO_GRAFO_ESCRITURA != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_cuaderno_leer_celda",
        |mut caller: Caller<'_, ContextoCapacidades>,
         cuaderno_hash_ptr: u32,
         indice_lineal: u32,
         salida_celda_ptr: u32|
         -> Result<i32, Error> {
            if crate::compositor::foco() != caller.data().indice_app {
                return Ok(CodigoError::SinFoco.como_i32());
            }
            if caller.data().paginas_dma_en_vuelo >= MAX_PAGINAS_DMA_PER_APP {
                return Ok(CodigoError::Saturado.como_i32());
            }
            caller.data_mut().paginas_dma_en_vuelo += 1;

            let memoria = obtener_memoria(&caller)?;
            let hash = {
                let m = memoria.data(&caller);
                match leer_hash(
                    m,
                    cuaderno_hash_ptr,
                    "WASM :: sys_cuaderno_leer_celda desbordo memoria (hash)",
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        caller.data_mut().paginas_dma_en_vuelo -= 1;
                        return Err(e);
                    }
                }
            };
            // Verificar que el destino de 73 B cabe en la memoria lineal
            // ANTES de tocar el disco. Un puntero invalido aborta limpio.
            {
                let m = memoria.data(&caller);
                if let Err(e) = rango(
                    m,
                    salida_celda_ptr,
                    73,
                    "WASM :: sys_cuaderno_leer_celda desbordo memoria (salida)",
                ) {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Err(e);
                }
            }

            // Recuperar el nodo del grafo.
            let objeto = match crate::almacen::recuperar(&hash) {
                Ok(Some(o)) => o,
                Ok(None) => {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Ok(CodigoError::Ausente.como_i32());
                }
                Err(_) => {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Ok(CodigoError::AlmacenamientoFallo.como_i32());
                }
            };
            // Deserializar el payload como Vec<CeldaWawa>.
            let celdas = match format::deserializar_celdas(&objeto.datos) {
                Ok(v) => v,
                Err(_) => {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Ok(CodigoError::PayloadInvalido.como_i32());
                }
            };
            // Indice fuera de rango: fin del cuaderno. El walker lo
            // interpreta como condicion de parada.
            let celda = match celdas.get(indice_lineal as usize) {
                Some(c) => c.clone(),
                None => {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Ok(CodigoError::Ausente.como_i32());
                }
            };

            // Construir el frame de 73 bytes en pila y volcarlo.
            let mut frame = [0u8; 73];
            let has_binario = celda.binario_hash.is_some();
            let has_retorno = celda.ultimo_retorno.is_some();
            frame[0] = (celda.marca_error as u8)
                | ((has_binario as u8) << 1)
                | ((has_retorno as u8) << 2);
            frame[1..5].copy_from_slice(&celda.id_secuencial.to_le_bytes());
            frame[5..37].copy_from_slice(&celda.fuente_hash);
            if let Some(b) = celda.binario_hash {
                frame[37..69].copy_from_slice(&b);
            }
            // Si !has_binario, los bytes 37..69 quedan en 0 (init).
            if let Some(r) = celda.ultimo_retorno {
                frame[69..73].copy_from_slice(&r.to_le_bytes());
            }
            // Si !has_retorno, los bytes 69..73 quedan en 0 (init).

            let m = memoria.data_mut(&mut caller);
            m[salida_celda_ptr as usize..salida_celda_ptr as usize + 73]
                .copy_from_slice(&frame);
            caller.data_mut().paginas_dma_en_vuelo -= 1;
            Ok(CodigoError::Ok.como_i32())
        },
    )?;
    } // PERMISO_GRAFO_ESCRITURA (cuaderno_leer_celda)

    // --- CAPACIDAD 3f :: sys_subsistema_vincular_macro -----------------------
    // sys_subsistema_vincular_macro(binario_hash_ptr, salida_info_ptr) -> i32
    //
    // EL PUENTE INTER-APP (Fase 36 :: Cross-App Semantic Bridge). Una app
    // —el cuaderno (`apps/pluma`), por ejemplo— le pasa al kernel el hash
    // de un binario emitido en OTRA pestaña (ayer, hace un mes, por el
    // IDE viejo o el que sea) y obtiene a cambio un PARTE de inspeccion:
    //
    //   * Byte 0 = 0x01 si el nodo existe en el grafo, contiene la magia
    //     `\0asm` Y expone una funcion `"run"` en sus exports — el binario
    //     queda "vinculado" como macro lista para que la app la dispare via
    //     `sys_subsistema_ejecutar_dinamico` sin recompilar nada.
    //     Byte 0 = 0x00 si CUALQUIERA de las pre-condiciones falla.
    //   * Bytes 1..4 = tamaño en BLOQUES DE 256 BYTES del binario, LE u24.
    //     Acota cuanto va a pesar el `sys_subsistema_ejecutar_dinamico`
    //     posterior: la app puede negarse a importar macros gigantes.
    //
    // INSPECCION SIN INSTANCIAR. `Module::new` parsea y valida el modulo
    // (magia + secciones + tabla de tipos) pero NO crea Store ni reserva
    // memoria lineal. Solo cuando la app dispare la macro con
    // `sys_subsistema_ejecutar_dinamico` se levanta una sub-jaula efimera
    // con su techo de FUEL_DINAMICO. La inspeccion es barata; la ejecucion
    // sigue gateada igual que siempre.
    //
    // GATEADA por PERMISO_GRAFO_ESCRITURA + FOCO (misma autoridad que
    // ejecutar_dinamico, porque el resultado de inspeccionar se usa para
    // disparar la macro). Hereda back-pressure DMA: la operacion lee del
    // disco (sectores del log), cuenta como una pagina.
    if permisos & PERMISO_GRAFO_ESCRITURA != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_subsistema_vincular_macro",
        |mut caller: Caller<'_, ContextoCapacidades>,
         binario_hash_ptr: u32,
         salida_info_ptr: u32|
         -> Result<i32, Error> {
            if crate::compositor::foco() != caller.data().indice_app {
                return Ok(CodigoError::SinFoco.como_i32());
            }
            if caller.data().paginas_dma_en_vuelo >= MAX_PAGINAS_DMA_PER_APP {
                return Ok(CodigoError::Saturado.como_i32());
            }
            caller.data_mut().paginas_dma_en_vuelo += 1;

            let memoria = obtener_memoria(&caller)?;
            let hash = {
                let m = memoria.data(&caller);
                match leer_hash(
                    m,
                    binario_hash_ptr,
                    "WASM :: sys_subsistema_vincular_macro desbordo memoria (hash)",
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        caller.data_mut().paginas_dma_en_vuelo -= 1;
                        return Err(e);
                    }
                }
            };
            // Verificar que la salida cabe ANTES de tocar el disco.
            {
                let m = memoria.data(&caller);
                if let Err(e) = rango(
                    m,
                    salida_info_ptr,
                    4,
                    "WASM :: sys_subsistema_vincular_macro desbordo memoria (salida)",
                ) {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Err(e);
                }
            }

            let objeto = match crate::almacen::recuperar(&hash) {
                Ok(Some(o)) => o,
                Ok(None) => {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Ok(CodigoError::Ausente.como_i32());
                }
                Err(_) => {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Ok(CodigoError::AlmacenamientoFallo.como_i32());
                }
            };
            // La operacion de disco termino — liberar la pagina DMA aqui.
            caller.data_mut().paginas_dma_en_vuelo -= 1;

            // Verificacion semantica: magia WASM + parsear modulo + exigir
            // export `"run"`. Si cualquier paso falla, byte 0 = 0x00 y
            // salimos con Ok (no es error del syscall, es "vinculacion
            // rechazada con dictamen estructurado").
            const WASM_MAGIA: [u8; 4] = [0x00, 0x61, 0x73, 0x6D];
            let valido = objeto.datos.len() >= 8 && objeto.datos[..4] == WASM_MAGIA && {
                // Parseo wasmi sin instanciar — barato, sin Store.
                let mut config = wasmi::Config::default();
                config.compilation_mode(wasmi::CompilationMode::Eager);
                let motor = wasmi::Engine::new(&config);
                match wasmi::Module::new(&motor, &objeto.datos[..]) {
                    Ok(modulo) => modulo.exports().any(|e| e.name() == "run"),
                    Err(_) => false,
                }
            };

            // Tamaño en bloques de 256 B (ceil). MAX_OBJETO = 1 MiB =>
            // 4096 bloques => 0x1000, cabe holgado en 24 bits LE.
            let bloques = (objeto.datos.len() + 255) / 256;
            let bloques = bloques.min(0xFF_FFFF) as u32;

            let m = memoria.data_mut(&mut caller);
            let off = salida_info_ptr as usize;
            m[off] = if valido { 0x01 } else { 0x00 };
            m[off + 1] = (bloques & 0xFF) as u8;
            m[off + 2] = ((bloques >> 8) & 0xFF) as u8;
            m[off + 3] = ((bloques >> 16) & 0xFF) as u8;
            Ok(CodigoError::Ok.como_i32())
        },
    )?;
    } // PERMISO_GRAFO_ESCRITURA (vincular_macro)

    // --- CAPACIDAD 3g :: sys_cuaderno_firmar_y_anclar -----------------------
    // sys_cuaderno_firmar_y_anclar(cuaderno_firmado_ptr) -> i32
    //
    // LA FIRMA DEL TEJIDO CELULAR (Fase 37 :: Soberania Criptografica).
    // La app entrega un sobre `CuadernoFirmado` (32 + 32 + 64 = 128 B
    // crudos + preludio postcard) ya producido fuera de la jaula
    // —tipicamente por `wawactl` con la clave privada del operador, o
    // por una futura clave de sesion del kernel—. El kernel:
    //
    //   1. Copia el sobre a una pila estatica de 256 B (zero-alloc).
    //   2. Lo deserializa con `CuadernoFirmado::deserializar` —si los
    //      bytes no parsean, cae con `Ausente`—.
    //   3. Verifica criptograficamente via `claves::verificar_cuaderno_firmado`:
    //      autor ajeno -> `CapacidadInsuficiente`; firma forjada o
    //      tampered -> `AlmacenamientoFallo`.
    //   4. Si la matematica es licita, ANCLA el cuaderno como nueva
    //      raiz del grafo userspace via `almacen::fijar_raiz`. Esta
    //      operacion ES una escritura atomica del superbloque
    //      (sector 0); el sistema "ve" el cuaderno soberano desde el
    //      proximo fotograma.
    //
    // Notese que el chequeo de PERMISO_RAIZ se SALTA aqui: la
    // autoridad ya no viene de un bit del manifiesto sino de la firma
    // Ed25519 verificada en Ring 0. Una app sin PERMISO_RAIZ pero con
    // un sobre legitimo del operador local puede mover la raiz.
    //
    // GATEADA por PERMISO_GRAFO_ESCRITURA + foco: la app que invoca el
    // anclaje debe poseer la autoridad de escritura del grafo y ser
    // la ventana enfocada por el usuario. El bit es necesario pero
    // no suficiente — sin firma valida, el syscall no mueve un byte
    // del superbloque.
    //
    // ZERO-ALLOC + NO PANICOS: la deserializacion y la criptografia
    // viven en la pila. Un sobre adversario malformado, oversized o
    // con bytes maliciosos cae por `Result` lineal hasta el `Ok(i32)`;
    // el kernel jamas levanta `panic!`.
    if permisos & PERMISO_GRAFO_ESCRITURA != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_cuaderno_firmar_y_anclar",
        |caller: Caller<'_, ContextoCapacidades>,
         cuaderno_firmado_ptr: u32|
         -> Result<i32, Error> {
            if crate::compositor::foco() != caller.data().indice_app {
                return Ok(CodigoError::SinFoco.como_i32());
            }
            // Cota dura del sobre: 32 + 32 + 64 + preludio postcard < 256 B.
            // Acota tambien una llamada adversaria con un len absurdo que
            // pretendiera desbordar la pila.
            const MAX_CF: usize = 256;
            let memoria = obtener_memoria(&caller)?;
            // Copiar el sobre a una pila local — sin tocar al asignador
            // del kernel. `MAX_CF` es el limite duro: si la app pide leer
            // mas alla, el `rango` deniega antes de tocar la memoria.
            let mut buf = [0u8; MAX_CF];
            {
                let m = memoria.data(&caller);
                let crudo = rango(
                    m,
                    cuaderno_firmado_ptr,
                    MAX_CF,
                    "WASM :: sys_cuaderno_firmar_y_anclar desbordo memoria",
                )?;
                buf.copy_from_slice(crudo);
            }
            let cf = match format::CuadernoFirmado::deserializar(&buf) {
                Ok(cf) => cf,
                Err(_) => return Ok(CodigoError::Ausente.como_i32()),
            };
            // FASE 41 :: CRL — un cuaderno cuyo hash este en la lista de
            // revocacion del kernel se rechaza ANTES de tocar la
            // criptografia, aunque la firma sea matematicamente perfecta.
            // El operador retiro la confianza despues del sellado original;
            // el direccionamiento por contenido conserva el sello pero el
            // anillo soberano lo repudia.
            if crate::almacen::esta_revocado(&cf.cuaderno_raiz_hash) {
                return Ok(CodigoError::PayloadInvalido.como_i32());
            }
            // Verificacion criptografica. Sin firma valida no hay anclaje.
            if let Err(err) = crate::claves::verificar_cuaderno_firmado(&cf) {
                return Ok(err.como_i32());
            }
            // Defensa-en-profundidad: el cuaderno referenciado tiene que
            // estar ingestado localmente. Sin esto, un peer hostil podria
            // anunciar un hash que NUNCA tuvo payload — y el sistema lo
            // aceptaria como raiz solo porque la firma cuadra. El
            // direccionamiento por contenido exige que el bytes esten.
            match crate::almacen::recuperar(&cf.cuaderno_raiz_hash) {
                Ok(Some(_)) => {}
                Ok(None) => return Ok(CodigoError::Ausente.como_i32()),
                Err(_) => return Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            }
            // Anclaje atomico: superbloque queda apuntando al cuaderno
            // soberano firmado. El proximo fotograma —y todo arranque
            // ulterior hasta que otra firma valida proponga otra raiz—
            // veran este cuaderno.
            match crate::almacen::fijar_raiz(cf.cuaderno_raiz_hash) {
                Ok(()) => Ok(CodigoError::Ok.como_i32()),
                Err(_) => Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            }
        },
    )?;
    } // PERMISO_GRAFO_ESCRITURA (cuaderno_firmar_y_anclar)

    // --- CAPACIDAD 3h :: sys_cuaderno_solicitar_firma_host ------------------
    // sys_cuaderno_solicitar_firma_host(hash_ptr, salida_firma_ptr) -> i32
    //
    // EL CANAL DEL FIRMADOR EXTERNO (Fase 38/49). El kernel preserva la
    // ley inmutable de la Fase 25 —jamas firma desde Ring 0; solo
    // verifica— y delega el sellado criptografico en el operador del
    // host (`wawactl` o un HSM futuro). Esta syscall es el cordon
    // umbilical limpio entre Wawa y el firmador:
    //
    //   1. La app entrega los 32 bytes del hash del cuaderno a firmar.
    //   2. El kernel emite la baliza estructurada compacta de la
    //      Fase 49: 17 bytes de prefijo `wawactl::sign_pci::` + 32
    //      bytes RAW del hash = 49 bytes BINARIOS. El transporte es la
    //      consola paravirtualizada de VirtIO sobre PCI (driver
    //      `consola_virtio`); si el firmware no expuso un virtconsole,
    //      el kernel cae al UART de COM1 (Fase 38) sin alterar el
    //      contrato del Userspace.
    //   3. El kernel intenta leer 65 bytes del ring RX (rellenado por
    //      el demonio `wawactl daemon-firma` a traves de la consola
    //      VirtIO o el PTY de COM1, segun el transporte vivo).
    //   4. Si los 65 bytes ya estan completos en el ring, los escribe
    //      en `salida_firma_ptr` y devuelve `Ok(0)`. Si todavia no,
    //      devuelve `Saturado (-6)` — la app re-llama en el proximo tick.
    //
    // Para que el reintento no inunde el host con peticiones duplicadas,
    // el kernel recuerda el hash pendiente; mientras la app vuelva a
    // pedir el mismo hash, el prefijo se emite UNA SOLA VEZ. Un hash
    // distinto se considera "nueva solicitud" y vuelve a emitir.
    //
    // GATEADA por PERMISO_GRAFO_ESCRITURA + foco. Back-pressure DMA: la
    // operacion no toca el bus virtio-blk, pero contamos una pagina por
    // simetria con las otras syscalls de cuaderno — la cuota se reinicia
    // cada tic y el reintento no la satura.
    //
    // ZERO-ALLOC EN EL CAMINO CALIENTE: la baliza de 49 B vive en un
    // buffer en pila; el ring RX es un array global de 256 B en cada
    // transporte. El cambio Fase 38 -> Fase 49 ahorra 36 B por solicitud
    // (sin hex-encoding, sin newline) y multiplica la velocidad por
    // ordenes de magnitud (115200 baud -> bus PCI nativo).
    if permisos & PERMISO_GRAFO_ESCRITURA != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_cuaderno_solicitar_firma_host",
        |mut caller: Caller<'_, ContextoCapacidades>,
         hash_ptr: u32,
         salida_firma_ptr: u32|
         -> Result<i32, Error> {
            if crate::compositor::foco() != caller.data().indice_app {
                return Ok(CodigoError::SinFoco.como_i32());
            }
            if caller.data().paginas_dma_en_vuelo >= MAX_PAGINAS_DMA_PER_APP {
                return Ok(CodigoError::Saturado.como_i32());
            }
            caller.data_mut().paginas_dma_en_vuelo += 1;

            let memoria = obtener_memoria(&caller)?;
            let hash = {
                let m = memoria.data(&caller);
                match leer_hash(
                    m,
                    hash_ptr,
                    "WASM :: sys_cuaderno_solicitar_firma_host desbordo memoria (hash)",
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        caller.data_mut().paginas_dma_en_vuelo -= 1;
                        return Err(e);
                    }
                }
            };
            // FASE 42 :: la salida ahora son 65 B (1 slot + 64 firma).
            // Verificar que el rango completo cabe ANTES de tocar el bus.
            {
                let m = memoria.data(&caller);
                if let Err(e) = rango(
                    m,
                    salida_firma_ptr,
                    65,
                    "WASM :: sys_cuaderno_solicitar_firma_host desbordo memoria (firma+slot)",
                ) {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Err(e);
                }
            }

            // De-duplicacion de la solicitud: emitimos el prefijo solo si
            // el hash pendiente cambio. Asi, un loop de la app reintentando
            // cada tick no inunda al host con sign_requests duplicadas.
            // El estado vive en un Mutex spin —el reactor cooperativo no
            // se contiende—. El acumulador de 65 B (slot + firma) y el
            // ultimo hash viajan acoplados para que un cambio de solicitud
            // reinicie todo el camino limpio.
            use spin::Mutex;
            static ULTIMO_HASH: Mutex<Option<crate::almacen::Hash>> = Mutex::new(None);
            // FASE 42 :: 65 B = byte 0 (slot 0/1/2) + bytes 1..65 (firma).
            static ACUMULADOR: Mutex<([u8; 65], usize)> = Mutex::new(([0; 65], 0));
            let emitir = {
                let mut slot = ULTIMO_HASH.lock();
                let cambio = slot.as_ref() != Some(&hash);
                if cambio {
                    *slot = Some(hash);
                }
                cambio
            };
            // FASE 49 :: seleccion de transporte. Si el virtio-console
            // se monto durante el boot, todo el dialogo (emision +
            // drenado) viaja por PCI; si no, caemos al UART de COM1
            // (Fase 38). La decision es por solicitud — un transporte
            // que cambie en caliente seria un bug del firmware, no
            // un caso que la app deba contemplar.
            let usar_virtio = crate::drivers::consola_virtio::montada();

            if emitir {
                // FASE 39 :: solicitud nueva. Limpiamos el ring de RX para
                // descartar bytes huerfanos de una solicitud anterior
                // abortada (el demonio rechazo, timeout, etc.) Y reseteamos
                // el acumulador de 65 bytes — el siguiente byte que entre
                // sera el byte 0 (slot id) de la nueva respuesta.
                if usar_virtio {
                    crate::drivers::consola_virtio::vaciar_input();
                } else {
                    crate::drivers::serial::vaciar_input();
                }
                let mut acc = ACUMULADOR.lock();
                acc.0 = [0; 65];
                acc.1 = 0;
                drop(acc);

                // FASE 49 :: baliza estructurada compacta. 17 bytes de
                // prefijo + 32 bytes RAW del hash = 49 bytes BINARIOS.
                // Sin hex-encoding, sin newline — el parser del demonio
                // mide por longitud fija. Cabe holgado en pila.
                let mut frame = [0u8; 64];
                let prefijo = b"wawactl::sign_pci::";
                frame[..prefijo.len()].copy_from_slice(prefijo);
                let n = prefijo.len();
                frame[n..n + 32].copy_from_slice(&hash);
                let total = n + 32;
                if usar_virtio {
                    crate::drivers::consola_virtio::escribir(&frame[..total]);
                } else {
                    crate::drivers::serial::escribir(&frame[..total]);
                }
            }

            // Drenar lo que haya llegado del host al ring interno y luego
            // intentar leer 65 B (slot + firma). Si todavia faltan, la
            // app reintenta en el proximo tic.
            let mut frame = [0u8; 65];
            let leidos = if usar_virtio {
                crate::drivers::consola_virtio::drenar_input();
                crate::drivers::consola_virtio::leer_disponible(&mut frame)
            } else {
                crate::drivers::serial::drenar_input();
                crate::drivers::serial::leer_disponible(&mut frame)
            };

            if leidos < 65 {
                // Devolvemos los bytes parciales al acumulador estatico
                // declarado arriba — el ring no tiene push_front, asi que
                // conservamos los bytes parciales en `ACUMULADOR` hasta
                // juntar los 65 a traves de multiples tics.
                let mut acc = ACUMULADOR.lock();
                let (ref mut buf, ref mut llenos) = *acc;
                let cap = (65 - *llenos).min(leidos);
                for i in 0..cap {
                    buf[*llenos + i] = frame[i];
                }
                *llenos += cap;
                if *llenos < 65 {
                    caller.data_mut().paginas_dma_en_vuelo -= 1;
                    return Ok(CodigoError::Saturado.como_i32());
                }
                // Tenemos los 65 bytes acumulados ahora; copiarlos a la
                // memoria del modulo + reset del acumulador.
                let frame_total = *buf;
                *buf = [0; 65];
                *llenos = 0;
                drop(acc);
                let m = memoria.data_mut(&mut caller);
                m[salida_firma_ptr as usize..salida_firma_ptr as usize + 65]
                    .copy_from_slice(&frame_total);
                // Reset del hash pendiente — proxima solicitud volvera a emitir.
                *ULTIMO_HASH.lock() = None;
                caller.data_mut().paginas_dma_en_vuelo -= 1;
                return Ok(CodigoError::Ok.como_i32());
            }

            // Llegaron los 65 bytes de un golpe — caso ideal.
            let m = memoria.data_mut(&mut caller);
            m[salida_firma_ptr as usize..salida_firma_ptr as usize + 65]
                .copy_from_slice(&frame);
            *ULTIMO_HASH.lock() = None;
            caller.data_mut().paginas_dma_en_vuelo -= 1;
            Ok(CodigoError::Ok.como_i32())
        },
    )?;
    } // PERMISO_GRAFO_ESCRITURA (solicitar_firma_host)
    Ok(())
}

fn enlazar_objeto(
    enlazador: &mut Linker<ContextoCapacidades>,
) -> Result<(), Error> {
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
    Ok(())
}

fn enlazar_raiz_canal(
    enlazador: &mut Linker<ContextoCapacidades>,
    permisos: Permisos,
) -> Result<(), Error> {
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
            // Ultima compuerta: el manifiesto debe ser INSTANCIABLE —cada
            // bytecode presente y WASM cargable con el ABI de fotograma—. Un
            // sobre firmado por el anillo pero que apunte a un `.wasm` corrupto
            // (o cuya cascada del DAG aun no trajo todos los bytecodes)
            // ladrillaria el proximo arranque; lo rechazamos sin mover la raiz.
            if let Err(err) = crate::wasm::validar_manifiesto_instanciable(&mf.manifiesto_hash) {
                return Ok(err.como_i32());
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

    // --- CAPACIDAD 7d :: sys_canal_aceptar(raiz_ptr) -> i32 ---
    // Fase 64 :: ACEPTA el ultimo `AnunciarCanal` recibido por Akasha y reancla
    // el manifiesto a su raiz — la cara VIVA de `sys_manifiesto_proponer`. La
    // app `mudanza` la invoca cuando el operador confirma una propuesta que vio
    // por `sys_canal_anuncio`. El argumento es el hash de 32 B de la raiz que el
    // operador ACEPTO; el kernel:
    //
    //   1. Lee la raiz aceptada de la memoria lineal de la app.
    //   2. Toma el anuncio retenido (`akasha::ultimo_anuncio`). Si no hay, o si
    //      su raiz no casa con la aceptada —un anuncio nuevo lo reemplazo entre
    //      "mostrar" y "aceptar"—, cae con `Ausente` SIN tocar nada (cierra el
    //      TOCTOU: el operador acepta EXACTAMENTE lo que vio).
    //   3. Lee el objeto `Canal` del grafo para obtener su `nombre`. Confiar en
    //      ese nombre es seguro: la firma del anuncio LO CUBRE (paso 4).
    //   4. Verificacion SOBERANA: anillo `AGORA_AUTH_RING` + firma Ed25519
    //      canonica sobre `mensaje_a_firmar(nombre, timestamp, raiz)`. Autor
    //      ajeno -> `CapacidadInsuficiente`; firma forjada -> `AlmacenamientoFallo`.
    //   5. El manifiesto recomendado debe estar ingestado (el demuxer ya lo
    //      pidio al recibir el anuncio); si falta -> `Ausente`, reintentar.
    //   6. Reancla atomica: una sola escritura del superbloque.
    //
    // GATEADA por PERMISO_RAIZ, igual que `sys_manifiesto_proponer`: misma
    // autoridad. La diferencia es el ESQUEMA de firma (canonico del canal, no
    // hash pelado) — por eso vive aparte y no reusa el sobre `ManifiestoFirmado`.
    enlazador.func_wrap(
        "renaser",
        "sys_canal_aceptar",
        |caller: Caller<'_, ContextoCapacidades>, raiz_ptr: u32| -> Result<i32, Error> {
            let memoria = obtener_memoria(&caller)?;
            let mut raiz = [0u8; 32];
            {
                let m = memoria.data(&caller);
                let crudo = rango(
                    m,
                    raiz_ptr,
                    32,
                    "WASM :: sys_canal_aceptar desbordo la memoria lineal",
                )?;
                raiz.copy_from_slice(crudo);
            }
            // El anuncio retenido debe existir Y casar la raiz aceptada.
            let anuncio = match crate::akasha::ultimo_anuncio() {
                Some(a) if a.raiz == raiz => a,
                _ => return Ok(CodigoError::Ausente.como_i32()),
            };
            // Nombre del canal desde su objeto del grafo (la firma lo cubre).
            let canal_obj = match crate::almacen::recuperar(&anuncio.canal) {
                Ok(Some(o)) => o,
                Ok(None) => return Ok(CodigoError::Ausente.como_i32()),
                Err(_) => return Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            };
            let canal = match format::Canal::deserializar(&canal_obj.datos) {
                Ok(c) => c,
                Err(_) => return Ok(CodigoError::Ausente.como_i32()),
            };
            // Verificacion soberana: anillo + firma canonica.
            if let Err(err) = crate::claves::verificar_anuncio_canal(
                &anuncio.autor,
                &canal.nombre,
                anuncio.timestamp,
                &anuncio.raiz,
                &anuncio.firma,
            ) {
                return Ok(err.como_i32());
            }
            // El manifiesto recomendado tiene que estar ingestado localmente.
            match crate::almacen::recuperar(&anuncio.raiz) {
                Ok(Some(_)) => {}
                Ok(None) => return Ok(CodigoError::Ausente.como_i32()),
                Err(_) => return Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            }
            // Misma compuerta que `sys_manifiesto_proponer`: el manifiesto
            // aceptado debe ser instanciable (bytecodes presentes y WASM
            // cargable) antes de mover el superbloque. La cascada del DAG pudo
            // no haber convergido aun; en ese caso `Ausente` y mudanza reintenta.
            if let Err(err) = crate::wasm::validar_manifiesto_instanciable(&anuncio.raiz) {
                return Ok(err.como_i32());
            }
            // Reancla atomica del manifiesto vivo.
            match crate::almacen::fijar_manifiesto(anuncio.raiz) {
                Ok(()) => Ok(CodigoError::Ok.como_i32()),
                Err(_) => Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            }
        },
    )?;

    // --- CAPACIDAD 7e :: sys_canal_descartar(raiz_ptr) -> i32 ---
    // Fase 67 :: RECHAZA el anuncio retenido cuya raiz casa con `raiz_ptr`,
    // vaciando la ranura `ULTIMO_ANUNCIO` para que `sys_canal_anuncio` deje de
    // ofrecerlo. Es el gemelo de `sys_canal_aceptar` con el que mudanza cierra
    // el bucle Aceptar/Rechazar: una propuesta vista y descartada no reaparece
    // cada fotograma. A diferencia de aceptar, NO mueve el superbloque ni
    // verifica firma —solo limpia el buzon—; pero comparte el guard TOCTOU
    // (descarta EXACTAMENTE la raiz que el operador vio, de modo que un anuncio
    // mas nuevo llegado entre mostrar y rechazar sobrevive). Devuelve `Ok` si
    // descarto, `Ausente` si la ranura estaba vacia o la raiz no casaba.
    //
    // GATEADA por PERMISO_RAIZ por simetria con aceptar: el ciclo de vida del
    // anuncio de re-ancla es competencia de la app soberana, no de cualquiera
    // que pudiera vaciar el buzon ajeno como negacion de servicio.
    enlazador.func_wrap(
        "renaser",
        "sys_canal_descartar",
        |caller: Caller<'_, ContextoCapacidades>, raiz_ptr: u32| -> Result<i32, Error> {
            let memoria = obtener_memoria(&caller)?;
            let mut raiz = [0u8; 32];
            {
                let m = memoria.data(&caller);
                let crudo = rango(
                    m,
                    raiz_ptr,
                    32,
                    "WASM :: sys_canal_descartar desbordo la memoria lineal",
                )?;
                raiz.copy_from_slice(crudo);
            }
            if crate::akasha::descartar_anuncio(&raiz) {
                Ok(CodigoError::Ok.como_i32())
            } else {
                Ok(CodigoError::Ausente.como_i32())
            }
        },
    )?;
    } // PERMISO_RAIZ

    // --- CAPACIDAD 7c :: sys_grafo_compactar() -> i32 ---
    // Lanza una pasada del compactador semantico (MARK -> SWEEP -> SWAP) sobre
    // el log direccionado por contenido. El GC ya corre solo en el tic ocioso
    // del compositor cuando `escrituras_pendientes() >= UMBRAL_GC`; esta
    // syscall expone la palanca EXPLICITA para `wawactl gc`, `cronista` y
    // similares: forzar la compactacion AHORA, sin esperar al umbral.
    //
    // RETORNO: numero de nodos VIVOS supervivientes (>= 0) si la pasada tuvo
    // exito, o `CodigoError::AlmacenamientoFallo` (-3) si el almacen fallo.
    // El cap superior del disco (32 MiB / 512 B = 65 536 nodos) cae comodo
    // dentro de i32 positivo, asi que la mezcla con codigos de error en
    // [-7, -1] no colisiona — la convencion del ABI sigue intacta.
    //
    // GATEADA por PERMISO_COMPACTAR: una app sin el bit no ve la syscall.
    // No se exige foco —es una operacion de mantenimiento, no interactiva—,
    // pero el hecho de tomar el cerrojo del almacen durante toda la pasada
    // hace que el fotograma del invocador (y el resto del reactor) se
    // estire; por eso el bit se asume reservado a apps privilegiadas.
    if permisos & PERMISO_COMPACTAR != 0 {
    enlazador.func_wrap(
        "renaser",
        "sys_grafo_compactar",
        |_caller: Caller<'_, ContextoCapacidades>| -> Result<i32, Error> {
            match crate::almacen::compactar() {
                Ok(stats) => {
                    // Cap a i32::MAX por defensa logica; en la practica el
                    // disco de 32 MiB nunca alcanza ese techo.
                    let vivos = core::cmp::min(stats.nodos_vivos, i32::MAX as usize);
                    Ok(vivos as i32)
                }
                Err(_) => Ok(CodigoError::AlmacenamientoFallo.como_i32()),
            }
        },
    )?;
    } // PERMISO_COMPACTAR
    Ok(())
}

fn enlazar_estado_dispositivos(
    enlazador: &mut Linker<ContextoCapacidades>,
    permisos: Permisos,
) -> Result<(), Error> {
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
    Ok(())
}

fn enlazar_red(
    enlazador: &mut Linker<ContextoCapacidades>,
    permisos: Permisos,
) -> Result<(), Error> {
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
    Ok(())
}

fn enlazar_config(
    enlazador: &mut Linker<ContextoCapacidades>,
    permisos: Permisos,
) -> Result<(), Error> {
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

fn enlazar_anuncio_tinkuy(
    enlazador: &mut Linker<ContextoCapacidades>,
    permisos: Permisos,
) -> Result<(), Error> {
    // --- CAPACIDAD pasiva :: sys_canal_anuncio(salida, capacidad) -> i32 ---
    // Fase 64 :: vuelca el ULTIMO `AnunciarCanal` recibido por Akasha a la
    // memoria de la app, en un layout fijo de 168 B —idéntico al `anuncio.bin`
    // que produce `agora-cli wawa publicar`—:
    //
    //   canal(32) | raiz(32) | autor(32) | timestamp_le(8) | firma(64)
    //
    // Retorno: 168 si habia un anuncio (bytes escritos), 0 si la ranura esta
    // vacia, `CapacidadInsuficiente` si `capacidad` < 168. Lectura PASIVA: es
    // dato publico de red, no muta nada; la app `mudanza` la sondea cada
    // fotograma para descubrir propuestas. La decision de aceptar (y la
    // verificacion soberana) viven en `sys_canal_aceptar`, gateada por RAIZ.
    enlazador.func_wrap(
        "renaser",
        "sys_canal_anuncio",
        |mut caller: Caller<'_, ContextoCapacidades>,
         salida: u32,
         capacidad: u32|
         -> Result<i32, Error> {
            const LARGO: usize = 32 + 32 + 32 + 8 + 64; // 168
            let Some(anuncio) = crate::akasha::ultimo_anuncio() else {
                return Ok(0);
            };
            if (capacidad as usize) < LARGO {
                return Ok(CodigoError::CapacidadInsuficiente.como_i32());
            }
            let mut buf = [0u8; LARGO];
            buf[0..32].copy_from_slice(&anuncio.canal);
            buf[32..64].copy_from_slice(&anuncio.raiz);
            buf[64..96].copy_from_slice(&anuncio.autor);
            buf[96..104].copy_from_slice(&anuncio.timestamp.to_le_bytes());
            buf[104..168].copy_from_slice(&anuncio.firma);
            let memoria = obtener_memoria(&caller)?;
            {
                let m = memoria.data(&caller);
                rango(
                    m,
                    salida,
                    LARGO,
                    "WASM :: sys_canal_anuncio desbordo la memoria lineal",
                )?;
            }
            let m = memoria.data_mut(&mut caller);
            m[salida as usize..salida as usize + LARGO].copy_from_slice(&buf);
            Ok(LARGO as i32)
        },
    )?;

    // =========================================================================
    //  FASE C4 :: motor `tinkuy` empotrado
    // -------------------------------------------------------------------------
    //  Las apps con PERMISO_TINKUY reciben acceso al motor de particulas del
    //  kernel — una sub-jaula `wasmi` aparte que carga `assets/tinkuy.wasm` y
    //  resuelve sus exports `tk_*`. Cada syscall delega al modulo
    //  `crate::tinkuy`, que toma el cerrojo del motor, hace `data_mut` sobre
    //  SU memoria lineal (no la de la app), llama al `TypedFunc` y, si hace
    //  falta, copia el resultado a la memoria lineal de la app llamante con
    //  los limites verificados. Dos memorias jamas se mezclan.
    //
    //  Las syscalls comparten contrato:
    //    * Toman un `slot: u32` que la app obtuvo de `sys_tinkuy_sim_new`.
    //    * Verifican que el slot pertenezca a SU `indice_app` — el aislamiento
    //      entre apps tinkuy es matematica, no tabla de permisos.
    //    * Devuelven `TK_HOST_OK = 0`, valores positivos especificos (slot
    //      asignado, len) o negativos (`Agotado`, `Ajeno`, `Invalido`,
    //      `Motor` — codigos de `crate::tinkuy`).
    // =========================================================================
    if permisos & PERMISO_TINKUY != 0 {
        // --- CAPACIDAD :: sys_tinkuy_sim_new() -> i32 ---
        // Reserva una sim con geometria fija (cubo [-50, +50]^3, grid 34^3,
        // cell_size 3.0). Devuelve el indice de slot (>=0) o un error.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_new",
            |caller: Caller<'_, ContextoCapacidades>| -> i32 {
                let owner = caller.data().indice_app;
                crate::tinkuy::sim_new(owner)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_spawn(slot, x,y,z, vx,vy,vz, m, q) -> i32 ---
        // Añade una particula a la sim del slot. Codifica los nueve f32 como
        // tipos WASM nativos — sin punteros, sin scratch.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_spawn",
            |caller: Caller<'_, ContextoCapacidades>,
             slot: u32,
             x: f32,
             y: f32,
             z: f32,
             vx: f32,
             vy: f32,
             vz: f32,
             masa: f32,
             carga: f32|
             -> i32 {
                let owner = caller.data().indice_app;
                crate::tinkuy::sim_spawn(owner, slot, x, y, z, vx, vy, vz, masa, carga)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_rebuild_grid(slot) -> i32 ---
        // Reconstruye la grilla espacial. Llamada obligada despues de spawn
        // masivo y antes del primer `step_lj`.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_rebuild_grid",
            |caller: Caller<'_, ContextoCapacidades>, slot: u32| -> i32 {
                let owner = caller.data().indice_app;
                crate::tinkuy::sim_rebuild_grid(owner, slot)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_step_lj(slot, n_steps, dt, eps, sigma, cutoff) -> i32 ---
        // Avanza `n_steps` substeps de Velocity-Verlet con fuerza LJ. Los
        // bmin/bmax los fija el motor (mismo cubo de `sim_new`).
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_step_lj",
            |caller: Caller<'_, ContextoCapacidades>,
             slot: u32,
             n_steps: u32,
             dt: f32,
             eps: f32,
             sigma: f32,
             cutoff: f32|
             -> i32 {
                let owner = caller.data().indice_app;
                crate::tinkuy::sim_step_lj(owner, slot, n_steps, dt, eps, sigma, cutoff)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_len(slot) -> i32 ---
        // Particulas vivas en la sim.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_len",
            |caller: Caller<'_, ContextoCapacidades>, slot: u32| -> i32 {
                let owner = caller.data().indice_app;
                crate::tinkuy::sim_len(owner, slot)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_observables(slot, out_24_ptr) -> i32 ---
        // Escribe 24 bytes en la memoria de la app: step (u64 LE, 8 B) +
        // KE (f64 LE, 8 B) + T (f64 LE, 8 B). Las apps lo leen plano sin
        // depender de la crate `format`. Limites verificados a fondo.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_observables",
            |mut caller: Caller<'_, ContextoCapacidades>,
             slot: u32,
             out_24_ptr: u32|
             -> Result<i32, Error> {
                let owner = caller.data().indice_app;
                let (step, ke, temp) = match crate::tinkuy::sim_observables(owner, slot) {
                    Ok(t) => t,
                    Err(codigo) => return Ok(codigo),
                };
                let memoria = obtener_memoria(&caller)?;
                {
                    let m = memoria.data(&caller);
                    rango(
                        m,
                        out_24_ptr,
                        24,
                        "WASM :: sys_tinkuy_sim_observables desbordo memoria",
                    )?;
                }
                let m = memoria.data_mut(&mut caller);
                let off = out_24_ptr as usize;
                m[off..off + 8].copy_from_slice(&step.to_le_bytes());
                m[off + 8..off + 16].copy_from_slice(&ke.to_le_bytes());
                m[off + 16..off + 24].copy_from_slice(&temp.to_le_bytes());
                Ok(crate::tinkuy::TK_HOST_OK)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_snapshot_cid(slot, out_32_ptr) -> i32 ---
        // Escribe 32 bytes del CID BLAKE3 del estado actual en la memoria
        // de la app. El kernel lo obtiene del motor en su scratch y lo
        // copia con limites firmes.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_snapshot_cid",
            |mut caller: Caller<'_, ContextoCapacidades>,
             slot: u32,
             out_32_ptr: u32|
             -> Result<i32, Error> {
                let owner = caller.data().indice_app;
                let cid = match crate::tinkuy::sim_snapshot_cid(owner, slot) {
                    Ok(c) => c,
                    Err(codigo) => return Ok(codigo),
                };
                let memoria = obtener_memoria(&caller)?;
                {
                    let m = memoria.data(&caller);
                    rango(
                        m,
                        out_32_ptr,
                        32,
                        "WASM :: sys_tinkuy_sim_snapshot_cid desbordo memoria",
                    )?;
                }
                let m = memoria.data_mut(&mut caller);
                m[out_32_ptr as usize..out_32_ptr as usize + 32].copy_from_slice(&cid);
                Ok(crate::tinkuy::TK_HOST_OK)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_free(slot) -> i32 ---
        // Libera el slot y la sim. Idempotente sobre slots libres/ajenos.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_free",
            |caller: Caller<'_, ContextoCapacidades>, slot: u32| -> i32 {
                let owner = caller.data().indice_app;
                crate::tinkuy::sim_free(owner, slot)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_positions(slot, out_ptr, cap_count) -> i32 ---
        // Copia las posiciones (x,y,z) en AoS hacia la memoria lineal de la
        // app. `cap_count` es el numero de PARTICULAS que cabe en `out_ptr`;
        // la syscall valida que `cap_count * 12` esta dentro de la memoria
        // de la app y devuelve la cantidad real copiada (>= 0) o un codigo
        // negativo. Cota dura: `MAX_PARTICULAS_VIZ` (256) — el kernel
        // truncara silenciosamente si la sim tuviera mas.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_positions",
            |mut caller: Caller<'_, ContextoCapacidades>,
             slot: u32,
             out_ptr: u32,
             cap_count: u32|
             -> Result<i32, Error> {
                let owner = caller.data().indice_app;
                let (n, posiciones) = match crate::tinkuy::sim_positions(owner, slot) {
                    Ok(t) => t,
                    Err(codigo) => return Ok(codigo),
                };
                let n_a_copiar = n.min(cap_count).min(
                    crate::tinkuy::MAX_PARTICULAS_VIZ as u32,
                ) as usize;
                let bytes_a_copiar = n_a_copiar * 12;
                let memoria = obtener_memoria(&caller)?;
                {
                    let m = memoria.data(&caller);
                    rango(
                        m,
                        out_ptr,
                        bytes_a_copiar,
                        "WASM :: sys_tinkuy_sim_positions desbordo memoria",
                    )?;
                }
                let m = memoria.data_mut(&mut caller);
                let off = out_ptr as usize;
                for i in 0..n_a_copiar {
                    let base = off + i * 12;
                    m[base..base + 4]
                        .copy_from_slice(&posiciones[i][0].to_le_bytes());
                    m[base + 4..base + 8]
                        .copy_from_slice(&posiciones[i][1].to_le_bytes());
                    m[base + 8..base + 12]
                        .copy_from_slice(&posiciones[i][2].to_le_bytes());
                }
                Ok(n_a_copiar as i32)
            },
        )?;
    } // PERMISO_TINKUY
    Ok(())
}

