// =============================================================================
//  renaser :: kernel/src/compositor/pata_marco.rs — Fase 9 :: el marco (pata)
// -----------------------------------------------------------------------------
//  El kernel consume el MISMO `pata-core` que el frontend Llimphi en Linux: el
//  modelo agnóstico de widgets (un `WidgetSpec` → `Widget` que, alimentado por
//  un `WidgetCtx`, emite un `WidgetView`). Acá el kernel arma ese `WidgetCtx`
//  desde sus propios datos (la RAM del heap) y traduce el view-model a sus
//  primitivas de dibujo (`grafico::Lienzo` + `texto::rasterizar`). Es la regla
//  del repo hecha carne: un modelo, dos pinceles.
//
//  Hoy pinta el cluster de indicadores del taskbar (un medidor de RAM real, el
//  primer dato del sistema que pata pinta sobre el framebuffer de wawa). El
//  layout de pantalla completa (`pata_core::resolve`) y el resto de los widgets
//  llegan cuando el config venga por akasha — el modelo ya está, sólo falta la
//  fuente de config y el ruteo de input al `start_button`.
// =============================================================================

use alloc::vec::Vec;

use pata_core::widget::{build_all, WidgetCtx, WidgetView};
use pata_core::WidgetSpec;

use crate::grafico::{Color, Lienzo, RegionPantalla};
use crate::texto;

/// Ancho de la barrita de un medidor, en px.
const BARRA_W: usize = 56;
/// Alto de la barrita de un medidor, en px.
const BARRA_H: usize = 6;
/// Separación entre piezas de un widget y entre widgets, en px.
const GAP: usize = 8;
/// Tamaño de fuente de las leyendas, en px (igual que el reloj del taskbar).
const TAM: f32 = 16.0;

/// El snapshot del sistema que el kernel le entrega a los widgets de `pata-core`.
/// Hoy sólo la RAM (usado/total del heap); el resto queda en cero (un widget que
/// dependa de un campo en cero se ve vacío, no rompe). Cuando el kernel exponga
/// más contadores (CPU, etc.) se rellenan acá, sin tocar a `pata-core`.
pub(crate) fn ctx_kernel() -> WidgetCtx {
    let (usado, total) = crate::memory::allocator::stats();
    let total = total.max(1);
    let mib = 1024 * 1024;
    WidgetCtx {
        ram: (usado as f32 / total as f32).clamp(0.0, 1.0),
        ram_used_mb: (usado / mib) as u32,
        ram_total_mb: (total / mib) as u32,
        ..WidgetCtx::default()
    }
}

/// Los widgets del cluster de indicadores del kernel, declarados como lo haría un
/// config de `pata` (mismos `kind`s que en Linux). Cuando akasha entregue el
/// config real, esto se reemplaza por lo que venga del grafo.
fn specs_indicadores() -> Vec<WidgetSpec> {
    alloc::vec![WidgetSpec::new("ram_meter")]
}

/// Mezcla `fondo`→`tinta` según una cobertura de glifo `0..=255` (anti-aliasing).
fn mezclar(fondo: Color, tinta: Color, cobertura: u8) -> Color {
    let c = cobertura as u16;
    let inv = 255 - c;
    let mez = |f: u8, t: u8| ((f as u16 * inv + t as u16 * c) / 255) as u8;
    Color {
        r: mez(fondo.r, tinta.r),
        g: mez(fondo.g, tinta.g),
        b: mez(fondo.b, tinta.b),
    }
}

/// Ancho en px de una cadena al tamaño [`TAM`] (suma de avances de glifo).
fn medir_texto(s: &str) -> usize {
    s.chars()
        .map(|c| texto::rasterizar(c, TAM).0.advance_width as usize)
        .sum()
}

/// Funde una cadena sobre el lienzo en `(x, base_y)`, sobre un fondo conocido —
/// el mismo patrón que `consola::pintar_etiqueta`, sin estado de pluma.
fn dibujar_texto(lienzo: &mut Lienzo, x: usize, base_y: usize, s: &str, fondo: Color, tinta: Color) {
    let mut cursor = x;
    for caracter in s.chars() {
        let (m, cob) = texto::rasterizar(caracter, TAM);
        let inicio_x = cursor as isize + m.xmin as isize;
        let inicio_y = base_y as isize - m.ymin as isize - m.height as isize;
        for fila in 0..m.height {
            for col in 0..m.width {
                let opacidad = cob[fila * m.width + col];
                if opacidad == 0 {
                    continue;
                }
                let px = inicio_x + col as isize;
                let py = inicio_y + fila as isize;
                if px < 0 || py < 0 {
                    continue;
                }
                lienzo.pintar_pixel(px as usize, py as usize, mezclar(fondo, tinta, opacidad));
            }
        }
        cursor += m.advance_width as usize;
    }
}

/// Ancho en px que ocupará un view-model al pintarse.
fn medir_vista(v: &WidgetView) -> usize {
    match v {
        WidgetView::Empty => 0,
        WidgetView::Text(t) | WidgetView::Placeholder(t) => medir_texto(t),
        WidgetView::Meter { label, caption, .. } => {
            let mut w = 0;
            if let Some(l) = label {
                w += medir_texto(l) + GAP;
            }
            w += BARRA_W + GAP + medir_texto(caption);
            w
        }
    }
}

/// Pinta un view-model en `(x, …)` dentro de `rect`, sobre `fondo`. Devuelve el
/// ancho pintado (para avanzar al siguiente widget).
fn pintar_vista(lienzo: &mut Lienzo, v: &WidgetView, x: usize, rect: RegionPantalla, fondo: Color) -> usize {
    let base_y = rect.y + (rect.alto + 14) / 2;
    match v {
        WidgetView::Empty => 0,
        WidgetView::Text(t) | WidgetView::Placeholder(t) => {
            dibujar_texto(lienzo, x, base_y, t, fondo, Color::TEXTO);
            medir_texto(t)
        }
        WidgetView::Meter { label, fraction, caption } => {
            let mut cur = x;
            if let Some(l) = label {
                dibujar_texto(lienzo, cur, base_y, l, fondo, Color::TEXTO);
                cur += medir_texto(l) + GAP;
            }
            // Pista (track) + relleno proporcional. El framebuffer es color
            // sólido (no hay gradiente como en Llimphi/vello).
            let barra_y = rect.y + rect.alto.saturating_sub(BARRA_H) / 2;
            lienzo.rellenar_rect(cur, barra_y, BARRA_W, BARRA_H, Color::SIN_FOCO);
            let relleno = (BARRA_W as f32 * fraction.clamp(0.0, 1.0)) as usize;
            lienzo.rellenar_rect(cur, barra_y, relleno, BARRA_H, Color::FOCO);
            cur += BARRA_W + GAP;
            dibujar_texto(lienzo, cur, base_y, caption, fondo, Color::TEXTO);
            cur += medir_texto(caption);
            cur - x
        }
    }
}

/// Pinta el cluster de indicadores de `pata` **alineado a la derecha** dentro de
/// `rect`, sobre `fondo`. Construye los widgets desde el config del kernel, los
/// `tick`ea con [`ctx_kernel`] y traduce cada `WidgetView` a las primitivas del
/// framebuffer. Lo llama el taskbar (`consola::pintar_taskbar`).
pub(crate) fn pintar_cluster(lienzo: &mut Lienzo, rect: RegionPantalla, fondo: Color) {
    let specs = specs_indicadores();
    let ctx = ctx_kernel();
    let mut widgets = build_all(&specs);
    for w in &mut widgets {
        w.tick(&ctx);
    }
    let vistas: Vec<WidgetView> = widgets.iter().map(|w| w.view()).collect();

    let suma: usize = vistas.iter().map(medir_vista).sum();
    let total = suma + GAP * vistas.len().saturating_sub(1);
    // Alineado a la derecha: arranca a `total` px del borde derecho del rect.
    let mut x = rect.x + rect.ancho.saturating_sub(total);
    for v in &vistas {
        let w = pintar_vista(lienzo, v, x, rect, fondo);
        x += w + GAP;
    }
}
