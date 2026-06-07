//! `llimphi-widget-terminal` — superficie de terminal **infinita y
//! virtualizada**.
//!
//! Diseño completo: `02_ruway/shuma/SDD-TERMINAL.md`. El control reemplaza por
//! fases al `output_pane` del shell: scrollback ilimitado a costo de render
//! **constante** (sólo se pinta la ventana visible), tres modos sobre la misma
//! tela (línea IDE / grilla TUI / híbrido) y GPU directo donde paga.
//!
//! **Fase 0:** la **Capa 0** — el [`store::Scrollback`], store de scrollback
//! append-only con índice de líneas, cap por memoria y acceso O(1). Puro, sin
//! dependencias de UI: el núcleo agnóstico vive aparte de quien lo pinta
//! (Regla 2).
//!
//! **Fase 1:** modo línea — [`view::line_surface`] materializa sólo las filas
//! visibles bajo un `scroll_y` propio del widget (costo de render **constante**
//! a scrollback ilimitado), numeración global y color por runs.
//!
//! **Fase 2 (esto):** la **Capa 1** — el modelo de **bloques** ([`blocks`]):
//! el stream es una secuencia de [`blocks::Item`]s (chrome de alto fijo que el
//! caller pinta + rangos de líneas del store), virtualizados sobre alturas
//! mixtas con búsqueda binaria; colapsar un bloque = no emitir su body. Mapea
//! el `output_pane` del shell (header/badge/etapas/colapso) sin que el widget
//! sepa de comandos (Regla 2). El modo línea de la Fase 1 es el caso de un solo
//! `Item::Lines`.

#![forbid(unsafe_code)]

pub mod blocks;
pub mod select;
pub mod store;
pub mod view;

pub use blocks::{block_surface, blocks_height, blocks_scroll_to_bottom, Item};
pub use select::{Point, SelectionRange};
pub use store::Scrollback;
pub use view::{
    content_height, line_surface, scroll_to_bottom, visible_window, LineStyle, TermMetrics,
    TermPalette, VisibleWindow,
};
