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
//    * sys_tiempo_mono       — leer el reloj monotono del sistema (Fase 11).
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
use crate::async_system::teclado::CanalTeclado;

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
    /// El techo de recursos de la aplicacion — hoy, su memoria lineal maxima.
    /// `wasmi` lo consulta en cada `memory.grow` via `Store::limiter`.
    pub(crate) limites: StoreLimits,
    /// El indice de esta app — su identidad. La usan las capacidades de estado
    /// (Fase 7c) para hallar su `EntradaApp` del manifiesto, y el compositor
    /// (Fase 8) para hallar su ventana en el escritorio: jamas la de otra.
    pub(crate) indice_app: usize,
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
/// Devuelve `Err` si una capacidad no se pudo enlazar — un fallo del kernel,
/// no de la app; aun asi se propaga como `Result` para no incendiar nada.
pub(crate) fn enlazar_capacidades(
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

    // --- CAPACIDAD 3 :: sys_object_put(datos, datos_len, hijos, hijos_cnt, salida) -> i32 ---
    // Graba un objeto en el grafo. El modulo entrega, en su memoria lineal, la
    // carga util y un arreglo de `hijos_cnt` hashes de 32 bytes (las aristas).
    // El kernel escribe el hash resultante —la identidad del objeto— en
    // `salida`. Devuelve 0 si el objeto se grabo (o ya existia), -1 si el
    // almacenamiento fallo.
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

            // --- Grabar. Un fallo del almacen NO es culpa de la app: -1. ---
            match crate::almacen::almacenar(datos, hijos) {
                Ok(hash) => {
                    let m = memoria.data_mut(&mut caller);
                    m[salida as usize..salida as usize + 32].copy_from_slice(&hash);
                    Ok(0)
                }
                Err(_) => Ok(-1),
            }
        },
    )?;

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
                Ok(None) => return Ok(-1),
                Err(_) => return Ok(-3),
            };
            if objeto.datos.len() > capacidad as usize {
                return Ok(-2);
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
    // `salida`. Devuelve -1 si el objeto no existe, -3 si el almacen fallo.
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
                Ok(None) => return Ok(-1),
                Err(_) => return Ok(-3),
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
                None => Ok(0),
            }
        },
    )?;

    // --- CAPACIDAD 7 :: sys_object_fijar_raiz(hash) -> i32 ---
    // Corona el objeto `hash` como raiz del grafo. Devuelve 0 si se logro, -3
    // si el almacenamiento fallo.
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
                Ok(()) => Ok(0),
                Err(_) => Ok(-3),
            }
        },
    )?;

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
                None => return Ok(0), // Sin estado previo: nada que cargar.
            };
            let objeto = match crate::almacen::recuperar(&hash) {
                Ok(Some(objeto)) => objeto,
                Ok(None) => return Ok(-1),
                Err(_) => return Ok(-3),
            };
            if objeto.datos.len() > capacidad as usize {
                return Ok(-2);
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
            // la app: se le devuelve -3, y ella decide que hacer.
            let hash = match crate::almacen::almacenar(datos, alloc::vec::Vec::new()) {
                Ok(hash) => hash,
                Err(_) => return Ok(-3),
            };
            // Anclarlo: muta el manifiesto vivo, lo re-graba y lo re-ancla.
            match crate::manifiesto::fijar_estado(indice, hash) {
                Ok(()) => Ok(0),
                Err(_) => Ok(-3),
            }
        },
    )?;

    // --- CAPACIDAD 10 :: sys_tiempo_mono() -> u64 ---
    // El reloj MONOTONO del sistema: milisegundos transcurridos desde el
    // arranque. Le da al userspace un sentido del tiempo independiente del
    // ritmo de los fotogramas — una app sabe CUANTO ha pasado, no solo CUANTAS
    // veces la han llamado—. Jamas retrocede. No toca la memoria del modulo:
    // es una lectura pura, sin puntero que validar.
    enlazador.func_wrap(
        "renaser",
        "sys_tiempo_mono",
        |_caller: Caller<'_, ContextoCapacidades>| -> u64 {
            crate::async_system::reloj::milisegundos()
        },
    )?;

    Ok(())
}
