use super::*;

// =============================================================================
//  Fundacion y consulta — el arranque
// =============================================================================

/// Funda el escritorio: crea una ventana por app, con su marco teselado inicial
/// y su cache de respaldo ya reservada al tamaño natural. `naturales` da el
/// `(ancho, alto)` del lienzo de cada app, en el orden del manifiesto.
///
/// FASE 59 v1 :: si el registro de outputs ya esta fundado, las dimensiones
/// se toman del output PRIMARIO en lugar de los parametros — la fuente de
/// verdad del «area que el compositor ocupa» pasa a vivir en
/// `pantallas`, no en los args de esta funcion. Mantenemos los parametros
/// como fallback por compatibilidad con flujos de boot que aun no inician
/// `pantallas` (tests, futuros backends).
pub fn fundar(ancho: usize, alto: usize, naturales: &[(usize, usize, &str)]) {
    let (ancho, alto) = match crate::pantallas::primario() {
        Some(region) => (region.ancho, region.alto),
        None => (ancho, alto),
    };
    MANDOS.call_once(|| ArrayQueue::new(CAPACIDAD_MANDOS));
    PARTOS_POR_INDICE.call_once(|| Mutex::new(Vec::new()));

    // FASE 64 :: cuantos monitores hay. Con 2+ outputs, las ventanas del
    // genesis se REPARTEN round-robin entre ellos —asi el escritorio se ve
    // extendido de entrada, sin esperar a que el operador mueva nada con
    // Alt+O—. Con un solo output, `n_outputs == 1` y todas nacen en el 0 (el
    // comportamiento de siempre).
    let n_outputs = crate::pantallas::count().max(1);
    let mut ventanas = Vec::with_capacity(naturales.len());
    for (idx, &(nat_ancho, nat_alto, nombre)) in naturales.iter().enumerate() {
        ventanas.push(Ventana {
            nombre: nombre.to_string(),
            natural_ancho: nat_ancho,
            natural_alto: nat_alto,
            // Marco provisional; `aplicar_teselado` lo fija enseguida.
            marco: RegionPantalla {
                x: 0,
                y: 0,
                ancho: 0,
                alto: 0,
            },
            // La cache: reservada UNA vez, acotada al lienzo natural.
            cache: vec![0u8; nat_ancho.saturating_mul(nat_alto).saturating_mul(4)],
            pintada: false,
            baliza: None,
            cerrada: false,
            // Reparto round-robin entre los outputs disponibles. `aplicar_teselado`
            // agrupa por este indice y tesela cada grupo en su monitor.
            output: idx % n_outputs,
        });
    }

    // El orden de teselado arranca como la identidad: la ventana `i` ocupa la
    // celda `i`. Ninguna ventana flota al nacer — el escritorio es puro
    // teselado hasta que el teclado lo decida (`Alt+F`).
    let orden = (0..ventanas.len()).collect();
    let mut escritorio = Escritorio {
        modo: MODO_INICIAL,
        ancho,
        alto,
        ventanas,
        orden,
        flotantes: Vec::new(),
        raton_izq: false,
        arrastre: None,
        // Buffers de recomposicion: reservados UNA SOLA VEZ al fundar el
        // escritorio; `recomponer` los reusa con `clear() + push()` sin
        // tocar al asignador. La cota es MAX_VENTANAS — apps por encima
        // se omiten silenciosamente del repintado, no se aloca tras este punto.
        capas_buf: Vec::with_capacity(MAX_VENTANAS),
        celdas_buf: Vec::with_capacity(MAX_VENTANAS),
        launcher_abierto: false,
        launcher_seleccion: 0,
        catalogo: Vec::new(),
        launcher_query: String::new(),
        launcher_filtrado: Vec::new(),
        launcher_mascaras: Vec::new(),
        launcher_scroll: 0,
    };
    aplicar_teselado(&mut escritorio);

    ESCRITORIO.call_once(|| Mutex::new(escritorio));
}

/// Recalcula el teselado y asigna a cada ventana TESELADA su marco. La celda
/// `slot` del teselado va a la ventana `orden[slot]`: manda el orden, no la
/// identidad. Las ventanas flotantes no estan en `orden` y conservan su marco.
///
/// FASE 59 v2 :: las ventanas se AGRUPAN por `Ventana::output` y cada grupo se
/// tesela dentro de la `RegionPantalla` de ese output (`pantallas::todos()`).
/// Para el caso vivo (un solo output que cubre el framebuffer), el resultado
/// es identico al teselado anterior: un solo grupo con todas las ventanas
/// teseladas, una sola region. Cuando un driver registre outputs adicionales,
/// las ventanas asociadas a cada uno se tesselan independientemente —sin
/// invadir la pantalla del vecino—.
pub(crate) fn aplicar_teselado(escritorio: &mut Escritorio) {
    let outputs = crate::pantallas::todos();
    if outputs.is_empty() {
        // Sin registro de outputs (situacion imposible tras `fundar`, pero
        // defensiva): caer al comportamiento legacy con el ancho/alto del
        // escritorio.
        let marcos = teselar(
            escritorio.orden.len(),
            escritorio.ancho,
            escritorio.alto,
            escritorio.modo,
        );
        for (slot, marco) in marcos.into_iter().enumerate() {
            let ventana = escritorio.orden[slot];
            escritorio.ventanas[ventana].marco = marco;
        }
        return;
    }

    for output in &outputs {
        // Las ventanas TESELADAS que viven en este output, en el orden
        // preservado de `escritorio.orden`. Un mismo paso para N=1 (todas
        // caen en el output 0) o para N>1 (cada output recibe su sub-orden).
        let mut indices_local: Vec<usize> = Vec::with_capacity(escritorio.orden.len());
        for &i in &escritorio.orden {
            if escritorio.ventanas[i].output == output.id {
                indices_local.push(i);
            }
        }
        let n = indices_local.len();
        if n == 0 {
            continue;
        }
        // Teselar dentro de la region del output: como `teselar` espera
        // ancho/alto absolutos de "la pantalla", le pasamos los del output;
        // luego trasladamos los marcos por el origen del output.
        let marcos = teselar(n, output.region.ancho, output.region.alto, escritorio.modo);
        for (slot, mut marco) in marcos.into_iter().enumerate() {
            marco.x = marco.x.saturating_add(output.region.x);
            marco.y = marco.y.saturating_add(output.region.y);
            let ventana = indices_local[slot];
            escritorio.ventanas[ventana].marco = marco;
        }
    }
}

/// Pinta el escenario inicial del compositor. Se invoca una vez, tras `fundar`,
/// antes de encender las apps: recompone el escritorio con todas las ventanas
/// aun sin pintar — el teselado se ve como una rejilla de paneles.
pub fn componer_escenario() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    recomponer(&mut escritorio);
}

/// El indice de la ventana enfocada. Lo LEE el manejador de IRQ1 para enrutar
/// cada tecla — por eso es una simple lectura atomica, sin cerrojo alguno.
pub fn foco() -> usize {
    FOCO.load(Ordering::Relaxed)
}

/// Encola un mando del teclado. Lo invoca el manejador de IRQ1: empujar a una
/// cola lock-free es seguro en contexto de interrupcion.
pub fn solicitar(mando: Mando) {
    if let Some(mandos) = MANDOS.get() {
        // Si la cola se desborda, el mando se pierde en silencio: mas vale
        // perder una pulsacion que arriesgar un panico dentro de una IRQ.
        let _ = mandos.push(mando);
    }
}

// =============================================================================
//  El fotograma de una app — cache y composicion
// =============================================================================

/// Recibe el fotograma de la app `indice`: lo copia a su CACHE de respaldo —el
/// kernel asume la persistencia visual— y lo lleva a pantalla. Sin ventanas
/// flotantes ninguna ventana solapa a otra: basta repintar la que cambio —el
/// camino RAPIDO—. Con flotantes vivas el solapamiento obliga a RECOMPONER el
/// escritorio entero, respetando el orden-Z. Lo invoca la capacidad
/// `sys_render_frame` desde el `tick` cooperativo.
pub fn presentar_fotograma(indice: usize, datos: &[u8]) {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    {
        let Some(ventana) = escritorio.ventanas.get_mut(indice) else {
            return;
        };
        // Una ventana cerrada (Fase 10) ya no se pinta: su app pudo emitir un
        // ultimo fotograma antes de que su tarea advirtiera la baja.
        if ventana.cerrada {
            return;
        }
        // Cachear el fotograma. El destino esta acotado al lienzo natural; se
        // copia el minimo de ambas longitudes — jamas se desborda la cache.
        let n = ventana.cache.len().min(datos.len());
        ventana.cache[..n].copy_from_slice(&datos[..n]);
        ventana.pintada = true;
    }

    if escritorio.flotantes.is_empty() {
        // Camino RAPIDO: sin flotantes el escritorio es puro teselado y la app
        // pinta directamente en su marco, como en la Fase 8.
        let ventana = &escritorio.ventanas[indice];
        let marco = ventana.marco;
        let nat_ancho = ventana.natural_ancho;
        let nat_alto = ventana.natural_alto;
        let enfocada = FOCO.load(Ordering::Relaxed) == indice;
        drop(escritorio);
        consola::volcar_marco(marco, nat_ancho, nat_alto, datos, enfocada);
    } else {
        // Hay ventanas flotantes: el solapamiento obliga a recomponer.
        recomponer(&mut escritorio);
    }
}

/// Marca la ventana `indice` como desalojada y tatua su marco con la baliza.
/// Desde aqui queda excluida del foco — el teclado la salta.
pub fn desalojar(indice: usize, color: Color) {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    {
        let Some(ventana) = escritorio.ventanas.get_mut(indice) else {
            return;
        };
        // Una ventana ya cerrada (Fase 10) no recibe baliza: la baja limpia
        // gana a un desalojo que llegue tarde, en la misma vuelta.
        if ventana.cerrada {
            return;
        }
        ventana.baliza = Some(color);
    }
    // Fase 15: la voz del kernel anuncia el desalojo.
    crate::drivers::altavoz::agendar(&crate::drivers::altavoz::VOZ_DESALOJO);

    if escritorio.flotantes.is_empty() {
        let marco = escritorio.ventanas[indice].marco;
        let enfocada = FOCO.load(Ordering::Relaxed) == indice;
        drop(escritorio);
        consola::pintar_desalojo(marco, color, enfocada);
    } else {
        recomponer(&mut escritorio);
    }
}

// =============================================================================
//  Los mandos del teclado — el escritorio interactivo
// =============================================================================

/// Atiende los mandos pendientes del teclado. La invoca la tarea del compositor
/// en cada fotograma, desde el reactor cooperativo — el unico contexto donde es
/// seguro bloquear el `ESCRITORIO` y la consola.
pub fn atender_mandos() {
    let Some(mandos) = MANDOS.get() else {
        return;
    };
    while let Some(mando) = mandos.pop() {
        // FASE 58 :: si el launcher esta abierto, se queda con el foco del
        // teclado: navega su seleccion, lanza la app elegida o se cierra. El
        // resto de los mandos se ignoran en silencio hasta que se cierre,
        // para que el escritorio no mute por debajo del overlay.
        if launcher_intercepta(mando) {
            continue;
        }
        match mando {
            Mando::CiclarLayout => ciclar_layout(),
            Mando::FocoSiguiente => mover_foco(true),
            Mando::FocoAnterior => mover_foco(false),
            Mando::Promover => promover(),
            Mando::MoverAdelante => mover_ventana(true),
            Mando::MoverAtras => mover_ventana(false),
            Mando::Flotar => flotar(),
            Mando::MoverVentanaOutput => mover_ventana_output(),
            Mando::Cerrar => cerrar(),
            // El alta de una app necesita instanciar un WASM — algo que el
            // compositor no sabe hacer—. Solo se cuenta la peticion; el
            // orquestador del kernel la atendera (ver `partos_pendientes`).
            Mando::Lanzar => {
                PARTOS.fetch_add(1, Ordering::Relaxed);
            }
            // ToggleLauncher / TextoLauncher / LanzarFila se atienden SIEMPRE
            // en `launcher_intercepta` —si llegan hasta aqui es que el
            // escritorio aun no esta fundado o el launcher se cerro entre
            // medias—. En cualquier caso, se descartan sin efecto.
            Mando::ToggleLauncher => {}
            Mando::TextoLauncher(_) => {}
            Mando::LanzarFila(_) => {}
            // Fase 57 :: GC manual desde el teclado. La pasada toma el cerrojo
            // del almacen durante toda la operacion, asi que el fotograma
            // del compositor se estira — aceptable como gesto explicito del
            // operador, no como rutina automatica (eso ya lo cubre el tic
            // ocioso del compositor cuando `escrituras_pendientes() >= UMBRAL_GC`).
            // El resultado va a la baliza serial: el operador lee el COM1
            // para confirmar nodos_vivos / muertos / sectores recuperados.
            Mando::CompactarGrafo => {
                use core::fmt::Write;
                // CAPA R :: en modo ramdisk el grafo es read-only; compactar
                // solo provocaria un `Err("ramdisk :: read-only")`. Cortamos
                // antes con una traza honesta en vez de fingir una pasada.
                if crate::drivers::disco::es_ramdisk() {
                    let _ = writeln!(
                        crate::baliza::Serie,
                        "gc :: manual (Alt+G) :: omitido :: ramdisk read-only",
                    );
                    continue;
                }
                match crate::almacen::compactar() {
                    Ok(stats) => {
                        let _ = writeln!(
                            crate::baliza::Serie,
                            "gc :: manual (Alt+G) :: vivos={} muertos={} sectores={}->{}",
                            stats.nodos_vivos,
                            stats.nodos_muertos,
                            stats.sectores_antes,
                            stats.sectores_despues,
                        );
                    }
                    Err(motivo) => {
                        let _ = writeln!(
                            crate::baliza::Serie,
                            "gc :: manual (Alt+G) :: fallo :: {}",
                            motivo,
                        );
                    }
                }
            }
        }
    }
}
