use super::*;

pub(crate) fn enlazar_objeto(
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

