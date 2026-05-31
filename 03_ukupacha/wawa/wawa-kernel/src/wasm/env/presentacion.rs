use super::*;

pub(crate) fn enlazar_presentacion(
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

