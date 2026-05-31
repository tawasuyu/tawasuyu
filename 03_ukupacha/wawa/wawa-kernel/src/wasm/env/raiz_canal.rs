use super::*;

pub(crate) fn enlazar_raiz_canal(
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

