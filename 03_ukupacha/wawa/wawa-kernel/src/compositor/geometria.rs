use super::*;

/// FASE 58 :: constantes del overlay del launcher (Alt+P). Prefijo `PICKER_`
/// para no chocar con `LAUNCHER_*` que define al boton `+` de la taskbar —
/// dos «launchers» distintos, uno es la palanca, otro es el picker modal.
/// `consola::pintar_launcher` las re-importa para mantener la geometria en
/// un solo sitio.
pub(crate) const PICKER_ANCHO: usize = 480;
pub(crate) const PICKER_ALTURA_TITULO: usize = 32;
pub(crate) const PICKER_ALTURA_FILA: usize = 26;
pub(crate) const PICKER_PADDING_INFERIOR: usize = 8;
/// Maximo de filas visibles a la vez — un poco mas que el genesis (12).
pub(crate) const PICKER_MAX_FILAS: usize = 16;

/// FASE 58 :: la region del overlay del launcher, centrada en la pantalla.
/// La caja escala con el numero de items hasta un techo razonable (cubre el
/// genesis con holgura sin tapar el escritorio entero); si el catalogo crece
/// mas alla del techo, las filas sobrantes se omiten en silencio —el launcher
/// MVP no hace scroll—. La altura del titulo y la fila se mantienen alineadas
/// con las constantes de `consola::pintar_launcher` via `PICKER_*`.
pub(crate) fn region_launcher(ancho_pantalla: usize, alto_pantalla: usize, items: usize) -> RegionPantalla {
    let filas_visibles = items.min(PICKER_MAX_FILAS).max(1);
    let alto = PICKER_ALTURA_TITULO + filas_visibles * PICKER_ALTURA_FILA + PICKER_PADDING_INFERIOR;
    let alto = alto.min(alto_pantalla);
    let ancho = PICKER_ANCHO.min(ancho_pantalla);
    RegionPantalla {
        x: (ancho_pantalla.saturating_sub(ancho)) / 2,
        y: (alto_pantalla.saturating_sub(alto)) / 2,
        ancho,
        alto,
    }
}

/// FASE 58 :: traduce un punto (x, y) en pantalla al indice de la fila del
/// launcher bajo el. `None` si el punto cae fuera de `region`, en la barra
/// de titulo, en el padding inferior, o sobre una fila que excede el numero
/// real de items del catalogo. El llamante debe acotar el indice si lo usa
/// para indexar — la funcion ya lo recorta a `items.min(MAX_FILAS)`.
pub(crate) fn fila_launcher_en(region: RegionPantalla, x: usize, y: usize, items: usize) -> Option<usize> {
    if !contiene(region, x, y) {
        return None;
    }
    let filas_y0 = region.y + PICKER_ALTURA_TITULO;
    let filas_y_max = region.y + region.alto.saturating_sub(PICKER_PADDING_INFERIOR);
    if y < filas_y0 || y >= filas_y_max {
        return None;
    }
    let idx = (y - filas_y0) / PICKER_ALTURA_FILA;
    let max_idx = items.min(PICKER_MAX_FILAS);
    if idx >= max_idx {
        return None;
    }
    Some(idx)
}

/// El area de la barra de tareas: una franja al pie de la pantalla.
pub(crate) fn area_taskbar(ancho_pantalla: usize, alto_pantalla: usize) -> RegionPantalla {
    let pie = FRANJA_TASKBAR.min(alto_pantalla);
    RegionPantalla {
        x: 0,
        y: alto_pantalla.saturating_sub(pie),
        ancho: ancho_pantalla,
        alto: pie,
    }
}

/// El color de tinta —oscuro o claro— que da contraste legible sobre `fondo`.
/// Sin esto, la pestaña amarilla palida del desalojo por memoria quedaba con
/// texto blanco sobre crema: ilegible. La regla de luminancia ITU-R BT.601 fija
/// el umbral: fondos claros llevan tinta oscura, fondos oscuros la clara.
pub(crate) fn tinta_para(fondo: Color) -> Color {
    let brillo =
        (fondo.r as u32 * 299 + fondo.g as u32 * 587 + fondo.b as u32 * 114) / 1000;
    if brillo > 160 {
        // Fondo claro: tinta del reposo del lienzo, casi negra.
        Color::LIENZO_EN_REPOSO
    } else {
        Color::TEXTO
    }
}

/// Tesela el area de apps en `n` marcos con el modo dado. El vector resultante
/// tiene exactamente `n` elementos, en el orden de las celdas del teselado.
pub(crate) fn teselar(n: usize, ancho: usize, alto: usize, modo: LayoutMode) -> Vec<RegionPantalla> {
    let area = area_apps(ancho, alto);
    let pantalla = Rect::new(
        area.x as i32,
        area.y as i32,
        area.ancho as i32,
        area.alto as i32,
    );
    let params = LayoutParams {
        mode: modo,
        gap: MARGEN,
        ..LayoutParams::default()
    };
    tile(pantalla, n, &params)
        .into_iter()
        .map(rect_a_region)
        .collect()
}

/// Traduce un `Rect` de `mirada-layout` (`i32`) a la `RegionPantalla` del
/// kernel (`usize`). Un rectangulo degenerado queda en cero.
pub(crate) fn rect_a_region(r: Rect) -> RegionPantalla {
    RegionPantalla {
        x: r.x.max(0) as usize,
        y: r.y.max(0) as usize,
        ancho: r.w.max(0) as usize,
        alto: r.h.max(0) as usize,
    }
}
