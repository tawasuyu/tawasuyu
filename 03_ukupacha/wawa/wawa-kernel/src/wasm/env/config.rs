use super::*;

pub(crate) fn enlazar_config(
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

    // --- CAPACIDAD 19 :: sys_marco_proponer(ptr, len) -> i32 ---
    // El camino para mutar el **marco del escritorio** (la barra de menú de
    // `pata`) desde una app: entrega un puntero + largo a un `WireConfig`
    // serializado con postcard (el espejo postcard-safe de `pata_core`) en su
    // memoria lineal. El kernel lo deserializa (validándolo), lo graba como un
    // nodo NUEVO del grafo direccionado por contenido y reemplaza el marco
    // activo — el siguiente `tick` pinta el marco nuevo. Mismo espíritu que
    // `sys_config_proponer`: sin estados mutables sueltos, el config viaja por
    // akasha. Gateada por PERMISO_CONFIG y por el foco (el marco lo gobierna el
    // usuario, que interactúa con la ventana enfocada).
    //
    // Devuelve 0 al aplicar, -2 si la app no está enfocada, y `PayloadInvalido`
    // si el largo es absurdo o los bytes no son un `WireConfig` válido.
    enlazador.func_wrap(
        "renaser",
        "sys_marco_proponer",
        |caller: Caller<'_, ContextoCapacidades>, ptr: u32, len: u32| -> Result<i32, Error> {
            if crate::compositor::foco() != caller.data().indice_app {
                return Ok(CodigoError::SinFoco.como_i32());
            }
            // Tope defensivo: el config del marco es pequeño; lo absurdo se
            // rechaza antes de tocar la memoria o el grafo.
            if len == 0 || len > 64 * 1024 {
                return Ok(CodigoError::PayloadInvalido.como_i32());
            }
            let memoria = obtener_memoria(&caller)?;
            let datos = memoria.data(&caller);
            let bytes = rango(
                datos,
                ptr,
                len as usize,
                "WASM :: sys_marco_proponer desbordo la memoria lineal",
            )?;
            match crate::compositor::pata_marco::proponer(bytes) {
                Ok(()) => Ok(CodigoError::Ok.como_i32()),
                Err(_) => Ok(CodigoError::PayloadInvalido.como_i32()),
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

