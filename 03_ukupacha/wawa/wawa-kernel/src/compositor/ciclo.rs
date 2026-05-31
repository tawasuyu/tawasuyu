use super::*;

/// Escribe "M:SS" o "MM:SS" en `dst` y devuelve la longitud. Sin alocacion,
/// sin `core::fmt::Write`: un formateador ad-hoc para el reloj de la
/// taskbar. Acota los minutos a 99 — un disclaimer barato para no
/// engordar el formato y evitar tener que reanclar buffer en runtime.
pub(crate) fn formatear_reloj(dst: &mut [u8; RELOJ_BUFFER_LEN], segs: u64) -> usize {
    let mut min = segs / 60;
    let sec = (segs % 60) as u8;
    if min > 99 {
        min = 99;
    }
    let min = min as u8;
    let mut n = 0usize;
    if min >= 10 {
        dst[n] = b'0' + (min / 10);
        n += 1;
    }
    dst[n] = b'0' + (min % 10);
    n += 1;
    dst[n] = b':';
    n += 1;
    dst[n] = b'0' + (sec / 10);
    n += 1;
    dst[n] = b'0' + (sec % 10);
    n += 1;
    n
}

/// Localiza la celda de la barra de tareas bajo la coordenada x: itera las
/// ventanas vivas en orden de creacion y devuelve la N-esima donde la N es la
/// posicion en la barra. `None` si el clic cae en el lanzador, en el reloj, en
/// un hueco entre celdas, o fuera del rango de las pestañas.
pub(crate) fn celda_taskbar_en(escritorio: &Escritorio, x: usize) -> Option<usize> {
    let area_bar = area_taskbar(escritorio.ancho, escritorio.alto);
    // Las pestañas empiezan despues del lanzador.
    let cells_x0 = area_bar.x + CELDA_TASKBAR_MARGEN + LAUNCHER_ANCHO + LAUNCHER_HUECO;
    let cells_x_max =
        area_bar.x + area_bar.ancho - CELDA_TASKBAR_MARGEN - RELOJ_ANCHO - RELOJ_HUECO;
    if x < cells_x0 || x >= cells_x_max {
        return None;
    }
    let rel = x - cells_x0;
    let paso = CELDA_TASKBAR_ANCHO + CELDA_TASKBAR_HUECO;
    let posicion = rel / paso;
    let offset = rel % paso;
    if offset >= CELDA_TASKBAR_ANCHO {
        return None;
    }
    let mut k = 0;
    for (indice, ventana) in escritorio.ventanas.iter().enumerate() {
        if ventana.cerrada {
            continue;
        }
        if k == posicion {
            return Some(indice);
        }
        k += 1;
    }
    None
}

/// ¿Cae la coordenada x en el boton lanzador («+»)?
pub(crate) fn clic_en_launcher(escritorio: &Escritorio, x: usize) -> bool {
    let area_bar = area_taskbar(escritorio.ancho, escritorio.alto);
    let x0 = area_bar.x + CELDA_TASKBAR_MARGEN;
    let x1 = x0 + LAUNCHER_ANCHO;
    x >= x0 && x < x1
}

// =============================================================================
//  FASE 10 — alta y baja de aplicaciones en vivo
// =============================================================================

/// Cierra la aplicacion enfocada (`Alt+Q`): una baja LIMPIA, distinta del
/// desalojo por falla. Marca la ventana como cerrada, libera su cache de
/// respaldo, la saca del teselado y del orden-Z, y traslada el foco a una
/// ventana viva contigua. La app, en su tarea, advertira la baja y concluira.
pub(crate) fn cerrar() {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    let foco = FOCO.load(Ordering::Relaxed);
    // Solo se cierra una ventana viva. El foco jamas se posa en una muerta o
    // cerrada, pero la guarda lo deja explicito.
    match escritorio.ventanas.get(foco) {
        Some(v) if v.baliza.is_none() && !v.cerrada => {}
        _ => return,
    }
    // Fase 15: el kernel se despide de la app con un repique descendente.
    crate::drivers::altavoz::agendar(&crate::drivers::altavoz::VOZ_CERRAR);
    // Marcar la baja y liberar el respaldo: la cache de un fotograma puede
    // pesar un megabyte — no tiene sentido retenerla en una ranura inerte.
    let ventana = &mut escritorio.ventanas[foco];
    ventana.cerrada = true;
    ventana.pintada = false;
    ventana.cache = Vec::new();
    // Sacarla del teselado y del orden-Z. El censo conserva la ranura —los
    // indices son la identidad, jamas se reciclan—, pero ya nadie la dibuja.
    escritorio.orden.retain(|&v| v != foco);
    escritorio.flotantes.retain(|&v| v != foco);
    // Si la estabamos arrastrando con el raton (Fase 13), soltarla.
    if escritorio.arrastre.map(|a| a.ventana) == Some(foco) {
        escritorio.arrastre = None;
    }
    // El foco salta a la primera ventana viva que quede; si no queda ninguna,
    // se queda donde estaba —inofensivo: no hay a quien enrutar el teclado—.
    let nuevo = escritorio
        .orden
        .iter()
        .chain(escritorio.flotantes.iter())
        .copied()
        .find(|&v| {
            let w = &escritorio.ventanas[v];
            w.baliza.is_none() && !w.cerrada
        })
        .unwrap_or(foco);
    FOCO.store(nuevo, Ordering::Relaxed);
    // El foco cambia: callar la bocina (Fase 12) — ver `mover_foco`.
    crate::drivers::altavoz::tono(0);
    alzar_si_flota(&mut escritorio, nuevo);
    aplicar_teselado(&mut escritorio);
    recomponer(&mut escritorio);
}

/// Da de alta una ventana NUEVA y devuelve su indice —su identidad—. La crea
/// con su cache de respaldo al tamaño natural, la añade al final del orden de
/// teselado, recalcula el teselado y recompone. La invoca el orquestador del
/// kernel justo antes de instanciar el WASM de la app, que necesita ese indice.
pub fn nacer_ventana(nat_ancho: usize, nat_alto: usize, nombre: &str) -> usize {
    let Some(escritorio) = ESCRITORIO.get() else {
        return 0;
    };
    let mut escritorio = escritorio.lock();
    let indice = escritorio.ventanas.len();
    escritorio.ventanas.push(Ventana {
        nombre: nombre.to_string(),
        natural_ancho: nat_ancho,
        natural_alto: nat_alto,
        marco: RegionPantalla {
            x: 0,
            y: 0,
            ancho: 0,
            alto: 0,
        },
        cache: vec![0u8; nat_ancho.saturating_mul(nat_alto).saturating_mul(4)],
        pintada: false,
        baliza: None,
        cerrada: false,
        // FASE 59 v2 :: las altas en vivo nacen siempre en el output
        // primario. Un futuro `nacer_ventana_en(output, ...)` aceptara
        // un output explicito cuando haya N>1.
        output: 0,
    });
    escritorio.orden.push(indice);
    aplicar_teselado(&mut escritorio);
    recomponer(&mut escritorio);
    // Fase 15: el kernel saluda al nacimiento con un repique ascendente.
    crate::drivers::altavoz::agendar(&crate::drivers::altavoz::VOZ_LANZAR);
    indice
}

/// ¿Se ha pedido cerrar la ventana `indice`? Cada app la consulta en su tarea,
/// fotograma a fotograma: cuando es `true`, concluye su tarea y se libera. Una
/// ventana inexistente cuenta como cerrada.
pub fn ventana_cerrada(indice: usize) -> bool {
    let Some(escritorio) = ESCRITORIO.get() else {
        return false;
    };
    escritorio
        .lock()
        .ventanas
        .get(indice)
        .map(|ventana| ventana.cerrada)
        .unwrap_or(true)
}

/// Cuantas aplicaciones nuevas se han pedido lanzar desde la ultima consulta —y
/// pone el contador a cero—. La invoca el orquestador del kernel —el unico que
/// sabe instanciar un WASM— en cada fotograma de la tarea del compositor.
pub fn partos_pendientes() -> usize {
    PARTOS.swap(0, Ordering::Relaxed)
}

/// FASE 58 :: drena la cola de partos DIRIGIDOS —cada indice apunta a la
/// plantilla a instanciar—. La rellena el launcher al cerrar con Alt+Enter;
/// la consume el orquestador del kernel. Se reusa el `Vec` interno con un
/// `mem::take` para no obligar al llamante a tomar el cerrojo dos veces.
pub fn partos_por_indice_pendientes() -> Vec<usize> {
    let Some(cola) = PARTOS_POR_INDICE.get() else {
        return Vec::new();
    };
    core::mem::take(&mut *cola.lock())
}

/// FASE 58 :: fija el catalogo de apps lanzables — el listado que el launcher
/// muestra al usuario. El indice de cada nombre coincide con el de la plantilla
/// homonima en `main.rs::PLANTILLAS`. Se invoca una sola vez, justo despues de
/// armar las plantillas del manifiesto en el arranque.
pub fn fijar_catalogo(nombres: Vec<String>) {
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    escritorio.catalogo = nombres;
    // Si el catalogo cambia y el launcher seguia abierto (no deberia, pero
    // defensivo), refiltrar para que la lista visible no quede obsoleta.
    if escritorio.launcher_abierto {
        refiltrar_launcher(&mut escritorio);
    }
}

/// Avanza el reloj de la barra de tareas (Fase 16): si el segundo del reloj
/// monotono cambio respecto al ultimo mostrado, recompone para refrescar la
/// pantalla. Si el segundo es el mismo, vuelve sin hacer nada — un fotograma
/// barato—. La invoca la tarea del compositor cada fotograma.
pub fn tick_reloj() {
    let actual = crate::async_system::reloj::milisegundos() / 1000;
    if ULTIMO_SEGUNDO.load(Ordering::Relaxed) == actual {
        return;
    }
    let Some(escritorio) = ESCRITORIO.get() else {
        return;
    };
    let mut escritorio = escritorio.lock();
    recomponer(&mut escritorio);
}
