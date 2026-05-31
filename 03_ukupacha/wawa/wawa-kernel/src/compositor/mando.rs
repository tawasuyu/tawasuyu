use super::*;

/// Cicla al siguiente modo de teselado: recalcula los marcos de las ventanas
/// teseladas y recompone el escritorio entero desde las caches de respaldo.
pub(crate) fn ciclar_layout() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    escritorio.modo = escritorio.modo.next();
    aplicar_teselado(&mut escritorio);
    recomponer(&mut escritorio);
}

/// Mueve el foco a la siguiente ventana VIVA. El recorrido abarca TODAS las
/// ventanas —las teseladas y, tras ellas, las flotantes— saltando las
/// desalojadas. Si la ventana recien enfocada flota, sube al frente del
/// orden-Z: la flotante con el foco esta SIEMPRE delante.
pub(crate) fn mover_foco(adelante: bool) {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    // El recorrido del foco: las teseladas, luego las flotantes — un orden
    // estable y visualmente coherente.
    let recorrido: Vec<usize> = escritorio
        .orden
        .iter()
        .chain(escritorio.flotantes.iter())
        .copied()
        .collect();
    let n = recorrido.len();
    if n == 0 {
        return;
    }
    let anterior = FOCO.load(Ordering::Relaxed);
    let pos = recorrido.iter().position(|&v| v == anterior).unwrap_or(0);

    // Avanzar saltando las ventanas desalojadas. Si no hay ninguna viva, tras
    // `n` pasos se vuelve al punto de partida y el foco no cambia.
    let mut nueva_pos = pos;
    let mut nuevo = anterior;
    for _ in 0..n {
        nueva_pos = if adelante {
            (nueva_pos + 1) % n
        } else {
            (nueva_pos + n - 1) % n
        };
        let candidata = recorrido[nueva_pos];
        if escritorio.ventanas[candidata].baliza.is_none() {
            nuevo = candidata;
            break;
        }
    }
    FOCO.store(nuevo, Ordering::Relaxed);
    // La bocina pertenece a la ventana enfocada (Fase 12): al cambiar el foco,
    // callar — la nueva dueña la reclamara en su proximo fotograma si quiere.
    crate::drivers::altavoz::tono(0);
    // La ventana recien enfocada, si flota, al frente del orden-Z.
    alzar_si_flota(&mut escritorio, nuevo);
    recomponer(&mut escritorio);
}

/// Promueve la ventana enfocada a la posicion maestra —la celda 0— del
/// teselado. Si la ventana enfocada flota, no esta en el orden de teselado y
/// el mando no hace nada — promover es una operacion del teselado.
pub(crate) fn promover() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    let foco = FOCO.load(Ordering::Relaxed);
    if let Some(pos) = escritorio.orden.iter().position(|&v| v == foco) {
        let ventana = escritorio.orden.remove(pos);
        escritorio.orden.insert(0, ventana);
        aplicar_teselado(&mut escritorio);
        recomponer(&mut escritorio);
    }
}

/// Mueve la ventana enfocada una posicion en el orden de teselado,
/// intercambiandola con su vecina. Una ventana flotante no esta en el orden:
/// el mando no la afecta.
pub(crate) fn mover_ventana(adelante: bool) {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    let n = escritorio.orden.len();
    if n < 2 {
        return;
    }
    let foco = FOCO.load(Ordering::Relaxed);
    if let Some(pos) = escritorio.orden.iter().position(|&v| v == foco) {
        let destino = if adelante {
            (pos + 1) % n
        } else {
            (pos + n - 1) % n
        };
        escritorio.orden.swap(pos, destino);
        aplicar_teselado(&mut escritorio);
        recomponer(&mut escritorio);
    }
}

// =============================================================================
//  FASE 9 — orden-Z y ventanas flotantes
// =============================================================================

/// Alterna la ventana enfocada entre TESELADA y FLOTANTE. Al flotar, la ventana
/// abandona el teselado —que se recalcula para las que quedan—, recibe un marco
/// propio en cascada y sube al frente del orden-Z. Al volver al teselado, se
/// reincorpora al final del orden. El foco no cambia: viaja con la ventana.
pub(crate) fn flotar() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    let foco = FOCO.load(Ordering::Relaxed);

    if let Some(pos) = escritorio.orden.iter().position(|&v| v == foco) {
        // Teselada -> flotante: se desliga del teselado, recibe su marco
        // propio en cascada y sube al frente del orden-Z.
        escritorio.orden.remove(pos);
        let marco = marco_flotante(&escritorio, foco);
        escritorio.ventanas[foco].marco = marco;
        escritorio.flotantes.push(foco);
        aplicar_teselado(&mut escritorio);
        recomponer(&mut escritorio);
    } else if let Some(pos) = escritorio.flotantes.iter().position(|&v| v == foco) {
        // Flotante -> teselada: vuelve a la rejilla, al final del orden.
        escritorio.flotantes.remove(pos);
        escritorio.orden.push(foco);
        aplicar_teselado(&mut escritorio);
        recomponer(&mut escritorio);
    }
}

/// Si la ventana `indice` es flotante, la lleva al frente del orden-Z —al final
/// de `flotantes`—. Si esta teselada, no hace nada.
pub(crate) fn alzar_si_flota(escritorio: &mut Escritorio, indice: usize) {
    if let Some(pos) = escritorio.flotantes.iter().position(|&v| v == indice) {
        let ventana = escritorio.flotantes.remove(pos);
        escritorio.flotantes.push(ventana);
    }
}

/// El marco de una ventana recien hecha flotante: su lienzo natural mas un
/// reborde de cromo, colocado en cascada —para que varias flotantes no se
/// tapen del todo— y acotado al area de apps. Se invoca ANTES de inscribir la
/// ventana en `flotantes`: su longitud da el escalon de la cascada.
pub(crate) fn marco_flotante(escritorio: &Escritorio, indice: usize) -> RegionPantalla {
    let area = area_apps(escritorio.ancho, escritorio.alto);
    let ventana = &escritorio.ventanas[indice];
    let ancho = (ventana.natural_ancho + 2 * CROMO_FLOTANTE).min(area.ancho);
    let alto = (ventana.natural_alto + 2 * CROMO_FLOTANTE).min(area.alto);

    // La cascada: un escalon por cada flotante ya existente.
    let escalon = escritorio.flotantes.len().saturating_mul(PASO_CASCADA);
    let mut x = area.x + 48 + escalon;
    let mut y = area.y + 40 + escalon;
    // Acotar: la ventana entera ha de caber dentro del area de apps.
    if x + ancho > area.x + area.ancho {
        x = area.x + area.ancho.saturating_sub(ancho);
    }
    if y + alto > area.y + area.alto {
        y = area.y + area.alto.saturating_sub(alto);
    }
    RegionPantalla {
        x,
        y,
        ancho,
        alto,
    }
}

/// Recompone el escritorio entero respetando el orden-Z. Arma la lista de capas
/// —primero las ventanas TESELADAS, la capa de fondo; despues las FLOTANTES, de
/// atras hacia adelante— y se la entrega a la consola, que las funde en ese
/// orden de una sola pasada. El solapamiento se resuelve por el orden del
/// pintado. La invocan los mandos del teclado y `presentar_fotograma` cuando
/// hay flotantes vivas. El llamante sostiene ya el cerrojo del `ESCRITORIO`.
pub(crate) fn recomponer(escritorio: &mut Escritorio) {
    let area = area_apps(escritorio.ancho, escritorio.alto);
    let foco = FOCO.load(Ordering::Relaxed);

    // --- BUFFER de capas reusado. clear() no libera capacidad; push() abajo
    //     se mantiene dentro de la capacity reservada al fundar (no toca al
    //     asignador). Si MAX_VENTANAS se queda corto, el `take` lo acota sin
    //     panico — las apps extras quedan sin recomponer este fotograma. ---
    escritorio.capas_buf.clear();
    for &indice in escritorio
        .orden
        .iter()
        .chain(escritorio.flotantes.iter())
        .take(MAX_VENTANAS)
    {
        let ventana = &escritorio.ventanas[indice];
        let contenido = match ventana.baliza {
            Some(color) => consola::ContenidoSlot::Baliza(color),
            None if ventana.pintada => consola::ContenidoSlot::Fotograma(indice),
            None => consola::ContenidoSlot::Panel,
        };
        escritorio.capas_buf.push(consola::CapaSlot {
            marco: ventana.marco,
            nat_ancho: ventana.natural_ancho,
            nat_alto: ventana.natural_alto,
            contenido,
            enfocada: indice == foco,
        });
    }

    // --- FASE 14/16 :: armar la barra de tareas. El mismo trato: clear() +
    //     push() sobre el buffer pre-alocado de celdas. ---
    let area_bar = area_taskbar(escritorio.ancho, escritorio.alto);
    let cy = area_bar.y + 4;
    let calto = area_bar.alto.saturating_sub(8);
    let launcher = RegionPantalla {
        x: area_bar.x + CELDA_TASKBAR_MARGEN,
        y: cy,
        ancho: LAUNCHER_ANCHO,
        alto: calto,
    };
    let cells_x0 = launcher.x + launcher.ancho + LAUNCHER_HUECO;
    let cells_x_max =
        area_bar.x + area_bar.ancho - CELDA_TASKBAR_MARGEN - RELOJ_ANCHO - RELOJ_HUECO;
    escritorio.celdas_buf.clear();
    let mut cx = cells_x0;
    for (indice, ventana) in escritorio
        .ventanas
        .iter()
        .enumerate()
        .take(MAX_VENTANAS)
    {
        if ventana.cerrada {
            continue;
        }
        if cx + CELDA_TASKBAR_ANCHO > cells_x_max {
            break;
        }
        let fondo = match ventana.baliza {
            Some(color) => color,
            None if indice == foco => Color::FOCO,
            None => Color::PANEL,
        };
        escritorio.celdas_buf.push(consola::CeldaTaskbarSlot {
            region: RegionPantalla {
                x: cx,
                y: cy,
                ancho: CELDA_TASKBAR_ANCHO,
                alto: calto,
            },
            ventana: indice,
            fondo,
            tinta: tinta_para(fondo),
        });
        cx += CELDA_TASKBAR_ANCHO + CELDA_TASKBAR_HUECO;
    }

    // --- Reloj formateado SOBRE PILA. Reemplaza `alloc::format!("{}:{:02}", ...)`
    //     por escritura en un `[u8; 8]` mediante un `core::fmt::Write` minimo.
    //     Cero allocaciones. El segundero cubre hasta 99:59 (~6000 segundos);
    //     a partir de ahi se acota a "99:59" — el escritorio se reinicia
    //     antes en cualquier escenario realista. ---
    let ms = crate::async_system::reloj::milisegundos();
    let segs = ms / 1000;
    let mut reloj_buf = [0u8; RELOJ_BUFFER_LEN];
    let reloj_len = formatear_reloj(&mut reloj_buf, segs);
    // SEGURIDAD: `formatear_reloj` escribe SOLO ASCII (digitos y ':'),
    // garantizando un &str valido sin pasar por `from_utf8`.
    let reloj_texto =
        unsafe { core::str::from_utf8_unchecked(&reloj_buf[..reloj_len]) };

    let reloj_region = RegionPantalla {
        x: area_bar.x + area_bar.ancho - CELDA_TASKBAR_MARGEN - RELOJ_ANCHO,
        y: cy,
        ancho: RELOJ_ANCHO,
        alto: calto,
    };
    let taskbar = consola::TaskbarSlot {
        area: area_bar,
        launcher,
        celdas: &escritorio.celdas_buf,
        reloj: reloj_texto,
        reloj_region,
    };
    let resolver = ResolverEscritorio {
        ventanas: &escritorio.ventanas,
    };
    // FASE 58 :: si el launcher esta abierto, calcular su region centrada y
    // entregar el overlay a la consola como ultima capa. La caja escala con
    // las filas FILTRADAS (v3) —no con todo el catalogo—, asi al escribir la
    // caja encoge a las que matchean. El overlay viaja con el slice del
    // catalogo + el slice del filtrado: la consola itera el filtrado y
    // resuelve el nombre via `catalogo[filtrado[i]]`.
    let overlay = if escritorio.launcher_abierto {
        Some(consola::LauncherOverlay {
            region: region_launcher(
                escritorio.ancho,
                escritorio.alto,
                escritorio.launcher_filtrado.len(),
            ),
            catalogo: &escritorio.catalogo,
            filtrado: &escritorio.launcher_filtrado,
            mascaras: &escritorio.launcher_mascaras,
            seleccion: escritorio.launcher_seleccion,
            scroll: escritorio.launcher_scroll,
            filas_visibles: PICKER_MAX_FILAS,
            query: &escritorio.launcher_query,
        })
    } else {
        None
    };
    consola::recomponer(area, &escritorio.capas_buf, &taskbar, &resolver, overlay.as_ref());
    // Recordar el segundo recien mostrado: `tick_reloj` evita repintar de mas
    // mientras dure este mismo segundo.
    ULTIMO_SEGUNDO.store(segs, Ordering::Relaxed);
}

/// Resolver concreto del compositor para la consola: traduce un indice de
/// ventana a su cache de fotograma y a su nombre legible. Se construye en
/// la pila justo antes de invocar `consola::recomponer` — su lifetime no
/// se extiende mas alla del cerrojo del escritorio.
struct ResolverEscritorio<'a> {
    ventanas: &'a [Ventana],
}

impl<'a> consola::Resolver for ResolverEscritorio<'a> {
    fn cache(&self, indice: usize) -> &[u8] {
        &self.ventanas[indice].cache
    }
    fn nombre(&self, indice: usize) -> &str {
        &self.ventanas[indice].nombre
    }
}
