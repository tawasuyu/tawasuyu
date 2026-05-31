use super::*;

pub(crate) fn enlazar_estado_dispositivos(
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

