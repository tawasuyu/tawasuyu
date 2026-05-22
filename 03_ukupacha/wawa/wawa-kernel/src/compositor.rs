// =============================================================================
//  renaser :: kernel/src/compositor.rs — Fase 8 :: el compositor teselante
// -----------------------------------------------------------------------------
//  Hasta la Fase 7, cada app llevaba su region escrita a mano en el manifiesto:
//  coordenadas fijas, una composicion rigida. La Fase 8 entrega esa decision a
//  un COMPOSITOR: el kernel ya no coloca las ventanas a mano, las TESELA.
//
//  El motor de teselado es `mirada-layout` — el mismo nucleo `no_std` que
//  ordena las ventanas del compositor Wayland de brahman. Cruza la frontera de
//  workspace y se enlaza aqui sin una linea de codigo nueva: geometria pura,
//  determinista, la misma en Linux y en el bare-metal de renaser.
//
//  Cada app conserva su tamaño NATURAL —el lienzo que sabe pintar, fijo—; el
//  compositor decide DONDE va ese lienzo. El kernel centra el fotograma natural
//  de la app dentro del marco teselado. Asi el compositor reordena la pantalla
//  sin que ninguna app cambie una sola instruccion.
// =============================================================================

use alloc::vec::Vec;

use mirada_layout::{tile, LayoutMode, LayoutParams, Rect};

use crate::grafico::RegionPantalla;

/// Altura del strip superior reservado a la consola; las apps teselan debajo.
/// La consola conserva ahi su registro de arranque completo —seis lineas,
/// hasta la sonda asincrona de disco— legible sobre el teselado.
const FRANJA_CONSOLA: usize = 296;

/// El modo de teselado del compositor. Fijo por ahora — la Fase 8b lo hara
/// conmutable en caliente desde el teclado, recorriendo los siete modos que
/// `mirada-layout` ofrece.
const MODO: LayoutMode = LayoutMode::MasterStack;

/// Margen entre ventanas teseladas, en pixeles — el aire que separa un marco
/// de sus vecinos.
const MARGEN: i32 = 14;

/// El area de pantalla que el compositor tesela: toda la pantalla menos la
/// franja de la consola en la cima.
pub fn area_apps(ancho_pantalla: usize, alto_pantalla: usize) -> RegionPantalla {
    RegionPantalla {
        x: 0,
        y: FRANJA_CONSOLA.min(alto_pantalla),
        ancho: ancho_pantalla,
        alto: alto_pantalla.saturating_sub(FRANJA_CONSOLA),
    }
}

/// Tesela el area de apps en `n` marcos —uno por ventana, en el orden de las
/// apps del manifiesto— con el algoritmo de `mirada-layout`. El vector
/// resultante tiene exactamente `n` elementos.
pub fn disponer(n: usize, ancho_pantalla: usize, alto_pantalla: usize) -> Vec<RegionPantalla> {
    let area = area_apps(ancho_pantalla, alto_pantalla);
    let pantalla = Rect::new(
        area.x as i32,
        area.y as i32,
        area.ancho as i32,
        area.alto as i32,
    );
    let params = LayoutParams {
        mode: MODO,
        gap: MARGEN,
        ..LayoutParams::default()
    };
    tile(pantalla, n, &params)
        .into_iter()
        .map(rect_a_region)
        .collect()
}

/// Traduce un `Rect` de `mirada-layout` (`i32`, en teoria con signo) a la
/// `RegionPantalla` del kernel (`usize`). Un rectangulo degenerado queda en
/// cero — el kernel no compondra nada en el.
fn rect_a_region(r: Rect) -> RegionPantalla {
    RegionPantalla {
        x: r.x.max(0) as usize,
        y: r.y.max(0) as usize,
        ancho: r.w.max(0) as usize,
        alto: r.h.max(0) as usize,
    }
}
