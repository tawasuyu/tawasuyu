//! `llimphi-widget-terminal` — superficie de terminal **infinita y
//! virtualizada**.
//!
//! Diseño completo: `02_ruway/shuma/SDD-TERMINAL.md`. El control reemplaza por
//! fases al `output_pane` del shell: scrollback ilimitado a costo de render
//! **constante** (sólo se pinta la ventana visible), tres modos sobre la misma
//! tela (línea IDE / grilla TUI / híbrido) y GPU directo donde paga.
//!
//! **Fase 0 (esto):** la **Capa 0** del SDD — el [`store::Scrollback`], el store
//! de scrollback append-only con índice de líneas, cap por memoria y acceso
//! O(1). Sin render todavía (llega en la Fase 1). Es puro y sin dependencias de
//! UI a propósito: el núcleo agnóstico vive aparte de quien lo pinta (Regla 2).

#![forbid(unsafe_code)]

pub mod store;

pub use store::Scrollback;
