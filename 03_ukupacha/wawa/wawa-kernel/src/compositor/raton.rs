use super::*;

// =============================================================================
//  FASE 13 — raton, puntero y arrastre de ventanas flotantes
// =============================================================================

/// La ultima posicion del puntero que el compositor REFRESCO. Si la posicion
/// actual del raton coincide con esta, no hay nada nuevo que estampar; si
/// difiere, la consola debe volver a presentar. Empacada como `y * 65536 + x`,
/// con `usize::MAX` como centinela de «aun no he visto al raton».
static PUNTERO_REFRESCADO: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Drena los eventos del raton y los aplica: clic enfoca la ventana bajo el
/// puntero (y, si flota, inicia un arrastre); el boton sostenido la arrastra;
/// soltarlo termina el gesto. La invoca la tarea del compositor en cada
/// fotograma, desde el reactor cooperativo.
pub fn atender_raton() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    let mut cambio = false;
    while let Some(evento) = crate::drivers::raton::siguiente_evento() {
        let izq = evento.botones & 0b001 != 0;
        let x = evento.x as usize;
        let y = evento.y as usize;
        let izq_antes = escritorio.raton_izq;

        // FASE 58 :: el launcher abierto SE QUEDA con el raton. Mientras lo
        // este, ningun clic ni movimiento llega al userspace ni a la
        // taskbar — el overlay es modal. La fila se mapea contra el
        // FILTRADO (v3), no contra el catalogo entero: si el operador
        // escribio "p", solo las apps con "p" en su nombre se ven y el
        // indice de fila resuelve a un indice real del catalogo via
        // `launcher_filtrado[fila]`.
        if escritorio.launcher_abierto {
            let total = escritorio.launcher_filtrado.len();
            let scroll = escritorio.launcher_scroll;
            let region = region_launcher(escritorio.ancho, escritorio.alto, total);
            // FASE 58 v7 :: `fila_launcher_en` devuelve un indice VISIBLE
            // (0..filas_visibles). El indice absoluto en `launcher_filtrado`
            // es `scroll + visible`; sin sumar el scroll, hover y clic
            // engancharian la app equivocada cuando hay scroll.
            let fila_visible =
                fila_launcher_en(region, x, y, total.saturating_sub(scroll));
            let fila_absoluta = fila_visible.map(|v| scroll + v);
            // Hover: la fila bajo el puntero se vuelve la seleccion vigente,
            // de modo que Alt+Enter y el clic se mantengan coherentes.
            if let Some(idx_filtrado) = fila_absoluta {
                if escritorio.launcher_seleccion != idx_filtrado {
                    escritorio.launcher_seleccion = idx_filtrado;
                    ajustar_scroll_launcher(&mut escritorio);
                    cambio = true;
                }
            }
            // Clic-bajada: si cae sobre una fila, lanzar esa app y cerrar;
            // si cae fuera del overlay, cerrar sin lanzar (clic-para-cancelar).
            // Un clic en el titulo o el padding del overlay no hace nada — el
            // usuario aun puede mover la seleccion o salir.
            if izq && !izq_antes {
                if let Some(idx_filtrado) = fila_absoluta {
                    if let Some(&idx_real) = escritorio.launcher_filtrado.get(idx_filtrado) {
                        if let Some(cola) = PARTOS_POR_INDICE.get() {
                            cola.lock().push(idx_real);
                        }
                    }
                    cerrar_launcher(&mut escritorio);
                    cambio = true;
                } else if !contiene(region, x, y) {
                    cerrar_launcher(&mut escritorio);
                    cambio = true;
                }
            }
            escritorio.raton_izq = izq;
            continue;
        }

        if izq && !izq_antes {
            // Boton bajó: un CLIC. FASE 9 :: ¿cayó en el `start_button` del marco
            // (pata)? Entonces abre el launcher —el mismo gesto que Alt+P—, sin
            // tocar foco ni arrastre. El rect lo resuelve `pata_marco` con la
            // misma geometría con que lo pinta (start pegado al borde izquierdo).
            let barra_r = region_barra_marco(escritorio.ancho, escritorio.alto);
            let en_start = pata_marco::start_button_rect(barra_r)
                .is_some_and(|sb| contiene(sb, x, y));
            // Boton bajó: un CLIC. Si cae en la barra de tareas, enfocar la
            // pestaña pulsada SIN iniciar arrastre. Si no, comportamiento
            // habitual: enfocar la ventana topmost bajo el puntero.
            let area_bar = area_taskbar(escritorio.ancho, escritorio.alto);
            if en_start {
                abrir_launcher(&mut escritorio);
                cambio = true;
            } else if y >= area_bar.y && y < area_bar.y + area_bar.alto {
                if clic_en_launcher(&escritorio, x) {
                    // El boton «+» equivale a `Alt+N`: solicita un parto. La
                    // tarea del compositor lo recogera en su proxima vuelta.
                    PARTOS.fetch_add(1, Ordering::Relaxed);
                } else if let Some(v) = celda_taskbar_en(&escritorio, x) {
                    let viva = {
                        let w = &escritorio.ventanas[v];
                        w.baliza.is_none() && !w.cerrada
                    };
                    if viva {
                        FOCO.store(v, Ordering::Relaxed);
                        crate::drivers::altavoz::tono(0);
                        alzar_si_flota(&mut escritorio, v);
                        cambio = true;
                    }
                }
            } else if let Some(v) = ventana_en(&escritorio, x, y) {
                let viva = {
                    let w = &escritorio.ventanas[v];
                    w.baliza.is_none() && !w.cerrada
                };
                if viva {
                    // Enfocar como hace `mover_foco`: foco + bocina muda + alza
                    // si flota.
                    FOCO.store(v, Ordering::Relaxed);
                    crate::drivers::altavoz::tono(0);
                    alzar_si_flota(&mut escritorio, v);
                    // Si la ventana flota, empezar a arrastrarla.
                    if escritorio.flotantes.contains(&v) {
                        let marco = escritorio.ventanas[v].marco;
                        escritorio.arrastre = Some(Arrastre {
                            ventana: v,
                            agarre_dx: x.saturating_sub(marco.x),
                            agarre_dy: y.saturating_sub(marco.y),
                        });
                    }
                    cambio = true;
                }
            }
        } else if izq && izq_antes {
            // Boton sostenido: arrastrar la ventana asida, si la hay.
            if let Some(arr) = escritorio.arrastre {
                mover_arrastrada(&mut escritorio, arr, x, y);
                cambio = true;
            }
        } else if !izq && izq_antes {
            // Boton subió: fin del arrastre.
            escritorio.arrastre = None;
        }
        escritorio.raton_izq = izq;

        // ENRUTADO AL USERSPACE. Despues de aplicar foco y arrastre, entregar
        // el evento ya TRADUCIDO al canal del puntero de la ventana enfocada.
        // El kernel hace toda la matematica: la app no ve coordenadas globales
        // ni eventos que caigan fuera de su lienzo natural. Si el cursor esta
        // sobre el cromo de su propia ventana o sobre otras ventanas, el
        // evento se descarta en silencio dentro de `puntero::enrutar`.
        let foco = FOCO.load(Ordering::Relaxed);
        if let Some(ventana) = escritorio.ventanas.get(foco) {
            if !ventana.cerrada && ventana.baliza.is_none() {
                let marco = ventana.marco;
                let nat_ancho = ventana.natural_ancho;
                let nat_alto = ventana.natural_alto;
                crate::async_system::puntero::enrutar(
                    foco,
                    x,
                    y,
                    evento.botones,
                    marco.x,
                    marco.y,
                    marco.ancho,
                    marco.alto,
                    nat_ancho,
                    nat_alto,
                );
            }
        }
    }
    if cambio {
        recomponer(&mut escritorio);
        // El recomponer ya presento; sincronizar el centinela para no presentar
        // dos veces en la misma vuelta.
        PUNTERO_REFRESCADO.store(empacar_puntero(), Ordering::Relaxed);
    }
}

/// La ventana topmost que contiene el punto (x, y), si la hay. Recorre el
/// orden-Z de delante hacia atras: primero las flotantes (la ultima es la
/// frontal), despues las teseladas.
pub(crate) fn ventana_en(escritorio: &Escritorio, x: usize, y: usize) -> Option<usize> {
    for &v in escritorio.flotantes.iter().rev() {
        if contiene(escritorio.ventanas[v].marco, x, y) {
            return Some(v);
        }
    }
    for &v in escritorio.orden.iter().rev() {
        if contiene(escritorio.ventanas[v].marco, x, y) {
            return Some(v);
        }
    }
    None
}

/// ¿Contiene el marco al punto (x, y)?
pub(crate) fn contiene(marco: RegionPantalla, x: usize, y: usize) -> bool {
    x >= marco.x && x < marco.x + marco.ancho && y >= marco.y && y < marco.y + marco.alto
}

/// Mueve la ventana arrastrada de modo que el punto del puntero —la asa— siga
/// estando, dentro de la ventana, donde se asio. La ventana queda acotada al
/// area de apps.
pub(crate) fn mover_arrastrada(escritorio: &mut Escritorio, arr: Arrastre, x: usize, y: usize) {
    let area = area_apps(escritorio.ancho, escritorio.alto);
    let Some(ventana) = escritorio.ventanas.get_mut(arr.ventana) else {
        return;
    };
    let ancho = ventana.marco.ancho;
    let alto = ventana.marco.alto;
    let mut nx = x.saturating_sub(arr.agarre_dx);
    let mut ny = y.saturating_sub(arr.agarre_dy);
    // Acotar al area de apps: la ventana entera ha de caber dentro.
    if nx + ancho > area.x + area.ancho {
        nx = (area.x + area.ancho).saturating_sub(ancho);
    }
    if ny + alto > area.y + area.alto {
        ny = (area.y + area.alto).saturating_sub(alto);
    }
    nx = nx.max(area.x);
    ny = ny.max(area.y);
    ventana.marco.x = nx;
    ventana.marco.y = ny;
}

/// Empaca la posicion actual del puntero en un solo `usize` —`y * 65536 + x`—
/// para compararla atomicamente con la ultima refrescada. `usize::MAX` indica
/// «el raton no esta vivo».
pub(crate) fn empacar_puntero() -> usize {
    match crate::drivers::raton::posicion() {
        Some((x, y)) => (y << 16) | (x & 0xFFFF),
        None => usize::MAX,
    }
}

/// Sprite del puntero: ancho y alto en pixeles. Debe coincidir con
/// `grafico::PUNTERO` (12×18). Si el sprite cambia de tamano, ajustar aqui.
const SPRITE_ANCHO: usize = 12;
const SPRITE_ALTO: usize = 18;

/// Refresca el sprite del puntero cuando su posicion atomica difiere de la
/// ultima pintada. Seguro en CUALQUIER contexto — incluso dentro de la IRQ12
/// del raton — porque toma la consola con `try_lock`: si esta ocupada, sale
/// en silencio y el siguiente disparo (sea otro paquete del raton o el
/// proximo tic del compositor) reintentara. La posicion vive en atomicos
/// (`drivers::raton::RATON_X/Y`), asi que nunca se pierde un movimiento —
/// solo se acumula hasta el proximo refresco exitoso.
///
/// PRINCIPIO :: «el lienzo HACE de save-under» (grafico.rs, Fase 13). El
/// puntero vive solo en el framebuffer, no en el lienzo; blitear el lienzo
/// sobre la region anterior borra el cursor viejo sin save-under explicito.
/// `presentar_region` re-estampa el cursor SOLO si su rect actual intersecta
/// la region — asi blitear el rect nuevo deja el cursor pintado alli.
///
/// Costo: dos blits de 12×18 = 216 px (~864 bytes) cada uno por movimiento.
/// Comparado con el `presentar()` completo (memcpy 1920×1080×4 ≈ 8 MiB) que
/// se hacia antes: factor ~10 000× menos memoria tocada por refresh.
///
/// LATENCIA :: invocado desde la IRQ12 del raton (drivers::raton::procesar),
/// el cursor se redibuja al ritmo del sample rate del PS/2 (200 Hz = 5 ms)
/// en lugar del PIT del compositor (100 Hz = 10 ms). Es la diferencia entre
/// «el cursor salta entre tics» y «movimiento fluido».
pub fn refrescar_puntero() {
    // Fast path sin lock: si nada cambio, salir antes de tomar la consola.
    let actual = empacar_puntero();
    if actual == usize::MAX {
        return;
    }
    // FASE 62 :: si el kernel gobierna un cursor por HARDWARE (virtio-gpu),
    // mover el puntero es un comando diminuto en la cola de cursor —el host lo
    // compone en un plano aparte—, no un volcado de pantalla entera. Es la via
    // optima cuando hay virtio-gpu (QEMU, o metal con GPU virtio).
    if crate::drivers::gpu::cursor_hardware() {
        if PUNTERO_REFRESCADO.swap(actual, Ordering::Relaxed) != actual {
            let x = actual & 0xFFFF;
            let y = actual >> 16;
            crate::drivers::gpu::mover_cursor(x, y);
        }
        return;
    }

    // En metal real SIN virtio-gpu el cursor es por SOFTWARE: dirty-region de
    // 12×18 sobre el framebuffer GOP, sin re-presentar la pantalla entera. Es la
    // cura del lag del puntero EN METAL, donde el cursor por hardware no existe
    // (antes de Fase 62 aqui se hacia `consola::refrescar()` = present completo).
    if PUNTERO_REFRESCADO.load(Ordering::Relaxed) == actual {
        return;
    }

    // Tomar la consola con `try_lock`. Desde IRQ12 esto evita el deadlock
    // contra el compositor mid-recomponer; desde la tarea del compositor
    // jamas falla porque solo otra IRQ puede arrebatarsela brevemente.
    let consola = match crate::consola::CONSOLA.get() {
        Some(c) => c,
        None => return,
    };
    let mut consola = match consola.try_lock() {
        Some(g) => g,
        None => return,
    };

    // Re-leer dentro del lock para minimizar la ventana entre lectura y
    // pintado — el raton se mueve mientras peleamos por el cerrojo.
    let actual = empacar_puntero();
    if actual == usize::MAX {
        return;
    }
    let previo = PUNTERO_REFRESCADO.load(Ordering::Relaxed);
    if previo == actual {
        return;
    }

    // Borrar el sprite viejo: blit del lienzo (save-under) en su rect.
    if previo != usize::MAX {
        let x_prev = previo & 0xFFFF;
        let y_prev = previo >> 16;
        consola.presentar_region(RegionPantalla {
            x: x_prev,
            y: y_prev,
            ancho: SPRITE_ANCHO,
            alto: SPRITE_ALTO,
        });
    }

    // Estampar el sprite en la nueva posicion: `presentar_region` re-estampa
    // el cursor porque su rect actual (raton::posicion) cae dentro.
    let x = actual & 0xFFFF;
    let y = actual >> 16;
    consola.presentar_region(RegionPantalla {
        x,
        y,
        ancho: SPRITE_ANCHO,
        alto: SPRITE_ALTO,
    });

    // Solo avanzar el marcador cuando ambos blits se hicieron — asi un
    // fallo de lock no deja PUNTERO_REFRESCADO mintiendo sobre el estado
    // del framebuffer.
    PUNTERO_REFRESCADO.store(actual, Ordering::Relaxed);
}

// =============================================================================
//  Teselado — la geometria pura de `mirada-layout`
// =============================================================================

/// El area de pantalla que el compositor tesela: toda la pantalla menos la
/// franja de la consola en la cima, la **barra de menú del marco (pata)** justo
/// debajo, y la barra de tareas al pie. Reservar la franja de la barra (FASE 9)
/// es lo que evita que las ventanas queden tapadas por ella — el equivalente al
/// `Frame::work_area` que `pata_core::resolve` computa.
pub fn area_apps(ancho_pantalla: usize, alto_pantalla: usize) -> RegionPantalla {
    let cabeza = (FRANJA_CONSOLA + pata_marco::ALTO_BARRA).min(alto_pantalla);
    let pie = FRANJA_TASKBAR.min(alto_pantalla.saturating_sub(cabeza));
    RegionPantalla {
        x: 0,
        y: cabeza,
        ancho: ancho_pantalla,
        alto: alto_pantalla.saturating_sub(cabeza).saturating_sub(pie),
    }
}

/// La franja reservada para la barra de menú del marco: justo encima del área de
/// apps, de grosor [`pata_marco::ALTO_BARRA`]. La derivan tanto el render (la
/// pinta) como el ratón (hit-test del `start_button`) desde `area_apps`, así no
/// hay drift entre dónde se reserva, dónde se pinta y dónde se clickea.
pub fn region_barra_marco(ancho_pantalla: usize, alto_pantalla: usize) -> RegionPantalla {
    let apps = area_apps(ancho_pantalla, alto_pantalla);
    RegionPantalla {
        x: apps.x,
        y: apps.y.saturating_sub(pata_marco::ALTO_BARRA),
        ancho: apps.ancho,
        alto: pata_marco::ALTO_BARRA,
    }
}
