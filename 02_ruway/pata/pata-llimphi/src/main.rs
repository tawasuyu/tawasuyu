//! Binario del frontend `pata`: levanta el marco sobre Llimphi.
//!
//! Carga el `launcher.toml` del usuario (o el preset), resuelve la geometría y
//! pinta las superficies. Corre sobre el compositor `mirada`; standalone se ve
//! como un overlay a pantalla completa.
//!
//! ```sh
//! cargo run -p pata-llimphi --release
//! ```

fn main() {
    llimphi_ui::run::<pata_llimphi::PataApp>();
}
