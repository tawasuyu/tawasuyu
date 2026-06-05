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
//! **Fase 1 (esto):** las **Capas 1–2** — [`view::line_surface`], la superficie
//! modo línea **virtualizada**: materializa sólo las filas visibles bajo un
//! `scroll_y` propio del widget (costo de render **constante** a scrollback
//! ilimitado), con numeración global y color por runs. El corazón
//! ([`view::visible_window`]) es puro y testeable sin GPU.

#![forbid(unsafe_code)]

pub mod store;
pub mod view;

pub use store::Scrollback;
pub use view::{
    content_height, line_surface, scroll_to_bottom, visible_window, LineStyle, TermMetrics,
    TermPalette, VisibleWindow,
};
