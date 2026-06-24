//! Binario del panel de historial de notificaciones (sidebar derecho).

fn main() {
    pata_notify::init_tracing();
    pata_notify::panel::run();
}
