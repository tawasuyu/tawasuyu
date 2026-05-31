use super::*;

pub(crate) fn enlazar_red(
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

