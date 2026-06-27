//! `shuma-shell-llimphi` (bin) — entrypoint fino.
//!
//! Toda la lógica (Model/update/view/App, sesiones, chrome) vive en la **lib**
//! homónima para que sea un frontend sobre core agnóstico (Regla 2) y la pueda
//! hospedar también pata. Este bin sólo arranca la app de ventana.

#![forbid(unsafe_code)]

fn main() {
    bitacora::abrir("shuma");
    // `--dock` arranca shuma como barra wlr-layer-shell (modo dock); sin flag,
    // como ventana normal.
    if std::env::args().skip(1).any(|a| a == "--dock") {
        shuma_shell_llimphi::run_dock();
    } else {
        shuma_shell_llimphi::run();
    }
}
