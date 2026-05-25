// =============================================================================
//  renaser :: kernel/src/texto.rs — Fase 3 :: el texto como caso del dibujo
// -----------------------------------------------------------------------------
//  Con el heap activo, el texto deja de ser un mapa de bits estatico. Una
//  tipografia vectorial se empotra en el binario y se rasteriza glifo a glifo,
//  bajo demanda: el texto se vuelve, literalmente, un caso particular del
//  dibujo — la promesa fundacional de renaser.
// =============================================================================

use alloc::vec::Vec;

use fontdue::{Font, FontSettings, Metrics};
use spin::Once;

/// La tipografia vectorial, empotrada en el propio binario del kernel.
static FUENTE_TTF: &[u8] = include_bytes!("../assets/font.ttf");

/// La fuente ya analizada. Se funde una sola vez, tras activar el heap.
static FUENTE: Once<Font> = Once::new();

/// Analiza la tipografia empotrada. Requiere el heap ya activo.
pub fn init() {
    FUENTE.call_once(|| {
        Font::from_bytes(FUENTE_TTF, FontSettings::default())
            .expect("renaser :: la tipografia empotrada es invalida")
    });
}

/// Rasteriza un glifo al vuelo: devuelve sus metricas de colocacion y un mapa
/// de cobertura (un byte de opacidad, 0..=255, por cada pixel del glifo).
pub fn rasterizar(caracter: char, tam_px: f32) -> (Metrics, Vec<u8>) {
    FUENTE
        .get()
        .expect("renaser :: la tipografia no fue fundida")
        .rasterize(caracter, tam_px)
}
