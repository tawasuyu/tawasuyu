//! Librería de `mirada-app-llimphi`: piezas de render reutilizables y
//! verificables headless, separadas del binario `mirada-llimphi`. Hoy expone la
//! **vista espacial** (el "Prezi" de mirada) para que `examples/dump_overview`
//! la pinte a PNG sin levantar el compositor.

pub mod overview;
