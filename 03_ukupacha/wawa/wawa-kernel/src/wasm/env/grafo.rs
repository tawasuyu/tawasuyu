use super::*;

pub(crate) fn enlazar_grafo(
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

